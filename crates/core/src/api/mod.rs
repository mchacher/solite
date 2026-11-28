//! Sowel's routes for devices, zones, equipments, recipes and settings (`src/api/routes/{devices,
//! zones,equipments,recipes,settings}.ts` of the Sowel repository), and Solite's own routes, as a pure
//! function: a request in, a status and a JSON body out. The host (or the box) only moves bytes.
//! The time and the random bytes come from the caller.

mod auth;
mod devices;
pub mod drain;
mod equipments;
pub(crate) mod input;
mod orders;
mod pairing;
mod recipes;
mod settings;
mod slots;
pub mod static_files;
mod system;
mod zones;

pub use crate::home::{CHUNK_ITEMS, ListCursor};
pub use orders::order_response;
pub use slots::{MAX_WORKERS, SlotGuard, Slots};
pub use static_files::{StaticFile, content_type};
pub(crate) use system::version_response;

use std::sync::Mutex;

use serde_json::{Map, Value};

use crate::backup::BackupTicket;
use crate::devices::{Random, lock, to_json};
use crate::failure::Failure;
use crate::home::Home;
use crate::orders::OrderWait;
use crate::recipes::install::{Op, RecipeTicket};
use crate::supervisor::Mode;
use crate::update::release::ReleaseTicket;

/// Request bodies over this many bytes are refused (413): 32 KB, the longest script, and the 128
/// bytes of its signed file's header (spec 053), so that a signed file of 32 KB of source passes.
pub const MAX_BODY_BYTES: usize = 32 * 1024 + crate::update::HEADER_BYTES;

/// How long the box's HTTP server (one task) waits for the next bytes of a request, in seconds
/// (ESP-IDF's `recv_wait_timeout`). A client that sends its headers and never the body holds that
/// task for this long, and every other request waits behind it. The wait is per receive, an
/// inactivity gap and not a total: 2 s keeps a 2 MiB upload over Wi-Fi working, and stays above
/// the 1 s that curl waits after `Expect: 100-continue` before sending the body. esp-idf-svc
/// hardcodes 5 s.
pub const HTTPD_RECV_WAIT_SECS: u16 = 2;

const PREFIX: &str = "/api/v1";

/// The routes this module serves, as (method, path): a segment starting with `:` is a
/// parameter. The `/ws` upgrade is the host's, not listed. The router is a `match` on path
/// segments, so a route added there and missing here is not caught by a test (the contract check
/// of spec 027 reads this table); every route listed is tested to answer something other than
/// the route-level `Not found`.
pub const ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/v1/devices"),
    ("GET", "/api/v1/devices/:id"),
    ("PUT", "/api/v1/devices/:id"),
    ("DELETE", "/api/v1/devices/:id"),
    ("GET", "/api/v1/zones"),
    ("POST", "/api/v1/zones"),
    ("GET", "/api/v1/zones/aggregation"),
    ("GET", "/api/v1/zones/:id"),
    ("PUT", "/api/v1/zones/:id"),
    ("DELETE", "/api/v1/zones/:id"),
    ("GET", "/api/v1/equipments"),
    ("POST", "/api/v1/equipments"),
    ("GET", "/api/v1/equipments/:id"),
    ("PUT", "/api/v1/equipments/:id"),
    ("DELETE", "/api/v1/equipments/:id"),
    ("POST", "/api/v1/equipments/:id/data-bindings"),
    ("DELETE", "/api/v1/equipments/:id/data-bindings/:bindingId"),
    ("POST", "/api/v1/equipments/:id/order-bindings"),
    ("DELETE", "/api/v1/equipments/:id/order-bindings/:bindingId"),
    ("POST", "/api/v1/equipments/:id/orders/:alias"),
    ("GET", "/api/v1/recipes"),
    ("GET", "/api/v1/recipes/:id"),
    ("GET", "/api/v1/plugins"),
    ("GET", "/api/v1/plugins/store"),
    ("POST", "/api/v1/plugins/store/refresh"),
    ("POST", "/api/v1/plugins/install"),
    ("POST", "/api/v1/plugins/:id/update"),
    ("POST", "/api/v1/plugins/:id/uninstall"),
    ("GET", "/api/v1/recipe-instances"),
    ("POST", "/api/v1/recipe-instances"),
    ("PUT", "/api/v1/recipe-instances/:id"),
    ("DELETE", "/api/v1/recipe-instances/:id"),
    ("POST", "/api/v1/recipe-instances/:id/enable"),
    ("POST", "/api/v1/recipe-instances/:id/disable"),
    ("POST", "/api/v1/recipe-instances/:id/actions"),
    ("GET", "/api/v1/recipe-instances/:id/log"),
    ("GET", "/api/v1/settings"),
    ("PUT", "/api/v1/settings"),
    ("GET", "/api/v1/settings/timezones"),
    ("GET", "/api/v1/system/timezone"),
    ("GET", "/api/v1/integrations/solite/permit-join"),
    ("POST", "/api/v1/integrations/solite/permit-join"),
    ("GET", "/api/v1/auth/status"),
    ("POST", "/api/v1/auth/setup"),
    ("POST", "/api/v1/auth/login"),
    ("POST", "/api/v1/auth/refresh"),
    ("POST", "/api/v1/auth/logout"),
    ("POST", "/api/v1/system/factory-reset"),
    ("GET", "/api/v1/system/version"),
    ("GET", "/api/v1/system/supervisor"),
    ("GET", "/api/v1/system/resources"),
    ("POST", "/api/v1/system/update"),
    ("POST", "/api/v1/system/version/check"),
    ("POST", "/api/v1/system/restart"),
    ("GET", "/api/v1/backup"),
    ("POST", "/api/v1/backup"),
];

/// Who can reach the server: it decides which `Host` headers are accepted (spec 022, D33).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach<'a> {
    /// The Mac host: `Host` is `127.0.0.1:<port>` or `localhost:<port>`.
    Loopback,
    /// The box, on the home's network: private addresses and the box's own name (`solite-<6
    /// hex>`) are accepted.
    Lan { name: &'a str },
}

/// One HTTP request, as the host read it.
#[derive(Debug)]
pub struct Request<'a> {
    pub method: &'a str,
    /// The path, with its query string if any (`GET /equipments` reads `type`).
    pub path: &'a str,
    /// The `Host` header, if any.
    pub host: Option<&'a str>,
    /// The port the server listens on: the only `Host` ports accepted.
    pub port: u16,
    /// Who can reach the server.
    pub reach: Reach<'a>,
    /// The `Content-Type` header, if any.
    pub content_type: Option<&'a str>,
    /// The `Authorization` header, if any (`Bearer <access token>`), read by `handle_guarded`.
    pub authorization: Option<&'a str>,
    /// The declared length of the body (spec 039): `Some(0)` for a request with neither
    /// `Content-Length` nor `Transfer-Encoding` too, `None` for a chunked body. It tells the
    /// update route's two forms apart: an empty body pulls the latest release.
    pub content_length: Option<u64>,
    /// At most `MAX_BODY_BYTES + 1` bytes are needed to see that it is too long.
    pub body: &'a [u8],
}

/// What a request is answered with: a whole response, a list streamed from a cursor (status
/// 200, `application/json; charset=utf-8`), or an order to wait for.
#[derive(Debug)]
pub enum Reply {
    Whole(Response),
    List(ListCursor),
    /// An order put in the home's queue (spec 011): the caller releases the home, then waits for
    /// the radio loop's answer with `OrderWait::wait` and turns it into a response with
    /// `order_response`.
    Order(OrderWait),
    /// A file of the interface (`/`, `/assets/<name>`): the host opens it under its `--ui` folder
    /// and streams it. The path is already checked.
    Static(StaticFile),
    /// A refusal (404, 405) of a path of the interface: a whole response like `Whole`, but the host
    /// adds the interface's security headers to it.
    StaticError(Response),
    /// `POST /system/update` with its gate and content type passed (spec 032, R6): the body is the
    /// signed update file, not read yet. The caller streams it into `Updates::install` (the host
    /// from its server's reader, the box from its socket), the home's lock being released.
    Update,
    /// `POST /system/update` with an empty body (spec 039, R4): pull the latest release. The
    /// caller runs `ReleaseTicket::pull`, the lock being released.
    UpdatePull(ReleaseTicket),
    /// `POST /system/version/check` with its gate passed (spec 039, R2): the caller runs
    /// `ReleaseTicket::check`, the lock being released.
    VersionCheck(ReleaseTicket),
    /// `POST /system/restart` with its gate passed and nothing to interrupt (spec 043, D79): the
    /// caller answers Sowel's 200 `{"success": true, "message": "Restarting"}`, then restarts the box as after an update (the
    /// host cannot: 501).
    Restart,
    /// `GET /backup` with its gate passed (spec 035, R1): the caller reads the header
    /// `X-Backup-Password` and calls `BackupTicket::export`, then streams the archive it returns,
    /// the home's lock being released.
    BackupExport(BackupTicket),
    /// `POST /plugins/install`, `/plugins/:id/update` or `/plugins/:id/uninstall` with its gate
    /// passed (spec 042 R3): the caller runs `RecipeTicket::run` on its job thread, the home's lock
    /// being released. The file of an upload is an ordinary body (at most 32 KB and a header), in the ticket.
    RecipeJob(RecipeTicket),
    /// `POST /plugins/store/refresh` with its gate passed (spec 042 R2.4): the caller runs
    /// `RecipeTicket::run`, as for a job.
    CatalogueRefresh(RecipeTicket),
    /// `POST /backup` with its gate, content type, presence and pairing checks passed: the
    /// body is the archive, not read yet. The caller calls `BackupTicket::restore` with its
    /// reader, the lock being released. `wifi`: `?wifi=true` was given.
    BackupRestore {
        ticket: BackupTicket,
        wifi: bool,
    },
}

/// The answer: a status, and a JSON body (`None` for 204). The content type is
/// `application/json; charset=utf-8` whenever there is a body.
#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Option<String>,
}

pub(crate) fn error(status: u16, message: &str) -> Response {
    Response {
        status,
        body: Some(Value::Object(Map::from_iter([("error".into(), message.into())])).to_string()),
    }
}

pub(crate) fn failure(failure: &Failure) -> Response {
    match failure.code {
        None => error(failure.status, &failure.message),
        Some(code) => Response {
            status: failure.status,
            body: Some(
                Value::Object(Map::from_iter([
                    ("error".into(), code.into()),
                    ("message".into(), failure.message.as_str().into()),
                ]))
                .to_string(),
            ),
        },
    }
}

pub(crate) fn json<T: serde::Serialize>(status: u16, value: &T) -> Response {
    match to_json(value) {
        Some(body) => Response {
            status,
            body: Some(body),
        },
        None => error(500, "Internal error"),
    }
}

pub(crate) fn no_content() -> Response {
    Response {
        status: 204,
        body: None,
    }
}

pub(crate) fn method_not_allowed() -> Response {
    error(405, "Method not allowed")
}

/// The suffixes a home network gives the box's name, besides the bare name.
const NAME_SUFFIXES: [&str; 4] = [".local", ".lan", ".home", ".home.arpa"];

/// True when `host` is an IPv4 address of `10/8`, `172.16/12`, `192.168/16` or `127/8`.
pub(crate) fn private_ipv4(host: &str) -> bool {
    let mut octets = [0u8; 4];
    let mut count = 0;
    for part in host.split('.') {
        if count == 4 || part.is_empty() || part.len() > 3 {
            return false;
        }
        let Ok(octet) = part.parse::<u8>() else {
            return false;
        };
        octets[count] = octet;
        count += 1;
    }
    count == 4
        && match octets {
            [10, ..] | [127, ..] => true,
            [172, second, ..] => (16..=31).contains(&second),
            [192, 168, ..] => true,
            _ => false,
        }
}

/// True when the `Host` header is one the server accepts for this reach. Under `Loopback`:
/// `127.0.0.1:<port>` and `localhost:<port>`, so a page of another origin cannot reach the API
/// through a DNS name that points here. Under `Lan`, without case: a private IPv4 address,
/// `localhost` or the box's name (alone or with a home suffix), the port, when given, being the
/// server's; anything else (a public address, an IPv6 literal, another name) is refused, which
/// also stops DNS rebinding.
pub fn host_allowed(host: Option<&str>, port: u16, reach: Reach) -> bool {
    let Some(host) = host else {
        return false;
    };
    let host = host.trim().to_ascii_lowercase();
    match reach {
        Reach::Loopback => {
            host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
        }
        Reach::Lan { name } => {
            let (name_part, given_port) = match host.split_once(':') {
                Some((name_part, given)) => (name_part, Some(given)),
                None => (host.as_str(), None),
            };
            if given_port.is_some_and(|given| given.parse::<u16>().ok() != Some(port)) {
                return false;
            }
            let name = name.to_ascii_lowercase();
            private_ipv4(name_part)
                || name_part == "localhost"
                || (!name.is_empty()
                    && (name_part == name
                        || NAME_SUFFIXES
                            .iter()
                            .any(|suffix| name_part.strip_suffix(suffix) == Some(name.as_str()))))
        }
    }
}

/// Answers a request; the long lists are drained into one body (tests, tools). The host streams
/// them with `handle_stream`.
pub fn handle(home: &mut Home, request: &Request, now: u64, random: Random) -> Response {
    match dispatch(home, request, now, random, false) {
        Reply::Whole(response) => response,
        // Not reachable: without the streaming handler, an order is refused before it is queued.
        Reply::Order(_) => error(501, ORDERS_NEED_STREAMING),
        // The interface's files are the host's to send.
        Reply::Static(_) => error(404, "Not found"),
        Reply::StaticError(response) => response,
        // Not reachable: the plain handler refuses the route before the gate.
        Reply::Update | Reply::UpdatePull(_) | Reply::VersionCheck(_) => {
            error(501, UPDATES_NEED_STREAMING)
        }
        Reply::Restart => error(501, RESTARTS_NEED_STREAMING),
        Reply::RecipeJob(_) | Reply::CatalogueRefresh(_) => error(501, RECIPES_NEED_STREAMING),
        Reply::BackupExport(_) | Reply::BackupRestore { .. } => error(501, BACKUPS_NEED_STREAMING),
        Reply::List(mut cursor) => {
            let mut body = String::new();
            loop {
                match cursor.next_chunk(home) {
                    Ok(Some(chunk)) => body.push_str(&chunk),
                    Ok(None) => break,
                    Err(_) => return error(500, "Internal error"),
                }
            }
            Response {
                status: 200,
                body: Some(body),
            }
        }
    }
}

/// Answers a request. `now` is the time in milliseconds since 1970 (UTC); `random` gives the
/// bytes of the ids of what is created. After a change, the zone aggregation is brought up to
/// date. `GET /devices` and `GET /equipments` answer a cursor: the caller serves its chunks,
/// taking the home for each one and never holding it while writing.
pub fn handle_stream(home: &mut Home, request: &Request, now: u64, random: Random) -> Reply {
    dispatch(home, request, now, random, true)
}

/// Answers a request behind Sowel's gate (spec 029, R4), taking the home's lock itself, phase by
/// phase: the `Host` check, the interface's files (public), then `/auth/status`, `/setup`, `/login`
/// and `/refresh` (public: the two that hash a password do it with the lock released), and every
/// other path of `/api/v1` after the gate (403 `Setup required`, 401 `Authentication required`,
/// 401 `Invalid or expired token`), before its body is read. A path outside `/api/v1` is 404. The
/// reply is returned with the lock released: serve a list's chunks and wait for an order as with
/// `handle_stream`.
pub fn handle_guarded(home: &Mutex<Home>, request: &Request, now: u64, random: Random) -> Reply {
    if !host_allowed(request.host, request.port, request.reach) {
        return Reply::Whole(error(403, "Forbidden host"));
    }
    if let Some(reply) = static_files::route(request) {
        return reply;
    }
    let path = request.path.split(['?', '#']).next().unwrap_or("");
    let Some(rest) = path
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_prefix('/'))
    else {
        return Reply::Whole(error(404, "Not found"));
    };
    let segments: Vec<&str> = rest.split('/').collect();
    if auth::is_public(&segments) {
        if request.body.len() > MAX_BODY_BYTES {
            return Reply::Whole(error(413, "Body too large"));
        }
        if request.method == "POST" && !json_content_type(request.content_type) {
            return Reply::Whole(error(415, "Unsupported Media Type"));
        }
        return Reply::Whole(auth::route(
            &mut auth::Access::Shared(home),
            &segments,
            request,
            now,
            random,
        ));
    }
    let mut guard = lock(home);
    if let Some(refusal) = auth::gate(&mut guard, request, now) {
        return Reply::Whole(refusal);
    }
    // In safe mode nothing is accepted that would never run or never be saved (spec 036 R5); the
    // restore (`POST /backup`) is refused here, before its body is read.
    if let Some(refusal) = safe_mode_refusal(&guard, request) {
        return Reply::Whole(refusal);
    }
    // The check route reads no body (a body is ignored) and needs no content type (spec 039 R2):
    // answered before the JSON check, the ticket taken here under the lock.
    if is_version_check_path(request.path) {
        if request.method != "POST" {
            return Reply::Whole(method_not_allowed());
        }
        return match guard.release_ticket(true, now) {
            Ok(ticket) => Reply::VersionCheck(ticket),
            Err(e) => Reply::Whole(failure(&e)),
        };
    }
    // The recipe jobs and the refresh (spec 042): the ticket is taken here, under the lock, and the
    // job runs with the lock released.
    if let Some(reply) = recipe_job_reply(&guard, request, now) {
        return reply;
    }
    // The restart reads no body either (spec 043): refused while it would break an update, a
    // check, a backup or a probation, else the caller restarts.
    if is_restart_path(request.path) {
        if request.method != "POST" {
            return Reply::Whole(method_not_allowed());
        }
        return match guard.restart_refusal() {
            Some(message) => Reply::Whole(error(409, message)),
            None => Reply::Restart,
        };
    }
    // The update's body is a file of megabytes, streamed by the caller: no size or JSON check,
    // and the home is let go before the caller reads it. An empty body is the pull (spec 039).
    if is_update_request(request.method, request.path) {
        if request.content_length == Some(0) {
            return match guard.release_ticket(false, now) {
                Ok(ticket) => Reply::UpdatePull(ticket),
                Err(e) => Reply::Whole(failure(&e)),
            };
        }
        return if octet_stream_content_type(request.content_type) {
            Reply::Update
        } else {
            Reply::Whole(error(415, "Unsupported Media Type"))
        };
    }
    // The backup's routes: the gate has passed; the checks that need the home are made here, under
    // the lock, before any body is read (spec 035, R1).
    if is_backup_path(request.path) && matches!(request.method, "GET" | "POST") {
        return backup_reply(&mut guard, request);
    }
    dispatch(&mut guard, request, now, random, true)
}

/// The 503 of the safe mode (spec 036 R5, D58): every request that is not a `GET` but the
/// account's (`/auth/*`), the update's (`/system/update`, `/system/version/check`) and the
/// restart's (`/system/restart`, spec 043, D79), while the engine is stopped. `None`
/// otherwise.
fn safe_mode_refusal(home: &Home, request: &Request) -> Option<Response> {
    if home.supervision().mode == Mode::Normal || request.method == "GET" {
        return None;
    }
    let path = request.path.split(['?', '#']).next().unwrap_or("");
    let rest = path
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_prefix('/'))?;
    let first = rest.split('/').next().unwrap_or("");
    let segments: Vec<&str> = rest.split('/').collect();
    if first == "auth"
        || matches!(segments.as_slice(), ["plugins", _, "uninstall"])
        || rest == "system/update"
        || rest == "system/version/check"
        || rest == "system/restart"
    {
        return None;
    }
    Some(error(503, SAFE_MODE_MESSAGE))
}

/// The body's message of the safe mode's 503.
pub const SAFE_MODE_MESSAGE: &str = "Safe mode: the engine is stopped";

/// True for `/api/v1/backup`, with or without a query.
fn is_backup_path(path: &str) -> bool {
    path.split(['?', '#']).next() == Some("/api/v1/backup")
}

/// True for the two requests the servers must not read the body of before `handle_guarded` has
/// answered, because it is a file they stream: `POST /api/v1/system/update` (spec 032) and `POST
/// /api/v1/backup` (spec 035, moved to Sowel's path by spec 059).
pub fn is_streamed_request(method: &str, path: &str) -> bool {
    is_update_request(method, path) || (method == "POST" && is_backup_path(path))
}

/// The backup routes after the gate (R1), in order: a restore waiting for the restart 409, then
/// for the restore the content type 415, the presence 403 and the pairing window 409, then
/// another backup running 409. The lock is held by the caller and released with the reply.
fn backup_reply(home: &mut Home, request: &Request) -> Reply {
    let restore = request.method == "POST";
    if home.backup_state().is_pending() {
        return Reply::Whole(error(409, "A restore waits for the restart"));
    }
    if restore {
        if !octet_stream_content_type(request.content_type) {
            return Reply::Whole(error(415, "Unsupported Media Type"));
        }
        if !home.presence_confirmed() {
            return Reply::Whole(auth::confirm_required(home.presence_hint()));
        }
        if home.pairing_in_progress() {
            return Reply::Whole(error(409, "Pairing in progress"));
        }
    }
    match home.backup_begin(restore) {
        Err(e) => Reply::Whole(failure(&e)),
        Ok(ticket) if restore => Reply::BackupRestore {
            ticket,
            wifi: wifi_asked(request.path),
        },
        Ok(ticket) => Reply::BackupExport(ticket),
    }
}

/// True when the query has `wifi=true`.
fn wifi_asked(path: &str) -> bool {
    path.split_once('?').is_some_and(|(_, query)| {
        query
            .split('#')
            .next()
            .unwrap_or("")
            .split('&')
            .any(|p| p == "wifi=true")
    })
}

/// True for `/api/v1/system/version/check`, with or without a query.
fn is_version_check_path(path: &str) -> bool {
    path.split(['?', '#']).next() == Some("/api/v1/system/version/check")
}

/// True for `/api/v1/system/restart`, with or without a query.
fn is_restart_path(path: &str) -> bool {
    path.split(['?', '#']).next() == Some("/api/v1/system/restart")
}

/// True for `POST /api/v1/system/update`: the one request whose body the caller must not read
/// before `handle_guarded` has answered, and streams (spec 032, R6).
pub fn is_update_request(method: &str, path: &str) -> bool {
    method == "POST" && path.split(['?', '#']).next() == Some("/api/v1/system/update")
}

/// The interface's answer for the box's server (spec 023 R4.6): `None` when the path is not the
/// interface's (`/`, `/assets/<name>`), left to `handle_stream`; else, in the order of
/// `handle_stream`, 403 for a `Host` the reach refuses, then 405, 404 or the file to send. The
/// box's server calls it on its own task, without the home; the host keeps `handle_stream`.
pub fn static_reply(request: &Request) -> Option<Reply> {
    let reply = static_files::route(request)?;
    if !host_allowed(request.host, request.port, request.reach) {
        return Some(Reply::Whole(error(403, "Forbidden host")));
    }
    Some(reply)
}

/// What the plain handlers answer to the update route: they cannot stream a body.
const UPDATES_NEED_STREAMING: &str = "Updates need the streaming handler";

/// What the plain handlers answer to the restart route: only the streaming handler checks what it
/// would interrupt.
const RESTARTS_NEED_STREAMING: &str = "Restart needs the streaming handler";

/// What the plain handlers answer to the recipe jobs: only the streaming handler takes the ticket.
const RECIPES_NEED_STREAMING: &str = "Recipe installs need the streaming handler";

/// The recipe job or refresh a path names (spec 042, at Sowel's `/plugins` since spec 059): `POST
/// /plugins/install`, `/plugins/store/refresh`, `/plugins/:id/update` and `/plugins/:id/uninstall`,
/// with or without a query.
fn recipe_job_path(path: &str) -> Option<Op> {
    let path = path.split(['?', '#']).next()?;
    let rest = path.strip_prefix(PREFIX)?.strip_prefix('/')?;
    match rest.split('/').collect::<Vec<_>>().as_slice() {
        ["plugins", "install"] => Some(Op::Upload { bytes: Vec::new() }),
        ["plugins", "store", "refresh"] => Some(Op::Refresh),
        ["plugins", id, "update"] => Some(Op::Update {
            id: (*id).to_string(),
        }),
        ["plugins", id, "uninstall"] => Some(Op::Uninstall {
            id: (*id).to_string(),
        }),
        _ => None,
    }
}

/// The reply of a recipe route after the gate (spec 042 R2.4, R3): the ticket under the lock, 405
/// for another method, 413, 415 or 400 for an install's body. `None` for any other path.
fn recipe_job_reply(home: &Home, request: &Request, now: u64) -> Option<Reply> {
    let shape = recipe_job_path(request.path)?;
    if request.method != "POST" {
        return Some(Reply::Whole(method_not_allowed()));
    }
    let op = match shape {
        Op::Upload { .. } => {
            if request.body.len() > MAX_BODY_BYTES {
                return Some(Reply::Whole(error(413, "Body too large")));
            }
            if octet_stream_content_type(request.content_type) {
                Op::Upload {
                    bytes: request.body.to_vec(),
                }
            } else if json_content_type(request.content_type) {
                // Sowel's body: `{repo, confirmed?, expectedSha256?}`; the last two are ignored.
                let repo = serde_json::from_slice::<Value>(request.body)
                    .ok()
                    .and_then(|body| {
                        body.get("repo")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .map(str::to_string)
                    });
                match repo {
                    Some(repo) if crate::recipes::install::catalogue::repo_format(&repo) => {
                        Op::Install { repo }
                    }
                    _ => {
                        return Some(Reply::Whole(error(
                            400,
                            "Invalid 'repo' field (expected owner/repo)",
                        )));
                    }
                }
            } else {
                return Some(Reply::Whole(error(415, "Unsupported Media Type")));
            }
        }
        other => other,
    };
    #[cfg(feature = "script-recipes")]
    {
        let refresh = op == Op::Refresh;
        Some(match home.recipe_ticket(op, now) {
            Ok(ticket) if refresh => Reply::CatalogueRefresh(ticket),
            Ok(ticket) => Reply::RecipeJob(ticket),
            Err(e) => Reply::Whole(failure(&e)),
        })
    }
    #[cfg(not(feature = "script-recipes"))]
    {
        let _ = (home, op, now);
        Some(Reply::Whole(error(501, "Script recipes are not built in")))
    }
}

/// What the plain handlers answer to the backup routes: they cannot stream.
const BACKUPS_NEED_STREAMING: &str = "Backups need the streaming handler";

/// True when the body is declared `application/octet-stream` (parameters allowed).
fn octet_stream_content_type(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        v.split(';')
            .next()
            .is_some_and(|t| t.trim().eq_ignore_ascii_case(crate::update::CONTENT_TYPE))
    })
}

/// What the plain `handle` answers to an order: it cannot wait for the radio, so it does not
/// queue the order.
const ORDERS_NEED_STREAMING: &str = "Orders need the streaming handler";

/// True when the body of a POST or PUT is declared as JSON (`application/json`, parameters such
/// as `charset` allowed), the only type Fastify's JSON parser takes. A request of another type
/// cannot come from a cross-origin "simple" form POST.
fn json_content_type(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        v.split(';')
            .next()
            .is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json"))
    })
}

fn dispatch(
    home: &mut Home,
    request: &Request,
    now: u64,
    random: Random,
    streaming: bool,
) -> Reply {
    if !host_allowed(request.host, request.port, request.reach) {
        return Reply::Whole(error(403, "Forbidden host"));
    }
    // The interface's files come before the API's checks: they take no body and no JSON.
    if let Some(reply) = static_files::route(request) {
        return reply;
    }
    if let Some(refusal) = safe_mode_refusal(home, request) {
        return Reply::Whole(refusal);
    }
    // The update route is the streaming handler's: refused here, before the size and JSON checks.
    if is_update_request(request.method, request.path) {
        return Reply::Whole(error(501, UPDATES_NEED_STREAMING));
    }
    if is_version_check_path(request.path) {
        return Reply::Whole(if request.method == "POST" {
            error(501, UPDATES_NEED_STREAMING)
        } else {
            method_not_allowed()
        });
    }
    if is_restart_path(request.path) {
        return Reply::Whole(if request.method == "POST" {
            error(501, RESTARTS_NEED_STREAMING)
        } else {
            method_not_allowed()
        });
    }
    if recipe_job_path(request.path).is_some() {
        return Reply::Whole(if request.method == "POST" {
            error(501, RECIPES_NEED_STREAMING)
        } else {
            method_not_allowed()
        });
    }
    if is_backup_path(request.path) && matches!(request.method, "GET" | "POST") {
        return Reply::Whole(error(501, BACKUPS_NEED_STREAMING));
    }
    if request.body.len() > MAX_BODY_BYTES {
        return Reply::Whole(error(413, "Body too large"));
    }
    // Every POST and PUT of the API takes a JSON body (the only routes without one are GET and
    // DELETE).
    if matches!(request.method, "POST" | "PUT") && !json_content_type(request.content_type) {
        return Reply::Whole(error(415, "Unsupported Media Type"));
    }
    let (path, query) = match request.path.split_once('?') {
        Some((path, query)) => (path, query),
        None => (request.path, ""),
    };
    let path = path.split('#').next().unwrap_or("");
    let Some(rest) = path
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_prefix('/'))
    else {
        return Reply::Whole(error(404, "Not found"));
    };
    let segments: Vec<&str> = rest.split('/').collect();
    // The clock may have been set since the radio loop's last turn: the devices' dates move
    // first (D45), so a device heard during the wait is not seen offline.
    home.observe_wall(now);
    // Silence alone can turn a device offline: the aggregation follows.
    if home.devices.refresh_status(now) {
        home.recompute(now);
    }
    if request.method == "GET" {
        match segments.as_slice() {
            ["devices"] => return Reply::List(ListCursor::devices(home, now)),
            ["equipments"] => {
                return Reply::List(ListCursor::equipments(
                    home,
                    equipments::type_filter(query),
                    now,
                ));
            }
            ["recipe-instances"] => return Reply::List(ListCursor::recipe_instances(home)),
            _ => {}
        }
    }
    if let ["equipments", id, "orders", alias] = segments.as_slice() {
        if request.method == "POST" {
            return orders::route(home, id, alias, request.body, now, streaming);
        }
        return Reply::Whole(method_not_allowed());
    }
    let response = match segments.first().copied() {
        Some("devices") => devices::route(home, &segments, request, now),
        Some("zones") => zones::route(home, &segments, request, now, random),
        Some("equipments") => equipments::route(home, &segments, request, now, random),
        Some("integrations") => pairing::route(home, &segments, request),
        Some("recipes" | "recipe-instances") => {
            recipes::route(home, &segments, request, now, random)
        }
        Some("plugins") => recipes::plugins(home, &segments, request),
        Some("backup") if segments.len() == 1 => method_not_allowed(),
        Some("settings") => settings::route(home, &segments, request),
        Some("auth") => auth::route(
            &mut auth::Access::Direct(home),
            &segments,
            request,
            now,
            random,
        ),
        Some("system") => system::route(home, &segments, request, now),
        _ => error(404, "Not found"),
    };
    if request.method != "GET" && response.status < 300 {
        home.recompute(now);
    }
    Reply::Whole(response)
}
