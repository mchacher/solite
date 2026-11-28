//! The script phase of the image QEMU runs (feature `scripts`, spec 041 R6.2), after the engine's
//! own line (its gate, D27, is untouched: no script is loaded before it, the home's recipe
//! instances are kept stopped, spec 046). The image carries no script (spec 042 R3.10): the phase
//! reads `recipes/scripts/motion-light.js` at build time, for itself only. First it makes an
//! install (`install_step`): that file signed at boot with a throwaway key, installed through an
//! in-memory fetcher on a `recipe` thread, loaded again as at start into a home of no use, and
//! uninstalled; the thread logs `recipe: thread stack <n> of 20480 bytes never used`. On the
//! realistic home the engine just ran:
//!
//! 1. loads `motion-light`, starts the home's three kept `motion-light` instances (they were
//!    stopped for want of the recipe, as a box that has not installed it) with their own settings,
//!    and plays the scenario again (a motion event); the heap then (`heap peak` of the line below,
//!    taken before the depth script) is the figure the second gate reads, with the instances running;
//! 2. loads a script that recurses as deep as the stack limit allows and calls `get` and `order` at
//!    its deepest (`fixtures/depth-host.js`), and starts an instance of it twice: on this (the
//!    engine) thread, and on a `script` job thread, as the radio image's HTTP server runs a
//!    request that changes something (the router's frames included, `handle_stream`);
//! 3. logs `recipes: script motion-light ...` (the memory taken by the phase, the lowest free
//!    internal RAM, the two stacks), and the `scripts:` line the radio image prints.
//!
//! Then, outside that line's window, the same with `MAX_INSTANCES_PER_SCRIPT` instances (4, the most one
//! script recipe holds), to read the script's memory at the bound: `script sizes: ...`.
//!
//! `scripts/qemu.sh` fails when the line is missing, the three instances' peak (the `scripts:`
//! line's) passes 75 % of the script memory limit, the internal RAM's minimum is under 64 KB, or
//! either stack keeps less than 2 KB unused.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ed25519_dalek::{Signer, SigningKey};
use solite_core::api::{Reach, Reply, Request, handle_stream};
use solite_core::backup::Pacing;
use solite_core::devices::lock;
use solite_core::engine::{self, Clocks, Saver, Storage};
use solite_core::home::Home;
use solite_core::radio::Radio;
use solite_core::recipes::install::catalogue::DEFAULT_CATALOGUE_URL;
use solite_core::recipes::install::file::{digest, signed_bytes};
use solite_core::recipes::install::{JobEnv, Op, RecipeInstalls, ScriptThread};
use solite_core::script_phase_line;
use solite_core::update::release::{Fetched, Fetcher, Limits};
use solite_core::update::{HEADER_BYTES, Version};
use solite_core::{
    ScriptPhase,
    recipes::{MAX_INSTANCES_PER_SCRIPT, ScriptClock},
};
use solite_sim::Scenario;

use super::{
    BASE, FatStorage, MEMORY_NAME, RunStorage, SCENARIO, SCENARIO_NAME, SPEED, STACK_BYTES,
    SimTransport, WaitedClock, Watched, fill, measure, memory, wall_ms,
};
use crate::job_thread::{self, FETCH_STACK_BYTES, StackPlace};
use crate::scripts::{self, SCRIPT_THREAD_STACK_BYTES};

/// QEMU's emulated CPU runs the engine far slower than the box's (the first load of a script, which
/// takes a few ms on the box, passes 100 ms there): the scripts' time limits read a clock running
/// this many times slower, so that the phase measures memory and stacks, which do not depend on the
/// speed. The time limits themselves are tested on the Mac.
const QEMU_SLOWDOWN: u64 = 50;

const MOTION_SCRIPT: &str = "motion-light";
/// The script of the phase: read here, at build time, never carried by an image that is not this one.
const MOTION_SCRIPT_SOURCE: &str = include_str!("../../recipes/scripts/motion-light.js");
/// A script of about 32 KB (spec 053), `crates/core/tests/fixtures/scripts/big-recipe.js`: its memory
/// at the source bound, with `MAX_INSTANCES_PER_SCRIPT` instances (the `script sizes: big-recipe` line).
const BIG_SCRIPT: &str = include_str!("../../crates/core/tests/fixtures/scripts/big-recipe.js");
const DEPTH_SCRIPT: &str = include_str!("../fixtures/depth-host.js");
/// The realistic home's zone and first light (`fixtures/recipes.json`): the depth script's slots.
const ZONE: &str = "00000191-5a5a-4a5a-9a5a-5a5a5a5a5a5a";
const LIGHT: &str = "000002ab-5a5a-4a5a-9a5a-5a5a5a5a5a5a";

/// Plays the realistic scenario again on the home as it is, as the first run did (the radio is a
/// new simulated one; the saves are run and dropped).
fn replay(home: &Mutex<Home>, clocks: &Clocks, start: Instant) -> Result<(), String> {
    let scenario = Scenario::parse(SCENARIO, SCENARIO_NAME)?;
    let memory_file = format!("{BASE}/{MEMORY_NAME}");
    let stop = Arc::new(AtomicBool::new(false));
    let mut inner = SimTransport::for_scenario(
        &scenario,
        Some(Path::new(&memory_file)),
        SPEED,
        WaitedClock {
            waited_ms: Cell::new(0),
        },
    )?;
    inner.keep_memory_in_ram();
    inner.hold();
    let go = Arc::new(AtomicBool::new(false));
    let transport = Watched {
        inner,
        go: Arc::clone(&go),
        started: false,
        stop: Arc::clone(&stop),
    };
    let mut radio = Radio::new(transport, move || {
        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
    });
    radio.identify().map_err(|e| format!("radio: {e}"))?;
    radio.listen().map_err(|e| format!("radio: {e}"))?;
    let mut saver = Saver::new(
        RunStorage {
            inner: FatStorage::new(),
            to_flash: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(AtomicUsize::new(0)),
        },
        wall_ms(),
    );
    go.store(true, std::sync::atomic::Ordering::SeqCst);
    engine::run(
        &mut radio,
        home,
        &mut saver,
        clocks,
        &stop,
        &mut fill,
        false,
        &mut |_| {},
    )?;
    Ok(())
}

/// The version the script declares (`version: "x.y.z"`): the installed file's header must say it.
fn script_version(source: &str) -> Result<Version, String> {
    let at = source
        .find("version: \"")
        .ok_or("script phase: the script declares no version")?;
    let text = &source[at + "version: \"".len()..];
    let end = text.find('"').ok_or("script phase: bad version")?;
    Version::parse(&text[..end]).ok_or_else(|| "script phase: bad version".to_string())
}

/// The params text of each `motion-light` instance the home keeps, with their ids.
fn motion_light_instances(home: &Mutex<Home>) -> Vec<(String, String)> {
    lock(home)
        .recipes()
        .instances()
        .filter(|i| i.recipe_id == "motion-light")
        .map(|i| (i.id.to_string(), i.params.to_string()))
        .collect()
}

fn create(home: &Mutex<Home>, recipe: &str, params: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(params).map_err(|e| e.to_string())?;
    lock(home)
        .create_recipe_instance(recipe, value, wall_ms(), &mut fill)
        .map(|i| i.id.to_string())
        .map_err(|f| format!("script phase: {recipe}: {} {}", f.status, f.message))
}

fn delete_all(home: &Mutex<Home>, recipe: &str) {
    let ids: Vec<String> = lock(home)
        .recipes()
        .instances()
        .filter(|i| i.recipe_id == recipe)
        .map(|i| i.id.to_string())
        .collect();
    for id in ids {
        if let Err(f) = lock(home).delete_recipe_instance(&id) {
            log::warn!("script phase: delete {id}: {}", f.message);
        }
    }
}

/// What the last instance of `recipe` logged (the depth script says how deep it went).
fn last_log(home: &Mutex<Home>, id: &str) -> String {
    lock(home)
        .recipes()
        .log(id, 1)
        .first()
        .map_or_else(String::new, |entry| entry.message.to_string())
}

/// The counters of `id`: what its runtime holds and the most it held.
fn stats_of(home: &Mutex<Home>, id: &str) -> Option<(usize, usize)> {
    lock(home)
        .script_stats()
        .into_iter()
        .find(|(name, _)| name == id)
        .map(|(_, s)| (s.held, s.peak))
}

// ---- The install step (spec 042) ----

/// The wall time the install step passes to the core: a clock that is set (the QEMU image's is not).
const INSTALL_WALL_MS: u64 = 1_795_252_323_120;
/// A recipe's address in the in-memory catalogue.
fn recipe_url(id: &str) -> String {
    format!("https://recipes.example.test/{id}.srs")
}
/// The recipes the install step installs: `motion-light` (the phase's own) and `schedule-on-off`
/// (the one with the most slots and texts, whose load once took 94 KB of the internal RAM).
const SCHEDULE_SCRIPT: &str = "schedule-on-off";
const SCHEDULE_SCRIPT_SOURCE: &str = include_str!("../../recipes/scripts/schedule-on-off.js");

/// The files of the install step, in memory: the flash is not the subject.
#[derive(Default)]
struct MemoryFiles {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl Storage for MemoryFiles {
    fn read(&self, _name: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn write(
        &self,
        _name: &str,
        _next: &mut dyn FnMut() -> std::io::Result<Option<String>>,
    ) -> Result<(), String> {
        Err("the install step writes bytes only".to_string())
    }

    fn locate(&self, name: &str) -> String {
        format!("memory://{name}")
    }

    fn read_bytes(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(lock(&self.files).get(name).cloned())
    }

    fn write_bytes(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        lock(&self.files).insert(name.to_string(), bytes.to_vec());
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<(), String> {
        lock(&self.files).remove(name);
        Ok(())
    }

    fn list(&self, folder: &str) -> Result<Vec<String>, String> {
        let prefix = format!("{folder}/");
        Ok(lock(&self.files)
            .keys()
            .filter_map(|key| key.strip_prefix(&prefix).map(str::to_string))
            .collect())
    }
}

/// The fetch adapter of the step: the catalogue and the signed file, in memory.
struct InMemory {
    catalogue: Vec<u8>,
    /// The files, by address.
    files: Vec<(String, Vec<u8>)>,
}

impl Fetcher for InMemory {
    fn get(&mut self, url: &str, _idle_ms: u64) -> Result<Fetched, String> {
        let body = |bytes: &[u8]| Fetched::Body {
            length: Some(bytes.len() as u64),
            etag: None,
            body: Box::new(Cursor::new(bytes.to_vec())),
        };
        if url == DEFAULT_CATALOGUE_URL {
            return Ok(body(&self.catalogue));
        }
        match self.files.iter().find(|(address, _)| address == url) {
            Some((_, file)) => Ok(body(file)),
            None => Ok(Fetched::Status(404)),
        }
    }
}

/// The clock and the pause of the step, on std (the box's kind).
struct StepPacing(Instant);

impl Pacing for StepPacing {
    fn now_ms(&mut self) -> u64 {
        u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn pause(&mut self) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    fn wait_ms(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

/// A recipe job's script steps, on a `script` job thread (32 KB, PSRAM) as the box runs
/// them: the `recipe` thread's 20 KB cannot hold a script's stack.
struct ScriptJobThread;

impl ScriptThread for ScriptJobThread {
    fn run(&self, job: &mut dyn FnMut()) {
        if let Err(e) =
            job_thread::run_borrowed(c"script", SCRIPT_THREAD_STACK_BYTES, StackPlace::Psram, job)
        {
            log::error!("script phase: {e}");
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The answer's status of one recipe job on a `recipe` thread (`FETCH_STACK_BYTES`), the very
/// thread the radio image runs it on: it logs `recipe: thread stack <n> of 20480 bytes never
/// used`, which `scripts/qemu.sh` reads.
fn job(
    home: &Mutex<Home>,
    op: Op,
    files: &MemoryFiles,
    fetcher: InMemory,
    runtimes: &Arc<dyn solite_core::recipes::script::ScriptRuntimes>,
) -> Result<u16, String> {
    let ticket = lock(home)
        .recipe_ticket(op, INSTALL_WALL_MS)
        .map_err(|f| format!("script phase: install: {} {}", f.status, f.message))?;
    let runtimes = Arc::clone(runtimes);
    let ran = job_thread::run(
        c"recipe",
        FETCH_STACK_BYTES,
        StackPlace::Internal,
        move || {
            let mut fetcher = fetcher;
            let mut pacing = StepPacing(Instant::now());
            let mut env = JobEnv {
                fetcher: &mut fetcher,
                pacing: &mut pacing,
                storage: files,
                runtimes,
                clock: scripts::clock(),
                script_thread: &ScriptJobThread,
                check_limits: Limits::CHECK,
                download_limits: Limits::DOWNLOAD,
            };
            let response = ticket.run(home, &mut env);
            if response.status >= 300 {
                log::error!(
                    "script phase: install: {} {}",
                    response.status,
                    response.body.as_deref().unwrap_or("")
                );
            }
            response.status
        },
    )?;
    ran.ok_or_else(|| "script phase: the recipe thread panicked".to_string())
}

/// Installs `motion-light` and `schedule-on-off`, signed now with a key made from the ESP32's
/// random numbers (no key in any file), from an in-memory catalogue that lists both, on a `recipe`
/// thread; loads each again as the start does, into a home of no use; uninstalls them. Logs
/// `script phase: install <id>: internal RAM dip <n> KB` for each (the lowest free internal RAM
/// below what was free before its install) and `script phase: install 200, boot load ok,
/// uninstall 200`. The home is left as it was: the script phase's own measures follow.
#[allow(unsafe_code)]
fn install_step(home: &Mutex<Home>) -> Result<(), String> {
    let mut seed = [0u8; 32];
    // SAFETY: `seed` is valid for writes of its length for the duration of the call.
    unsafe { esp_idf_svc::sys::esp_fill_random(seed.as_mut_ptr().cast(), seed.len()) };
    let key = SigningKey::from_bytes(&seed);
    let public_hex = hex(&key.verifying_key().to_bytes());
    let mut files_by_url = Vec::new();
    let mut entries = Vec::new();
    let mut versions = Vec::new();
    for (id, text) in [
        (MOTION_SCRIPT, MOTION_SCRIPT_SOURCE),
        (SCHEDULE_SCRIPT, SCHEDULE_SCRIPT_SOURCE),
    ] {
        let version = script_version(text)?;
        let source = text.as_bytes();
        let head = signed_bytes(
            version,
            u32::try_from(source.len()).map_err(|_| "script phase: script too large")?,
            &digest(source),
        );
        let mut file = head.to_vec();
        file.extend_from_slice(&key.sign(&head).to_bytes());
        file.extend_from_slice(source);
        debug_assert_eq!(file.len(), HEADER_BYTES + source.len());
        entries.push(format!(
            r#"{{"id":"{id}","version":"{version}","name":"Recipe","description":"The script recipe","size":{},"sha256":"{}","url":"{}"}}"#,
            file.len(),
            hex(&digest(&file)),
            recipe_url(id)
        ));
        files_by_url.push((recipe_url(id), file));
        versions.push((id, version));
    }
    let catalogue = format!(r#"{{"format":1,"recipes":[{}]}}"#, entries.join(",")).into_bytes();
    let (installs, line) =
        RecipeInstalls::from_key_text(Some(&public_hex), Arc::new(crate::memory::EspProbe));
    log::info!("script phase: {line}");
    let runtimes = scripts::runtimes();
    let files = MemoryFiles::default();
    // The home that runs is given the throwaway key for the step, and gets the real one back.
    let kept = lock(home).recipe_installs().clone();
    lock(home).set_recipe_installs(installs.clone());
    let fetcher = || InMemory {
        catalogue: catalogue.clone(),
        files: files_by_url.clone(),
    };
    let (mut installed, mut booted, mut removed) = (200, true, 200);
    for (id, version) in versions {
        // The install's own window of the lowest free internal RAM.
        measure::stop_local_minimum();
        measure::start_local_minimum();
        let free_before = memory::read().internal_free;
        let done = job(
            home,
            Op::Install {
                repo: format!("solite-recipes/{id}"),
            },
            &files,
            fetcher(),
            &runtimes,
        );
        let (lowest, _) = measure::local_minimums();
        log::info!(
            "script phase: install {id}: internal RAM dip {} KB",
            free_before.saturating_sub(lowest) / 1024
        );
        // As at start: the files read, checked again and loaded, into a home of no use.
        let loaded = {
            let mut scratch = Home::new(INSTALL_WALL_MS);
            scratch.set_recipe_installs(installs.clone());
            let lines = scratch.load_installed_scripts(&files, &runtimes, &scripts::clock());
            lines
                .iter()
                .any(|l| l.starts_with(&format!("recipes: script {id} {version} installed")))
        };
        let gone = job(
            home,
            Op::Uninstall { id: id.to_string() },
            &files,
            fetcher(),
            &runtimes,
        );
        installed = match done {
            Ok(200) => installed,
            Ok(other) => other,
            Err(e) => {
                lock(home).set_recipe_installs(kept.clone());
                return Err(e);
            }
        };
        booted &= loaded;
        removed = match gone {
            Ok(200) => removed,
            Ok(other) => other,
            Err(e) => {
                lock(home).set_recipe_installs(kept.clone());
                return Err(e);
            }
        };
    }
    lock(home).set_recipe_installs(kept);
    log::info!(
        "script phase: install {installed}, boot load {}, uninstall {removed}",
        if booted { "ok" } else { "failed" }
    );
    if installed != 200 || !booted || removed != 200 {
        return Err("script phase: the install step failed".to_string());
    }
    Ok(())
}

pub fn run(
    home: &Mutex<Home>,
    clocks: &Clocks,
    start: Instant,
    engine_before: usize,
) -> Result<(), String> {
    log::info!(
        "script phase: engine stack {} of {STACK_BYTES} bytes unused before it",
        job_thread::stack_unused()
    );
    // The install step's uninstall deletes the recipe's instances (spec 042): the home's kept ones
    // are noted first and made again once the recipe is loaded.
    let kept_before = motion_light_instances(home);
    install_step(home)?;
    let runtimes = scripts::runtimes();
    let real = Arc::clone(&clocks.monotonic);
    let clock: ScriptClock = Arc::new(move || real() / QEMU_SLOWDOWN);
    // A window of its own: the monitor of the local minimum starts again.
    measure::stop_local_minimum();
    measure::start_local_minimum();
    let mut lows = measure::Lows::new();
    let before = memory::read();
    let psram_before = before.psram.map_or(0, |p| p.free);

    let file = format!("{MOTION_SCRIPT}.js");
    let began = Instant::now();
    let summary = scripts::load_one(
        &mut lock(home),
        &file,
        MOTION_SCRIPT_SOURCE,
        &runtimes,
        &clock,
    )
    .ok_or("script phase: motion-light did not load")?;
    log::info!(
        "script phase: loaded in {} ms of QEMU's time",
        began.elapsed().as_millis()
    );

    // 1. The three kept `motion-light` instances start now that the recipe is loaded (they were
    // stopped with `Recipe not found`, as on a box that has not installed it), the scenario again.
    let mut originals = motion_light_instances(home);
    if originals.is_empty() {
        for (_, params) in &kept_before {
            create(home, MOTION_SCRIPT, params)?;
        }
        originals = motion_light_instances(home);
    }
    if originals.is_empty() {
        return Err("script phase: the home has no motion-light instance".into());
    }
    lock(home).start_recipes((clocks.monotonic)(), wall_ms());
    let stopped: Vec<String> = {
        let guard = lock(home);
        guard
            .recipes()
            .instances()
            .filter(|i| i.recipe_id == MOTION_SCRIPT && !guard.recipes().is_running(&i.id))
            .map(|i| i.id.to_string())
            .collect()
    };
    if !stopped.is_empty() {
        let why = stopped
            .first()
            .map(|id| last_log(home, id))
            .unwrap_or_default();
        return Err(format!(
            "script phase: {} kept instance(s) did not start: {why}",
            stopped.len()
        ));
    }
    replay(home, clocks, start)?;
    let (held3, peak3) = stats_of(home, MOTION_SCRIPT).ok_or("script phase: no counters")?;
    // The home with `motion-light` loaded and its three instances running, the scenario replayed:
    // the figure the script phase's gate reads (free before the engine started minus the lowest
    // free of the phase so far; the depth script and the `script` thread's stack come after it).
    lows.mark();
    let heap_peak = engine_before.saturating_sub(lows.total);
    for line in scripts::lines_of(&lock(home)) {
        log::info!("{line}");
    }

    // 2. The deep recursion, on this thread and on a `script` job thread.
    let depth_params = format!(r#"{{"zone":"{ZONE}","lights":["{LIGHT}"]}}"#);
    scripts::load_one(
        &mut lock(home),
        "depth-host.js",
        DEPTH_SCRIPT,
        &runtimes,
        &clock,
    )
    .ok_or("script phase: depth-host did not load")?;
    let id = create(home, "depth-host", &depth_params)?;
    log::info!("script phase: engine thread, {}", last_log(home, &id));
    let engine_unused = job_thread::stack_unused();
    delete_all(home, "depth-host");
    let body = format!(r#"{{"recipeId":"depth-host","params":{depth_params}}}"#);
    let ran = job_thread::run(
        c"script",
        SCRIPT_THREAD_STACK_BYTES,
        StackPlace::Psram,
        || {
            let mut random = |bytes: &mut [u8]| fill(bytes);
            let request = Request {
                method: "POST",
                path: "/api/v1/recipe-instances",
                host: Some("localhost:80"),
                port: 80,
                reach: Reach::Lan { name: "solite" },
                content_type: Some("application/json"),
                authorization: None,
                content_length: Some(body.len() as u64),
                body: body.as_bytes(),
            };
            let reply = handle_stream(&mut lock(home), &request, wall_ms(), &mut random);
            let status = match reply {
                Reply::Whole(response) => response.status,
                _ => 0,
            };
            (status, job_thread::stack_unused())
        },
    )?;
    let (status, script_unused) = ran.ok_or("script phase: the script thread panicked")?;
    let id = lock(home)
        .recipes()
        .instances()
        .find(|i| i.recipe_id == "depth-host")
        .map(|i| i.id.to_string())
        .unwrap_or_default();
    log::info!(
        "script phase: script thread, POST status {status}, {}",
        last_log(home, &id)
    );
    delete_all(home, "depth-host");

    // 3. The line.
    lows.mark();
    let now = memory::read();
    let phase = ScriptPhase {
        source: summary.source_bytes,
        psram_used: psram_before.saturating_sub(now.psram.map_or(0, |p| p.free)),
        psram_peak: psram_before.saturating_sub(lows.psram),
        heap_peak,
        internal_used: before.internal_free.saturating_sub(now.internal_free),
        internal_min: lows.internal,
        engine_stack: (engine_unused, STACK_BYTES),
        script_stack: (script_unused, SCRIPT_THREAD_STACK_BYTES),
    };
    log::info!("{}", script_phase_line(MOTION_SCRIPT, &phase));

    // 4. Four instances, the most one script recipe holds: its memory at the bound.
    delete_all(home, MOTION_SCRIPT);
    for i in 0..MAX_INSTANCES_PER_SCRIPT {
        let (_, params) = &originals[i % originals.len()];
        create(home, MOTION_SCRIPT, params)?;
    }
    replay(home, clocks, start)?;
    let (held_max, peak_max) = stats_of(home, MOTION_SCRIPT).ok_or("script phase: no counters")?;
    log::info!(
        "script sizes: {MOTION_SCRIPT} loaded {} B, 3 instances {held3} B (peak {peak3} B), {MAX_INSTANCES_PER_SCRIPT} instances {held_max} B (peak {peak_max} B)",
        summary.runtime_bytes,
    );
    for line in scripts::lines_of(&lock(home)) {
        log::info!("{line}");
    }

    // 5. A script at the source bound (32 KB), with as many instances: a third script recipe
    // beside `motion-light` and the depth script, its memory read the same way.
    delete_all(home, MOTION_SCRIPT);
    let big = scripts::load_one(
        &mut lock(home),
        "big-recipe.js",
        BIG_SCRIPT,
        &runtimes,
        &clock,
    )
    .ok_or("script phase: big-recipe did not load")?;
    for i in 0..MAX_INSTANCES_PER_SCRIPT {
        let (_, params) = &originals[i % originals.len()];
        create(home, "big-recipe", params)?;
    }
    replay(home, clocks, start)?;
    let (held_big, peak_big) = stats_of(home, "big-recipe").ok_or("script phase: no counters")?;
    log::info!(
        "script sizes: big-recipe {} B of source, loaded {} B, {MAX_INSTANCES_PER_SCRIPT} instances {held_big} B (peak {peak_big} B)",
        big.source_bytes,
        big.runtime_bytes,
    );
    Ok(())
}
