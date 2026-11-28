//! The slots of a script recipe (spec 041, R1.2, R1.3): their definitions as the script declares
//! them, the `RecipeInfo` the engine serves from them, the host's validation of an instance's
//! params (Solite's own texts, since Sowel has no generic slot validation), and what a script
//! gets of the params once they passed: durations in milliseconds, equipments as ids.

use serde_json::value::RawValue;
use serde_json::{Map, Value, json};
use solite_recipe_sdk::duration::{js_number, js_string, parse_duration};
use solite_recipe_sdk::{Host, canonical_id};

/// Equipments in one `list` slot, at most (as the built-in recipe's lights).
pub const MAX_SLOT_EQUIPMENTS: usize = 16;
/// Slots a script declares, at most. Worst case: 32 definitions of about 300 bytes kept as the
/// recipe's info.
pub const MAX_SLOTS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Zone,
    Equipment,
    Number,
    Duration,
    Time,
    Boolean,
    Select,
    /// Free text (Sowel's `text`): its value is checked by the recipe's own `validate`.
    Text,
    /// A data alias of the equipment chosen in the recipe's first single `equipment` slot (Sowel's
    /// `data-key`): a text like `Text`; the recipe's own `validate` checks the alias exists.
    DataKey,
}

impl SlotKind {
    fn parse(text: &str) -> Option<SlotKind> {
        Some(match text {
            "zone" => SlotKind::Zone,
            "equipment" => SlotKind::Equipment,
            "number" => SlotKind::Number,
            "duration" => SlotKind::Duration,
            "time" => SlotKind::Time,
            "boolean" => SlotKind::Boolean,
            "select" => SlotKind::Select,
            "text" => SlotKind::Text,
            "data-key" => SlotKind::DataKey,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            SlotKind::Zone => "zone",
            SlotKind::Equipment => "equipment",
            SlotKind::Number => "number",
            SlotKind::Duration => "duration",
            SlotKind::Time => "time",
            SlotKind::Boolean => "boolean",
            SlotKind::Select => "select",
            SlotKind::Text => "text",
            SlotKind::DataKey => "data-key",
        }
    }
}

/// One slot, as declared (Sowel's `RecipeSlotDef`, `src/shared/types.ts` T:680).
#[derive(Debug, Clone)]
pub struct SlotDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: SlotKind,
    pub required: bool,
    pub list: bool,
    pub default: Option<Value>,
    /// `select`: (value, label).
    pub options: Vec<(String, String)>,
    /// The equipment types allowed (empty: any).
    pub equipment_types: Vec<String>,
    /// How the constraint was written (one text or a list), to serve it as it came.
    equipment_type_value: Option<Value>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub cross_zone: bool,
    pub include_descendants: bool,
    /// Sowel's `group`: the form shows the slots of one group together (a text, served as given).
    pub group: Option<String>,
    /// Sowel's `hiddenWhen` (`{ slot, equals }`, `equals` a text or a list of texts): the form hides
    /// the slot while the named slot holds one of them. Kept as given, served in the info; the
    /// host does nothing with it.
    pub hidden_when: Option<Box<Value>>,
}

/// The longest value of a `text` slot, in bytes (a bound: a value is stored in the instance's
/// settings, and Sowel has none; its words, such as `open`, are short).
pub const MAX_TEXT_BYTES: usize = 64;

fn text_of(value: Option<&Value>, default: &str) -> Result<String, ()> {
    match value {
        None | Some(Value::Null) => Ok(default.to_string()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(()),
    }
}

/// The slot ids a script may declare: letters, digits, `_` and `-`, 1 to 64 characters.
fn valid_slot_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The slots a script declared, from their JSON text: each one is read into a tree, turned
/// into a `SlotDef` and dropped before the next is read. The trees of 30 slots together are most
/// of the memory the load takes, in small blocks that the ESP32 keeps in its internal RAM.
pub fn parse_slots_raw(declared: &[(String, Box<RawValue>)]) -> Result<Vec<SlotDef>, String> {
    parse_slot_list(
        declared.len(),
        declared.iter().map(|(id, raw)| {
            let tree: Value = serde_json::from_str(raw.get())
                .map_err(|e| format!("slot {id}: a definition is expected ({e})"))?;
            Ok((id.as_str(), tree))
        }),
    )
}

fn parse_slot_list<'a>(
    count: usize,
    declared: impl Iterator<Item = Result<(&'a str, Value), String>>,
) -> Result<Vec<SlotDef>, String> {
    if count > MAX_SLOTS {
        return Err(format!("more than {MAX_SLOTS} slots"));
    }
    let mut defs: Vec<SlotDef> = Vec::new();
    for item in declared {
        let (id, raw) = item?;
        if !valid_slot_id(id) {
            return Err(format!(
                "slot \"{id}\": the id is 1 to 64 characters among letters, digits, _ and -"
            ));
        }
        let Some(raw) = raw.as_object() else {
            return Err(format!("slot {id}: a definition is expected"));
        };
        let bad = |what: &str| format!("slot {id}: {what}");
        let type_text = raw.get("type").and_then(Value::as_str).unwrap_or("");
        let kind = match SlotKind::parse(type_text) {
            Some(kind) => kind,
            None => return Err(bad(&format!("unknown type \"{type_text}\""))),
        };
        if matches!(kind, SlotKind::Equipment | SlotKind::Zone)
            && raw
                .get("defaultValue")
                .or_else(|| raw.get("default"))
                .is_some_and(|v| !v.is_null())
        {
            // A default would enter the allow-list of ids unchecked.
            return Err(bad(&format!(
                "a default is not allowed for type {}",
                kind.name()
            )));
        }
        let name = text_of(raw.get("name"), id).map_err(|()| bad("name must be a text"))?;
        let description =
            text_of(raw.get("description"), "").map_err(|()| bad("description must be a text"))?;
        let flag = |key: &str| matches!(raw.get(key), Some(Value::Bool(true)));
        let constraints = raw.get("constraints").and_then(Value::as_object);
        let from_constraints = |key: &str| constraints.and_then(|c| c.get(key));
        let equipment_type_value = from_constraints("equipmentType")
            .or_else(|| raw.get("equipmentType"))
            .cloned();
        let equipment_types = match &equipment_type_value {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::String(one)) => vec![one.clone()],
            Some(Value::Array(many)) => many
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            Some(_) => return Err(bad("equipmentType must be a text or a list")),
        };
        let options = match raw.get("options") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(list)) => list
                .iter()
                .map(|option| match option {
                    Value::String(value) => Ok((value.clone(), value.clone())),
                    Value::Object(fields) => {
                        let value = fields.get("value").and_then(Value::as_str);
                        let label = fields.get("label").and_then(Value::as_str);
                        value
                            .map(|value| (value.to_string(), label.unwrap_or(value).to_string()))
                            .ok_or_else(|| bad("an option has a value"))
                    }
                    _ => Err(bad("an option is a text or {value, label}")),
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(bad("options must be a list")),
        };
        let number = |key: &str| from_constraints(key).and_then(Value::as_f64);
        let hidden_when = match raw.get("hiddenWhen") {
            None | Some(Value::Null) => None,
            Some(Value::Object(rule)) => {
                let slot = rule.get("slot").and_then(Value::as_str);
                let equals = match rule.get("equals") {
                    Some(Value::String(one)) => Some(json!(one)),
                    Some(Value::Array(list)) if list.iter().all(Value::is_string) => {
                        Some(Value::Array(list.clone()))
                    }
                    _ => None,
                };
                match (slot, equals) {
                    (Some(slot), Some(equals)) => {
                        Some(Box::new(json!({"slot": slot, "equals": equals})))
                    }
                    _ => {
                        return Err(bad(
                            "hiddenWhen is {slot: a slot id, equals: a text or a list of texts}",
                        ));
                    }
                }
            }
            Some(_) => return Err(bad("hiddenWhen is {slot, equals}")),
        };
        defs.push(SlotDef {
            id: id.to_string(),
            name,
            description,
            kind,
            required: flag("required"),
            list: flag("list"),
            default: raw
                .get("defaultValue")
                .or_else(|| raw.get("default"))
                .filter(|v| !v.is_null())
                .cloned(),
            options,
            equipment_types,
            equipment_type_value,
            min: number("min"),
            max: number("max"),
            cross_zone: matches!(from_constraints("crossZone"), Some(Value::Bool(true))),
            include_descendants: matches!(
                from_constraints("includeDescendants"),
                Some(Value::Bool(true))
            ),
            group: raw.get("group").and_then(Value::as_str).map(str::to_string),
            hidden_when,
        });
    }
    match defs.iter().find(|d| d.id == "zone") {
        Some(zone) if zone.kind != SlotKind::Zone => {
            return Err("slot zone must be of type zone".to_string());
        }
        Some(_) => {}
        None => defs.insert(
            0,
            SlotDef {
                id: "zone".into(),
                name: "Zone".into(),
                description: "Zone to monitor".into(),
                kind: SlotKind::Zone,
                required: true,
                list: false,
                default: None,
                options: Vec::new(),
                equipment_types: Vec::new(),
                equipment_type_value: None,
                min: None,
                max: None,
                cross_zone: false,
                include_descendants: false,
                group: None,
                hidden_when: None,
            },
        ),
    }
    Ok(defs)
}

/// A bound as JavaScript writes it: a whole number has no decimal point (`0`, not `0.0`), so that
/// a script's `min: 0` is served as Sowel's TypeScript recipe serves it (spec 046, R2).
fn number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

/// One slot as `RecipeInfo.slots` serves it (Sowel's `RecipeSlotDef`).
fn slot_info(def: &SlotDef) -> Value {
    let mut slot = Map::new();
    slot.insert("id".into(), json!(def.id));
    slot.insert("name".into(), json!(def.name));
    slot.insert("description".into(), json!(def.description));
    slot.insert("type".into(), json!(def.kind.name()));
    slot.insert("required".into(), json!(def.required));
    if def.list {
        slot.insert("list".into(), json!(true));
    }
    if let Some(default) = &def.default {
        slot.insert("defaultValue".into(), default.clone());
    }
    if !def.options.is_empty() {
        let options: Vec<Value> = def
            .options
            .iter()
            .map(|(value, label)| json!({"value": value, "label": label}))
            .collect();
        slot.insert("options".into(), Value::Array(options));
    }
    let mut constraints = Map::new();
    if let Some(types) = &def.equipment_type_value {
        constraints.insert("equipmentType".into(), types.clone());
    }
    if let Some(min) = def.min {
        constraints.insert("min".into(), number_value(min));
    }
    if let Some(max) = def.max {
        constraints.insert("max".into(), number_value(max));
    }
    if def.cross_zone {
        constraints.insert("crossZone".into(), json!(true));
    }
    if def.include_descendants {
        constraints.insert("includeDescendants".into(), json!(true));
    }
    if !constraints.is_empty() {
        slot.insert("constraints".into(), Value::Object(constraints));
    }
    if let Some(group) = &def.group {
        slot.insert("group".into(), json!(group));
    }
    if let Some(rule) = &def.hidden_when {
        slot.insert("hiddenWhen".into(), (**rule).clone());
    }
    Value::Object(slot)
}

/// The slots as `RecipeInfo.slots` serves them (Sowel's `RecipeSlotDef`), as JSON text in
/// declaration order, one slot's tree at a time: the trees of every slot together are a big
/// part of the memory a load takes, in small blocks the ESP32 keeps in its internal RAM.
pub fn info_slots_text(defs: &[SlotDef]) -> String {
    let mut text = String::from("[");
    for (n, def) in defs.iter().enumerate() {
        if n > 0 {
            text.push(',');
        }
        text.push_str(&slot_info(def).to_string());
    }
    text.push(']');
    text
}

/// No value given: absent, `null`, `""` or an empty list.
fn missing(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(Value::Array(list)) => list.is_empty(),
        Some(_) => false,
    }
}

fn bound_text(def: &SlotDef) -> String {
    match (def.min, def.max) {
        (Some(min), Some(max)) => format!(
            "{} must be a number between {} and {}",
            def.id,
            js_number(min),
            js_number(max)
        ),
        (Some(min), None) => format!("{} must be a number of at least {}", def.id, js_number(min)),
        (None, Some(max)) => format!("{} must be a number of at most {}", def.id, js_number(max)),
        (None, None) => format!("{} must be a number", def.id),
    }
}

fn valid_time(text: &str) -> bool {
    let mut parts = text.split(':');
    let (Some(h), Some(m), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.len() <= 2 && s.chars().all(|c| c.is_ascii_digit());
    digits(h)
        && m.len() == 2
        && digits(m)
        && h.parse::<u32>().is_ok_and(|h| h < 24)
        && m.parse::<u32>().is_ok_and(|m| m < 60)
}

/// The host's validation of params against the slots (R1.3), in the order the slots are declared;
/// the first failure's text, to be served as `Invalid params: <text>`.
pub fn validate_params(defs: &[SlotDef], host: &dyn Host, params: &Value) -> Result<(), String> {
    let zone = params
        .get("zone")
        .and_then(Value::as_str)
        .map(str::to_string);
    for def in defs {
        let value = params.get(&def.id);
        if missing(value) {
            if def.required && def.default.is_none() {
                return Err(format!("{} is required", def.id));
            }
            continue;
        }
        let Some(value) = value else { continue };
        match def.kind {
            SlotKind::Zone => {
                let id = value.as_str().unwrap_or("");
                if host.zone_data(id).is_none() {
                    return Err(format!("{}: zone not found: {}", def.id, js_string(value)));
                }
            }
            SlotKind::Number => match value.as_f64() {
                Some(n)
                    if n.is_finite()
                        && def.min.is_none_or(|min| n >= min)
                        && def.max.is_none_or(|max| n <= max) => {}
                _ => return Err(bound_text(def)),
            },
            SlotKind::Duration => {
                if parse_duration(value).is_err() {
                    return Err(format!(
                        "{}: invalid duration \"{}\"",
                        def.id,
                        js_string(value)
                    ));
                }
            }
            SlotKind::Time => {
                if !value.as_str().is_some_and(valid_time) {
                    return Err(format!("{}: invalid time \"{}\"", def.id, js_string(value)));
                }
            }
            SlotKind::Boolean => {
                if !value.is_boolean() {
                    return Err(format!("{} must be true or false", def.id));
                }
            }
            SlotKind::Text | SlotKind::DataKey => match value.as_str() {
                Some(text) if text.len() <= MAX_TEXT_BYTES => {}
                Some(_) => {
                    return Err(format!("{}: at most {MAX_TEXT_BYTES} bytes", def.id));
                }
                None => return Err(format!("{} must be a text", def.id)),
            },
            SlotKind::Select => {
                let allowed = value
                    .as_str()
                    .is_some_and(|v| def.options.iter().any(|(o, _)| o == v));
                if !allowed {
                    let list: Vec<&str> = def.options.iter().map(|(o, _)| o.as_str()).collect();
                    return Err(format!("{} must be one of {}", def.id, list.join(", ")));
                }
            }
            SlotKind::Equipment => validate_equipments(def, host, value, zone.as_deref())?,
        }
    }
    Ok(())
}

fn validate_equipments(
    def: &SlotDef,
    host: &dyn Host,
    value: &Value,
    zone: Option<&str>,
) -> Result<(), String> {
    let ids: Vec<&Value> = match value {
        Value::Array(list) if def.list => list.iter().collect(),
        Value::Array(_) => return Err(format!("{} must be one equipment", def.id)),
        other if def.list => {
            return Err(format!(
                "{} must be a list of equipments, not {}",
                def.id,
                js_string(other)
            ));
        }
        other => vec![other],
    };
    if ids.len() > MAX_SLOT_EQUIPMENTS {
        return Err(format!(
            "{}: at most {MAX_SLOT_EQUIPMENTS} equipments",
            def.id
        ));
    }
    // The equipments of the zone and below, when the slot allows them.
    let below: Vec<String> = match (zone, def.include_descendants && !def.cross_zone) {
        (Some(zone), true) => host.zone_equipment_ids(zone),
        _ => Vec::new(),
    };
    for id in ids {
        let id = id.as_str().unwrap_or("");
        let Some(view) = host.equipment(id) else {
            return Err(format!("{}: equipment not found: {id}", def.id));
        };
        if !def.equipment_types.is_empty() && !def.equipment_types.contains(&view.kind) {
            return Err(format!(
                "{}: {} is not a {}",
                def.id,
                view.name,
                def.equipment_types.join(" or ")
            ));
        }
        let in_zone = match zone {
            None => true,
            Some(_) if def.cross_zone => true,
            Some(zone) if def.include_descendants => {
                below.contains(&view.id) || canonical_id(zone).as_deref() == Some(&view.zone_id)
            }
            Some(zone) => canonical_id(zone).as_deref() == Some(&view.zone_id),
        };
        if !in_zone {
            return Err(format!("{}: {} is not in the zone", def.id, view.name));
        }
    }
    Ok(())
}

/// What one zone or equipment slot holds for an instance: the ids the allow-list lets the script
/// reach (`get`, `order`, `on`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotIds {
    pub name: String,
    pub kind: SlotKind,
    pub list: bool,
    pub ids: Vec<String>,
}

/// The params as the script reads them (`api.params`) and the ids of its zone and equipment
/// slots, from params that passed `validate_params`.
pub fn resolve(defs: &[SlotDef], params: &Value) -> (Value, Vec<SlotIds>) {
    let mut parsed = Map::new();
    let mut slots = Vec::new();
    for def in defs {
        let value = match params.get(&def.id) {
            value if !missing(value) => value.cloned(),
            _ => def.default.clone(),
        };
        let Some(value) = value else { continue };
        let canon = |v: &Value| -> String {
            let text = v.as_str().unwrap_or("");
            canonical_id(text).unwrap_or_else(|| text.to_string())
        };
        let out = match def.kind {
            SlotKind::Duration => parse_duration(&value)
                .map(|ms| json!(ms))
                .unwrap_or(value.clone()),
            SlotKind::Zone => {
                slots.push(SlotIds {
                    name: def.id.clone(),
                    kind: SlotKind::Zone,
                    list: false,
                    ids: vec![canon(&value)],
                });
                json!(canon(&value))
            }
            SlotKind::Equipment => {
                let ids: Vec<String> = match &value {
                    Value::Array(list) => list.iter().map(canon).collect(),
                    other => vec![canon(other)],
                };
                slots.push(SlotIds {
                    name: def.id.clone(),
                    kind: SlotKind::Equipment,
                    list: def.list,
                    ids: ids.clone(),
                });
                if def.list {
                    json!(ids)
                } else {
                    json!(ids.first().cloned().unwrap_or_default())
                }
            }
            _ => value,
        };
        parsed.insert(def.id.clone(), out);
    }
    (Value::Object(parsed), slots)
}

/// The slots the shim is told of: `{"<slot>": {"t": "zone"|"equipment", "l": list, "ids": [..]}}`.
pub fn slots_json(slots: &[SlotIds]) -> String {
    let mut map = Map::new();
    for slot in slots {
        map.insert(
            slot.name.clone(),
            json!({
                "t": if slot.kind == SlotKind::Zone { "zone" } else { "equipment" },
                "l": slot.list,
                "ids": slot.ids,
            }),
        );
    }
    Value::Object(map).to_string()
}
