//! The full backup through the real server (spec 035): an export of a simulated `--serve`, its
//! restore into another data folder, the restart that applies it, the refusals that close the
//! connection.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const GARAGE: &str = "../../scenarios/garage.json";
const PASSWORD: &str = "test-password-1234";
const BACKUP_PASSWORD: &str = "backup-secret-5678";
const BACKUP: &str = "/api/v1/backup";
/// The home's files and the network's, under the host's names.
const FILES: [&str; 6] = [
    "devices.json",
    "zones.json",
    "equipments.json",
    "recipes.json",
    "settings.json",
    "network.json",
];
static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "solite-backup-e2e-{name}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Serving {
    child: Child,
    port: u16,
    lines: Receiver<String>,
    shown: Vec<String>,
    token: String,
}

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve(data: &Path) -> Serving {
    for _ in 0..5 {
        if let Some(serving) = try_serve(data) {
            return serving;
        }
    }
    panic!("the host could not listen on a free port in five tries");
}

fn try_serve(data: &Path) -> Option<Serving> {
    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_solite-host"))
        .args([
            "--sim",
            GARAGE,
            "--serve",
            &port.to_string(),
            "--speed",
            "10",
            "--data",
        ])
        .arg(data)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the host program starts");
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let (sender, lines) = mpsc::channel();
    thread::spawn(move || {
        for line in stdout.lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let mut serving = Serving {
        child,
        port,
        lines,
        shown: Vec::new(),
        token: String::new(),
    };
    loop {
        match serving.lines.recv_timeout(Duration::from_secs(20)) {
            Ok(line) => {
                let ready = line.starts_with("serving:");
                serving.shown.push(line);
                if ready {
                    serving.sign_in();
                    return Some(serving);
                }
            }
            Err(error) => {
                let _ = serving.child.kill();
                let _ = serving.child.wait();
                let mut stderr = String::new();
                if let Some(mut pipe) = serving.child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                if error == mpsc::RecvTimeoutError::Disconnected
                    && stderr.starts_with("serve: cannot listen on ")
                {
                    return None;
                }
                panic!(
                    "the host stopped or stayed silent: {:?}, {stderr}",
                    serving.shown
                );
            }
        }
    }
}

/// One answer: status, head, body (a chunked body is decoded). The door asks the server to close
/// after each answer, so the connection is read to its end and the answers on it are counted.
struct Answer {
    status: u16,
    head: String,
    body: Vec<u8>,
    answers: usize,
}

impl Answer {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// The bytes of a chunked body, up to its last chunk.
fn dechunk(mut bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let end = bytes
            .windows(2)
            .position(|w| w == b"\r\n")
            .expect("a chunk size");
        let size = usize::from_str_radix(std::str::from_utf8(&bytes[..end]).unwrap().trim(), 16)
            .expect("a hexadecimal chunk size");
        bytes = &bytes[end + 2..];
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&bytes[..size]);
        bytes = &bytes[size + 2..];
    }
}

fn read_answer(stream: &mut TcpStream) -> Answer {
    let mut bytes = Vec::new();
    let _ = stream.read_to_end(&mut bytes);
    let split = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("an HTTP answer");
    let head = String::from_utf8_lossy(&bytes[..split]).into_owned();
    let rest = &bytes[split + 4..];
    let chunked = head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked");
    Answer {
        status: head.split(' ').nth(1).unwrap().parse().unwrap(),
        body: if chunked {
            dechunk(rest)
        } else {
            rest.to_vec()
        },
        answers: String::from_utf8_lossy(&bytes).matches("HTTP/1.1 ").count(),
        head,
    }
}

impl Serving {
    fn sign_in(&mut self) {
        let answer = self.request("GET", "/api/v1/auth/status", &[], "application/json", &[]);
        let fresh = serde_json::from_slice::<Value>(&answer.body).unwrap()["setupRequired"] == true;
        let (route, payload) = if fresh {
            (
                "setup",
                json!({"username": "admin", "password": PASSWORD, "displayName": "Admin"}),
            )
        } else {
            ("login", json!({"username": "admin", "password": PASSWORD}))
        };
        let answer = self.request(
            "POST",
            &format!("/api/v1/auth/{route}"),
            payload.to_string().as_bytes(),
            "application/json",
            &[],
        );
        assert!(
            answer.status == 200 || answer.status == 201,
            "{}",
            answer.text()
        );
        self.token = serde_json::from_slice::<Value>(&answer.body).unwrap()["accessToken"]
            .as_str()
            .unwrap()
            .to_string();
    }

    /// A request, with the token unless `extra` names an `Authorization` of its own.
    fn request(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        content_type: &str,
        extra: &[(&str, &str)],
    ) -> Answer {
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n",
            self.port,
            body.len()
        );
        for (name, value) in extra {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        self.send_raw(head.as_bytes(), body)
    }

    fn authorized(&self, method: &str, path: &str, body: &[u8], content_type: &str) -> Answer {
        let bearer = format!("Bearer {}", self.token);
        self.request(
            method,
            path,
            body,
            content_type,
            &[("Authorization", &bearer)],
        )
    }

    fn send_raw(&self, head: &[u8], body: &[u8]) -> Answer {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        stream.write_all(head).unwrap();
        // The server may answer and close before the whole body is sent: a failed write is fine.
        let _ = stream.write_all(body);
        read_answer(&mut stream)
    }

    fn export(&self, password: &str) -> Answer {
        let bearer = format!("Bearer {}", self.token);
        self.request(
            "GET",
            BACKUP,
            &[],
            "application/json",
            &[("Authorization", &bearer), ("X-Backup-Password", password)],
        )
    }

    fn restore(&self, archive: &[u8], password: &str) -> Answer {
        let bearer = format!("Bearer {}", self.token);
        self.request(
            "POST",
            BACKUP,
            archive,
            "application/octet-stream",
            &[("Authorization", &bearer), ("X-Backup-Password", password)],
        )
    }

    fn wait_for_line(&mut self, wanted: &str) -> String {
        if let Some(line) = self.shown.iter().find(|l| l.contains(wanted)) {
            return line.clone();
        }
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("no line with {wanted:?}: {:?}", self.shown));
            self.shown.push(line.clone());
            if line.contains(wanted) {
                return line;
            }
        }
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(20), "{what}");
        thread::sleep(Duration::from_millis(50));
    }
}

fn read_files(data: &Path) -> Vec<(&'static str, Option<Vec<u8>>)> {
    FILES
        .iter()
        .map(|name| (*name, std::fs::read(data.join(name)).ok()))
        .collect()
}

/// The data folder holds no staged copy, marker or mark.
fn nothing_left(data: &Path) {
    for entry in std::fs::read_dir(data).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            !name.ends_with(".restore")
                && !name.starts_with("restore.")
                && name != "zigbee.restore",
            "left behind: {name}"
        );
    }
}

#[test]
fn an_export_restored_into_another_folder_gives_the_same_files_after_the_restart() {
    let source_data = temp("source");
    let source = serve(&source_data);
    let zone = source.authorized(
        "POST",
        "/api/v1/zones",
        json!({"name": "Cellar"}).to_string().as_bytes(),
        "application/json",
    );
    assert_eq!(zone.status, 201, "{}", zone.text());
    wait_until("zones.json written", || {
        std::fs::read_to_string(source_data.join("zones.json")).is_ok_and(|t| t.contains("Cellar"))
    });
    assert!(source_data.join("network.json").exists());

    let export = source.export(BACKUP_PASSWORD);
    assert_eq!(export.status, 200, "{}", export.text());
    let head = export.head.to_ascii_lowercase();
    assert!(head.contains("content-type: application/octet-stream"));
    assert!(head.contains("content-disposition: attachment; filename=\"solite-backup.slb\""));
    assert!(head.contains("transfer-encoding: chunked"), "{head}");
    assert!(export.body.starts_with(b"SOLITEBK"));
    // The network's key is not in the clear in the file.
    let network = std::fs::read_to_string(source_data.join("network.json")).unwrap();
    let key = serde_json::from_str::<Value>(&network).unwrap()["networkKey"]
        .as_str()
        .map(str::to_string);
    if let Some(key) = key {
        assert!(!String::from_utf8_lossy(&export.body).contains(&key));
    }
    let expected = read_files(&source_data);

    let target_data = temp("target");
    let mut target = serve(&target_data);
    // A wrong password: 400, the connection closed, nothing staged.
    let wrong = target.restore(&export.body, "another-password-1");
    assert_eq!(wrong.status, 400, "{}", wrong.text());
    assert!(wrong.text().contains("Wrong backup password"));
    assert_eq!(wrong.answers, 1, "the archive was read as a request");
    nothing_left(&target_data);

    let done = target.restore(&export.body, BACKUP_PASSWORD);
    assert_eq!(done.status, 200, "{}", done.text());
    let line = target.wait_for_line("backup: restored");
    assert!(
        line.ends_with("files; restart the host to load them"),
        "{line}"
    );
    assert_eq!(done.answers, 1);
    // The files wait, staged, for the restart; a second job is refused until then.
    assert!(target_data.join("restore.pending").exists());
    for name in [
        "restore.pending",
        "network.json.restore",
        "zones.json.restore",
    ] {
        let mode = std::fs::metadata(target_data.join(name))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{name}");
    }
    drop(target);

    // The restart applies it, before the home is read and the radio started.
    let mut again = serve(&target_data);
    again.wait_for_line("network:");
    wait_until("the mark cleared", || {
        !target_data.join("zigbee.restore").exists()
    });
    nothing_left(&target_data);
    for ((name, was), (_, now)) in expected.iter().zip(read_files(&target_data)) {
        if let Some(was) = was {
            assert_eq!(Some(was), now.as_ref(), "{name}");
        }
    }
    let zones = again.authorized("GET", "/api/v1/zones", &[], "application/json");
    assert!(zones.text().contains("Cellar"), "{}", zones.text());
}

#[test]
fn the_restore_mark_is_cleared_once_the_network_is_up_even_when_the_file_does_not_parse() {
    let data = temp("unreadable-network");
    std::fs::write(data.join("network.json"), "not a network").unwrap();
    std::fs::write(data.join("zigbee.restore"), "restore\n").unwrap();
    let mut serving = serve(&data);
    serving.wait_for_line("network:");
    wait_until("the mark cleared", || !data.join("zigbee.restore").exists());
}

#[test]
fn the_refusals_close_the_connection_and_a_length_over_the_bound_is_413_before_any_read() {
    let data = temp("refusals");
    let serving = serve(&data);
    // No token: 401 on both, the body not answered as a request.
    let no_token = serving.request(
        "GET",
        BACKUP,
        &[],
        "application/json",
        &[("X-Backup-Password", BACKUP_PASSWORD)],
    );
    assert_eq!(no_token.status, 401);
    // No or short password.
    let missing = serving.authorized("GET", BACKUP, &[], "application/json");
    assert_eq!(missing.status, 400, "{}", missing.text());
    let short = serving.export("short");
    assert_eq!(short.status, 400, "{}", short.text());
    assert!(short.text().contains("8 to 128"));
    // Garbage with a good password: 400, and the garbage is not read as a request.
    let garbage = serving.restore(&vec![7u8; 3000], BACKUP_PASSWORD);
    assert_eq!(garbage.status, 400, "{}", garbage.text());
    assert_eq!(garbage.answers, 1);
    // Another content type: 415.
    let json_type = serving.request(
        "POST",
        BACKUP,
        &[7u8; 100],
        "application/json",
        &[
            ("Authorization", &format!("Bearer {}", serving.token)),
            ("X-Backup-Password", BACKUP_PASSWORD),
        ],
    );
    assert_eq!(json_type.status, 415);
    assert_eq!(json_type.answers, 1);
    // A declared length over the bound: 413, nothing read.
    let too_long = serving.send_raw(
        format!(
            "POST {BACKUP} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/octet-stream\r\nAuthorization: Bearer {}\r\nX-Backup-Password: {BACKUP_PASSWORD}\r\nContent-Length: 5000000000\r\n\r\n",
            serving.port, serving.token
        )
        .as_bytes(),
        &[],
    );
    assert_eq!(too_long.status, 413, "{}", too_long.text());
    assert_eq!(too_long.answers, 1);
    // The old route (before spec 059) is gone.
    let old = serving.authorized("GET", "/api/v1/system/backup", &[], "application/json");
    assert_eq!(old.status, 404);
    nothing_left(&data);
}
