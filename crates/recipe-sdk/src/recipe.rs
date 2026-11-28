//! The `Recipe` trait: what a recipe implements (spec 041, R1.1). One value serves every
//! instance of the recipe; an instance is a handle the engine gives at `start`. The calls that
//! change an instance return what the engine keeps of it: the state text and the listen set, or
//! no listen set when it is unchanged (R2.5), so that readers never call the recipe.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Level;
use crate::bounds::{MAX_LISTEN_DEVICES, MAX_LISTEN_EQUIPMENTS, MAX_LISTEN_ZONES, MAX_STATE_BYTES};
use crate::event::{Event, ListenEntry};
use crate::host::Host;

/// The two clocks of a call: the monotonic one (`mono`, for deadlines) and the wall one (`wall`,
/// milliseconds since 1970, for the dates written).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub mono: u64,
    pub wall: u64,
}

/// What an instance is after a call: its state text (a JSON object, at most `MAX_STATE_BYTES`,
/// served as written) and what it listens to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub state: String,
    /// The listen set, or `None` when it is the one the last update gave (a recipe need not keep
    /// a copy of it to answer a call that changed only its state). A start always gives one.
    pub listen: Option<Vec<ListenEntry>>,
}

impl Update {
    /// The bounds of the SDK: the state's size and the listen set's counts. The reason is the
    /// failure's.
    pub fn check(&self) -> Result<(), String> {
        if self.state.len() > MAX_STATE_BYTES {
            return Err(format!(
                "state of {} bytes, over {MAX_STATE_BYTES}",
                self.state.len()
            ));
        }
        let listen = self.listen.as_deref().unwrap_or(&[]);
        let count = |f: fn(&ListenEntry) -> bool| listen.iter().filter(|e| f(e)).count();
        let zones = count(|e| matches!(e, ListenEntry::Zone { .. }));
        let equipments = count(|e| matches!(e, ListenEntry::Equipment { .. }));
        let devices = count(|e| matches!(e, ListenEntry::Device { .. }));
        if zones > MAX_LISTEN_ZONES
            || equipments > MAX_LISTEN_EQUIPMENTS
            || devices > MAX_LISTEN_DEVICES
        {
            return Err(format!(
                "listen set of {zones} zones, {equipments} equipments, {devices} devices, over {MAX_LISTEN_ZONES}, {MAX_LISTEN_EQUIPMENTS}, {MAX_LISTEN_DEVICES}"
            ));
        }
        Ok(())
    }
}

/// A line to log: Sowel's `RecipeLogEntry` level and message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub level: Level,
    pub message: String,
}

/// An equipment that was deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedEquipment {
    pub id: String,
    pub name: String,
}

/// What changed in the home, as `prune` is told. The engine already took the deleted equipment's
/// id out of every `equipment` slot of the params (`pruned` names those slots) and logged it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub equipment: Option<ChangedEquipment>,
    pub pruned: Vec<String>,
}

/// What `prune` returns when the recipe changed its params: the new params, the lines to log
/// (after the engine's own), and whether a running instance restarts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pruned {
    pub params: Value,
    pub lines: Vec<LogLine>,
    pub restart: bool,
}

/// How a call failed (spec 041, R3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// This instance only: the engine logs it and the instance goes on (or stays stopped, for a
    /// start).
    Instance,
    /// The recipe as a whole can no longer run (a script that overran its time or memory, or
    /// reached for what is not its own): the engine stops every instance of it.
    Recipe,
}

/// The error of a call: its kind and its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeError {
    pub kind: ErrorKind,
    pub message: String,
}

impl RecipeError {
    /// An error of the instance.
    pub fn instance(message: impl Into<String>) -> RecipeError {
        RecipeError {
            kind: ErrorKind::Instance,
            message: message.into(),
        }
    }

    /// A failure of the recipe.
    pub fn recipe(message: impl Into<String>) -> RecipeError {
        RecipeError {
            kind: ErrorKind::Recipe,
            message: message.into(),
        }
    }
}

/// A message alone is an error of the instance.
impl From<String> for RecipeError {
    fn from(message: String) -> RecipeError {
        RecipeError::instance(message)
    }
}

impl From<&str> for RecipeError {
    fn from(message: &str) -> RecipeError {
        RecipeError::instance(message)
    }
}

impl std::fmt::Display for RecipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// What a recipe that runs foreign code counts about its calls (spec 041 R6.3), for the box's
/// `scripts:` line. Totals since the recipe was loaded; `held` and `peak` are the bytes its runtime
/// holds now and the most it held at the end of a call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunStats {
    pub instances: usize,
    pub calls: u64,
    pub total_us: u64,
    pub max_us: u64,
    pub held: usize,
    pub peak: usize,
}

/// When an instance next needs a `tick`, so that the engine calls a recipe for its timers only
/// when one is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deadline {
    /// On every turn of the engine (the built-in recipes check their own timers).
    EveryTurn,
    /// No timer.
    Never,
    /// When the monotonic clock reaches `mono`, or the wall clock `wall` (milliseconds since 1970;
    /// a timer set for a time of day, which waits while the clock is not set).
    At {
        mono: Option<u64>,
        wall: Option<u64>,
    },
}

impl Deadline {
    /// True when a tick is due at these clocks.
    pub fn is_due(self, clock: Clock) -> bool {
        match self {
            Deadline::EveryTurn => true,
            Deadline::Never => false,
            Deadline::At { mono, wall } => {
                mono.is_some_and(|m| clock.mono >= m) || wall.is_some_and(|w| clock.wall >= w)
            }
        }
    }
}

/// A recipe.
pub trait Recipe {
    /// Sowel's `RecipeInfo` of the recipe.
    fn info(&self) -> Value;
    /// The same as JSON text. A recipe that holds it as text gives it without building the tree
    /// (a big one for a script recipe, in small blocks).
    fn info_text(&self) -> String {
        self.info().to_string()
    }
    /// Checks params against the home, with the recipe's messages (the engine wraps them as
    /// `Invalid params: <message>`).
    fn validate(&self, host: &dyn Host, params: &Value) -> Result<(), RecipeError>;
    /// Starts the instance `handle`. A refusal is the message.
    fn start(
        &mut self,
        host: &dyn Host,
        handle: u32,
        params: &Value,
        clock: Clock,
    ) -> Result<Update, RecipeError>;
    /// The instance is gone.
    fn stop(&mut self, handle: u32);
    /// An event the instance listens to. `Some` when its state changed.
    fn on_event(
        &mut self,
        host: &dyn Host,
        handle: u32,
        event: &Event,
        clock: Clock,
    ) -> Result<Option<Update>, RecipeError>;
    /// The time passed: timers that are due fire. `Some` when the state changed.
    fn tick(
        &mut self,
        host: &dyn Host,
        handle: u32,
        clock: Clock,
    ) -> Result<Option<Update>, RecipeError>;
    /// The counters of a recipe that runs foreign code (a script recipe); `None` for the others.
    fn run_stats(&self) -> Option<RunStats> {
        None
    }
    /// When the instance next needs a `tick`. Asked on every turn, so it must be cheap (no call
    /// into a script). Default: every turn.
    fn next_deadline(&self, _host: &dyn Host, _handle: u32) -> Deadline {
        Deadline::EveryTurn
    }
    /// The home changed and the instance is kept: takes again what it resolved from the home,
    /// without touching its timers or state. No order, no log.
    fn refresh(
        &mut self,
        host: &dyn Host,
        handle: u32,
        params: &Value,
    ) -> Result<Update, RecipeError>;
    /// The recipe's own part of a settle: `None` when the params stay as they are.
    fn prune(&self, host: &dyn Host, params: &Value, change: &Change) -> Option<Pruned>;
}
