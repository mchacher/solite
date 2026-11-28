//! The loader of a script recipe (spec 041, R1.1, R1.2, R3.8): a fresh runtime and context, the
//! shim, then the script's own source, which calls `recipe({...})` once; what it declared is
//! checked here, with the texts of R4.1 (`Script recipe invalid: <reason>`, the reason is made
//! here), and turned into the `RecipeInfo` the engine serves.

use std::sync::Arc;

use rquickjs::context::intrinsic::{Date, Eval, Json, MapSet, Promise};
use rquickjs::{Context, Object};
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;
use solite_recipe_sdk::bounds::MAX_LISTEN_EQUIPMENTS;
use solite_recipe_sdk::bounds::MAX_STATE_BYTES;

use super::limits::{
    self, Gate, MAX_ALIAS_BYTES, MAX_LOG_LINES_PER_CALL, MAX_SCRIPT_BYTES, MAX_TIMERS,
    SCRIPT_LOAD_LIMIT_MS, ScriptRuntimes,
};
use super::runtime::{Parts, SHIM_GLOBAL, ScriptRecipe, describe, register};
use super::slots::{info_slots_text, parse_slots_raw};
use crate::home::CLOCK_SET_AFTER_MS;
use crate::recipes::ScriptClock;

const SHIM: &str = include_str!("shim.js");

/// What a loaded script recipe is, as the host prints it (`recipes: script <id> <version> ...`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSummary {
    pub id: String,
    pub version: Option<String>,
    /// The script's source, in bytes.
    pub source_bytes: usize,
    /// What QuickJS holds for the recipe once it is loaded, in bytes.
    pub runtime_bytes: usize,
}

/// What the shim reports of the script's `recipe({...})` call.
#[derive(Debug, Deserialize)]
struct Declared {
    count: u32,
    api: Option<Value>,
    id: Option<Value>,
    version: Option<Value>,
    name: Option<Value>,
    description: Option<Value>,
    icon: Option<Value>,
    i18n: Option<Box<RawValue>>,
    slots: Option<Vec<(String, Box<RawValue>)>>,
    #[serde(rename = "hasStart")]
    has_start: Option<bool>,
    #[serde(rename = "hasValidate")]
    has_validate: Option<bool>,
}

/// The shim with its comment lines and indentation left out: QuickJS keeps the source of every
/// function it compiles, so each byte of the shim is memory held by every runtime.
fn shim() -> String {
    let compact: Vec<&str> = SHIM
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect();
    compact
        .join("\n")
        .replace("@MAX_TIMERS@", &MAX_TIMERS.to_string())
        .replace("@MAX_LISTEN@", &MAX_LISTEN_EQUIPMENTS.to_string())
        .replace("@MAX_STATE@", &MAX_STATE_BYTES.to_string())
        .replace("@MAX_LINES@", &MAX_LOG_LINES_PER_CALL.to_string())
        .replace("@MAX_ALIAS@", &MAX_ALIAS_BYTES.to_string())
        .replace("@CLOCK_SET@", &CLOCK_SET_AFTER_MS.to_string())
}

/// The ids no script may have (spec 042 R1.4): `/recipes/store` and `/recipes/install` sit beside
/// Sowel's `/recipes/:recipeId`.
pub const RESERVED_IDS: [&str; 2] = ["store", "install"];

/// The longest id of an installed script (R1.4): its file is `scripts/<id>.srs`, and the FAT's
/// names stay under 64 characters.
pub const MAX_INSTALLED_ID_CHARS: usize = 32;

/// What a script is held to: the rules of an installed script (spec 042 R1.3, R1.4) are stricter
/// than those of a script given by `--recipe-script`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    /// An id of 1 to 32 characters and a version that is `x.y.z` (three numbers of 0 to 65535).
    pub installed: bool,
}

impl Rules {
    pub const BUILT_IN: Rules = Rules { installed: false };
    pub const INSTALLED: Rules = Rules { installed: true };
}

/// The id of a script recipe: 1 to 64 characters among `a-z`, `0-9` and `-`, 1 to 32 when
/// installed.
fn valid_id(id: &str, longest: usize) -> bool {
    (1..=longest).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn text(value: &Option<Value>, what: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("{what} must be a text")),
    }
}

/// Loads a script: the recipe, and what the host prints of it. The error is the reason, without
/// the `Script recipe invalid: ` the engine puts in front of it.
pub fn load(
    source: &str,
    runtimes: &Arc<dyn ScriptRuntimes>,
    clock: &ScriptClock,
    rules: Rules,
) -> Result<(ScriptRecipe, ScriptSummary), String> {
    if source.trim().is_empty() {
        return Err("empty".to_string());
    }
    if source.len() > MAX_SCRIPT_BYTES {
        return Err(format!(
            "too large ({} bytes, at most {MAX_SCRIPT_BYTES})",
            source.len()
        ));
    }
    let made = runtimes
        .new_runtime()
        .map_err(|e| format!("no runtime: {e}"))?;
    let (rt, refused, held) = (made.runtime, made.refused, made.held);
    let gate = Arc::new(Gate::default());
    let limit = runtimes.memory_limit();
    limits::configure(&rt, &gate, clock, limit, runtimes.quickjs_limit());
    limits::before_load(&rt, &gate, clock, limit, &refused);
    let ctx = Context::custom::<(Date, Eval, Json, MapSet, Promise)>(&rt)
        .map_err(|e| format!("no context: {e}"))?;
    // A promise rejected and never handled goes to the shim, which reports it at the end of the
    // call (a throw inside a `then` is one).
    rt.set_host_promise_rejection_tracker(Some(Box::new(|ctx, promise, reason, handled| {
        if let Ok(entry) = ctx.globals().get::<_, Object>(SHIM_GLOBAL)
            && let Ok(hook) = entry.get::<_, rquickjs::Function>("rejected")
            && hook.call::<_, ()>((promise, reason, handled)).is_err()
        {
            ctx.catch();
        }
    })));
    let loaded = ctx.with(|ctx| -> Result<String, String> {
        register(&ctx).map_err(|e| describe(&ctx, e))?;
        ctx.eval::<(), _>(shim())
            .map_err(|e| format!("the engine's shim failed: {}", describe(&ctx, e)))?;
        if let Err(error) = ctx.eval::<(), _>(source.as_bytes().to_vec()) {
            let thrown = ctx.catch();
            let name = thrown
                .as_object()
                .and_then(|o| o.get::<_, Option<String>>("name").ok().flatten());
            let message = thrown
                .as_exception()
                .and_then(|e| e.message())
                .unwrap_or_else(|| describe(&ctx, error));
            return Err(match name.as_deref() {
                Some("SyntaxError") => format!("does not compile: {message}"),
                _ => format!("the script failed when loaded: {message}"),
            });
        }
        let declared: String = ctx
            .globals()
            .get::<_, Object>(SHIM_GLOBAL)
            .and_then(|entry| entry.get::<_, rquickjs::Function>("def")?.call(()))
            .map_err(|e| describe(&ctx, e))?;
        Ok(declared)
    });
    if let Some(reason) = gate.take() {
        return Err(match reason {
            limits::Reason::Time => {
                format!("the script took more than {SCRIPT_LOAD_LIMIT_MS} ms when loaded")
            }
            other => format!("the script {other} when loaded"),
        });
    }
    if limits::was_refused(&refused) {
        return Err("the script ran out of memory when loaded".to_string());
    }
    let declared = loaded?;
    let declared: Declared =
        serde_json::from_str(&declared).map_err(|e| format!("bad definition: {e}"))?;
    let (info, summary_id, version, defs, has_validate) = check(declared, rules)?;
    let runtime_bytes = limits::held(&rt, &held);
    let summary = ScriptSummary {
        id: summary_id,
        version,
        source_bytes: source.len(),
        runtime_bytes,
    };
    let recipe = ScriptRecipe::assemble(Parts {
        ctx,
        rt,
        gate,
        clock: Arc::clone(clock),
        limit,
        refused,
        held,
        info,
        defs,
        has_validate,
    });
    Ok((recipe, summary))
}

type Checked = (
    String,
    String,
    Option<String>,
    Vec<super::slots::SlotDef>,
    bool,
);

/// The `RecipeInfo` as it is served, its keys in the order a JSON object of Sowel's has them
/// when serialized sorted.
#[derive(serde::Serialize)]
struct Info<'a> {
    description: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    i18n: Option<&'a RawValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<&'a str>,
    id: &'a str,
    name: &'a str,
    slots: &'a RawValue,
}

/// A JSON text with the keys of every object in sorted order, as a `serde_json::Value` writes it
/// (the order the API serves has always had), made from the pieces of the text: each value is
/// borrowed, only the keys of the object being written are copied, so no tree of the whole text
/// is built (a tree of the `i18n` text is the biggest of a load, in small blocks).
fn sorted_keys(out: &mut String, text: &str) -> Result<(), serde_json::Error> {
    match text.as_bytes().first() {
        Some(b'{') => {
            let fields: std::collections::BTreeMap<String, &RawValue> = serde_json::from_str(text)?;
            out.push('{');
            for (n, (key, value)) in fields.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key)?);
                out.push(':');
                sorted_keys(out, value.get())?;
            }
            out.push('}');
        }
        Some(b'[') => {
            let items: Vec<&RawValue> = serde_json::from_str(text)?;
            out.push('[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                sorted_keys(out, item.get())?;
            }
            out.push(']');
        }
        _ => out.push_str(text),
    }
    Ok(())
}

/// Checks what the script declared and builds the `RecipeInfo`.
fn check(declared: Declared, rules: Rules) -> Result<Checked, String> {
    match declared.count {
        0 => return Err("no recipe() call".to_string()),
        1 => {}
        _ => return Err("recipe() is called more than once".to_string()),
    }
    if declared.api.as_ref().and_then(Value::as_u64) != Some(1) {
        return Err("api must be 1".to_string());
    }
    let longest = if rules.installed {
        MAX_INSTALLED_ID_CHARS
    } else {
        64
    };
    let id = match declared.id.as_ref().and_then(Value::as_str) {
        Some(id) if valid_id(id, longest) => id.to_string(),
        _ => {
            return Err(format!(
                "id must be 1 to {longest} characters among a-z, 0-9 and -"
            ));
        }
    };
    if RESERVED_IDS.contains(&id.as_str()) {
        return Err(format!("id {id} is reserved"));
    }
    let name = match declared.name.as_ref().and_then(Value::as_str) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => return Err("name is required".to_string()),
    };
    let description = text(&declared.description, "description")?.unwrap_or_default();
    let version = text(&declared.version, "version")?;
    if rules.installed
        && version
            .as_deref()
            .is_none_or(|v| crate::update::Version::parse(v).is_none())
    {
        return Err("version must be x.y.z".to_string());
    }
    let icon = text(&declared.icon, "icon")?;
    // `i18n` stays the script's own text: a tree of it is the biggest one a load would build.
    if declared
        .i18n
        .as_ref()
        .is_some_and(|raw| !raw.get().starts_with('{'))
    {
        return Err("i18n must be an object".to_string());
    }
    if declared.has_start != Some(true) {
        return Err("no start function".to_string());
    }
    let Some(declared_slots) = declared.slots else {
        return Err("slots must be an object".to_string());
    };
    let defs = parse_slots_raw(&declared_slots)?;
    let i18n = match declared.i18n.as_deref() {
        Some(raw) => {
            let mut sorted = String::with_capacity(raw.get().len());
            sorted_keys(&mut sorted, raw.get()).map_err(|e| format!("bad definition: {e}"))?;
            Some(RawValue::from_string(sorted).map_err(|e| format!("bad definition: {e}"))?)
        }
        None => None,
    };
    let mut info = serde_json::to_string(&Info {
        description: &description,
        i18n: i18n.as_deref(),
        icon: icon.as_deref(),
        id: &id,
        name: &name,
        slots: &RawValue::from_string(info_slots_text(&defs))
            .map_err(|e| format!("bad definition: {e}"))?,
    })
    .map_err(|e| format!("bad definition: {e}"))?;
    info.shrink_to_fit();
    Ok((info, id, version, defs, declared.has_validate == Some(true)))
}
