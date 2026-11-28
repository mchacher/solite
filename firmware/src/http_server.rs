//! The radio image's HTTP server (feature `radio`, spec 023 R4): ESP-IDF's server on port 80, on
//! core 0. `/ws` is `live_ws.rs`'s; `/` and `/assets/*` are answered on the server's task from the
//! interface embedded in the flash, as the core's `static_reply` decides; every other request is
//! handed, as an ESP-IDF asynchronous request, to one of 4 worker threads (stacks and body buffers
//! in PSRAM), which asks the core's `handle_guarded` exactly as the Mac host does
//! (`crates/host/src/serve.rs`), so an order waiting for the radio never holds the server's task.
//! The core decides every answer; this file moves the bytes. The full backup's two routes read or
//! write the flash, which a stack in PSRAM forbids, and the update's signature check needs more
//! stack than a worker has: their answer runs on a thread started for the job (`job_thread.rs`),
//! its stack in internal RAM, while the worker waits for it. The handler of every request that is not a
//! `GET` (it can start or call a script recipe, spec 041) runs on the `script` thread, made once at
//! boot, its 32 KB stack in PSRAM (nothing it runs touches the flash, D83), one request at a time: a
//! worker's 10 KB cannot hold the script's 16 KB stack.

use std::ffi::{CStr, CString};
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::task::thread::{MallocCap, ThreadSpawnConfiguration};
use esp_idf_svc::sys::{
    ESP_FAIL, ESP_OK, HTTPD_SOCK_ERR_TIMEOUT, MALLOC_CAP_8BIT, MALLOC_CAP_INTERNAL,
    MALLOC_CAP_SPIRAM, esp_err_t, heap_caps_malloc, http_method_HTTP_DELETE, http_method_HTTP_GET,
    http_method_HTTP_HEAD, http_method_HTTP_PATCH, http_method_HTTP_POST, http_method_HTTP_PUT,
    httpd_config_t, httpd_handle_t, httpd_register_uri_handler, httpd_req_async_handler_begin,
    httpd_req_async_handler_complete, httpd_req_get_hdr_value_len, httpd_req_get_hdr_value_str,
    httpd_req_recv, httpd_req_t, httpd_req_to_sockfd, httpd_resp_send, httpd_resp_send_chunk,
    httpd_resp_set_hdr, httpd_resp_set_status, httpd_resp_set_type, httpd_sess_trigger_close,
    httpd_start, httpd_stop, httpd_uri_match_wildcard, httpd_uri_t,
};
use solite_core::api::drain::{DRAIN_LIMIT_MS, drain_body};
use solite_core::api::static_files::{STATIC_HEADERS, cache_control, find};
use solite_core::api::{
    HTTPD_RECV_WAIT_SECS, MAX_BODY_BYTES, MAX_WORKERS, Reach, Reply, Request, Response, SlotGuard,
    Slots, handle_guarded, is_streamed_request, is_update_request, order_response, static_reply,
};
use solite_core::backup::{BackupTicket, CHUNK_BYTES};
use solite_core::devices::lock;
use solite_core::home::Home;
use solite_core::recipes::install::{JobEnv, RecipeTicket, ScriptThread};
use solite_core::update::MAX_UPLOAD_BYTES;
use solite_core::update::release::{Limits, ReleaseTicket};

use crate::backup_files::{BoxPacing, data_files};
use crate::fetch::EspFetcher;
use crate::job_thread::{
    self, BACKUP_STACK_BYTES, FETCH_STACK_BYTES, StackPlace, UPDATE_STACK_BYTES, stack_unused,
};
use crate::live_ws;
use crate::ota;
use crate::radio_engine::{Shared, fill, wall_ms};
use crate::scripts::SCRIPT_THREAD_STACK_BYTES;
use crate::storage::FatStorage;
use crate::ui_files::UI_FILES;

/// Each worker's stack, in PSRAM (R9.4): the workers never touch the flash (a read neither: it
/// turns the caches off, which ESP-IDF refuses from a stack in PSRAM), and never run the update's
/// signature check (9 KB of stack alone, D60).
pub const WORKER_STACK_BYTES: usize = 10 * 1024;
/// Each worker's body buffer, in PSRAM (R4.4): one byte more than a body may hold.
const BODY_BUFFER_BYTES: usize = MAX_BODY_BYTES + 1;
/// The server's task stack, in internal RAM.
const SERVER_STACK_BYTES: usize = 8 * 1024;
/// At most this many connections; the least recently used one is closed beyond (R4.2).
const MAX_OPEN_SOCKETS: usize = 12;
/// The longest header value read (the request head is at most 4096 bytes, `sdkconfig.radio.defaults`).
const MAX_HEADER_BYTES: usize = 4096;
/// The content type of every JSON answer.
const JSON: &str = "application/json; charset=utf-8";
const BUSY: &str = r#"{"error":"Busy"}"#;

/// What the server's handlers and the workers share.
struct Api {
    home: Arc<Mutex<Home>>,
    name: &'static str,
    port: u16,
    slots: Slots,
    queue: SyncSender<Job>,
}

static API: OnceLock<Api> = OnceLock::new();
/// Set by the first restart asked through the API (spec 043): one restart, whatever the requests.
static RESTARTING: AtomicBool = AtomicBool::new(false);

/// Held by the worker whose request runs on the `script` thread: one request at a time.
static SCRIPT_THREAD: Mutex<()> = Mutex::new(());

/// A request handed to the `script` thread: owned copies of what `Request` borrows (the worker's
/// body buffer stays with the worker), and where the thread sends the reply.
struct ScriptJob {
    method: String,
    path: String,
    host: Option<String>,
    content_type: Option<String>,
    authorization: Option<String>,
    content_length: Option<u64>,
    body: Vec<u8>,
    reply: SyncSender<Reply>,
}

/// A step of a recipe job (spec 042): a closure the `recipe` thread lends to the `script` thread, which
/// runs it and says so on `done`. Only the `recipe` thread's `on_script_thread` makes one.
struct CallJob {
    call: CallPointer,
    done: SyncSender<()>,
}

/// The address of the lent closure, for the thread that runs it.
struct CallPointer(*mut (dyn FnMut() + 'static));

// SAFETY: the pointer is used by the `script` thread only, once, while the lending thread waits
// for `done` (see `on_script_thread`): the closure it stands for outlives that use.
#[allow(unsafe_code)]
unsafe impl Send for CallPointer {}

impl CallPointer {
    /// Calls the closure (a method, so that a closure moves the whole pointer, not its field).
    #[allow(unsafe_code)]
    fn invoke(self) {
        // SAFETY: see above; called once.
        unsafe { (*self.0)() }
    }
}

/// What the `script` thread is given: a request to route, or a step of a recipe job.
enum Work {
    Request(ScriptJob),
    Call(CallJob),
}

/// The way to the `script` thread, set once it runs.
static SCRIPT_JOBS: OnceLock<SyncSender<Work>> = OnceLock::new();

/// The `script` thread's loop: one request at a time, as the router would answer it on a worker.
/// The firmware is built with `panic = "abort"`: a panic reboots the chip (the supervisor reads
/// it) instead of ending this thread, so the loop needs no `catch_unwind` and no restart; the
/// thread ends only if its channel closes, which nothing does (`run_on_script_thread` then
/// answers 503).
fn script_loop(jobs: Receiver<Work>) {
    let Some(api) = API.get() else {
        return;
    };
    while let Ok(work) = jobs.recv() {
        let job = match work {
            Work::Request(job) => job,
            // A step of a recipe job: a script's load, swap or unload, which needs this stack.
            Work::Call(CallJob { call, done }) => {
                call.invoke();
                let _ = done.send(());
                continue;
            }
        };
        let request = Request {
            method: &job.method,
            path: &job.path,
            host: job.host.as_deref(),
            port: api.port,
            reach: Reach::Lan { name: api.name },
            content_type: job.content_type.as_deref(),
            authorization: job.authorization.as_deref(),
            content_length: job.content_length,
            body: &job.body,
        };
        let mut random = |bytes: &mut [u8]| fill(bytes);
        let reply = handle_guarded(&api.home, &request, wall_ms(), &mut random);
        // A worker that gave up waiting has dropped its end: nothing to tell.
        let _ = job.reply.send(reply);
    }
}

/// Starts the `script` thread, its stack in PSRAM (`job_thread::spawn`, spec 045 R1), for the
/// whole run. In PSRAM because nothing it runs touches the flash, a read neither (D59, and
/// `storage.rs` refuses such a call from a PSRAM stack).
fn start_script_thread() -> Result<(), String> {
    let (jobs, wait) = sync_channel::<Work>(0);
    job_thread::spawn(
        c"script",
        SCRIPT_THREAD_STACK_BYTES,
        StackPlace::Psram,
        move || script_loop(wait),
    )?;
    SCRIPT_JOBS
        .set(jobs)
        .map_err(|_| "the script thread is already started".to_string())
}

/// A recipe job or a catalogue refresh handed to the `recipe` thread: the request it answers on
/// (lent: the worker waits for `done`), the core's ticket, the home, and where to say it is over.
struct RecipeWork {
    raw: *const Raw,
    ticket: RecipeTicket,
    home: Arc<Mutex<Home>>,
    done: SyncSender<()>,
}

// SAFETY: the worker that owns the request lends it to the `recipe` thread and waits for `done`
// before it uses the request again (`run_on_recipe_thread`): one task at a time uses it, as
// ESP-IDF allows for an asynchronous request.
#[allow(unsafe_code)]
unsafe impl Send for RecipeWork {}

/// The way to the permanent `recipe` thread, set once it runs.
static RECIPE_JOBS: OnceLock<SyncSender<RecipeWork>> = OnceLock::new();

/// Held by the worker whose job runs on the `recipe` thread: one job at a time (the core's own
/// one-job rule answers 409 before this).
static RECIPE_THREAD: Mutex<()> = Mutex::new(());

/// The `recipe` thread's loop: one job at a time. The firmware is built with `panic = "abort"`,
/// as for `script_loop`: no `catch_unwind`; the thread ends only if its channel closes.
fn recipe_loop(jobs: Receiver<RecipeWork>) {
    while let Ok(work) = jobs.recv() {
        let RecipeWork {
            raw,
            ticket,
            home,
            done,
        } = work;
        // SAFETY: the worker lent the request and waits for `done` below.
        #[allow(unsafe_code)]
        let raw = unsafe { &*raw };
        answer_recipe(raw, ticket, &home);
        log::info!(
            "recipe: thread stack {} of {FETCH_STACK_BYTES} bytes never used",
            stack_unused()
        );
        let _ = done.send(());
    }
}

/// Starts the `recipe` thread (`FETCH_STACK_BYTES`, internal RAM, core 0) once, for the whole run
/// (spec 045 R3): the recipe jobs no longer need a whole block of internal RAM at the time of the
/// job. Called before the installed scripts are loaded (`scripts::load_installed`), which tells the
/// core's admission whether it runs; calling it again does nothing. When it cannot start, logged:
/// every recipe job then answers 503 `Busy`.
pub fn start_recipe_thread() -> bool {
    if RECIPE_JOBS.get().is_some() {
        return true;
    }
    let (jobs, wait) = sync_channel::<RecipeWork>(0);
    let started = job_thread::spawn(
        c"recipe",
        FETCH_STACK_BYTES,
        StackPlace::Internal,
        move || recipe_loop(wait),
    );
    match started {
        Ok(()) => RECIPE_JOBS.set(jobs).is_ok(),
        Err(e) => {
            log::error!("recipe: {e}, recipe jobs answer 503");
            false
        }
    }
}

/// Runs a recipe job on the `recipe` thread and waits for it. `Err` when the thread is not there
/// (it could not start, or it ended): the job did not run.
fn run_on_recipe_thread(
    raw: &Raw,
    ticket: RecipeTicket,
    home: Arc<Mutex<Home>>,
) -> Result<Option<()>, String> {
    let jobs = RECIPE_JOBS
        .get()
        .ok_or_else(|| "recipe: its thread is not running".to_string())?;
    let _one = RECIPE_THREAD.lock().unwrap_or_else(PoisonError::into_inner);
    let (done, wait) = sync_channel(1);
    let work = RecipeWork {
        raw: std::ptr::from_ref(raw),
        ticket,
        home,
        done,
    };
    jobs.send(work)
        .map_err(|_| "recipe: its thread ended".to_string())?;
    Ok(wait.recv().ok())
}

/// Runs the request on the `script` thread and waits for its reply (one at a time). `None` when
/// the thread is not there (it could not start, or it ended).
fn run_on_script_thread(request: &Request<'_>) -> Option<Reply> {
    let jobs = SCRIPT_JOBS.get()?;
    let _one = SCRIPT_THREAD.lock().unwrap_or_else(PoisonError::into_inner);
    let (reply, wait) = sync_channel(1);
    let job = ScriptJob {
        method: request.method.to_string(),
        path: request.path.to_string(),
        host: request.host.map(str::to_string),
        content_type: request.content_type.map(str::to_string),
        authorization: request.authorization.map(str::to_string),
        content_length: request.content_length,
        body: request.body.to_vec(),
        reply,
    };
    jobs.send(Work::Request(job)).ok()?;
    wait.recv().ok()
}

/// Runs `job` on the `script` thread and waits for it (one at a time with the requests): the
/// recipe job's loads and unloads, whose JavaScript stack the `recipe` thread's 20 KB cannot hold.
/// When the thread is not there, `job` does not run (the core answers 500).
#[allow(unsafe_code)]
fn on_script_thread(job: &mut dyn FnMut()) {
    let Some(jobs) = SCRIPT_JOBS.get() else {
        return;
    };
    let _one = SCRIPT_THREAD.lock().unwrap_or_else(PoisonError::into_inner);
    let (done, wait) = sync_channel(1);
    // SAFETY: only the lifetime of the trait object is erased: this function waits for `done`
    // before it returns, so the closure outlives the `script` thread's one call of it.
    let call = CallPointer(unsafe {
        std::mem::transmute::<&mut dyn FnMut(), *mut (dyn FnMut() + 'static)>(job)
    });
    if jobs.send(Work::Call(CallJob { call, done })).is_ok() {
        let _ = wait.recv();
    }
}

/// The steps of a recipe job that run scripts, on the box.
struct BoxScriptThread;

impl ScriptThread for BoxScriptThread {
    fn run(&self, job: &mut dyn FnMut()) {
        on_script_thread(job);
    }
}

/// The smallest unused stack of each worker since it started, in bytes (`heap:` line, R9.2).
static WORKER_STACK_UNUSED: [AtomicUsize; MAX_WORKERS] =
    [const { AtomicUsize::new(usize::MAX) }; MAX_WORKERS];

/// The smallest high-water mark among the workers and their stack, once the server runs.
pub fn http_stack() -> Option<(usize, usize)> {
    let least = WORKER_STACK_UNUSED
        .iter()
        .map(|unused| unused.load(Ordering::Relaxed))
        .min()
        .unwrap_or(usize::MAX);
    (least != usize::MAX).then_some((least, WORKER_STACK_BYTES))
}

/// The running server; dropping it stops the server (it is kept for the whole run).
pub struct Server {
    handle: httpd_handle_t,
}

// SAFETY: the handle is ESP-IDF's, used from any task by its own API (as `register` does).
#[allow(unsafe_code)]
unsafe impl Send for Server {}

impl Drop for Server {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful `httpd_start` and is stopped once.
        unsafe { httpd_stop(self.handle) };
    }
}

/// One request of ESP-IDF's server: valid while the handler that got it runs, or, for an
/// asynchronous copy, until `complete`.
struct Raw(*mut httpd_req_t);

// SAFETY: an asynchronous copy of a request (`httpd_req_async_handler_begin`) belongs to no task:
// ESP-IDF lets any task answer it until `httpd_req_async_handler_complete`; it is used by one
// worker at a time (it moves with its job).
#[allow(unsafe_code)]
unsafe impl Send for Raw {}

/// What the server's task read of the request before handing it over.
struct Head {
    method: &'static str,
    uri: String,
    host: Option<String>,
    content_type: Option<String>,
    /// The `Authorization` header, whole (the token goes in it, spec 029).
    authorization: Option<String>,
    chunked: bool,
    length: usize,
}

/// A request handed to a worker, with its slot (freed when the job ends).
struct Job {
    raw: Raw,
    head: Head,
    _slot: SlotGuard,
}

impl Raw {
    #[allow(unsafe_code, non_upper_case_globals)]
    fn method(&self) -> &'static str {
        // SAFETY: the request is valid (see `Raw`); reads one field.
        let method = unsafe { (*self.0).method } as u32;
        match method {
            http_method_HTTP_GET => "GET",
            http_method_HTTP_POST => "POST",
            http_method_HTTP_PUT => "PUT",
            http_method_HTTP_DELETE => "DELETE",
            http_method_HTTP_PATCH => "PATCH",
            http_method_HTTP_HEAD => "HEAD",
            _ => "OTHER",
        }
    }

    #[allow(unsafe_code)]
    fn uri(&self) -> String {
        // SAFETY: the request is valid; its `uri` field is a NUL-terminated string that ESP-IDF
        // keeps for the request's life, copied here.
        unsafe { CStr::from_ptr((*self.0).uri.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    #[allow(unsafe_code)]
    fn content_length(&self) -> usize {
        // SAFETY: the request is valid; reads one field.
        unsafe { (*self.0).content_len }
    }

    /// A header's value, at most `MAX_HEADER_BYTES`; `None` when absent.
    #[allow(unsafe_code)]
    fn header(&self, name: &CStr) -> Option<String> {
        // SAFETY: the request is valid; `name` is NUL-terminated; ESP-IDF writes at most
        // `buffer.len()` bytes, NUL included, into `buffer`, which outlives the call.
        unsafe {
            let length = httpd_req_get_hdr_value_len(self.0, name.as_ptr());
            if length == 0 || length > MAX_HEADER_BYTES {
                return None;
            }
            let mut buffer = vec![0u8; length + 1];
            if httpd_req_get_hdr_value_str(
                self.0,
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            ) != ESP_OK
            {
                return None;
            }
            buffer.truncate(length);
            Some(String::from_utf8_lossy(&buffer).into_owned())
        }
    }

    /// Reads body bytes: their number, or ESP-IDF's error (`HTTPD_SOCK_ERR_TIMEOUT` after
    /// `HTTPD_RECV_WAIT_SECS` without a byte).
    #[allow(unsafe_code)]
    fn receive(&self, buffer: &mut [u8]) -> i32 {
        // SAFETY: the request is valid; ESP-IDF writes at most `buffer.len()` bytes into it.
        unsafe { httpd_req_recv(self.0, buffer.as_mut_ptr().cast(), buffer.len()) }
    }

    /// Sets the status line and the headers; the strings are returned, to be kept alive until the
    /// answer is sent (ESP-IDF keeps their pointers).
    #[allow(unsafe_code)]
    fn set_head(
        &self,
        status: u16,
        content_type: &str,
        headers: &[(&str, &str)],
    ) -> Result<Vec<CString>, String> {
        let c = |text: &str| CString::new(text).map_err(|e| e.to_string());
        let mut kept = vec![
            c(&format!("{status} {}", reason(status)))?,
            c(content_type)?,
        ];
        // SAFETY: the request is valid; the strings are NUL-terminated and kept by the caller
        // until the answer is sent.
        unsafe {
            httpd_resp_set_status(self.0, kept[0].as_ptr());
            httpd_resp_set_type(self.0, kept[1].as_ptr());
        }
        for (name, value) in headers {
            let (name, value) = (c(name)?, c(value)?);
            // SAFETY: as above.
            unsafe { httpd_resp_set_hdr(self.0, name.as_ptr(), value.as_ptr()) };
            kept.push(name);
            kept.push(value);
        }
        Ok(kept)
    }

    /// Sends a whole answer, with its length.
    #[allow(unsafe_code)]
    fn send(&self, body: &[u8]) -> bool {
        // SAFETY: the request is valid; `body` is valid for `body.len()` bytes during the call.
        unsafe { httpd_resp_send(self.0, body.as_ptr().cast(), body.len() as isize) == ESP_OK }
    }

    /// Sends one chunk of a chunked answer; an empty one ends it.
    #[allow(unsafe_code)]
    fn send_chunk(&self, chunk: &[u8]) -> bool {
        // SAFETY: as `send`.
        unsafe {
            httpd_resp_send_chunk(self.0, chunk.as_ptr().cast(), chunk.len() as isize) == ESP_OK
        }
    }

    /// A whole JSON answer (none for 204).
    fn answer(&self, response: &Response, headers: &[(&str, &str)]) {
        let Ok(_kept) = self.set_head(response.status, JSON, headers) else {
            return;
        };
        let body = response.body.as_deref().unwrap_or("");
        let _ = self.send(body.as_bytes());
    }

    /// What a synchronous handler returns once it has answered without reading the body: with a
    /// body declared, `ESP_FAIL`, so that ESP-IDF closes the connection instead of reading and
    /// discarding up to `Content-Length` bytes on the server's task (`httpd_req_delete`), which a
    /// slow client could hold for as long as it likes.
    fn unread_end(&self) -> esp_err_t {
        if self.content_length() > 0 {
            ESP_FAIL
        } else {
            ESP_OK
        }
    }

    /// Asks the server to close the connection once this answer is out.
    #[allow(unsafe_code)]
    fn close_after(&self) {
        // SAFETY: the request is valid; ESP-IDF queues the close of the socket on its own task.
        unsafe {
            let fd = httpd_req_to_sockfd(self.0);
            httpd_sess_trigger_close((*self.0).handle, fd);
        }
    }

    /// Ends an asynchronous request: ESP-IDF frees the copy and reads the next request of the
    /// connection.
    #[allow(unsafe_code)]
    fn complete(self) {
        // SAFETY: `self` is an asynchronous copy, completed once (it is consumed here).
        unsafe { httpd_req_async_handler_complete(self.0) };
    }
}

/// The reason phrase of a status.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Status",
    }
}

fn error_body(status: u16, message: &str) -> Response {
    Response {
        status,
        body: Some(json_error(message)),
    }
}

/// `{"error":"<message>"}` for the firmware's own refusals (fixed ASCII messages).
fn json_error(message: &str) -> String {
    format!(r#"{{"error":"{message}"}}"#)
}

// ---- The interface, on the server's task ----

/// `/` and `/assets/*`, every method: the core's `static_reply` decides, the file comes from the
/// flash, gzip as built.
#[allow(unsafe_code)]
extern "C" fn static_entry(request: *mut httpd_req_t) -> esp_err_t {
    let Some(api) = API.get() else {
        return ESP_FAIL;
    };
    let raw = Raw(request);
    let uri = raw.uri();
    let host = raw.header(c"Host");
    let reply = static_reply(&Request {
        method: raw.method(),
        path: &uri,
        host: host.as_deref(),
        port: api.port,
        reach: Reach::Lan { name: api.name },
        content_type: None,
        authorization: None,
        content_length: Some(0),
        body: b"",
    });
    match reply {
        Some(Reply::Static(file)) => match find(UI_FILES, &file.path) {
            Some(bytes) => {
                let mut headers = vec![
                    ("Content-Encoding", "gzip"),
                    ("Vary", "Accept-Encoding"),
                    ("Cache-Control", cache_control(&file)),
                ];
                headers.extend_from_slice(&STATIC_HEADERS);
                if let Ok(_kept) = raw.set_head(200, file.content_type, &headers) {
                    let _ = raw.send(bytes);
                }
            }
            None => raw.answer(&error_body(404, "Not found"), &STATIC_HEADERS),
        },
        Some(Reply::StaticError(response)) => raw.answer(&response, &STATIC_HEADERS),
        Some(Reply::Whole(response)) => raw.answer(&response, &[]),
        Some(
            Reply::List(_)
            | Reply::Order(_)
            | Reply::Update
            | Reply::UpdatePull(_)
            | Reply::VersionCheck(_)
            | Reply::Restart
            | Reply::RecipeJob(_)
            | Reply::CatalogueRefresh(_)
            | Reply::BackupExport(_)
            | Reply::BackupRestore { .. },
        )
        | None => {
            raw.answer(&error_body(404, "Not found"), &STATIC_HEADERS);
        }
    }
    raw.unread_end()
}

// ---- The API: the server's task hands each request to a worker ----

/// Every other path: a slot (503 `Busy` beyond 4 in progress), the head read, then an
/// asynchronous copy for a worker.
#[allow(unsafe_code)]
extern "C" fn api_entry(request: *mut httpd_req_t) -> esp_err_t {
    let Some(api) = API.get() else {
        return ESP_FAIL;
    };
    let raw = Raw(request);
    let Some(slot) = api.slots.try_take() else {
        if let Ok(_kept) = raw.set_head(503, JSON, &[]) {
            let _ = raw.send(BUSY.as_bytes());
        }
        return raw.unread_end();
    };
    let head = Head {
        method: raw.method(),
        uri: raw.uri(),
        host: raw.header(c"Host"),
        content_type: raw.header(c"Content-Type"),
        authorization: raw.header(c"Authorization"),
        chunked: raw.header(c"Transfer-Encoding").is_some(),
        length: raw.content_length(),
    };
    let mut copy: *mut httpd_req_t = std::ptr::null_mut();
    // SAFETY: `request` is the live request of this handler; ESP-IDF writes the copy's address.
    if unsafe { httpd_req_async_handler_begin(request, &mut copy) } != ESP_OK || copy.is_null() {
        raw.answer(&error_body(500, "Internal error"), &[]);
        return raw.unread_end();
    }
    let job = Job {
        raw: Raw(copy),
        head,
        _slot: slot,
    };
    // The slots admit at most `MAX_WORKERS` jobs, the queue holds as many: never full.
    if let Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) = api.queue.try_send(job)
    {
        job.raw.answer(&error_body(503, "Busy"), &[]);
        job.raw.complete();
    }
    ESP_OK
}

/// The update's body as a `Read`: the bytes of the request, ending at its `Content-Length` (so that
/// "one more byte than declared" never waits on the socket). A socket error or timeout is an
/// error, the installation then aborts.
struct BodyReader<'a> {
    raw: &'a Raw,
    remaining: usize,
}

impl Read for BodyReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let want = buffer.len().min(self.remaining);
        if want == 0 {
            return Ok(0);
        }
        let count = self.raw.receive(&mut buffer[..want]);
        if count < 0 {
            return Err(std::io::Error::other(format!("socket error {count}")));
        }
        let count = count as usize;
        self.remaining -= count;
        Ok(count)
    }
}

impl BodyReader<'_> {
    /// Reads and drops what is left of the body, before the connection closes after a refusal: a
    /// socket closed with unread bytes ends with a reset and the client loses the answer. Bounded
    /// by the declared length and `DRAIN_LIMIT_MS`, checked between two reads (a read may wait its
    /// receive timeout, 5 s, past it); the 1 KB buffer is on the stack, which fits
    /// the jobs' threads (10 KB for the backup, 16 KB for the update). Not used on the 413 of a
    /// declared length over the limit: the client must not be made to send 2 MB.
    fn drain(&mut self) {
        static START: OnceLock<Instant> = OnceLock::new();
        let remaining = self.remaining;
        let mut clock = || {
            u64::try_from(START.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
        };
        let began = Instant::now();
        let dropped = drain_body(self, remaining, &mut clock, DRAIN_LIMIT_MS);
        log::info!(
            "http: refused body drained: {dropped} bytes dropped, {} left, {} ms",
            remaining.saturating_sub(dropped),
            began.elapsed().as_millis()
        );
    }
}

/// `POST /api/v1/system/update`, once the core has let it through: the body goes into
/// `Updates::install`, through the OTA thread. Answers; true when the update is installed and the
/// ESP32 must restart (after `complete`, so the answer is out first). Runs on the `update` thread
/// (`job_thread::run`, `UPDATE_STACK_BYTES`): the signature check needs more stack than a worker
/// has left (D60).
fn answer_update(raw: &Raw, length: usize) -> bool {
    let (Some(updates), Some(mut adapter)) = (ota::updates(), ota::adapter()) else {
        raw.answer(&error_body(503, ota::unavailable()), &[]);
        raw.close_after();
        return false;
    };
    let mut source = BodyReader {
        raw,
        remaining: length,
    };
    let response = updates.install(&mut source, &mut adapter);
    let installed = response.status == 200;
    raw.answer(&response, &[]);
    if !installed {
        source.drain();
        raw.close_after();
    }
    installed
}

/// `POST /api/v1/system/version/check`, once the core has let it through: one check of the
/// manifest, over TLS. Runs on the `update-check` thread (`FETCH_STACK_BYTES`), the home's lock not
/// held.
fn answer_check(raw: &Raw, ticket: ReleaseTicket) {
    let response = ticket.check(&mut EspFetcher, &mut BoxPacing, Limits::CHECK);
    raw.answer(&response, &[]);
}

/// `POST /api/v1/system/update` with an empty body, once the core has let it through: the latest
/// release is downloaded into the installation, through the OTA thread. Answers; true when it is
/// installed and the ESP32 must restart (after `complete`, so the answer is out first). Runs on
/// the `update-pull` thread (`FETCH_STACK_BYTES`): the signature check needs more stack than a
/// worker has left (D60) and the TLS handshake runs in the calling task (D68). The request has no
/// body: nothing is left unread, the connection stays open after a refusal.
fn answer_pull(raw: &Raw, ticket: ReleaseTicket) -> bool {
    let (Some(updates), Some(mut adapter)) = (ota::updates(), ota::adapter()) else {
        raw.answer(&error_body(503, ota::unavailable()), &[]);
        return false;
    };
    let response = ticket.pull(
        updates,
        &mut EspFetcher,
        &mut adapter,
        &mut BoxPacing,
        Limits::CHECK,
        Limits::DOWNLOAD,
    );
    raw.answer(&response, &[]);
    response.status == 200
}

/// `POST /api/v1/plugins/install`, `/plugins/:id/update`, `/plugins/:id/uninstall` or
/// `/plugins/store/refresh`, once the core has let it through (spec 042): the download, the checks,
/// the write and the removal. Runs on the permanent `recipe` thread (`FETCH_STACK_BYTES`, its stack in
/// internal RAM: the flash is never touched from a worker); the loads and unloads go to the `script`
/// thread, and the home's lock is never held across a download or a write.
fn answer_recipe(raw: &Raw, ticket: RecipeTicket, home: &Arc<Mutex<Home>>) {
    let storage = FatStorage::new();
    let mut fetcher = EspFetcher;
    let mut pacing = BoxPacing;
    let mut env = JobEnv {
        fetcher: &mut fetcher,
        pacing: &mut pacing,
        storage: &storage,
        runtimes: crate::scripts::runtimes(),
        clock: crate::scripts::clock(),
        script_thread: &BoxScriptThread,
        check_limits: Limits::CHECK,
        download_limits: Limits::DOWNLOAD,
    };
    let response = ticket.run(home, &mut env);
    raw.answer(&response, &[]);
}

/// A request lent to a job's thread (`backup`, `update`, `update-check` or `update-pull`) for one job.
struct Lent<'a>(&'a Raw);

// SAFETY: the worker that owns the request lends it to the job's thread and waits for that thread
// to end before it uses the request again (`job_thread::run` joins it): one task at a time uses
// it, as ESP-IDF allows for an asynchronous request.
#[allow(unsafe_code)]
unsafe impl Send for Lent<'_> {}

impl<'a> Lent<'a> {
    /// The request (a method, so that a closure moves the whole `Lent`, not its field).
    fn get(self) -> &'a Raw {
        self.0
    }
}

/// What the worker does once a job's thread (`label`) is over: nothing after a job that ran; after
/// a thread that did not start (the job dropped), a 503; after a panic, the connection closes (the
/// answer may be half sent).
fn after_job_thread<T>(raw: &Raw, label: &str, ran: Result<Option<T>, String>) -> Option<T> {
    match ran {
        Ok(Some(value)) => Some(value),
        Ok(None) => {
            log::error!("{label}: its thread panicked");
            raw.close_after();
            None
        }
        Err(e) => {
            log::error!("{e}");
            raw.answer(&error_body(503, "Busy, try again"), &[]);
            raw.close_after();
            None
        }
    }
}

/// `GET /api/v1/backup`, once the core has let it through: the archive streams out in chunks
/// of 4 KB, from a buffer allocated for this request. The key derivation runs inside `export`,
/// before the status line, so that every refusal is a whole answer. A read error ends the answer
/// without its end: the client holds a truncated file, which its MAC refuses. Runs on the `backup`
/// thread (`job_thread::run`): it reads the flash.
fn answer_export(raw: &Raw, ticket: BackupTicket, password: Option<&str>) {
    let mut random = |bytes: &mut [u8]| fill(bytes);
    let mut reader = match ticket.export(
        password,
        Box::new(data_files()),
        Box::new(BoxPacing),
        &mut random,
    ) {
        Ok(reader) => reader,
        Err(response) => return raw.answer(&response, &[]),
    };
    let headers = [
        (
            "Content-Disposition",
            "attachment; filename=\"solite-backup.slb\"",
        ),
        ("Cache-Control", "no-store"),
    ];
    let Ok(_kept) = raw.set_head(200, "application/octet-stream", &headers) else {
        return;
    };
    let mut buffer = vec![0u8; CHUNK_BYTES];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                let _ = raw.send_chunk(&[]);
                break;
            }
            Ok(count) => {
                if !raw.send_chunk(&buffer[..count]) {
                    break;
                }
            }
            Err(_) => {
                raw.close_after();
                break;
            }
        }
    }
}

/// `POST /api/v1/backup`, once the core has let it through: the body goes into
/// `BackupTicket::restore`. Answers; true when the restore is staged and the ESP32 must restart
/// (after `complete`, so the answer is out first). A refusal reads and drops the rest of
/// the body, then the connection closes. Runs on the `backup` thread (`job_thread::run`): it writes the flash.
fn answer_restore(
    raw: &Raw,
    ticket: BackupTicket,
    wifi: bool,
    length: usize,
    password: Option<&str>,
) -> bool {
    let mut source = BodyReader {
        raw,
        remaining: length,
    };
    let response = ticket.restore(
        wifi,
        Some(length as u64),
        password,
        &mut source,
        &mut data_files(),
        &mut BoxPacing,
    );
    raw.answer(&response, &[]);
    if response.status == 200 {
        return true;
    }
    // A body declared over the limit is refused before a byte is read: the client is not made to
    // send it (413 `Backup too large`).
    if response.status != 413 {
        source.drain();
    }
    raw.close_after();
    false
}

/// A worker's answer to one request, as the host's `answer` (`crates/host/src/serve.rs`).
fn answer(job: Job, buffer: &mut [u8], api: &Api) {
    let Job { raw, head, _slot } = job;
    // The update's and the backup's bodies are files of megabytes: they are not read here (spec 032
    // R6, spec 035), the core answers the gate first, and the body streams into the installation
    // or the restore.
    let streaming = is_streamed_request(head.method, &head.uri);
    if head.chunked {
        raw.answer(&error_body(411, "Length required"), &[]);
        raw.close_after();
        return raw.complete();
    }
    // A body sent without a length is refused, except on the update route, where an empty one
    // (neither `Content-Length` nor `Transfer-Encoding`) is the pull of the latest release (spec
    // 039 R4): the core sees `content_length` `Some(0)`. A chunked update stays 411 (above), and
    // the backup restore keeps its 411.
    if streaming
        && raw.header(c"Content-Length").is_none()
        && !is_update_request(head.method, &head.uri)
    {
        raw.answer(&error_body(411, "Length required"), &[]);
        raw.close_after();
        return raw.complete();
    }
    if !streaming && head.length > MAX_BODY_BYTES {
        raw.answer(&error_body(413, "Body too large"), &[]);
        raw.close_after();
        return raw.complete();
    }
    let mut received = 0;
    while !streaming && received < head.length {
        let count = raw.receive(&mut buffer[received..head.length]);
        if count <= 0 {
            if count == HTTPD_SOCK_ERR_TIMEOUT {
                raw.answer(&error_body(408, "Request timeout"), &[]);
            }
            raw.close_after();
            return raw.complete();
        }
        received += count as usize;
    }
    // The core takes and lets go of the home itself: a password hash runs without it.
    let request = Request {
        method: head.method,
        path: &head.uri,
        host: head.host.as_deref(),
        port: api.port,
        reach: Reach::Lan { name: api.name },
        content_type: head.content_type.as_deref(),
        authorization: head.authorization.as_deref(),
        // The declared length: 0 when absent and not chunked, none when chunked.
        content_length: (!head.chunked).then_some(head.length as u64),
        body: &buffer[..received],
    };
    let answer_it = || {
        let mut random = |bytes: &mut [u8]| fill(bytes);
        handle_guarded(&api.home, &request, wall_ms(), &mut random)
    };
    // A request that changes something can start a script (an instance's start, a settle, a
    // validation): its handler runs on the `script` thread, made at boot, its 32 KB stack in PSRAM
    // and sized for the script's own (D83), never on this worker's 10 KB (spec 041 R3.4).
    // One at a time: the home's lock serialises them anyway. A `GET` never reaches a script, nor
    // do the account's routes (`/api/v1/auth/*`: the login's password hash must not hold every
    // other write behind it); they stay on this worker, as they were before scripts.
    let on_script_thread = head.method != "GET"
        && head.uri.starts_with("/api/v1")
        && !head.uri.starts_with("/api/v1/auth/");
    let reply = if on_script_thread {
        match run_on_script_thread(&request) {
            Some(reply) => reply,
            None => {
                log::warn!("script: the script thread is not running");
                Reply::Whole(error_body(503, "Busy"))
            }
        }
    } else {
        answer_it()
    };
    // An order waits for the radio loop's answer, without the lock.
    let reply = match reply {
        Reply::Order(wait) => Reply::Whole(order_response(wait)),
        other => other,
    };
    // The gate answered first; then a declared length above the maximum is refused (no body read).
    let too_large = matches!(reply, Reply::Update) && head.length as u64 > MAX_UPLOAD_BYTES;
    // Every refusal of a streamed route leaves body bytes unread: the connection closes after the
    // answer (a restore closes it itself, once the core has answered).
    let close_after_answer = streaming
        && (too_large
            || !matches!(
                reply,
                Reply::Update | Reply::UpdatePull(_) | Reply::BackupRestore { .. }
            ));
    let mut restart = false;
    if too_large {
        raw.answer(&error_body(413, "Update too large"), &[]);
        if close_after_answer {
            raw.close_after();
        }
        return raw.complete();
    }
    match reply {
        Reply::Whole(response) => raw.answer(&response, &[]),
        Reply::StaticError(response) => raw.answer(&response, &STATIC_HEADERS),
        Reply::List(mut cursor) => {
            if let Ok(_kept) = raw.set_head(200, JSON, &[]) {
                loop {
                    // The home is taken for one chunk, and let go while it is written (D15).
                    let chunk = cursor.next_chunk(&lock(&api.home));
                    match chunk {
                        Ok(Some(chunk)) => {
                            if !raw.send_chunk(chunk.as_bytes()) {
                                break;
                            }
                        }
                        Ok(None) => {
                            let _ = raw.send_chunk(&[]);
                            break;
                        }
                        // The client sees a broken stream, not a short list.
                        Err(_) => {
                            raw.close_after();
                            break;
                        }
                    }
                }
            }
        }
        Reply::BackupExport(ticket) => {
            let password = raw.header(c"X-Backup-Password");
            let lent = Lent(&raw);
            let ran = job_thread::run(
                c"backup",
                BACKUP_STACK_BYTES,
                StackPlace::Internal,
                move || {
                    answer_export(lent.get(), ticket, password.as_deref());
                },
            );
            after_job_thread(&raw, "backup", ran);
        }
        Reply::BackupRestore { ticket, wifi } => {
            let password = raw.header(c"X-Backup-Password");
            let length = head.length;
            let lent = Lent(&raw);
            let ran = job_thread::run(
                c"backup",
                BACKUP_STACK_BYTES,
                StackPlace::Internal,
                move || answer_restore(lent.get(), ticket, wifi, length, password.as_deref()),
            );
            restart = after_job_thread(&raw, "backup", ran).unwrap_or(false);
        }
        Reply::Order(_) | Reply::Static(_) => {
            raw.answer(&error_body(500, "Internal error"), &[]);
        }
        // The restart (spec 043): Sowel's answer, then as after an update, once whatever the number
        // of requests: a second one is answered the same and does not start another.
        Reply::Restart => {
            raw.answer(
                &Response {
                    status: 200,
                    body: Some(r#"{"success":true,"message":"Restarting"}"#.to_string()),
                },
                &[],
            );
            restart = !RESTARTING.swap(true, Ordering::SeqCst);
        }
        Reply::VersionCheck(ticket) => {
            let lent = Lent(&raw);
            let ran = job_thread::run(
                c"update-check",
                FETCH_STACK_BYTES,
                StackPlace::Internal,
                move || {
                    answer_check(lent.get(), ticket);
                },
            );
            after_job_thread(&raw, "update-check", ran);
        }
        Reply::RecipeJob(ticket) | Reply::CatalogueRefresh(ticket) => {
            let home = Arc::clone(&api.home);
            // The `recipe` thread made at boot; when it could not be made, a thread for this job
            // (the admission then counts its 20 KB stack, spec 045 R3), so an uninstall, safe
            // mode's way out of a bad recipe, still runs.
            let ran = if RECIPE_JOBS.get().is_some() {
                run_on_recipe_thread(&raw, ticket, home)
            } else {
                let lent = Lent(&raw);
                job_thread::run(
                    c"recipe",
                    FETCH_STACK_BYTES,
                    StackPlace::Internal,
                    move || {
                        answer_recipe(lent.get(), ticket, &home);
                    },
                )
            };
            after_job_thread(&raw, "recipe", ran);
        }
        Reply::UpdatePull(ticket) => {
            let lent = Lent(&raw);
            let ran = job_thread::run(
                c"update-pull",
                FETCH_STACK_BYTES,
                StackPlace::Internal,
                move || answer_pull(lent.get(), ticket),
            );
            restart = after_job_thread(&raw, "update-pull", ran).unwrap_or(false);
        }
        Reply::Update => {
            let length = head.length;
            let lent = Lent(&raw);
            let ran = job_thread::run(
                c"update",
                UPDATE_STACK_BYTES,
                StackPlace::Internal,
                move || answer_update(lent.get(), length),
            );
            restart = after_job_thread(&raw, "update", ran).unwrap_or(false);
        }
    }
    if close_after_answer {
        raw.close_after();
    }
    raw.complete();
    if restart {
        ota::restart_after_update();
    }
}

/// A buffer of `BODY_BUFFER_BYTES` in PSRAM, kept for the server's life.
#[allow(unsafe_code)]
fn body_buffer() -> Result<&'static mut [u8], String> {
    // SAFETY: asks ESP-IDF for a block of PSRAM; checked for null below.
    let pointer =
        unsafe { heap_caps_malloc(BODY_BUFFER_BYTES, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT) };
    if pointer.is_null() {
        return Err("serving: no PSRAM for a body buffer".to_string());
    }
    // SAFETY: the block is `BODY_BUFFER_BYTES` long, never freed (kept for the run), and owned by
    // this one slice; it is zeroed before use.
    let buffer = unsafe { std::slice::from_raw_parts_mut(pointer.cast::<u8>(), BODY_BUFFER_BYTES) };
    buffer.fill(0);
    Ok(buffer)
}

/// Starts the 4 workers on core 0, their stacks in PSRAM, each with its own body buffer.
fn start_workers(jobs: Receiver<Job>) -> Result<(), String> {
    let jobs = Arc::new(Mutex::new(jobs));
    ThreadSpawnConfiguration {
        name: Some(c"http-worker"),
        stack_size: WORKER_STACK_BYTES,
        pin_to_core: Some(Core::Core0),
        stack_alloc_caps: MallocCap::Spiram | MallocCap::Cap8bit,
        ..Default::default()
    }
    .set()
    .map_err(|e| format!("serving: cannot configure the workers: {e}"))?;
    let mut result = Ok(());
    for (index, unused) in WORKER_STACK_UNUSED.iter().enumerate() {
        let buffer = match body_buffer() {
            Ok(buffer) => buffer,
            Err(e) => {
                result = Err(e);
                break;
            }
        };
        let jobs = Arc::clone(&jobs);
        let spawned = std::thread::Builder::new()
            .name(format!("http-worker-{index}"))
            .stack_size(WORKER_STACK_BYTES)
            .spawn(move || {
                unused.store(stack_unused(), Ordering::Relaxed);
                loop {
                    let job = match lock(&jobs).recv() {
                        Ok(job) => job,
                        Err(_) => return,
                    };
                    if let Some(api) = API.get() {
                        answer(job, buffer, api);
                    }
                    unused.fetch_min(stack_unused(), Ordering::Relaxed);
                }
            });
        if let Err(e) = spawned {
            result = Err(format!("serving: cannot start a worker: {e}"));
            break;
        }
    }
    // Threads started later get the default configuration again.
    let _ = ThreadSpawnConfiguration::default().set();
    result
}

/// Registers a handler of ours for a path and a method.
#[allow(unsafe_code)]
fn register(
    server: httpd_handle_t,
    uri: &'static CStr,
    method: u32,
    handler: extern "C" fn(*mut httpd_req_t) -> esp_err_t,
) -> Result<(), String> {
    let conf = httpd_uri_t {
        uri: uri.as_ptr(),
        method,
        handler: Some(handler),
        user_ctx: std::ptr::null_mut(),
        ..Default::default()
    };
    // SAFETY: the server handle is live; ESP-IDF copies the description (and the path) it is
    // given; the path is static anyway.
    let code = unsafe { httpd_register_uri_handler(server, &conf) };
    if code == ESP_OK {
        Ok(())
    } else {
        Err(format!(
            "cannot register {} (error {code})",
            uri.to_string_lossy()
        ))
    }
}

/// Starts ESP-IDF's server with the configuration esp-idf-svc 0.53 builds for its
/// `EspHttpServer` (same fields, same values), except `recv_wait_timeout`: esp-idf-svc hardcodes
/// 5 s and offers no field, and a request whose body never comes holds the server's single task
/// that long (`HTTPD_RECV_WAIT_SECS`). The handlers are all registered raw, so nothing here needs
/// esp-idf-svc's own server type.
#[allow(unsafe_code)]
fn start_httpd(port: u16) -> Result<httpd_handle_t, String> {
    let configuration = httpd_config_t {
        task_priority: 5,
        task_caps: MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT,
        stack_size: SERVER_STACK_BYTES,
        core_id: Core::Core0.into(),
        server_port: port,
        ctrl_port: 32768,
        max_open_sockets: MAX_OPEN_SOCKETS as _,
        max_uri_handlers: 32,
        max_resp_headers: 10,
        backlog_conn: 5,
        lru_purge_enable: true,
        recv_wait_timeout: HTTPD_RECV_WAIT_SECS,
        send_wait_timeout: 5,
        uri_match_fn: Some(httpd_uri_match_wildcard),
        keep_alive_enable: true,
        keep_alive_idle: 30,
        keep_alive_interval: 5,
        keep_alive_count: 3,
        ..Default::default()
    };
    let mut handle: httpd_handle_t = std::ptr::null_mut();
    // SAFETY: `configuration` is a complete `httpd_config_t` that lives through the call (ESP-IDF
    // copies it); `handle` is a valid place for the result.
    let code = unsafe { httpd_start(&mut handle, &configuration) };
    if code == ESP_OK && !handle.is_null() {
        Ok(handle)
    } else {
        Err(format!("httpd_start failed ({code})"))
    }
}

/// Starts the server for the shared home: `/ws`, the interface, then the API on every other
/// path (ESP-IDF tries the handlers in the order they were registered).
pub fn start(shared: Shared, name: &'static str, port: u16) -> Result<Server, String> {
    let (queue, jobs) = sync_channel(MAX_WORKERS);
    let api = Api {
        home: Arc::clone(&shared.home),
        name,
        port,
        slots: Slots::new(MAX_WORKERS),
        queue,
    };
    API.set(api)
        .map_err(|_| "the server is already started".to_string())?;
    start_script_thread()?;
    start_workers(jobs)?;
    let handle = start_httpd(port)?;
    live_ws::register(handle, shared.hub, Arc::clone(&shared.home), name, port)?;
    for method in [
        http_method_HTTP_GET,
        http_method_HTTP_POST,
        http_method_HTTP_PUT,
        http_method_HTTP_DELETE,
    ] {
        register(handle, c"/", method, static_entry)?;
        register(handle, c"/assets/*", method, static_entry)?;
    }
    for method in [
        http_method_HTTP_GET,
        http_method_HTTP_POST,
        http_method_HTTP_PUT,
        http_method_HTTP_DELETE,
        http_method_HTTP_PATCH,
    ] {
        register(handle, c"/*", method, api_entry)?;
    }
    ota::API_LISTENING.store(true, Ordering::SeqCst);
    Ok(Server { handle })
}
