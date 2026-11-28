//! The recipe job and the refresh (spec 042 R2.4, R3): what a `RecipeTicket` runs once the home's
//! lock is let go. The download, the checks, the write and the removal happen here, on the thread
//! the caller gives (the box's `recipe` thread, its stack in internal RAM); the steps on the
//! engine (load, swap, unload, start) go through `ScriptThread`, which runs them where a script
//! may run (the box's `script` thread, the caller's own on the Mac), each under the home's lock.

use std::sync::{Arc, Mutex};

use super::catalogue::{self, Entry, MAX_CATALOGUE_BYTES};
use super::{Op, RecipeTicket, file};
use crate::api::{Response, failure, json, no_content};
use crate::backup::Pacing;
use crate::devices::lock;
use crate::engine::{Storage, script_file};
use crate::failure::{Failure, Outcome};
use crate::home::Home;
use crate::recipes::ScriptClock;
use crate::recipes::script::{ScriptRecipe, ScriptRuntimes, ScriptSummary};
use crate::update::Version;
use crate::update::release::{Fetcher, FileError, Limits, download_file, fetch_text};

/// Where the steps on the engine run: a script may only run on a thread whose stack holds the
/// script's (the box's `script` thread). `run` returns when `job` has run.
pub trait ScriptThread {
    fn run(&self, job: &mut dyn FnMut());
}

/// The thread that calls: the Mac, whose threads all have a large stack.
pub struct Inline;

impl ScriptThread for Inline {
    fn run(&self, job: &mut dyn FnMut()) {
        job();
    }
}

/// What a job needs from the program that runs it.
pub struct JobEnv<'a> {
    pub fetcher: &'a mut dyn Fetcher,
    pub pacing: &'a mut dyn Pacing,
    pub storage: &'a dyn Storage,
    pub runtimes: Arc<dyn ScriptRuntimes>,
    pub clock: ScriptClock,
    pub script_thread: &'a dyn ScriptThread,
    /// The catalogue's read.
    pub check_limits: Limits,
    /// A file's download.
    pub download_limits: Limits,
}

/// Loads a script under the rules of an installed one, to read its id and version, and drops it
/// (`--recipe-sign`, an upload's id). The error is `Script recipe invalid: <reason>`.
pub fn describe_script(
    source: &str,
    runtimes: &Arc<dyn ScriptRuntimes>,
    clock: &ScriptClock,
) -> Result<ScriptSummary, String> {
    ScriptRecipe::load_installed(source, runtimes, clock)
        .map(|(_, summary)| summary)
        .map_err(|reason| format!("Script recipe invalid: {reason}"))
}

/// The id an uploaded script declares, read from its source without running it: the first
/// `id: "<text>"` after a `recipe(` call. The real load decides (`install` and `update` refuse a
/// script whose id differs), so a wrong guess here is a refusal, never a wrong install.
fn declared_id(source: &str) -> Option<String> {
    fn word_start(source: &str, at: usize) -> bool {
        source[..at]
            .bytes()
            .next_back()
            .is_none_or(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'.')))
    }
    let call = source
        .match_indices("recipe")
        .find(|(at, _)| word_start(source, *at) && source[at + 6..].trim_start().starts_with("({"))?
        .0;
    let rest = &source[call + 6..];
    rest.match_indices("id").find_map(|(at, _)| {
        let quoted = rest[..at].ends_with(['"', '\'']);
        if !(quoted || word_start(rest, at)) {
            return None;
        }
        let mut tail = &rest[at + 2..];
        if quoted {
            tail = tail.strip_prefix(['"', '\''])?;
        }
        let tail = tail.trim_start().strip_prefix(':')?.trim_start();
        let quote = tail.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let text = &tail[1..];
        let id = &text[..text.find(quote)?];
        (!id.is_empty()
            && id.len() <= crate::recipes::script::MAX_INSTALLED_ID_CHARS
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
        .then(|| id.to_string())
    })
}

/// Runs `job` through the script thread and gives what it returned.
fn hop<R>(env: &JobEnv<'_>, job: impl FnOnce() -> R) -> Outcome<R> {
    let mut job = Some(job);
    let mut out = None;
    env.script_thread.run(&mut || {
        if let Some(job) = job.take() {
            out = Some(job());
        }
    });
    out.ok_or_else(|| Failure::new(500, "Internal error"))
}

/// The recipe's definition with its `version`, the answer of an install or an update. `version`
/// sorts after every other key of the definition, so it goes last, written into the text.
fn recipe_answer(info: Option<String>, version: Version) -> Box<serde_json::value::RawValue> {
    let mut text = info.unwrap_or_else(|| "null".to_string());
    if text.ends_with('}') {
        text.pop();
        text.push_str(&format!(",\"version\":\"{version}\"}}"));
    }
    serde_json::value::RawValue::from_string(text)
        .unwrap_or_else(|_| serde_json::value::RawValue::NULL.to_owned())
}

/// The figures of the storage, read by the thread that wrote and kept (R5.2).
fn note_storage(home: &Mutex<Home>, env: &JobEnv<'_>) {
    if let Ok((used, total)) = env.storage.usage() {
        lock(home).set_storage_figures(used, total);
    }
}

impl RecipeTicket {
    /// Runs the job and answers: the refresh with the store (R2.3), an install with 201 and the
    /// recipe, an update with 200 and the recipe, an uninstall with 204, or the failure's status
    /// and reason. The home's lock is not held while it runs; the one-job mark is held, and ends
    /// when the ticket is dropped at the end of this call.
    pub fn run(self, home: &Mutex<Home>, env: &mut JobEnv<'_>) -> Response {
        match self.run_job(home, env) {
            Ok(response) => response,
            Err(refusal) => failure(&refusal),
        }
    }

    fn run_job(&self, home: &Mutex<Home>, env: &mut JobEnv<'_>) -> Outcome<Response> {
        match &self.op {
            Op::Refresh => self.refresh(home, env),
            Op::Install { id } => {
                let (_, bytes, signed) = self.entry_file(home, env, id)?;
                self.install(home, env, id, signed, &bytes)
            }
            Op::Update { id } => {
                let installed = lock(home)
                    .installed_version(id)
                    .ok_or_else(|| Failure::new(404, format!("Recipe not found: {id}")))?;
                let entry = self.entry(home, env, id)?;
                let version = entry
                    .version()
                    .ok_or_else(|| Failure::new(400, "Invalid catalogue"))?;
                if version <= installed {
                    return Err(Failure::new(
                        409,
                        format!("{id} {version} is not newer than {installed}"),
                    ));
                }
                let (_, bytes, signed) = self.entry_file(home, env, id)?;
                self.update(home, env, id, signed, &bytes)
            }
            Op::Upload { bytes } => self.upload(home, env, bytes),
            Op::Uninstall { id } => self.uninstall(home, env, id),
        }
    }

    /// Logs the internal RAM at a step of a job (spec 056 R1): free now / lowest since boot.
    /// Nothing on a platform that reads no memory.
    fn step(&self, what: &str) {
        if let Some(reading) = self.installs.probe().read() {
            log::info!(
                "recipe install: {what} internal {}/{}",
                reading.stats.internal_free,
                reading.internal_min
            );
        }
    }

    // ---- The catalogue ----

    /// Reads the catalogue and keeps it, or keeps the reason it failed.
    fn read_catalogue(&self, home: &Mutex<Home>, env: &mut JobEnv<'_>) -> Result<(), String> {
        let read = fetch_text(
            &mut *env.fetcher,
            &self.url,
            env.check_limits,
            &mut *env.pacing,
            MAX_CATALOGUE_BYTES,
            "Catalogue too large",
        )
        .and_then(|body| {
            self.step("catalogue downloaded");
            catalogue::parse(&body)
        });
        self.step("catalogue read");
        let mut home = lock(home);
        match read {
            Ok(entries) => {
                log::info!(
                    "recipes: catalogue read, {} recipes ({})",
                    entries.len(),
                    self.url
                );
                home.keep_catalogue(entries, self.wall);
                Ok(())
            }
            Err(reason) => {
                log::warn!("recipes: catalogue read failed: {reason}");
                home.catalogue_failed(reason.clone());
                Err(reason)
            }
        }
    }

    /// `POST /recipes/store/refresh` (R2.4; the 429 is the ticket's): the catalogue is
    /// read and the store answered; a read that fails keeps the list and answers it with `error`.
    fn refresh(&self, home: &Mutex<Home>, env: &mut JobEnv<'_>) -> Outcome<Response> {
        let _ = self.read_catalogue(home, env);
        Ok(json(200, &lock(home).recipe_store()))
    }

    /// The catalogue's entry of an id, the catalogue read first when none is held (502 when it
    /// cannot be), 404 when it does not list the id, 409 when it needs a newer firmware.
    fn entry(&self, home: &Mutex<Home>, env: &mut JobEnv<'_>, id: &str) -> Outcome<Entry> {
        let held = lock(home).catalogue_held();
        if !held && let Err(reason) = self.read_catalogue(home, env) {
            return Err(Failure::new(
                502,
                format!("Recipe catalogue unreachable: {reason}"),
            ));
        }
        let entry = lock(home)
            .catalogue_entry(id)
            .ok_or_else(|| Failure::new(404, format!("Recipe not in the catalogue: {id}")))?;
        if let Some(min) = entry.min_version()
            && min > self.firmware
        {
            return Err(Failure::new(
                409,
                format!("{id} {} needs Solite {min} or later", entry.version),
            ));
        }
        Ok(entry)
    }

    /// The entry's file, downloaded and checked: size, SHA-256 and the header's version against
    /// the entry (400, a stale or forged catalogue cannot offer an older signed file under a newer
    /// number), then R1's checks (400).
    fn entry_file(
        &self,
        home: &Mutex<Home>,
        env: &mut JobEnv<'_>,
        id: &str,
    ) -> Outcome<(Entry, Vec<u8>, file::Signed)> {
        let entry = self.entry(home, env, id)?;
        let digest = entry
            .digest()
            .ok_or_else(|| Failure::new(400, "Invalid catalogue"))?;
        let downloaded = download_file(
            &mut *env.fetcher,
            &entry.url,
            entry.size,
            digest,
            env.download_limits,
            &mut *env.pacing,
        );
        self.step("download");
        let bytes = downloaded.map_err(|e| match e {
            FileError::Failed(reason) => Failure::new(502, format!("Download failed: {reason}")),
            FileError::Differs(reason) => Failure::new(400, reason),
        })?;
        let key = self
            .installs
            .key()
            .ok_or_else(|| Failure::new(503, super::NOT_CONFIGURED))?;
        let checked = file::check(&bytes, key);
        self.step("check");
        let signed = checked.map_err(|reason| Failure::new(400, reason))?;
        if Some(signed.version) != entry.version() {
            return Err(Failure::new(
                400,
                "The signed version differs from the catalogue's",
            ));
        }
        Ok((entry, bytes, signed))
    }

    // ---- Install, update, uninstall ----

    /// Loads a new recipe, writes its file, starts the instances kept for it (R3.4). The script
    /// must declare `id`.
    fn install(
        &self,
        home: &Mutex<Home>,
        env: &mut JobEnv<'_>,
        id: &str,
        signed: file::Signed,
        bytes: &[u8],
    ) -> Outcome<Response> {
        let (runtimes, clock, wall) =
            (Arc::clone(&env.runtimes), Arc::clone(&env.clock), self.wall);
        let loaded = hop(env, || {
            lock(home).install_load(id, &signed.source, signed.version, &runtimes, &clock)
        });
        self.step("load");
        let summary = loaded??;
        if summary.id != id {
            hop(env, || lock(home).install_abort(&summary.id, wall))?;
            return Err(Failure::new(
                400,
                format!("The recipe declares the id {}, not {id}", summary.id),
            ));
        }
        let name = script_file(id);
        let written = env.storage.write_bytes(&name, bytes);
        self.step("store");
        if let Err(reason) = written {
            log::error!("recipes: {id}: {reason}");
            hop(env, || lock(home).install_abort(id, wall))?;
            return Err(Failure::new(500, "Could not save the recipe"));
        }
        note_storage(home, env);
        let info = hop(env, || lock(home).install_commit(id, wall))?;
        Ok(json(201, &recipe_answer(info, signed.version)))
    }

    /// Replaces an installed recipe by a newer file (R3.5).
    fn update(
        &self,
        home: &Mutex<Home>,
        env: &mut JobEnv<'_>,
        id: &str,
        signed: file::Signed,
        bytes: &[u8],
    ) -> Outcome<Response> {
        let (runtimes, clock, wall) =
            (Arc::clone(&env.runtimes), Arc::clone(&env.clock), self.wall);
        let swapped = hop(env, || {
            lock(home).update_swap(id, &signed.source, signed.version, &runtimes, &clock, wall)
        });
        self.step("update load");
        let replaced = swapped??;
        let name = script_file(id);
        let written = env.storage.write_bytes(&name, bytes);
        self.step("update store");
        if let Err(reason) = written {
            log::error!("recipes: {id}: {reason}");
            let restored = hop(env, || {
                lock(home).update_abort(id, &replaced, &runtimes, &clock, wall)
            })?;
            return Err(match restored {
                Ok(()) => Failure::new(500, "Could not save the recipe"),
                Err(old) => Failure::new(
                    500,
                    format!(
                        "Could not save the recipe; the old version does not load again: {old}"
                    ),
                ),
            });
        }
        note_storage(home, env);
        let info = hop(env, || lock(home).update_commit(id, wall))?;
        Ok(json(200, &recipe_answer(info, signed.version)))
    }

    /// A signed file as the body (R3.2): the file is checked, its id read; a recipe not installed
    /// is installed, an installed one updated if the file is newer.
    fn upload(&self, home: &Mutex<Home>, env: &mut JobEnv<'_>, bytes: &[u8]) -> Outcome<Response> {
        let key = self
            .installs
            .key()
            .ok_or_else(|| Failure::new(503, super::NOT_CONFIGURED))?;
        let signed = file::check(bytes, key).map_err(|reason| Failure::new(400, reason))?;
        let id = declared_id(&signed.source).ok_or_else(|| {
            Failure::new(
                400,
                "Script recipe invalid: id must be 1 to 32 characters among a-z, 0-9 and -",
            )
        })?;
        let installed = lock(home).installed_version(&id);
        match installed {
            Some(old) => {
                if signed.version <= old {
                    return Err(Failure::new(
                        409,
                        format!("{id} {} is not newer than {old}", signed.version),
                    ));
                }
                self.update(home, env, &id, signed, bytes)
            }
            None => {
                lock(home).check_install_id(&id)?;
                self.install(home, env, &id, signed, bytes)
            }
        }
    }

    /// Takes a recipe out and removes its file (R3.6).
    fn uninstall(&self, home: &Mutex<Home>, env: &mut JobEnv<'_>, id: &str) -> Outcome<Response> {
        let wall = self.wall;
        hop(env, || lock(home).uninstall_step(id, wall))?;
        self.step("removal unloaded");
        let removed = env.storage.remove(&script_file(id));
        self.step("removal stored");
        match removed {
            Ok(()) => {
                note_storage(home, env);
                Ok(no_content())
            }
            Err(reason) => {
                log::error!("recipes: {id}: {reason}");
                Err(Failure::new(500, "Could not remove the recipe"))
            }
        }
    }
}
