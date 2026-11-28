// The measuring code of spec 057 (what one script engine costs), shared by two callers with
// `include!`: `crates/core/tests/engine_layouts.rs` (the Mac) and `firmware/src/script_phase.rs`
// (the QEMU image, through the firmware's PSRAM allocator). It uses rquickjs directly, with the
// engine's own context intrinsics; the caller sets the runtime's stack limit (the engine's is 16 KB) and context intrinsics (`Date, Eval, Json, MapSet,
// Promise`), the engine's shim text, and a stand-in for the engine's host functions (`__host`: the
// engine's own are native functions bound to a call; the stand-in is nine JavaScript closures that
// answer with empty values, so a recipe starts as if on an empty home). Nothing of it is in the
// engine.

use rquickjs::context::intrinsic::{Date, Eval, Json, MapSet, Promise};
use rquickjs::{Context, Runtime};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A runtime the caller made, with the allocator's own count of the bytes it holds when it keeps one.
pub struct Made {
    pub runtime: Runtime,
    pub held: Option<Arc<AtomicUsize>>,
}

/// One reading of QuickJS's accounting (after a collection).
#[derive(Debug, Clone, Default)]
pub struct Fig {
    /// The allocator's bytes when it counts them (the box's), else QuickJS's `malloc_size`.
    pub bytes: usize,
    pub malloc_size: i64,
    pub used: i64,
    pub objs: i64,
    pub atoms: i64,
    pub funcs: i64,
    pub code: i64,
}

impl Fig {
    pub fn add(&mut self, other: &Fig) {
        self.bytes += other.bytes;
        self.malloc_size += other.malloc_size;
        self.used += other.used;
        self.objs += other.objs;
        self.atoms += other.atoms;
        self.funcs += other.funcs;
        self.code += other.code;
    }

    pub fn text(&self) -> String {
        format!(
            "{} (malloc {} used {} objs {} atoms {} funcs {} code {})",
            self.bytes, self.malloc_size, self.used, self.objs, self.atoms, self.funcs, self.code
        )
    }
}

pub type Emit<'a> = &'a mut dyn FnMut(&str, &Fig);

fn read(rt: &Runtime, held: &Option<Arc<AtomicUsize>>) -> Fig {
    rt.run_gc();
    let u = rt.memory_usage();
    Fig {
        bytes: match held {
            Some(h) => h.load(Ordering::Relaxed),
            None => usize::try_from(u.malloc_size).unwrap_or(0),
        },
        malloc_size: u.malloc_size,
        used: u.memory_used_size,
        objs: u.obj_count,
        atoms: u.atom_count,
        funcs: u.js_func_count,
        code: u.js_func_code_size,
    }
}

fn context(rt: &Runtime) -> Result<Context, String> {
    Context::custom::<(Date, Eval, Json, MapSet, Promise)>(rt)
        .map_err(|e| format!("no context: {e}"))
}

fn fail(ctx: &rquickjs::Ctx<'_>, e: rquickjs::Error) -> String {
    let thrown = ctx.catch();
    thrown
        .as_exception()
        .and_then(|x| x.message())
        .unwrap_or_else(|| format!("{e}"))
}

fn run(ctx: &Context, code: String) -> Result<(), String> {
    ctx.with(|c| c.eval::<(), _>(code.into_bytes()).map_err(|e| fail(&c, e)))
}

fn text(ctx: &Context, code: String) -> Result<String, String> {
    ctx.with(|c| {
        c.eval::<String, _>(code.into_bytes())
            .map_err(|e| fail(&c, e))
    })
}

/// The stand-in for the engine's `__host`.
const HOST: &str = r#"{
  get: function () { return [null]; },
  zone: function () { return null; },
  order: function () { return "[]"; },
  facts: function () { return null; },
  zoneFacts: function () { return null; },
  sun: function () { return { isDaylight: true, sunrise: null, sunset: null }; },
  dur: function (d) { return typeof d === "number" ? d : 60000; },
  offset: function () { return 0; },
  atNext: function () { return null; }
}"#;

/// The shim as the loader makes it: comment lines and indentation left out, constants put in.
pub fn shim_text(raw: &str) -> String {
    let compact: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect();
    compact
        .join("\n")
        .replace("@MAX_TIMERS@", "16")
        .replace("@MAX_LISTEN@", "32")
        .replace("@MAX_STATE@", "512")
        .replace("@MAX_LINES@", "20")
        .replace("@MAX_ALIAS@", "64")
        .replace("@CLOCK_SET@", "1577836800000")
}

/// The shim turned into a factory, for one context that holds several recipes: each call makes a
/// recipe's own entry points and its own `recipe`, `console` and `Date`, instead of setting them as
/// globals. The shim's code is compiled once; each recipe gets its own closures.
pub fn factory_text(shim: &str) -> Result<String, String> {
    let mut text = shim.to_string();
    for (from, to) in [
        ("(function () {", "globalThis.__make = function (H) {"),
        ("var H = globalThis.__host;", "var G = {};"),
        ("delete globalThis.__host;", ""),
        ("globalThis.Date = PinnedDate;", "G.Date = PinnedDate;"),
        ("globalThis.console =", "G.console ="),
        ("globalThis.recipe = function", "G.recipe = function"),
    ] {
        if !text.contains(from) {
            return Err(format!(
                "the shim has no `{from}`: the factory cannot be made"
            ));
        }
        text = text.replacen(from, to, 1);
    }
    let tail = text.find("Object.defineProperty(globalThis, \"__solite_shim\"");
    let Some(tail) = tail else {
        return Err("the shim has no entry point to take".to_string());
    };
    text.truncate(tail);
    text.push_str("G.entry = entry; return G;\n};");
    Ok(text)
}

/// The params and slots JSON an instance of the recipe is started with, from what the script
/// declared (`def()`): an id for each zone and equipment slot, the declared default or a plain value
/// for the others.
fn start_args(def: &str) -> Result<(String, String), String> {
    let d: Value = serde_json::from_str(def).map_err(|e| format!("bad definition: {e}"))?;
    let mut params = serde_json::Map::new();
    let mut slots = serde_json::Map::new();
    for pair in d["slots"].as_array().map_or(&[][..], Vec::as_slice) {
        let (Some(id), definition) = (pair[0].as_str(), &pair[1]) else {
            continue;
        };
        let list = definition["list"].as_bool() == Some(true);
        match definition["type"].as_str().unwrap_or("") {
            "zone" => {
                slots.insert(
                    id.into(),
                    json!({"t": "zone", "l": false, "ids": ["zone-1"]}),
                );
                params.insert(id.into(), json!("zone-1"));
            }
            "equipment" => {
                let ids = vec![format!("eq-{id}-1")];
                slots.insert(id.into(), json!({"t": "equipment", "l": list, "ids": ids}));
                params.insert(id.into(), if list { json!(ids) } else { json!(ids[0]) });
            }
            kind => {
                let value = if !definition["default"].is_null() {
                    definition["default"].clone()
                } else {
                    match kind {
                        "duration" => json!(60000),
                        "number" => json!(10),
                        "boolean" => json!(false),
                        "time" => json!("07:30"),
                        "select" => match &definition["options"][0] {
                            Value::String(s) => json!(s),
                            other if other["value"].is_string() => other["value"].clone(),
                            _ => json!(""),
                        },
                        _ => json!("x"),
                    }
                };
                params.insert(id.into(), value);
            }
        }
    }
    Ok((
        Value::Object(params).to_string(),
        Value::Object(slots).to_string(),
    ))
}

fn js_string(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

fn call_start(ctx: &Context, entry: &str, handle: u32, def: &str) -> Result<String, String> {
    let (params, slots) = start_args(def)?;
    text(
        ctx,
        format!(
            "{entry}.start({handle}, {}, {}, 1000, 1795252323120)",
            js_string(&params),
            js_string(&slots)
        ),
    )
}

fn new_runtime(make: &dyn Fn() -> Result<Made, String>) -> Result<Made, String> {
    make()
}

/// A context with the host stand-in and the shim, as the loader leaves it before the script.
fn with_shim(rt: &Runtime, shim: &str) -> Result<Context, String> {
    let ctx = context(rt)?;
    run(&ctx, format!("globalThis.__host = {HOST};"))?;
    run(&ctx, shim.to_string()).map_err(|e| format!("the engine's shim failed: {e}"))?;
    Ok(ctx)
}

/// R1: the parts of the fixed cost, each after the one before it.
pub fn parts(
    make: &dyn Fn() -> Result<Made, String>,
    shim_raw: &str,
    first: &str,
    emit: Emit,
) -> Result<(), String> {
    let shim = shim_text(shim_raw);
    {
        let made = new_runtime(make)?;
        emit("runtime (empty)", &read(&made.runtime, &made.held));
        let ctx = context(&made.runtime)?;
        emit("context (intrinsics)", &read(&made.runtime, &made.held));
        run(&ctx, format!("globalThis.__host = {HOST};"))?;
        emit("host stand-in", &read(&made.runtime, &made.held));
        run(&ctx, shim.clone())?;
        emit("shim evaluated", &read(&made.runtime, &made.held));
        run(
            &ctx,
            "recipe({ id: 'empty', api: 1, version: '1.0.0', name: 'Empty', description: 'Empty', slots: {}, start: function () {} });"
                .to_string(),
        )?;
        text(&ctx, "__solite_shim.def()".to_string())?;
        emit("empty script recipe()", &read(&made.runtime, &made.held));
    }
    let made = new_runtime(make)?;
    let ctx = with_shim(&made.runtime, &shim)?;
    run(&ctx, first.to_string())?;
    let def = text(&ctx, "__solite_shim.def()".to_string())?;
    emit("motion-light loaded", &read(&made.runtime, &made.held));
    let status = call_start(&ctx, "__solite_shim", 1, &def)?;
    emit("one instance started", &read(&made.runtime, &made.held));
    if !status.is_empty() {
        return Err(format!("the first instance did not start: {status}"));
    }
    let status = call_start(&ctx, "__solite_shim", 2, &def)?;
    emit("two instances started", &read(&made.runtime, &made.held));
    if !status.is_empty() {
        return Err(format!("the second instance did not start: {status}"));
    }
    Ok(())
}

/// R2, layout 1: one runtime and one context for each recipe (the engine today). The emitted figure
/// after each recipe is the sum of the runtimes made so far.
pub fn layout_runtimes(
    make: &dyn Fn() -> Result<Made, String>,
    shim_raw: &str,
    sources: &[(&str, &str)],
    emit: Emit,
) -> Result<Vec<String>, String> {
    let shim = shim_text(shim_raw);
    let mut notes = Vec::new();
    let mut kept = Vec::new();
    for (name, source) in sources {
        let made = new_runtime(make)?;
        let ctx = with_shim(&made.runtime, &shim)?;
        let step = (|| -> Result<(), String> {
            run(&ctx, (*source).to_string())?;
            let def = text(&ctx, "__solite_shim.def()".to_string())?;
            let status = call_start(&ctx, "__solite_shim", 1, &def)?;
            if status.is_empty() {
                Ok(())
            } else {
                Err(format!("start: {status}"))
            }
        })();
        if let Err(e) = step {
            notes.push(format!("{name}: {e}"));
        }
        kept.push((made, ctx));
        let mut sum = Fig::default();
        for (made, _) in &kept {
            sum.add(&read(&made.runtime, &made.held));
        }
        emit(&format!("one runtime each, after {name}"), &sum);
    }
    Ok(notes)
}

/// R2, layout 2: one runtime, one context for each recipe.
pub fn layout_contexts(
    make: &dyn Fn() -> Result<Made, String>,
    shim_raw: &str,
    sources: &[(&str, &str)],
    emit: Emit,
) -> Result<Vec<String>, String> {
    let shim = shim_text(shim_raw);
    let made = new_runtime(make)?;
    let mut notes = Vec::new();
    let mut kept = Vec::new();
    for (name, source) in sources {
        let ctx = with_shim(&made.runtime, &shim)?;
        let step = (|| -> Result<(), String> {
            run(&ctx, (*source).to_string())?;
            let def = text(&ctx, "__solite_shim.def()".to_string())?;
            let status = call_start(&ctx, "__solite_shim", 1, &def)?;
            if status.is_empty() {
                Ok(())
            } else {
                Err(format!("start: {status}"))
            }
        })();
        if let Err(e) = step {
            notes.push(format!("{name}: {e}"));
        }
        kept.push(ctx);
        emit(
            &format!("one runtime, a context each, after {name}"),
            &read(&made.runtime, &made.held),
        );
    }
    Ok(notes)
}

/// What a recipe loaded into the shared context gave.
#[derive(Debug, Clone)]
pub struct Shared {
    pub name: String,
    /// What `recipe({...})` declared, the text the loader checks.
    pub def: Option<String>,
    /// Why it did not load or start; None when it did both.
    pub reason: Option<String>,
}

/// R2, layout 3 (and R3): one runtime, one context; the shim compiled once as a factory; each source
/// in its own function scope, with its own `recipe`, `console` and `Date` as parameters.
pub fn layout_shared(
    make: &dyn Fn() -> Result<Made, String>,
    shim_raw: &str,
    sources: &[(&str, &str)],
    emit: Emit,
) -> Result<Vec<Shared>, String> {
    let factory = factory_text(&shim_text(shim_raw))?;
    let made = new_runtime(make)?;
    let ctx = context(&made.runtime)?;
    run(
        &ctx,
        format!("globalThis.__host = {HOST}; globalThis.__regs = [];"),
    )?;
    emit("shared: host stand-in", &read(&made.runtime, &made.held));
    run(&ctx, factory)?;
    emit("shared: shim factory", &read(&made.runtime, &made.held));
    let mut out = Vec::new();
    let mut loaded = 0;
    for (name, source) in sources {
        let load = format!(
            "(function () {{ var g = __make(__host); (function (recipe, console, Date) {{\n{source}\n}}).call(undefined, g.recipe, g.console, g.Date); var d = g.entry.def(); __regs.push(g); return d; }})()"
        );
        let mut shared = Shared {
            name: (*name).to_string(),
            def: None,
            reason: None,
        };
        match text(&ctx, load) {
            Err(e) => {
                shared.reason = Some(format!("did not load: {e}"));
            }
            Ok(def) => {
                let status = call_start(&ctx, &format!("__regs[{loaded}].entry"), 1, &def)?;
                loaded += 1;
                shared.def = Some(def);
                if !status.is_empty() {
                    shared.reason = Some(format!("did not start: {status}"));
                }
            }
        }
        emit(
            &format!("one runtime, one context, after {name}"),
            &read(&made.runtime, &made.held),
        );
        out.push(shared);
    }
    Ok(out)
}

/// What a recipe declares loaded alone, in its own runtime (the engine today): the text to compare.
pub fn alone_def(
    make: &dyn Fn() -> Result<Made, String>,
    shim_raw: &str,
    source: &str,
) -> Result<String, String> {
    let made = new_runtime(make)?;
    let ctx = with_shim(&made.runtime, &shim_text(shim_raw))?;
    run(&ctx, source.to_string())?;
    text(&ctx, "__solite_shim.def()".to_string())
}
