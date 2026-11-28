//! `ScriptRecipe`: a script recipe as one more implementation of the SDK's `Recipe` trait (spec
//! 041). It owns the recipe's QuickJS runtime and context, calls the shim's entry points for each
//! instance (start, event, tick, refresh, stop), and serves the functions the shim reaches the
//! engine through (`get`, `order`, `zone`, `sun`, `dur`, `atNext`), by instance handle and slot
//! name: the slots' ids are held here, in the instance table (the allow-list), never in the
//! script.

use std::sync::{Arc, Mutex, PoisonError};

use rquickjs::{Array, Context, Ctx, Exception, Function, IntoJs, Object, Runtime, Value};
use scoped_tls_hkt::scoped_thread_local;
use serde::Deserialize;
use serde_json::json;
use solite_recipe_sdk::bounds::{MAX_LISTEN_DEVICES, MAX_LISTEN_EQUIPMENTS, MAX_STATE_BYTES};
use solite_recipe_sdk::duration::parse_duration;
use solite_recipe_sdk::{
    Change, Clock, Deadline, Event, EventValue, Host, Level, ListenEntry, OrderStatus, Pruned,
    Recipe, RecipeError, RunStats, Update,
};

use super::limits::{
    self, Gate, HeldCounter, MAX_ALIAS_BYTES, MAX_END_LINE_BYTES, MAX_LOG_LINES_PER_CALL,
    MAX_TIMERS, RefusedFlag, ScriptRuntimes,
};
use super::slots::{self, SlotDef, SlotIds, SlotKind};
use crate::home::CLOCK_SET_AFTER_MS;
use crate::recipes::ScriptClock;

// What the host functions reach while a call runs: the host of the call, the instance table (the
// allow-list), the gate. Set around each call into the context, on the calling thread.
struct CallCx<'a> {
    host: &'a dyn Host,
    instances: &'a [Instance],
    gate: &'a Gate,
    /// The equipments and zones `validate` may read the facts of: the ones the params it is given
    /// name, as the host resolved them. Empty outside `validate`.
    named: &'a [String],
}

scoped_thread_local!(static CALL: for<'a> &'a CallCx<'a>);

/// A motion sensor of an instance's zone: the (device, key) whose reports it listens to, and the
/// equipment that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sensor {
    device: String,
    key: String,
    equipment: String,
}

/// What the engine keeps of one instance of a script recipe, besides what lives in the context.
#[derive(Debug)]
struct Instance {
    handle: u32,
    /// The zone and equipment slots, with the ids the script may reach.
    slots: Vec<SlotIds>,
    /// The state text the last call left (the engine's copy is served). The listen set is not
    /// kept here: the engine holds it, and a call that leaves it as it was says nothing of it.
    state: String,
    /// The motion sensors, resolved when the script listens to motion reports.
    sensors: Vec<Sensor>,
    /// The timers, as the last call left them: the next monotonic due time, and the times of day
    /// (hour, minute, the wall time they last fired; none while the clock was not set).
    next_mono: Option<u64>,
    at_timers: Vec<(u32, u32, Option<u64>)>,
}

impl Instance {
    fn slot(&self, name: &str) -> Option<&SlotIds> {
        self.slots.iter().find(|s| s.name == name)
    }

    fn zone(&self) -> Option<&str> {
        self.slot("zone")
            .and_then(|s| s.ids.first())
            .map(String::as_str)
    }
}

/// The shim's answer at the end of a call.
#[derive(Debug, Deserialize)]
struct End {
    /// The log lines: level (0 info, 1 warn, 2 error) and text.
    g: Option<Vec<(u8, String)>>,
    /// The state text, when it changed.
    s: Option<String>,
    /// The listen set, when it changed.
    l: Option<ListenJson>,
    d: DeadlineJson,
    /// A failure of the recipe the pending jobs ran into (a stack overflow, no memory).
    f: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListenJson {
    e: Vec<(String, String)>,
    z: bool,
    m: bool,
}

#[derive(Debug, Deserialize)]
struct DeadlineJson {
    m: Option<f64>,
    a: Vec<(u32, u32, Option<f64>)>,
}

/// The largest instant (milliseconds) a timer of a script may name: 2^53, what a JavaScript number
/// holds exactly. Over it, or negative, or not a number, the end of the call is refused.
const MAX_INSTANT_MS: f64 = 9_007_199_254_740_992.0;

fn sane_instant(ms: f64) -> bool {
    ms.is_finite() && (0.0..=MAX_INSTANT_MS).contains(&ms)
}

/// What the engine accepts of the end of a call, whatever the script did to the objects the shim
/// builds it from: the state is a JSON object of at most `MAX_STATE_BYTES`, at most `MAX_TIMERS`
/// timers (times of day within a day, instants that a clock can read), at most
/// `MAX_LOG_LINES_PER_CALL` lines and the one that says some were dropped, aliases of at most
/// `MAX_ALIAS_BYTES`. The reason when it is not.
fn validate_end(end: &End) -> Result<(), String> {
    let bad = |what: &str| Err(format!("bad end of call: {what}"));
    if end.f.as_ref().is_some_and(|f| f.len() > 100) {
        return bad("the failure is too long");
    }
    if let Some(lines) = &end.g {
        if lines.len() > MAX_LOG_LINES_PER_CALL + 1 {
            return bad("too many log lines");
        }
        if lines
            .iter()
            .any(|(_, text)| text.len() > MAX_END_LINE_BYTES)
        {
            return bad("a log line is too long");
        }
    }
    if let Some(state) = &end.s {
        if state.len() > MAX_STATE_BYTES {
            return bad(&format!("the state is over {MAX_STATE_BYTES} bytes"));
        }
        let object =
            state.starts_with('{') && serde_json::from_str::<serde::de::IgnoredAny>(state).is_ok();
        if !object {
            return bad("the state is not a JSON object");
        }
    }
    if let Some(listen) = &end.l {
        if listen.e.len() > MAX_LISTEN_EQUIPMENTS {
            return bad("too many listened equipments");
        }
        if listen
            .e
            .iter()
            .any(|(_, alias)| alias.len() > MAX_ALIAS_BYTES)
        {
            return bad("an alias is too long");
        }
    }
    if end.d.m.is_some_and(|m| !sane_instant(m)) {
        return bad("a timer's time is out of range");
    }
    if end.d.a.len() > MAX_TIMERS {
        return bad("too many timers");
    }
    if end
        .d
        .a
        .iter()
        .any(|(h, m, last)| *h >= 24 || *m >= 60 || last.is_some_and(|l| !sane_instant(l)))
    {
        return bad("a time of day is out of range");
    }
    Ok(())
}

/// An argument of a call into the shim.
enum Arg {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
}

impl<'js> IntoJs<'js> for Arg {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            Arg::Null => Ok(Value::new_null(ctx.clone())),
            Arg::Bool(b) => Ok(Value::new_bool(ctx.clone(), b)),
            Arg::Number(n) => Ok(Value::new_float(ctx.clone(), n)),
            Arg::Text(text) => text.into_js(ctx),
        }
    }
}

fn arg_of(value: &EventValue) -> Arg {
    match value {
        EventValue::Null => Arg::Null,
        EventValue::Bool(b) => Arg::Bool(*b),
        EventValue::Number(n) => Arg::Number(*n),
        EventValue::Text(text) => Arg::Text(text.clone()),
    }
}

/// What a call into the shim does.
enum Op<'a> {
    Start { params: &'a str, slots: &'a str },
    Event { kind: u8, a: Arg, b: Arg, c: Arg },
    Tick,
    Refresh { params: &'a str, slots: &'a str },
}

/// The global the shim's entry points are kept under (see `shim.js`).
pub(super) const SHIM_GLOBAL: &str = "__solite_shim";

/// What the loader hands `ScriptRecipe::assemble`.
pub(super) struct Parts {
    pub ctx: Context,
    pub rt: Runtime,
    pub gate: Arc<Gate>,
    pub clock: ScriptClock,
    pub limit: usize,
    pub refused: Option<RefusedFlag>,
    pub held: Option<HeldCounter>,
    pub info: String,
    pub defs: Vec<SlotDef>,
    pub has_validate: bool,
}

/// A script recipe: the runtime, the declared slots and the instances.
pub struct ScriptRecipe {
    // Field order is drop order: the context goes before the runtime.
    ctx: Context,
    rt: Runtime,
    gate: Arc<Gate>,
    clock: ScriptClock,
    /// The memory limit this recipe's runtime was given.
    limit: usize,
    /// The runtime's own refusal flag (the box's allocator sets it; the system's has none).
    refused: Option<RefusedFlag>,
    /// The allocator's count of what the runtime holds (the box's), read after each call.
    held: Option<HeldCounter>,
    /// The `RecipeInfo`, as JSON text: one block, where a `Value` tree is hundreds of small ones
    /// (on the box, every block of 4 KB or less goes to the internal RAM). Parsed when asked for:
    /// the API's recipe list, a settle, the engine's registry.
    info: String,
    defs: Vec<SlotDef>,
    has_validate: bool,
    instances: Vec<Instance>,
    /// Calls, time and memory counted for the box's `scripts:` line.
    counters: Mutex<Counters>,
}

/// What `run_stats` reports: totals since the recipe was loaded.
#[derive(Debug, Default, Clone, Copy)]
struct Counters {
    calls: u64,
    total_us: u64,
    max_us: u64,
    held: usize,
    peak: usize,
    /// What the runtime held after its last collection (the collector's rule reads it).
    after_gc: usize,
}

impl std::fmt::Debug for ScriptRecipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptRecipe")
            .field("instances", &self.instances.len())
            .finish()
    }
}

/// The text of a JavaScript exception, for a log line.
pub(super) fn describe(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if matches!(error, rquickjs::Error::Exception) {
        let thrown = ctx.catch();
        // QuickJS could not even build the exception: it ran out of memory.
        if thrown.type_of() == rquickjs::Type::Uninitialized {
            return "out of memory".to_string();
        }
        if let Some(exception) = thrown.as_exception()
            && let Some(message) = exception.message()
        {
            return message;
        }
        if let Some(text) = thrown.as_string().and_then(|s| s.to_string().ok()) {
            return text;
        }
        return format!("an exception ({thrown:?})");
    }
    error.to_string()
}

impl ScriptRecipe {
    /// Assembled by the loader.
    pub(super) fn assemble(parts: Parts) -> ScriptRecipe {
        let loaded = limits::held(&parts.rt, &parts.held);
        ScriptRecipe {
            counters: Mutex::new(Counters {
                held: loaded,
                peak: loaded,
                after_gc: loaded,
                ..Counters::default()
            }),
            ctx: parts.ctx,
            rt: parts.rt,
            gate: parts.gate,
            clock: parts.clock,
            limit: parts.limit,
            refused: parts.refused,
            held: parts.held,
            info: parts.info,
            defs: parts.defs,
            has_validate: parts.has_validate,
            instances: Vec::new(),
        }
    }

    /// Loads a script: the recipe, and what the host prints of it. The error is the reason, with
    /// no `Script recipe invalid: ` in front (the engine's `load_script` adds it).
    pub fn load(
        source: &str,
        runtimes: &Arc<dyn ScriptRuntimes>,
        clock: &ScriptClock,
    ) -> Result<(ScriptRecipe, super::ScriptSummary), String> {
        super::load::load(source, runtimes, clock, super::load::Rules::BUILT_IN)
    }

    /// The same under the rules of an installed script (spec 042 R1.3, R1.4): an id of 1 to 32
    /// characters and a `version` that is `x.y.z`.
    pub fn load_installed(
        source: &str,
        runtimes: &Arc<dyn ScriptRuntimes>,
        clock: &ScriptClock,
    ) -> Result<(ScriptRecipe, super::ScriptSummary), String> {
        super::load::load(source, runtimes, clock, super::load::Rules::INSTALLED)
    }

    /// The bytes QuickJS holds for this recipe now.
    pub fn held_bytes(&self) -> usize {
        limits::held(&self.rt, &self.held)
    }

    /// QuickJS's memory accounting for this recipe (its own, for the measures).
    pub fn memory_usage(&self) -> rquickjs::qjs::JSMemoryUsage {
        self.rt.memory_usage()
    }

    /// Instances running in this recipe.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Calls a host function of the engine with `handle`, as a script that forged one would: the
    /// call is refused and fails the recipe (the test of the allow-list). Not part of the engine.
    #[doc(hidden)]
    pub fn call_host_with_handle(&self, host: &dyn Host, handle: u32) -> Result<(), RecipeError> {
        limits::before_call(&self.rt, &self.gate, &self.clock, self.limit, &self.refused);
        let cx = CallCx {
            host,
            instances: &self.instances,
            gate: &self.gate,
            named: &[],
        };
        let refused = CALL.set(&cx, || CALL.with(|cx| cx.instance(handle).is_err()));
        self.after_call_check()?;
        if refused {
            return Err(RecipeError::instance("refused"));
        }
        Ok(())
    }

    /// The failure of the recipe the call left, if it did: stopped from outside, an allocation
    /// refused, or the runtime holding nearly all its memory after the call.
    fn after_call_check(&self) -> Result<(), RecipeError> {
        if let Some(reason) = self.gate.take() {
            return Err(RecipeError::recipe(reason.to_string()));
        }
        let refused = limits::was_refused(&self.refused);
        let after = {
            let mut counters = self.counters.lock().unwrap_or_else(PoisonError::into_inner);
            let after =
                limits::after_call(&self.rt, &self.held, self.limit, &mut counters.after_gc);
            counters.held = after.now;
            counters.peak = counters.peak.max(after.at_end);
            after
        };
        if refused || after.exhausted {
            return Err(RecipeError::recipe(format!(
                "out of memory (limit {} KB)",
                self.limit / 1024
            )));
        }
        Ok(())
    }

    /// One call into an instance: the shim's entry point, the pending jobs, the end of the call.
    /// The lines the script logged go to the host's log; an error of the script is the instance's
    /// (`Script error: ...`), a failure of the recipe is the error's kind.
    fn call(
        &self,
        host: &dyn Host,
        handle: u32,
        op: Op<'_>,
        clock: Clock,
    ) -> Result<End, RecipeError> {
        limits::before_call(&self.rt, &self.gate, &self.clock, self.limit, &self.refused);
        let began = std::time::Instant::now();
        let (mono, wall) = (clock.mono as f64, clock.wall as f64);
        let cx = CallCx {
            host,
            instances: &self.instances,
            gate: &self.gate,
            named: &[],
        };
        let raw = CALL.set(&cx, || {
            self.ctx.with(|ctx| {
                let run = || -> rquickjs::Result<(String, String)> {
                    let entry: Object = ctx.globals().get(SHIM_GLOBAL)?;
                    let status: String = match op {
                        Op::Start { params, slots } => entry
                            .get::<_, Function>("start")?
                            .call((handle, params, slots, mono, wall))?,
                        Op::Event { kind, a, b, c } => entry
                            .get::<_, Function>("event")?
                            .call((handle, kind, mono, wall, a, b, c))?,
                        Op::Tick => entry
                            .get::<_, Function>("tick")?
                            .call((handle, mono, wall))?,
                        Op::Refresh { params, slots } => entry
                            .get::<_, Function>("refresh")?
                            .call((handle, params, slots))?,
                    };
                    // The promises' jobs run after the call, within its limits.
                    while ctx.execute_pending_job() {}
                    if self.gate.is_killed() {
                        return Ok((status, String::new()));
                    }
                    let end: String = entry.get::<_, Function>("end")?.call((handle,))?;
                    Ok((status, end))
                };
                run().map_err(|e| describe(&ctx, e))
            })
        });
        // The check after the call (the memory read, the collector) is part of the call's time:
        // the engine waits for it as for the script.
        let checked = self.after_call_check();
        {
            let us = u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX);
            let mut counters = self.counters.lock().unwrap_or_else(PoisonError::into_inner);
            counters.calls += 1;
            counters.total_us = counters.total_us.saturating_add(us);
            counters.max_us = counters.max_us.max(us);
        }
        checked?;
        let (status, end) = raw.map_err(|m| classify(m, self.limit))?;
        // The end of the call is the shim's text, but the script runs in the same context: the
        // engine checks it as input, and a recipe that corrupts it fails.
        let end: End = serde_json::from_str(&end)
            .map_err(|e| RecipeError::recipe(format!("bad end of call: {e}")))?;
        validate_end(&end).map_err(RecipeError::recipe)?;
        for (level, text) in end.g.iter().flatten() {
            let level = match level {
                0 => Level::Info,
                1 => Level::Warn,
                _ => Level::Error,
            };
            host.log(level, text);
        }
        if status.is_empty()
            && let Some(reason) = &end.f
        {
            return Err(RecipeError::recipe(reason.clone()));
        }
        match status.split_once(':') {
            None => Ok(end),
            Some(("f", reason)) => Err(RecipeError::recipe(reason)),
            Some(("e", message)) => Err(RecipeError::instance(format!("Script error: {message}"))),
            Some(_) => Err(RecipeError::instance("Script error: bad answer")),
        }
    }

    /// Keeps what a call left of an instance and says what the engine is to keep: `Some` when the
    /// state or the listen set changed (or `always`).
    fn apply(&mut self, host: &dyn Host, handle: u32, end: End, always: bool) -> Option<Update> {
        let instance = self.instances.iter_mut().find(|i| i.handle == handle)?;
        instance.next_mono = end.d.m.map(|m| m as u64);
        instance.at_timers = end
            .d
            .a
            .iter()
            .map(|(h, m, last)| (*h, *m, last.map(|l| l as u64)))
            .collect();
        let mut changed = always;
        if let Some(state) = end.s {
            instance.state = state;
            changed = true;
        }
        let mut listened = None;
        if let Some(listen) = end.l {
            instance.sensors = if listen.m {
                instance
                    .zone()
                    .map(|zone| resolve_sensors(host, zone))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let mut entries = Vec::new();
            if listen.z
                && let Some(zone) = instance.zone()
            {
                entries.push(ListenEntry::Zone {
                    zone: zone.to_string(),
                });
            }
            // Only equipments of the instance's own slots, whatever the script's side says.
            let own: Vec<&String> = instance
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::Equipment)
                .flat_map(|s| &s.ids)
                .collect();
            entries.extend(
                listen
                    .e
                    .into_iter()
                    .filter(|(equipment, _)| own.contains(&equipment))
                    .take(MAX_LISTEN_EQUIPMENTS)
                    .map(|(equipment, alias)| ListenEntry::Equipment { equipment, alias }),
            );
            for sensor in &instance.sensors {
                let entry = ListenEntry::Device {
                    device: sensor.device.clone(),
                    key: sensor.key.clone(),
                };
                if !entries.contains(&entry) {
                    entries.push(entry);
                }
            }
            listened = Some(entries);
            changed = true;
        }
        changed.then(|| Update {
            state: instance.state.clone(),
            listen: listened,
        })
    }

    /// The instance is forgotten, here and in the context.
    fn drop_instance(&mut self, handle: u32, in_context: bool) {
        self.instances.retain(|i| i.handle != handle);
        if !in_context {
            return;
        }
        limits::before_call(&self.rt, &self.gate, &self.clock, self.limit, &self.refused);
        self.ctx.with(|ctx| {
            let stopped = ctx
                .globals()
                .get::<_, Object>(SHIM_GLOBAL)
                .and_then(|entry| entry.get::<_, Function>("stop")?.call::<_, ()>((handle,)));
            if let Err(error) = stopped {
                log::warn!("script recipe: stop failed: {}", describe(&ctx, error));
            }
        });
        // A stop that ran out of time is not the recipe's failure: nothing runs any more.
        self.gate.take();
    }

    /// The script's own `validate` first, then the host's checks of the params (spec 046, R3): a
    /// refusal the script makes carries its text. A `validate` that throws or runs out of its time
    /// on settings of the wrong shape gives way to the host's check, whose text is then the answer;
    /// the settings that check lets through get the script's failure. A script can never make a
    /// setting pass that the slot definitions refuse.
    fn check(&self, host: &dyn Host, params: &serde_json::Value) -> Result<(), RecipeError> {
        let failure = if self.has_validate {
            match self.run_validate(host, params) {
                Ok(None) => None,
                Ok(Some(text)) => return Err(RecipeError::instance(text)),
                Err(error) => Some(error),
            }
        } else {
            None
        };
        slots::validate_params(&self.defs, host, params).map_err(RecipeError::instance)?;
        failure.map_or(Ok(()), Err)
    }

    /// The script's `validate`: `Ok(None)` passes, `Ok(Some(text))` is its refusal, `Err` is a
    /// failure of the script (an exception) or of the recipe (a limit).
    fn run_validate(
        &self,
        host: &dyn Host,
        params: &serde_json::Value,
    ) -> Result<Option<String>, RecipeError> {
        let (parsed, slots) = slots::resolve(&self.defs, params);
        let named: Vec<String> = slots
            .iter()
            .filter(|s| matches!(s.kind, SlotKind::Equipment | SlotKind::Zone))
            .flat_map(|s| s.ids.iter().cloned())
            .collect();
        limits::before_call(&self.rt, &self.gate, &self.clock, self.limit, &self.refused);
        let cx = CallCx {
            host,
            instances: &self.instances,
            gate: &self.gate,
            named: &named,
        };
        let text = parsed.to_string();
        let mono = (self.clock)();
        let raw = CALL.set(&cx, || {
            self.ctx.with(|ctx| {
                let run = || -> rquickjs::Result<String> {
                    let entry: Object = ctx.globals().get(SHIM_GLOBAL)?;
                    // `validate` has no clock of the home: the wall time reads 0 (not set).
                    let status: String = entry.get::<_, Function>("validate")?.call((
                        text.as_str(),
                        mono as f64,
                        0.0f64,
                    ))?;
                    while ctx.execute_pending_job() {}
                    Ok(status)
                };
                run().map_err(|e| describe(&ctx, e))
            })
        });
        self.after_call_check()?;
        let status = raw.map_err(|m| classify(m, self.limit))?;
        match status.split_once(':') {
            None => Ok(None),
            Some(("v", text)) => Ok(Some(text.to_string())),
            Some(("f", reason)) => Err(RecipeError::recipe(reason)),
            Some(("e", message)) => Err(RecipeError::instance(format!("Script error: {message}"))),
            Some(_) => Err(RecipeError::instance("Script error: bad answer")),
        }
    }
}

/// What a call that ended in an exception is: the recipe's failure for what the limits do inside
/// QuickJS, else an error of the script.
fn classify(message: String, limit: usize) -> RecipeError {
    if message.contains("out of memory") {
        RecipeError::recipe(format!("out of memory (limit {} KB)", limit / 1024))
    } else if message.contains("call stack size") {
        RecipeError::recipe("stack overflow")
    } else {
        RecipeError::instance(format!("Script error: {message}"))
    }
}

impl Recipe for ScriptRecipe {
    fn run_stats(&self) -> Option<RunStats> {
        let counters = *self.counters.lock().unwrap_or_else(PoisonError::into_inner);
        Some(RunStats {
            instances: self.instances.len(),
            calls: counters.calls,
            total_us: counters.total_us,
            max_us: counters.max_us,
            held: counters.held,
            peak: counters.peak,
        })
    }

    fn info(&self) -> serde_json::Value {
        serde_json::from_str(&self.info).unwrap_or(serde_json::Value::Null)
    }

    fn info_text(&self) -> String {
        self.info.clone()
    }

    fn validate(&self, host: &dyn Host, params: &serde_json::Value) -> Result<(), RecipeError> {
        self.check(host, params)
    }

    fn start(
        &mut self,
        host: &dyn Host,
        handle: u32,
        params: &serde_json::Value,
        clock: Clock,
    ) -> Result<Update, RecipeError> {
        self.check(host, params)?;
        let (parsed, slots) = slots::resolve(&self.defs, params);
        self.drop_instance(handle, true);
        let (params_text, slots_text) = (parsed.to_string(), slots::slots_json(&slots));
        self.instances.push(Instance {
            handle,
            slots,
            state: "{}".to_string(),
            sensors: Vec::new(),
            next_mono: None,
            at_timers: Vec::new(),
        });
        let op = Op::Start {
            params: &params_text,
            slots: &slots_text,
        };
        match self.call(host, handle, op, clock) {
            // The shim always gives the listen set of a start: without it the instance would be
            // deaf, so the end of the call is refused as any other corrupt one.
            Ok(end) if end.l.is_none() => {
                self.drop_instance(handle, false);
                Err(RecipeError::recipe("bad end of call: no listen set"))
            }
            Ok(end) => self
                .apply(host, handle, end, true)
                .ok_or_else(|| RecipeError::instance("Script error: the instance is gone")),
            Err(error) => {
                // A failure of the recipe drops the runtime: nothing to tell the context.
                let recipe_failed = error.kind == solite_recipe_sdk::ErrorKind::Recipe;
                self.drop_instance(handle, !recipe_failed);
                Err(error)
            }
        }
    }

    fn stop(&mut self, handle: u32) {
        self.drop_instance(handle, true);
    }

    fn on_event(
        &mut self,
        host: &dyn Host,
        handle: u32,
        event: &Event,
        clock: Clock,
    ) -> Result<Option<Update>, RecipeError> {
        let calls: Vec<Op<'static>> = match event {
            Event::Equipment {
                equipment,
                alias,
                value,
            } => vec![Op::Event {
                kind: 1,
                a: Arg::Text(equipment.clone()),
                b: Arg::Text(alias.clone()),
                c: arg_of(value),
            }],
            Event::Zone {
                motion, luminosity, ..
            } => vec![Op::Event {
                kind: 2,
                a: Arg::Null,
                b: Arg::Bool(*motion),
                c: luminosity.map_or(Arg::Null, |l| Arg::Number(l as f64)),
            }],
            Event::Device { device, key, value } => self
                .instances
                .iter()
                .find(|i| i.handle == handle)
                .map(|i| {
                    i.sensors
                        .iter()
                        .filter(|s| s.device == *device && s.key == *key)
                        .map(|s| Op::Event {
                            kind: 3,
                            a: Arg::Text(s.equipment.clone()),
                            b: arg_of(value),
                            c: Arg::Null,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };
        let mut update: Option<Update> = None;
        for op in calls {
            let end = self.call(host, handle, op, clock)?;
            if let Some(mut next) = self.apply(host, handle, end, false) {
                // A later call that left the listen set as it was does not undo an earlier one's
                // change: the engine still has to hear of it.
                if next.listen.is_none() {
                    next.listen = update.and_then(|u| u.listen);
                }
                update = Some(next);
            }
        }
        Ok(update)
    }

    fn tick(
        &mut self,
        host: &dyn Host,
        handle: u32,
        clock: Clock,
    ) -> Result<Option<Update>, RecipeError> {
        let end = self.call(host, handle, Op::Tick, clock)?;
        Ok(self.apply(host, handle, end, false))
    }

    fn next_deadline(&self, host: &dyn Host, handle: u32) -> Deadline {
        let Some(instance) = self.instances.iter().find(|i| i.handle == handle) else {
            return Deadline::Never;
        };
        let wall = instance
            .at_timers
            .iter()
            .filter_map(|(h, m, last)| match last {
                // The clock was not set: the timer waits for it.
                None => Some(CLOCK_SET_AFTER_MS),
                Some(last) => next_at(*h, *m, *last, &|utc_ms| host.utc_offset_seconds(utc_ms)),
            })
            .min();
        if instance.next_mono.is_none() && wall.is_none() {
            return Deadline::Never;
        }
        Deadline::At {
            mono: instance.next_mono,
            wall,
        }
    }

    fn refresh(
        &mut self,
        host: &dyn Host,
        handle: u32,
        params: &serde_json::Value,
    ) -> Result<Update, RecipeError> {
        let (parsed, slots) = slots::resolve(&self.defs, params);
        let (params_text, slots_text) = (parsed.to_string(), slots::slots_json(&slots));
        if let Some(instance) = self.instances.iter_mut().find(|i| i.handle == handle) {
            instance.slots = slots;
        }
        let clock = Clock {
            mono: (self.clock)(),
            wall: 0,
        };
        let op = Op::Refresh {
            params: &params_text,
            slots: &slots_text,
        };
        let end = self.call(host, handle, op, clock)?;
        self.apply(host, handle, end, true)
            .ok_or_else(|| RecipeError::instance("Script error: the instance is gone"))
    }

    fn prune(
        &self,
        _host: &dyn Host,
        params: &serde_json::Value,
        change: &Change,
    ) -> Option<Pruned> {
        // A deleted equipment leaves the slots through the engine's generic settle (the params
        // are already without it). A running instance restarts when the equipment was in a
        // required slot (it lost what it works with, as the built-in recipe's lost light); one
        // that lost an equipment of an optional slot (a button) is refreshed in place.
        let restart = change
            .pruned
            .iter()
            .any(|slot| self.defs.iter().any(|def| def.id == *slot && def.required));
        restart.then(|| Pruned {
            params: params.clone(),
            lines: Vec::new(),
            restart: true,
        })
    }
}

// ---- The host functions the shim reaches the engine through ----

impl CallCx<'_> {
    /// The instance of a handle. A handle that is not one of this recipe's instances is a forged
    /// one: the call is stopped and the recipe fails, whatever the script does with the error.
    fn instance(&self, handle: u32) -> Result<&Instance, String> {
        match self.instances.iter().find(|i| i.handle == handle) {
            Some(instance) => Ok(instance),
            None => {
                self.gate.kill(limits::Reason::ForgedHandle);
                Err("unknown instance".to_string())
            }
        }
    }

    fn equipments(&self, handle: u32, slot: &str) -> Result<&[String], String> {
        let instance = self.instance(handle)?;
        match instance.slot(slot) {
            Some(s) if s.kind == SlotKind::Equipment => Ok(&s.ids),
            _ => Err(format!("Unknown slot: {slot}")),
        }
    }
}

fn throw<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Error {
    Exception::throw_message(ctx, message)
}

/// Runs `f` with the call's context; an error is thrown into the script.
fn with_call<'js, R>(
    ctx: &Ctx<'js>,
    f: impl FnOnce(&CallCx<'_>) -> Result<R, String>,
) -> rquickjs::Result<R> {
    if !CALL.is_set() {
        return Err(throw(ctx, "the engine is not in a call"));
    }
    CALL.with(f).map_err(|message| throw(ctx, &message))
}

fn to_js<'js>(ctx: &Ctx<'js>, value: &EventValue) -> rquickjs::Result<Value<'js>> {
    match value {
        EventValue::Null => Ok(Value::new_null(ctx.clone())),
        EventValue::Bool(b) => Ok(Value::new_bool(ctx.clone(), *b)),
        EventValue::Number(n) => Ok(Value::new_float(ctx.clone(), *n)),
        EventValue::Text(text) => text.as_str().into_js(ctx),
    }
}

/// What `get` reads of an equipment: the value under the alias; for `state`, the data of the
/// category `light_state` first, as Sowel's `isAnyLightOn` does.
fn read(host: &dyn Host, id: &str, alias: &str) -> EventValue {
    let Some(view) = host.equipment(id) else {
        return EventValue::Null;
    };
    let by_category = (alias == "state")
        .then(|| {
            view.data_bindings
                .iter()
                .find(|b| b.category.as_deref() == Some("light_state"))
        })
        .flatten();
    by_category
        .or_else(|| view.data_bindings.iter().find(|b| b.alias == alias))
        .map_or(EventValue::Null, |b| b.value.clone())
}

/// What `order` does for one equipment: the order binding of the alias, for `state` a
/// `light_toggle` or `toggle_power` binding first (Sowel's `turnOnLights`); a disabled equipment
/// gets nothing and says nothing.
fn order_one(host: &dyn Host, id: &str, alias: &str, value: &serde_json::Value) -> OrderStatus {
    let Some(view) = host.equipment(id) else {
        return OrderStatus::NoBinding;
    };
    if !view.enabled {
        return OrderStatus::Disabled;
    }
    let by_category = (alias == "state")
        .then(|| {
            view.order_bindings
                .iter()
                .find(|b| matches!(b.category.as_deref(), Some("light_toggle" | "toggle_power")))
        })
        .flatten();
    let binding = by_category.or_else(|| view.order_bindings.iter().find(|b| b.alias == alias));
    match binding {
        Some(binding) => host.order(&view.id, &binding.id, value),
        None => OrderStatus::NoBinding,
    }
}

/// The motion sensors of a zone and the zones below it: each data binding of category `motion`
/// of an enabled equipment, once per (device, key, equipment); at most `MAX_LISTEN_DEVICES`
/// (device, key) pairs.
fn resolve_sensors(host: &dyn Host, zone: &str) -> Vec<Sensor> {
    let mut sensors: Vec<Sensor> = Vec::new();
    for id in host.zone_equipment_ids(zone) {
        let Some(view) = host.equipment(&id) else {
            continue;
        };
        for binding in &view.data_bindings {
            if binding.category.as_deref() != Some("motion") {
                continue;
            }
            let (Some(device), Some(key)) = (&binding.device_id, &binding.key) else {
                continue;
            };
            let sensor = Sensor {
                device: device.clone(),
                key: key.clone(),
                equipment: view.id.clone(),
            };
            let pairs = sensors
                .iter()
                .filter(|s| !(s.device == sensor.device && s.key == sensor.key))
                .count();
            if !sensors.contains(&sensor) && pairs < MAX_LISTEN_DEVICES {
                sensors.push(sensor);
            }
        }
    }
    sensors
}

/// Puts the engine's functions in the context as the object `__host`; the shim takes it and
/// deletes it from the globals.
pub(super) fn register<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let host = Object::new(ctx.clone())?;
    host.set(
        "get",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>,
             handle: u32,
             slot: String,
             alias: String|
             -> rquickjs::Result<Array<'js>> {
                let values = with_call(&ctx, |cx| {
                    let ids = cx.equipments(handle, &slot)?;
                    Ok(ids
                        .iter()
                        .map(|id| read(cx.host, id, &alias))
                        .collect::<Vec<_>>())
                })?;
                let array = Array::new(ctx.clone())?;
                for (i, value) in values.iter().enumerate() {
                    array.set(i, to_js(&ctx, value)?)?;
                }
                Ok(array)
            },
        ),
    )?;
    host.set(
        "zone",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>,
             handle: u32,
             slot: String,
             alias: String|
             -> rquickjs::Result<Value<'js>> {
                let data = with_call(&ctx, |cx| {
                    let instance = cx.instance(handle)?;
                    let zone = match instance.slot(&slot) {
                        Some(s) if s.kind == SlotKind::Zone => s.ids.first().cloned(),
                        _ => return Err(format!("Unknown slot: {slot}")),
                    };
                    Ok(zone.and_then(|zone| cx.host.zone_data(&zone)))
                })?;
                let Some(data) = data else {
                    return Ok(Value::new_null(ctx.clone()));
                };
                match alias.as_str() {
                    "motion" => Ok(Value::new_bool(ctx.clone(), data.motion)),
                    "luminosity" => data
                        .luminosity
                        .map_or(Ok(Value::new_null(ctx.clone())), |l| {
                            Ok(Value::new_float(ctx.clone(), l as f64))
                        }),
                    "motionSensors" => Ok(Value::new_float(
                        ctx.clone(),
                        f64::from(data.motion_sensors),
                    )),
                    other => Err(throw(&ctx, &format!("Unknown zone data: {other}"))),
                }
            },
        ),
    )?;
    host.set(
        "order",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>,
             handle: u32,
             slot: String,
             alias: String,
             value: String|
             -> rquickjs::Result<String> {
                let value: serde_json::Value = serde_json::from_str(&value)
                    .map_err(|_| throw(&ctx, "order: the value cannot be sent"))?;
                let refused = with_call(&ctx, |cx| {
                    let ids = cx.equipments(handle, &slot)?;
                    let mut refused: Vec<(String, String)> = Vec::new();
                    for id in ids {
                        match order_one(cx.host, id, &alias, &value) {
                            OrderStatus::Sent | OrderStatus::Disabled => {}
                            OrderStatus::NoBinding => {
                                refused.push((id.clone(), format!("no order {alias}")));
                            }
                            OrderStatus::QueueFull => {
                                refused.push((id.clone(), "queue full".to_string()));
                            }
                            OrderStatus::Refused => {
                                refused.push((id.clone(), "value refused".to_string()));
                            }
                        }
                    }
                    Ok(refused)
                })?;
                Ok(json!(refused).to_string())
            },
        ),
    )?;
    // What a script's `validate` may know of an equipment its own params name (the shim checks the
    // id is one of them): its name, its `type` and `zoneId` (Sowel's fields) and the aliases of its
    // order and data bindings.
    host.set(
        "facts",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, id: String| -> rquickjs::Result<Option<String>> {
                with_call(&ctx, |cx| {
                    // Only what the params under check name as an equipment: any other text (a
                    // zone id, a time, an option) gives nothing, and the facts stay out of reach.
                    if !cx.named.contains(&id) {
                        return Ok(None);
                    }
                    Ok(cx.host.equipment(&id).map(|view| {
                        json!({
                            "name": view.name,
                            "type": view.kind,
                            "zoneId": view.zone_id,
                            "orders": view.order_bindings.iter().map(|b| &b.alias).collect::<Vec<_>>(),
                            "data": view.data_bindings.iter().map(|b| &b.alias).collect::<Vec<_>>(),
                        })
                        .to_string()
                    }))
                })
            },
        ),
    )?;
    // Whether a zone its own params name exists (Sowel's `Zone not found`): `null` for any other
    // text.
    host.set(
        "zoneFacts",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, id: String| -> rquickjs::Result<Option<String>> {
                with_call(&ctx, |cx| {
                    if !cx.named.contains(&id) || cx.host.zone_data(&id).is_none() {
                        return Ok(None);
                    }
                    Ok(Some(json!({ "id": id }).to_string()))
                })
            },
        ),
    )?;
    host.set(
        "sun",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
                let sun = with_call(&ctx, |cx| {
                    serde_json::to_string(&cx.host.sunlight()).map_err(|e| e.to_string())
                })?;
                ctx.json_parse(sun)
            },
        ),
    )?;
    host.set(
        "dur",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, value: Value<'js>| -> rquickjs::Result<f64> {
                let parsed = if let Some(n) = value.as_number() {
                    json!(n)
                } else if let Some(text) = value.as_string().and_then(|s| s.to_string().ok()) {
                    json!(text)
                } else {
                    return Err(throw(
                        &ctx,
                        "a duration is a number of milliseconds or a text like \"10m\"",
                    ));
                };
                parse_duration(&parsed)
                    .map(|ms| ms as f64)
                    .map_err(|message| throw(&ctx, &message))
            },
        ),
    )?;
    // The home's offset from UTC at an instant, in seconds east (`now().localTime`).
    host.set(
        "offset",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, utc_ms: f64| -> rquickjs::Result<f64> {
                with_call(&ctx, |cx| {
                    Ok(cx.host.utc_offset_seconds(utc_ms as u64) as f64)
                })
            },
        ),
    )?;
    host.set(
        "atNext",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, hour: u32, minute: u32, after: f64| -> rquickjs::Result<Option<f64>> {
                with_call(&ctx, |cx| {
                    Ok(next_at(hour, minute, after as u64, &|utc_ms| {
                        cx.host.utc_offset_seconds(utc_ms)
                    })
                    .map(|ms| ms as f64))
                })
            },
        ),
    )?;
    ctx.globals().set("__host", host)
}

/// The first instant after `after` (milliseconds since 1970) at which the home's clock reads
/// `hour`:`minute`; `offset` gives the home's offset from UTC, in seconds east, at an instant (in
/// milliseconds). A day on which that time does not exist (the clock jumps over it) has its
/// timer fire at the instant the clock jumps; a day on which it occurs twice has it fire at the
/// first one only.
pub(super) fn next_at(
    hour: u32,
    minute: u32,
    after: u64,
    offset: &dyn Fn(u64) -> i64,
) -> Option<u64> {
    // Every step is checked: `after` comes from a script's timers, whatever it is, nothing panics.
    if hour >= 24 || minute >= 60 {
        return None;
    }
    let after = i64::try_from(after).ok()?;
    let off = |utc_seconds: i64| {
        offset(
            u64::try_from(utc_seconds.max(0))
                .unwrap_or(0)
                .saturating_mul(1000),
        )
    };
    let first_day = after
        .div_euclid(1000)
        .checked_add(off(after.div_euclid(1000)))?
        .div_euclid(86_400);
    let time_of_day = i64::from(hour) * 3600 + i64::from(minute) * 60;
    for day in first_day..first_day.checked_add(3)? {
        // The wall-clock seconds of that time on that day, as if it were UTC.
        let target = day.checked_mul(86_400)?.checked_add(time_of_day)?;
        let (before, later) = (
            off(target.checked_sub(86_400)?),
            off(target.checked_add(86_400)?),
        );
        let mut instants: Vec<i64> = Vec::new();
        for o in [before, later] {
            let utc = target.checked_sub(o)?;
            if off(utc) == target.checked_sub(utc)? {
                instants.push(utc);
            }
        }
        if instants.is_empty() {
            // The clock jumps over this time: it fires at the jump, the first minute after it.
            let (mut low, mut high) = (
                target.checked_sub(before.max(later))?,
                target.checked_sub(before.min(later))?,
            );
            if low > high {
                std::mem::swap(&mut low, &mut high);
            }
            let jumped_to = off(high);
            while low < high {
                let middle = low + (high - low) / 2;
                if off(middle) == jumped_to {
                    high = middle;
                } else {
                    low = middle + 1;
                }
            }
            instants.push(low);
        }
        let first = instants.into_iter().min()?.checked_mul(1000)?;
        if first > after {
            return u64::try_from(first).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{End, next_at, validate_end};
    use crate::settings::tz;

    fn end(json: &str) -> End {
        serde_json::from_str(json).unwrap()
    }

    const NO_TIMER: &str = r#""d":{"m":null,"a":[]}"#;

    #[test]
    fn the_end_of_a_call_is_checked_before_the_engine_keeps_any_of_it() {
        let ok = |json: String| {
            validate_end(&end(&json)).map_err(|e| e.replace("bad end of call: ", ""))
        };
        assert_eq!(ok(format!(r#"{{"s":"{{\"a\":1}}",{NO_TIMER}}}"#)), Ok(()));
        // A state that is not a JSON object, or over its bound.
        assert_eq!(
            ok(format!(r#"{{"s":"{{\"broken\":",{NO_TIMER}}}"#)),
            Err("the state is not a JSON object".to_string())
        );
        assert_eq!(
            ok(format!(r#"{{"s":"[1]",{NO_TIMER}}}"#)),
            Err("the state is not a JSON object".to_string())
        );
        let big = format!(
            r#"{{"s":"{{\"a\":\"{}\"}}",{NO_TIMER}}}"#,
            "x".repeat(100_000)
        );
        assert_eq!(ok(big), Err("the state is over 512 bytes".to_string()));
        // 300 timers, a time of day that does not exist, instants nobody can read.
        let timers = vec!["[1,2,3]"; 300].join(",");
        assert_eq!(
            ok(format!(r#"{{"d":{{"m":null,"a":[{timers}]}}}}"#)),
            Err("too many timers".to_string())
        );
        for bad in [
            r#"[[24,0,1]]"#,
            r#"[[0,60,1]]"#,
            r#"[[23,59,9223372036854000000]]"#,
            r#"[[1,1,-5]]"#,
        ] {
            assert_eq!(
                ok(format!(r#"{{"d":{{"m":null,"a":{bad}}}}}"#)),
                Err("a time of day is out of range".to_string()),
                "{bad}"
            );
        }
        assert_eq!(
            ok(r#"{"d":{"m":1e300,"a":[]}}"#.to_string()),
            Err("a timer's time is out of range".to_string())
        );
        // Log lines: 21 are the most (20 and the one that says some were dropped).
        let lines = |n: usize| {
            format!(
                r#"{{"g":[{}],{NO_TIMER}}}"#,
                vec![r#"[0,"x"]"#; n].join(",")
            )
        };
        assert_eq!(ok(lines(21)), Ok(()));
        assert_eq!(ok(lines(22)), Err("too many log lines".to_string()));
        // An alias is at most 64 bytes.
        let alias = |n: usize| {
            format!(
                r#"{{"l":{{"e":[["id","{}"]],"z":false,"m":false}},{NO_TIMER}}}"#,
                "a".repeat(n)
            )
        };
        assert_eq!(ok(alias(64)), Ok(()));
        assert_eq!(ok(alias(65)), Err("an alias is too long".to_string()));
    }

    #[test]
    fn a_time_of_day_after_any_instant_never_overflows() {
        let offset = |_: u64| 3600;
        for after in [
            u64::MAX,
            i64::MAX as u64,
            9_223_372_036_854_000_000,
            9_007_199_254_740_992,
        ] {
            // Some or None, never a panic.
            let _ = next_at(23, 59, after, &offset);
            let _ = next_at(0, 0, after, &offset);
        }
        assert_eq!(next_at(24, 0, 0, &offset), None);
    }

    /// The next instant `hour`:`minute` reads in `zone`, after `after` (milliseconds since 1970).
    fn next(zone: &str, hour: u32, minute: u32, after: u64) -> Option<u64> {
        let zone = tz::find(zone).unwrap();
        next_at(hour, minute, after, &|utc_ms| {
            zone.offset_at(i64::try_from(utc_ms).unwrap())
        })
    }

    // 2027-03-27T12:00:00Z, the day before Europe's clocks go forward (at 01:00Z on the 28th).
    const BEFORE_SPRING: u64 = 1_806_148_800_000;
    const SPRING_JUMP: u64 = 1_806_195_600_000;
    // 2026-10-24T12:00:00Z, the day before they go back (at 01:00Z on the 25th).
    const BEFORE_AUTUMN: u64 = 1_792_843_200_000;

    #[test]
    fn a_time_of_day_is_read_in_the_homes_zone() {
        // Paris is an hour ahead of UTC in winter: 07:00 there is 06:00Z.
        let after = 1_795_651_200_000; // 2026-11-26T00:00:00Z
        assert_eq!(
            next("Europe/Paris", 7, 0, after),
            Some(after + 6 * 3_600_000)
        );
        assert_eq!(next("UTC", 7, 0, after), Some(after + 7 * 3_600_000));
        // The instant itself is not "after": the next one is the next day.
        let at = after + 7 * 3_600_000;
        assert_eq!(next("UTC", 7, 0, at), Some(at + 24 * 3_600_000));
    }

    #[test]
    fn a_time_that_does_not_exist_fires_at_the_jump_on_the_spring_day_only() {
        // 02:30 does not exist on 2027-03-28 in Paris: the timer fires at the jump (01:00Z, when
        // the clock reads 03:00, the first minute after the missing hour).
        assert_eq!(
            next("Europe/Paris", 2, 30, BEFORE_SPRING),
            Some(SPRING_JUMP)
        );
        // After it fired, the next one is the next day's 02:30 (CEST, 00:30Z).
        assert_eq!(
            next("Europe/Paris", 2, 30, SPRING_JUMP),
            Some(1_806_280_200_000)
        );
    }

    #[test]
    fn a_time_that_occurs_twice_fires_once_on_the_autumn_day() {
        // 02:30 reads twice on 2026-10-25 in Paris (00:30Z in CEST, 01:30Z in CET): the first one.
        let first = 1_792_888_200_000;
        assert_eq!(next("Europe/Paris", 2, 30, BEFORE_AUTUMN), Some(first));
        // After the first, not the second (01:30Z): the next day's, which is 01:30Z in CET.
        assert_eq!(next("Europe/Paris", 2, 30, first), Some(1_792_978_200_000));
    }

    #[test]
    fn a_zone_without_daylight_saving_has_one_instant_a_day() {
        let after = BEFORE_SPRING;
        let day = 24 * 3_600_000;
        let one = next("Asia/Tokyo", 9, 0, after).unwrap();
        assert_eq!(next("Asia/Tokyo", 9, 0, one), Some(one + day));
    }
}
