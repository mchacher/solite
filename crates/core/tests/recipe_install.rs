//! Script recipes installed, updated and removed (spec 042): the signed file, the catalogue, the
//! install, update and uninstall through the routes behind the gate, the one-job rule against the
//! release and backup jobs, the memory rules, the boot load, the safe mode, the factory reset and
//! the resources. Everything is made here: key pairs from random bytes (never a real key), scripts
//! as strings, a fake network, a fake probe and storage in memory.
//!
//! These tests need the `script-recipes` feature (the host program turns it on, so
//! `cargo test --workspace` runs them).

mod mem_storage;
mod support;

use std::cell::Cell;
use std::collections::HashMap;
use std::io::{self, Cursor, ErrorKind, Read};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signer, SigningKey};
use mem_storage::MemStorage;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use solite_core::api::{Reach, Reply, Request, handle_guarded};
use solite_core::auth::{Account, AlwaysConfirmed, Language, PasswordHash};
use solite_core::backup::Pacing;
use solite_core::devices::Id;
use solite_core::engine::{Saver, Storage};
use solite_core::home::Home;
use solite_core::recipes::ScriptClock;
use solite_core::recipes::install::catalogue::{self, DEFAULT_CATALOGUE_URL};
use solite_core::recipes::install::file::{self, MAX_FILE_BYTES};
use solite_core::recipes::install::{
    FETCH_STACK_BYTES, JobEnv, RECIPE_INTERNAL_MARGIN, RecipeInstalls, SCRIPT_PSRAM_RESERVE,
    ScriptThread,
};
use solite_core::recipes::script::{
    NewRuntime, SYSTEM_MEMORY_LIMIT, ScriptRuntimes, SystemRuntimes,
};
use solite_core::recipes::{MAX_SCRIPT_RECIPES, RecipeInstance};
use solite_core::supervisor::{Mode, Status};
use solite_core::update::release::{Fetched, Fetcher, Limits};
use solite_core::update::{HEADER_BYTES, Updates, Version, signed_bytes};
use solite_core::{MemoryProbe, MemoryReading, MemoryStats, PsramStats};
use support::update_fake::FakeAdapter;
use support::{HOST, MOTION_LIGHT, PORT, ROOT, T0, counter};

// ---- Keys, scripts, files ----

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .expect("random bytes");
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn new_key() -> SigningKey {
    SigningKey::from_bytes(&random_bytes::<32>())
}

fn public_hex(signing: &SigningKey) -> String {
    hex(&signing.verifying_key().to_bytes())
}

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

/// A script recipe of this id and version that takes the zone and logs when it starts.
fn script(id: &str, version: &str) -> String {
    format!(
        r#"recipe({{
  api: 1, id: "{id}", name: "{id}", description: "A test recipe", version: "{version}",
  slots: {{ timeout: {{ type: "duration", defaultValue: "10m" }} }},
  start: function (api) {{ api.log("started {version}"); }},
}});"#
    )
}

/// The signed file of a script: the header (R1.1) of this version, then the source.
fn signed_with(signing: &SigningKey, version: &str, source: &str) -> Vec<u8> {
    let length = u32::try_from(source.len()).unwrap();
    let head = signed_bytes_of(v(version), length, &sha256(source.as_bytes()));
    let mut out = head.to_vec();
    out.extend_from_slice(&signing.sign(&head).to_bytes());
    out.extend_from_slice(source.as_bytes());
    assert_eq!(out.len(), HEADER_BYTES + source.len());
    out
}

fn signed_bytes_of(version: Version, length: u32, digest: &[u8; 32]) -> [u8; 64] {
    file::signed_bytes(version, length, digest)
}

/// The signed file of `script(id, version)`.
fn signed(signing: &SigningKey, id: &str, version: &str) -> Vec<u8> {
    signed_with(signing, version, &script(id, version))
}

/// A catalogue entry for a signed file served at `url`.
fn entry(id: &str, version: &str, bytes: &[u8], url: &str) -> Value {
    json!({
        "id": id, "version": version, "name": id, "description": "A test recipe",
        "size": bytes.len(), "sha256": hex(&sha256(bytes)), "url": url,
    })
}

fn catalogue_of(entries: &[Value]) -> Vec<u8> {
    json!({ "format": 1, "recipes": entries })
        .to_string()
        .into_bytes()
}

const CATALOGUE: &str = "https://example.com/recipes.json";
const SECOND: u64 = 1000;

fn file_url(id: &str, version: &str) -> String {
    format!("https://example.com/{id}-{version}.srs")
}

// ---- The fakes ----

/// What a GET of an address gives.
#[derive(Clone)]
enum Answer {
    Body(Vec<u8>),
    Fail(String),
    /// A body cut after `step` bytes, then resumed by ranged requests that are cut after `step`
    /// bytes each, `cuts` times in all; then it ends.
    Cuts {
        data: Vec<u8>,
        step: usize,
        cuts: usize,
    },
}

/// A body that gives its bytes, then fails (a cut connection) or ends.
struct Chunk {
    data: Cursor<Vec<u8>>,
    cut: bool,
}

impl Read for Chunk {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = self.data.read(out)?;
        if count == 0 && self.cut {
            return Err(io::Error::new(ErrorKind::ConnectionReset, "cut"));
        }
        Ok(count)
    }
}

fn body_of(data: Vec<u8>, cut: bool) -> Box<dyn Read> {
    Box::new(Chunk {
        data: Cursor::new(data),
        cut,
    })
}

#[derive(Default)]
struct FakeFetcher {
    answers: HashMap<String, Answer>,
    log: Vec<String>,
    cuts_used: HashMap<String, usize>,
}

impl FakeFetcher {
    fn serve(&mut self, url: &str, answer: Answer) {
        self.answers.insert(url.to_string(), answer);
    }

    fn answer(&mut self, url: &str, from: Option<u64>) -> Result<Fetched, String> {
        match self.answers.get(url).cloned() {
            None => Ok(Fetched::Status(404)),
            Some(Answer::Fail(message)) => Err(message),
            Some(Answer::Body(data)) => Ok(Fetched::Body {
                length: Some(data.len() as u64),
                etag: None,
                body: body_of(data, false),
            }),
            Some(Answer::Cuts { data, step, cuts }) => {
                let total = data.len();
                let used = self.cuts_used.entry(url.to_string()).or_default();
                let cut = *used < cuts;
                match from {
                    None => {
                        *used += 1;
                        Ok(Fetched::Body {
                            length: Some(total as u64),
                            etag: None,
                            body: body_of(data[..step.min(total)].to_vec(), cut),
                        })
                    }
                    Some(from) => {
                        let from = usize::try_from(from).unwrap();
                        let end = if cut { (from + step).min(total) } else { total };
                        *used += 1;
                        Ok(Fetched::Partial {
                            content_range: Some(format!("bytes {from}-{}/{total}", total - 1)),
                            length: Some((total - from) as u64),
                            etag: None,
                            body: body_of(data[from..end].to_vec(), cut && end < total),
                        })
                    }
                }
            }
        }
    }
}

impl Fetcher for FakeFetcher {
    fn get(&mut self, url: &str, _idle_ms: u64) -> Result<Fetched, String> {
        self.log.push(url.to_string());
        self.answer(url, None)
    }

    fn get_range(&mut self, url: &str, from: u64, _idle_ms: u64) -> Result<Fetched, String> {
        self.log.push(format!("{url} from {from}"));
        self.answer(url, Some(from))
    }
}

struct FakePacing {
    clock: Rc<Cell<u64>>,
}

impl Pacing for FakePacing {
    fn now_ms(&mut self) -> u64 {
        self.clock.get()
    }

    fn pause(&mut self) {}

    fn wait_ms(&mut self, ms: u64) {
        self.clock.set(self.clock.get() + ms);
    }
}

/// The memory a fake probe reads, changed by the test.
struct FakeProbe {
    reading: Mutex<Option<MemoryReading>>,
    /// The runtimes `Counting` made, and what each holds in PSRAM while it is alive.
    runtimes: Mutex<Vec<std::sync::Weak<AtomicUsize>>>,
    held_each: AtomicUsize,
}

impl FakeProbe {
    fn new() -> FakeProbe {
        FakeProbe {
            reading: Mutex::new(None),
            runtimes: Mutex::new(Vec::new()),
            held_each: AtomicUsize::new(0),
        }
    }

    fn set(&self, internal_largest: usize, psram_free: usize) {
        *self.reading.lock().unwrap() = Some(MemoryReading {
            stats: MemoryStats {
                internal_free: 150 * 1024,
                internal_largest,
                psram: Some(PsramStats {
                    free: psram_free,
                    total: 2 * 1024 * 1024,
                }),
            },
            internal_min: 90 * 1024,
            psram_min: psram_free.min(1_500_000),
        });
    }
}

impl MemoryProbe for FakeProbe {
    fn read(&self) -> Option<MemoryReading> {
        let mut reading = (*self.reading.lock().unwrap())?;
        // Each runtime still alive takes its share of what `set` gave as free once none is held.
        let alive = self
            .runtimes
            .lock()
            .unwrap()
            .iter()
            .filter(|runtime| runtime.strong_count() > 0)
            .count();
        let taken = alive * self.held_each.load(Ordering::SeqCst);
        if let Some(psram) = reading.stats.psram.as_mut() {
            psram.free = psram.free.saturating_sub(taken);
        }
        Some(reading)
    }
}

/// Runtimes that the probe sees: each one is alive as long as the recipe holds it, and takes
/// `held_each` bytes of the PSRAM the probe reports (a runtime dropped gives them back).
struct Counting {
    probe: Arc<FakeProbe>,
    made: AtomicUsize,
}

impl Counting {
    fn new(probe: &Arc<FakeProbe>, held_each: usize) -> Arc<Counting> {
        probe.held_each.store(held_each, Ordering::SeqCst);
        Arc::new(Counting {
            probe: Arc::clone(probe),
            made: AtomicUsize::new(0),
        })
    }
}

impl ScriptRuntimes for Counting {
    fn new_runtime(&self) -> Result<NewRuntime, String> {
        self.made.fetch_add(1, Ordering::SeqCst);
        let mut made = SystemRuntimes.new_runtime()?;
        let counter = Arc::new(AtomicUsize::new(0));
        self.probe
            .runtimes
            .lock()
            .unwrap()
            .push(Arc::downgrade(&counter));
        made.held = Some(counter);
        Ok(made)
    }

    fn memory_limit(&self) -> usize {
        SystemRuntimes.memory_limit()
    }
}

/// Runs a job's steps on the script thread, remembering when it does.
struct Hops {
    inside: Rc<Cell<bool>>,
    count: Cell<usize>,
}

impl ScriptThread for Hops {
    fn run(&self, job: &mut dyn FnMut()) {
        self.inside.set(true);
        self.count.set(self.count.get() + 1);
        job();
        self.inside.set(false);
    }
}

/// The storage of the world: the flash is never touched from the script thread (R6.3).
struct Guarded<'a> {
    inner: &'a MemStorage,
    inside: &'a Cell<bool>,
}

impl Storage for Guarded<'_> {
    fn read(&self, name: &str) -> Result<Option<String>, String> {
        self.inner.read(name)
    }

    fn write(
        &self,
        name: &str,
        next: &mut dyn FnMut() -> io::Result<Option<String>>,
    ) -> Result<(), String> {
        self.inner.write(name, next)
    }

    fn locate(&self, name: &str) -> String {
        self.inner.locate(name)
    }

    fn read_bytes(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        assert!(!self.inside.get(), "a flash read on the script thread");
        self.inner.read_bytes(name)
    }

    fn write_bytes(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        assert!(!self.inside.get(), "a flash write on the script thread");
        self.inner.write_bytes(name, bytes)
    }

    fn remove(&self, name: &str) -> Result<(), String> {
        assert!(!self.inside.get(), "a flash removal on the script thread");
        self.inner.remove(name)
    }

    fn list(&self, folder: &str) -> Result<Vec<String>, String> {
        self.inner.list(folder)
    }

    fn usage(&self) -> Result<(u64, Option<u64>), String> {
        self.inner.usage()
    }
}

/// Runtimes that stop being made on demand: the old file of an update that does not load again.
struct Flaky {
    calls: AtomicUsize,
    fail_from: AtomicUsize,
}

impl Flaky {
    fn new() -> Arc<Flaky> {
        Arc::new(Flaky {
            calls: AtomicUsize::new(0),
            fail_from: AtomicUsize::new(usize::MAX),
        })
    }

    /// From now on no runtime is made.
    fn fail_from_now(&self) {
        self.fail_from
            .store(self.calls.load(Ordering::SeqCst), Ordering::SeqCst);
    }
}

impl ScriptRuntimes for Flaky {
    fn new_runtime(&self) -> Result<NewRuntime, String> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n >= self.fail_from.load(Ordering::SeqCst) {
            return Err("no memory".to_string());
        }
        SystemRuntimes.new_runtime()
    }

    fn memory_limit(&self) -> usize {
        SystemRuntimes.memory_limit()
    }
}

// ---- The world of a test ----

const CLOCK_START: u64 = 1_000_000;
const OCTET: &str = "application/octet-stream";
const JSON: &str = "application/json";

struct World {
    home: Mutex<Home>,
    random: Mutex<support::Random>,
    token: String,
    now: Cell<u64>,
    signing: SigningKey,
    storage: MemStorage,
    fetch: FakeFetcher,
    clock: Rc<Cell<u64>>,
    probe: Arc<FakeProbe>,
    runtimes: Arc<dyn ScriptRuntimes>,
    hops: Hops,
}

impl World {
    fn new() -> World {
        let signing = new_key();
        let key = public_hex(&signing);
        World::build(signing, Some(&key), T0)
    }

    fn with_key_text(text: Option<&str>) -> World {
        World::build(new_key(), text, T0)
    }

    fn at(now: u64) -> World {
        let signing = new_key();
        let key = public_hex(&signing);
        World::build(signing, Some(&key), now)
    }

    /// A world over a home a test made (devices, zones, equipments, saved instances), with the
    /// recipe key and the account as `new` gives them.
    fn over(home: Home, now: u64) -> World {
        let signing = new_key();
        let key = public_hex(&signing);
        World::build_with(home, signing, Some(&key), now)
    }

    /// The program keeps the recipe job's thread from the start, as the box does (spec 045).
    fn keep_job_thread(&mut self) {
        let mut home = self.home.lock().unwrap();
        let mut installs = home.recipe_installs().clone();
        installs.set_job_thread_kept(true);
        home.set_recipe_installs(installs);
    }

    fn build(signing: SigningKey, key: Option<&str>, now: u64) -> World {
        World::build_with(Home::new(T0), signing, key, now)
    }

    fn build_with(mut home: Home, signing: SigningKey, key: Option<&str>, now: u64) -> World {
        let probe = Arc::new(FakeProbe::new());
        let (installs, line) = RecipeInstalls::from_key_text(key, probe.clone());
        home.set_recipe_installs(installs);
        home.set_presence(AlwaysConfirmed);
        home.load_auth(Some(Account {
            id: Id::parse("11111111-1111-4111-8111-111111111111").unwrap(),
            username: "admin".into(),
            display_name: "Owner".into(),
            language: Language::En,
            created_at: T0,
            password: PasswordHash::new("correct horse battery", [7; 16]),
        }));
        let _ = line;
        let mut world = World {
            home: Mutex::new(home),
            random: Mutex::new(counter()),
            token: String::new(),
            now: Cell::new(now),
            signing,
            storage: MemStorage::default(),
            fetch: FakeFetcher::default(),
            clock: Rc::new(Cell::new(CLOCK_START)),
            probe,
            runtimes: Arc::new(SystemRuntimes),
            hops: Hops {
                inside: Rc::new(Cell::new(false)),
                count: Cell::new(0),
            },
        };
        let login = br#"{"username":"admin","password":"correct horse battery"}"#;
        let reply = world.send("POST", "/api/v1/auth/login", None, Some(JSON), login, None);
        let (status, answer) = whole(reply);
        assert_eq!(status, 200, "{answer}");
        world.token = answer["accessToken"].as_str().unwrap().to_string();
        world
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Home> {
        self.home.lock().unwrap()
    }

    fn script_clock(&self) -> ScriptClock {
        Arc::new(|| 0)
    }

    /// Loads the files of the storage as the start does.
    fn boot(&mut self) -> Vec<String> {
        let clock = self.script_clock();
        let runtimes = Arc::clone(&self.runtimes);
        self.lock()
            .load_installed_scripts(&&self.storage, &runtimes, &clock)
    }

    fn send(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        content_type: Option<&str>,
        body: &[u8],
        content_length: Option<u64>,
    ) -> Reply {
        let authorization = token.map(|t| format!("Bearer {t}"));
        let mut random = |buffer: &mut [u8]| (self.random.lock().unwrap())(buffer);
        handle_guarded(
            &self.home,
            &Request {
                method,
                path,
                host: Some(HOST),
                port: PORT,
                reach: Reach::Loopback,
                content_type,
                authorization: authorization.as_deref(),
                content_length,
                body,
            },
            // The wall clock moves with the fake monotonic one (a refresh is paced on the wall).
            self.now.get() + (self.clock.get() - CLOCK_START),
            &mut random,
        )
    }

    /// A request of the session; the reply, not yet run.
    fn ask(&self, method: &str, path: &str, content_type: Option<&str>, body: &[u8]) -> Reply {
        let token = self.token.clone();
        self.send(
            method,
            path,
            Some(&token),
            content_type,
            body,
            Some(body.len() as u64),
        )
    }

    /// Runs the reply the core gave, as a program does: a job or a refresh on a thread of its own.
    fn run(&mut self, reply: Reply) -> (u16, Value) {
        match reply {
            Reply::RecipeJob(ticket) | Reply::CatalogueRefresh(ticket) => {
                let guarded = Guarded {
                    inner: &self.storage,
                    inside: &self.hops.inside,
                };
                let mut pacing = FakePacing {
                    clock: Rc::clone(&self.clock),
                };
                let mut env = JobEnv {
                    fetcher: &mut self.fetch,
                    pacing: &mut pacing,
                    storage: &guarded,
                    runtimes: Arc::clone(&self.runtimes),
                    clock: Arc::new(|| 0),
                    script_thread: &self.hops,
                    check_limits: Limits {
                        idle_ms: 100,
                        total_ms: 5 * SECOND,
                    },
                    download_limits: Limits {
                        idle_ms: 100,
                        total_ms: 120 * SECOND,
                    },
                };
                let response = ticket.run(&self.home, &mut env);
                parse(response.status, response.body)
            }
            Reply::List(mut cursor) => {
                let mut text = String::new();
                while let Some(chunk) = cursor.next_chunk(&self.lock()).unwrap() {
                    text.push_str(&chunk);
                }
                (200, serde_json::from_str(&text).unwrap())
            }
            other => whole(other),
        }
    }

    fn call(
        &mut self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: &[u8],
    ) -> (u16, Value) {
        let reply = self.ask(method, path, content_type, body);
        self.run(reply)
    }

    fn get(&mut self, path: &str) -> (u16, Value) {
        self.call("GET", path, None, b"")
    }

    fn post(&mut self, path: &str) -> (u16, Value) {
        self.call("POST", path, None, b"")
    }

    fn put_json(&mut self, path: &str, body: Value) -> (u16, Value) {
        self.call("PUT", path, Some(JSON), body.to_string().as_bytes())
    }

    /// After a factory reset: the password set again, and the new session kept.
    fn relogin(&mut self) {
        let body =
            br#"{"username":"admin","password":"correct horse battery","displayName":"Owner"}"#;
        let reply = self.send("POST", "/api/v1/auth/setup", None, Some(JSON), body, None);
        let (status, answer) = whole(reply);
        assert_eq!(status, 201, "{answer}");
        self.token = answer["accessToken"].as_str().unwrap().to_string();
    }

    fn install_id(&mut self, id: &str) -> (u16, Value) {
        self.call(
            "POST",
            "/api/v1/plugins/install",
            Some(JSON),
            json!({ "repo": format!("solite-recipes/{id}") })
                .to_string()
                .as_bytes(),
        )
    }

    fn upload(&mut self, bytes: &[u8]) -> (u16, Value) {
        self.call("POST", "/api/v1/plugins/install", Some(OCTET), bytes)
    }

    fn update(&mut self, id: &str) -> (u16, Value) {
        self.post(&format!("/api/v1/plugins/{id}/update"))
    }

    fn uninstall(&mut self, id: &str) -> (u16, Value) {
        self.post(&format!("/api/v1/plugins/{id}/uninstall"))
    }

    fn refresh(&mut self) -> (u16, Value) {
        self.post("/api/v1/plugins/store/refresh")
    }

    /// Serves the catalogue and the files of these recipes: `(id, version, signed file)`.
    fn serve(&mut self, recipes: &[(&str, &str, &[u8])]) {
        let entries: Vec<Value> = recipes
            .iter()
            .map(|(id, version, bytes)| entry(id, version, bytes, &file_url(id, version)))
            .collect();
        self.fetch
            .serve(DEFAULT_CATALOGUE_URL, Answer::Body(catalogue_of(&entries)));
        self.fetch
            .serve(CATALOGUE, Answer::Body(catalogue_of(&entries)));
        for (id, version, bytes) in recipes {
            self.fetch
                .serve(&file_url(id, version), Answer::Body(bytes.to_vec()));
        }
    }

    /// Serves a new catalogue and reads it (a minute after the last read).
    fn publish(&mut self, recipes: &[(&str, &str, &[u8])]) {
        self.serve(recipes);
        self.clock.set(self.clock.get() + 61 * SECOND);
        let (status, body) = self.refresh();
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["source"], "remote", "{body}");
    }

    /// Installs a recipe from the catalogue, and checks that it went through.
    fn installed(&mut self, id: &str, version: &str) {
        let bytes = signed(&self.signing, id, version);
        self.serve(&[(id, version, &bytes)]);
        let (status, body) = self.install_id(id);
        assert_eq!((status, &body["success"]), (200, &json!(true)), "{body}");
    }

    /// Creates an instance of a recipe in the root zone; its id.
    fn instance(&mut self, recipe: &str) -> String {
        let (status, body) = self.call(
            "POST",
            "/api/v1/recipe-instances",
            Some(JSON),
            json!({ "recipeId": recipe, "params": { "zone": ROOT } })
                .to_string()
                .as_bytes(),
        );
        assert_eq!(status, 201, "{body}");
        body["id"].as_str().unwrap().to_string()
    }

    fn running(&self, instance: &str) -> bool {
        self.lock()
            .recipes()
            .is_running(&Id::parse(instance).unwrap())
    }

    fn messages(&mut self, instance: &str) -> Vec<String> {
        let (status, body) = self.get(&format!("/api/v1/recipe-instances/{instance}/log"));
        assert_eq!(status, 200);
        body.as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["message"].as_str().unwrap().to_string())
            .collect()
    }

    fn file(&self, id: &str) -> Option<Vec<u8>> {
        self.storage.file(&format!("scripts/{id}.srs"))
    }

    fn writes(&self, id: &str) -> usize {
        self.storage.byte_writes_of(&format!("scripts/{id}.srs"))
    }

    fn version_of(&self, id: &str) -> Option<String> {
        self.lock()
            .recipes()
            .installed_version(id)
            .map(|v| v.to_string())
    }
}

fn parse(status: u16, body: Option<String>) -> (u16, Value) {
    (
        status,
        body.map_or(Value::Null, |text| serde_json::from_str(&text).unwrap()),
    )
}

fn whole(reply: Reply) -> (u16, Value) {
    match reply {
        Reply::Whole(response) | Reply::StaticError(response) => {
            parse(response.status, response.body)
        }
        other => panic!("expected a whole answer, got {other:?}"),
    }
}

fn error_of(answer: &(u16, Value)) -> String {
    answer.1["error"].as_str().unwrap_or("").to_string()
}

fn refused(answer: (u16, Value), status: u16, message: &str) {
    assert_eq!(answer.0, status, "{}", answer.1);
    assert_eq!(error_of(&answer), message, "{}", answer.1);
}

fn no_file_touched(world: &World) {
    assert!(world.storage.bytes.borrow().is_empty());
    assert_eq!(world.storage.removals.get(), 0);
}

// ---- The signed file (R1) ----

#[test]
fn a_file_signed_with_the_recipe_key_is_accepted_with_its_source_and_version() {
    let signing = new_key();
    let bytes = signed(&signing, "a", "1.2.3");
    let checked = file::check(&bytes, &signing.verifying_key()).unwrap();
    assert_eq!(checked.version, v("1.2.3"));
    assert_eq!(checked.source, script("a", "1.2.3"));
}

#[test]
fn a_file_is_refused_for_each_thing_that_is_wrong_with_it() {
    let signing = new_key();
    let key = signing.verifying_key();
    let good = signed(&signing, "a", "1.0.0");
    let reason = |bytes: &[u8]| file::check(bytes, &key).unwrap_err();

    // Signed with another key (the firmware's, for one): the signature does not verify.
    let firmware = new_key();
    assert_eq!(
        reason(&signed(&firmware, "a", "1.0.0")),
        "Invalid signature"
    );
    // One byte of the source changed: its SHA-256 differs from the header's.
    let mut changed = good.clone();
    *changed.last_mut().unwrap() ^= 1;
    assert_eq!(reason(&changed), "Recipe SHA-256 differs from the header");
    // One byte of the signed header changed (the version): the signature fails.
    let mut changed = good.clone();
    changed[12] ^= 1;
    assert_eq!(reason(&changed), "Invalid signature");
    // The firmware's magic.
    let mut changed = good.clone();
    changed[..8].copy_from_slice(b"SOLITEUP");
    assert_eq!(reason(&changed), "Not a Solite recipe file");
    // A newer format.
    let mut changed = good.clone();
    changed[8] = 2;
    assert_eq!(reason(&changed), "format 2 is not supported");
    // Reserved bytes set.
    let mut changed = good.clone();
    changed[18] = 1;
    assert_eq!(reason(&changed), "Malformed recipe header");
    let mut changed = good.clone();
    changed[60] = 1;
    assert_eq!(reason(&changed), "Malformed recipe header");
    // The length field.
    let mut changed = good.clone();
    changed[20] ^= 1;
    assert_eq!(reason(&changed), "Recipe length differs from the header");
    // A header alone, and a file over 32 KB and a header.
    assert_eq!(reason(&good[..HEADER_BYTES]), "Recipe file too short");
    let big = vec![b' '; MAX_FILE_BYTES + 1 - HEADER_BYTES];
    let over = signed_with(&signing, "1.0.0", std::str::from_utf8(&big).unwrap());
    assert_eq!(over.len(), MAX_FILE_BYTES + 1);
    assert_eq!(
        reason(&over),
        format!("Recipe file too large (at most {MAX_FILE_BYTES} bytes)")
    );
}

#[test]
fn the_firmwares_header_is_not_a_recipe_file_and_the_recipe_key_does_not_sign_firmware() {
    // The same fields under the other magic: the update header's check refuses it, and this one
    // refuses the update's magic.
    let signing = new_key();
    let source = script("a", "1.0.0");
    let mut update = signed_bytes(
        Version::new(1, 0, 0),
        u32::try_from(source.len()).unwrap(),
        &sha256(source.as_bytes()),
    )
    .to_vec();
    update.extend_from_slice(&signing.sign(&update).to_bytes());
    update.extend_from_slice(source.as_bytes());
    assert_eq!(
        file::check(&update, &signing.verifying_key()).unwrap_err(),
        "Not a Solite recipe file"
    );
    let recipe = signed(&signing, "a", "1.0.0");
    let key = signing.verifying_key();
    let header: &[u8; HEADER_BYTES] = recipe[..HEADER_BYTES].try_into().unwrap();
    assert!(
        solite_core::update::check_header(header, &key, Version::new(0, 0, 0), 1 << 20).is_err()
    );
}

#[test]
fn an_upload_over_32_kb_is_413_and_an_upload_of_exactly_32_kb_is_checked() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let big = vec![b' '; MAX_FILE_BYTES + 1 - HEADER_BYTES];
    let over = signed_with(&signing, "1.0.0", std::str::from_utf8(&big).unwrap());
    refused(world.upload(&over), 413, "Body too large");
    // At the limit the body goes through to the checks: a file of spaces is refused by the load.
    let big = vec![b' '; MAX_FILE_BYTES - HEADER_BYTES];
    let at = signed_with(&signing, "1.0.0", std::str::from_utf8(&big).unwrap());
    let answer = world.upload(&at);
    assert_eq!(answer.0, 400, "{}", answer.1);
    no_file_touched(&world);
}

#[test]
fn a_signed_file_of_a_32_kb_script_of_real_code_uploads_and_loads() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let source = include_str!("fixtures/scripts/big-recipe.js");
    assert!(source.len() > 32_000 && source.len() <= MAX_FILE_BYTES - HEADER_BYTES);
    let version = source
        .split("version: \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap();
    let file = signed_with(&signing, version, source);
    assert!(file.len() <= MAX_FILE_BYTES);
    let answer = world.upload(&file);
    assert_eq!(answer.0, 200, "{}", answer.1);
    assert_eq!(world.version_of("big-recipe"), Some(version.to_string()));
}

#[test]
fn a_fourth_installed_runtime_is_409_when_the_psram_is_under_its_limit_and_the_reserve() {
    let mut world = World::new();
    let signing = world.signing.clone();
    for id in ["a", "b", "c"] {
        assert_eq!(world.upload(&signed(&signing, id, "1.0.0")).0, 200, "{id}");
    }
    // Three runtimes loaded, the PSRAM short of a fourth runtime's limit and the reserve.
    let needed = SYSTEM_MEMORY_LIMIT + SCRIPT_PSRAM_RESERVE;
    world.probe.set(64 * 1024, needed - 1);
    let answer = world.upload(&signed(&signing, "d", "1.0.0"));
    assert_eq!(answer.0, 409, "{}", answer.1);
    assert!(error_of(&answer).starts_with("Not enough memory for d:"));
    assert_eq!(world.version_of("d"), None);
    // With the room, the fourth is admitted.
    world.probe.set(64 * 1024, needed);
    assert_eq!(world.upload(&signed(&signing, "d", "1.0.0")).0, 200);
}

#[test]
fn an_installed_script_must_declare_its_version_as_x_y_z_and_the_header_must_agree() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let source = |version: &str| {
        let base = script("a", "1.0.0");
        base.replace(r#"version: "1.0.0","#, version)
    };
    // No version at all, and one that is not x.y.z.
    for text in ["", r#"version: "1.0","#, r#"version: 1,"#] {
        let bytes = signed_with(&signing, "1.0.0", &source(text));
        let answer = world.upload(&bytes);
        assert_eq!(answer.0, 400, "{text}: {}", answer.1);
        let reason = error_of(&answer);
        assert!(
            reason == "Script recipe invalid: version must be x.y.z"
                || reason == "Script recipe invalid: version must be a text",
            "{text}: {reason}"
        );
    }
    let bytes = signed_with(&signing, "1.0.0", &source(r#"version: "1.0","#));
    refused(
        world.upload(&bytes),
        400,
        "Script recipe invalid: version must be x.y.z",
    );
    // The script says 1.0.1 behind a header of 1.0.0.
    let bytes = signed_with(&signing, "1.0.0", &script("a", "1.0.1"));
    refused(
        world.upload(&bytes),
        400,
        "version differs from the signed one",
    );
    no_file_touched(&world);
}

#[test]
fn an_id_of_33_characters_or_with_a_slash_is_refused() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let long = "a".repeat(33);
    let bytes = signed(&signing, &long, "1.0.0");
    refused(
        world.upload(&bytes),
        400,
        "Script recipe invalid: id must be 1 to 32 characters among a-z, 0-9 and -",
    );
    let bytes = signed(&signing, "a/b", "1.0.0");
    refused(
        world.upload(&bytes),
        400,
        "Script recipe invalid: id must be 1 to 32 characters among a-z, 0-9 and -",
    );
    // 32 characters pass.
    let bytes = signed(&signing, &"a".repeat(32), "1.0.0");
    assert_eq!(world.upload(&bytes).0, 200);
}

#[test]
fn the_ids_store_and_install_are_reserved_for_every_source() {
    let mut world = World::new();
    let signing = world.signing.clone();
    for id in ["store", "install"] {
        let reason = format!("Script recipe invalid: id {id} is reserved");
        // An upload.
        let bytes = signed(&signing, id, "1.0.0");
        refused(world.upload(&bytes), 400, &reason);
        // The catalogue.
        world.publish(&[(id, "1.0.0", &bytes)]);
        refused(world.install_id(id), 400, &reason);
        // `--recipe-script`: the loader of the Mac's flag.
        let mut home = Home::new(T0);
        let error = home
            .load_script_recipe(
                &script(id, "1.0.0"),
                Arc::new(SystemRuntimes),
                Arc::new(|| 0),
            )
            .unwrap_err();
        assert_eq!(error, reason);
    }
    no_file_touched(&world);
}

// ---- The recipe key (R1.5) ----

#[test]
fn a_build_without_the_recipe_key_refuses_every_install_update_and_refresh_with_503() {
    let mut world = World::with_key_text(None);
    let bytes = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &bytes)]);
    for answer in [
        world.install_id("a"),
        world.upload(&bytes),
        world.update("a"),
        world.refresh(),
    ] {
        refused(answer, 503, "Recipe installs are not configured");
    }
    assert!(world.fetch.log.is_empty());
    // An uninstall needs no key: a file that was there can still go.
    assert_eq!(world.uninstall("a").0, 404);
}

#[test]
fn a_key_of_small_order_or_of_63_characters_is_refused_at_start_and_installs_answer_503() {
    let small_order = format!("01{}", "00".repeat(31));
    for text in [small_order.as_str(), &"a".repeat(63)] {
        let probe = Arc::new(FakeProbe::new());
        let (_, line) = RecipeInstalls::from_key_text(Some(text), probe);
        assert_eq!(line, "recipes: invalid recipe key, installs refused");
        let mut world = World::with_key_text(Some(text));
        refused(world.refresh(), 503, "Recipe installs are not configured");
    }
    let probe = Arc::new(FakeProbe::new());
    let (_, line) = RecipeInstalls::from_key_text(None, probe);
    assert_eq!(line, "recipes: no recipe key, installs refused");
}

// ---- The catalogue (R2) ----

#[test]
fn the_store_before_any_read_is_empty() {
    let mut world = World::new();
    let (status, body) = world.get("/api/v1/plugins/store");
    assert_eq!((status, body), (200, json!([])));
    let (status, body) = world.get("/api/v1/plugins");
    assert_eq!((status, body), (200, json!([])));
    let (_, resources) = world.get("/api/v1/system/resources");
    assert_eq!(
        resources["catalogue"],
        json!({ "readAt": null, "error": null })
    );
}

#[test]
fn a_valid_catalogue_is_kept_and_served_as_sowels_store_entries() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let a = signed(&signing, "a", "1.0.0");
    let b = signed(&signing, "b", "1.0.0");
    let mut with = entry("a", "1.0.0", &a, &file_url("a", "1.0.0"));
    with["repo"] = json!("someone/sowel-recipe-a");
    with["category"] = json!("lighting");
    with["author"] = json!("Someone");
    with["tags"] = json!(["light", "motion"]);
    with["icon"] = json!("Lightbulb");
    let plain = entry("b", "1.0.0", &b, &file_url("b", "1.0.0"));
    world.fetch.serve(
        DEFAULT_CATALOGUE_URL,
        Answer::Body(catalogue_of(&[with, plain])),
    );
    let (status, body) = world.refresh();
    assert_eq!(
        (status, &body),
        (200, &json!({ "count": 2, "source": "remote" }))
    );
    let (status, store) = world.get("/api/v1/plugins/store");
    assert_eq!(status, 200);
    let first = &store[0];
    assert_eq!(first["id"], "a");
    assert_eq!(first["version"], "1.0.0");
    assert_eq!(first["repo"], "someone/sowel-recipe-a");
    assert_eq!(first["type"], "recipe");
    assert_eq!(first["category"], "lighting");
    assert_eq!(first["author"], "Someone");
    assert_eq!(first["tags"], json!(["light", "motion"]));
    assert_eq!(first["icon"], "Lightbulb");
    assert_eq!(first["compatible"], true);
    assert_eq!(first["isOfficial"], true);
    assert_eq!(first["tier"], "official");
    for gone in [
        "installed",
        "refused",
        "builtIn",
        "url",
        "sha256",
        "size",
        "compatReason",
    ] {
        assert!(first.get(gone).is_none(), "{gone}: {first}");
    }
    // A 0.3 entry: the repo is derived, nothing else is added.
    assert_eq!(store[1]["repo"], "solite-recipes/b");
    assert!(store[1].get("tags").is_none());
    // The read date is in the resources.
    let (_, resources) = world.get("/api/v1/system/resources");
    assert_eq!(resources["catalogue"]["readAt"], "2026-11-21T09:12:03.120Z");
    assert_eq!(resources["catalogue"]["error"], Value::Null);
    assert_eq!(world.fetch.log, vec![DEFAULT_CATALOGUE_URL.to_string()]);
}

#[test]
fn an_entry_that_needs_a_newer_solite_is_listed_as_not_compatible() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let a = signed(&signing, "a", "1.0.0");
    let mut with = entry("a", "1.0.0", &a, &file_url("a", "1.0.0"));
    with["minVersion"] = json!("99.0.0");
    world
        .fetch
        .serve(DEFAULT_CATALOGUE_URL, Answer::Body(catalogue_of(&[with])));
    world.refresh();
    let (_, store) = world.get("/api/v1/plugins/store");
    assert_eq!(store[0]["compatible"], false);
    assert_eq!(store[0]["compatReason"], "Requires Solite >= 99.0.0");
}

#[test]
fn the_store_serves_the_i18n_of_a_sowel_catalogue_whole_with_groups_and_options() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let a = signed(&signing, "a", "1.0.0");
    let mut with = entry("a", "1.0.0", &a, &file_url("a", "1.0.0"));
    let i18n = json!({
        "fr": {
            "name": "Lumi\u{e8}re", "description": "Allume",
            "groups": {"main": "Principal"},
            "slots": {"zone": {"name": "Zone", "description": "La zone",
                "options": {"on": "Allum\u{e9}", "off": "\u{c9}teint"}}}
        },
        "en": {"name": "Light"}
    });
    with["i18n"] = i18n.clone();
    world
        .fetch
        .serve(DEFAULT_CATALOGUE_URL, Answer::Body(catalogue_of(&[with])));
    let (status, body) = world.refresh();
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["source"], "remote", "{body}");
    let (_, store) = world.get("/api/v1/plugins/store");
    assert_eq!(store[0]["i18n"], i18n);
}

#[test]
fn an_i18n_that_is_not_an_object_refuses_the_catalogue_and_keeps_the_last_list() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let a = signed(&signing, "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    assert_eq!(world.refresh().0, 200);
    let mut bad = entry("a", "1.0.0", &a, &file_url("a", "1.0.0"));
    bad["i18n"] = json!({"fr": "x"}).get("fr").unwrap().clone();
    world.now.set(world.now.get() + 61 * SECOND);
    world.clock.set(world.clock.get() + 61 * SECOND);
    world
        .fetch
        .serve(DEFAULT_CATALOGUE_URL, Answer::Body(catalogue_of(&[bad])));
    let (status, answer) = world.refresh();
    assert_eq!(
        (status, &answer),
        (200, &json!({ "count": 1, "source": "local" }))
    );
    let (_, resources) = world.get("/api/v1/system/resources");
    assert_eq!(
        resources["catalogue"]["error"],
        "Invalid catalogue: a: i18n"
    );
    assert_eq!(
        world
            .get("/api/v1/plugins/store")
            .1
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn installed_recipes_leave_the_store_for_get_plugins_and_a_refused_file_is_in_the_resources() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let a = signed(&signing, "a", "1.0.0");
    let b = signed(&signing, "b", "1.0.0");
    let native = signed(&signing, "native-like", "1.0.0");
    // `motion-light` is the script given by `--recipe-script` (built in).
    let clock = world.script_clock();
    world
        .lock()
        .load_script_recipe(MOTION_LIGHT, Arc::new(SystemRuntimes), clock)
        .unwrap();
    // `b` is on the flash but changed: refused at start.
    let mut changed = b.clone();
    *changed.last_mut().unwrap() ^= 1;
    world.storage.put("scripts/b.srs", &changed);
    world.boot();
    world.serve(&[
        ("a", "1.0.0", &a),
        ("b", "1.0.0", &b),
        ("motion-light", "1.0.0", &native),
    ]);
    assert_eq!(world.install_id("a").0, 200);
    world.refresh();
    let (_, store) = world.get("/api/v1/plugins/store");
    let ids: Vec<&str> = store
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    // `a` is installed and `motion-light` is built in: both are in GET /plugins, not here.
    assert_eq!(ids, ["b"]);
    let (_, installed) = world.get("/api/v1/plugins");
    let by_id = |id: &str| {
        installed
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["manifest"]["id"] == id)
            .cloned()
    };
    let a = by_id("a").unwrap();
    assert_eq!(a["manifest"]["version"], "1.0.0");
    assert_eq!(a["manifest"]["type"], "recipe");
    assert_eq!(a["manifest"]["repo"], "solite-recipes/a");
    assert_eq!(a["enabled"], true);
    assert_eq!(a["status"], "connected");
    assert_eq!(a["deviceCount"], 0);
    assert_eq!(a["offlineDeviceCount"], 0);
    assert_eq!(a["source"], "registry");
    assert!(a["installedAt"].is_string(), "{a}");
    assert!(a.get("latestVersion").is_none());
    assert!(by_id("motion-light").is_some());
    // `b` is refused: not listed, its reason is in the resources.
    assert!(by_id("b").is_none());
    let (_, resources) = world.get("/api/v1/system/resources");
    let refused = resources["scripts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "b")
        .unwrap();
    assert_eq!(refused["reason"], "Recipe SHA-256 differs from the header");
}

#[test]
fn get_plugins_names_the_newer_version_the_catalogue_holds() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    let signing = world.signing.clone();
    let a2 = signed(&signing, "a", "1.1.0");
    world.publish(&[("a", "1.1.0", &a2)]);
    let (_, installed) = world.get("/api/v1/plugins");
    assert_eq!(installed[0]["manifest"]["version"], "1.0.0");
    assert_eq!(installed[0]["latestVersion"], "1.1.0");
    assert_eq!(world.get("/api/v1/plugins/store").1, json!([]));
}

#[test]
fn an_install_moves_a_recipe_from_the_store_to_get_plugins_and_an_uninstall_moves_it_back() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let a = signed(&signing, "a", "1.0.0");
    let b = signed(&signing, "b", "1.0.0");
    world.serve(&[("a", "1.0.0", &a), ("b", "1.0.0", &b)]);
    world.refresh();
    assert_eq!(
        world
            .get("/api/v1/plugins/store")
            .1
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(world.get("/api/v1/plugins").1, json!([]));
    assert_eq!(world.install_id("a"), (200, json!({ "success": true })));
    let (_, store) = world.get("/api/v1/plugins/store");
    assert_eq!(store.as_array().unwrap().len(), 1);
    assert_eq!(store[0]["id"], "b");
    assert_eq!(world.get("/api/v1/plugins").1[0]["manifest"]["id"], "a");
    assert_eq!(world.uninstall("a"), (200, json!({ "success": true })));
    assert_eq!(
        world
            .get("/api/v1/plugins/store")
            .1
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(world.get("/api/v1/plugins").1, json!([]));
}

#[test]
fn the_old_recipe_store_routes_are_unknown_routes() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    assert_eq!(
        world.get("/api/v1/recipes/store"),
        (404, json!({ "error": "Recipe not found" }))
    );
    assert_eq!(
        world
            .call("POST", "/api/v1/recipes/store/refresh", Some(JSON), b"{}")
            .0,
        404
    );
    assert_eq!(
        world
            .call("POST", "/api/v1/recipes/install", Some(JSON), b"{}")
            .0,
        405
    );
    assert_eq!(
        world
            .call("POST", "/api/v1/recipes/a/update", Some(JSON), b"{}")
            .0,
        404
    );
    assert_eq!(
        world
            .call("POST", "/api/v1/recipes/a/uninstall", Some(JSON), b"{}")
            .0,
        404
    );
    assert_eq!(world.version_of("a").as_deref(), Some("1.0.0"));
}

#[test]
fn a_catalogue_over_a_bound_is_refused_as_a_whole_and_the_last_good_list_is_kept() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let a = signed(&signing, "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    assert_eq!(world.refresh().0, 200);
    let one = entry("a", "1.0.0", &a, &file_url("a", "1.0.0"));
    let many: Vec<Value> = (0..33)
        .map(|n| entry(&format!("r{n}"), "1.0.0", &a, &file_url("a", "1.0.0")))
        .collect();
    let huge = format!(
        "{{\"format\":1,\"recipes\":[],\"pad\":\"{}\"}}",
        "x".repeat(33 * 1024)
    );
    let http_internet = {
        let mut bad = one.clone();
        bad["url"] = json!("http://example.com/a.srs");
        bad
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("33 entries", catalogue_of(&many)),
        ("32 KB + 1", huge.into_bytes()),
        ("not JSON", b"not json".to_vec()),
        (
            "format 2",
            json!({"format": 2, "recipes": []}).to_string().into_bytes(),
        ),
        ("http on the internet", catalogue_of(&[http_internet])),
    ];
    for (name, body) in cases {
        world.now.set(world.now.get() + 61 * SECOND);
        world.clock.set(world.clock.get() + 61 * SECOND);
        world.fetch.serve(DEFAULT_CATALOGUE_URL, Answer::Body(body));
        let (status, answer) = world.refresh();
        assert_eq!(status, 200, "{name}: {answer}");
        assert_eq!(answer, json!({ "count": 1, "source": "local" }), "{name}");
        let (_, resources) = world.get("/api/v1/system/resources");
        assert!(
            resources["catalogue"]["error"].is_string(),
            "{name}: {resources}"
        );
        assert_eq!(
            world
                .get("/api/v1/plugins/store")
                .1
                .as_array()
                .unwrap()
                .len(),
            1,
            "{name}"
        );
    }
    // The bounds themselves: 32 entries and exactly 32 KB are kept.
    let at: Vec<Value> = (0..32)
        .map(|n| entry(&format!("r{n}"), "1.0.0", &a, &file_url("a", "1.0.0")))
        .collect();
    assert_eq!(catalogue::parse(&catalogue_of(&at)).unwrap().len(), 32);
}

#[test]
fn two_refreshes_within_a_minute_are_200_then_429() {
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    assert_eq!(world.refresh().0, 200);
    let again = world.refresh();
    assert_eq!(again.0, 429, "{}", again.1);
    assert!(error_of(&again).starts_with("Please wait"));
    // The held list is still there.
    assert_eq!(world.get("/api/v1/plugins/store").1[0]["id"], "a");
    world.clock.set(world.clock.get() + 61 * SECOND);
    assert_eq!(world.refresh().0, 200);
}

#[test]
fn the_429_of_a_refresh_is_decided_before_the_busy_and_the_memory_answers() {
    let mut world = World::new();
    let hold = held_refresh(&world);
    // Within a minute: 429, not the 409 of the refresh that runs.
    let answer = world.refresh();
    assert_eq!(answer.0, 429, "{}", answer.1);
    drop(hold);
    // Even with the internal RAM short (503 otherwise).
    world.probe.set(1024, 1_500_000);
    assert_eq!(world.refresh().0, 429);
    a_minute_later(&world);
    assert_eq!(world.refresh().0, 503);
}

#[test]
fn a_refresh_with_the_clock_not_set_is_503() {
    let mut world = World::at(1_500_000_000_000);
    refused(
        world.refresh(),
        503,
        "The clock is not set yet: reading the recipe catalogue needs the date",
    );
    assert!(world.fetch.log.is_empty());
}

#[test]
fn a_refresh_with_the_network_down_answers_the_last_list_and_the_error() {
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    assert_eq!(world.refresh().0, 200);
    world.clock.set(world.clock.get() + 61 * SECOND);
    world.fetch.serve(
        DEFAULT_CATALOGUE_URL,
        Answer::Fail("no route to host".into()),
    );
    let (status, body) = world.refresh();
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "count": 1, "source": "local" }));
    assert_eq!(world.get("/api/v1/plugins/store").1[0]["id"], "a");
    let (_, resources) = world.get("/api/v1/system/resources");
    assert_eq!(resources["catalogue"]["error"], "no route to host");
}

#[test]
fn the_catalogue_setting_is_checked_at_the_save_and_empty_means_the_default() {
    let mut world = World::new();
    let key = "recipes.catalogue";
    for value in [
        "http://example.com/recipes.json".to_string(),
        format!("https://example.com/{}", "a".repeat(513 - 20)),
    ] {
        assert_eq!(
            value.chars().count().max(513),
            513.max(value.chars().count())
        );
        let answer = world.put_json("/api/v1/settings", json!({ key: value }));
        assert_eq!(answer.0, 400, "{}", answer.1);
        assert_eq!(error_of(&answer), format!("Invalid value for {key}"));
    }
    // 513 characters exactly.
    let long = format!(
        "https://example.com/{}",
        "a".repeat(513 - "https://example.com/".len())
    );
    assert_eq!(long.chars().count(), 513);
    assert_eq!(
        world.put_json("/api/v1/settings", json!({ key: long })).0,
        400
    );
    // A home network address over http is allowed, and read.
    let (status, _) = world.put_json(
        "/api/v1/settings",
        json!({ key: "http://192.168.1.5:8000/recipes.json" }),
    );
    assert_eq!(status, 200);
    world.fetch.serve(
        "http://192.168.1.5:8000/recipes.json",
        Answer::Body(catalogue_of(&[])),
    );
    assert_eq!(world.refresh().0, 200);
    assert_eq!(
        world.fetch.log,
        vec!["http://192.168.1.5:8000/recipes.json".to_string()]
    );
    // Empty: the default.
    assert_eq!(
        world.put_json("/api/v1/settings", json!({ key: "" })).0,
        200
    );
    world.clock.set(world.clock.get() + 61 * SECOND);
    world
        .fetch
        .serve(DEFAULT_CATALOGUE_URL, Answer::Body(catalogue_of(&[])));
    assert_eq!(world.refresh().0, 200);
    assert_eq!(world.fetch.log.last().unwrap(), DEFAULT_CATALOGUE_URL);
}

// ---- Install (R3.2 to R3.4) ----

#[test]
fn a_recipe_installs_from_the_catalogue_with_one_write() {
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    let (status, body) = world.install_id("a");
    assert_eq!((status, &body["success"]), (200, &json!(true)), "{body}");
    assert_eq!(body, json!({ "success": true }));
    assert_eq!(world.file("a"), Some(a));
    assert_eq!(world.writes("a"), 1);
    assert_eq!(world.version_of("a"), Some("1.0.0".to_string()));
    // `GET /recipes` lists it, and the flash was written from the job's thread only.
    let (_, list) = world.get("/api/v1/recipes");
    assert!(list.as_array().unwrap().iter().any(|r| r["id"] == "a"));
    assert!(
        world.hops.count.get() >= 2,
        "the load and the start run on the script thread"
    );
}

#[test]
fn a_recipe_installs_from_an_upload_with_one_write() {
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    let (status, body) = world.upload(&a);
    assert_eq!((status, body), (200, json!({ "success": true })));
    assert_eq!(world.file("a"), Some(a));
    assert_eq!(world.writes("a"), 1);
    assert!(world.fetch.log.is_empty(), "an upload touches no network");
}

#[test]
fn an_install_before_any_catalogue_read_reads_it_first() {
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    assert_eq!(world.install_id("a").0, 200);
    assert_eq!(
        world.fetch.log,
        vec![DEFAULT_CATALOGUE_URL.to_string(), file_url("a", "1.0.0")]
    );
    assert_eq!(
        world.get("/api/v1/plugins").1[0]["manifest"]["version"],
        "1.0.0"
    );
}

#[test]
fn an_install_whose_file_differs_from_the_catalogue_is_400_and_writes_nothing() {
    // The id: the catalogue says `a`, the signed script says `b`.
    let mut world = World::new();
    let b = signed(&world.signing.clone(), "b", "1.0.0");
    world.fetch.serve(
        DEFAULT_CATALOGUE_URL,
        Answer::Body(catalogue_of(&[entry(
            "a",
            "1.0.0",
            &b,
            &file_url("a", "1.0.0"),
        )])),
    );
    world
        .fetch
        .serve(&file_url("a", "1.0.0"), Answer::Body(b.clone()));
    let answer = world.install_id("a");
    assert_eq!(answer.0, 400, "{}", answer.1);
    assert!(
        error_of(&answer).contains("declares the id b"),
        "{}",
        answer.1
    );
    assert!(
        world.lock().recipes().recipe_info("b").is_none(),
        "unloaded again"
    );
    no_file_touched(&world);

    // The version: the catalogue says 1.0.1, the signed file says 1.0.0.
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.fetch.serve(
        DEFAULT_CATALOGUE_URL,
        Answer::Body(catalogue_of(&[entry(
            "a",
            "1.0.1",
            &a,
            &file_url("a", "1.0.1"),
        )])),
    );
    world
        .fetch
        .serve(&file_url("a", "1.0.1"), Answer::Body(a.clone()));
    let answer = world.install_id("a");
    assert_eq!(answer.0, 400, "{}", answer.1);
    no_file_touched(&world);

    // The size and the SHA-256.
    let mut world = World::new();
    let a2 = signed(&world.signing.clone(), "a", "1.0.0");
    let mut size = entry("a", "1.0.0", &a2, &file_url("a", "1.0.0"));
    size["size"] = json!(a2.len() + 1);
    world
        .fetch
        .serve(DEFAULT_CATALOGUE_URL, Answer::Body(catalogue_of(&[size])));
    world
        .fetch
        .serve(&file_url("a", "1.0.0"), Answer::Body(a2.clone()));
    assert_eq!(world.install_id("a").0, 400);
    no_file_touched(&world);
    let mut world = World::new();
    let mut digest = entry("a", "1.0.0", &a2, &file_url("a", "1.0.0"));
    digest["sha256"] = json!(hex(&sha256(b"another file")));
    world
        .fetch
        .serve(DEFAULT_CATALOGUE_URL, Answer::Body(catalogue_of(&[digest])));
    world.fetch.serve(&file_url("a", "1.0.0"), Answer::Body(a2));
    let answer = world.install_id("a");
    assert_eq!(answer.0, 400, "{}", answer.1);
    assert_eq!(
        error_of(&answer),
        "Download SHA-256 differs from the catalogue"
    );
    no_file_touched(&world);
}

#[test]
fn an_install_with_the_network_down_or_a_download_cut_for_good_is_502() {
    // The catalogue is unreachable.
    let mut world = World::new();
    world
        .fetch
        .serve(DEFAULT_CATALOGUE_URL, Answer::Fail("no route".into()));
    let answer = world.install_id("a");
    assert_eq!(answer.0, 502, "{}", answer.1);
    assert!(error_of(&answer).contains("no route"));

    // The file is cut twice and resumed: it installs.
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    world.fetch.serve(
        &file_url("a", "1.0.0"),
        Answer::Cuts {
            data: a.clone(),
            step: 150,
            cuts: 2,
        },
    );
    let (status, body) = world.install_id("a");
    assert_eq!(status, 200, "{body}");
    assert_eq!(world.file("a"), Some(a.clone()));
    assert!(
        world.fetch.log.iter().any(|l| l.contains("from 150")),
        "{:?}",
        world.fetch.log
    );

    // Cut three times, resumed three times, cut again: 502 with the reason.
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    world.fetch.serve(
        &file_url("a", "1.0.0"),
        Answer::Cuts {
            data: a,
            step: 50,
            cuts: usize::MAX,
        },
    );
    let answer = world.install_id("a");
    assert_eq!(answer.0, 502, "{}", answer.1);
    assert!(
        error_of(&answer).contains("after 3 resumes"),
        "{}",
        answer.1
    );
    no_file_touched(&world);
}

#[test]
fn an_install_is_refused_for_a_script_the_program_carries_an_installed_or_unknown_id_or_a_fifth_file()
 {
    let mut world = World::new();
    let signing = world.signing.clone();
    let a = signed(&signing, "a", "1.0.0");
    let b = signed(&signing, "b", "1.0.0");
    let c = signed(&signing, "c", "1.0.0");
    let d = signed(&signing, "d", "1.0.0");
    let local = signed(&signing, "local", "1.0.0");
    let old = signed(&signing, "old", "1.0.0");
    world.serve(&[("a", "1.0.0", &a), ("b", "1.0.0", &b), ("c", "1.0.0", &c)]);
    // A script given by `--recipe-script`.
    let clock = world.script_clock();
    world
        .lock()
        .load_script_recipe(&script("local", "1.0.0"), Arc::new(SystemRuntimes), clock)
        .unwrap();
    world.serve(&[
        ("local", "1.0.0", &local),
        ("a", "1.0.0", &a),
        ("b", "1.0.0", &b),
        ("c", "1.0.0", &c),
        ("d", "1.0.0", &d),
    ]);
    refused(world.install_id("local"), 409, "local is built in");
    // The catalogue does not list it.
    refused(world.install_id("nothing"), 404, "Plugin not found");
    // The id of a loaded file.
    assert_eq!(world.install_id("a").0, 200);
    refused(world.install_id("a"), 409, "Recipe already installed");
    // A script given by `--recipe-script` and three files make four: a fifth is refused.
    assert_eq!(world.install_id("b").0, 200);
    assert_eq!(world.install_id("c").0, 200);
    refused(
        world.install_id("d"),
        409,
        &format!("Too many script recipes (at most {MAX_SCRIPT_RECIPES})"),
    );
    let _ = old;

    // Four files on the flash, none of them a built-in's: a fifth id.
    let mut world = World::new();
    let signing = world.signing.clone();
    let files: Vec<(String, Vec<u8>)> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|id| (id.to_string(), signed(&signing, id, "1.0.0")))
        .collect();
    let entries: Vec<(&str, &str, &[u8])> = files
        .iter()
        .map(|(id, bytes)| (id.as_str(), "1.0.0", bytes.as_slice()))
        .collect();
    world.serve(&entries);
    for id in ["a", "b", "c", "d"] {
        assert_eq!(world.install_id(id).0, 200, "{id}");
    }
    refused(
        world.install_id("e"),
        409,
        &format!("Too many script recipes (at most {MAX_SCRIPT_RECIPES})"),
    );
}

#[test]
fn an_entry_that_needs_a_newer_firmware_is_409() {
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    let mut needs = entry("a", "1.0.0", &a, &file_url("a", "1.0.0"));
    needs["minVersion"] = json!("99.0.0");
    world
        .fetch
        .serve(DEFAULT_CATALOGUE_URL, Answer::Body(catalogue_of(&[needs])));
    world.fetch.serve(&file_url("a", "1.0.0"), Answer::Body(a));
    refused(
        world.install_id("a"),
        409,
        "a 1.0.0 needs Solite 99.0.0 or later",
    );
    no_file_touched(&world);
}

#[test]
fn an_install_after_an_uninstall_finds_no_instance_to_start() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    let one = world.instance("a");
    let two = world.instance("a");
    assert_eq!(
        world
            .call(
                "POST",
                &format!("/api/v1/recipe-instances/{two}/disable"),
                Some(JSON),
                b"{}"
            )
            .0,
        200
    );
    assert!(world.running(&one) && !world.running(&two));
    // An uninstall deletes them (R3.6): a new install starts none.
    assert_eq!(world.uninstall("a").0, 200);
    assert!(!world.running(&one));
    world.installed("a", "1.0.0");
    assert!(
        !world.running(&one),
        "the deleted instance does not come back"
    );
    assert!(world.lock().recipes().instances().next().is_none());
}

fn kept_instance(id: &str, params: &Value) -> RecipeInstance {
    RecipeInstance {
        id: Id::parse(id).unwrap(),
        recipe_id: "motion-light".into(),
        params: params.to_string().into(),
        enabled: true,
        created_at: T0,
    }
}

#[test]
fn the_real_motion_light_installed_by_an_upload_starts_the_three_instances_a_native_home_kept() {
    // Spec 046 R6: a home saved with native `motion-light` instances, flashed with its storage
    // kept: they are stopped (no recipe), then the signed 1.1.2 is uploaded and they start.
    let mut rig = support::Rig::new();
    let g = rig.garage();
    let params = json!({"zone": g.zone, "lights": [g.light], "timeout": "20s"});
    let ids = [
        "22222222-2222-4222-8222-000000000001",
        "22222222-2222-4222-8222-000000000002",
        "22222222-2222-4222-8222-000000000003",
    ];
    rig.home
        .load_recipes(ids.iter().map(|id| kept_instance(id, &params)).collect());
    rig.home.start_recipes(rig.now, rig.now);
    let mut world = World::over(rig.home, T0);
    for id in ids {
        assert!(!world.running(id));
        assert_eq!(
            world.messages(id).last().unwrap(),
            "Failed to restore: Recipe not found: motion-light"
        );
    }
    let file = signed_with(&world.signing.clone(), "1.1.2", MOTION_LIGHT);
    let (status, body) = world.upload(&file);
    assert_eq!((status, &body["success"]), (200, &json!(true)), "{body}");
    for id in ids {
        assert!(world.running(id), "{id}: {:?}", world.messages(id));
    }
    let (_, list) = world.get("/api/v1/recipe-instances");
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 3);
    for item in list {
        assert_eq!(item["params"], params, "the settings are kept");
        assert_eq!(item["enabled"], true);
    }
}

#[test]
fn a_boot_by_day_with_the_installed_file_starts_with_the_daylight_before_any_aggregate() {
    use solite_core::engine::{Saver, load_home};
    use solite_core::radio::zcl::Command;
    use support::{Rig, motion};
    const JUNE_NOON: u64 = 1_782_043_200_000;
    const PIR: u8 = support::PIR;
    let mut rig = Rig::new();
    rig.now = JUNE_NOON;
    let g = rig.garage();
    let (status, body) = rig.put(
        "/api/v1/settings",
        json!({
            "home.latitude": "48.8566",
            "home.longitude": "2.3522",
            "home.timezone": "Europe/Paris",
        }),
    );
    assert_eq!(status, 200, "{body}");
    rig.report(&motion(PIR, true));
    // The instance is made as the saved home had it (the script loaded here only to create it).
    let id = rig.garage_instance(&g, json!({"disableWhenDaylight": true}));
    let id = id.as_str();
    let saved = MemStorage::default();
    let home = Mutex::new(std::mem::replace(&mut rig.home, Home::new(0)));
    Saver::new(&saved, rig.now)
        .tick(&home, rig.now, true)
        .unwrap();
    // The boot: the home read back, the installed file loaded as the start does, the recipes
    // started before the first aggregate.
    let loaded = load_home(&&saved, JUNE_NOON + 1_000).unwrap();
    let mut world = World::over(loaded, JUNE_NOON + 1_000);
    let file = signed_with(&world.signing.clone(), "1.1.2", MOTION_LIGHT);
    world.storage.put("scripts/motion-light.srs", &file);
    let lines = world.boot();
    assert!(
        lines[0].contains("motion-light 1.1.2 installed"),
        "{lines:?}"
    );
    let now = JUNE_NOON + 1_000;
    world.lock().start_recipes(now, now);
    let mut sent = Vec::new();
    {
        let home = world.lock();
        while let Some(queued) = home.orders().take(now) {
            sent.extend(queued.request.commands.iter().map(|c| c.command));
            home.orders().done();
        }
    }
    assert!(!sent.contains(&Command::On), "{sent:?}");
    assert_eq!(
        world.messages(id).last().unwrap(),
        "Recipe activated — daytime, lights off"
    );
}

#[test]
fn a_write_that_fails_unloads_the_recipe_again_and_answers_500() {
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    world.storage.fail_byte_writes.set(true);
    refused(world.install_id("a"), 500, "Could not save the recipe");
    assert!(world.lock().recipes().recipe_info("a").is_none());
    assert_eq!(world.file("a"), None);
    // Nothing is left half done: the next install goes through.
    world.storage.fail_byte_writes.set(false);
    assert_eq!(world.install_id("a").0, 200);
}

// ---- Update (R3.5) ----

#[test]
fn a_newer_version_that_loads_replaces_the_file_and_restarts_the_instances_with_their_settings() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    let instance = world.instance("a");
    assert!(
        world
            .messages(&instance)
            .iter()
            .any(|m| m == "started 1.0.0")
    );
    let newer = signed(&world.signing.clone(), "a", "1.0.1");
    world.publish(&[("a", "1.0.1", &newer)]);
    world.clock.set(world.clock.get() + 61 * SECOND);
    let (status, body) = world.update("a");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "success": true }));
    assert_eq!(world.version_of("a"), Some("1.0.1".to_string()));
    assert_eq!(world.file("a"), Some(newer));
    assert_eq!(world.writes("a"), 2);
    assert!(world.running(&instance));
    assert!(
        world
            .messages(&instance)
            .iter()
            .any(|m| m == "started 1.0.1")
    );
    // Its settings are the ones it had.
    let (_, list) = world.get("/api/v1/recipe-instances");
    assert_eq!(list[0]["params"], json!({ "zone": ROOT }));
}

#[test]
fn a_newer_version_that_does_not_load_leaves_the_old_one_running_and_answers_400() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    let instance = world.instance("a");
    let old = world.file("a").unwrap();
    let broken = signed_with(
        &world.signing.clone(),
        "1.0.1",
        &script("a", "1.0.1").replace("recipe({", "recipe({ ("),
    );
    world.publish(&[("a", "1.0.1", &broken)]);
    let answer = world.update("a");
    assert_eq!(answer.0, 400, "{}", answer.1);
    assert!(
        error_of(&answer).starts_with("Script recipe invalid:"),
        "{}",
        answer.1
    );
    assert_eq!(world.version_of("a"), Some("1.0.0".to_string()));
    assert_eq!(world.file("a"), Some(old));
    assert!(world.running(&instance), "the old version runs again");
    assert_eq!(world.writes("a"), 1);
}

#[test]
fn the_same_or_an_older_version_is_409_by_the_catalogue_and_by_an_upload() {
    let mut world = World::new();
    world.installed("a", "1.1.0");
    let same = signed(&world.signing.clone(), "a", "1.1.0");
    let older = signed(&world.signing.clone(), "a", "1.0.0");
    world.publish(&[("a", "1.1.0", &same)]);
    world.clock.set(world.clock.get() + 61 * SECOND);
    refused(world.update("a"), 409, "a 1.1.0 is not newer than 1.1.0");
    world.publish(&[("a", "1.0.0", &older)]);
    refused(world.update("a"), 409, "a 1.0.0 is not newer than 1.1.0");
    refused(world.upload(&same), 409, "a 1.1.0 is not newer than 1.1.0");
    refused(world.upload(&older), 409, "a 1.0.0 is not newer than 1.1.0");
    assert_eq!(world.writes("a"), 1);
}

#[test]
fn an_upload_of_a_newer_file_of_an_installed_recipe_updates_it() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    let instance = world.instance("a");
    let newer = signed(&world.signing.clone(), "a", "2.0.0");
    let (status, body) = world.upload(&newer);
    assert_eq!(status, 200, "{body}");
    assert_eq!(world.version_of("a"), Some("2.0.0".to_string()));
    assert!(world.running(&instance));
}

#[test]
fn a_write_that_fails_after_the_new_version_loaded_gives_the_old_one_back_and_is_500() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    let instance = world.instance("a");
    let old = world.file("a").unwrap();
    let newer = signed(&world.signing.clone(), "a", "1.0.1");
    world.publish(&[("a", "1.0.1", &newer)]);
    world.storage.fail_byte_writes.set(true);
    refused(world.update("a"), 500, "Could not save the recipe");
    assert_eq!(world.version_of("a"), Some("1.0.0".to_string()));
    assert_eq!(world.file("a"), Some(old));
    assert!(world.running(&instance));
}

#[test]
fn an_old_file_that_does_not_load_again_fails_the_recipe_with_both_reasons() {
    let flaky = Flaky::new();
    let mut world = World::new();
    world.runtimes = flaky.clone();
    world.installed("a", "1.0.0");
    let instance = world.instance("a");
    let broken = signed_with(&world.signing.clone(), "1.0.1", &script("a", "1.0.1"));
    world.publish(&[("a", "1.0.1", &broken)]);
    // From now on no runtime is made: the new file does not load, nor does the old one.
    flaky.fail_from_now();
    let answer = world.update("a");
    assert_eq!(answer.0, 500, "{}", answer.1);
    let reason = error_of(&answer);
    assert!(
        reason.contains("no memory") && reason.contains("does not load again"),
        "{reason}"
    );
    assert!(!world.running(&instance));
    assert!(
        world
            .messages(&instance)
            .iter()
            .any(|m| m.starts_with("Script recipe failed:")),
        "{:?}",
        world.messages(&instance)
    );
    // The recipe is failed as in spec 041: it restarts from the old file.
    let (_, rows) = world.get("/api/v1/system/resources");
    assert_eq!(rows["scripts"][0]["state"], "failed");
}

// ---- Uninstall (R3.6) ----

/// An instance as `recipes.json` gives it back at start, in the root zone.
fn restored(recipe: &str, id: &str) -> RecipeInstance {
    RecipeInstance {
        id: Id::parse(id).unwrap(),
        recipe_id: recipe.into(),
        params: format!(r#"{{"zone":"{ROOT}"}}"#).into(),
        enabled: true,
        created_at: T0,
    }
}

#[test]
fn an_uninstall_deletes_the_instances_in_every_zone_drops_the_runtime_and_removes_the_file() {
    let mut world = World::new();
    let (a, b) = (
        signed(&world.signing.clone(), "a", "1.0.0"),
        signed(&world.signing.clone(), "b", "1.0.0"),
    );
    world.serve(&[("a", "1.0.0", &a), ("b", "1.0.0", &b)]);
    assert_eq!(world.install_id("a").0, 200);
    let one = world.instance("a");
    let (status, zone) = world.call(
        "POST",
        "/api/v1/zones",
        Some(JSON),
        json!({ "name": "Annex" }).to_string().as_bytes(),
    );
    assert_eq!(status, 201, "{zone}");
    let (status, body) = world.call(
        "POST",
        "/api/v1/recipe-instances",
        Some(JSON),
        json!({ "recipeId": "a", "params": { "zone": zone["id"] } })
            .to_string()
            .as_bytes(),
    );
    assert_eq!(status, 201, "{body}");
    let two = body["id"].as_str().unwrap().to_string();
    // Another recipe's instance stays.
    assert_eq!(world.install_id("b").0, 200);
    let other = world.instance("b");
    let (hub, client) = {
        let hub = world.lock().live();
        let client = hub.connect().expect("a free slot");
        let message = json!({"type": "subscribe", "topics": ["recipes"]}).to_string();
        assert!(hub.subscribe(client, &message));
        (hub, client)
    };
    world.lock().take_recipes_cursor();
    let (status, body) = world.uninstall("a");
    assert_eq!((status, body), (200, json!({ "success": true })));
    assert!(!world.running(&one) && !world.running(&two));
    assert!(world.running(&other));
    assert_eq!(world.file("a"), None);
    assert!(world.lock().recipes().recipe_info("a").is_none());
    let (_, list) = world.get("/api/v1/recipe-instances");
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [other.as_str()]);
    assert_eq!(
        world.get(&format!("/api/v1/recipe-instances/{one}/log")).1,
        json!([]),
        "the log goes with the instance"
    );
    // The saver is asked to write recipes.json, without them.
    let mut home = world.lock();
    let mut cursor = home.take_recipes_cursor().expect("recipes.json to write");
    let mut text = String::new();
    while let Some(chunk) = cursor.next_chunk(&home).unwrap() {
        text.push_str(&chunk);
    }
    let written: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(written.as_array().unwrap().len(), 1);
    assert_eq!(written[0]["id"], other.as_str());
    // One removed event per instance.
    let batch: Value = serde_json::from_str(&hub.take_batch(client).unwrap()).unwrap();
    let removed: Vec<&str> = batch
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "recipe.instance.removed")
        .map(|e| e["instanceId"].as_str().unwrap())
        .collect();
    assert_eq!(removed.len(), 2, "{batch}");
    assert!(removed.contains(&one.as_str()) && removed.contains(&two.as_str()));
}

#[test]
fn an_uninstall_of_a_recipe_script_or_nothing_is_refused() {
    let mut world = World::new();
    let clock = world.script_clock();
    world
        .lock()
        .load_script_recipe(&script("local", "1.0.0"), Arc::new(SystemRuntimes), clock)
        .unwrap();
    refused(world.uninstall("local"), 409, "local is built in");
    refused(world.uninstall("nothing"), 404, "Recipe not found: nothing");
}

#[test]
fn an_uninstall_removes_a_refused_file_too() {
    let mut world = World::new();
    world.storage.put("scripts/bad.srs", b"not a recipe file at all, and long enough to pass the length check of a header but no more");
    world.lock().load_recipes(vec![restored(
        "bad",
        "22222222-2222-4222-8222-222222222222",
    )]);
    world.boot();
    assert_eq!(world.lock().resources().scripts[0].state, "refused");
    assert_eq!(world.lock().recipes().instances().count(), 1);
    assert_eq!(world.uninstall("bad").0, 200);
    // The instance the refused file left goes with it.
    assert_eq!(world.lock().recipes().instances().count(), 0);
    assert!(world.lock().recipes_changed());
    assert!(world.storage.file("scripts/bad.srs").is_none());
    assert!(world.lock().resources().scripts.is_empty());
}

#[test]
fn a_removal_that_fails_is_500_the_runtime_stays_dropped_and_the_file_loads_at_the_next_boot() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    world.storage.fail_removals.set(true);
    refused(world.uninstall("a"), 500, "Could not remove the recipe");
    assert!(
        world.lock().recipes().recipe_info("a").is_none(),
        "the runtime stays dropped"
    );
    assert!(world.file("a").is_some());
    // A new program: the file loads again.
    let mut again = World::new();
    again
        .storage
        .put("scripts/a.srs", &world.file("a").unwrap());
    // (the other world has another key: sign again with this one)
    let bytes = signed(&again.signing.clone(), "a", "1.0.0");
    again.storage.put("scripts/a.srs", &bytes);
    let lines = again.boot();
    assert!(
        lines[0].starts_with("recipes: script a 1.0.0 installed"),
        "{lines:?}"
    );
}

// ---- The boot load (R3.7) ----

#[test]
fn at_boot_the_files_are_read_in_id_order_and_a_changed_file_is_refused_and_left_in_place() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let a = signed(&signing, "a", "1.0.0");
    let b = signed(&signing, "b", "1.0.0");
    let mut changed = a.clone();
    *changed.last_mut().unwrap() ^= 1;
    world.storage.put("scripts/b.srs", &b);
    world.storage.put("scripts/a.srs", &changed);
    let lines = world.boot();
    assert_eq!(lines.len(), 2);
    assert!(
        lines[0].starts_with("recipes: script a: Recipe SHA-256"),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with("recipes: script b 1.0.0 installed"),
        "{lines:?}"
    );
    assert_eq!(
        world.storage.file("scripts/a.srs"),
        Some(changed),
        "left in place"
    );
    let rows = world.lock().resources().scripts;
    assert_eq!((rows[0].id.as_str(), rows[0].state), ("b", "ready"));
    assert_eq!((rows[1].id.as_str(), rows[1].state), ("a", "refused"));
    assert!(
        rows[1]
            .reason
            .as_deref()
            .unwrap()
            .starts_with("Recipe SHA-256")
    );
    assert_eq!(world.version_of("b"), Some("1.0.0".to_string()));
}

#[test]
fn at_boot_a_format_2_file_a_fifth_file_and_a_file_over_the_memory_rule_are_refused() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let mut two = signed(&signing, "a", "1.0.0");
    two[8] = 2;
    world.storage.put("scripts/a.srs", &two);
    world
        .storage
        .put("scripts/b.srs", &signed(&signing, "b", "1.0.0"));
    world
        .storage
        .put("scripts/c.srs", &signed(&signing, "c", "1.0.0"));
    for id in ["d", "e", "f", "g"] {
        world
            .storage
            .put(&format!("scripts/{id}.srs"), &signed(&signing, id, "1.0.0"));
    }
    let lines = world.boot();
    assert!(lines[0].contains("format 2 is not supported"), "{lines:?}");
    assert!(
        lines[1].starts_with("recipes: script b 1.0.0 installed"),
        "{lines:?}"
    );
    // The refused file takes a place of the four: three installed, then the others refused.
    for line in &lines[1..=3] {
        assert!(line.contains(" installed"), "{lines:?}");
    }
    for line in &lines[4..=6] {
        assert!(
            line.contains(&format!(
                "Too many script recipes (at most {MAX_SCRIPT_RECIPES})"
            )),
            "{lines:?}"
        );
    }

    // Over the memory rule.
    let mut world = World::new();
    let signing = world.signing.clone();
    world
        .storage
        .put("scripts/a.srs", &signed(&signing, "a", "1.0.0"));
    world
        .probe
        .set(64 * 1024, SYSTEM_MEMORY_LIMIT + SCRIPT_PSRAM_RESERVE - 1);
    let lines = world.boot();
    assert!(lines[0].contains("Not enough memory for a"), "{lines:?}");
}

#[test]
fn a_lone_new_file_is_promoted_at_boot_and_a_tmp_file_is_ignored() {
    let mut world = World::new();
    let signing = world.signing.clone();
    world
        .storage
        .put("scripts/a.srs", &signed(&signing, "a", "1.0.0"));
    world.storage.cut_save("scripts/a.srs");
    world.storage.put("scripts/b.srs.tmp", b"cut short");
    let lines = world.boot();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("recipes: script a 1.0.0 installed"),
        "{lines:?}"
    );
    assert!(world.storage.file("scripts/a.srs").is_some(), "promoted");
}

#[test]
fn an_enabled_instance_of_an_installed_recipe_runs_after_the_restart() {
    let mut world = World::new();
    let signing = world.signing.clone();
    world
        .storage
        .put("scripts/a.srs", &signed(&signing, "a", "1.0.0"));
    let instance = RecipeInstance {
        id: Id::parse("22222222-2222-4222-8222-222222222222").unwrap(),
        recipe_id: "a".into(),
        params: format!(r#"{{"zone":"{ROOT}"}}"#).into(),
        enabled: true,
        created_at: T0,
    };
    // As the program starts: the instances are read, the files loaded, then the instances start.
    world.lock().load_recipes(vec![instance]);
    world.boot();
    world.lock().start_recipes(T0, T0);
    assert!(world.running("22222222-2222-4222-8222-222222222222"));
}

#[test]
fn an_installed_id_that_a_recipe_script_also_has_is_refused_at_boot_the_script_wins() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let clock = world.script_clock();
    world
        .lock()
        .load_script_recipe(&script("a", "0.1.0"), Arc::new(SystemRuntimes), clock)
        .unwrap();
    world
        .storage
        .put("scripts/a.srs", &signed(&signing, "a", "1.0.0"));
    let lines = world.boot();
    assert!(lines[0].ends_with("a: built in"), "{lines:?}");
    assert!(world.storage.file("scripts/a.srs").is_some());
    assert_eq!(world.version_of("a"), None);
    let rows = world.lock().resources().scripts;
    assert_eq!(
        rows.iter()
            .find(|r| r.state == "refused")
            .unwrap()
            .reason
            .as_deref(),
        Some("built in")
    );
}

#[test]
fn an_install_or_an_upload_over_a_refused_file_replaces_it() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let good = signed(&signing, "a", "1.0.0");
    let mut changed = good.clone();
    *changed.last_mut().unwrap() ^= 1;
    world.storage.put("scripts/a.srs", &changed);
    world.boot();
    assert_eq!(world.lock().resources().scripts[0].state, "refused");
    let (status, body) = world.upload(&good);
    assert_eq!(status, 200, "{body}");
    assert_eq!(world.storage.file("scripts/a.srs"), Some(good));
    let rows = world.lock().resources().scripts;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, "ready");
}

#[test]
fn the_motion_light_script_given_by_the_program_is_built_in_for_every_source() {
    // On the Mac, `--recipe-script recipes/scripts/motion-light.js` (spec 046): its id is taken.
    let mut world = World::new();
    let signing = world.signing.clone();
    let clock = world.script_clock();
    world
        .lock()
        .load_script_recipe(MOTION_LIGHT, Arc::new(SystemRuntimes), clock)
        .unwrap();
    let file = signed_with(&signing, "1.1.2", MOTION_LIGHT);
    world.serve(&[("motion-light", "1.1.2", &file)]);
    refused(
        world.install_id("motion-light"),
        409,
        "motion-light is built in",
    );
    refused(world.upload(&file), 409, "motion-light is built in");
    refused(
        world.uninstall("motion-light"),
        409,
        "motion-light is built in",
    );
    // An installed `scripts/motion-light.srs` beside it: refused at boot, the program's script runs.
    world.storage.put("scripts/motion-light.srs", &file);
    let lines = world.boot();
    assert!(lines[0].ends_with("motion-light: built in"), "{lines:?}");
    assert_eq!(world.version_of("motion-light"), None);
    assert!(world.storage.file("scripts/motion-light.srs").is_some());
}

// ---- The safe mode (R3.8) ----

fn safe(world: &World) {
    world.lock().set_supervision(Status {
        mode: Mode::Safe,
        ..Status::default()
    });
}

#[test]
fn in_the_safe_mode_no_file_is_loaded_and_each_is_shown_refused() {
    let mut world = World::new();
    let signing = world.signing.clone();
    world
        .storage
        .put("scripts/a.srs", &signed(&signing, "a", "1.0.0"));
    safe(&world);
    let lines = world.boot();
    assert!(lines[0].ends_with("a: Safe mode: not loaded"), "{lines:?}");
    assert_eq!(world.version_of("a"), None);
    let rows = world.lock().resources().scripts;
    assert_eq!(rows[0].reason.as_deref(), Some("Safe mode: not loaded"));
}

#[test]
fn in_the_safe_mode_the_uninstall_goes_through_and_install_update_and_refresh_are_503() {
    let mut world = World::new();
    let signing = world.signing.clone();
    world
        .storage
        .put("scripts/a.srs", &signed(&signing, "a", "1.0.0"));
    world
        .lock()
        .load_recipes(vec![restored("a", "22222222-2222-4222-8222-222222222222")]);
    safe(&world);
    world.boot();
    let a = signed(&signing, "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    for answer in [
        world.install_id("a"),
        world.upload(&a),
        world.update("a"),
        world.refresh(),
    ] {
        refused(answer, 503, "Safe mode: the engine is stopped");
    }
    // The GETs answer.
    assert_eq!(world.get("/api/v1/plugins/store").0, 200);
    assert_eq!(world.get("/api/v1/system/resources").0, 200);
    // The uninstall removes the refused file and the recipe's instance.
    assert_eq!(world.uninstall("a").0, 200);
    assert!(world.storage.file("scripts/a.srs").is_none());
    assert_eq!(world.lock().recipes().instances().count(), 0);
    assert!(world.lock().recipes_changed());
}

// ---- The factory reset (R3.9) ----

#[test]
fn a_factory_reset_drops_the_runtimes_and_the_saver_removes_the_files() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    world.instance("a");
    let (status, _) = world.call("POST", "/api/v1/system/factory-reset", Some(JSON), b"{}");
    assert_eq!(status, 204);
    assert!(
        world.lock().recipes().recipe_info("a").is_none(),
        "the runtime is dropped"
    );
    assert!(world.file("a").is_some(), "the file waits for the saver");
    // The saver, on its own thread, removes it and reads the figures.
    let mut saver = Saver::new(&world.storage, T0);
    saver.tick(&world.home, T0 + 1, true).unwrap();
    assert!(world.file("a").is_none());
    assert_eq!(
        world.lock().resources().storage.used,
        (&world.storage).usage().unwrap().0
    );
}

#[test]
fn a_removal_the_saver_cannot_make_is_tried_again_after_the_interval() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    assert_eq!(
        world
            .call("POST", "/api/v1/system/factory-reset", Some(JSON), b"{}")
            .0,
        204
    );
    world.storage.fail_removals.set(true);
    let mut saver = Saver::new(&world.storage, T0).with_sink(|_| {});
    saver.tick(&world.home, T0 + 1, false).unwrap();
    assert!(world.file("a").is_some());
    world.storage.fail_removals.set(false);
    // Not before the retry time, then removed.
    saver.tick(&world.home, T0 + 2, false).unwrap();
    assert!(world.file("a").is_some());
    saver.tick(&world.home, T0 + 70_000, false).unwrap();
    assert!(world.file("a").is_none());
}

// ---- The one-job rule (R3.1) ----

/// A minute and a second later: the next refresh is not paced (R2.4).
fn a_minute_later(world: &World) {
    world.clock.set(world.clock.get() + 61 * SECOND);
}

/// A recipe job's ticket, held: the job is "running".
fn held_job(world: &World) -> Reply {
    let reply = world.ask(
        "POST",
        "/api/v1/plugins/install",
        Some(JSON),
        br#"{"repo":"o/x"}"#,
    );
    assert!(matches!(reply, Reply::RecipeJob(_)), "{reply:?}");
    reply
}

fn held_refresh(world: &World) -> Reply {
    let reply = world.ask("POST", "/api/v1/plugins/store/refresh", None, b"");
    assert!(matches!(reply, Reply::CatalogueRefresh(_)), "{reply:?}");
    reply
}

#[test]
fn a_recipe_job_is_refused_while_another_job_or_a_check_or_a_pull_runs() {
    let mut world = World::new();
    let job = held_job(&world);
    refused(
        world.install_id("y"),
        409,
        "Recipe install already in progress",
    );
    refused(world.update("y"), 409, "Recipe install already in progress");
    refused(
        world.uninstall("y"),
        409,
        "Recipe install already in progress",
    );
    drop(job);
    let refresh = held_refresh(&world);
    refused(
        world.install_id("y"),
        409,
        "Recipe catalogue refresh already in progress",
    );
    drop(refresh);
    let check = world.lock().release_ticket(true, T0).unwrap();
    refused(
        world.install_id("y"),
        409,
        "Update check already in progress",
    );
    drop(check);
    let pull = world.lock().release_ticket(false, T0).unwrap();
    refused(world.install_id("y"), 409, "Update already in progress");
    a_minute_later(&world);
    refused(world.refresh(), 409, "Update already in progress");
    drop(pull);
    // The automatic check holds the same session.
    assert_eq!(
        world
            .put_json("/api/v1/settings", json!({ "update.autoCheck": "true" }))
            .0,
        200
    );
    let auto = world.lock().auto_check_ticket(T0, 700_000).expect("due");
    refused(
        world.install_id("y"),
        409,
        "Update check already in progress",
    );
    drop(auto);
    // Idle again: the job is admitted (and goes on to the catalogue).
    let reply = world.ask(
        "POST",
        "/api/v1/plugins/install",
        Some(JSON),
        br#"{"repo":"solite-recipes/y"}"#,
    );
    assert!(matches!(reply, Reply::RecipeJob(_)));
}

#[test]
fn a_refresh_is_refused_while_a_job_another_refresh_a_check_or_a_pull_runs() {
    let mut world = World::new();
    let job = held_job(&world);
    refused(world.refresh(), 409, "Recipe install already in progress");
    drop(job);
    let refresh = held_refresh(&world);
    a_minute_later(&world);
    refused(
        world.refresh(),
        409,
        "Recipe catalogue refresh already in progress",
    );
    drop(refresh);
    let check = world.lock().release_ticket(true, T0).unwrap();
    refused(world.refresh(), 409, "Update check already in progress");
    drop(check);
}

#[test]
fn a_check_and_a_pull_are_refused_while_a_recipe_job_or_a_refresh_runs() {
    let mut world = World::new();
    for refresh in [false, true] {
        let hold = if refresh {
            held_refresh(&world)
        } else {
            held_job(&world)
        };
        refused(
            world.post("/api/v1/system/version/check"),
            409,
            "Update check already in progress",
        );
        let pull = world.ask("POST", "/api/v1/system/update", None, b"");
        refused(whole(pull), 409, "Update already in progress");
        assert!(world.lock().auto_check_ticket(T0, 700_000).is_none());
        drop(hold);
    }
    // Free again.
    assert!(matches!(
        world.ask("POST", "/api/v1/system/version/check", None, b""),
        Reply::VersionCheck(_)
    ));
}

fn update_world() -> (World, Updates, ed25519_dalek::SigningKey) {
    let firmware = new_key();
    let (updates, line) = Updates::from_key_text(Some(&public_hex(&firmware)));
    assert!(updates.has_key(), "{line}");
    let world = World::new();
    world.lock().set_update_status(updates.status());
    (world, updates, firmware)
}

fn firmware_file(signing: &SigningKey, version: Version, image: &[u8]) -> Vec<u8> {
    let signed = signed_bytes(version, u32::try_from(image.len()).unwrap(), &sha256(image));
    let mut out = signed.to_vec();
    out.extend_from_slice(&signing.sign(&signed).to_bytes());
    out.extend_from_slice(image);
    out
}

/// A reader that gives its head, says it is waiting, then blocks until it is released.
struct Blocking {
    head: Cursor<Vec<u8>>,
    waiting: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

impl Read for Blocking {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = self.head.read(out)?;
        if count > 0 {
            return Ok(count);
        }
        let _ = self.waiting.send(());
        let _ = self.release.recv();
        Ok(0)
    }
}

#[test]
fn a_recipe_job_is_refused_while_an_update_is_received_or_installed_and_waits_for_the_restart() {
    let (mut world, updates, firmware) = update_world();
    let updates = Arc::new(updates);
    let bytes = firmware_file(&firmware, Version::new(9, 0, 0), &vec![7u8; 5000]);
    let (waiting, is_waiting) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let thread = {
        let updates = Arc::clone(&updates);
        let head = bytes[..HEADER_BYTES + 100].to_vec();
        std::thread::spawn(move || {
            let mut source = Blocking {
                head: Cursor::new(head),
                waiting,
                release: released,
            };
            updates.install(&mut source, &mut FakeAdapter::new());
        })
    };
    is_waiting.recv().unwrap();
    refused(world.install_id("y"), 409, "Update already in progress");
    refused(world.uninstall("y"), 409, "Update already in progress");
    release.send(()).unwrap();
    thread.join().unwrap();
    // A whole update, installed and waiting for the restart.
    let mut adapter = FakeAdapter::new();
    let response = updates.install(&mut Cursor::new(bytes), &mut adapter);
    assert_eq!(response.status, 200, "{response:?}");
    refused(
        world.install_id("y"),
        409,
        "An update is installed and waits for the restart",
    );
    // A refresh reads the catalogue and does not mind an update.
    assert!(matches!(held_refresh(&world), Reply::CatalogueRefresh(_)));
}

#[test]
fn an_upload_is_refused_while_a_recipe_job_runs_even_right_after_it_began() {
    let (world, updates, firmware) = update_world();
    let job = held_job(&world);
    // The upload's claim comes after the route let go of the lock: it reads the flag set with the
    // job's mark, with no window between the two.
    let bytes = firmware_file(&firmware, Version::new(9, 0, 0), &vec![7u8; 3000]);
    let response = updates.install(&mut Cursor::new(bytes.clone()), &mut FakeAdapter::new());
    assert_eq!(
        (response.status, response.body.as_deref()),
        (409, Some(r#"{"error":"Update already in progress"}"#))
    );
    // A refresh does not refuse the upload: both run.
    drop(job);
    let refresh = held_refresh(&world);
    let response = updates.install(&mut Cursor::new(bytes), &mut FakeAdapter::new());
    assert_eq!(response.status, 200, "{response:?}");
    drop(refresh);
}

#[test]
fn a_recipe_job_is_refused_while_a_backup_or_a_restore_runs_or_waits_and_a_refresh_is_not() {
    let mut world = World::new();
    let export = world.lock().backup_begin(false).unwrap();
    refused(world.install_id("y"), 409, "Backup already in progress");
    assert!(
        matches!(held_refresh(&world), Reply::CatalogueRefresh(_)),
        "both run"
    );
    drop(export);
    let restore = world.lock().backup_begin(true).unwrap();
    refused(world.install_id("y"), 409, "Backup already in progress");
    a_minute_later(&world);
    assert!(
        matches!(held_refresh(&world), Reply::CatalogueRefresh(_)),
        "both run"
    );
    drop(restore);
    let mut restore = world.lock().backup_begin(true).unwrap();
    restore.set_pending();
    refused(
        world.install_id("y"),
        409,
        "A restore waits for the restart",
    );
    a_minute_later(&world);
    assert!(matches!(held_refresh(&world), Reply::CatalogueRefresh(_)));
}

#[test]
fn a_backup_a_restore_a_restart_and_a_factory_reset_are_refused_during_a_recipe_job() {
    let mut world = World::new();
    let job = held_job(&world);
    refused(
        world.get("/api/v1/backup"),
        409,
        "Backup already in progress",
    );
    refused(
        whole(world.ask("POST", "/api/v1/backup", Some(OCTET), b"")),
        409,
        "Backup already in progress",
    );
    refused(
        whole(world.ask("POST", "/api/v1/system/restart", None, b"")),
        409,
        "Recipe install already in progress",
    );
    refused(
        world.call("POST", "/api/v1/system/factory-reset", Some(JSON), b"{}"),
        409,
        "Recipe install already in progress",
    );
    drop(job);
    let refresh = held_refresh(&world);
    refused(
        whole(world.ask("POST", "/api/v1/system/restart", None, b"")),
        409,
        "Recipe catalogue refresh already in progress",
    );
    // A refresh does not refuse a backup, a restore or a factory reset.
    assert!(matches!(
        world.ask("GET", "/api/v1/backup", None, b""),
        Reply::BackupExport(_)
    ));
    drop(refresh);
    assert!(whole_or_restart(world.ask(
        "POST",
        "/api/v1/system/restart",
        None,
        b""
    )));
}

fn whole_or_restart(reply: Reply) -> bool {
    matches!(reply, Reply::Restart)
}

// ---- Admission by memory (R4) ----

#[test]
fn an_install_that_would_take_the_psram_under_the_reserve_is_409_with_the_figures() {
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    let needed = SYSTEM_MEMORY_LIMIT + SCRIPT_PSRAM_RESERVE;
    world.probe.set(64 * 1024, needed - 1);
    let answer = world.install_id("a");
    assert_eq!(answer.0, 409, "{}", answer.1);
    assert_eq!(
        error_of(&answer),
        format!(
            "Not enough memory for a: {} KB free, {} KB needed",
            (needed - 1) / 1024,
            needed.div_ceil(1024)
        )
    );
    no_file_touched(&world);
    // Just over: admitted.
    world.probe.set(64 * 1024, needed);
    assert_eq!(world.install_id("a").0, 200);
    // The reserve is the same for an upload.
    let mut world = World::new();
    world.probe.set(64 * 1024, needed - 1);
    let b = signed(&world.signing.clone(), "b", "1.0.0");
    let answer = world.upload(&b);
    assert_eq!(answer.0, 409, "{}", answer.1);
    assert!(error_of(&answer).starts_with("Not enough memory for"));
}

#[test]
fn the_internal_ram_must_hold_the_job_thread_and_its_tls_session_else_503_with_the_figures() {
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    let needed = FETCH_STACK_BYTES + RECIPE_INTERNAL_MARGIN;
    world.probe.set(needed - 1, 1_500_000);
    for answer in [world.install_id("a"), world.refresh()] {
        assert_eq!(answer.0, 503, "{}", answer.1);
        assert_eq!(
            error_of(&answer),
            format!(
                "Not enough internal memory: largest free block {} KB, {} KB needed",
                (needed - 1) / 1024,
                needed / 1024
            )
        );
    }
    assert!(world.fetch.log.is_empty());
    world.probe.set(needed, 1_500_000);
    assert_eq!(world.refresh().0, 200);
}

#[test]
fn an_uninstall_needs_only_its_threads_stack_of_internal_ram() {
    let mut world = World::new();
    world.probe.set(64 * 1024, 1_500_000);
    world.installed("a", "1.0.0");
    // Between the stack and the stack plus the TLS margin: a download would be refused, the
    // removal goes through.
    world.probe.set(FETCH_STACK_BYTES, 1_500_000);
    assert_eq!(world.install_id("b").0, 503);
    assert_eq!(world.uninstall("a").0, 200);
    world.probe.set(64 * 1024, 1_500_000);
    world.installed("a", "1.0.0");
    // Under the stack: refused, with the 20 KB figure.
    world.probe.set(FETCH_STACK_BYTES - 1, 1_500_000);
    let answer = world.uninstall("a");
    assert_eq!(answer.0, 503, "{}", answer.1);
    assert_eq!(
        error_of(&answer),
        format!(
            "Not enough internal memory: largest free block {} KB, {} KB needed",
            (FETCH_STACK_BYTES - 1) / 1024,
            FETCH_STACK_BYTES / 1024
        )
    );
}

#[test]
fn with_the_job_thread_kept_an_install_needs_only_the_tls_margin() {
    let mut world = World::new();
    world.keep_job_thread();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    // 15 KB: under the margin, refused with the 16 KB figure.
    world.probe.set(15 * 1024, 1_500_000);
    let answer = world.install_id("a");
    assert_eq!(answer.0, 503, "{}", answer.1);
    assert_eq!(
        error_of(&answer),
        "Not enough internal memory: largest free block 15 KB, 16 KB needed"
    );
    assert!(world.fetch.log.is_empty());
    // 17 KB: far under the 36 KB of a thread made per job, and admitted.
    world.probe.set(17 * 1024, 1_500_000);
    assert_eq!(world.install_id("a").0, 200);
}

#[test]
fn with_the_job_thread_kept_an_uninstall_is_never_refused_for_internal_ram() {
    let mut world = World::new();
    world.probe.set(64 * 1024, 1_500_000);
    world.installed("a", "1.0.0");
    world.keep_job_thread();
    world.probe.set(1024, 1_500_000);
    assert_eq!(world.uninstall("a").0, 200);
}

#[test]
fn without_the_job_thread_kept_the_figures_stay_36_and_20_kb() {
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    world.probe.set(35 * 1024, 1_500_000);
    let answer = world.install_id("a");
    assert_eq!(answer.0, 503, "{}", answer.1);
    assert_eq!(
        error_of(&answer),
        "Not enough internal memory: largest free block 35 KB, 36 KB needed"
    );
    world.probe.set(64 * 1024, 1_500_000);
    world.installed("b", "1.0.0");
    world.probe.set(19 * 1024, 1_500_000);
    let answer = world.uninstall("b");
    assert_eq!(answer.0, 503, "{}", answer.1);
    assert_eq!(
        error_of(&answer),
        "Not enough internal memory: largest free block 19 KB, 20 KB needed"
    );
}

/// The world of the update's memory tests: runtimes that take 200 KB of the PSRAM while held, and
/// `a` 1.0.0 installed with the probe giving exactly what the rule needs once no runtime is held.
fn counting_world() -> (World, Arc<Counting>, usize) {
    let mut world = World::new();
    let needed = SYSTEM_MEMORY_LIMIT + SCRIPT_PSRAM_RESERVE;
    let counting = Counting::new(&world.probe, 200 * 1024);
    world.runtimes = counting.clone();
    world.probe.set(64 * 1024, needed);
    world.installed("a", "1.0.0");
    (world, counting, needed)
}

#[test]
fn an_update_counts_the_old_runtimes_memory_as_freed() {
    let (mut world, _, needed) = counting_world();
    // The old runtime holds 200 KB: the probe reads `needed - 200 KB` while it is held, `needed`
    // once it is dropped. The update is admitted only if the rule is asked after the drop.
    assert_eq!(
        world.probe.read().unwrap().stats.psram.unwrap().free,
        needed - 200 * 1024
    );
    let newer = signed(&world.signing.clone(), "a", "1.0.1");
    world.publish(&[("a", "1.0.1", &newer)]);
    assert_eq!(world.update("a").0, 200);
    // One byte short even with the old runtime gone: refused 409, and the old one runs again.
    let instance = world.instance("a");
    let newest = signed(&world.signing.clone(), "a", "1.0.2");
    world.publish(&[("a", "1.0.2", &newest)]);
    world.probe.set(64 * 1024, needed - 1);
    let answer = world.update("a");
    assert_eq!(answer.0, 409, "{}", answer.1);
    assert_eq!(world.version_of("a"), Some("1.0.1".to_string()));
    assert!(world.running(&instance));
}

#[test]
fn an_upload_that_updates_follows_the_catalogues_rules_with_one_new_runtime() {
    let (mut world, counting, needed) = counting_world();
    let instance = world.instance("a");
    let newer = signed(&world.signing.clone(), "a", "1.0.1");
    // Just under the rule, with the old runtime freed: refused, the old one runs again.
    world.probe.set(64 * 1024, needed - 1);
    let made = counting.made.load(Ordering::SeqCst);
    let answer = world.upload(&newer);
    assert_eq!(answer.0, 409, "{}", answer.1);
    assert!(error_of(&answer).starts_with("Not enough memory for a:"));
    assert_eq!(world.version_of("a"), Some("1.0.0".to_string()));
    assert!(world.running(&instance));
    // Exactly the rule: admitted, as the catalogue's update is. One new runtime is made (the
    // reload of the old one after the refusal made another), never a third beside the two held.
    world.probe.set(64 * 1024, needed);
    let before = counting.made.load(Ordering::SeqCst);
    assert!(before > made);
    let answer = world.upload(&newer);
    assert_eq!(answer.0, 200, "{}", answer.1);
    assert_eq!(counting.made.load(Ordering::SeqCst), before + 1);
    assert_eq!(world.version_of("a"), Some("1.0.1".to_string()));
    assert!(world.running(&instance));
}

#[test]
fn an_upload_is_refused_for_a_fifth_file_before_any_load_or_memory_check() {
    let (mut world, counting, needed) = counting_world();
    let signing = world.signing.clone();
    // The memory is short for any new runtime: a load or the memory rule would say so.
    world.probe.set(64 * 1024, 1024);
    let made = counting.made.load(Ordering::SeqCst);
    world.probe.set(64 * 1024, needed + 200 * 1024);
    let b = signed(&signing, "b", "1.0.0");
    let c = signed(&signing, "c", "1.0.0");
    let d = signed(&signing, "d", "1.0.0");
    let e = signed(&signing, "e", "1.0.0");
    assert_eq!(world.upload(&b).0, 200);
    // Each runtime held takes 200 KB of the PSRAM the probe reads: the rule is met each time.
    world.probe.set(64 * 1024, needed + 400 * 1024);
    assert_eq!(world.upload(&c).0, 200);
    world.probe.set(64 * 1024, needed + 600 * 1024);
    assert_eq!(world.upload(&d).0, 200);
    world.probe.set(64 * 1024, 1024);
    refused(
        world.upload(&e),
        409,
        &format!("Too many script recipes (at most {MAX_SCRIPT_RECIPES})"),
    );
    assert_eq!(
        counting.made.load(Ordering::SeqCst),
        made + 3,
        "only the runtimes of b, c and d were made"
    );
    // A script whose id cannot be read is 400 and loads nothing.
    let odd = signed_with(&signing, "1.0.0", "recipe(makeIt());");
    assert_eq!(world.upload(&odd).0, 400);
    assert_eq!(counting.made.load(Ordering::SeqCst), made + 3);
}

#[test]
fn the_id_of_an_upload_is_read_from_its_source_and_the_real_load_refuses_a_different_one() {
    let mut world = World::new();
    let signing = world.signing.clone();
    // A comment that names another id first: the guess is wrong, the load says so.
    let tricked = format!("// recipe({{ id: \"zzz\" }})\n{}", script("real", "1.0.0"));
    let answer = world.upload(&signed_with(&signing, "1.0.0", &tricked));
    assert_eq!(answer.0, 400, "{answer:?}");
    assert!(error_of(&answer).contains("declares the id real, not zzz"));
    assert!(world.lock().recipes().installed_version("real").is_none());
    assert!(world.file("real").is_none() && world.file("zzz").is_none());
    // An `id:` after a nested key that is not the recipe's: refused when it differs.
    let nested = r#"recipe({
  api: 1, name: "x", description: "d", version: "1.0.0",
  slots: { other: { type: "duration", defaultValue: "10m", id: "wrong" } },
  start: function (api) {}, id: "right",
});"#;
    let answer = world.upload(&signed_with(&signing, "1.0.0", nested));
    assert_eq!(answer.0, 400, "{answer:?}");
    assert!(world.lock().recipes().installed_version("wrong").is_none());
    assert!(world.file("wrong").is_none());
    assert!(world.file("right").is_none());
}

#[test]
fn an_update_whose_new_version_refuses_an_instances_settings_leaves_it_stopped_with_the_reason() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    let instance = world.instance("a");
    assert!(world.running(&instance));
    let newer = signed_with(
        &world.signing.clone(),
        "1.0.1",
        r#"recipe({
  api: 1, id: "a", name: "a", description: "A test recipe", version: "1.0.1",
  slots: { timeout: { type: "duration", defaultValue: "10m" },
           more: { type: "duration", required: true } },
  start: function (api) { api.log("started 1.0.1"); },
});"#,
    );
    world.publish(&[("a", "1.0.1", &newer)]);
    let (status, body) = world.update("a");
    assert_eq!(status, 200, "{body}");
    assert_eq!(world.version_of("a"), Some("1.0.1".to_string()));
    assert!(!world.running(&instance), "the settings are refused");
    let log = world.messages(&instance);
    assert!(
        log.iter().any(|m| m.starts_with("Failed to restart: ")),
        "{log:?}"
    );
}

#[test]
fn a_recipe_written_after_a_factory_reset_is_not_removed_by_the_saver_that_still_holds_its_name() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    let (status, _) = world.call("POST", "/api/v1/system/factory-reset", Some(JSON), b"{}");
    assert_eq!(status, 204);
    assert!(world.lock().has_removals(), "the name waits for the saver");
    // The same recipe is installed again before the saver runs (it was held).
    let signing = world.signing.clone();
    world.relogin();
    let again = signed(&signing, "a", "1.0.0");
    let answer = world.upload(&again);
    assert_eq!(answer.0, 200, "{answer:?}");
    let mut saver = Saver::new(&world.storage, T0);
    saver.tick(&world.home, T0 + 1, true).unwrap();
    assert!(world.file("a").is_some(), "the new file stays");
    assert!(!world.lock().has_removals());
}

// ---- The resources (R5) ----

#[test]
fn the_resources_answer_with_a_fake_probe_in_camel_case_with_the_scripts_rows() {
    let mut world = World::new();
    world.probe.set(80 * 1024, 1_900_000);
    world.installed("a", "1.0.0");
    let instance = world.instance("a");
    let _ = instance;
    let (status, body) = world.get("/api/v1/system/resources");
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["internal"],
        json!({ "free": 153600, "minFree": 92160, "largestBlock": 81920 })
    );
    assert_eq!(
        body["psram"],
        json!({ "free": 1_900_000, "minFree": 1_500_000, "total": 2_097_152, "reserve": SCRIPT_PSRAM_RESERVE })
    );
    assert!(body["storage"]["used"].as_u64().unwrap() > 0);
    assert_eq!(body["storage"]["total"], 1_048_576);
    let row = &body["scripts"][0];
    assert_eq!(row["id"], "a");
    assert_eq!(row["version"], "1.0.0");
    assert_eq!(row["source"], "installed");
    assert_eq!(row["state"], "ready");
    assert_eq!(row["reason"], Value::Null);
    assert_eq!(row["instances"], 1);
    assert!(row["held"].as_u64().unwrap() > 0);
    assert_eq!(row["limit"], SYSTEM_MEMORY_LIMIT);
    assert!(row["calls"].is_number() && row["averageUs"].is_number() && row["maxUs"].is_number());
}

#[test]
fn the_resources_of_a_platform_that_reads_no_memory_have_no_internal_and_no_psram() {
    let mut world = World::new();
    let clock = world.script_clock();
    world
        .lock()
        .load_script_recipe(&script("local", "0.1.0"), Arc::new(SystemRuntimes), clock)
        .unwrap();
    let (status, body) = world.get("/api/v1/system/resources");
    assert_eq!(status, 200);
    assert_eq!(body["internal"], Value::Null);
    assert_eq!(body["psram"], Value::Null);
    assert_eq!(body["storage"]["total"], Value::Null);
    assert_eq!(body["scripts"][0]["source"], "builtIn");
    assert_eq!(body["scripts"][0]["id"], "local");
}

#[test]
fn the_storage_figures_change_with_an_install_and_an_uninstall_and_the_get_reads_no_file() {
    let mut world = World::new();
    // The figures are read where the file is written: nothing before.
    assert_eq!(
        world.get("/api/v1/system/resources").1["storage"]["used"],
        0
    );
    world.installed("a", "1.0.0");
    let after_install = world.get("/api/v1/system/resources").1["storage"]["used"]
        .as_u64()
        .unwrap();
    assert_eq!(after_install, world.file("a").unwrap().len() as u64);
    // A GET reads no file, whatever the number of times.
    let reads = world.storage.reads.get();
    for _ in 0..3 {
        assert_eq!(world.get("/api/v1/system/resources").0, 200);
        assert_eq!(world.get("/api/v1/plugins/store").0, 200);
    }
    assert_eq!(world.storage.reads.get(), reads);
    assert_eq!(world.uninstall("a").0, 200);
    assert_eq!(
        world.get("/api/v1/system/resources").1["storage"]["used"],
        0
    );
}

// ---- The API (R2, R3) ----

#[test]
fn the_recipe_routes_without_a_session_are_401() {
    let world = World::new();
    for (method, path) in [
        ("GET", "/api/v1/plugins/store"),
        ("POST", "/api/v1/plugins/store/refresh"),
        ("POST", "/api/v1/plugins/install"),
        ("POST", "/api/v1/recipes/a/update"),
        ("POST", "/api/v1/recipes/a/uninstall"),
        ("GET", "/api/v1/system/resources"),
    ] {
        let reply = world.send(method, path, None, Some(JSON), b"{}", Some(2));
        let (status, body) = whole(reply);
        assert_eq!(status, 401, "{method} {path}: {body}");
    }
}

#[test]
fn the_install_route_takes_json_or_an_octet_stream_and_refuses_another_type() {
    let mut world = World::new();
    let a = signed(&world.signing.clone(), "a", "1.0.0");
    world.serve(&[("a", "1.0.0", &a)]);
    assert_eq!(world.install_id("a").0, 200);
    let b = signed(&world.signing.clone(), "b", "1.0.0");
    assert_eq!(world.upload(&b).0, 200);
    refused(
        world.call("POST", "/api/v1/plugins/install", Some("text/plain"), b"x"),
        415,
        "Unsupported Media Type",
    );
    refused(
        world.call("POST", "/api/v1/plugins/install", None, b"x"),
        415,
        "Unsupported Media Type",
    );
    refused(
        world.call("POST", "/api/v1/plugins/install", Some(JSON), b"{}"),
        400,
        "Invalid 'repo' field (expected owner/repo)",
    );
    refused(
        world.call("POST", "/api/v1/plugins/install", Some(JSON), b"[1]"),
        400,
        "Invalid 'repo' field (expected owner/repo)",
    );
    for method in ["GET", "PUT", "DELETE"] {
        assert_eq!(
            world.call(method, "/api/v1/plugins/install", None, b"").0,
            405,
            "{method}"
        );
    }
    // `store` and `install` do not meet Sowel's `/recipes/:recipeId`.
    assert_eq!(world.get("/api/v1/recipes/a").0, 200);
    assert_eq!(world.get("/api/v1/plugins/store").0, 200);
    assert_eq!(world.get("/api/v1/plugins/install").0, 405);
}

#[test]
fn the_plain_handler_does_not_run_a_recipe_job() {
    let mut home = Home::new(T0);
    let mut random = counter();
    let request = Request {
        method: "POST",
        path: "/api/v1/plugins/install",
        host: Some(HOST),
        port: PORT,
        reach: Reach::Loopback,
        content_type: Some(JSON),
        authorization: None,
        content_length: None,
        body: br#"{"id":"a"}"#,
    };
    let response = solite_core::api::handle(&mut home, &request, T0, &mut *random);
    assert_eq!(response.status, 501);
}

// ---- The thread discipline (R6.3) ----

#[test]
fn the_flash_is_written_and_removed_only_outside_the_script_thread_steps() {
    // `Guarded` panics when the storage is touched from inside a step on the script thread: a whole
    // install, update and uninstall through it prove no step writes the flash.
    let mut world = World::new();
    world.installed("a", "1.0.0");
    let newer = signed(&world.signing.clone(), "a", "1.0.1");
    world.publish(&[("a", "1.0.1", &newer)]);
    world.clock.set(world.clock.get() + 61 * SECOND);
    assert_eq!(world.update("a").0, 200);
    assert_eq!(world.uninstall("a").0, 200);
    assert!(world.hops.count.get() >= 5);
}

#[test]
fn an_install_by_repo_follows_sowels_answers_and_ignores_confirmed_and_the_digest() {
    let mut world = World::new();
    let signing = world.signing.clone();
    let a = signed(&signing, "a", "1.0.0");
    let mut with = entry("a", "1.0.0", &a, &file_url("a", "1.0.0"));
    with["repo"] = json!("someone/sowel-recipe-a");
    world
        .fetch
        .serve(DEFAULT_CATALOGUE_URL, Answer::Body(catalogue_of(&[with])));
    world
        .fetch
        .serve(&file_url("a", "1.0.0"), Answer::Body(a.clone()));
    let install = |world: &mut World, body: &str| {
        world.call(
            "POST",
            "/api/v1/plugins/install",
            Some(JSON),
            body.as_bytes(),
        )
    };
    refused(
        install(&mut world, r#"{"repo":"someone/other"}"#),
        404,
        "Plugin not found",
    );
    for bad in [
        r#"{"repo":"noslash"}"#,
        "{}",
        r#"{"repo":3}"#,
        r#"{"id":"a"}"#,
    ] {
        refused(
            install(&mut world, bad),
            400,
            "Invalid 'repo' field (expected owner/repo)",
        );
    }
    // By the derived repo, the id's old way, nothing is found: the entry has its own.
    refused(
        install(&mut world, r#"{"repo":"solite-recipes/a"}"#),
        404,
        "Plugin not found",
    );
    assert_eq!(
        install(
            &mut world,
            r#"{"repo":"someone/sowel-recipe-a","confirmed":true,"expectedSha256":"00"}"#
        ),
        (200, json!({ "success": true }))
    );
    assert_eq!(
        world.get("/api/v1/plugins").1[0]["manifest"]["repo"],
        "someone/sowel-recipe-a"
    );
    refused(
        install(&mut world, r#"{"repo":"someone/sowel-recipe-a"}"#),
        409,
        "Recipe already installed",
    );
}

#[test]
fn an_update_takes_no_body_or_sowels_optional_one() {
    let mut world = World::new();
    world.installed("a", "1.0.0");
    let signing = world.signing.clone();
    let a2 = signed(&signing, "a", "1.1.0");
    world.publish(&[("a", "1.1.0", &a2)]);
    let reply = world.call(
        "POST",
        "/api/v1/plugins/a/update",
        Some(JSON),
        br#"{"confirmed":true}"#,
    );
    assert_eq!(reply, (200, json!({ "success": true })));
    assert_eq!(world.version_of("a"), Some("1.1.0".to_string()));
    refused(world.update("a"), 409, "a 1.1.0 is not newer than 1.1.0");
    assert_eq!(world.update("nothing").0, 404);
}
