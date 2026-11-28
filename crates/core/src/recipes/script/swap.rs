//! What the engine does with an installed script recipe (spec 042): find it, replace its runtime
//! while keeping its instances, remove it, start the instances kept for it. The runtime of a
//! recipe is dropped before a new one is made, so the memory of the old one is free for the new
//! one (R4.1 counts it as freed).

use std::sync::Arc;

use solite_recipe_sdk::Recipe;

use super::load::{self, Rules};
use super::{ScriptRuntimes, ScriptSummary, rebuild_of};
use crate::home::Home;
use crate::live::EngineEvent;
use crate::recipes::{
    Failed, Level, Native, RecipeEngine, ScriptClock, ScriptSource, announce, provider_of,
    stop_running,
};
use crate::update::Version;

/// What an update leaves to finish: the old file's text and version, kept until the new one is
/// written, so that a write that fails can put it back.
#[derive(Debug, Clone)]
pub struct Replaced {
    pub old_text: Arc<str>,
    pub old_version: Version,
    pub summary: ScriptSummary,
}

/// Why an update did not happen.
#[derive(Debug)]
pub enum UpdateFailure {
    /// No installed script has that id.
    NotInstalled,
    /// The memory rule refused the new runtime (409); the old one is loaded again (`old`).
    Refused {
        reason: String,
        old: Result<(), String>,
    },
    /// The new file does not load (400); the old one is loaded again (`old`: `Err` with its
    /// reason when it does not load either, the recipe is then failed).
    DoesNotLoad {
        reason: String,
        old: Result<(), String>,
    },
}

impl RecipeEngine {
    /// The index of the script recipe with this id.
    fn script_provider(&self, id: &str) -> Option<usize> {
        let index = provider_of(&self.natives, id).map(|crate::recipes::Provider::Native(i)| i)?;
        self.natives[index].is_script().then_some(index)
    }

    /// The version of the installed script with this id; `None` for any other recipe.
    pub fn installed_version(&self, id: &str) -> Option<Version> {
        let index = self.script_provider(id)?;
        match self.natives[index].script.as_ref()?.source {
            ScriptSource::Installed { version } => Some(version),
            ScriptSource::BuiltIn => None,
        }
    }

    /// True when a recipe the program carries (a native recipe, or a script given by
    /// `--recipe-script`) has this id.
    pub fn is_built_in(&self, id: &str) -> bool {
        match provider_of(&self.natives, id) {
            None => false,
            Some(crate::recipes::Provider::Native(index)) => {
                let native = &self.natives[index];
                native
                    .script
                    .as_ref()
                    .is_none_or(|meta| meta.source == ScriptSource::BuiltIn)
            }
        }
    }

    /// The installed scripts, in load order: id and version.
    pub fn installed_scripts(&self) -> Vec<(String, Version)> {
        self.natives
            .iter()
            .filter_map(|native| {
                let meta = native.script.as_ref()?;
                match meta.source {
                    ScriptSource::Installed { version } => {
                        Some((native.names.id.to_string(), version))
                    }
                    ScriptSource::BuiltIn => None,
                }
            })
            .collect()
    }

    /// How many script recipes are loaded, of any origin.
    pub fn script_count(&self) -> usize {
        self.natives.iter().filter(|n| n.is_script()).count()
    }

    /// The rows of `GET /system/resources`: one per script recipe, in load order.
    pub(crate) fn script_rows(&self) -> Vec<ScriptRow> {
        self.natives
            .iter()
            .filter_map(|native| {
                let meta = native.script.as_ref()?;
                let stats = native
                    .recipe
                    .as_ref()
                    .and_then(|recipe| recipe.run_stats())
                    .unwrap_or_default();
                let id = native.names.id.to_string();
                let instances = self
                    .slots
                    .iter()
                    .filter(|slot| slot.instance.recipe_id == id)
                    .count();
                Some(ScriptRow {
                    version: meta.declared.clone(),
                    installed: matches!(meta.source, ScriptSource::Installed { .. }),
                    failed: meta.failed.as_ref().map(|f| f.reason.clone()),
                    held: stats.held,
                    limit: meta.limit,
                    instances,
                    calls: stats.calls,
                    average_us: stats.total_us.checked_div(stats.calls).unwrap_or(0),
                    max_us: stats.max_us,
                    id,
                })
            })
            .collect()
    }

    /// Stops the running instances of a recipe, keeping them (enabled, written). The recipe
    /// forgets each; a message is logged on every instance of it when `message` is given.
    fn stop_instances_of(&mut self, provider: usize) {
        for slot in &mut self.slots {
            if slot.running.as_ref().map(|r| r.provider) == Some(provider)
                && stop_running(&mut self.natives, slot)
            {
                RecipeEngine::announce_stopped(&self.live, slot);
            }
        }
        self.interest_stale = true;
        self.compact_listens();
    }

    /// Takes an installed script out: its instances stop and stay (`Recipe not found: <id>` in
    /// their log when `log`), the runtime is dropped, the other recipes keep running. False when
    /// no script has that id.
    pub fn remove_script(&mut self, id: &str, wall: u64, log: bool) -> bool {
        let Some(index) = self.script_provider(id) else {
            return false;
        };
        let message = format!("Recipe not found: {id}");
        // Only the instances of this recipe run on its runtime: stop those, leave the others.
        for slot in &mut self.slots {
            if slot.running.as_ref().map(|r| r.provider) == Some(index)
                && stop_running(&mut self.natives, slot)
            {
                RecipeEngine::announce_stopped(&self.live, slot);
            }
            if log && slot.instance.recipe_id == id {
                RecipeEngine::push_log(slot, &mut self.next_log_id, wall, Level::Error, &message);
            }
        }
        self.natives.remove(index);
        // The recipes after it moved up one place.
        for slot in &mut self.slots {
            if let Some(running) = slot.running.as_mut()
                && running.provider > index
            {
                running.provider -= 1;
            }
        }
        self.interest_stale = true;
        self.compact_listens();
        true
    }

    /// Starts the enabled instances of a recipe that are not running (the instances kept while it
    /// was absent, or stopped for an update): in creation order, each with its settings; one the
    /// recipe refuses stays stopped, with the reason in its log (`<failed>: <reason>`).
    pub fn start_recipe_instances(
        &mut self,
        home: &Home,
        id: &str,
        failed: &str,
        wall: u64,
        mono: u64,
    ) {
        for index in 0..self.slots.len() {
            let slot = &self.slots[index];
            if slot.instance.recipe_id != id || !slot.instance.enabled || slot.running.is_some() {
                continue;
            }
            let _ = self.start_slot(index, home, mono, wall, failed, false);
        }
        self.interest_stale = true;
        self.compact_listens();
    }

    /// Puts the runtime of a recipe in place: its definition, its way to be made again, its text.
    fn install_runtime(
        &mut self,
        provider: usize,
        recipe: Box<dyn solite_recipe_sdk::Recipe + Send>,
        text: Arc<str>,
        version: Version,
        runtimes: &Arc<dyn ScriptRuntimes>,
        clock: &ScriptClock,
    ) {
        let info = recipe.info_text();
        let native = &mut self.natives[provider];
        native.names = Native::names_of_text(&info);
        native.recipe = Some(recipe);
        if let Some(meta) = native.script.as_mut() {
            meta.info = info;
            meta.rebuild = rebuild_of(&text, runtimes, clock, Rules::INSTALLED);
            meta.text = text;
            meta.source = ScriptSource::Installed { version };
            meta.declared = Some(version.to_string());
            meta.limit = runtimes.memory_limit();
            meta.failed = None;
            meta.restarts.clear();
            meta.to_start.clear();
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// Loads the old file again; when it does not load either, the recipe is failed as in spec
    /// 041 (its instances stopped with the reason, the restart from the old file).
    fn reload_old(
        &mut self,
        provider: usize,
        old_text: &Arc<str>,
        old_version: Version,
        runtimes: &Arc<dyn ScriptRuntimes>,
        clock: &ScriptClock,
        wall: u64,
        mono: u64,
    ) -> Result<(), String> {
        match load::load(old_text, runtimes, clock, Rules::INSTALLED) {
            Ok((recipe, _)) => {
                self.install_runtime(
                    provider,
                    Box::new(recipe),
                    Arc::clone(old_text),
                    old_version,
                    runtimes,
                    clock,
                );
                Ok(())
            }
            Err(reason) => {
                let id = self.natives[provider].names.id.to_string();
                let native = &mut self.natives[provider];
                native.recipe = None;
                if let Some(meta) = native.script.as_mut() {
                    meta.rebuild = rebuild_of(old_text, runtimes, clock, Rules::INSTALLED);
                    meta.text = Arc::clone(old_text);
                    meta.source = ScriptSource::Installed {
                        version: old_version,
                    };
                    meta.failed = Some(Failed {
                        since: mono,
                        reason: reason.clone(),
                    });
                }
                log::error!("script recipe {id} failed: {reason}");
                let message = format!("Script recipe failed: {reason}");
                for slot in &mut self.slots {
                    if slot.instance.recipe_id == id {
                        RecipeEngine::push_log(
                            slot,
                            &mut self.next_log_id,
                            wall,
                            Level::Error,
                            &message,
                        );
                        if slot.instance.enabled {
                            let (instance_id, recipe_id) =
                                (slot.instance.id, &slot.instance.recipe_id);
                            announce(&self.live, || EngineEvent::RecipeInstanceError {
                                instance_id,
                                recipe_id: recipe_id.clone(),
                                error: message.clone(),
                            });
                        }
                    }
                }
                Err(reason)
            }
        }
    }

    /// Replaces the runtime of an installed script by one made from `source` (spec 042 R3.5): the
    /// instances stop (kept, enabled), the old runtime is dropped, `admit` is asked (the memory
    /// rule, with the old runtime's memory free), the new file is loaded. The instances are not
    /// started: the caller writes the file first, then calls `start_recipe_instances`, or gives
    /// the old file back with `restore_old`.
    #[allow(clippy::too_many_arguments)]
    pub fn replace_script(
        &mut self,
        id: &str,
        source: &str,
        version: Version,
        runtimes: &Arc<dyn ScriptRuntimes>,
        clock: &ScriptClock,
        wall: u64,
        mono: u64,
        admit: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<Replaced, UpdateFailure> {
        let (Some(provider), Some(old_version)) =
            (self.script_provider(id), self.installed_version(id))
        else {
            return Err(UpdateFailure::NotInstalled);
        };
        let old_text = match self.natives[provider].script.as_ref() {
            Some(meta) => Arc::clone(&meta.text),
            None => return Err(UpdateFailure::NotInstalled),
        };
        self.stop_instances_of(provider);
        // The old runtime goes first: its memory is free for the new one.
        self.natives[provider].recipe = None;
        if let Err(reason) = admit() {
            let old = self.reload_old(
                provider,
                &old_text,
                old_version,
                runtimes,
                clock,
                wall,
                mono,
            );
            return Err(UpdateFailure::Refused { reason, old });
        }
        let loaded =
            load::load(source, runtimes, clock, Rules::INSTALLED).and_then(|(recipe, summary)| {
                let declared = Native::names_of_text(&recipe.info_text()).id;
                if &*declared != id {
                    return Err(format!("id {declared} differs from {id}"));
                }
                if summary.version.as_deref().and_then(Version::parse) != Some(version) {
                    return Err("version differs from the signed one".to_string());
                }
                Ok((recipe, summary))
            });
        match loaded {
            Ok((recipe, summary)) => {
                self.install_runtime(
                    provider,
                    Box::new(recipe),
                    Arc::from(source),
                    version,
                    runtimes,
                    clock,
                );
                Ok(Replaced {
                    old_text,
                    old_version,
                    summary,
                })
            }
            Err(reason) => {
                let old = self.reload_old(
                    provider,
                    &old_text,
                    old_version,
                    runtimes,
                    clock,
                    wall,
                    mono,
                );
                let reason = if reason == "version differs from the signed one" {
                    reason
                } else {
                    format!("Script recipe invalid: {reason}")
                };
                Err(UpdateFailure::DoesNotLoad { reason, old })
            }
        }
    }

    /// Gives the old file back after the new one could not be written: the new runtime is dropped
    /// and the old one loaded again (the instances are started by the caller).
    pub fn restore_old(
        &mut self,
        id: &str,
        replaced: &Replaced,
        runtimes: &Arc<dyn ScriptRuntimes>,
        clock: &ScriptClock,
        wall: u64,
        mono: u64,
    ) -> Result<(), String> {
        let Some(provider) = self.script_provider(id) else {
            return Ok(());
        };
        self.stop_instances_of(provider);
        self.natives[provider].recipe = None;
        self.reload_old(
            provider,
            &replaced.old_text,
            replaced.old_version,
            runtimes,
            clock,
            wall,
            mono,
        )
    }
}

/// One row of `GET /system/resources`.
#[derive(Debug, Clone)]
pub(crate) struct ScriptRow {
    pub id: String,
    pub version: Option<String>,
    pub installed: bool,
    /// The reason it failed, when it did.
    pub failed: Option<String>,
    pub held: usize,
    pub limit: usize,
    pub instances: usize,
    pub calls: u64,
    pub average_us: u64,
    pub max_us: u64,
}
