//! `/system/timezone` (Sowel's `src/api/routes/system.ts`): the home's time zone as
//! `{tz, source, offsetHours}`; `GET /system/version`: Sowel's `VersionInfo` (spec 032, a subset:
//! no Docker fields, `latest` from the last check's cache, spec 039). `POST /system/version/check`
//! and `POST /system/update` are the streaming handler's
//! (`Reply::VersionCheck`, `Reply::Update`, `Reply::UpdatePull`, spec 032 and 039, D53, D65), only their other
//! methods are answered here (`/backup` is the streaming handler's too, spec 035). Sowel's other
//! `/system` routes (mode...) are not Solite's. `POST /system/factory-reset` is Solite's (spec 029,
//! D48). `GET /system/supervisor` is Solite's too (spec 036, D58): the supervisor's status.

use serde::Serialize;
use serde_json::{Number, Value};

use super::auth::{confirm_required, empty_body};
use super::{Request, Response, error, failure, json, method_not_allowed, no_content};
use crate::home::Home;
use crate::settings;
use crate::update::Version;
use crate::update::release::Releases;

pub fn route(home: &mut Home, segments: &[&str], request: &Request, now: u64) -> Response {
    match (segments, request.method) {
        (["system", "timezone"], "GET") => timezone(home, now),
        (["system", "timezone"], _) => method_not_allowed(),
        (["system", "factory-reset"], "POST") => factory_reset(home, request.body, now),
        (["system", "factory-reset"], _) => method_not_allowed(),
        (["system", "version"], "GET") => version_response(&home.releases()),
        (["system", "supervisor"], "GET") => json(200, home.supervision()),
        // The memories, the storage and the script recipes (spec 042 R5): Solite's own.
        (["system", "resources"], "GET") => json(200, &home.resources()),
        (
            ["system", "version"]
            | ["system", "version", "check"]
            | ["system", "update"]
            | ["system", "supervisor"]
            | ["system", "resources"],
            _,
        ) => method_not_allowed(),
        _ => error(404, "Not found"),
    }
}

/// `POST /system/factory-reset` (R7): an empty body or `{}`, then the presence (403 with its
/// hint), then the reset (409 while pairing runs), then 204.
fn factory_reset(home: &mut Home, body: &[u8], now: u64) -> Response {
    if let Err(e) = empty_body(body) {
        return failure(&e);
    }
    if !home.presence_confirmed() {
        return confirm_required(home.presence_hint());
    }
    match home.factory_reset(now) {
        Ok(()) => no_content(),
        Err(e) => failure(&e),
    }
}

/// Sowel's `VersionInfo` without its Docker fields, its keys in Sowel's order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionInfo {
    current: &'static str,
    latest: Option<String>,
    update_available: bool,
    release_url: Option<String>,
}

/// The running version and what the last check found (the cache; `null`/`false` before any).
pub(crate) fn version_response(releases: &Releases) -> Response {
    let running = Version::parse(crate::VERSION).unwrap_or(Version::new(0, 0, 0));
    let info = releases.info(running);
    json(
        200,
        &VersionInfo {
            current: crate::VERSION,
            latest: info.latest,
            update_available: info.update_available,
            release_url: info.release_url,
        },
    )
}

/// Sowel's `{tz, source, offsetHours}`, its keys in Sowel's order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Info<'a> {
    tz: &'a str,
    source: &'static str,
    offset_hours: Value,
}

/// With `home.timezone` set: the setting as `tz`, `source` `env` (the setting is the box's `TZ`,
/// decision D44), the offset from UTC now; unset: Sowel's fallback without a `TZ` and a
/// location, `UTC` (Sowel derives `auto` from the location, which the box has no room for).
fn timezone(home: &Home, now: u64) -> Response {
    let set = home
        .settings()
        .get(settings::TIMEZONE)
        .and_then(|name| settings::tz::find(name).map(|zone| (name, zone)));
    let info = match set {
        Some((name, zone)) => {
            let seconds = zone.offset_at(i64::try_from(now).unwrap_or(i64::MAX));
            Info {
                tz: name,
                source: "env",
                offset_hours: hours(seconds),
            }
        }
        None => Info {
            tz: "UTC",
            source: "fallback",
            offset_hours: Value::from(0),
        },
    };
    json(200, &info)
}

/// Seconds as hours: a whole number when it is one (`1`, `-3`), else a fraction (`5.5`).
fn hours(seconds: i64) -> Value {
    if seconds % 3600 == 0 {
        return Value::from(seconds / 3600);
    }
    // At most a few thousand seconds: exact in an `f64`.
    #[allow(clippy::cast_precision_loss)]
    Number::from_f64(seconds as f64 / 3600.0).map_or(Value::Null, Value::Number)
}
