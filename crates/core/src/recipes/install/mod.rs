//! Script recipes installed, updated and removed while the program runs (spec 042): the signed
//! file (`file`), the catalogue (`catalogue`), what the home keeps of them (`InstallState`), the
//! recipe job and the refresh with their one-session rule (`RecipeTicket`, `job`), the memory
//! admission and the boot load. The core never touches the flash or the network: the files are the
//! `Storage`'s, the downloads the `Fetcher`'s, the memory the `MemoryProbe`'s.

pub mod catalogue;
pub mod file;
#[cfg(feature = "script-recipes")]
mod job;

#[cfg(feature = "script-recipes")]
pub use job::{Inline, JobEnv, ScriptThread, describe_script};

use std::sync::Arc;

use ed25519_dalek::VerifyingKey;

use crate::memory::{MemoryProbe, NoProbe, StorageResources};
use crate::update::release::{RecipeMark, url_allowed};
use crate::update::{Version, parse_public_key};
use catalogue::Entry;

/// The thread a recipe job runs on has a stack of this many bytes in internal RAM (the firmware's
/// `FETCH_STACK_BYTES`: the same 20 KB as the release check's thread).
pub const FETCH_STACK_BYTES: usize = 20 * 1024;
/// What a recipe job's TLS session takes from the internal RAM besides its thread: mbedTLS's 8 KB
/// of small allocations and the HTTP client's small structures (its 4 KB and 1 KB buffers are in
/// PSRAM since spec 056), rounded up (R4.2).
pub const RECIPE_INTERNAL_MARGIN: usize = 16 * 1024;
/// What an install leaves free in PSRAM after the runtime's limit (R4.1, decided 2026-11-27).
pub const SCRIPT_PSRAM_RESERVE: usize = 512 * 1024;
/// The text of the 503 of a build without a valid recipe key (R1.5).
pub const NOT_CONFIGURED: &str = "Recipe installs are not configured";
/// The text of the 503 of a clock not set yet (R2.4).
pub const CLOCK_NOT_SET: &str =
    "The clock is not set yet: reading the recipe catalogue needs the date";
/// The refused files the home keeps (R3.7), at most.
pub const MAX_REFUSED: usize = 8;

/// The recipe key of the build and the way to read the memory: what an install needs from the
/// program that runs it.
#[derive(Clone)]
pub struct RecipeInstalls {
    key: Option<VerifyingKey>,
    probe: Arc<dyn MemoryProbe>,
    /// The program keeps the thread a recipe job runs on from the start (the box's permanent
    /// `recipe` thread, spec 045): the admission then counts only the TLS session, not the
    /// thread's stack.
    job_thread_kept: bool,
}

impl Default for RecipeInstalls {
    fn default() -> Self {
        RecipeInstalls {
            key: None,
            probe: Arc::new(NoProbe),
            job_thread_kept: false,
        }
    }
}

impl std::fmt::Debug for RecipeInstalls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecipeInstalls")
            .field("key", &self.key.is_some())
            .finish()
    }
}

impl RecipeInstalls {
    /// With the key of the build (`SOLITE_RECIPE_PUBLIC_KEY`, `option_env!`) and the memory probe,
    /// and the line to log at start: `recipes: no recipe key, installs refused`, `recipes: invalid
    /// recipe key, installs refused` or `recipes: key <first 8 hex characters>`.
    pub fn from_key_text(text: Option<&str>, probe: Arc<dyn MemoryProbe>) -> (Self, String) {
        let (key, line) = match text {
            None => (None, "recipes: no recipe key, installs refused".to_string()),
            Some(text) => match parse_public_key(text) {
                None => (
                    None,
                    "recipes: invalid recipe key, installs refused".to_string(),
                ),
                Some(key) => (
                    Some(key),
                    format!("recipes: key {}", text[..8].to_ascii_lowercase()),
                ),
            },
        };
        (
            RecipeInstalls {
                key,
                probe,
                job_thread_kept: false,
            },
            line,
        )
    }

    /// The key that signs the recipes, `None` when the build has no valid one.
    pub fn key(&self) -> Option<&VerifyingKey> {
        self.key.as_ref()
    }

    /// Says that the thread a recipe job runs on is made once, at the start: an install, an update
    /// or a refresh then needs only `RECIPE_INTERNAL_MARGIN` of internal RAM, an uninstall none.
    pub fn set_job_thread_kept(&mut self, kept: bool) {
        self.job_thread_kept = kept;
    }

    /// Whether the job's thread is kept (see `set_job_thread_kept`).
    pub fn job_thread_kept(&self) -> bool {
        self.job_thread_kept
    }

    /// The memory probe.
    pub fn probe(&self) -> &Arc<dyn MemoryProbe> {
        &self.probe
    }
}

/// A script file that failed its checks, left in place (R3.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub id: String,
    pub reason: String,
}

/// The catalogue the home holds: the last good list (at most 32 entries, each one's `i18n` kept
/// as raw JSON text, read once per catalogue read and never parsed again), when it was read (wall clock) and the reason the last
/// read failed. The box's allocator keeps the blocks over `CONFIG_SPIRAM_MALLOC_ALWAYSINTERNAL`
/// bytes in PSRAM (spec 056).
#[derive(Debug, Default, Clone)]
pub struct CatalogueState {
    entries: Vec<Entry>,
    pub read_at: Option<u64>,
    pub error: Option<String>,
}

impl CatalogueState {
    /// A list read.
    pub fn new(entries: Vec<Entry>, read_at: u64) -> CatalogueState {
        CatalogueState {
            entries,
            read_at: Some(read_at),
            error: None,
        }
    }

    /// The entries held.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
}

/// What the home keeps for the recipe installs. Bounds: the catalogue's 32 entries and 32 KB, at
/// most `MAX_REFUSED` refused files, at most `MAX_SCRIPT_RECIPES` + `MAX_REFUSED` (10) names to
/// remove.
#[derive(Debug, Default)]
pub struct InstallState {
    pub installs: RecipeInstalls,
    pub catalogue: CatalogueState,
    pub refused: Vec<Refused>,
    /// Read at start and after each write or removal, never by a request.
    pub storage: StorageResources,
    /// The files the saver removes (the factory reset).
    pub removals: Vec<String>,
    /// When each installed recipe was installed (wall clock, ms), kept while the box runs and
    /// only when the clock was set. At most `MAX_SCRIPT_RECIPES` ids: dropped at the removal.
    pub installed_at: std::collections::HashMap<String, u64>,
}

/// What a recipe job or a refresh does (R2.4, R3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// `POST /plugins/store/refresh`.
    Refresh,
    /// `POST /plugins/install` with `{"repo"}`: the catalogue's entry of that repository.
    Install { repo: String },
    /// `POST /plugins/install` with a signed file as the body.
    Upload { bytes: Vec<u8> },
    /// `POST /plugins/:id/update`.
    Update { id: String },
    /// `POST /plugins/:id/uninstall`.
    Uninstall { id: String },
}

/// The right to run one recipe job or one refresh, taken under the home's lock (R3.1): the one-job
/// mark, the update status's flag, and what the job needs. Dropping it ends the job.
#[derive(Debug)]
#[cfg_attr(not(feature = "script-recipes"), allow(dead_code))]
pub struct RecipeTicket {
    #[allow(dead_code)]
    pub(crate) mark: RecipeMark,
    pub(crate) op: Op,
    pub(crate) installs: RecipeInstalls,
    /// The catalogue's address: the setting, or the default.
    pub(crate) url: String,
    /// The wall clock of the request.
    pub(crate) wall: u64,
    pub(crate) firmware: Version,
}

impl RecipeTicket {
    /// True for a catalogue refresh (a recipe job otherwise).
    pub fn is_refresh(&self) -> bool {
        self.op == Op::Refresh
    }
}

/// True when `text` is an address the catalogue may be read at (the setting `recipes.catalogue`).
pub fn catalogue_url_allowed(text: &str) -> bool {
    text.chars().count() <= crate::settings::MAX_MANIFEST_URL_CHARS && url_allowed(text)
}
