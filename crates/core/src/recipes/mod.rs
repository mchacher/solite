//! Recipes (spec 012): Sowel's `RecipeInstance`, `RecipeLogEntry` and recipe routes, with the
//! recipe engine that holds the instances, gives them the events and the time, and keeps their
//! logs. A recipe is written once against the recipe SDK (`solite-recipe-sdk`): the engine asks the
//! registry for the provider of a recipe id (the built-in recipes, `motion-light`, then the script
//! recipes loaded, spec 041), calls it through the SDK's `Recipe` trait with a `NativeHost` over
//! the home, and keeps the state text and listen set each call returns (no listen set: the one it
//! had), so that readers never call
//! a recipe. A script recipe can fail as a whole (its runtime is dropped, its instances stop) and
//! comes back by itself within a bound. No I/O and no thread: the host's radio loop gives the
//! events and the time (`turn`, or `handle` and `tick`), a recipe's orders go to the home's queue,
//! its timers are deadlines checked on each turn.

pub mod install;
pub mod native;
#[cfg(feature = "script-recipes")]
pub mod script;

/// A log line's level (Sowel's `RecipeLogEntry.level`).
pub use solite_recipe_sdk::Level;
/// Durations as Sowel writes and reads them (moved to the SDK, which recipes share).
pub use solite_recipe_sdk::duration;

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;
use solite_recipe_sdk::bounds::MAX_PARAMS_DEPTH;
use solite_recipe_sdk::{
    Change, ChangedEquipment, Clock, ErrorKind, Event as SdkEvent, EventValue as SdkValue,
    ListenEntry, Recipe, RecipeError, Update,
};

use crate::devices::{Id, iso};
use crate::equipments::Alias;
use crate::failure::{Failure, Outcome};
use crate::home::Home;
use crate::home::events::{Event, EventValue, Interest, MAX_EVENTS, cut};
use crate::live::{EngineEvent, LiveHandle, Topic};
use native::{NativeHost, RecipeName};

/// Instances kept; one more is refused (400).
pub const MAX_INSTANCES: usize = 16;
/// Log entries kept per instance; the oldest goes first.
pub const MAX_LOG_ENTRIES: usize = 20;
/// A log message is cut at this many bytes.
pub const MAX_LOG_MESSAGE_BYTES: usize = 200;
/// The longest `params`, serialized, in bytes.
pub const MAX_PARAMS_BYTES: usize = 2048;
// Worst case of the bounds: 16 instances x 20 entries x about 260 bytes (200 of message, the rest
// of the entry) is about 83 KB of logs, plus 16 x 2 KB of params kept as text (never parsed
// while kept: 2 KB of nested JSON takes about 112 KB once parsed): about 115 KB. The ceiling test
// measures the params of 16 adversarial instances at about 2.5 KB each. It fits the box's PSRAM
// (2 MB); a home of a few instances holds a few KB.

/// Script recipes loaded at once (spec 041, R3.3). Worst case: 4 runtimes of at most
/// `SCRIPT_MEMORY_LIMIT` (224 KB) each, 896 KB, counted on their own line of the memory budget.
pub const MAX_SCRIPT_RECIPES: usize = 4;
/// Instances of one script recipe, at most (they share its runtime, which is bounded: about 10 KB
/// each on the box). A fifth is refused (400); `MAX_INSTANCES` still bounds all recipes together.
pub const MAX_INSTANCES_PER_SCRIPT: usize = 4;
/// A failed script recipe comes back by itself this long after the failure (R3.6).
pub const SCRIPT_RESTART_DELAY_MS: u64 = 60_000;
/// ... at most this many times in a sliding window of `SCRIPT_RESTART_WINDOW_MS` (24 hours); after
/// that, only a person's action brings it back. Worst case: 3 timestamps (24 bytes) per script
/// recipe.
pub const SCRIPT_MAX_AUTO_RESTARTS: usize = 3;
pub const SCRIPT_RESTART_WINDOW_MS: u64 = 24 * 60 * 60 * 1000;
/// Script calls of one turn of the engine's loop take at most this long (R3.3): the events and
/// timers left are carried to the next turn, so scripts never hold the radio loop longer.
pub const SCRIPT_TURN_LIMIT_MS: u64 = 300;
/// Events carried from one turn to the next when scripts ran out of time, at most. Worst case: 128
/// events (the home's own queue bound) of about 100 bytes, 13 KB, and only while scripts are slow;
/// past it the oldest is dropped (with a warning).
pub const MAX_CARRIED_EVENTS: usize = MAX_EVENTS;

/// Sowel's `RecipeInstance` (`src/shared/types.ts` T:834).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeInstance {
    pub id: Id,
    pub recipe_id: String,
    /// The parameters as the JSON text received (at most `MAX_PARAMS_BYTES`): kept as text, since
    /// parsed JSON takes far more memory; parsed only for a moment (validation, start).
    #[serde(with = "raw_json")]
    pub params: Box<str>,
    pub enabled: bool,
    #[serde(with = "iso")]
    pub created_at: u64,
}

/// `params` as it is written and read: the JSON text itself, never a parsed value.
pub(crate) mod raw_json {
    use serde::de::Error as _;
    use serde::ser::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::value::RawValue;

    pub fn serialize<S: Serializer>(text: &str, serializer: S) -> Result<S::Ok, S::Error> {
        let raw: &RawValue = serde_json::from_str(text).map_err(S::Error::custom)?;
        raw.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Box<str>, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer).map_err(D::Error::custom)?;
        Ok(raw.get().into())
    }
}

/// Sowel's `RecipeLogEntry`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeLogEntry {
    /// Counts from 1 since the program started.
    pub id: u64,
    pub instance_id: Id,
    #[serde(with = "iso")]
    pub timestamp: u64,
    pub message: Box<str>,
    pub level: Level,
}

/// The params text served as `state` when an instance has no state: no key.
const NO_STATE: &str = "{}";

/// Makes a script recipe's runtime again from its source: a new recipe, or the reason it cannot.
pub type Rebuild = Box<dyn FnMut() -> Result<Box<dyn Recipe + Send>, String> + Send>;

/// Where a script recipe comes from (spec 042 R2.3): given by `--recipe-script` (the Mac's
/// developer flag), or installed from a signed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptSource {
    BuiltIn,
    /// A signed file kept in the data folder, at this version (the header's).
    Installed {
        version: crate::update::Version,
    },
}

/// What a script recipe has that a built-in one has not: its definition (kept while its runtime is
/// dropped), how to make the runtime again, and whether it runs.
#[cfg_attr(not(feature = "script-recipes"), allow(dead_code))]
struct ScriptMeta {
    source: ScriptSource,
    /// The script's text, shared with `rebuild`: an update that does not load puts it back.
    text: Arc<str>,
    /// The memory limit of its runtime, in bytes (shown by `GET /system/resources`).
    limit: usize,
    /// The `version` the script declares, if any (shown by `GET /system/resources`).
    declared: Option<String>,
    /// The `RecipeInfo` as JSON text, parsed when asked for: one block, not a tree of small ones
    /// (the box's internal RAM, spec 041).
    info: String,
    rebuild: Rebuild,
    /// `Some` while the recipe failed and its runtime is dropped.
    failed: Option<Failed>,
    /// The monotonic times of the automatic restarts in the last 24 hours, at most
    /// `SCRIPT_MAX_AUTO_RESTARTS`.
    restarts: Vec<u64>,
    /// The instances a restart has not started yet (the turn's budget was spent), in creation
    /// order, at most `MAX_INSTANCES`; the next turns start them.
    to_start: Vec<Id>,
}

struct Failed {
    since: u64,
    reason: String,
}

/// What `add_script` keeps of where a script came from.
#[cfg(feature = "script-recipes")]
pub(crate) struct ScriptOrigin {
    pub source: ScriptSource,
    pub text: Arc<str>,
    pub limit: usize,
    pub declared: Option<String>,
}

/// A recipe the engine provides: linked into the core (a built-in recipe) or a script recipe, both
/// called through the SDK's `Recipe` trait. Its id and name are read once from its definition and
/// shared with the orders it sends.
struct Native {
    names: RecipeName,
    /// `None` while a script recipe is failed.
    recipe: Option<Box<dyn Recipe + Send>>,
    script: Option<ScriptMeta>,
}

impl Native {
    #[cfg(test)]
    fn new(recipe: Box<dyn Recipe + Send>) -> Native {
        let names = Native::names_of(&recipe.info());
        Native {
            names,
            recipe: Some(recipe),
            script: None,
        }
    }

    #[cfg(test)]
    fn names_of(info: &Value) -> RecipeName {
        let text = |key: &str| {
            info.get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        RecipeName {
            id: Arc::from(text("id")),
            name: Arc::from(text("name")),
        }
    }

    /// The id and the name read from the JSON text, with no tree of the rest.
    #[cfg(feature = "script-recipes")]
    fn names_of_text(info: &str) -> RecipeName {
        #[derive(serde::Deserialize, Default)]
        struct Head {
            #[serde(default)]
            id: String,
            #[serde(default)]
            name: String,
        }
        let head: Head = serde_json::from_str(info).unwrap_or_default();
        RecipeName {
            id: Arc::from(head.id),
            name: Arc::from(head.name),
        }
    }

    fn info(&self) -> Value {
        match (&self.script, &self.recipe) {
            (Some(script), _) => serde_json::from_str(&script.info).unwrap_or(Value::Null),
            (None, Some(recipe)) => recipe.info(),
            (None, None) => Value::Null,
        }
    }

    /// The definition to serve: a script's is its own JSON text, written as it is (no tree).
    fn info_ref(&self) -> InfoRef<'_> {
        match &self.script {
            Some(script) => serde_json::from_str::<&RawValue>(&script.info)
                .map_or(InfoRef::Tree(Value::Null), InfoRef::Text),
            None => InfoRef::Tree(self.info()),
        }
    }

    /// The keys of the params that hold equipment ids: a script's are read from its JSON text,
    /// slot by slot, with no tree of the definition.
    fn equipment_keys(&self) -> Vec<String> {
        #[derive(serde::Deserialize)]
        struct Slot {
            #[serde(rename = "type", default)]
            kind: Option<String>,
            #[serde(default)]
            id: Option<String>,
        }
        #[derive(serde::Deserialize, Default)]
        struct Head {
            #[serde(default)]
            slots: Vec<Slot>,
        }
        match &self.script {
            Some(script) => serde_json::from_str::<Head>(&script.info)
                .unwrap_or_default()
                .slots
                .into_iter()
                .filter(|slot| slot.kind.as_deref() == Some("equipment"))
                .filter_map(|slot| slot.id)
                .collect(),
            None => equipment_keys(&self.info()),
        }
    }

    fn is_script(&self) -> bool {
        self.script.is_some()
    }

    /// The reason a failed script recipe failed.
    fn failure(&self) -> Option<&str> {
        self.script
            .as_ref()
            .and_then(|s| s.failed.as_ref())
            .map(|f| f.reason.as_str())
    }
}

/// Who provides a recipe id: its index in the engine's list (built-in recipes first, then the
/// script recipes in the order they were loaded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Native(usize),
}

fn provider_of(natives: &[Native], recipe_id: &str) -> Option<Provider> {
    natives
        .iter()
        .position(|native| &*native.names.id == recipe_id)
        .map(Provider::Native)
}

/// One thing a running instance listens to, in the compact form the home filters with.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Wanted {
    Zone(Id),
    Equipment(Id, Alias),
    Device(Id, Alias),
}

/// The listen sets of the running instances, kept once: instances of one recipe in one zone
/// listen to the same lights, buttons and sensors, so each distinct entry is stored once here and
/// an instance holds the (2-byte) indexes of its entries. At most 1 + 32 + 64 entries per
/// instance, 16 instances, so at most 1552 entries live (about 62 KB when no two instances listen
/// to the same thing, a few KB in a home); entries nobody listens to any more are dropped when the
/// table holds twice what is live (`RecipeEngine::compact_listens`).
#[derive(Debug, Default)]
struct Listens {
    entries: Vec<Wanted>,
}

impl Listens {
    /// The index of an entry, added when it is new.
    fn intern(&mut self, wanted: Wanted) -> Option<u16> {
        let index = match self.entries.iter().position(|e| *e == wanted) {
            Some(index) => index,
            None => {
                self.entries.push(wanted);
                self.entries.len() - 1
            }
        };
        u16::try_from(index).ok()
    }

    /// The indexes of a listen set: an id that is not an id is dropped, each entry once.
    fn indexes(&mut self, listen: &[ListenEntry]) -> Vec<u16> {
        let mut indexes: Vec<u16> = Vec::new();
        for entry in listen {
            let wanted = match entry {
                ListenEntry::Zone { zone } => Id::parse(zone).map(Wanted::Zone),
                ListenEntry::Equipment { equipment, alias } => {
                    Id::parse(equipment).map(|e| Wanted::Equipment(e, Alias::new(alias)))
                }
                ListenEntry::Device { device, key } => {
                    Id::parse(device).map(|d| Wanted::Device(d, Alias::new(key)))
                }
            };
            if let Some(index) = wanted.and_then(|w| self.intern(w))
                && !indexes.contains(&index)
            {
                indexes.push(index);
            }
        }
        indexes.shrink_to_fit();
        indexes
    }
}

/// What the engine keeps of a running instance: which recipe and handle it is, and the last state
/// text and listen set a call returned.
struct Running {
    provider: usize,
    handle: u32,
    /// A JSON object, served as the recipe wrote it.
    state: Box<str>,
    /// The entries of the last listen set, indexes into the engine's `Listens`.
    listen: Vec<u16>,
}

/// An instance, running or not, with its log.
struct Slot {
    instance: RecipeInstance,
    running: Option<Running>,
    /// At most `MAX_LOG_ENTRIES`, the oldest first.
    log: VecDeque<RecipeLogEntry>,
}

/// Stops the running instance of a slot, if any: the recipe forgets it. True when it ran.
fn stop_running(natives: &mut [Native], slot: &mut Slot) -> bool {
    let Some(running) = slot.running.take() else {
        return false;
    };
    if let Some(recipe) = natives
        .get_mut(running.provider)
        .and_then(|native| native.recipe.as_mut())
    {
        recipe.stop(running.handle);
    }
    true
}

/// The monotonic clock the script runtimes' time limits and the turn's bound read, in
/// milliseconds (the host's and the firmware's real one; a fake in tests).
pub type ScriptClock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// An event that scripts have not all had yet: the instances from `resume` on (all of them while
/// `None`) still have to get it. The built-in recipes got it at once, in the turn it came.
struct Carried {
    event: Event,
    /// The instance to go on with and its place in the list when it was left: the list is
    /// in creation order, so when the instance was deleted meanwhile, the place it held is where
    /// the instances after it now are.
    resume: Option<(Id, usize)>,
}

/// How much script time is left in a turn.
struct Budget {
    clock: Option<ScriptClock>,
    started: u64,
}

impl Budget {
    fn none() -> Budget {
        Budget {
            clock: None,
            started: 0,
        }
    }

    fn of(clock: &Option<ScriptClock>) -> Budget {
        match clock {
            Some(clock) => Budget {
                started: clock(),
                clock: Some(clock.clone()),
            },
            None => Budget::none(),
        }
    }

    fn spent(&self) -> bool {
        self.clock
            .as_ref()
            .is_some_and(|clock| clock().saturating_sub(self.started) >= SCRIPT_TURN_LIMIT_MS)
    }
}

/// The instances, at most `MAX_INSTANCES`, in the order they were created.
pub struct RecipeEngine {
    slots: Vec<Slot>,
    next_log_id: u64,
    /// Where `recipe.instance.*` events go (spec 013), when the host listens.
    live: Option<LiveHandle>,
    /// The recipes: the built-in ones (a handful, fixed at build time), then the script recipes
    /// loaded (at most `MAX_SCRIPT_RECIPES`), in the order they were loaded.
    natives: Vec<Native>,
    /// Events the scripts have not all had (see `Carried`), at most `MAX_CARRIED_EVENTS`.
    carry: VecDeque<Carried>,
    /// The clock of the scripts' time limits, given with the first script loaded.
    script_clock: Option<ScriptClock>,
    /// What the running instances listen to, each distinct entry once.
    listens: Listens,
    /// The handle the next started instance gets (handles are never reused until they wrap).
    next_handle: u32,
    /// True when what the running instances listen to may have changed since `take_interest_stale`
    /// (an instance started, failed or left a new listen set): the home recomputes its interest.
    interest_stale: bool,
}

impl Default for RecipeEngine {
    fn default() -> Self {
        RecipeEngine::new()
    }
}

/// Builds and sends a `recipes` event, only if a client listens to that topic.
fn announce(live: &Option<LiveHandle>, build: impl FnOnce() -> EngineEvent) {
    if let Some(live) = live {
        live.emit(Topic::Recipes, build);
    }
}

impl std::fmt::Debug for RecipeEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecipeEngine")
            .field("instances", &self.slots.len())
            .finish()
    }
}

/// Counts the bytes written to it and keeps none.
struct Counter(usize);

impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The parameters serialized, kept as text; `None` when they are over `MAX_PARAMS_BYTES`. The
/// size is counted first, so that nothing large is ever built.
fn params_text(params: &Value) -> Option<Box<str>> {
    let mut counter = Counter(0);
    if serde_json::to_writer(&mut counter, params).is_err() || counter.0 > MAX_PARAMS_BYTES {
        return None;
    }
    serde_json::to_string(params)
        .ok()
        .map(String::into_boxed_str)
}

/// How deep `value` nests: a scalar is 0, an object or a list is 1 plus its deepest member (the
/// params object itself is the first level). It stops looking `limit` levels down: the answer is
/// at most `limit + 1`, so that nothing deeper is ever walked.
fn depth(value: &Value, limit: usize) -> usize {
    let members: Box<dyn Iterator<Item = &Value> + '_> = match value {
        Value::Array(list) => Box::new(list.iter()),
        Value::Object(fields) => Box::new(fields.values()),
        _ => return 0,
    };
    if limit == 0 {
        return 1;
    }
    1 + members.map(|v| depth(v, limit - 1)).max().unwrap_or(0)
}

/// 400 `Invalid params: params nested too deeply` for params nested over `MAX_PARAMS_DEPTH`
/// levels: checked by the engine before any recipe sees them, so that no recipe (built in or
/// script) ever parses params deeper than that.
fn check_depth(params: &Value) -> Result<(), Failure> {
    if depth(params, MAX_PARAMS_DEPTH) > MAX_PARAMS_DEPTH {
        return Err(Failure::new(
            400,
            "Invalid params: params nested too deeply",
        ));
    }
    Ok(())
}

fn not_found() -> Failure {
    Failure::new(404, "Instance not found")
}

/// A change of the home that a settle looks at (spec 024, R4): the equipment deleted (its id and
/// name), the zone deleted. Both `None` for a change that only can make settings invalid (an
/// equipment changed, a binding or a device removed).
#[derive(Debug, Default, Clone)]
pub struct Settle {
    pub equipment: Option<(Id, String)>,
    pub zone: Option<Id>,
}

/// A recipe's definition as it is served: a script's own JSON text, or a tree (a built-in's).
pub enum InfoRef<'a> {
    Text(&'a RawValue),
    Tree(Value),
}

impl serde::Serialize for InfoRef<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            InfoRef::Text(text) => text.serialize(serializer),
            InfoRef::Tree(tree) => tree.serialize(serializer),
        }
    }
}

/// The keys of a recipe's params that hold equipment ids: its `equipment` slots.
fn equipment_keys(info: &Value) -> Vec<String> {
    info.get("slots")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|slot| slot.get("type").and_then(Value::as_str) == Some("equipment"))
        .filter_map(|slot| slot.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// Takes the id `gone` out of the given keys of `params` (removed from a list, a single id
/// becomes `""`), and says which keys named it; none when nothing did (or `params` is not an
/// object).
fn prune_equipment(params: &mut Value, keys: &[String], gone: &str) -> Vec<String> {
    let Some(fields) = params.as_object_mut() else {
        return Vec::new();
    };
    let mut pruned = Vec::new();
    for key in keys {
        match fields.get_mut(key.as_str()) {
            Some(Value::Array(list)) => {
                let before = list.len();
                list.retain(|v| v.as_str() != Some(gone));
                if list.len() != before {
                    pruned.push(key.clone());
                }
            }
            Some(slot @ Value::String(_)) if slot.as_str() == Some(gone) => {
                *slot = Value::String(String::new());
                pruned.push(key.clone());
            }
            _ => {}
        }
    }
    pruned
}

/// A core event as a recipe gets it: ids as canonical texts.
fn sdk_event(event: &Event) -> SdkEvent {
    let value = |value: &EventValue| match value {
        EventValue::Null => SdkValue::Null,
        EventValue::Bool(b) => SdkValue::Bool(*b),
        EventValue::Number(n) => SdkValue::Number(*n),
        EventValue::Text(text) => SdkValue::Text(text.to_string()),
    };
    match event {
        Event::Zone {
            zone,
            motion,
            luminosity,
        } => SdkEvent::Zone {
            zone: zone.to_string(),
            motion: *motion,
            luminosity: *luminosity,
        },
        Event::Equipment {
            equipment,
            alias,
            value: v,
        } => SdkEvent::Equipment {
            equipment: equipment.to_string(),
            alias: alias.to_string(),
            value: value(v),
        },
        Event::Device {
            device,
            key,
            value: v,
        } => SdkEvent::Device {
            device: device.to_string(),
            key: key.to_string(),
            value: value(v),
        },
    }
}

impl RecipeEngine {
    /// An engine with no instance and no recipe: no recipe is built in (spec 046). The scripts
    /// come from the catalogue, a file or `--recipe-script`. It is also what the home leaves in
    /// the engine's place while the real one is taken out to read the home
    /// (`Home::engine_reads_home`).
    pub fn new() -> Self {
        RecipeEngine {
            slots: Vec::new(),
            next_log_id: 0,
            live: None,
            natives: Vec::new(),
            carry: VecDeque::new(),
            script_clock: None,
            listens: Listens::default(),
            next_handle: 0,
            interest_stale: false,
        }
    }

    /// True once when the running instances or their listen sets changed since last asked.
    pub(crate) fn take_interest_stale(&mut self) -> bool {
        std::mem::take(&mut self.interest_stale)
    }

    /// Where the engine sends its `recipe.instance.*` events.
    pub fn set_live(&mut self, live: LiveHandle) {
        self.live = Some(live);
    }

    // ---- The registry ----

    /// Who provides a recipe id; `None` for a recipe nobody provides.
    fn provider(&self, recipe_id: &str) -> Option<Provider> {
        provider_of(&self.natives, recipe_id)
    }

    /// The recipes there are, as `GET /recipes` lists them.
    /// The counters of each loaded script recipe that runs, in load order (spec 041 R6.3): its
    /// id and `solite_recipe_sdk::RunStats`. A script recipe that is failed has none.
    pub fn script_stats(&self) -> Vec<(String, solite_recipe_sdk::RunStats)> {
        self.natives
            .iter()
            .filter(|native| native.is_script())
            .filter_map(|native| {
                let stats = native.recipe.as_ref()?.run_stats()?;
                Some((native.names.id.to_string(), stats))
            })
            .collect()
    }

    /// Every recipe's definition, to serve as it is.
    pub fn recipe_infos(&self) -> Vec<InfoRef<'_>> {
        self.natives.iter().map(Native::info_ref).collect()
    }

    /// One recipe's definition, `None` for a recipe that does not exist.
    pub fn recipe_info(&self, id: &str) -> Option<InfoRef<'_>> {
        match self.provider(id)? {
            Provider::Native(index) => Some(self.natives[index].info_ref()),
        }
    }

    /// One recipe's definition as JSON text (the answer of an install or an update adds its
    /// `version`).
    pub fn recipe_info_text(&self, id: &str) -> Option<String> {
        match self.provider(id)? {
            Provider::Native(index) => {
                let native = &self.natives[index];
                Some(match &native.script {
                    Some(script) => script.info.clone(),
                    None => native.info().to_string(),
                })
            }
        }
    }

    /// Checks the parameters of an instance's recipe against the home, with the recipe's
    /// messages.
    fn validate(
        &self,
        home: &Home,
        recipe_id: &str,
        instance: Id,
        params: &Value,
        clock: Clock,
    ) -> Result<(), RecipeError> {
        match self.provider(recipe_id) {
            Some(Provider::Native(index)) => {
                let native = &self.natives[index];
                let Some(recipe) = &native.recipe else {
                    return Err(RecipeError::recipe(
                        native.failure().unwrap_or("failed").to_string(),
                    ));
                };
                let host = NativeHost::new(home, instance, &native.names, clock.mono, clock.wall);
                recipe.validate(&host, params)
            }
            None => Err(RecipeError::instance(format!(
                "Recipe not found: {recipe_id}"
            ))),
        }
    }

    /// Validates an instance's params text for its recipe.
    fn validate_text(
        &self,
        home: &Home,
        recipe_id: &str,
        instance: Id,
        text: &str,
        clock: Clock,
    ) -> Result<(), RecipeError> {
        let value = serde_json::from_str::<Value>(text)
            .map_err(|e| RecipeError::instance(format!("params are not JSON: {e}")))?;
        self.validate(home, recipe_id, instance, &value, clock)
    }

    // ---- Reading ----

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The instances, oldest first.
    pub fn instances(&self) -> impl Iterator<Item = &RecipeInstance> {
        self.slots.iter().map(|s| &s.instance)
    }

    pub fn get(&self, id: &Id) -> Option<&RecipeInstance> {
        self.slots
            .iter()
            .find(|s| s.instance.id == *id)
            .map(|s| &s.instance)
    }

    /// True while the instance runs (enabled, and started without failure).
    pub fn is_running(&self, id: &Id) -> bool {
        self.slots
            .iter()
            .any(|s| s.instance.id == *id && s.running.is_some())
    }

    /// The state of an instance, the JSON object its recipe wrote (`{}` when it has no key or
    /// does not run); `None` for an unknown id. It is the copy the last call left: no recipe is
    /// called.
    pub fn state_of(&self, id: &Id) -> Option<&str> {
        self.slots.iter().find(|s| s.instance.id == *id).map(|s| {
            s.running
                .as_ref()
                .map_or(NO_STATE, |running| &*running.state)
        })
    }

    /// The last `limit` entries of an instance's log, newest first; none for an unknown id.
    pub fn log(&self, id: &str, limit: usize) -> Vec<&RecipeLogEntry> {
        let Some(id) = Id::parse(id) else {
            return Vec::new();
        };
        self.slots
            .iter()
            .find(|s| s.instance.id == id)
            .map(|s| s.log.iter().rev().take(limit).collect())
            .unwrap_or_default()
    }

    /// What the running instances listen to, from the listen sets their last calls returned.
    pub fn interest(&self) -> Interest {
        let mut interest = Interest::new();
        for running in self.slots.iter().filter_map(|s| s.running.as_ref()) {
            for wanted in running
                .listen
                .iter()
                .filter_map(|i| self.listens.entries.get(usize::from(*i)))
            {
                match wanted {
                    Wanted::Zone(zone) => interest.add_zone(*zone),
                    Wanted::Equipment(equipment, alias) => {
                        interest.add_equipment(*equipment, alias.clone());
                    }
                    Wanted::Device(device, key) => interest.add_device(*device, key),
                }
            }
        }
        interest
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        let id = Id::parse(id)?;
        self.slots.iter().position(|s| s.instance.id == id)
    }

    /// Drops the listen entries nobody listens to any more, once the table holds twice what is
    /// live: an instance that stops or changes its listen set leaves its entries behind until
    /// then.
    fn compact_listens(&mut self) {
        let live: usize = self
            .slots
            .iter()
            .filter_map(|s| s.running.as_ref())
            .map(|r| r.listen.len())
            .sum();
        if self.listens.entries.len() <= 2 * live + 64 {
            return;
        }
        let old = std::mem::take(&mut self.listens.entries);
        for running in self.slots.iter_mut().filter_map(|s| s.running.as_mut()) {
            running.listen = running
                .listen
                .iter()
                .filter_map(|i| old.get(usize::from(*i)))
                .filter_map(|wanted| self.listens.intern(wanted.clone()))
                .collect();
        }
        self.listens.entries.shrink_to_fit();
    }

    // ---- The log ----

    fn push_log(slot: &mut Slot, next_id: &mut u64, wall: u64, level: Level, message: &str) {
        if slot.log.len() >= MAX_LOG_ENTRIES {
            slot.log.pop_front();
        }
        *next_id += 1;
        slot.log.push_back(RecipeLogEntry {
            id: *next_id,
            instance_id: slot.instance.id,
            timestamp: wall,
            message: cut(message, MAX_LOG_MESSAGE_BYTES).into(),
            level,
        });
    }

    fn push_lines(slot: &mut Slot, next_id: &mut u64, wall: u64, lines: Vec<(Level, String)>) {
        for (level, message) in lines {
            RecipeEngine::push_log(slot, next_id, wall, level, &message);
        }
    }

    // ---- Starting and stopping ----

    /// Starts the instance at `index` (it is stopped): the recipe validates and starts it. A
    /// failure is the instance's log line `<failed>: <error>`, and the instance stays stopped;
    /// with `tell_error` it is also a `recipe.instance.error` event (only the start right after
    /// a create does, as Sowel's `createInstance`). A failure of the recipe as a whole (a script
    /// that overran its bounds) stops every instance of it (`fail_script`) and is the error's kind.
    fn start_slot(
        &mut self,
        index: usize,
        home: &Home,
        mono: u64,
        wall: u64,
        failed: &str,
        tell_error: bool,
    ) -> Result<(), RecipeError> {
        if index >= self.slots.len() {
            return Ok(());
        }
        let handle = self.next_handle;
        let provider = provider_of(&self.natives, &self.slots[index].instance.recipe_id)
            .map(|Provider::Native(provider)| provider);
        // Every start goes through here: a script recipe runs at most `MAX_INSTANCES_PER_SCRIPT`
        // instances, whether they were created, restored, restarted after a failure or updated.
        let full = provider.is_some_and(|p| {
            self.natives[p].is_script()
                && self
                    .slots
                    .iter()
                    .filter(|s| s.running.as_ref().is_some_and(|r| r.provider == p))
                    .count()
                    >= MAX_INSTANCES_PER_SCRIPT
        });
        let (result, lines) = match provider {
            None => (
                Err(RecipeError::instance(format!(
                    "Recipe not found: {}",
                    self.slots[index].instance.recipe_id
                ))),
                Vec::new(),
            ),
            Some(_) if full => (
                Err(RecipeError::instance(format!(
                    "Too many instances of {} (max {MAX_INSTANCES_PER_SCRIPT})",
                    self.slots[index].instance.recipe_id
                ))),
                Vec::new(),
            ),
            Some(provider) => {
                let slot = &self.slots[index];
                let native = &mut self.natives[provider];
                let failure = native.failure().map(str::to_string);
                // The parsed value lives for this start only.
                let parsed = serde_json::from_str::<Value>(&slot.instance.params)
                    .map_err(|e| format!("params are not JSON: {e}"));
                match (parsed, native.recipe.as_mut(), failure) {
                    (_, None, failure) => (
                        Err(RecipeError::recipe(
                            failure.unwrap_or_else(|| "failed".to_string()),
                        )),
                        Vec::new(),
                    ),
                    (Err(error), _, _) => (Err(RecipeError::instance(error)), Vec::new()),
                    (Ok(value), Some(recipe), _) => {
                        let host =
                            NativeHost::new(home, slot.instance.id, &native.names, mono, wall);
                        let result = recipe.start(&host, handle, &value, Clock { mono, wall });
                        (result, host.take_lines())
                    }
                }
            }
        };
        let slot = &mut self.slots[index];
        RecipeEngine::push_lines(slot, &mut self.next_log_id, wall, lines);
        match (result, provider) {
            (Ok(update), Some(provider)) => {
                self.interest_stale = true;
                self.next_handle = self.next_handle.wrapping_add(1);
                let started = update.state.as_str() != NO_STATE;
                if update.listen.is_none() {
                    // The SDK's contract: a start gives its listen set. Without one the instance
                    // listens to nothing; said, not hidden.
                    log::warn!(
                        "recipes: {} started with no listen set: it hears no event",
                        slot.instance.recipe_id
                    );
                }
                slot.running = Some(Running {
                    provider,
                    handle,
                    state: update.state.into(),
                    listen: update
                        .listen
                        .as_deref()
                        .map(|l| self.listens.indexes(l))
                        .unwrap_or_default(),
                });
                let (id, recipe) = (slot.instance.id, &slot.instance.recipe_id);
                announce(&self.live, || EngineEvent::RecipeInstanceStarted {
                    instance_id: id,
                    recipe_id: recipe.clone(),
                });
                // A start that wrote state keys is a write of them.
                if started {
                    announce(&self.live, || EngineEvent::RecipeInstanceStateChanged {
                        instance_id: id,
                        recipe_id: recipe.clone(),
                    });
                }
                Ok(())
            }
            (Ok(_), None) => Ok(()),
            (Err(error), provider) => {
                slot.running = None;
                let (id, recipe) = (slot.instance.id, slot.instance.recipe_id.clone());
                if error.kind == ErrorKind::Recipe {
                    let message = format!("Script recipe failed: {}", error.message);
                    RecipeEngine::push_log(
                        slot,
                        &mut self.next_log_id,
                        wall,
                        Level::Error,
                        &message,
                    );
                    announce(&self.live, || EngineEvent::RecipeInstanceError {
                        instance_id: id,
                        recipe_id: recipe.clone(),
                        error: message.clone(),
                    });
                    if let Some(provider) = provider {
                        self.fail_script(provider, error.message.clone(), wall, mono);
                    }
                    return Err(error);
                }
                let message = format!("{failed}: {}", error.message);
                RecipeEngine::push_log(slot, &mut self.next_log_id, wall, Level::Error, &message);
                if tell_error {
                    announce(&self.live, || EngineEvent::RecipeInstanceError {
                        instance_id: id,
                        recipe_id: recipe.clone(),
                        error: error.message.clone(),
                    });
                }
                Err(error)
            }
        }
    }

    // ---- Script recipes: failure and return (spec 041, R3.5, R3.6) ----

    /// A script recipe failed as a whole: its runtime is dropped, each running instance of it
    /// stops with `Script recipe failed: <reason>` and the events `recipe.instance.stopped` and
    /// `recipe.instance.error`. The instances stay enabled and nothing is written. The engine and
    /// the other recipes run on.
    fn fail_script(&mut self, provider: usize, reason: String, wall: u64, mono: u64) {
        self.interest_stale = true;
        let Some(native) = self.natives.get_mut(provider) else {
            return;
        };
        let Some(meta) = native.script.as_mut() else {
            return;
        };
        if meta.failed.is_some() {
            return;
        }
        // The runtime goes with the recipe.
        native.recipe = None;
        meta.restarts
            .retain(|at| mono.saturating_sub(*at) < SCRIPT_RESTART_WINDOW_MS);
        meta.failed = Some(Failed {
            since: mono,
            reason: reason.clone(),
        });
        log::error!("script recipe {} failed: {reason}", native.names.id);
        let message = format!("Script recipe failed: {reason}");
        for slot in &mut self.slots {
            if slot.running.as_ref().map(|r| r.provider) != Some(provider) {
                continue;
            }
            // The recipe is gone: there is nothing to tell it.
            slot.running = None;
            RecipeEngine::push_log(slot, &mut self.next_log_id, wall, Level::Error, &message);
            RecipeEngine::announce_stopped(&self.live, slot);
            let (id, recipe) = (slot.instance.id, &slot.instance.recipe_id);
            announce(&self.live, || EngineEvent::RecipeInstanceError {
                instance_id: id,
                recipe_id: recipe.clone(),
                error: message.clone(),
            });
        }
        self.compact_listens();
    }

    /// Makes a failed script recipe's runtime again and starts its enabled instances. `auto` is
    /// the return by itself (counted in the 3 of 24 hours); otherwise a person's action. The
    /// reason when the runtime cannot be made.
    fn restart_script(
        &mut self,
        provider: usize,
        home: &Home,
        wall: u64,
        mono: u64,
        auto: bool,
        budget: &Budget,
    ) -> Result<(), String> {
        let Some(native) = self.natives.get_mut(provider) else {
            return Ok(());
        };
        let Some(meta) = native.script.as_mut() else {
            return Ok(());
        };
        if meta.failed.is_none() {
            return Ok(());
        }
        meta.restarts
            .retain(|at| mono.saturating_sub(*at) < SCRIPT_RESTART_WINDOW_MS);
        if auto {
            meta.restarts.push(mono);
        }
        let restarts = meta.restarts.len();
        let recipe = match (meta.rebuild)() {
            Ok(recipe) => recipe,
            Err(reason) => {
                meta.failed = Some(Failed {
                    since: mono,
                    reason: reason.clone(),
                });
                let message = format!("Script recipe failed: {reason}");
                for slot in &mut self.slots {
                    if slot.instance.enabled && slot.instance.recipe_id == *native.names.id {
                        RecipeEngine::push_log(
                            slot,
                            &mut self.next_log_id,
                            wall,
                            Level::Error,
                            &message,
                        );
                    }
                }
                return Err(reason);
            }
        };
        meta.failed = None;
        native.recipe = Some(recipe);
        let message = if auto {
            format!("Script recipe restarted ({restarts} of {SCRIPT_MAX_AUTO_RESTARTS} in 24 h)")
        } else {
            "Script recipe restarted".to_string()
        };
        // The enabled instances, in order: the log says so now, the starts follow while the turn's
        // budget lasts (a person's action has none) and on the next turns.
        let mut waiting = Vec::new();
        for slot in &mut self.slots {
            let mine = provider_of(&self.natives, &slot.instance.recipe_id)
                == Some(Provider::Native(provider));
            if !mine || !slot.instance.enabled || slot.running.is_some() {
                continue;
            }
            RecipeEngine::push_log(slot, &mut self.next_log_id, wall, Level::Info, &message);
            waiting.push(slot.instance.id);
        }
        if let Some(meta) = self.natives[provider].script.as_mut() {
            meta.to_start = waiting;
        }
        self.start_waiting(provider, home, wall, mono, budget);
        self.compact_listens();
        Ok(())
    }

    /// Starts the instances a restart left waiting, in order, while the turn's budget lasts.
    fn start_waiting(
        &mut self,
        provider: usize,
        home: &Home,
        wall: u64,
        mono: u64,
        budget: &Budget,
    ) {
        loop {
            let native = &mut self.natives[provider];
            let Some(meta) = native.script.as_mut() else {
                return;
            };
            if native.recipe.is_none() {
                meta.to_start.clear();
            }
            if meta.to_start.is_empty() || budget.spent() {
                return;
            }
            let id = meta.to_start.remove(0);
            let Some(index) = self.slots.iter().position(|s| s.instance.id == id) else {
                continue;
            };
            if !self.slots[index].instance.enabled || self.slots[index].running.is_some() {
                continue;
            }
            let _ = self.start_slot(index, home, mono, wall, "Failed to restart", false);
            self.interest_stale = true;
        }
    }

    /// A script that failed and may come back by itself: a minute after the failure, at most
    /// `SCRIPT_MAX_AUTO_RESTARTS` times in 24 hours. One per turn (its load takes up to
    /// `SCRIPT_LOAD_LIMIT_MS`, and its instances start after it, within the turn's budget).
    fn restart_due_scripts(&mut self, home: &Home, wall: u64, mono: u64, budget: &Budget) {
        for provider in 0..self.natives.len() {
            let due = self.natives[provider].script.as_ref().is_some_and(|meta| {
                meta.failed.as_ref().is_some_and(|failed| {
                    let recent = meta
                        .restarts
                        .iter()
                        .filter(|at| mono.saturating_sub(**at) < SCRIPT_RESTART_WINDOW_MS)
                        .count();
                    recent < SCRIPT_MAX_AUTO_RESTARTS
                        && mono.saturating_sub(failed.since) >= SCRIPT_RESTART_DELAY_MS
                })
            });
            if due {
                // A runtime that cannot be made is logged; the next try is a minute later.
                let _ = self.restart_script(provider, home, wall, mono, true, budget);
                self.interest_stale = true;
                return;
            }
        }
    }

    /// A person's action on a recipe whose script failed: its runtime is made again at once and
    /// its enabled instances start; 500 `Script recipe failed: <reason>` when it fails again.
    fn revive(&mut self, provider: usize, home: &Home, wall: u64, mono: u64) -> Outcome<()> {
        if self.natives[provider].failure().is_none() {
            return Ok(());
        }
        self.restart_script(provider, home, wall, mono, false, &Budget::none())
            .map_err(|reason| Failure::new(500, format!("Script recipe failed: {reason}")))?;
        match self.natives[provider].failure() {
            None => Ok(()),
            Some(reason) => Err(Failure::new(500, format!("Script recipe failed: {reason}"))),
        }
    }

    /// The failure a validation that went wrong is: 400 for the params, and 500 (with the recipe
    /// failed) when the script as a whole failed while checking them.
    fn validation_failure(
        &mut self,
        recipe_id: &str,
        error: RecipeError,
        wall: u64,
        mono: u64,
    ) -> Failure {
        if error.kind == ErrorKind::Recipe {
            if let Some(Provider::Native(provider)) = self.provider(recipe_id) {
                self.fail_script(provider, error.message.clone(), wall, mono);
            }
            return Failure::new(500, format!("Script recipe failed: {}", error.message));
        }
        Failure::new(400, format!("Invalid params: {}", error.message))
    }

    /// Adds a script recipe the loader made (spec 041, R3.8): refused when its id is a built-in
    /// recipe's or a script's already loaded, or when `MAX_SCRIPT_RECIPES` are loaded.
    #[cfg(feature = "script-recipes")]
    pub(crate) fn add_script(
        &mut self,
        recipe: Box<dyn Recipe + Send>,
        rebuild: Rebuild,
        clock: ScriptClock,
        origin: ScriptOrigin,
    ) -> Result<(), String> {
        let info = recipe.info_text();
        let names = Native::names_of_text(&info);
        match provider_of(&self.natives, &names.id) {
            Some(Provider::Native(index)) if self.natives[index].is_script() => {
                return Err(format!("id {} is already loaded", names.id));
            }
            Some(_) => return Err(format!("id {} is a built-in recipe", names.id)),
            None => {}
        }
        if self.natives.iter().filter(|n| n.is_script()).count() >= MAX_SCRIPT_RECIPES {
            return Err(format!(
                "too many script recipes (at most {MAX_SCRIPT_RECIPES})"
            ));
        }
        self.script_clock = Some(clock);
        self.natives.push(Native {
            names,
            recipe: Some(recipe),
            script: Some(ScriptMeta {
                source: origin.source,
                text: origin.text,
                limit: origin.limit,
                declared: origin.declared,
                info,
                rebuild,
                failed: None,
                restarts: Vec::new(),
                to_start: Vec::new(),
            }),
        });
        Ok(())
    }

    /// Tells a client that the instance at `index` stopped (it was running).
    fn announce_stopped(live: &Option<LiveHandle>, slot: &Slot) {
        let (id, recipe) = (slot.instance.id, &slot.instance.recipe_id);
        announce(live, || EngineEvent::RecipeInstanceStopped {
            instance_id: id,
            recipe_id: recipe.clone(),
        });
    }

    fn announce_state_changed(live: &Option<LiveHandle>, slot: &Slot) {
        let (id, recipe) = (slot.instance.id, &slot.instance.recipe_id);
        announce(live, || EngineEvent::RecipeInstanceStateChanged {
            instance_id: id,
            recipe_id: recipe.clone(),
        });
    }

    // ---- Changes ----

    /// Creates an instance (enabled) and starts it: 404 `Recipe not found: <id>`, 400 for too
    /// many instances, params too large, too deeply nested or invalid. A failure to start is only
    /// logged.
    pub fn create(
        &mut self,
        home: &Home,
        recipe_id: &str,
        params: Value,
        id: Id,
        wall: u64,
        mono: u64,
    ) -> Outcome<RecipeInstance> {
        let Some(Provider::Native(provider)) = self.provider(recipe_id) else {
            return Err(Failure::new(404, format!("Recipe not found: {recipe_id}")));
        };
        // A person's action brings a failed script recipe back, with its other instances.
        self.revive(provider, home, wall, mono)?;
        if self.natives[provider].is_script()
            && self
                .slots
                .iter()
                .filter(|s| s.instance.recipe_id == recipe_id)
                .count()
                >= MAX_INSTANCES_PER_SCRIPT
        {
            return Err(Failure::new(
                400,
                format!("Too many instances of {recipe_id} (max {MAX_INSTANCES_PER_SCRIPT})"),
            ));
        }
        if self.slots.len() >= MAX_INSTANCES {
            return Err(Failure::new(
                400,
                format!("Too many recipe instances (max {MAX_INSTANCES})"),
            ));
        }
        let text = params_text(&params)
            .ok_or_else(|| Failure::new(400, "Invalid params: params too large"))?;
        check_depth(&params)?;
        if let Err(error) = self.validate(home, recipe_id, id, &params, Clock { mono, wall }) {
            return Err(self.validation_failure(recipe_id, error, wall, mono));
        }
        drop(params);
        let instance = RecipeInstance {
            id,
            recipe_id: recipe_id.to_string(),
            params: text,
            enabled: true,
            created_at: wall,
        };
        let mut slot = Slot {
            instance: instance.clone(),
            running: None,
            log: VecDeque::new(),
        };
        RecipeEngine::push_log(
            &mut slot,
            &mut self.next_log_id,
            wall,
            Level::Info,
            "Instance created",
        );
        self.slots.push(slot);
        let index = self.slots.len() - 1;
        announce(&self.live, || EngineEvent::RecipeInstanceCreated {
            instance_id: id,
            recipe_id: recipe_id.to_string(),
        });
        // A failure to start is only logged and told: the instance is created. A script recipe
        // that fails at once is 500, and the instance is not kept.
        let started = self.start_slot(index, home, mono, wall, "Failed to start", true);
        if let Err(error) = started
            && error.kind == ErrorKind::Recipe
        {
            self.slots.pop();
            announce(&self.live, || EngineEvent::RecipeInstanceRemoved {
                instance_id: id,
                recipe_id: recipe_id.to_string(),
            });
            self.compact_listens();
            return Err(Failure::new(
                500,
                format!("Script recipe failed: {}", error.message),
            ));
        }
        self.compact_listens();
        Ok(instance)
    }

    /// Replaces the parameters: validated first (the running instance is untouched when they are
    /// refused), then the instance is stopped, saved, and started again if it is enabled.
    pub fn update(
        &mut self,
        home: &Home,
        id: &str,
        params: Value,
        wall: u64,
        mono: u64,
    ) -> Outcome<RecipeInstance> {
        let index = self.index_of(id).ok_or_else(not_found)?;
        let recipe_id = self.slots[index].instance.recipe_id.clone();
        let Some(Provider::Native(provider)) = self.provider(&recipe_id) else {
            return Err(Failure::new(404, format!("Recipe not found: {recipe_id}")));
        };
        self.revive(provider, home, wall, mono)?;
        let text = params_text(&params)
            .ok_or_else(|| Failure::new(400, "Invalid params: params too large"))?;
        check_depth(&params)?;
        let instance_id = self.slots[index].instance.id;
        if let Err(error) =
            self.validate(home, &recipe_id, instance_id, &params, Clock { mono, wall })
        {
            return Err(self.validation_failure(&recipe_id, error, wall, mono));
        }
        drop(params);
        let slot = &mut self.slots[index];
        if stop_running(&mut self.natives, slot) {
            RecipeEngine::announce_stopped(&self.live, slot);
        }
        slot.instance.params = text;
        let enabled = slot.instance.enabled;
        let instance = slot.instance.clone();
        if enabled {
            // A restart that fails is only logged (Sowel emits no error event here); a script
            // recipe that fails at once is 500, the new settings kept.
            let restarted = self.start_slot(index, home, mono, wall, "Failed to restart", false);
            if let Err(error) = restarted
                && error.kind == ErrorKind::Recipe
            {
                self.compact_listens();
                return Err(Failure::new(
                    500,
                    format!("Script recipe failed: {}", error.message),
                ));
            }
        }
        self.compact_listens();
        Ok(instance)
    }

    /// Stops an instance, forgets it and its log.
    pub fn delete(&mut self, id: &str) -> Outcome<()> {
        let index = self.index_of(id).ok_or_else(not_found)?;
        let mut slot = self.slots.remove(index);
        if stop_running(&mut self.natives, &mut slot) {
            RecipeEngine::announce_stopped(&self.live, &slot);
        }
        let (id, recipe) = (slot.instance.id, &slot.instance.recipe_id);
        announce(&self.live, || EngineEvent::RecipeInstanceRemoved {
            instance_id: id,
            recipe_id: recipe.clone(),
        });
        self.compact_listens();
        Ok(())
    }

    /// Deletes every instance of a recipe, as `delete` does one by one (events, logs forgotten).
    /// The number deleted.
    pub fn delete_instances_of(&mut self, recipe_id: &str) -> usize {
        let ids: Vec<String> = self
            .slots
            .iter()
            .filter(|slot| slot.instance.recipe_id == recipe_id)
            .map(|slot| slot.instance.id.to_string())
            .collect();
        ids.iter().filter(|id| self.delete(id).is_ok()).count()
    }

    /// Enables or disables an instance. Enabling one that is enabled already does nothing;
    /// otherwise it starts (a failure is only logged), and disabling stops it. True when the
    /// instance changed.
    pub fn set_enabled(
        &mut self,
        home: &Home,
        id: &str,
        enabled: bool,
        wall: u64,
        mono: u64,
    ) -> Outcome<bool> {
        let index = self.index_of(id).ok_or_else(not_found)?;
        let provider = provider_of(&self.natives, &self.slots[index].instance.recipe_id);
        if self.slots[index].instance.enabled == enabled {
            // A person's action on an enabled instance whose script recipe failed brings the
            // recipe back and starts its instances.
            if enabled
                && let Some(Provider::Native(provider)) = provider
                && self.natives[provider].failure().is_some()
            {
                self.revive(provider, home, wall, mono)?;
                return Ok(true);
            }
            return Ok(false);
        }
        if enabled && let Some(Provider::Native(provider)) = provider {
            self.revive(provider, home, wall, mono)?;
        }
        // An instance that cannot run is not enabled (spec 024, R4.7; Sowel enables it).
        if enabled {
            let slot = &self.slots[index];
            let (recipe_id, instance, params) = (
                slot.instance.recipe_id.clone(),
                slot.instance.id,
                slot.instance.params.clone(),
            );
            if let Err(error) =
                self.validate_text(home, &recipe_id, instance, &params, Clock { mono, wall })
            {
                return Err(self.validation_failure(&recipe_id, error, wall, mono));
            }
        }
        self.slots[index].instance.enabled = enabled;
        if enabled {
            // `started` came with the start; one that failed is told as a state change. A script
            // recipe that fails at once is 500, the instance enabled.
            match self.start_slot(index, home, mono, wall, "Failed to start", false) {
                Ok(()) => {}
                Err(error) if error.kind == ErrorKind::Recipe => {
                    return Err(Failure::new(
                        500,
                        format!("Script recipe failed: {}", error.message),
                    ));
                }
                Err(_) => RecipeEngine::announce_state_changed(&self.live, &self.slots[index]),
            }
        } else {
            let slot = &mut self.slots[index];
            if stop_running(&mut self.natives, slot) {
                RecipeEngine::announce_stopped(&self.live, slot);
            } else {
                RecipeEngine::announce_state_changed(&self.live, slot);
            }
            self.compact_listens();
        }
        Ok(true)
    }

    /// `POST /recipe-instances/:id/actions`: no recipe declares an action, so a running instance
    /// accepts any and ignores it; one that does not run (or does not exist) is 400.
    pub fn action(&self, id: &str) -> Outcome<()> {
        let running = Id::parse(id).is_some_and(|id| self.is_running(&id));
        if running {
            Ok(())
        } else {
            Err(Failure::new(400, "Instance not running"))
        }
    }

    // ---- Settling (spec 024, R4) ----

    /// Stops (if it runs) and disables the instance at `index`, with the reason in its log and
    /// the `error` event.
    fn disable_for(&mut self, index: usize, wall: u64, reason: &str) {
        let slot = &mut self.slots[index];
        if stop_running(&mut self.natives, slot) {
            RecipeEngine::announce_stopped(&self.live, slot);
        }
        slot.instance.enabled = false;
        let message = format!("Disabled: {reason}");
        RecipeEngine::push_log(slot, &mut self.next_log_id, wall, Level::Error, &message);
        let (id, recipe) = (slot.instance.id, &slot.instance.recipe_id);
        announce(&self.live, || EngineEvent::RecipeInstanceError {
            instance_id: id,
            recipe_id: recipe.clone(),
            error: reason.to_string(),
        });
    }

    /// Settles the instances after a change of the home: the instances of a deleted zone are
    /// deleted, a deleted equipment leaves every instance's settings (the engine takes it out of
    /// the `equipment` slots, then the recipe prunes what is its own and says whether a running
    /// instance restarts), an enabled instance that no longer validates is disabled, a running
    /// one whose settings changed restarts. True when something is to be written.
    pub fn settle(&mut self, home: &Home, change: &Settle, wall: u64, mono: u64) -> bool {
        let mut changed = false;
        let clock = Clock { mono, wall };
        // 4: the instances of a deleted zone go with it.
        if let Some(zone) = change.zone {
            let zone = zone.to_string();
            let gone: Vec<Id> = self
                .slots
                .iter()
                .filter(|s| {
                    serde_json::from_str::<Value>(&s.instance.params)
                        .ok()
                        .is_some_and(|p| p.get("zone").and_then(Value::as_str) == Some(&zone))
                })
                .map(|s| s.instance.id)
                .collect();
            for id in gone {
                changed |= self.delete(&id.to_string()).is_ok();
            }
        }
        // 1: a deleted equipment leaves the settings, out of every `equipment` slot.
        let mut pruned_slots: Vec<Vec<String>> = vec![Vec::new(); self.slots.len()];
        if let Some((gone, name)) = &change.equipment {
            let gone = gone.to_string();
            for (index, slot) in self.slots.iter_mut().enumerate() {
                let keys = match provider_of(&self.natives, &slot.instance.recipe_id) {
                    Some(Provider::Native(p)) => self.natives[p].equipment_keys(),
                    None => Vec::new(),
                };
                let Ok(mut params) = serde_json::from_str::<Value>(&slot.instance.params) else {
                    continue;
                };
                let pruned = prune_equipment(&mut params, &keys, &gone);
                if pruned.is_empty() {
                    continue;
                }
                // Shorter than the text it came from: it stays under `MAX_PARAMS_BYTES`.
                if let Ok(text) = serde_json::to_string(&params) {
                    slot.instance.params = text.into_boxed_str();
                }
                pruned_slots[index] = pruned;
                let message = format!("Equipment removed from settings: {name}");
                RecipeEngine::push_log(slot, &mut self.next_log_id, wall, Level::Warn, &message);
                changed = true;
            }
        }
        // 1b: the recipe's own pruning, for every instance, enabled or not (spec 028, R2); it
        // says whether the instance restarts. Only a lost light restarts a running instance (a
        // lost button is taken in place).
        let mut restart = vec![false; self.slots.len()];
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let Some(Provider::Native(provider)) =
                provider_of(&self.natives, &slot.instance.recipe_id)
            else {
                continue;
            };
            let Ok(params) = serde_json::from_str::<Value>(&slot.instance.params) else {
                continue;
            };
            let native = &self.natives[provider];
            // A script recipe that failed is left alone.
            let Some(recipe) = native.recipe.as_ref() else {
                continue;
            };
            let host = NativeHost::new(home, slot.instance.id, &native.names, mono, wall);
            let what = Change {
                equipment: change
                    .equipment
                    .as_ref()
                    .map(|(id, name)| ChangedEquipment {
                        id: id.to_string(),
                        name: name.clone(),
                    }),
                pruned: pruned_slots[index].clone(),
            };
            let Some(pruned) = recipe.prune(&host, &params, &what) else {
                continue;
            };
            // Shorter than the text it came from: it stays under `MAX_PARAMS_BYTES`.
            if let Ok(text) = serde_json::to_string(&pruned.params) {
                slot.instance.params = text.into_boxed_str();
            }
            restart[index] = pruned.restart;
            let lines = pruned
                .lines
                .into_iter()
                .map(|line| (line.level, line.message))
                .collect();
            RecipeEngine::push_lines(slot, &mut self.next_log_id, wall, lines);
            changed = true;
        }
        // 2, 3: every enabled instance checked again; a running one that is kept is refreshed.
        for (index, restart) in restart.into_iter().enumerate() {
            let slot = &self.slots[index];
            if !slot.instance.enabled {
                continue;
            }
            // The instances of a script recipe that failed are left alone.
            if let Some(Provider::Native(provider)) =
                provider_of(&self.natives, &slot.instance.recipe_id)
                && self.natives[provider].failure().is_some()
            {
                continue;
            }
            let checked = self.validate_text(
                home,
                &slot.instance.recipe_id,
                slot.instance.id,
                &slot.instance.params,
                clock,
            );
            match checked {
                Err(error) if error.kind == ErrorKind::Recipe => {
                    if let Some(Provider::Native(provider)) =
                        provider_of(&self.natives, &self.slots[index].instance.recipe_id)
                    {
                        self.fail_script(provider, error.message, wall, mono);
                    }
                }
                Err(error) => {
                    self.disable_for(index, wall, &error.message);
                    changed = true;
                }
                Ok(()) if restart && self.slots[index].running.is_some() => {
                    let slot = &mut self.slots[index];
                    stop_running(&mut self.natives, slot);
                    RecipeEngine::announce_stopped(&self.live, slot);
                    if let Err(error) =
                        self.start_slot(index, home, mono, wall, "Failed to restart", false)
                        && error.kind == ErrorKind::Instance
                    {
                        self.disable_for(index, wall, &error.message);
                    }
                }
                Ok(()) => self.refresh_slot(index, home, wall, mono),
            }
        }
        self.compact_listens();
        changed
    }

    /// 4: a running instance that is kept follows the home as it is now: its recipe takes again
    /// what it resolved from the home. A failure leaves the instance as it was; one of the recipe
    /// as a whole fails the recipe.
    fn refresh_slot(&mut self, index: usize, home: &Home, wall: u64, mono: u64) {
        let slot = &mut self.slots[index];
        let Some(running) = slot.running.as_mut() else {
            return;
        };
        let Ok(params) = serde_json::from_str::<Value>(&slot.instance.params) else {
            return;
        };
        let provider = running.provider;
        let native = &mut self.natives[provider];
        let Some(recipe) = native.recipe.as_mut() else {
            return;
        };
        let host = NativeHost::new(home, slot.instance.id, &native.names, mono, wall);
        let result = recipe.refresh(&host, running.handle, &params);
        let lines = host.take_lines();
        RecipeEngine::push_lines(slot, &mut self.next_log_id, wall, lines);
        match result {
            Ok(update) => {
                if let Some(running) = slot.running.as_mut() {
                    running.state = update.state.into();
                    if let Some(listen) = &update.listen {
                        running.listen = self.listens.indexes(listen);
                    }
                }
            }
            Err(error) if error.kind == ErrorKind::Recipe => {
                self.fail_script(provider, error.message, wall, mono);
            }
            Err(error) => log::warn!(
                "recipe instance {}: refresh failed: {}",
                slot.instance.id,
                error.message
            ),
        }
    }

    // ---- Loading ----

    /// Takes the instances read from storage, not started: at most `MAX_INSTANCES`, an id seen
    /// twice dropped (the log says so).
    pub fn load(&mut self, list: Vec<RecipeInstance>) {
        for instance in list {
            if self.slots.len() >= MAX_INSTANCES {
                log::warn!(
                    "recipes: more than {MAX_INSTANCES} instances, {} dropped",
                    instance.id
                );
                continue;
            }
            if self.slots.iter().any(|s| s.instance.id == instance.id) {
                log::warn!("recipes: instance {} is there twice, dropped", instance.id);
                continue;
            }
            self.slots.push(Slot {
                instance,
                running: None,
                log: VecDeque::new(),
            });
        }
    }

    /// Starts every enabled instance that is not running: the host's start, after the home was
    /// loaded. An unknown recipe or oversized parameters are kept and stay stopped.
    pub fn start_all(&mut self, home: &Home, wall: u64, mono: u64) {
        for index in 0..self.slots.len() {
            let provider = match self.provider(&self.slots[index].instance.recipe_id) {
                Some(Provider::Native(i)) => Some(i),
                _ => None,
            };
            let known = self
                .provider(&self.slots[index].instance.recipe_id)
                .is_some();
            let slot = &mut self.slots[index];
            if !slot.instance.enabled || slot.running.is_some() {
                continue;
            }
            if !known {
                let message = format!(
                    "Failed to restore: Recipe not found: {}",
                    slot.instance.recipe_id
                );
                RecipeEngine::push_log(slot, &mut self.next_log_id, wall, Level::Error, &message);
                continue;
            }
            if slot.instance.params.len() > MAX_PARAMS_BYTES {
                RecipeEngine::push_log(
                    slot,
                    &mut self.next_log_id,
                    wall,
                    Level::Error,
                    "Failed to restore: params too large",
                );
                continue;
            }
            if let Some(provider) = provider
                && self.natives[provider].is_script()
            {
                let recipe_id = &self.slots[index].instance.recipe_id;
                let before = self.slots[..index]
                    .iter()
                    .filter(|s| &s.instance.recipe_id == recipe_id)
                    .count();
                if before >= MAX_INSTANCES_PER_SCRIPT {
                    let message = format!(
                        "Failed to restore: Too many instances of {recipe_id} (max {MAX_INSTANCES_PER_SCRIPT})"
                    );
                    let slot = &mut self.slots[index];
                    RecipeEngine::push_log(
                        slot,
                        &mut self.next_log_id,
                        wall,
                        Level::Error,
                        &message,
                    );
                    continue;
                }
            }
            // A restore that fails is only logged: no `error` event (Sowel's `init`).
            let _ = self.start_slot(index, home, mono, wall, "Failed to restore", false);
        }
    }

    // ---- Events and time ----

    /// One call into the running instance at `index`. A call that returns a new state replaces
    /// the cached state and listen set (and is a `recipe.instance.state.changed`); the lines it
    /// logged go to the instance's log; a failure is logged and never stops the others; a failure
    /// of the recipe as a whole fails it (`fail_script`).
    fn call_slot(
        &mut self,
        index: usize,
        home: &Home,
        wall: u64,
        mono: u64,
        call: impl FnOnce(
            &mut dyn Recipe,
            &NativeHost<'_>,
            u32,
            Clock,
        ) -> Result<Option<Update>, RecipeError>,
    ) {
        let slot = &mut self.slots[index];
        let Some(running) = slot.running.as_mut() else {
            return;
        };
        let provider = running.provider;
        let native = &mut self.natives[provider];
        let Some(recipe) = native.recipe.as_mut() else {
            return;
        };
        let host = NativeHost::new(home, slot.instance.id, &native.names, mono, wall);
        let result = call(recipe.as_mut(), &host, running.handle, Clock { mono, wall });
        let lines = host.take_lines();
        let mut failure = None;
        let error = match result {
            Ok(Some(update)) => {
                // A write of a state key is `recipe.instance.state.changed`.
                if *update.state != *running.state {
                    let (id, recipe) = (slot.instance.id, &slot.instance.recipe_id);
                    announce(&self.live, || EngineEvent::RecipeInstanceStateChanged {
                        instance_id: id,
                        recipe_id: recipe.clone(),
                    });
                }
                running.state = update.state.into();
                // A call that changed only the state leaves the listen set, and the home's
                // interest, as they were.
                if let Some(listen) = &update.listen {
                    running.listen = self.listens.indexes(listen);
                    self.interest_stale = true;
                }
                None
            }
            Ok(None) => None,
            Err(error) if error.kind == ErrorKind::Recipe => {
                failure = Some(error.message);
                None
            }
            Err(error) => Some(error.message),
        };
        RecipeEngine::push_lines(slot, &mut self.next_log_id, wall, lines);
        if let Some(error) = error {
            log::error!("recipe instance {}: {error}", slot.instance.id);
            let message = format!("Error: {error}");
            RecipeEngine::push_log(slot, &mut self.next_log_id, wall, Level::Error, &message);
        }
        if let Some(reason) = failure {
            self.fail_script(provider, reason, wall, mono);
        }
    }

    /// True when the running instance listens to the event (what its last call said).
    fn listens_to(&self, running: &Running, event: &Event) -> bool {
        running
            .listen
            .iter()
            .filter_map(|i| self.listens.entries.get(usize::from(*i)))
            .any(|wanted| match (wanted, event) {
                (Wanted::Zone(zone), Event::Zone { zone: z, .. }) => zone == z,
                (
                    Wanted::Equipment(equipment, alias),
                    Event::Equipment {
                        equipment: e,
                        alias: a,
                        ..
                    },
                ) => equipment == e && alias == a,
                (
                    Wanted::Device(device, key),
                    Event::Device {
                        device: d, key: k, ..
                    },
                ) => device == d && **key == **k,
                _ => false,
            })
    }

    /// Gives an event to the instances that still have to get it: the built-in recipes first, all
    /// of them, at once, whatever the budget (`natives`, only the first time the event is given);
    /// then the script instances that listen to it, while the budget lasts. False when scripts ran
    /// out of time first (`entry` says where to go on).
    fn dispatch(
        &mut self,
        home: &Home,
        entry: &mut Carried,
        natives: bool,
        wall: u64,
        mono: u64,
        budget: &Budget,
    ) -> bool {
        let event = sdk_event(&entry.event);
        if natives {
            for index in 0..self.slots.len() {
                let Some(running) = &self.slots[index].running else {
                    continue;
                };
                if self.natives[running.provider].is_script() {
                    continue;
                }
                self.call_slot(index, home, wall, mono, |recipe, host, handle, clock| {
                    recipe.on_event(host, handle, &event, clock)
                });
            }
        }
        let from = match &entry.resume {
            None => 0,
            Some((id, place)) => {
                let place = (*place).min(self.slots.len());
                if self.slots.get(place).is_some_and(|s| s.instance.id == *id) {
                    place
                } else {
                    self.slots
                        .iter()
                        .position(|s| s.instance.id == *id)
                        .unwrap_or(place)
                }
            }
        };
        for index in from..self.slots.len() {
            let slot = &self.slots[index];
            let Some(running) = &slot.running else {
                continue;
            };
            if !self.natives[running.provider].is_script()
                || !self.listens_to(running, &entry.event)
            {
                continue;
            }
            if budget.spent() {
                entry.resume = Some((slot.instance.id, index));
                return false;
            }
            self.call_slot(index, home, wall, mono, |recipe, host, handle, clock| {
                recipe.on_event(host, handle, &event, clock)
            });
        }
        true
    }

    /// The timers that are due fire: the built-in recipes check theirs on every turn, a script
    /// instance is called only when its deadline is due, and while the budget lasts (a due one
    /// that waits is called on the next turn).
    fn tick_with(&mut self, home: &Home, wall: u64, mono: u64, budget: &Budget) {
        for index in 0..self.slots.len() {
            let Some(running) = &self.slots[index].running else {
                continue;
            };
            let native = &self.natives[running.provider];
            if let Some(recipe) = &native.recipe {
                let host = NativeHost::new(
                    home,
                    self.slots[index].instance.id,
                    &native.names,
                    mono,
                    wall,
                );
                let deadline = recipe.next_deadline(&host, running.handle);
                if !deadline.is_due(Clock { mono, wall }) {
                    continue;
                }
                if native.is_script() && budget.spent() {
                    continue;
                }
            }
            self.call_slot(index, home, wall, mono, |recipe, host, handle, clock| {
                recipe.tick(host, handle, clock)
            });
        }
    }

    /// Gives an event to every running instance, now (no time bound).
    pub fn handle(&mut self, home: &Home, event: &Event, wall: u64, mono: u64) {
        if self.slots.iter().all(|s| s.running.is_none()) {
            return;
        }
        let mut entry = Carried {
            event: event.clone(),
            resume: None,
        };
        self.dispatch(home, &mut entry, true, wall, mono, &Budget::none());
    }

    /// The time passed: the timers that are due fire.
    pub fn tick(&mut self, home: &Home, wall: u64, mono: u64) {
        self.tick_with(home, wall, mono, &Budget::none());
    }

    /// One turn of the radio loop: a failed script that may come back does (one per turn), the
    /// events (those carried from the last turn first) go to the instances, then the timers that
    /// are due fire. Scripts get `SCRIPT_TURN_LIMIT_MS` of the turn, a restart's load and starts
    /// included; the script deliveries left wait for the next turn, while the built-in recipes
    /// always get each event at once (nothing is copied while every event is handled in the turn,
    /// as it is without scripts).
    pub fn turn(&mut self, home: &Home, events: Vec<Event>, wall: u64, mono: u64) {
        let budget = Budget::of(&self.script_clock);
        self.restart_due_scripts(home, wall, mono, &budget);
        for provider in 0..self.natives.len() {
            self.start_waiting(provider, home, wall, mono, &budget);
        }
        while let Some(mut entry) = self.carry.pop_front() {
            if self.slots.iter().all(|s| s.running.is_none()) {
                continue;
            }
            if !self.dispatch(home, &mut entry, false, wall, mono, &budget) {
                self.carry.push_front(entry);
                break;
            }
        }
        for event in events {
            if self.slots.iter().all(|s| s.running.is_none()) {
                continue;
            }
            let mut entry = Carried {
                event,
                resume: None,
            };
            // With the budget spent the built-in recipes still get the event (`dispatch` serves
            // them first); only the scripts' part is carried, behind the ones carried before.
            if self.dispatch(home, &mut entry, true, wall, mono, &budget) {
                continue;
            }
            if self.carry.len() >= MAX_CARRIED_EVENTS {
                self.carry.pop_front();
                log::warn!("recipes: carried events over {MAX_CARRIED_EVENTS}, the oldest dropped");
            }
            self.carry.push_back(entry);
        }
        self.tick_with(home, wall, mono, &budget);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use solite_recipe_sdk::{Host, Pruned};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A recipe that fails on every event, and one that counts the events it gets.
    struct Failing;
    impl Recipe for Failing {
        fn info(&self) -> Value {
            json!({"id": "failing", "name": "Failing", "slots": []})
        }
        fn validate(&self, _: &dyn Host, _: &Value) -> Result<(), RecipeError> {
            Ok(())
        }
        fn start(
            &mut self,
            _: &dyn Host,
            _: u32,
            _: &Value,
            _: Clock,
        ) -> Result<Update, RecipeError> {
            Ok(Update {
                state: "{}".into(),
                listen: Some(Vec::new()),
            })
        }
        fn stop(&mut self, _: u32) {}
        fn on_event(
            &mut self,
            _: &dyn Host,
            _: u32,
            _: &SdkEvent,
            _: Clock,
        ) -> Result<Option<Update>, RecipeError> {
            Err("boom".into())
        }
        fn tick(&mut self, _: &dyn Host, _: u32, _: Clock) -> Result<Option<Update>, RecipeError> {
            Err("boom".into())
        }
        fn refresh(&mut self, _: &dyn Host, _: u32, _: &Value) -> Result<Update, RecipeError> {
            Ok(Update {
                state: "{}".into(),
                listen: Some(Vec::new()),
            })
        }
        fn prune(&self, _: &dyn Host, _: &Value, _: &Change) -> Option<Pruned> {
            None
        }
    }

    struct Counting(Arc<AtomicUsize>);
    impl Recipe for Counting {
        fn info(&self) -> Value {
            json!({"id": "counting", "name": "Counting", "slots": []})
        }
        fn validate(&self, _: &dyn Host, _: &Value) -> Result<(), RecipeError> {
            Ok(())
        }
        fn start(
            &mut self,
            _: &dyn Host,
            _: u32,
            _: &Value,
            _: Clock,
        ) -> Result<Update, RecipeError> {
            Ok(Update {
                state: "{}".into(),
                listen: Some(Vec::new()),
            })
        }
        fn stop(&mut self, _: u32) {}
        fn on_event(
            &mut self,
            _: &dyn Host,
            _: u32,
            _: &SdkEvent,
            _: Clock,
        ) -> Result<Option<Update>, RecipeError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
        fn tick(&mut self, _: &dyn Host, _: u32, _: Clock) -> Result<Option<Update>, RecipeError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
        fn refresh(&mut self, _: &dyn Host, _: u32, _: &Value) -> Result<Update, RecipeError> {
            Ok(Update {
                state: "{}".into(),
                listen: Some(Vec::new()),
            })
        }
        fn prune(&self, _: &dyn Host, _: &Value, _: &Change) -> Option<Pruned> {
            None
        }
    }

    /// The origin of a script recipe given by the program (`--recipe-script`), for the tests.
    #[cfg(feature = "script-recipes")]
    fn test_origin() -> ScriptOrigin {
        ScriptOrigin {
            source: ScriptSource::BuiltIn,
            text: Arc::from(""),
            limit: 0,
            declared: None,
        }
    }

    fn engine_with(recipes: Vec<Box<dyn Recipe + Send>>) -> RecipeEngine {
        let mut engine = RecipeEngine::new();
        engine.natives.extend(recipes.into_iter().map(Native::new));
        engine
    }

    fn slot(n: u8, recipe_id: &str, provider: Option<usize>) -> Slot {
        Slot {
            instance: RecipeInstance {
                id: Id::from_random([n; 16]),
                recipe_id: recipe_id.into(),
                params: "null".into(),
                enabled: true,
                created_at: 0,
            },
            running: provider.map(|provider| Running {
                provider,
                handle: u32::from(n),
                state: NO_STATE.into(),
                listen: Vec::new(),
            }),
            log: VecDeque::new(),
        }
    }

    #[cfg(feature = "script-recipes")]
    /// A recipe that takes `ms` of the script clock for each event and says who it was called for.
    struct Slow {
        id: &'static str,
        clock: Arc<std::sync::atomic::AtomicU64>,
        ms: u64,
        calls: Arc<std::sync::Mutex<Vec<u32>>>,
    }
    #[cfg(feature = "script-recipes")]
    impl Recipe for Slow {
        fn info(&self) -> Value {
            json!({"id": self.id, "name": "Slow", "slots": []})
        }
        fn validate(&self, _: &dyn Host, _: &Value) -> Result<(), RecipeError> {
            Ok(())
        }
        fn start(
            &mut self,
            _: &dyn Host,
            _: u32,
            _: &Value,
            _: Clock,
        ) -> Result<Update, RecipeError> {
            Ok(Update {
                state: "{}".into(),
                listen: Some(Vec::new()),
            })
        }
        fn stop(&mut self, _: u32) {}
        fn on_event(
            &mut self,
            _: &dyn Host,
            handle: u32,
            _: &SdkEvent,
            _: Clock,
        ) -> Result<Option<Update>, RecipeError> {
            self.clock.fetch_add(self.ms, Ordering::SeqCst);
            self.calls.lock().unwrap().push(handle);
            Ok(None)
        }
        fn tick(&mut self, _: &dyn Host, _: u32, _: Clock) -> Result<Option<Update>, RecipeError> {
            Ok(None)
        }
        fn refresh(&mut self, _: &dyn Host, _: u32, _: &Value) -> Result<Update, RecipeError> {
            Ok(Update {
                state: "{}".into(),
                listen: Some(Vec::new()),
            })
        }
        fn prune(&self, _: &dyn Host, _: &Value, _: &Change) -> Option<Pruned> {
            None
        }
    }

    #[cfg(feature = "script-recipes")]
    type Calls = Arc<std::sync::Mutex<Vec<u32>>>;

    #[cfg(feature = "script-recipes")]
    /// An engine with a counting native recipe (provider 0) and a slow script recipe (provider
    /// 1, 400 ms per event, over the 300 ms of a turn), `scripts` instances of the script, all
    /// listening to the root zone, and one of the counting recipe.
    fn slow_world(scripts: u8) -> (RecipeEngine, Arc<AtomicUsize>, Calls) {
        let clock = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let calls: Calls = Arc::default();
        let count = Arc::new(AtomicUsize::new(0));
        let mut engine = engine_with(vec![Box::new(Counting(count.clone()))]);
        let read = clock.clone();
        let script_clock: ScriptClock = Arc::new(move || read.load(Ordering::SeqCst));
        let recipe = Slow {
            id: "slow",
            clock,
            ms: 400,
            calls: calls.clone(),
        };
        engine
            .add_script(
                Box::new(recipe),
                Box::new(|| Err("no".to_string())),
                script_clock,
                test_origin(),
            )
            .unwrap();
        engine.slots.push(slot(1, "counting", Some(0)));
        for n in 0..scripts {
            engine.slots.push(slot(10 + n, "slow", Some(1)));
        }
        let listen = engine.listens.indexes(&[ListenEntry::Zone {
            zone: Id::ROOT_ZONE.to_string(),
        }]);
        for slot in &mut engine.slots {
            slot.running.as_mut().unwrap().listen = listen.clone();
        }
        (engine, count, calls)
    }

    #[cfg(feature = "script-recipes")]
    fn root_motion(motion: bool) -> Event {
        Event::Zone {
            zone: Id::ROOT_ZONE,
            motion,
            luminosity: None,
        }
    }

    #[cfg(feature = "script-recipes")]
    #[test]
    fn built_in_recipes_get_every_event_at_once_even_when_the_scripts_spent_the_turn() {
        let home = Home::new(1);
        let (mut engine, count, calls) = slow_world(2);
        let events = vec![root_motion(true), root_motion(false), root_motion(true)];
        engine.turn(&home, events, 1, 1);
        // The first script instance took the budget: one script call in the turn, the rest carried
        // (three events, none of them lost) ...
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(engine.carry.len(), 3);
        // ... but the built-in recipe had all three events in the turn (and its timer's tick).
        assert_eq!(count.load(Ordering::SeqCst), 3 + 1);
    }

    #[cfg(feature = "script-recipes")]
    #[test]
    fn past_the_carried_bound_the_oldest_script_deliveries_are_dropped_the_built_ins_lose_none() {
        let home = Home::new(1);
        let (mut engine, count, _) = slow_world(2);
        let sent = MAX_CARRIED_EVENTS + 5;
        let events: Vec<Event> = (0..sent).map(|n| root_motion(n % 2 == 0)).collect();
        engine.turn(&home, events, 1, 1);
        assert_eq!(engine.carry.len(), MAX_CARRIED_EVENTS);
        assert_eq!(count.load(Ordering::SeqCst), sent + 1); // and the turn's tick
    }

    #[cfg(feature = "script-recipes")]
    #[test]
    fn a_carried_event_goes_on_with_the_next_instance_when_the_one_it_waited_for_was_deleted() {
        let home = Home::new(1);
        let (mut engine, _, calls) = slow_world(3);
        engine.turn(&home, vec![root_motion(true)], 1, 1);
        // The first script instance (handle 10) took the budget; the event waits for the second.
        assert_eq!(*calls.lock().unwrap(), vec![10]);
        let waiting = engine.slots[2].instance.id;
        assert_eq!(engine.carry[0].resume.as_ref().map(|r| r.0), Some(waiting));
        engine.delete(&waiting.to_string()).unwrap();
        engine.turn(&home, Vec::new(), 2, 2);
        // The third (handle 12) is the one the event goes to now: it is not lost.
        assert_eq!(*calls.lock().unwrap(), vec![10, 12]);
    }

    #[cfg(feature = "script-recipes")]
    #[test]
    fn at_most_one_failed_script_comes_back_in_a_turn() {
        let home = Home::new(1);
        let clock: ScriptClock = Arc::new(|| 0);
        let mut engine = RecipeEngine::new();
        for id in ["first", "second"] {
            let recipe = Slow {
                id,
                clock: Arc::default(),
                ms: 0,
                calls: Arc::default(),
            };
            engine
                .add_script(
                    Box::new(recipe),
                    Box::new(move || {
                        Ok(Box::new(Slow {
                            id,
                            clock: Arc::default(),
                            ms: 0,
                            calls: Arc::default(),
                        }) as Box<dyn Recipe + Send>)
                    }),
                    clock.clone(),
                    test_origin(),
                )
                .unwrap();
        }
        for native in engine.natives.iter_mut().filter(|n| n.is_script()) {
            native.recipe = None;
            native.script.as_mut().unwrap().failed = Some(Failed {
                since: 0,
                reason: "test".into(),
            });
        }
        let failed = |engine: &RecipeEngine| {
            engine
                .natives
                .iter()
                .filter(|n| n.failure().is_some())
                .count()
        };
        engine.turn(&home, Vec::new(), 60_000, 60_000);
        assert_eq!(failed(&engine), 1);
        engine.turn(&home, Vec::new(), 60_001, 60_001);
        assert_eq!(failed(&engine), 0);
    }

    #[test]
    fn an_instance_whose_handler_fails_does_not_stop_the_others() {
        let home = Home::new(1);
        let count = Arc::new(AtomicUsize::new(0));
        let mut engine = engine_with(vec![Box::new(Failing), Box::new(Counting(count.clone()))]);
        engine.slots.push(slot(1, "failing", Some(0)));
        engine.slots.push(slot(2, "counting", Some(1)));
        let event = Event::Zone {
            zone: Id::ROOT_ZONE,
            motion: true,
            luminosity: None,
        };
        engine.handle(&home, &event, 1, 1);
        engine.tick(&home, 2, 2);
        assert_eq!(count.load(Ordering::SeqCst), 2);
        // The failure is in the failing instance's log, not in the other's.
        let failing = engine.slots[0].instance.id.to_string();
        let healthy = engine.slots[1].instance.id.to_string();
        assert_eq!(engine.log(&failing, 50).len(), 2);
        assert_eq!(engine.log(&failing, 50)[0].level, Level::Error);
        assert_eq!(engine.log(&failing, 50)[0].message.as_ref(), "Error: boom");
        assert!(engine.log(&healthy, 50).is_empty());
    }

    #[test]
    fn a_start_that_fails_tells_an_error_event_only_when_asked_to_and_a_restore_never_does() {
        let home = Home::new(1);
        let hub = crate::live::LiveHub::handle();
        let client = hub.connect().unwrap();
        assert!(hub.subscribe(client, r#"{"type":"subscribe","topics":["recipes"]}"#));
        let mut engine = RecipeEngine::new();
        engine.set_live(hub.clone());
        // A recipe nobody provides: the start fails whatever the way it is asked for.
        engine.slots.push(slot(1, "motion-light", None));
        // The start right after a create: log line and event.
        assert!(
            engine
                .start_slot(0, &home, 1, 1, "Failed to start", true)
                .is_err()
        );
        let batch: Vec<Value> = serde_json::from_str(&hub.take_batch(client).unwrap()).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0]["type"], "recipe.instance.error");
        assert_eq!(batch[0]["recipeId"], "motion-light");
        assert!(batch[0]["error"].as_str().unwrap().len() > 3);
        // A restore (or a restart, or an enable) that fails: the log line only.
        for failed in ["Failed to restore", "Failed to restart"] {
            assert!(engine.start_slot(0, &home, 1, 1, failed, false).is_err());
        }
        assert_eq!(hub.take_batch(client), None);
        let id = engine.slots[0].instance.id.to_string();
        assert_eq!(engine.log(&id, 50).len(), 3);
    }

    /// `levels` nested lists inside the params object: the object is the first level.
    fn nested(levels: usize) -> Value {
        let mut value = json!([]);
        for _ in 0..levels.saturating_sub(2) {
            value = json!([value]);
        }
        json!({ "deep": value })
    }

    #[test]
    fn params_nested_eight_levels_deep_pass_and_nine_or_a_hundred_are_refused() {
        // Counting levels: the params object is the first one.
        for (levels, ok) in [(1, true), (7, true), (8, true), (9, false), (100, false)] {
            let value = if levels == 1 {
                json!({})
            } else {
                nested(levels)
            };
            assert_eq!(
                depth(&value, MAX_PARAMS_DEPTH),
                levels.min(MAX_PARAMS_DEPTH + 1)
            );
            assert_eq!(check_depth(&value).is_ok(), ok, "{levels} levels");
        }
        let refused = check_depth(&nested(9)).unwrap_err();
        assert_eq!(refused.status, 400);
        assert_eq!(refused.message, "Invalid params: params nested too deeply");
        // Wide is not deep: 200 members in one list.
        assert!(check_depth(&json!({"list": (0..200).collect::<Vec<_>>()})).is_ok());
    }
}
