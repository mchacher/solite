//! `/recipes`, `/recipe-instances` and `/plugins` (Sowel's `src/api/routes/recipes.ts`). Each route answers at
//! once: an order a recipe sends is not waited for. `GET /recipe-instances` is streamed by the
//! caller (`ListCursor::recipe_instances`).

use serde_json::{Map, Value};

use super::{Request, Response, error, failure, json, method_not_allowed, no_content};
use crate::devices::Random;
use crate::home::Home;
use crate::recipes::MAX_LOG_ENTRIES;

/// The text of a body that is not what the route takes (Sowel's Fastify says more; the text is
/// not part of the contract).
const INVALID_BODY: &str = "Invalid body";

/// The body as a JSON object.
fn object(body: &[u8]) -> Option<Map<String, Value>> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(fields)) => Some(fields),
        _ => None,
    }
}

/// `limit` of the query: a positive whole number, at most `MAX_LOG_ENTRIES`; anything else is
/// `MAX_LOG_ENTRIES`.
fn limit_of(query: &str) -> usize {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("limit="))
        .and_then(|text| text.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .map_or(MAX_LOG_ENTRIES, |n| n.min(MAX_LOG_ENTRIES))
}

pub fn route(
    home: &mut Home,
    segments: &[&str],
    request: &Request,
    now: u64,
    random: Random,
) -> Response {
    let query = request.path.split_once('?').map_or("", |(_, query)| query);
    match (segments, request.method) {
        (["recipes"], "GET") => json(200, &home.recipes().recipe_infos()),
        (["recipes"], _) => method_not_allowed(),
        (["recipes", id], "GET") => match home.recipes().recipe_info(id) {
            Some(info) => json(200, &info),
            None => error(404, "Recipe not found"),
        },
        (["recipes", _], _) => method_not_allowed(),
        (["recipe-instances"], "POST") => create(home, request.body, now, random),
        (["recipe-instances"], _) => method_not_allowed(),
        (["recipe-instances", id], "PUT") => update(home, id, request.body, now),
        (["recipe-instances", id], "DELETE") => match home.delete_recipe_instance(id) {
            Ok(()) => no_content(),
            Err(e) => failure(&e),
        },
        (["recipe-instances", _], _) => method_not_allowed(),
        (["recipe-instances", id, "enable"], "POST") => set_enabled(home, id, true, now),
        (["recipe-instances", id, "disable"], "POST") => set_enabled(home, id, false, now),
        (["recipe-instances", id, "actions"], "POST") => action(home, id, request.body),
        (["recipe-instances", id, "log"], "GET") => {
            json(200, &home.recipes().log(id, limit_of(query)))
        }
        (
            [
                "recipe-instances",
                _,
                "enable" | "disable" | "actions" | "log",
            ],
            _,
        ) => method_not_allowed(),
        _ => error(404, "Not found"),
    }
}

/// `/plugins` (Sowel's `src/api/routes/plugins.ts`, the recipe part): the installed recipes and
/// the store. The `POST` routes are the streaming handler's (`Reply::RecipeJob`,
/// `Reply::CatalogueRefresh`): only their other methods are answered here.
pub fn plugins(home: &mut Home, segments: &[&str], request: &Request) -> Response {
    match (segments, request.method) {
        (["plugins"], "GET") => json(200, &home.installed_plugins()),
        (["plugins", "store"], "GET") => json(200, &home.store_entries()),
        (["plugins"] | ["plugins", "store"], _) => method_not_allowed(),
        _ => error(404, "Not found"),
    }
}

fn create(home: &mut Home, body: &[u8], now: u64, random: Random) -> Response {
    let Some(mut fields) = object(body) else {
        return error(400, INVALID_BODY);
    };
    let recipe_id = match fields.get("recipeId") {
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        _ => return error(400, INVALID_BODY),
    };
    let Some(params @ Value::Object(_)) = fields.remove("params") else {
        return error(400, INVALID_BODY);
    };
    match home.create_recipe_instance(&recipe_id, params, now, random) {
        Ok(instance) => json(201, &instance),
        Err(e) => failure(&e),
    }
}

fn update(home: &mut Home, id: &str, body: &[u8], now: u64) -> Response {
    let Some(mut fields) = object(body) else {
        return error(400, INVALID_BODY);
    };
    let Some(params @ Value::Object(_)) = fields.remove("params") else {
        return error(400, INVALID_BODY);
    };
    match home.update_recipe_instance(id, params, now) {
        Ok(instance) => json(200, &instance),
        Err(e) => failure(&e),
    }
}

fn set_enabled(home: &mut Home, id: &str, enabled: bool, now: u64) -> Response {
    match home.set_recipe_enabled(id, enabled, now) {
        Ok(()) => json(200, &serde_json::json!({ "success": true })),
        Err(e) => failure(&e),
    }
}

fn action(home: &mut Home, id: &str, body: &[u8]) -> Response {
    let Some(fields) = object(body) else {
        return error(400, INVALID_BODY);
    };
    match fields.get("action") {
        Some(Value::String(text)) if !text.is_empty() => {}
        _ => return error(400, INVALID_BODY),
    }
    if fields.get("payload").is_some_and(|p| !p.is_object()) {
        return error(400, INVALID_BODY);
    }
    match home.recipes().action(id) {
        Ok(()) => json(200, &serde_json::json!({ "ok": true })),
        Err(e) => failure(&e),
    }
}
