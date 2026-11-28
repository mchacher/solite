//! `scripts/box-session.sh` (spec 033): the options, the stops, the groups and the report, run with
//! a fake `box.sh` (a shell script that logs its arguments and answers from environment
//! variables), a fake firmware builder, the Mac host as a simulator, and small test servers for
//! the cases the simulator cannot show (an update that restarts, a box that is busy, a clock that
//! is not set). No test touches a real box or a real serial port. The runs with `--yes-flash` go
//! through a pseudo-terminal, since the flag needs `/dev/tty`.
//!
//! Also here: `fw_size_line` of `scripts/lib/firmware.sh` (R7).

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/box-session.sh");
const FIRMWARE_LIB: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/lib/firmware.sh");
const GARAGE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios/garage.json");
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The synthetic password of the tests' accounts.
const PASSWORD: &str = "test-password-1234";
const NEXT: &str = "9.9.9";
static COUNTER: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------------------------
// Small tools
// ---------------------------------------------------------------------------------------------

struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "solite-bs-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // A host the fake `saves` left running.
        if let Ok(pid) = fs::read_to_string(self.0.join("state/host.pid")) {
            let _ = Command::new("kill").args(["-INT", pid.trim()]).status();
            thread::sleep(Duration::from_millis(300));
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn host() -> Command {
    Command::new(env!("CARGO_BIN_EXE_solite-host"))
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The tree's own version (`[workspace.package]` of the root Cargo.toml) with its patch number raised
/// by `by`: what the script builds for the next version and the rollback image. Read, not written
/// in the tests, so that a release does not break them.
fn tree_version_plus(by: u32) -> String {
    let manifest =
        fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml")).unwrap();
    let version = manifest
        .lines()
        .find_map(|line| line.strip_prefix("version = \""))
        .and_then(|rest| rest.split('"').next())
        .expect("a version in the workspace manifest");
    let mut parts: Vec<u32> = version.split('.').map(|p| p.parse().unwrap()).collect();
    parts[2] += by;
    format!("{}.{}.{}", parts[0], parts[1], parts[2])
}

fn write_exec(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// A request with curl; the answer's body.
fn curl(args: &[&str]) -> String {
    let output = Command::new("curl")
        .arg("-s")
        .args(args)
        .output()
        .expect("curl runs");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A simulated host serving on a free port: the devices of the garage paired first (or not), and
/// an account set up when a password is given.
struct Sim {
    _child: Reaped,
    port: u16,
    _data: TempDir,
}

impl Sim {
    fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

fn pair_into(data: &Path) {
    let output = host()
        .env("SOLITE_CLOSE_GRACE_SECONDS", "1")
        .args(["--sim", GARAGE, "--pair", "2", "--speed", "100", "--data"])
        .arg(data)
        .output()
        .expect("the host program starts");
    assert!(output.status.success(), "{output:?}");
}

fn serve_into(data: &Path, port: u16) -> Reaped {
    let mut child = Reaped(
        host()
            .env("SOLITE_CLOSE_GRACE_SECONDS", "1")
            .args([
                "--sim",
                GARAGE,
                "--serve",
                &port.to_string(),
                "--speed",
                "100",
            ])
            .arg("--data")
            .arg(data)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the host program starts"),
    );
    let mut stdout = child.0.stdout.take().unwrap();
    let mut seen = String::new();
    let mut byte = [0u8; 1];
    while !seen.contains("serving:") {
        match stdout.read(&mut byte) {
            Ok(1) => seen.push(byte[0] as char),
            _ => panic!("the host stopped or stayed silent: {seen}"),
        }
    }
    // The pipe is not read any more: the host's later lines go nowhere.
    let _ = child.0.stdout.insert(stdout);
    child
}

fn sim(pair: bool, account: Option<&str>) -> Sim {
    let data = TempDir::new("sim");
    if pair {
        pair_into(data.path());
    }
    let port = free_port();
    let child = serve_into(data.path(), port);
    if let Some(password) = account {
        let body =
            format!(r#"{{"username":"admin","password":"{password}","displayName":"admin"}}"#);
        let answer = curl(&[
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
            &format!("http://127.0.0.1:{port}/api/v1/auth/setup"),
        ]);
        assert!(answer.contains("accessToken"), "{answer}");
    }
    Sim {
        _child: child,
        port,
        _data: data,
    }
}

// ---------------------------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------------------------

struct Out {
    code: i32,
    text: String,
}

impl Out {
    /// The status of a step or a check (`OK (...)`, `KO (...)`, `skipped (...)`), by its id.
    fn status(&self, id: &str) -> String {
        let prefix = format!("box-session: {id} ");
        for line in self.text.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(rest) = line.strip_prefix(&prefix) {
                return rest.split_once(": ").map_or(rest, |(_, s)| s).to_string();
            }
        }
        panic!("no line for {id} in:\n{}", self.text);
    }

    fn ok(&self, id: &str) {
        let status = self.status(id);
        assert!(
            status == "OK" || status.starts_with("OK ("),
            "{id}: {status}\n{}",
            self.text
        );
    }

    fn ko(&self, id: &str) -> String {
        let status = self.status(id);
        assert!(status.starts_with("KO"), "{id}: {status}\n{}", self.text);
        status
    }

    fn skipped(&self, id: &str, reason: &str) {
        let status = self.status(id);
        assert_eq!(status, format!("skipped ({reason})"), "{id}\n{}", self.text);
    }

    fn last_line(&self) -> String {
        self.text
            .lines()
            .map(|l| l.trim_end_matches('\r'))
            .rfind(|l| !l.is_empty())
            .unwrap_or("")
            .to_string()
    }
}

// ---------------------------------------------------------------------------------------------
// One group, straight (the way the script calls itself inside `box.sh saves`)
// ---------------------------------------------------------------------------------------------

/// Runs `--group <name>` against an address, with a run folder of its own. The environment is
/// clean of the user's `SOLITE_*` variables; `envs` adds to it.
fn group_in(dir: &Path, name: &str, addr: &str, envs: &[(&str, &str)]) -> Out {
    let mut command = Command::new("sh");
    command.arg(SCRIPT).args(["--group", name]);
    for key in [
        "SOLITE_PASSWORD",
        "SOLITE_PYTHON",
        "SOLITE_HOME_LATITUDE",
        "SOLITE_HOME_LONGITUDE",
        "SOLITE_HOME_TIMEZONE",
        "SOLITE_SESSION_API_ONLY",
        "SOLITE_SESSION_RATE_WAIT",
        "SOLITE_SESSION_CLOCK_WAIT",
        "SOLITE_SESSION_BOOT_WAIT",
        "SOLITE_SESSION_CONFIRM_WAIT",
        "SOLITE_SESSION_ROLLBACK_WAIT",
        "SOLITE_SESSION_POLL",
        "SOLITE_SESSION_ID",
    ] {
        command.env_remove(key);
    }
    command
        .env("SOLITE_SESSION_DIR", dir)
        .env("SOLITE_BOX_ADDRESS", addr)
        .env("SOLITE_PASSWORD_FILE", dir.join("password"))
        .envs(envs.iter().copied());
    let output = command.output().expect("sh runs the script");
    Out {
        code: output.status.code().unwrap_or(-1),
        text: String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr),
    }
}

fn group(name: &str, addr: &str, envs: &[(&str, &str)]) -> (Out, TempDir) {
    let dir = TempDir::new("group");
    let out = group_in(dir.path(), name, addr, envs);
    (out, dir)
}

// ---------------------------------------------------------------------------------------------
// A small test server: the handler answers (status, body)
// ---------------------------------------------------------------------------------------------

struct Req {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// False when the declared body did not arrive.
    complete: bool,
}

impl Req {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn bearer(&self) -> Option<&str> {
        self.header("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
    }
}

type Handler = Arc<dyn Fn(&Req) -> (u16, String) + Send + Sync>;

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        409 => "Conflict",
        429 => "Too Many Requests",
        _ => "Service Unavailable",
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Req> {
    let mut raw = Vec::new();
    let mut buffer = [0u8; 8192];
    let head_end = loop {
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&buffer[..n]);
        if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut words = lines.next()?.split(' ');
    let method = words.next()?.to_string();
    let path = words.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let length = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = raw[head_end + 4..].to_vec();
    stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .ok()?;
    let mut complete = true;
    while body.len() < length {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&buffer[..n]),
            Err(_) => {
                complete = false;
                break;
            }
        }
    }
    Some(Req {
        method,
        path,
        headers,
        body,
        complete,
    })
}

fn serve_one(mut stream: TcpStream, handler: &Handler) {
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let (status, body) = handler(&request);
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len()
    );
    let _ = stream.write_all(body.as_bytes());
}

/// Serves with the handler on a free port; returns the port.
fn fake_server(handler: Handler) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = Arc::clone(&handler);
            thread::spawn(move || serve_one(stream, &handler));
        }
    });
    port
}

fn tokens(epoch: u32) -> String {
    format!(r#"{{"accessToken":"tok-{epoch}","refreshToken":"ref-{epoch}"}}"#)
}

fn days_to_date(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// ISO 8601 of `seconds_ago` seconds ago.
fn iso_ago(seconds_ago: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let t = now - seconds_ago;
    let (y, m, d) = days_to_date(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

// ---------------------------------------------------------------------------------------------
// The auth group
// ---------------------------------------------------------------------------------------------

#[test]
fn the_auth_group_on_a_host_with_no_account_sets_it_up_and_checks_the_rate_limit() {
    let host = sim(false, None);
    let (out, dir) = group("auth", &host.addr(), &[]);
    assert_eq!(out.code, 0, "{}", out.text);
    for n in 1..=7 {
        out.ok(&format!("B4.{n}"));
    }
    assert!(out.status("B4.2").contains("password in "), "{}", out.text);
    assert!(out.status("B4.5").contains("closed 4001"), "{}", out.text);
    assert!(out.status("B4.6").contains("202"), "{}", out.text);
    // The setup and the timed login count: the 429 is at the 11th attempt of the minute.
    assert_eq!(out.status("B4.7"), "OK (429 at attempt 11)");
    let password = dir.path().join("password");
    assert_eq!(mode(&password), 0o600);
    assert!(dir.path().join("account-usable").exists());
    // The generated password: 24 hexadecimal characters, in no line of the output.
    let text = fs::read_to_string(&password).unwrap();
    let generated = text.trim();
    assert_eq!(generated.len(), 24, "{generated}");
    assert!(generated.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(!out.text.contains(generated), "{}", out.text);
}

#[test]
fn the_auth_group_on_a_host_with_an_account_and_no_password_says_so() {
    let host = sim(false, Some(PASSWORD));
    let (out, _dir) = group("auth", &host.addr(), &[]);
    assert_eq!(out.code, 1, "{}", out.text);
    assert_eq!(
        out.status("B4.2"),
        "KO (set SOLITE_PASSWORD (the box has an account))"
    );
    // The checks that need no password still run.
    out.ok("B4.1");
    out.ok("B4.4");
}

#[test]
fn a_403_confirm_required_at_setup_is_ko_with_the_boxs_text() {
    let handler: Handler = Arc::new(|r: &Req| match (r.method.as_str(), r.path.as_str()) {
        ("GET", "/api/v1/auth/status") => (200, r#"{"setupRequired":true}"#.into()),
        ("POST", "/api/v1/auth/setup") => (
            403,
            r#"{"error":"Press the button on the box, then retry","confirmRequired":true}"#.into(),
        ),
        _ => (401, r#"{"error":"Authentication required"}"#.into()),
    });
    let port = fake_server(handler);
    let (out, dir) = group("auth", &format!("127.0.0.1:{port}"), &[]);
    let status = out.ko("B4.2");
    assert!(
        status.contains("Press the button on the box, then retry"),
        "{status}"
    );
    assert!(!dir.path().join("account-usable").exists());
}

// ---------------------------------------------------------------------------------------------
// The home group
// ---------------------------------------------------------------------------------------------

#[test]
fn the_home_group_with_the_location_in_the_environment_sets_it_and_reads_it_back() {
    let host = sim(true, Some(PASSWORD));
    let (out, dir) = group(
        "home",
        &host.addr(),
        &[
            ("SOLITE_PASSWORD", PASSWORD),
            ("SOLITE_SESSION_API_ONLY", "1"),
            ("SOLITE_HOME_LATITUDE", "45.0"),
            ("SOLITE_HOME_LONGITUDE", "5.0"),
            ("SOLITE_HOME_TIMEZONE", "Europe/Paris"),
            ("SOLITE_SESSION_CLOCK_WAIT", "5"),
        ],
    );
    assert_eq!(out.code, 0, "{}", out.text);
    for n in 1..=5 {
        out.ok(&format!("B7.{n}"));
    }
    assert!(out.status("B7.3").contains("sunrise "), "{}", out.text);
    assert!(out.status("B7.4").contains("Europe/Paris"), "{}", out.text);
    out.skipped("B7.6", "box only");
    out.skipped("B7.7", "box only");
    assert_eq!(
        fs::read_to_string(dir.path().join("location")).unwrap(),
        "45.0\n5.0\nEurope/Paris\n"
    );
}

#[test]
fn the_home_group_without_a_location_puts_the_settings_back_and_skips_the_sun_times() {
    let host = sim(true, Some(PASSWORD));
    let (out, dir) = group(
        "home",
        &host.addr(),
        &[
            ("SOLITE_PASSWORD", PASSWORD),
            ("SOLITE_SESSION_API_ONLY", "1"),
        ],
    );
    assert_eq!(out.code, 0, "{}", out.text);
    out.ok("B7.1");
    out.ok("B7.2");
    out.skipped(
        "B7.3",
        "set SOLITE_HOME_LATITUDE, SOLITE_HOME_LONGITUDE and SOLITE_HOME_TIMEZONE",
    );
    // The time zone is the fallback while the settings hold none.
    assert_eq!(out.status("B7.4"), "OK (UTC (fallback))");
    out.ok("B7.5");
    assert!(!dir.path().join("location").exists());
}

/// A box whose settings hold a location and whose clock never sets: the aggregation's `sunrise`
/// stays `null`.
fn home_fake(devices: Arc<Mutex<String>>, sunrise: &'static str) -> Handler {
    Arc::new(move |r: &Req| {
        match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/api/v1/auth/login") => (200, tokens(1)),
        ("POST", "/api/v1/integrations/solite/permit-join") => {
            (202, r#"{"permitJoin":false,"remaining":0}"#.into())
        }
        ("GET", "/api/v1/system/backup") => match r.header("x-backup-password") {
            Some(_) => (200, "SOLITEBK-fake-archive".into()),
            None => (400, r#"{"error":"Backup password required"}"#.into()),
        },
        ("POST", "/api/v1/system/backup") => {
            (400, r#"{"error":"Wrong backup password"}"#.into())
        }
        ("PUT", "/api/v1/settings") => (200, r#"{"success":true}"#.into()),
        ("GET", "/api/v1/settings") => {
            if sunrise == "location" {
                (
                    200,
                    r#"{"home.latitude":"45.0","home.longitude":"5.0","home.timezone":"Europe/Paris"}"#
                        .into(),
                )
            } else {
                (200, "{}".into())
            }
        }
        ("GET", "/api/v1/zones/aggregation") => (
            200,
            r#"{"00000000-0000-0000-0000-000000000001":{"sunrise":null,"sunset":null,"isDaylight":null}}"#
                .into(),
        ),
        ("GET", "/api/v1/system/timezone") => {
            if sunrise == "location" {
                (200, r#"{"tz":"Europe/Paris","source":"env","offsetHours":1}"#.into())
            } else {
                (200, r#"{"tz":"UTC","source":"fallback","offsetHours":0}"#.into())
            }
        }
        ("GET", "/api/v1/devices") => (200, devices.lock().unwrap().clone()),
        _ => (404, r#"{"error":"Not found"}"#.into()),
    }
    })
}

#[test]
fn a_restore_refused_for_presence_or_pairing_is_not_checked_rather_than_ko() {
    for (status, text, reason) in [
        (
            403,
            r#"{"error":"Confirm on the box","confirmRequired":true}"#,
            "presence: Confirm on the box",
        ),
        (
            409,
            r#"{"error":"Pairing in progress"}"#,
            "pairing window open: Pairing in progress",
        ),
    ] {
        let devices = Arc::new(Mutex::new("[]".to_string()));
        let inner = home_fake(devices, "none");
        let handler: Handler = Arc::new(move |r: &Req| {
            if r.method == "POST" && r.path == "/api/v1/system/backup" {
                (status, text.to_string())
            } else {
                inner(r)
            }
        });
        let port = fake_server(handler);
        let (out, _dir) = group(
            "home",
            &format!("127.0.0.1:{port}"),
            &[
                ("SOLITE_PASSWORD", PASSWORD),
                ("SOLITE_SESSION_API_ONLY", "1"),
                ("SOLITE_SESSION_CLOCK_WAIT", "1"),
            ],
        );
        out.skipped("B7.1", &format!("exported; restore not checked ({reason})"));
    }
}

#[test]
fn a_clock_that_is_never_set_is_ko_and_the_dates_are_skipped() {
    let devices = Arc::new(Mutex::new("[]".to_string()));
    let port = fake_server(home_fake(devices, "location"));
    let (out, _dir) = group(
        "home",
        &format!("127.0.0.1:{port}"),
        &[
            ("SOLITE_PASSWORD", PASSWORD),
            ("SOLITE_SESSION_API_ONLY", "1"),
            ("SOLITE_HOME_LATITUDE", "45.0"),
            ("SOLITE_HOME_LONGITUDE", "5.0"),
            ("SOLITE_HOME_TIMEZONE", "Europe/Paris"),
            ("SOLITE_SESSION_CLOCK_WAIT", "1"),
        ],
    );
    assert_eq!(out.code, 1, "{}", out.text);
    assert_eq!(out.status("B7.3"), "KO (clock not set within 1 s)");
    out.skipped("B7.5", "clock not set");
}

#[test]
fn a_device_heard_20_minutes_ago_is_offline_if_it_is_mains_and_online_is_ko() {
    let device = |status: &str| {
        format!(
            r#"[{{"id":"d1","name":"Relay","powerSource":"mains","status":"{status}","lastSeen":"{}","createdAt":"{}","updatedAt":"{}"}}]"#,
            iso_ago(1200),
            iso_ago(3000),
            iso_ago(1200)
        )
    };
    let devices = Arc::new(Mutex::new(device("offline")));
    let port = fake_server(home_fake(Arc::clone(&devices), "none"));
    let envs = [
        ("SOLITE_PASSWORD", PASSWORD),
        ("SOLITE_SESSION_API_ONLY", "1"),
        ("SOLITE_SESSION_CLOCK_WAIT", "1"),
    ];
    let address = format!("127.0.0.1:{port}");
    let (out, _dir) = group("home", &address, &envs);
    assert_eq!(out.status("B7.5"), "OK (1 devices checked)", "{}", out.text);
    *devices.lock().unwrap() = device("online");
    let (out, _dir) = group("home", &address, &envs);
    let status = out.ko("B7.5");
    assert!(
        status.contains("Relay") && status.contains("not offline"),
        "{status}"
    );
}

/// A box that is busy: four requests whose body never comes keep its workers, the fifth answers
/// 503 (or 200, to show the check failing), and the four end with 408 after a second.
fn busy_fake(fifth_answers_200: bool) -> Handler {
    let held = Arc::new(AtomicUsize::new(0));
    Arc::new(move |r: &Req| match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/api/v1/auth/login") if !r.complete => {
            held.fetch_add(1, Ordering::SeqCst);
            thread::sleep(Duration::from_secs(1));
            held.fetch_sub(1, Ordering::SeqCst);
            (408, r#"{"error":"Request Timeout"}"#.into())
        }
        ("POST", "/api/v1/auth/login") => (200, tokens(1)),
        ("GET", "/api/v1/auth/status") => {
            if held.load(Ordering::SeqCst) >= 4 && !fifth_answers_200 {
                (503, r#"{"error":"Busy"}"#.into())
            } else {
                (200, r#"{"setupRequired":false}"#.into())
            }
        }
        ("POST", "/api/v1/integrations/solite/permit-join") => {
            (202, r#"{"permitJoin":false,"remaining":0}"#.into())
        }
        ("GET", "/api/v1/system/backup") => match r.header("x-backup-password") {
            Some(_) => (200, "SOLITEBK-fake-archive".into()),
            None => (400, r#"{"error":"Backup password required"}"#.into()),
        },
        ("POST", "/api/v1/system/backup") => (400, r#"{"error":"Wrong backup password"}"#.into()),
        ("GET", "/api/v1/settings") => (200, "{}".into()),
        ("PUT", "/api/v1/settings") => (200, r#"{"success":true}"#.into()),
        ("GET", "/api/v1/system/timezone") => (
            200,
            r#"{"tz":"UTC","source":"fallback","offsetHours":0}"#.into(),
        ),
        ("GET", "/api/v1/devices") => (200, "[]".into()),
        ("GET", "/") => (
            200,
            r#"<script src="/assets/index-abc123.js"></script>"#.into(),
        ),
        // A script the client never reads: the answer is larger than any buffer.
        ("GET", "/assets/index-abc123.js") => (200, "x".repeat(8 * 1024 * 1024)),
        _ => (404, r#"{"error":"Not found"}"#.into()),
    })
}

#[test]
fn the_busy_box_and_the_client_that_stops_reading_are_checked_on_a_box() {
    let port = fake_server(busy_fake(false));
    let (out, _dir) = group(
        "home",
        &format!("127.0.0.1:{port}"),
        &[
            ("SOLITE_PASSWORD", PASSWORD),
            ("SOLITE_SESSION_CLOCK_WAIT", "1"),
        ],
    );
    out.ok("B7.6");
    out.ok("B7.7");
    assert_eq!(out.code, 0, "{}", out.text);
}

#[test]
fn a_fifth_request_answered_200_makes_the_busy_check_ko() {
    let port = fake_server(busy_fake(true));
    let (out, _dir) = group(
        "home",
        &format!("127.0.0.1:{port}"),
        &[
            ("SOLITE_PASSWORD", PASSWORD),
            ("SOLITE_SESSION_CLOCK_WAIT", "1"),
        ],
    );
    out.ko("B7.6");
    assert_eq!(out.code, 1);
}

#[test]
fn a_missing_python_skips_the_websocket_the_busy_and_the_reader_checks() {
    let host = sim(true, Some(PASSWORD));
    let (out, _dir) = group(
        "home",
        &host.addr(),
        &[
            ("SOLITE_PASSWORD", PASSWORD),
            ("SOLITE_PYTHON", "no-such-python-program"),
            ("SOLITE_SESSION_CLOCK_WAIT", "1"),
        ],
    );
    out.skipped("B7.6", "no python3");
    out.skipped("B7.7", "no python3");
    assert_eq!(out.code, 0, "{}", out.text);
    // The websocket check of the auth group, on a host with an account the test sets up.
    let (auth, _dir) = group(
        "auth",
        &host.addr(),
        &[
            ("SOLITE_PASSWORD", PASSWORD),
            ("SOLITE_PYTHON", "no-such-python-program"),
        ],
    );
    auth.skipped("B4.5", "no python3");
}

// ---------------------------------------------------------------------------------------------
// The update group
// ---------------------------------------------------------------------------------------------

/// What the fake update server does.
#[derive(Clone, Copy, Default)]
struct UpdateBehaviour {
    /// The settings still hold the location after the restart.
    keeps_location: bool,
    /// The version never changes.
    stuck: bool,
    /// The second upload is refused: the running image is not confirmed.
    unconfirmed: bool,
    /// The second upload does not restart the box (the old token stays valid).
    no_restart: bool,
}

struct UpdateState {
    epoch: u32,
    current: String,
    uploads: u32,
}

/// A box that installs an update, restarts (its sessions end), comes back with the new version,
/// takes a second upload and restarts again on the first version, as the bootloader's rollback does.
fn update_fake(behaviour: UpdateBehaviour) -> Handler {
    let state = Arc::new(Mutex::new(UpdateState {
        epoch: 1,
        current: "0.1.0".into(),
        uploads: 0,
    }));
    Arc::new(move |r: &Req| {
        let mut state = state.lock().unwrap();
        match (r.method.as_str(), r.path.as_str()) {
            ("GET", "/api/v1/auth/status") => return (200, r#"{"setupRequired":false}"#.into()),
            ("POST", "/api/v1/auth/login") => return (200, tokens(state.epoch)),
            _ => {}
        }
        match r.bearer() {
            None => return (401, r#"{"error":"Authentication required"}"#.into()),
            Some(token) if token != format!("tok-{}", state.epoch) => {
                return (401, r#"{"error":"Invalid or expired token"}"#.into());
            }
            Some(_) => {}
        }
        match (r.method.as_str(), r.path.as_str()) {
            ("POST", "/api/v1/integrations/solite/permit-join") => {
                (202, r#"{"permitJoin":false,"remaining":0}"#.into())
            }
            ("GET", "/api/v1/system/version") => (
                200,
                format!(
                    r#"{{"current":"{}","latest":null,"updateAvailable":false}}"#,
                    state.current
                ),
            ),
            ("GET", "/api/v1/settings") => {
                if behaviour.keeps_location {
                    (
                        200,
                        r#"{"home.latitude":"45.0","home.longitude":"5.0","home.timezone":"Europe/Paris"}"#
                            .into(),
                    )
                } else {
                    (200, "{}".into())
                }
            }
            ("POST", "/api/v1/system/update") if r.body.len() <= 128 => {
                (400, r#"{"error":"Invalid signature"}"#.into())
            }
            ("POST", "/api/v1/system/update") => {
                state.uploads += 1;
                if state.uploads == 1 {
                    state.epoch += 1;
                    if !behaviour.stuck {
                        state.current = NEXT.into();
                    }
                    (
                        200,
                        format!(r#"{{"success":true,"message":"Installed {NEXT}, restarting"}}"#),
                    )
                } else if behaviour.unconfirmed {
                    (
                        409,
                        r#"{"error":"The running image is not confirmed yet"}"#.into(),
                    )
                } else {
                    if !behaviour.no_restart {
                        state.epoch += 1;
                    }
                    (
                        200,
                        r#"{"success":true,"message":"Installed 9.9.10, restarting"}"#.into(),
                    )
                }
            }
            _ => (404, r#"{"error":"Not found"}"#.into()),
        }
    })
}

/// The files the update group reads (P4's), in a run folder.
fn update_files(dir: &Path, location: bool) {
    fs::write(dir.join("next.version"), format!("{NEXT}\n")).unwrap();
    for name in ["next.sup", "rollback.sup", "bad.sup"] {
        fs::write(dir.join(name), vec![7u8; 300]).unwrap();
    }
    if location {
        fs::write(dir.join("location"), "45.0\n5.0\nEurope/Paris\n").unwrap();
    }
}

const FAST: [(&str, &str); 5] = [
    ("SOLITE_PASSWORD", PASSWORD),
    ("SOLITE_SESSION_BOOT_WAIT", "4"),
    ("SOLITE_SESSION_CONFIRM_WAIT", "1"),
    ("SOLITE_SESSION_ROLLBACK_WAIT", "6"),
    ("SOLITE_SESSION_POLL", "1"),
];

fn update_run(behaviour: UpdateBehaviour, location: bool) -> Out {
    let port = fake_server(update_fake(behaviour));
    let dir = TempDir::new("update");
    update_files(dir.path(), location);
    group_in(dir.path(), "update", &format!("127.0.0.1:{port}"), &FAST)
}

#[test]
fn an_update_that_installs_restarts_confirms_and_rolls_back_passes_all_six_checks() {
    let out = update_run(
        UpdateBehaviour {
            keeps_location: true,
            ..Default::default()
        },
        true,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    for n in 1..=6 {
        out.ok(&format!("B8.{n}"));
    }
    assert!(
        out.status("B8.4")
            .contains("the settings survived the restart"),
        "{}",
        out.text
    );
}

#[test]
fn the_settings_that_lost_the_location_in_the_restart_make_check_4_ko() {
    let out = update_run(UpdateBehaviour::default(), true);
    assert!(out.ko("B8.4").contains("lost the location"), "{}", out.text);
    assert_eq!(out.code, 1);
}

#[test]
fn a_box_that_never_comes_back_with_the_next_version_is_ko() {
    let out = update_run(
        UpdateBehaviour {
            stuck: true,
            ..Default::default()
        },
        false,
    );
    assert_eq!(
        out.ko("B8.4"),
        format!("KO (the box did not come back with {NEXT} within 4 s)"),
        "{}",
        out.text
    );
    out.skipped("B8.5", "after check 4");
}

#[test]
fn an_unconfirmed_image_refuses_the_second_upload_and_check_5_is_ko() {
    let out = update_run(
        UpdateBehaviour {
            unconfirmed: true,
            ..Default::default()
        },
        false,
    );
    out.ok("B8.4");
    let status = out.ko("B8.5");
    assert!(
        status.contains("The running image is not confirmed yet"),
        "{status}"
    );
    out.skipped("B8.6", "after check 5");
}

#[test]
fn a_rollback_with_no_restart_is_ko_and_asks_for_a_stop() {
    let dir = TempDir::new("update");
    update_files(dir.path(), false);
    let port = fake_server(update_fake(UpdateBehaviour {
        no_restart: true,
        ..Default::default()
    }));
    let out = group_in(dir.path(), "update", &format!("127.0.0.1:{port}"), &FAST);
    let status = out.ko("B8.6");
    assert!(status.contains("no restart seen"), "{status}");
    assert!(
        status.contains("box.sh flash --image radio --keep-storage"),
        "{status}"
    );
    assert!(dir.path().join("stop").exists(), "the stop file");
}

#[test]
fn a_host_with_no_key_refuses_with_a_503_and_the_checks_after_are_skipped() {
    let host = sim(false, Some(PASSWORD));
    let dir = TempDir::new("update");
    update_files(dir.path(), false);
    let out = group_in(
        dir.path(),
        "update",
        &host.addr(),
        &[
            ("SOLITE_PASSWORD", PASSWORD),
            ("SOLITE_SESSION_API_ONLY", "1"),
        ],
    );
    assert_eq!(out.code, 0, "{}", out.text);
    out.ok("B8.1");
    out.skipped("B8.2", "this build has no update key");
    for n in 3..=6 {
        out.skipped(&format!("B8.{n}"), "this build has no update key");
    }
}

// ---------------------------------------------------------------------------------------------
// The reset group
// ---------------------------------------------------------------------------------------------

#[test]
fn the_reset_group_resets_asks_for_setup_and_sets_up_again() {
    let host = sim(false, Some(PASSWORD));
    let (out, _dir) = group("reset", &host.addr(), &[("SOLITE_PASSWORD", PASSWORD)]);
    assert_eq!(out.code, 0, "{}", out.text);
    out.ok("B10.0");
    out.ok("B10.1");
    out.ok("B10.2");
    out.ok("B10.3");
}

// ---------------------------------------------------------------------------------------------
// The outer run: a fake box.sh, a fake builder, the simulator
// ---------------------------------------------------------------------------------------------

const FAKE_BOX: &str = r#"#!/bin/sh
# A fake box.sh: logs its arguments, answers from FAKE_* variables.
echo "$*" >> "$FAKE_LOG"
cmd=$1
shift
n=$(cat "$FAKE_STATE/count" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$FAKE_STATE/count"
budget=
raw=false
while [ $# -gt 0 ]; do
  case "$1" in
    --budget) budget=$2; shift 2 ;;
    --raw) raw=true; shift ;;
    --image | --port | --backup | --seconds) shift 2 ;;
    --) shift; break ;;
    *) shift ;;
  esac
done
write_serial() {
  {
    echo "I (100) solite: solite $FAKE_VERSION"
    echo "I (200) storage: /data, 40 KB used of 8064 KB"
    [ -n "$FAKE_NO_RESUMED" ] || echo "I (300) network: resumed, channel 15, PAN 0x3C3D"
    echo "I (400) memory-network: internal 120 KB free (largest block 60 KB), psram 1800 KB free of 2048 KB"
    echo "I (500) heap: internal 90 KB free (min 80 KB), largest block 45 KB, psram 1900 KB free (min 1850 KB), largest block 1800 KB, engine stack 20 KB unused of 32 KB"
    [ -z "$FAKE_LEAK" ] || echo "I (600) leak: $FAKE_LEAK"
  } > "$1"
}
case "$cmd" in
  identify)
    if [ -n "$FAKE_IDENTIFY_RC" ]; then echo "box: no answer from the box" >&2; exit "$FAKE_IDENTIFY_RC"; fi
    echo "box: ESP32-D0WDR2-V3, 16 MB"
    ;;
  flash)
    if [ -n "$FAKE_FLASH_FAIL" ]; then echo "box: the flash failed" >&2; exit 1; fi
    echo "box: flashed"
    ;;
  log)
    if [ -n "$FAKE_LOG_FAIL" ]; then echo "box: the log is missing the memory line" >&2; exit 1; fi
    f="$FAKE_STATE/serial-$n.log"
    write_serial "$f"
    echo "box: serial log: $f"
    if ! $raw; then
      grep -a "solite $FAKE_VERSION" "$f"
      grep -a 'memory-network' "$f"
      grep -a 'heap:' "$f"
      grep -a 'storage:' "$f"
    fi
    if [ -n "$budget" ]; then
      echo "flash: 1 writes since boot"
    fi
    ;;
  saves)
    if [ -n "$FAKE_ADDRESS" ]; then
      addr=$FAKE_ADDRESS
    else
      if [ -f "$FAKE_STATE/host.pid" ]; then
        old=$(cat "$FAKE_STATE/host.pid")
        kill -INT "$old" 2>/dev/null
        i=0
        while kill -0 "$old" 2>/dev/null && [ "$i" -lt 50 ]; do sleep 0.2; i=$((i + 1)); done
      fi
      port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
      SOLITE_CLOSE_GRACE_SECONDS=1 "$SOLITE_HOST_BIN" --sim "$FAKE_SCENARIO" --serve "$port" --speed 100 --data "$FAKE_DATA" \
        </dev/null >"$FAKE_STATE/host-$n.log" 2>&1 &
      echo $! > "$FAKE_STATE/host.pid"
      i=0
      until curl -s -o /dev/null "http://127.0.0.1:$port/api/v1/auth/status"; do
        i=$((i + 1))
        [ "$i" -lt 100 ] || break
        sleep 0.2
      done
      addr=127.0.0.1:$port
    fi
    echo "box: saves held; serving at http://$addr/"
    SOLITE_BOX_ADDRESS=$addr "$@"
    code=$?
    echo "box: saves flushed (2 files written)"
    f="$FAKE_STATE/serial-$n.log"
    write_serial "$f"
    m=${FAKE_WRITES:-1}
    echo "box: flash: $m writes since boot"
    rc=0
    if [ -n "$budget" ] && [ "$m" -gt "$budget" ]; then
      echo "box: flash: $m writes since boot, over the budget of $budget"
      rc=1
    fi
    if [ "$code" -ne 0 ]; then
      echo "box: the command failed (exit $code)"
      rc=1
    fi
    echo "box: serial log: $f"
    exit "$rc"
    ;;
esac
exit 0
"#;

/// A builder that writes a few bytes; fails for the kind in FAKE_BUILD_FAIL.
const FAKE_BUILDER: &str = r#"#!/bin/sh
echo "builder $1 $2 key:${3%"${3#????????}"}" >> "$FAKE_LOG"
if [ "$1" = "$FAKE_BUILD_FAIL" ]; then
  echo "the $1 build failed: no space left" >&2
  exit 1
fi
printf 'fake image %s %s' "$1" "$2" > "$4"
"#;

/// The script in a pseudo-terminal (it reads /dev/tty), its output kept.
const PTY_HELPER: &str = r#"
import os, pty, sys
pid, fd = pty.fork()
if pid == 0:
    os.execvp(sys.argv[1], sys.argv[1:])
while True:
    try:
        data = os.read(fd, 4096)
    except OSError:
        break
    if not data:
        break
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()
_, status = os.waitpid(pid, 0)
sys.exit(os.waitstatus_to_exitcode(status))
"#;

/// The script with no controlling terminal at all.
const NOTTY_HELPER: &str = r#"
import os, sys
pid = os.fork()
if pid == 0:
    os.setsid()
    os.execvp(sys.argv[1], sys.argv[1:])
_, status = os.waitpid(pid, 0)
sys.exit(os.waitstatus_to_exitcode(status))
"#;

#[derive(Clone, Copy)]
enum Tty {
    Pty,
    None,
}

/// The folders and fakes of one outer run.
struct Session {
    dir: TempDir,
    extra: Vec<(String, String)>,
}

impl Session {
    /// With the garage's devices paired into the data folder the fake `saves` serves.
    fn new() -> Session {
        let dir = TempDir::new("session");
        let root = dir.path();
        for name in ["fake", "state", "data", "runs", "home", "radio"] {
            fs::create_dir_all(root.join(name)).unwrap();
        }
        write_exec(&root.join("fake/box.sh"), FAKE_BOX);
        write_exec(&root.join("fake/builder.sh"), FAKE_BUILDER);
        fs::write(root.join("fake/backup.bin"), b"backup").unwrap();
        fs::write(root.join("log"), b"").unwrap();
        pair_into(&root.join("data"));
        Session {
            dir,
            extra: Vec::new(),
        }
    }

    fn with(mut self, key: &str, value: &str) -> Session {
        self.extra.push((key.into(), value.into()));
        self
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn backup(&self) -> String {
        self.root().join("fake/backup.bin").display().to_string()
    }

    /// The fake box's log: one line per call.
    fn calls(&self) -> String {
        fs::read_to_string(self.root().join("log")).unwrap()
    }

    /// The run folder the script made (there is one per session).
    fn run_dir(&self) -> PathBuf {
        let mut runs: Vec<PathBuf> = fs::read_dir(self.root().join("runs"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        assert_eq!(runs.len(), 1, "{runs:?}");
        runs.remove(0)
    }

    fn command(&self, args: &[&str], tty: Tty) -> Command {
        let root = self.root();
        let mut command = match tty {
            Tty::Pty => {
                let mut c = Command::new("python3");
                c.args(["-c", PTY_HELPER, "sh", SCRIPT]);
                c
            }
            Tty::None => {
                let mut c = Command::new("python3");
                c.args(["-c", NOTTY_HELPER, "sh", SCRIPT]);
                c
            }
        };
        command.args(args);
        for key in [
            "SOLITE_PASSWORD",
            "SOLITE_BOX_ADDRESS",
            "SOLITE_BOX_BACKUP",
            "SOLITE_PYTHON",
            "SOLITE_HOME_LATITUDE",
            "SOLITE_HOME_LONGITUDE",
            "SOLITE_HOME_TIMEZONE",
            "SOLITE_SESSION_API_ONLY",
            "SOLITE_SESSION_RATE_WAIT",
            "SOLITE_SESSION_TEST_HOST",
        ] {
            command.env_remove(key);
        }
        command
            .env("SOLITE_BOX_SCRIPT", root.join("fake/box.sh"))
            .env("SOLITE_HOST_BIN", env!("CARGO_BIN_EXE_solite-host"))
            .env("SOLITE_SESSION_BUILDER", root.join("fake/builder.sh"))
            .env("SOLITE_SESSION_ROOT", root.join("runs"))
            .env("SOLITE_PASSWORD_FILE", root.join("password"))
            .env("SOLITE_RADIO_TARGET_DIR", root.join("radio"))
            .env("HOME", root.join("home"))
            .env("SOLITE_SESSION_BOOT_WAIT", "2")
            .env("SOLITE_SESSION_CONFIRM_WAIT", "1")
            .env("SOLITE_SESSION_ROLLBACK_WAIT", "2")
            .env("SOLITE_SESSION_POLL", "1")
            .env("SOLITE_SESSION_CLOCK_WAIT", "1")
            .env("FAKE_LOG", root.join("log"))
            .env("FAKE_STATE", root.join("state"))
            .env("FAKE_DATA", root.join("data"))
            .env("FAKE_SCENARIO", GARAGE)
            .env("FAKE_VERSION", VERSION)
            .env_remove("FAKE_ADDRESS")
            .env_remove("FAKE_IDENTIFY_RC")
            .env_remove("FAKE_FLASH_FAIL")
            .env_remove("FAKE_LOG_FAIL")
            .env_remove("FAKE_WRITES")
            .env_remove("FAKE_NO_RESUMED")
            .env_remove("FAKE_BUILD_FAIL")
            .env_remove("FAKE_LEAK");
        for (key, value) in &self.extra {
            command.env(key, value);
        }
        command
    }

    fn run(&self, args: &[&str], tty: Tty) -> Out {
        let output = self
            .command(args, tty)
            .stdin(Stdio::null())
            .output()
            .expect("the script runs");
        Out {
            code: output.status.code().unwrap_or(-1),
            text: String::from_utf8_lossy(&output.stdout).replace('\r', "")
                + &String::from_utf8_lossy(&output.stderr),
        }
    }

    /// A run that flashes, the way the user types it: `--yes-flash --backup <file>` on a terminal.
    fn flash(&self, more: &[&str]) -> Out {
        let backup = self.backup();
        let mut args = vec!["--yes-flash", "--backup", backup.as_str()];
        args.extend_from_slice(more);
        self.run(&args, Tty::Pty)
    }
}

#[test]
fn without_yes_flash_nothing_is_flashed_and_nothing_is_written() {
    let session = Session::new();
    let out = session.run(&[], Tty::None);
    assert_eq!(out.code, 0, "{}", out.text);
    let calls = session.calls();
    assert!(
        !calls.contains("flash") && !calls.contains("saves"),
        "{calls}"
    );
    out.ok("P1");
    out.ok("B1");
    out.ok("B3");
    out.skipped("P5", "no radio build");
    for id in [
        "P2", "P3", "P4", "B2", "B4", "B5", "B6", "B7", "B8", "B9", "B10",
    ] {
        out.skipped(id, "needs --yes-flash");
    }
}

#[test]
fn yes_flash_without_a_backup_is_a_usage_error_and_the_box_is_never_called() {
    let session = Session::new();
    let out = session.run(&["--yes-flash"], Tty::Pty);
    assert_eq!(out.code, 2, "{}", out.text);
    assert!(
        out.text
            .contains("--yes-flash needs --backup <file> (the original firmware's backup)"),
        "{}",
        out.text
    );
    assert_eq!(session.calls(), "");
}

#[test]
fn yes_flash_with_a_missing_backup_file_is_a_usage_error_and_the_box_is_never_called() {
    let session = Session::new();
    let out = session.run(
        &["--yes-flash", "--backup", "/nonexistent/backup.bin"],
        Tty::Pty,
    );
    assert_eq!(out.code, 2, "{}", out.text);
    assert!(
        out.text
            .contains("backup not found: /nonexistent/backup.bin"),
        "{}",
        out.text
    );
    assert_eq!(session.calls(), "");
}

#[test]
fn yes_flash_with_api_only_or_an_unknown_option_is_a_usage_error() {
    let session = Session::new();
    let backup = session.backup();
    let out = session.run(
        &["--yes-flash", "--backup", &backup, "--api-only", "8080"],
        Tty::Pty,
    );
    assert_eq!(out.code, 2, "{}", out.text);
    let out = session.run(&["--no-such-option"], Tty::None);
    assert_eq!(out.code, 2, "{}", out.text);
    assert!(
        out.text.contains("unknown option --no-such-option"),
        "{}",
        out.text
    );
    assert_eq!(session.calls(), "");
}

#[test]
fn a_session_root_inside_the_repository_is_refused() {
    let session = Session::new().with(
        "SOLITE_SESSION_ROOT",
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../solite-box-session-test"),
    );
    let out = session.run(&[], Tty::None);
    assert_eq!(out.code, 2, "{}", out.text);
    assert!(out.text.contains("inside the repository"), "{}", out.text);
    assert!(
        !Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../solite-box-session-test"
        ))
        .exists()
    );
}

#[test]
fn api_only_refuses_a_host_that_is_not_the_mac() {
    let session = Session::new();
    let out = session.run(&["--api-only", "192.168.1.42"], Tty::None);
    assert_eq!(out.code, 2, "{}", out.text);
    assert!(
        out.text.contains(
            "box-session: --api-only is for the Mac host (127.0.0.1, localhost, ::1): 192.168.1.42 refused"
        ),
        "{}",
        out.text
    );
    let out = session.run(&["--api-only", "box.local:80"], Tty::None);
    assert_eq!(out.code, 2, "{}", out.text);
    assert_eq!(session.calls(), "");
}

#[test]
fn factory_reset_alone_is_a_usage_error() {
    let session = Session::new();
    let out = session.run(&["--factory-reset"], Tty::None);
    assert_eq!(out.code, 2, "{}", out.text);
    assert!(
        out.text
            .contains("--factory-reset needs --yes-flash or --api-only"),
        "{}",
        out.text
    );
}

#[test]
fn yes_flash_with_no_terminal_is_refused() {
    let session = Session::new();
    let backup = session.backup();
    let out = session.run(&["--yes-flash", "--backup", &backup], Tty::None);
    assert_eq!(out.code, 2, "{}", out.text);
    assert!(
        out.text
            .contains("--yes-flash is typed by the user in a terminal"),
        "{}",
        out.text
    );
    assert_eq!(session.calls(), "");
}

#[test]
fn the_nominal_pass_flashes_once_runs_the_groups_inside_saves_and_gives_a_verdict() {
    let session = Session::new()
        .with("SOLITE_SESSION_TEST_HOST", "1")
        .with("SOLITE_SESSION_RATE_WAIT", "0");
    let backup = session.backup();
    let out = session.flash(&[]);
    assert_eq!(out.code, 0, "{}", out.text);
    for id in [
        "P1", "P2", "P3", "P4", "B1", "B2", "B3", "B4", "B5", "B6", "B7", "B8", "B9",
    ] {
        out.ok(id);
    }
    out.skipped("B10", "erases the home: needs --factory-reset");
    let calls = session.calls();
    assert!(
        calls.contains(&format!(
            "flash --image radio --keep-storage --backup {backup}"
        )),
        "{calls}"
    );
    assert_eq!(calls.matches("flash --image").count(), 1, "{calls}");
    assert!(calls.contains("saves --budget 3 "), "{calls}");
    assert!(calls.contains("saves --budget 5 "), "{calls}");
    assert!(calls.contains("saves --budget 4 "), "{calls}");
    assert!(calls.contains("log --image radio --budget 1"), "{calls}");
    assert!(
        calls.contains("log --raw --seconds 300 --budget 1"),
        "{calls}"
    );
    // The builds: the base image with the key, the next version and the rollback image.
    assert!(
        calls.contains(&format!("builder base {} key:", tree_version_plus(0))),
        "{calls}"
    );
    assert!(
        calls.contains(&format!("builder next {} key:", tree_version_plus(1))),
        "{calls}"
    );
    assert!(
        calls.contains(&format!("builder rollback {} key:", tree_version_plus(2))),
        "{calls}"
    );
    // B8 against the simulator: no key, the checks after the 401 are skipped.
    out.ok("B8.1");
    out.skipped("B8.2", "this build has no update key");
    // B3 names the memory lines of the serial log.
    let b3 = out.status("B3");
    assert!(
        b3.contains("psram 1800 KB free")
            && b3.contains("min 80 KB")
            && b3.contains("largest block 45 KB")
            && b3.contains("40 KB used of 8064 KB"),
        "{b3}"
    );
    // The verdict.
    assert!(out.last_line().starts_with("box-session: "), "{}", out.text);
    assert!(out.last_line().contains(" 0 KO, "), "{}", out.last_line());
}

#[test]
fn a_run_has_its_folder_its_log_and_a_verdict_line_with_the_log_path() {
    let session = Session::new();
    let out = session.run(&[], Tty::None);
    let run = session.run_dir();
    assert_eq!(mode(&run), 0o700);
    let log = run.join("session.log");
    assert!(log.exists());
    let last = out.last_line();
    assert!(last.starts_with("box-session: "), "{last}");
    assert!(last.ends_with(&format!("log: {}", log.display())), "{last}");
    assert!(
        fs::read_to_string(&log)
            .unwrap()
            .contains("box-session: P1 preflight: OK")
    );
}

#[test]
fn the_generated_password_is_kept_in_the_password_file_and_never_printed() {
    let session = Session::new()
        .with("SOLITE_SESSION_TEST_HOST", "1")
        .with("SOLITE_SESSION_RATE_WAIT", "0");
    let out = session.flash(&[]);
    assert_eq!(out.code, 0, "{}", out.text);
    let file = session.root().join("password");
    assert_eq!(mode(&file), 0o600);
    let password = fs::read_to_string(&file).unwrap();
    let password = password.trim();
    assert_eq!(password.len(), 24, "{password}");
    assert!(!out.text.contains(password), "{}", out.text);
    assert!(
        out.last_line()
            .ends_with(&format!("password: {}", file.display())),
        "{}",
        out.last_line()
    );
    // The log neither.
    let log = fs::read_to_string(session.run_dir().join("session.log")).unwrap();
    assert!(!log.contains(password));
    // No key, no token left in the run folder.
    assert!(!session.run_dir().join("update.key").exists());
    assert!(!session.run_dir().join("tokens").exists());
}

#[test]
fn a_saves_over_its_budget_is_ko_with_the_count_and_the_later_box_steps_stop() {
    let session = Session::new()
        .with("FAKE_WRITES", "9")
        .with("SOLITE_SESSION_TEST_HOST", "1");
    let out = session.flash(&[]);
    assert_eq!(out.code, 1, "{}", out.text);
    assert_eq!(
        out.ko("B4"),
        "KO (flash: 9 writes since boot, over the budget of 3)",
        "{}",
        out.text
    );
    for id in ["B5", "B6", "B7", "B8", "B9"] {
        out.skipped(id, "stopped after B4");
    }
}

#[test]
fn a_leftover_api_only_variable_does_not_skip_the_box_only_checks_of_a_box_run() {
    let session = Session::new()
        .with("SOLITE_SESSION_API_ONLY", "1")
        .with("SOLITE_SESSION_RATE_WAIT", "0");
    let out = session.flash(&[]);
    for id in ["B7.6", "B7.7", "B8.3", "B8.4"] {
        let status = out.status(id);
        assert!(!status.contains("box only"), "{id}: {status}\n{}", out.text);
    }
}

#[test]
fn a_failing_log_after_the_flash_is_ko_with_its_reason_and_stops_the_box_steps() {
    let session = Session::new().with("FAKE_LOG_FAIL", "1");
    let out = session.flash(&[]);
    assert_eq!(out.code, 1, "{}", out.text);
    let status = out.ko("B3");
    assert!(
        status.contains("the log is missing the memory line"),
        "{status}"
    );
    for id in ["B4", "B5", "B6", "B7", "B8", "B9"] {
        out.skipped(id, "stopped after B3");
    }
}

#[test]
fn a_box_that_identify_refuses_or_does_not_answer_stops_the_box_steps() {
    for code in ["3", "4"] {
        let session = Session::new().with("FAKE_IDENTIFY_RC", code);
        let out = session.flash(&[]);
        assert_eq!(out.code, 1, "{}", out.text);
        out.ko("B1");
        for id in ["B2", "B3", "B4", "B6", "B7", "B8", "B9"] {
            out.skipped(id, "stopped after B1");
        }
        let calls = session.calls();
        assert!(
            !calls.contains("flash") && !calls.contains("saves"),
            "{calls}"
        );
    }
}

#[test]
fn a_builder_that_fails_for_the_base_image_is_a_ko_of_p3_and_no_flash_runs() {
    let session = Session::new().with("FAKE_BUILD_FAIL", "base");
    let out = session.flash(&[]);
    assert_eq!(out.code, 1, "{}", out.text);
    let status = out.ko("P3");
    assert!(status.contains("the base build failed"), "{status}");
    out.ok("B1");
    out.skipped("B2", "after P3 KO");
    assert!(!session.calls().contains("flash"), "{}", session.calls());
}

#[test]
fn a_builder_that_fails_for_the_next_version_skips_the_update_but_not_the_reset() {
    let session = Session::new()
        .with("FAKE_BUILD_FAIL", "next")
        .with("SOLITE_SESSION_TEST_HOST", "1")
        .with("SOLITE_SESSION_RATE_WAIT", "0");
    let out = session.flash(&["--factory-reset"]);
    assert_eq!(out.code, 1, "{}", out.text);
    assert!(
        out.ko("P4").contains("the next build failed"),
        "{}",
        out.text
    );
    out.skipped("B8", "after P4 KO");
    out.skipped("B9", "after B8 skipped");
    out.ok("B10");
}

#[test]
fn a_missing_version_line_in_the_copy_of_head_is_a_ko_of_p4_naming_the_line() {
    // A repository of its own, with the script, and a firmware/Cargo.toml with no version line.
    let session = Session::new();
    let repo = session.root().join("repo");
    fs::create_dir_all(repo.join("scripts/lib")).unwrap();
    fs::create_dir_all(repo.join("firmware/src")).unwrap();
    fs::copy(SCRIPT, repo.join("scripts/box-session.sh")).unwrap();
    fs::copy(FIRMWARE_LIB, repo.join("scripts/lib/firmware.sh")).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[workspace]\n\n[workspace.package]\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        repo.join("firmware/Cargo.toml"),
        "[package]\nname = \"solite-firmware\"\n",
    )
    .unwrap();
    fs::write(repo.join("firmware/src/main.rs"), "fn main() {\n}\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "-A"],
        vec![
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "x",
        ],
    ] {
        let status = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(&args)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let backup = session.backup();
    let command = session.command(&["--yes-flash", "--backup", &backup], Tty::Pty);
    // The script of the fake repository, with no builder seam: its own builder runs.
    let mut args: Vec<std::ffi::OsString> = command.get_args().map(Into::into).collect();
    let script = repo.join("scripts/box-session.sh").into_os_string();
    for arg in &mut args {
        if arg == SCRIPT {
            *arg = script.clone();
        }
    }
    let mut command2 = Command::new(command.get_program());
    command2.args(&args);
    for (key, value) in command.get_envs() {
        match value {
            Some(value) if key != "SOLITE_SESSION_BUILDER" => {
                command2.env(key, value);
            }
            _ => {
                command2.env_remove(key);
            }
        }
    }
    command2.env_remove("SOLITE_SESSION_BUILDER");
    let output = command2.stdin(Stdio::null()).output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout).replace('\r', "");
    let out = Out {
        code: output.status.code().unwrap_or(-1),
        text,
    };
    let status = out.ko("P4");
    assert!(
        status.contains("no version line under [package] in box-session-tree/firmware/Cargo.toml"),
        "{status}"
    );
}

#[test]
fn a_serial_log_without_network_resumed_is_a_ko_of_b5_and_the_run_goes_on() {
    let session = Session::new()
        .with("FAKE_NO_RESUMED", "1")
        .with("SOLITE_SESSION_TEST_HOST", "1")
        .with("SOLITE_SESSION_RATE_WAIT", "0");
    let out = session.flash(&[]);
    assert_eq!(out.code, 1, "{}", out.text);
    let status = out.ko("B5");
    assert!(status.contains("network: resumed"), "{status}");
    out.ok("B6");
    out.ok("B7");
}

#[test]
fn the_password_in_the_serial_log_is_a_ko_of_b5() {
    let session = Session::new()
        .with("SOLITE_PASSWORD", PASSWORD)
        .with("FAKE_LEAK", PASSWORD)
        .with("SOLITE_SESSION_TEST_HOST", "1")
        .with("SOLITE_SESSION_RATE_WAIT", "0");
    let out = session.flash(&[]);
    let status = out.ko("B5");
    assert!(
        status.contains("the password is in the serial log"),
        "{status}"
    );
    assert!(!out.text.contains(PASSWORD), "{}", out.text);
}

#[test]
fn address_names_the_box_the_update_group_talks_to() {
    let session = Session::new()
        .with("SOLITE_SESSION_TEST_HOST", "1")
        .with("SOLITE_SESSION_RATE_WAIT", "0");
    let closed = format!("127.0.0.1:{}", free_port());
    let out = session.flash(&["--address", &closed]);
    // The update group went to the address given (nothing answers there), not to the one saves printed.
    let status = out.ko("B8");
    assert!(status.contains("1 of 1 checks"), "{status}");
    assert!(
        out.status("B8.0").contains("login refused (000)"),
        "{}",
        out.text
    );
    out.ok("B7");
}

/// An account in a data folder, as a box that was set up before: a host serves it, the account is
/// made through the API, and the host is stopped the way the box's restart does.
fn seed_account(data: &Path, password: &str) {
    let port = free_port();
    let mut child = serve_into(data, port);
    let body = format!(r#"{{"username":"admin","password":"{password}","displayName":"admin"}}"#);
    let answer = curl(&[
        "-H",
        "Content-Type: application/json",
        "-d",
        &body,
        &format!("http://127.0.0.1:{port}/api/v1/auth/setup"),
    ]);
    assert!(answer.contains("accessToken"), "{answer}");
    // Ctrl-C: the host saves what it holds before it ends.
    let _ = Command::new("kill")
        .args(["-INT", &child.0.id().to_string()])
        .status();
    let _ = child.0.wait();
}

#[test]
fn a_known_account_with_a_wrong_password_is_ko_and_the_later_steps_have_no_account() {
    let session = Session::new().with("SOLITE_SESSION_TEST_HOST", "1");
    seed_account(&session.root().join("data"), PASSWORD);
    fs::write(session.root().join("password"), "not-the-box-password\n").unwrap();
    let out = session.flash(&["--factory-reset"]);
    assert_eq!(out.code, 1, "{}", out.text);
    assert_eq!(
        out.status("B4.2"),
        "KO (the password does not open the box's account)"
    );
    out.ko("B4");
    for id in ["B6", "B7", "B8", "B10"] {
        out.skipped(id, "after B4: no account");
    }
}

#[test]
fn a_403_confirm_required_at_setup_skips_the_steps_that_need_an_account() {
    let handler: Handler = Arc::new(|r: &Req| match (r.method.as_str(), r.path.as_str()) {
        ("GET", "/api/v1/auth/status") => (200, r#"{"setupRequired":true}"#.into()),
        ("POST", "/api/v1/auth/setup") => (
            403,
            r#"{"error":"Press the button on the box, then retry","confirmRequired":true}"#.into(),
        ),
        _ => (401, r#"{"error":"Authentication required"}"#.into()),
    });
    let port = fake_server(handler);
    let session = Session::new()
        .with("FAKE_ADDRESS", &format!("127.0.0.1:{port}"))
        .with("SOLITE_SESSION_TEST_HOST", "1");
    let out = session.flash(&["--factory-reset"]);
    assert!(
        out.ko("B4.2").contains("Press the button on the box"),
        "{}",
        out.text
    );
    for id in ["B6", "B7", "B8", "B10"] {
        out.skipped(id, "after B4: no account");
    }
}

#[test]
fn a_password_in_the_environment_sets_up_the_account_and_measure_api_logs_in_with_it() {
    let session = Session::new()
        .with("SOLITE_PASSWORD", PASSWORD)
        .with("SOLITE_SESSION_TEST_HOST", "1")
        .with("SOLITE_SESSION_RATE_WAIT", "0");
    let out = session.flash(&[]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert_eq!(out.status("B4.2"), "OK (account set up)");
    let b6 = out.status("B6");
    assert!(b6.contains("17 answered 201"), "{b6}");
    // The password is the user's: no password file, no password on the verdict line.
    assert!(!session.root().join("password").exists());
    assert!(
        !out.last_line().contains("password:"),
        "{}",
        out.last_line()
    );
    assert!(!out.text.contains(PASSWORD), "{}", out.text);
}

#[test]
fn the_gesture_steps_are_skipped_with_what_a_person_does_unless_asked() {
    let session = Session::new();
    let out = session.run(&[], Tty::None);
    for n in 1..=11 {
        let status = out.status(&format!("H{n}"));
        assert!(
            status.starts_with("skipped (needs a person: "),
            "H{n}: {status}"
        );
    }
}

#[test]
fn asked_for_the_gestures_with_no_terminal_each_is_skipped_for_it() {
    let session = Session::new();
    let out = session.run(&["--with-gestures"], Tty::None);
    assert_eq!(out.code, 0, "{}", out.text);
    for n in 1..=11 {
        out.skipped(&format!("H{n}"), "no terminal");
    }
}

#[test]
fn api_only_makes_the_key_signs_16_bytes_flips_them_and_leaves_no_key() {
    let session = Session::new();
    let port = free_port().to_string();
    let out = session.run(&["--api-only", &port], Tty::None);
    out.ok("P2");
    assert_eq!(
        out.status("P4"),
        format!("OK (next {})", tree_version_plus(1)),
        "{}",
        out.text
    );
    out.skipped("P3", "box only");
    out.skipped("B1", "box only");
    let run = session.run_dir();
    assert!(!run.join("update.key").exists());
    let signed = fs::read(run.join("next.sup")).unwrap();
    let flipped = fs::read(run.join("bad.sup")).unwrap();
    assert_eq!(signed.len(), 128 + 16);
    assert_eq!(flipped.len(), signed.len());
    let differing: Vec<usize> = (0..signed.len())
        .filter(|i| signed[*i] != flipped[*i])
        .collect();
    assert_eq!(differing, vec![64]);
}

#[test]
fn a_sigterm_during_a_run_leaves_no_key_in_the_run_folder() {
    // A server that accepts and never answers: the run waits on it.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            held.push(stream);
        }
    });
    let session = Session::new();
    let command = session.command(&["--api-only", &port.to_string()], Tty::None);
    // The script itself, so that the signal reaches it.
    let mut direct = Command::new("sh");
    direct.arg(SCRIPT).args(["--api-only", &port.to_string()]);
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            direct.env(key, value);
        }
    }
    let mut child = Reaped(
        direct
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let runs = session.root().join("runs");
    let mut key = None;
    for _ in 0..200 {
        if let Some(found) = fs::read_dir(&runs)
            .unwrap()
            .flatten()
            .map(|e| e.path().join("update.key"))
            .find(|p| p.exists())
        {
            key = Some(found);
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let key = key.expect("the key was made");
    let status = Command::new("kill")
        .arg(child.0.id().to_string())
        .status()
        .unwrap();
    assert!(status.success());
    let _ = child.0.wait();
    assert!(!key.exists(), "the key outlived the run");
}

#[test]
#[ignore = "slow: --api-only waits 61 s for the Mac host's rate limit"]
fn api_only_against_the_garage_runs_every_api_group_and_skips_the_box_only_checks() {
    let host = sim(true, None);
    let session = Session::new();
    let out = session.run(&["--api-only", &host.addr(), "--factory-reset"], Tty::None);
    assert_eq!(out.code, 0, "{}", out.text);
    for id in ["P1", "P2", "P4", "B4", "B6", "B7", "B8", "B10"] {
        out.ok(id);
    }
    out.skipped("B1", "box only");
    out.skipped("B9", "box only");
    let b6 = out.status("B6");
    assert!(b6.contains("17 answered 201"), "{b6}");
    out.skipped("B7.6", "box only");
    out.skipped("B8.2", "this build has no update key");
    for n in 0..=3 {
        out.ok(&format!("B10.{n}"));
    }
}

// ---------------------------------------------------------------------------------------------
// The image sizes (R7)
// ---------------------------------------------------------------------------------------------

fn size_line(bytes: usize) -> String {
    let dir = TempDir::new("size");
    let image = dir.path().join("app.bin");
    fs::write(&image, vec![0u8; bytes]).unwrap();
    let output = Command::new("sh")
        .arg("-c")
        .arg(r#". "$1"; SLOT_BYTES=4194304; fw_size_line box "$2""#)
        .arg("sh")
        .arg(FIRMWARE_LIB)
        .arg(&image)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn an_image_above_75_percent_of_the_slot_gets_a_warning_and_a_small_one_does_not() {
    assert_eq!(
        size_line(3_355_444),
        "box: 3276 KB, 80 % of the 4096 KB ota_0 slot WARNING: above 75 % of the slot"
    );
    assert_eq!(
        size_line(1_048_576),
        "box: 1024 KB, 25 % of the 4096 KB ota_0 slot"
    );
}

// ---------------------------------------------------------------------------------------------
// The update's stack check of the QEMU image (D60)
// ---------------------------------------------------------------------------------------------

/// `fw_check_update_stack` on a log made of `lines`: its exit status and what it printed.
fn update_stack_check(lines: &str) -> (bool, String) {
    let dir = TempDir::new("update-stack");
    let log = dir.path().join("serial.log");
    fs::write(&log, lines).unwrap();
    let output = Command::new("sh")
        .arg("-c")
        .arg(r#". "$1"; fw_check_update_stack "$2""#)
        .arg("sh")
        .arg(FIRMWARE_LIB)
        .arg(&log)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    )
}

const STATUSES: &str = "I (5527) solite_firmware::update_check: update check: valid 200, image changed 400, signature changed 400\n";
/// The pull of a release (spec 039 R10, spec 040 R6.8): its status line with its one resume, and
/// its thread's stack line.
const PULL: &str =
    "I (5557) solite_firmware::update_check: update pull check: status 200, resumes 1\n";

fn pull_stack(unused: u32) -> String {
    format!(
        "I (5557) solite_firmware::job_thread: update-pull: thread stack {unused} of 20480 bytes never used\n"
    )
}

#[test]
fn the_update_check_passes_with_2_kb_unused_and_fails_below_or_on_a_wrong_status() {
    let stack = |unused: u32| {
        format!(
            "I (5527) solite_firmware::job_thread: update: thread stack {unused} of 16384 bytes never used\n"
        )
    };
    let pull = PULL.to_string() + &pull_stack(8336);
    assert_eq!(
        update_stack_check(&(stack(5940) + STATUSES + &pull)),
        (true, String::new())
    );
    assert!(update_stack_check(&(stack(2048) + STATUSES + &pull)).0);
    let (ok, reason) = update_stack_check(&(stack(464) + STATUSES + &pull));
    assert!(!ok);
    assert!(reason.contains("left 464 bytes"), "{reason}");
    let (ok, reason) = update_stack_check(&(STATUSES.to_string() + &pull));
    assert!(!ok);
    assert!(reason.contains("stack line"), "{reason}");
    let wrong = STATUSES.replace("image changed 400", "image changed 200");
    let (ok, reason) = update_stack_check(&(stack(5940) + &wrong + &pull));
    assert!(!ok);
    assert!(reason.contains("went wrong"), "{reason}");
    let (ok, reason) = update_stack_check(&(stack(5940) + &pull));
    assert!(!ok);
    assert!(reason.contains("update check's line"), "{reason}");
}

#[test]
fn the_pull_check_needs_status_200_one_resume_and_1_kb_of_its_stack_unused() {
    let base = |lines: &str| {
        format!(
            "I (5527) solite_firmware::job_thread: update: thread stack 5940 of 16384 bytes never used\n{STATUSES}{lines}"
        )
    };
    // At the margin: passes; one byte below: refused with the numbers.
    assert!(update_stack_check(&base(&(PULL.to_string() + &pull_stack(1024)))).0);
    let (ok, reason) = update_stack_check(&base(&(PULL.to_string() + &pull_stack(1023))));
    assert!(!ok);
    assert!(reason.contains("left 1023 of its 20480 bytes"), "{reason}");
    // The cut download must have been resumed once.
    let (ok, reason) = update_stack_check(&base(
        &(PULL.replace("resumes 1", "resumes 0") + &pull_stack(8336)),
    ));
    assert!(!ok);
    assert!(reason.contains("did not resume"), "{reason}");
    let (ok, reason) = update_stack_check(&base(&PULL.replace("status 200", "status 400")));
    assert!(!ok);
    assert!(reason.contains("pull check went wrong"), "{reason}");
    let (ok, reason) = update_stack_check(&base(&pull_stack(9412)));
    assert!(!ok);
    assert!(reason.contains("pull check's line"), "{reason}");
    let (ok, reason) = update_stack_check(&base(PULL));
    assert!(!ok);
    assert!(
        reason.contains("update-pull thread's stack line"),
        "{reason}"
    );
}

// ---------------------------------------------------------------------------------------------
// The script phase of the QEMU image and the QuickJS search (spec 041)
// ---------------------------------------------------------------------------------------------

const ROOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// The script memory limit of the core, in bytes: the figure the check reads from `limits.rs`.
fn script_limit_bytes() -> u64 {
    let text = fs::read_to_string(format!(
        "{ROOT_DIR}/crates/core/src/recipes/script/limits.rs"
    ))
    .unwrap();
    let line = text
        .lines()
        .find(|l| l.starts_with("pub const SCRIPT_MEMORY_LIMIT: usize = "))
        .unwrap();
    let kb: u64 = line
        .split('=')
        .nth(1)
        .unwrap()
        .trim()
        .trim_end_matches(';')
        .trim_end_matches("* 1024")
        .trim()
        .parse()
        .unwrap();
    kb * 1024
}

/// `fw_check_script_phase` on a log made of `lines`: its exit status and what it printed.
fn script_phase_check(lines: &str) -> (bool, String) {
    let dir = TempDir::new("script-phase");
    let log = dir.path().join("serial.log");
    fs::write(&log, lines).unwrap();
    let output = Command::new("sh")
        .arg("-c")
        .arg(r#"ROOT=$(cd "$3" && pwd); . "$1"; fw_check_script_phase "$2""#)
        .arg("sh")
        .arg(FIRMWARE_LIB)
        .arg(&log)
        .arg(ROOT_DIR)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    )
}

/// The phase's line, and the two lines of the depth script before it (the engine thread's, and the
/// router's create on the `script` thread), with the depth the engine was called at.
fn phase_with_depth(
    internal_min: u32,
    engine: u32,
    script: u32,
    called: (u32, u32),
    post: u32,
) -> String {
    format!(
        "I (8000) solite_firmware::simulated::script_phase: script phase: engine thread, depth 600, engine called at {}\nI (8500) solite_firmware::simulated::script_phase: script phase: script thread, POST status {post}, depth 600, engine called at {}\nI (9000) solite_firmware::simulated::script_phase: recipes: script motion-light 14 KB source; psram +250 KB (peak +300 KB); internal +50 KB (min {internal_min} KB free); engine stack {engine} of 32768 unused; script thread stack {script} of 32768 unused; heap peak 480 KB\n",
        called.0, called.1
    )
}

fn phase_line(internal_min: u32, engine: u32, script: u32) -> String {
    phase_with_depth(internal_min, engine, script, (592, 592), 201)
}

fn sizes_line(peak3: u64) -> String {
    format!(
        "I (9500) solite_firmware::simulated::script_phase: script sizes: motion-light loaded 100000 B, 3 instances 130000 B (peak {peak3} B), 6 instances 160000 B (peak 170000 B)\nI (9600) solite_firmware::simulated::script_phase: script sizes: big-recipe 32142 B of source, loaded 120000 B, 6 instances 190000 B (peak 200000 B)\n"
    )
}

#[test]
fn the_script_phase_needs_its_line_the_internal_ram_and_both_stacks() {
    let limit = script_limit_bytes();
    let fine = phase_line(76, 5824, 8032) + &sizes_line(limit * 70 / 100);
    assert!(
        script_phase_check(&fine).0,
        "{}",
        script_phase_check(&fine).1
    );
    // The 32 KB script's line missing, or its peak over the limit.
    let without_big: String = fine
        .lines()
        .filter(|line| !line.contains("big-recipe"))
        .map(|line| format!("{line}\n"))
        .collect();
    let (ok, reason) = script_phase_check(&without_big);
    assert!(!ok);
    assert!(reason.contains("big-recipe"), "{reason}");
    let over = fine.replace("(peak 200000 B)", &format!("(peak {} B)", limit + 1));
    let (ok, reason) = script_phase_check(&over);
    assert!(!ok);
    assert!(reason.contains("above the"), "{reason}");
    // The line missing.
    let (ok, reason) = script_phase_check(&sizes_line(limit * 70 / 100));
    assert!(!ok);
    assert!(reason.contains("script phase's line"), "{reason}");
    // Internal RAM under 64 KB.
    let (ok, reason) = script_phase_check(&(phase_line(63, 5824, 8032) + &sizes_line(1)));
    assert!(!ok);
    assert!(
        reason.contains("internal RAM minimum free is 63 KB"),
        "{reason}"
    );
    // Each stack at its margin, one byte below.
    assert!(script_phase_check(&(phase_line(64, 2048, 2048) + &sizes_line(1))).0);
    let (ok, reason) = script_phase_check(&(phase_line(76, 2047, 8032) + &sizes_line(1)));
    assert!(!ok);
    assert!(reason.contains("engine thread keeps 2047"), "{reason}");
    let (ok, reason) = script_phase_check(&(phase_line(76, 5824, 2047) + &sizes_line(1)));
    assert!(!ok);
    assert!(reason.contains("script thread keeps 2047"), "{reason}");
}

#[test]
fn the_script_phase_needs_its_heap_peak_at_most_574_kb_with_the_instances_running() {
    let sizes = sizes_line(1);
    let line = phase_line(76, 5824, 8032);
    let with = |kb: &str| line.replace("heap peak 480 KB", &format!("heap peak {kb} KB"));
    assert!(script_phase_check(&(with("574") + &sizes)).0);
    let (ok, reason) = script_phase_check(&(with("575") + &sizes));
    assert!(!ok);
    assert!(reason.contains("heap peaks at 575 KB"), "{reason}");
    // The figure missing: the line of an image that does not measure it.
    let (ok, reason) = script_phase_check(&(line.replace("; heap peak 480 KB", "") + &sizes));
    assert!(!ok);
    assert!(reason.contains("cannot read the heap peak"), "{reason}");
}

#[test]
fn the_script_phase_needs_the_depth_script_to_reach_the_engine_on_both_threads_and_a_201() {
    let sizes = sizes_line(1);
    // A stack that is unused because the recursion never got to call the engine proves nothing.
    let (ok, reason) =
        script_phase_check(&(phase_with_depth(76, 5824, 8032, (0, 592), 201) + &sizes));
    assert!(!ok);
    assert!(
        reason.contains("engine called at 0") && reason.contains("engine thread"),
        "{reason}"
    );
    let (ok, reason) =
        script_phase_check(&(phase_with_depth(76, 5824, 8032, (592, 0), 201) + &sizes));
    assert!(!ok);
    assert!(
        reason.contains("engine called at 0") && reason.contains("script thread"),
        "{reason}"
    );
    // The router's create on the script thread did not answer 201.
    let (ok, reason) =
        script_phase_check(&(phase_with_depth(76, 5824, 8032, (592, 592), 500) + &sizes));
    assert!(!ok);
    assert!(reason.contains("answered 500, not 201"), "{reason}");
    // The lines missing.
    let line_only = phase_line(76, 5824, 8032)
        .lines()
        .filter(|l| l.contains("recipes: script"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
        + &sizes;
    let (ok, reason) = script_phase_check(&line_only);
    assert!(!ok);
    assert!(
        reason.contains("cannot read where the depth script called"),
        "{reason}"
    );
}

#[test]
fn the_three_instances_peak_passes_at_75_percent_of_the_limit_and_fails_above() {
    let limit = script_limit_bytes();
    let at = limit * 75 / 100;
    assert!(script_phase_check(&(phase_line(76, 5824, 8032) + &sizes_line(at))).0);
    let (ok, reason) = script_phase_check(&(phase_line(76, 5824, 8032) + &sizes_line(at + 1)));
    assert!(!ok);
    assert!(reason.contains("above 75 %"), "{reason}");
    let (ok, reason) = script_phase_check(&phase_line(76, 5824, 8032));
    assert!(!ok);
    assert!(reason.contains("sizes line"), "{reason}");
}

/// `fw_check_quickjs` on an ELF made of `bytes`, with no `nm` in the (fake) ESP-IDF tree, so the
/// search reads the file.
fn quickjs_check(bytes: &[u8], want: &str) -> (bool, String) {
    let dir = TempDir::new("quickjs");
    let elf = dir.path().join("solite-firmware");
    fs::write(&elf, bytes).unwrap();
    let output = Command::new("sh")
        .arg("-c")
        .arg(r#"ROOT=/nonexistent; ESPRESSIF="$4"; . "$1"; fw_check_quickjs "$2" box "$3""#)
        .arg("sh")
        .arg(FIRMWARE_LIB)
        .arg(&elf)
        .arg(want)
        .arg(dir.path())
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    )
}

#[test]
fn an_image_with_quickjs_is_refused_where_there_must_be_none_and_the_other_way_round() {
    let with = b"\x7fELF....JS_NewRuntime....";
    let without = b"\x7fELF....nothing of the kind....";
    assert!(quickjs_check(without, "without").0);
    assert!(quickjs_check(with, "with").0);
    let (ok, reason) = quickjs_check(with, "without");
    assert!(!ok);
    assert!(reason.contains("box image is with QuickJS"), "{reason}");
    let (ok, reason) = quickjs_check(without, "with");
    assert!(!ok);
    assert!(reason.contains("without QuickJS"), "{reason}");
}

// ---------------------------------------------------------------------------------------------
// The script group (spec 041 R7, spec 046): the wave checked on the garage's own motion-light
// ---------------------------------------------------------------------------------------------

/// How the fake box's light behaves.
#[derive(Clone, Copy, PartialEq)]
enum Light {
    /// On at the first look, off from the third.
    Follows,
    /// Never on.
    NeverOn,
    /// On, and never off.
    NeverOff,
}

/// What the fake box saw and holds: the calls, and the recipe (installed by an upload or not).
#[derive(Default)]
struct ScriptBox {
    calls: Vec<String>,
    looks: u32,
    /// `motion-light` was installed by an upload (spec 042 R7.6), and the body it came with.
    installed: bool,
    uploaded: Option<Vec<u8>>,
    /// The content type of the upload.
    upload_type: Option<String>,
    /// The status the upload answers (201 by default).
    upload_status: Option<u16>,
    /// The garage instance's timeout, as written in its params ("10m" by default).
    timeout: Option<&'static str>,
}

fn script_fake(
    light: Light,
    has_garage: bool,
    has_recipe: bool,
    state: Arc<Mutex<ScriptBox>>,
) -> Handler {
    Arc::new(move |r: &Req| {
        let mut st = state.lock().unwrap();
        st.calls.push(format!("{} {}", r.method, r.path));
        match (r.method.as_str(), r.path.as_str()) {
            ("POST", "/api/v1/auth/login") => (200, tokens(1)),
            ("GET", "/api/v1/auth/status") => (200, r#"{"setupRequired":false}"#.into()),
            ("POST", "/api/v1/integrations/solite/permit-join") => {
                (202, r#"{"permitJoin":false,"remaining":0}"#.into())
            }
            ("POST", "/api/v1/recipes/install") => {
                st.uploaded = Some(r.body.clone());
                st.upload_type = r.header("content-type").map(str::to_string);
                match st.upload_status {
                    Some(status) if status != 201 => (
                        status,
                        r#"{"error":"Recipe installs are not configured"}"#.into(),
                    ),
                    _ => {
                        st.installed = true;
                        (201, r#"{"id":"motion-light","version":"1.1.2"}"#.into())
                    }
                }
            }
            ("POST", "/api/v1/recipes/motion-light/uninstall") => {
                st.installed = false;
                (204, String::new())
            }
            ("GET", "/api/v1/recipes") => {
                if has_recipe || st.installed {
                    (
                        200,
                        r#"[{"id":"motion-light","name":"Motion Light"}]"#.into(),
                    )
                } else {
                    (200, "[]".into())
                }
            }
            ("GET", "/api/v1/recipe-instances") => {
                let timeout = st.timeout.unwrap_or("10m");
                let garage = if has_garage {
                    format!(
                        r#"{{"id":"g1","recipeId":"motion-light","enabled":true,"params":{{"zone":"z1","lights":["L1","L2"],"buttons":["B1"],"luxThreshold":50,"timeout":"{timeout}","maxOnDuration":"1h"}}}}"#
                    )
                } else {
                    String::new()
                };
                (200, format!("[{garage}]"))
            }
            ("GET", "/api/v1/equipments/L1") => {
                st.looks += 1;
                let on = match light {
                    Light::Follows => st.looks <= 2,
                    Light::NeverOn => false,
                    Light::NeverOff => true,
                };
                let value = if on { r#""ON""# } else { r#""OFF""# };
                (
                    200,
                    format!(
                        r#"{{"id":"L1","dataBindings":[{{"alias":"state","category":"light_state","value":{value}}}]}}"#
                    ),
                )
            }
            _ => (404, r#"{"error":"Not found"}"#.into()),
        }
    })
}

fn script_group(handler: Handler, extra: &[(&str, &str)]) -> (Out, TempDir) {
    let port = fake_server(handler);
    let mut envs = vec![
        ("SOLITE_PASSWORD", PASSWORD),
        ("SOLITE_SESSION_WAVE", "auto"),
        ("SOLITE_SESSION_ON_WAIT", "2"),
        ("SOLITE_SESSION_OFF_WAIT", "2"),
        ("SOLITE_SESSION_SCRIPT_WAIT", "0"),
    ];
    envs.extend_from_slice(extra);
    group("script", &format!("127.0.0.1:{port}"), &envs)
}

/// No call that changes the home: nothing is created, disabled, enabled, deleted or uninstalled.
fn changes_nothing(calls: &str) {
    for change in [
        "POST /api/v1/recipe-instances",
        "DELETE",
        "disable",
        "enable",
        "uninstall",
    ] {
        assert!(!calls.contains(change), "{change}: {calls}");
    }
}

#[test]
fn the_script_group_checks_the_wave_on_the_garages_own_instance_and_changes_nothing() {
    let state = Arc::new(Mutex::new(ScriptBox::default()));
    let (out, _dir) = script_group(script_fake(Light::Follows, true, true, state.clone()), &[]);
    assert_eq!(out.code, 0, "{}", out.text);
    out.ok("B11.1");
    out.ok("B11.2");
    out.ok("B11.3");
    out.ok("B11.4");
    out.ok("B11.5");
    // The instance's own timeout (10 minutes) is read, nothing is copied: the garage is not touched.
    assert!(out.text.contains("timeout 600s"), "{}", out.text);
    assert!(!out.text.contains("B11.7"), "{}", out.text);
    let st = state.lock().unwrap();
    assert!(st.uploaded.is_none());
    changes_nothing(&st.calls.join("\n"));
}

#[test]
fn a_timeout_the_group_cannot_read_skips_the_off_check_with_a_line() {
    let state = Arc::new(Mutex::new(ScriptBox {
        timeout: Some("1h30m"),
        ..ScriptBox::default()
    }));
    // No override of the wait: the timeout is what sets it.
    let (out, _dir) = script_group(
        script_fake(Light::NeverOff, true, true, state.clone()),
        &[("SOLITE_SESSION_OFF_WAIT", "")],
    );
    assert_eq!(out.code, 0, "{}", out.text);
    out.ok("B11.2");
    out.ok("B11.3");
    out.skipped(
        "B11.4",
        "the instance's timeout cannot be read, so how long to wait is unknown",
    );
    assert!(out.text.contains("timeout unreadable"), "{}", out.text);
    changes_nothing(&state.lock().unwrap().calls.join("\n"));
}

#[test]
fn a_light_that_never_comes_on_is_ko_and_nothing_is_changed() {
    let state = Arc::new(Mutex::new(ScriptBox::default()));
    let (out, _dir) = script_group(script_fake(Light::NeverOn, true, true, state.clone()), &[]);
    assert_eq!(out.code, 1, "{}", out.text);
    assert!(
        out.ko("B11.3").contains("was not on 2 s after the wave"),
        "{}",
        out.text
    );
    changes_nothing(&state.lock().unwrap().calls.join("\n"));
}

#[test]
fn a_light_that_never_goes_off_is_ko_and_nothing_is_changed() {
    let state = Arc::new(Mutex::new(ScriptBox::default()));
    let (out, _dir) = script_group(script_fake(Light::NeverOff, true, true, state.clone()), &[]);
    assert_eq!(out.code, 1, "{}", out.text);
    out.ok("B11.3");
    assert!(
        out.ko("B11.4")
            .contains("still on 2 s after it was seen on"),
        "{}",
        out.text
    );
    changes_nothing(&state.lock().unwrap().calls.join("\n"));
}

#[test]
fn no_script_recipe_or_no_garage_instance_changes_nothing() {
    let state = Arc::new(Mutex::new(ScriptBox::default()));
    let (out, _dir) = script_group(script_fake(Light::Follows, true, false, state.clone()), &[]);
    // Spec 042: no script is built in; without a file to upload the group says so and changes nothing.
    assert_eq!(out.code, 0, "{}", out.text);
    out.skipped("B11.1", "not installed: --recipe-file");
    changes_nothing(&state.lock().unwrap().calls.join("\n"));
    let state = Arc::new(Mutex::new(ScriptBox::default()));
    let (out, _dir) = script_group(script_fake(Light::Follows, false, true, state.clone()), &[]);
    assert_eq!(out.code, 0, "{}", out.text);
    out.skipped(
        "B11.2",
        "the box has no enabled motion-light instance (the garage's)",
    );
    changes_nothing(&state.lock().unwrap().calls.join("\n"));
}

#[test]
fn the_step_is_skipped_without_the_gestures_or_a_terminal() {
    let session = Session::new();
    let out = session.run(&[], Tty::None);
    out.skipped("B11", "needs a person: --with-gestures");
    let out = session.run(&["--with-gestures"], Tty::None);
    out.skipped("B11", "no terminal");
}

/// A signed recipe file of the tests: any bytes, the fake box does not read them.
fn recipe_file(dir: &Path) -> PathBuf {
    let file = dir.join("motion-light.srs");
    fs::write(
        &file,
        b"SOLITERS and some bytes the fake box never reads\x00\x01\x02",
    )
    .unwrap();
    file
}

#[test]
fn a_recipe_file_is_uploaded_when_the_recipe_is_not_installed_and_kept_while_an_instance_uses_it() {
    let dir = TempDir::new("recipe-file");
    let file = recipe_file(dir.path());
    let state = Arc::new(Mutex::new(ScriptBox::default()));
    let (out, _dir) = script_group(
        script_fake(Light::Follows, true, false, state.clone()),
        &[("SOLITE_SESSION_RECIPE_FILE", file.to_str().unwrap())],
    );
    assert_eq!(out.code, 0, "{}", out.text);
    out.ok("B11.1");
    out.ok("B11.4");
    out.ok("B11.7");
    let st = state.lock().unwrap();
    assert_eq!(st.uploaded.as_deref(), Some(&fs::read(&file).unwrap()[..]));
    assert_eq!(st.upload_type.as_deref(), Some("application/octet-stream"));
    // The garage's instance uses the recipe: it is never uninstalled.
    assert!(st.installed, "kept: an instance uses it");
    let calls = st.calls.join("\n");
    assert!(!calls.contains("uninstall"), "{calls}");
}

#[test]
fn an_uploaded_recipe_no_instance_uses_is_uninstalled_again() {
    let dir = TempDir::new("recipe-file");
    let file = recipe_file(dir.path());
    let state = Arc::new(Mutex::new(ScriptBox::default()));
    let (out, _dir) = script_group(
        script_fake(Light::Follows, false, false, state.clone()),
        &[("SOLITE_SESSION_RECIPE_FILE", file.to_str().unwrap())],
    );
    assert_eq!(out.code, 0, "{}", out.text);
    out.ok("B11.1");
    out.skipped(
        "B11.2",
        "the box has no enabled motion-light instance (the garage's)",
    );
    out.ok("B11.7");
    let st = state.lock().unwrap();
    assert!(!st.installed, "no instance uses it: taken away again");
    assert!(
        st.calls
            .join("\n")
            .contains("POST /api/v1/recipes/motion-light/uninstall")
    );
}

#[test]
fn a_recipe_already_installed_is_neither_uploaded_nor_uninstalled() {
    let dir = TempDir::new("recipe-file");
    let file = recipe_file(dir.path());
    let state = Arc::new(Mutex::new(ScriptBox::default()));
    let (out, _dir) = script_group(
        script_fake(Light::Follows, true, true, state.clone()),
        &[("SOLITE_SESSION_RECIPE_FILE", file.to_str().unwrap())],
    );
    assert_eq!(out.code, 0, "{}", out.text);
    let st = state.lock().unwrap();
    assert!(st.uploaded.is_none());
    assert!(!st.calls.join("\n").contains("uninstall"));
}

#[test]
fn a_recipe_file_the_box_refuses_or_that_is_missing_is_ko_and_nothing_is_changed() {
    let dir = TempDir::new("recipe-file");
    let file = recipe_file(dir.path());
    let state = Arc::new(Mutex::new(ScriptBox::default()));
    state.lock().unwrap().upload_status = Some(503);
    let (out, _dir) = script_group(
        script_fake(Light::Follows, true, false, state.clone()),
        &[("SOLITE_SESSION_RECIPE_FILE", file.to_str().unwrap())],
    );
    assert_eq!(out.code, 1, "{}", out.text);
    assert!(
        out.ko("B11.1")
            .contains("answered 503: Recipe installs are not configured"),
        "{}",
        out.text
    );
    assert!(!state.lock().unwrap().calls.join("\n").contains("uninstall"));
    // A file that is not there.
    let state = Arc::new(Mutex::new(ScriptBox::default()));
    let (out, _dir) = script_group(
        script_fake(Light::Follows, true, false, state),
        &[("SOLITE_SESSION_RECIPE_FILE", "/nonexistent/recipe.srs")],
    );
    assert_eq!(out.code, 1, "{}", out.text);
    assert!(out.ko("B11.1").contains("does not exist"), "{}", out.text);
}

#[test]
fn the_recipe_file_option_must_name_a_file() {
    let session = Session::new();
    let out = session.run(&["--recipe-file", "/nonexistent/recipe.srs"], Tty::None);
    assert_eq!(out.code, 2, "{}", out.text);
    assert!(out.text.contains("recipe file not found"), "{}", out.text);
    let out = session.run(&["--recipe-file"], Tty::None);
    assert_eq!(out.code, 2, "{}", out.text);
    assert!(
        out.text.contains("--recipe-file needs a file"),
        "{}",
        out.text
    );
}

// ---------------------------------------------------------------------------------------------
// The recipe install step of the QEMU image and the search for the script's source (spec 042)
// ---------------------------------------------------------------------------------------------

/// `fw_check_recipe_install` on a log made of `lines`: its exit status and what it printed.
fn recipe_install_check(lines: &str) -> (bool, String) {
    let dir = TempDir::new("recipe-install");
    let log = dir.path().join("serial.log");
    fs::write(&log, lines).unwrap();
    let output = Command::new("sh")
        .arg("-c")
        .arg(r#"ROOT=$(cd "$3" && pwd); . "$1"; fw_check_recipe_install "$2""#)
        .arg("sh")
        .arg(FIRMWARE_LIB)
        .arg(&log)
        .arg(ROOT_DIR)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    )
}

fn install_lines(step: &str, unused: &[u32]) -> String {
    install_lines_with_dip(step, unused, 42)
}

fn install_lines_with_dip(step: &str, unused: &[u32], dip_kb: u32) -> String {
    let mut text = format!(
        "I (6900) solite_firmware::simulated::script_phase: script phase: install schedule-on-off: internal RAM dip {dip_kb} KB\n\
         I (7000) solite_firmware::simulated::script_phase: script phase: {step}\n"
    );
    for n in unused {
        text.push_str(&format!(
            "I (7100) solite_firmware::job_thread: recipe: thread stack {n} of 20480 bytes never used\n"
        ));
    }
    text
}

#[test]
fn the_install_step_needs_its_line_its_statuses_and_a_recipe_thread_stack_of_2048_unused() {
    let step = "install 201, boot load ok, uninstall 204";
    let (ok, reason) = recipe_install_check(&install_lines(step, &[6000, 5200]));
    assert!(ok, "{reason}");
    // Exactly the margin passes, one byte under fails, whichever of the jobs it was.
    assert!(recipe_install_check(&install_lines(step, &[2048, 9000])).0);
    let (ok, reason) = recipe_install_check(&install_lines(step, &[9000, 2047]));
    assert!(!ok);
    assert!(
        reason.contains("left 2047 of its 20480 bytes unused"),
        "{reason}"
    );
    // No `recipe: thread stack` line, no step line, a step that went wrong.
    let (ok, reason) = recipe_install_check(&install_lines(step, &[]));
    assert!(!ok);
    assert!(reason.contains("recipe: thread stack"), "{reason}");
    let (ok, reason) = recipe_install_check("I (1) nothing here\n");
    assert!(!ok);
    assert!(reason.contains("install step's line"), "{reason}");
    let (ok, reason) = recipe_install_check(&install_lines(
        "install 409, boot load ok, uninstall 204",
        &[6000],
    ));
    assert!(!ok);
    assert!(reason.contains("the install step went wrong"), "{reason}");
    let (ok, _) = recipe_install_check(&install_lines(
        "install 201, boot load failed, uninstall 204",
        &[6000],
    ));
    assert!(!ok);
    // The internal RAM dip of schedule-on-off's install: 60 KB passes, 61 fails, none fails.
    assert!(recipe_install_check(&install_lines_with_dip(step, &[6000], 60)).0);
    let (ok, reason) = recipe_install_check(&install_lines_with_dip(step, &[6000], 61));
    assert!(!ok);
    assert!(
        reason.contains("took 61 KB of the internal RAM"),
        "{reason}"
    );
    let (ok, reason) = recipe_install_check(&format!(
        "I (7000) x: script phase: {step}\nI (7100) x: recipe: thread stack 6000 of 20480 bytes never used\n"
    ));
    assert!(!ok);
    assert!(reason.contains("internal RAM dip"), "{reason}");
}

/// `fw_check_script_source` on an ELF made of `bytes`, which must be `with` or `without` it.
fn script_source_check(bytes: &[u8], want: &str) -> (bool, String) {
    let dir = TempDir::new("script-source");
    let elf = dir.path().join("firmware.elf");
    fs::write(&elf, bytes).unwrap();
    let output = Command::new("sh")
        .arg("-c")
        .arg(r#". "$1"; fw_check_script_source "$2" radio "$3""#)
        .arg("sh")
        .arg(FIRMWARE_LIB)
        .arg(&elf)
        .arg(want)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    )
}

#[test]
fn an_image_that_carries_the_scripts_source_is_refused_where_there_must_be_none_and_the_other_way_round()
 {
    let with = b"\x7fELF....recipe({ api: 1, id: \"motion-light\", version: \"1.1.2\" })....";
    let without = b"\x7fELF....recipe nothing of the kind....";
    assert!(script_source_check(without, "without").0);
    assert!(script_source_check(with, "with").0);
    let (ok, reason) = script_source_check(with, "without");
    assert!(!ok);
    assert!(
        reason.contains("the radio image is with the source of motion-light"),
        "{reason}"
    );
    let (ok, reason) = script_source_check(without, "with");
    assert!(!ok);
    assert!(reason.contains("without the source"), "{reason}");
}
