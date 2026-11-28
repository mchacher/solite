//! The home's side of the recipe installs (spec 042): the catalogue and the files it knows of, the
//! tickets of the recipe jobs taken under the lock (R3.1), the steps a job takes on the engine, the
//! boot load of the installed files (R3.7), `GET /system/resources` and the figures it reads.

use super::Home;
use crate::devices::format_iso;
use crate::failure::Failure;
use crate::memory::{
    CatalogueResources, InternalResources, PsramResources, Resources, ScriptResources,
    StorageResources,
};
use crate::recipes::install::catalogue::{DEFAULT_CATALOGUE_URL, Entry};
use crate::recipes::install::{
    CatalogueState, FETCH_STACK_BYTES, MAX_REFUSED, RECIPE_INTERNAL_MARGIN, RecipeInstalls,
    Refused, SCRIPT_PSRAM_RESERVE,
};
use crate::settings;

/// The manifest of a package, Sowel's `PluginManifest` (`src/shared/types.ts`), as a recipe has it.
/// Written field by field into the answer's body (no value tree).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest<'a> {
    id: String,
    name: String,
    version: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    repo: String,
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<&'a str>,
}

/// One entry of `GET /plugins`, Sowel's `PluginInfo`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfoView<'a> {
    manifest: Manifest<'a>,
    enabled: bool,
    /// ISO 8601; `null` when the box does not know (no clock at install, or the box restarted).
    installed_at: Option<String>,
    status: &'static str,
    device_count: u32,
    offline_device_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_version: Option<&'a str>,
    source: &'static str,
}

/// One entry of `GET /plugins/store`, Sowel's `StoreEntry` (`PluginManifest` and the store's
/// fields).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreEntryView<'a> {
    id: &'a str,
    name: &'a str,
    version: &'a str,
    description: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<&'a str>,
    repo: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<&'a str>,
    compatible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    compat_reason: Option<String>,
    is_official: bool,
    tier: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    i18n: Option<&'a serde_json::value::RawValue>,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    tags: &'a [String],
}

/// What a script recipe's definition says of itself (the fields of a `PluginManifest` it can
/// fill).
#[cfg(feature = "script-recipes")]
#[derive(serde::Deserialize)]
struct Definition {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    icon: Option<String>,
}

/// The unit of the figures in the messages: KiB, the needed rounded up and the free rounded down.
#[cfg_attr(not(feature = "script-recipes"), allow(dead_code))]
fn kib_down(bytes: usize) -> usize {
    bytes / 1024
}

#[cfg_attr(not(feature = "script-recipes"), allow(dead_code))]
fn kib_up(bytes: usize) -> usize {
    bytes.div_ceil(1024)
}

#[cfg_attr(not(feature = "script-recipes"), allow(dead_code))]
impl Home {
    /// The recipe key and the memory probe of the program (spec 042 R1.5, R4.4): until set, no key
    /// (installs refused) and no memory read.
    pub fn set_recipe_installs(&mut self, installs: RecipeInstalls) {
        self.install.installs = installs;
    }

    pub fn recipe_installs(&self) -> &RecipeInstalls {
        &self.install.installs
    }

    /// The catalogue's address: the setting, or the default when unset or empty (R2.2).
    pub(crate) fn catalogue_url(&self) -> String {
        self.settings
            .get(settings::RECIPES_CATALOGUE)
            .filter(|url| !url.is_empty())
            .unwrap_or(DEFAULT_CATALOGUE_URL)
            .to_string()
    }

    /// The storage figures, read by the thread that wrote or removed a file (R5.2) and kept.
    pub fn set_storage_figures(&mut self, used: u64, total: Option<u64>) {
        self.install.storage = StorageResources { used, total };
    }

    /// The files the factory reset asks the saver to remove (R3.9), taken.
    pub fn take_removals(&mut self) -> Vec<String> {
        std::mem::take(&mut self.install.removals)
    }

    /// Puts back the files a removal failed for, to be tried again.
    pub fn restore_removals(&mut self, names: Vec<String>) {
        for name in names {
            if !self.install.removals.contains(&name) {
                self.install.removals.push(name);
            }
        }
    }

    /// True while files wait for the saver to remove them.
    pub fn has_removals(&self) -> bool {
        !self.install.removals.is_empty()
    }

    /// Keeps a catalogue read: the list, when, and no error.
    pub(crate) fn keep_catalogue(&mut self, entries: Vec<Entry>, wall: u64) {
        self.install.catalogue = CatalogueState::new(entries, wall);
    }

    /// A read that failed: the last list is kept, with the reason.
    pub(crate) fn catalogue_failed(&mut self, reason: String) {
        self.install.catalogue.error = Some(reason);
    }

    /// The catalogue's entry for an id, if one is held.
    pub(crate) fn catalogue_entry(&self, id: &str) -> Option<Entry> {
        self.install
            .catalogue
            .entries()
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
    }

    /// How many entries the held catalogue lists.
    pub(crate) fn catalogue_count(&self) -> usize {
        self.install.catalogue.entries().len()
    }

    /// True when a catalogue read is held (even an empty list).
    pub(crate) fn catalogue_held(&self) -> bool {
        self.install.catalogue.read_at.is_some()
    }

    pub(crate) fn refuse_file(&mut self, id: &str, reason: &str) {
        log::warn!("recipes: script {id}: {reason}");
        self.install.refused.retain(|refused| refused.id != id);
        if self.install.refused.len() < MAX_REFUSED {
            self.install.refused.push(Refused {
                id: id.to_string(),
                reason: reason.to_string(),
            });
        }
    }

    /// The catalogue's entry for a `repo`, if one is held.
    pub(crate) fn catalogue_entry_of_repo(&self, repo: &str) -> Option<Entry> {
        self.install
            .catalogue
            .entries()
            .iter()
            .find(|entry| entry.repo == repo)
            .cloned()
    }

    /// `GET /plugins` (R1): the script recipes the box runs, built-in scripts included, as
    /// Sowel's `PluginInfo`. What the box holds, never the network. Serialised straight into the
    /// answer's body, no value tree.
    #[cfg(feature = "script-recipes")]
    pub fn installed_plugins(&self) -> Vec<PluginInfoView<'_>> {
        let entries = self.install.catalogue.entries();
        let mut out = Vec::new();
        for row in self.recipes.script_rows() {
            if row.failed.is_some() {
                continue;
            }
            let Some(text) = self.recipes.recipe_info_text(&row.id) else {
                continue;
            };
            let Ok(definition) = serde_json::from_str::<Definition>(&text) else {
                continue;
            };
            let entry = entries.iter().find(|entry| entry.id == row.id);
            let installed = self.recipes.installed_version(&row.id);
            let latest = entry.filter(|entry| match (installed, entry.version()) {
                (Some(old), Some(new)) => new > old,
                _ => false,
            });
            out.push(PluginInfoView {
                installed_at: self
                    .install
                    .installed_at
                    .get(&row.id)
                    .copied()
                    .map(format_iso),
                manifest: Manifest {
                    version: installed
                        .map(|v| v.to_string())
                        .or(row.version)
                        .unwrap_or_default(),
                    name: definition.name,
                    description: definition.description,
                    icon: entry.and_then(|e| e.icon.clone()).or(definition.icon),
                    repo: entry
                        .map_or_else(|| format!("solite-recipes/{}", row.id), |e| e.repo.clone()),
                    kind: "recipe",
                    category: Some(entry.and_then(|e| e.category.as_deref()).unwrap_or("other")),
                    author: entry.and_then(|e| e.author.as_deref()),
                    id: row.id,
                },
                enabled: true,
                status: "connected",
                device_count: 0,
                offline_device_count: 0,
                latest_version: latest.map(|e| e.version.as_str()),
                source: "registry",
            });
        }
        out
    }

    #[cfg(not(feature = "script-recipes"))]
    pub fn installed_plugins(&self) -> Vec<PluginInfoView<'_>> {
        Vec::new()
    }

    /// `GET /plugins/store` (R2): the catalogue's entries that are not installed, as Sowel's
    /// `getStore`. Serialised straight into the answer's body.
    pub fn store_entries(&self) -> Vec<StoreEntryView<'_>> {
        let firmware = crate::update::Version::parse(crate::VERSION);
        self.install
            .catalogue
            .entries()
            .iter()
            .filter(|entry| {
                self.installed_text(&entry.id).is_none() && !self.is_built_in_recipe(&entry.id)
            })
            .map(|entry| {
                let incompatible = match (entry.min_version(), firmware) {
                    (Some(min), Some(now)) => min > now,
                    _ => false,
                };
                StoreEntryView {
                    id: &entry.id,
                    name: &entry.name,
                    version: &entry.version,
                    description: &entry.description,
                    icon: entry.icon.as_deref(),
                    repo: &entry.repo,
                    kind: "recipe",
                    category: entry.category.as_deref(),
                    author: entry.author.as_deref(),
                    compatible: !incompatible,
                    compat_reason: incompatible.then(|| {
                        format!(
                            "Requires Solite >= {}",
                            entry.min_version.as_deref().unwrap_or("")
                        )
                    }),
                    is_official: true,
                    tier: "official",
                    i18n: entry.i18n.as_deref(),
                    tags: &entry.tags,
                }
            })
            .collect()
    }

    /// The catalogue's figures for `GET /system/resources`: when it was read, and why the last
    /// read failed.
    pub fn catalogue_report(&self) -> (Option<String>, Option<&str>) {
        let state = &self.install.catalogue;
        (state.read_at.map(format_iso), state.error.as_deref())
    }

    /// The time a recipe was installed, kept while the box runs (R1): only when the clock is set.
    pub(crate) fn note_installed(&mut self, id: &str, wall: u64) {
        if wall >= crate::home::CLOCK_SET_AFTER_MS {
            self.install
                .installed_at
                .entry(id.to_string())
                .or_insert(wall);
        }
    }

    pub(crate) fn forget_installed(&mut self, id: &str) {
        self.install.installed_at.remove(id);
    }

    #[cfg(feature = "script-recipes")]
    fn installed_text(&self, id: &str) -> Option<String> {
        self.recipes.installed_version(id).map(|v| v.to_string())
    }

    #[cfg(not(feature = "script-recipes"))]
    fn installed_text(&self, _id: &str) -> Option<String> {
        None
    }

    #[cfg(feature = "script-recipes")]
    fn is_built_in_recipe(&self, id: &str) -> bool {
        self.recipes.is_built_in(id)
    }

    #[cfg(not(feature = "script-recipes"))]
    fn is_built_in_recipe(&self, _id: &str) -> bool {
        false
    }

    // ---- The memory rules (R4) ----

    /// R4.1: the PSRAM free now, less a runtime's limit, must leave the reserve. `None` when the
    /// platform reads no memory.
    pub(crate) fn memory_refusal(&self, id: &str, limit: usize) -> Option<String> {
        let psram = self.install.installs.probe().read()?.stats.psram?;
        let needed = limit + SCRIPT_PSRAM_RESERVE;
        (psram.free < needed).then(|| {
            format!(
                "Not enough memory for {id}: {} KB free, {} KB needed",
                kib_down(psram.free),
                kib_up(needed)
            )
        })
    }

    /// R4.2: the largest block of internal RAM must hold a job's thread and its TLS session; an
    /// uninstall (`tls` false) opens no session and needs only its thread's stack. When the
    /// program keeps the thread from the start (spec 045 R3), its stack is not counted: an
    /// install needs the session's margin, an uninstall nothing.
    pub(crate) fn internal_refusal(&self, tls: bool) -> Option<Failure> {
        let reading = self.install.installs.probe().read()?;
        let stack = if self.install.installs.job_thread_kept() {
            0
        } else {
            FETCH_STACK_BYTES
        };
        let needed = if tls {
            stack + RECIPE_INTERNAL_MARGIN
        } else {
            stack
        };
        (reading.stats.internal_largest < needed).then(|| {
            Failure::new(
                503,
                format!(
                    "Not enough internal memory: largest free block {} KB, {} KB needed",
                    kib_down(reading.stats.internal_largest),
                    kib_up(needed)
                ),
            )
        })
    }

    // ---- GET /system/resources (R5) ----

    pub fn resources(&self) -> Resources {
        let reading = self.install.installs.probe().read();
        let internal = reading.map(|r| InternalResources {
            free: r.stats.internal_free,
            min_free: r.internal_min,
            largest_block: r.stats.internal_largest,
        });
        let psram = reading.and_then(|r| {
            r.stats.psram.map(|p| PsramResources {
                free: p.free,
                min_free: r.psram_min,
                total: p.total,
                reserve: SCRIPT_PSRAM_RESERVE,
            })
        });
        Resources {
            internal,
            psram,
            storage: self.install.storage,
            scripts: self.script_resources(),
            catalogue: {
                let (read_at, error) = self.catalogue_report();
                CatalogueResources {
                    read_at,
                    error: error.map(str::to_string),
                }
            },
        }
    }

    #[cfg(feature = "script-recipes")]
    fn script_resources(&self) -> Vec<ScriptResources> {
        let mut rows: Vec<ScriptResources> = self
            .recipes
            .script_rows()
            .into_iter()
            .map(|row| ScriptResources {
                source: if row.installed {
                    "installed"
                } else {
                    "builtIn"
                },
                state: if row.failed.is_some() {
                    "failed"
                } else {
                    "ready"
                },
                reason: row.failed,
                id: row.id,
                version: row.version,
                held: row.held,
                limit: row.limit,
                instances: row.instances,
                calls: row.calls,
                average_us: row.average_us,
                max_us: row.max_us,
            })
            .collect();
        for refused in &self.install.refused {
            rows.push(ScriptResources {
                id: refused.id.clone(),
                version: None,
                source: "installed",
                state: "refused",
                reason: Some(refused.reason.clone()),
                held: 0,
                limit: 0,
                instances: self
                    .recipes
                    .instances()
                    .filter(|instance| instance.recipe_id == refused.id)
                    .count(),
                calls: 0,
                average_us: 0,
                max_us: 0,
            });
        }
        rows
    }

    #[cfg(not(feature = "script-recipes"))]
    fn script_resources(&self) -> Vec<ScriptResources> {
        Vec::new()
    }
}
