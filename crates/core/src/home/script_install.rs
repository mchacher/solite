//! The steps a recipe job takes on the home (spec 042 R3): the ticket taken under the lock, the
//! install, update and uninstall of a script recipe on the engine, and the boot load of the files.
//! Each step is short and runs with the lock held; the download, the write and the removal, which
//! are not the engine's, run between them without it (`recipes::install::job`).

use std::sync::Arc;

use super::{CLOCK_SET_AFTER_MS, Home};
use crate::backup::Phase;
use crate::engine::{SCRIPT_EXTENSION, SCRIPTS_DIR, Storage};
use crate::failure::{Failure, Outcome};
use crate::recipes::install::{CLOCK_NOT_SET, MAX_REFUSED, NOT_CONFIGURED, Op, RecipeTicket, file};
use crate::recipes::script::{Replaced, ScriptRuntimes, ScriptSummary, UpdateFailure};
use crate::recipes::{MAX_SCRIPT_RECIPES, ScriptClock, ScriptSource};
use crate::supervisor::Mode;
use crate::update::release::RecipeKind;
use crate::update::{Refusal, Version};

fn refused(refusal: Refusal) -> Failure {
    Failure::new(refusal.status, refusal.message)
}

impl Home {
    /// The right to run a recipe job or a refresh (R2.4, R3.1, R3.3), taken under the lock: 503
    /// without a valid recipe key, 503 while the clock is not set (what reads the catalogue), 409
    /// with the text of what runs, 503 for the internal RAM (R4.2), then what the id alone
    /// decides: a built-in id, an installed one, too many files, no such recipe to update or
    /// remove. `wall` is the wall clock.
    pub fn recipe_ticket(&self, op: Op, wall: u64) -> Outcome<RecipeTicket> {
        let installs = self.install.installs.clone();
        if op == Op::Refresh {
            self.releases.pace_refresh(wall).map_err(refused)?;
        }
        if !matches!(op, Op::Uninstall { .. }) && installs.key().is_none() {
            return Err(Failure::new(503, NOT_CONFIGURED));
        }
        if matches!(op, Op::Refresh | Op::Install { .. } | Op::Update { .. })
            && wall < CLOCK_SET_AFTER_MS
        {
            return Err(Failure::new(503, CLOCK_NOT_SET));
        }
        let job = op != Op::Refresh;
        let kind = if job {
            RecipeKind::Job
        } else {
            RecipeKind::Refresh
        };
        let mut mark = self.releases.begin_recipe(kind)?;
        if job {
            self.update_status.begin_recipe_job().map_err(refused)?;
            mark = mark.with_status(Arc::clone(&self.update_status));
            if self.backup_state().is_pending() {
                return Err(Failure::new(409, "A restore waits for the restart"));
            }
            if self.backup_state().phase() != Phase::Idle {
                return Err(Failure::new(409, "Backup already in progress"));
            }
        }
        if let Some(failure) = self.internal_refusal(!matches!(op, Op::Uninstall { .. })) {
            return Err(failure);
        }
        if op == Op::Refresh {
            self.releases.note_refresh(wall);
        }
        match &op {
            Op::Install { id } => self.check_install_id(id)?,
            Op::Update { id } => self.check_update_id(id)?,
            Op::Uninstall { id } => self.check_uninstall_id(id)?,
            Op::Refresh | Op::Upload { .. } => {}
        }
        Ok(RecipeTicket {
            mark,
            op,
            installs,
            url: self.catalogue_url(),
            wall,
            firmware: Version::parse(crate::VERSION).unwrap_or(Version::new(0, 0, 0)),
        })
    }

    /// The files of the installed recipes on the flash, loaded or refused, not counting `but`.
    fn files_but(&self, but: &str) -> usize {
        self.recipes
            .installed_scripts()
            .iter()
            .filter(|(id, _)| id != but)
            .count()
            + self
                .install
                .refused
                .iter()
                .filter(|refused| refused.id != but)
                .count()
    }

    /// An id to install: not a built-in recipe's (409 `<id> is built in`), not installed (409
    /// `<id> is installed: update it`), and room for one more file (409 `Too many script recipes
    /// (at most 4)`).
    pub(crate) fn check_install_id(&self, id: &str) -> Outcome<()> {
        if self.recipes.is_built_in(id) {
            return Err(Failure::new(409, format!("{id} is built in")));
        }
        if self.recipes.installed_version(id).is_some() {
            return Err(Failure::new(409, format!("{id} is installed: update it")));
        }
        if self.files_but(id) >= MAX_SCRIPT_RECIPES
            || self.recipes.script_count() >= MAX_SCRIPT_RECIPES
        {
            return Err(Failure::new(
                409,
                format!("Too many script recipes (at most {MAX_SCRIPT_RECIPES})"),
            ));
        }
        Ok(())
    }

    /// An id to update: built in (409), or not installed (404).
    pub(crate) fn check_update_id(&self, id: &str) -> Outcome<()> {
        if self.recipes.is_built_in(id) {
            return Err(Failure::new(409, format!("{id} is built in")));
        }
        if self.recipes.installed_version(id).is_none() {
            return Err(Failure::new(404, format!("Recipe not found: {id}")));
        }
        Ok(())
    }

    /// An id to remove: built in (409), or no file of that id (404).
    pub(crate) fn check_uninstall_id(&self, id: &str) -> Outcome<()> {
        if self.recipes.is_built_in(id) {
            return Err(Failure::new(409, format!("{id} is built in")));
        }
        if self.recipes.installed_version(id).is_none()
            && !self.install.refused.iter().any(|refused| refused.id == id)
        {
            return Err(Failure::new(404, format!("Recipe not found: {id}")));
        }
        Ok(())
    }

    /// The installed version of a recipe, if any.
    pub(crate) fn installed_version(&self, id: &str) -> Option<Version> {
        self.recipes.installed_version(id)
    }

    // ---- The steps ----

    /// Loads a new installed recipe (R3.4): the memory rule (409), then the script's own checks
    /// (400), and the version the signed header gives (400). Its instances are not started: the
    /// file is written first.
    pub(crate) fn install_load(
        &mut self,
        id: &str,
        source: &str,
        version: Version,
        runtimes: &Arc<dyn ScriptRuntimes>,
        clock: &ScriptClock,
    ) -> Outcome<ScriptSummary> {
        if let Some(reason) = self.memory_refusal(id, runtimes.memory_limit()) {
            return Err(Failure::new(409, reason));
        }
        let loaded = self.recipes.load_script_as(
            source,
            Arc::clone(runtimes),
            Arc::clone(clock),
            ScriptSource::Installed { version },
        );
        self.interest = self.recipes.interest();
        loaded.map_err(|reason| Failure::new(400, reason))
    }

    /// The file is written: the instances kept for the recipe start if they are enabled, a
    /// refused file of that id is no more. The recipe's definition.
    pub(crate) fn install_commit(&mut self, id: &str, wall: u64) -> Option<String> {
        self.install.refused.retain(|refused| refused.id != id);
        self.keep_file(id);
        let mono = self.mono(wall);
        self.with_engine(|engine, home| {
            engine.start_recipe_instances(home, id, "Failed to start", wall, mono);
        });
        self.recipes.recipe_info_text(id)
    }

    /// A file just written is not one the saver removes later (a factory reset queued it).
    fn keep_file(&mut self, id: &str) {
        let name = crate::engine::script_file(id);
        self.install.removals.retain(|queued| *queued != name);
    }

    /// The file could not be written: the recipe is unloaded again.
    pub(crate) fn install_abort(&mut self, id: &str, wall: u64) {
        self.with_engine(|engine, _| engine.remove_script(id, wall, false));
    }

    /// Swaps the runtime of an installed recipe for a new file's (R3.5): the instances stop, the
    /// old runtime goes, the memory rule (409), the new file loads (400). When it does not, the old
    /// file is loaded again and its instances start; when the old one does not load either the
    /// recipe is failed and the answer is 500 with both reasons.
    pub(crate) fn update_swap(
        &mut self,
        id: &str,
        source: &str,
        version: Version,
        runtimes: &Arc<dyn ScriptRuntimes>,
        clock: &ScriptClock,
        wall: u64,
    ) -> Outcome<Replaced> {
        let mono = self.mono(wall);
        let limit = runtimes.memory_limit();
        let result = self.with_engine(|engine, home| {
            let mut admit = || match home.memory_refusal(id, limit) {
                Some(reason) => Err(reason),
                None => Ok(()),
            };
            engine.replace_script(id, source, version, runtimes, clock, wall, mono, &mut admit)
        });
        let restart_old = |home: &mut Home| {
            home.with_engine(|engine, h| {
                engine.start_recipe_instances(h, id, "Failed to restart", wall, mono);
            });
        };
        match result {
            Ok(replaced) => Ok(replaced),
            Err(UpdateFailure::NotInstalled) => {
                Err(Failure::new(404, format!("Recipe not found: {id}")))
            }
            Err(UpdateFailure::Refused { reason, old }) => match old {
                Ok(()) => {
                    restart_old(self);
                    Err(Failure::new(409, reason))
                }
                Err(old_reason) => Err(Failure::new(
                    500,
                    format!("{reason}; the old version does not load again: {old_reason}"),
                )),
            },
            Err(UpdateFailure::DoesNotLoad { reason, old }) => match old {
                Ok(()) => {
                    restart_old(self);
                    Err(Failure::new(400, reason))
                }
                Err(old_reason) => Err(Failure::new(
                    500,
                    format!("{reason}; the old version does not load again: {old_reason}"),
                )),
            },
        }
    }

    /// The new file is written: the instances start again with their settings.
    pub(crate) fn update_commit(&mut self, id: &str, wall: u64) -> Option<String> {
        self.keep_file(id);
        let mono = self.mono(wall);
        self.with_engine(|engine, home| {
            engine.start_recipe_instances(home, id, "Failed to restart", wall, mono);
        });
        self.recipes.recipe_info_text(id)
    }

    /// The new file could not be written: the new runtime is dropped and the old file loaded again,
    /// its instances started. `Err` with the reason when the old file does not load again either.
    pub(crate) fn update_abort(
        &mut self,
        id: &str,
        replaced: &Replaced,
        runtimes: &Arc<dyn ScriptRuntimes>,
        clock: &ScriptClock,
        wall: u64,
    ) -> Result<(), String> {
        let mono = self.mono(wall);
        self.with_engine(|engine, home| {
            let restored = engine.restore_old(id, replaced, runtimes, clock, wall, mono);
            if restored.is_ok() {
                engine.start_recipe_instances(home, id, "Failed to restart", wall, mono);
            }
            restored
        })
    }

    /// An uninstall (R3.6): the recipe's instances are deleted (as `DELETE /recipe-instances/:id`
    /// does, `recipes.json` to write), the runtime is dropped, a refused file of that id is
    /// forgotten. The file is the job's to remove.
    pub(crate) fn uninstall_step(&mut self, id: &str, wall: u64) {
        let deleted = self.with_engine(|engine, _| engine.delete_instances_of(id));
        if deleted > 0 {
            self.recipes_changed = true;
        }
        self.with_engine(|engine, _| engine.remove_script(id, wall, false));
        self.install.refused.retain(|refused| refused.id != id);
    }

    /// The factory reset's part (R3.9): the installed runtimes are dropped (their instances go
    /// with the rest), and the saver is given every file of the recipes to remove.
    pub(crate) fn drop_installed_scripts(&mut self, wall: u64) {
        let ids: Vec<String> = self
            .recipes
            .installed_scripts()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        for id in &ids {
            self.with_engine(|engine, _| engine.remove_script(id, wall, false));
        }
        let refused: Vec<String> = self
            .install
            .refused
            .drain(..)
            .map(|refused| refused.id)
            .collect();
        for id in ids.into_iter().chain(refused) {
            let name = crate::engine::script_file(&id);
            if !self.install.removals.contains(&name) {
                self.install.removals.push(name);
            }
        }
    }

    // ---- At start (R3.7) ----

    /// Loads the files of `scripts/`, in the order of their ids, after any `--recipe-script` and
    /// before the instances start. A file refused is left in place, logged and shown. The lines to
    /// print, one per file. Reads the storage figures last.
    pub fn load_installed_scripts(
        &mut self,
        storage: &dyn Storage,
        runtimes: &Arc<dyn ScriptRuntimes>,
        clock: &ScriptClock,
    ) -> Vec<String> {
        self.install.refused.clear();
        let mut lines = Vec::new();
        let names = match storage.list(SCRIPTS_DIR) {
            Ok(names) => names,
            Err(reason) => {
                log::warn!("recipes: cannot list {SCRIPTS_DIR}: {reason}");
                return lines;
            }
        };
        let mut ids: Vec<String> = names
            .iter()
            .filter_map(|name| name.strip_suffix(SCRIPT_EXTENSION).map(str::to_string))
            .collect();
        ids.sort();
        let safe = self.supervision.mode != Mode::Normal;
        let key = self.install.installs.key().cloned();
        for (index, id) in ids.iter().enumerate() {
            let outcome = (|| -> Result<ScriptSummary, String> {
                if id.is_empty()
                    || id.len() > crate::recipes::script::MAX_INSTALLED_ID_CHARS
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                {
                    return Err("not a recipe file name".to_string());
                }
                if safe {
                    return Err("Safe mode: not loaded".to_string());
                }
                if index >= MAX_SCRIPT_RECIPES || self.recipes.script_count() >= MAX_SCRIPT_RECIPES
                {
                    return Err(format!(
                        "Too many script recipes (at most {MAX_SCRIPT_RECIPES})"
                    ));
                }
                let Some(key) = &key else {
                    return Err(NOT_CONFIGURED.to_string());
                };
                let name = crate::engine::script_file(id);
                let Some(bytes) = storage.read_bytes(&name)? else {
                    return Err("the file is gone".to_string());
                };
                let signed = file::check(&bytes, key)?;
                if self.recipes.is_built_in(id) {
                    return Err("built in".to_string());
                }
                if let Some(reason) = self.memory_refusal(id, runtimes.memory_limit()) {
                    return Err(reason);
                }
                let summary = self
                    .recipes
                    .load_script_as(
                        &signed.source,
                        Arc::clone(runtimes),
                        Arc::clone(clock),
                        ScriptSource::Installed {
                            version: signed.version,
                        },
                    )
                    .map_err(|reason| {
                        reason
                            .strip_prefix("Script recipe invalid: ")
                            .unwrap_or(&reason)
                            .to_string()
                    })?;
                if summary.id != *id {
                    self.recipes.remove_script(&summary.id, 0, false);
                    return Err(format!(
                        "the file is named {id} but the recipe is {}",
                        summary.id
                    ));
                }
                Ok(summary)
            })();
            match outcome {
                Ok(summary) => lines.push(format!(
                    "recipes: script {} {} installed ({:.1} KB of source, {:.1} KB in its runtime)",
                    summary.id,
                    summary.version.as_deref().unwrap_or("-"),
                    summary.source_bytes as f64 / 1024.0,
                    summary.runtime_bytes as f64 / 1024.0,
                )),
                Err(reason) => {
                    self.refuse_file(id, &reason);
                    lines.push(format!("recipes: script {id}: {reason}"));
                }
            }
        }
        if ids.len() > MAX_REFUSED + MAX_SCRIPT_RECIPES {
            log::warn!(
                "recipes: {} files in {SCRIPTS_DIR}, the first ones are shown",
                ids.len()
            );
        }
        self.interest = self.recipes.interest();
        if let Ok((used, total)) = storage.usage() {
            self.set_storage_figures(used, total);
        }
        lines
    }
}
