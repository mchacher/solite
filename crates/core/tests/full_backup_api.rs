//! Spec 035: the routes of the full backup behind the gate (R1), the home's backup state, and the
//! Saver held while an export reads the files (R4).

mod mem_storage;
mod support;

use std::sync::Mutex;

use mem_storage::MemStorage;
use serde_json::{Value, json};
use solite_core::api::{
    ROUTES, Reach, Reply, Request, handle, handle_guarded, is_streamed_request,
};
use solite_core::auth::{Account, AlwaysConfirmed, Language, NeverConfirmed, PasswordHash};
use solite_core::backup::Phase;
use solite_core::devices::{Id, lock};
use solite_core::engine::Saver;
use solite_core::home::Home;
use support::backup_fake::{Clock, MemFiles};
use support::{HOST, PORT, T0, counter};

const BACKUP: &str = "/api/v1/backup";
const OCTET: &str = "application/octet-stream";
const PASSWORD: &str = "correct horse battery";

struct Server {
    home: Mutex<Home>,
    random: Mutex<support::Random>,
    token: String,
}

impl Server {
    fn new() -> Server {
        let mut home = Home::new(T0);
        home.load_auth(Some(Account {
            id: Id::parse("11111111-1111-4111-8111-111111111111").unwrap(),
            username: "admin".into(),
            display_name: "Owner".into(),
            language: Language::En,
            created_at: T0,
            password: PasswordHash::new(PASSWORD, [7; 16]),
        }));
        home.set_presence(AlwaysConfirmed);
        let mut server = Server {
            home: Mutex::new(home),
            random: Mutex::new(counter()),
            token: String::new(),
        };
        let reply = server.send(
            "POST",
            "/api/v1/auth/login",
            false,
            Some("application/json"),
            br#"{"username":"admin","password":"correct horse battery"}"#,
        );
        let Reply::Whole(login) = reply else {
            panic!("login")
        };
        let body: Value = serde_json::from_str(login.body.as_deref().unwrap()).unwrap();
        server.token = body["accessToken"].as_str().unwrap().to_string();
        server
    }

    fn send(
        &self,
        method: &str,
        path: &str,
        token: bool,
        content_type: Option<&str>,
        body: &[u8],
    ) -> Reply {
        let authorization = token.then(|| format!("Bearer {}", self.token));
        let mut random = |buffer: &mut [u8]| (self.random.lock().unwrap())(buffer);
        handle_guarded(
            &self.home,
            &Request {
                method,
                path,
                host: Some(HOST),
                port: PORT,
                reach: Reach::Loopback,
                content_type,
                authorization: authorization.as_deref(),
                content_length: None,
                body,
            },
            T0,
            &mut random,
        )
    }

    /// (status, error text) of a whole answer.
    fn refusal(
        &self,
        method: &str,
        path: &str,
        token: bool,
        content_type: Option<&str>,
    ) -> (u16, Value) {
        match self.send(method, path, token, content_type, b"") {
            Reply::Whole(response) => (
                response.status,
                serde_json::from_str(response.body.as_deref().unwrap_or("null")).unwrap(),
            ),
            other => panic!("not a whole answer: {other:?}"),
        }
    }

    fn phase(&self) -> Phase {
        lock(&self.home).backup_state().phase()
    }
}

#[test]
fn both_routes_need_the_token_and_read_nothing_before() {
    let server = Server::new();
    for method in ["GET", "POST"] {
        let (status, body) = server.refusal(method, BACKUP, false, Some(OCTET));
        assert_eq!(
            (status, body),
            (401, json!({"error": "Authentication required"})),
            "{method}"
        );
    }
    assert_eq!(server.phase(), Phase::Idle);
}

#[test]
fn a_get_with_the_token_is_an_export_ticket_and_a_post_with_octet_stream_a_restore_ticket() {
    let server = Server::new();
    let Reply::BackupExport(ticket) = server.send("GET", BACKUP, true, None, b"") else {
        panic!("export")
    };
    assert_eq!(server.phase(), Phase::Preparing);
    drop(ticket);
    assert_eq!(
        server.phase(),
        Phase::Idle,
        "the ticket's drop ends the job"
    );
    for (path, wifi) in [
        (BACKUP.to_string(), false),
        (format!("{BACKUP}?wifi=true"), true),
        (format!("{BACKUP}?wifi=false"), false),
        (format!("{BACKUP}?x=1&wifi=true"), true),
        (format!("{BACKUP}?wifi=truex"), false),
    ] {
        let reply = server.send(
            "POST",
            &path,
            true,
            Some("Application/Octet-Stream; x=1"),
            b"",
        );
        let Reply::BackupRestore {
            ticket,
            wifi: asked,
        } = reply
        else {
            panic!("restore {path}")
        };
        assert_eq!(asked, wifi, "{path}");
        assert_eq!(server.phase(), Phase::Restoring);
        drop(ticket);
    }
    assert!(is_streamed_request("POST", BACKUP));
    assert!(is_streamed_request("POST", &format!("{BACKUP}?wifi=true")));
    assert!(is_streamed_request("POST", "/api/v1/system/update"));
    assert!(!is_streamed_request("GET", BACKUP));
    assert!(!is_streamed_request("POST", "/api/v1/backup/"));
}

#[test]
fn a_restore_with_another_content_type_is_415() {
    let server = Server::new();
    for content_type in [Some("application/json"), None, Some("text/plain")] {
        let (status, body) = server.refusal("POST", BACKUP, true, content_type);
        assert_eq!(
            (status, body),
            (415, json!({"error": "Unsupported Media Type"})),
            "{content_type:?}"
        );
    }
    assert_eq!(server.phase(), Phase::Idle);
}

#[test]
fn a_restore_without_the_presence_is_403_with_its_hint_and_starts_nothing() {
    let server = Server::new();
    lock(&server.home).set_presence(NeverConfirmed);
    let (status, body) = server.refusal("POST", BACKUP, true, Some(OCTET));
    assert_eq!(status, 403);
    assert_eq!(body["confirmRequired"], true);
    assert_eq!(body["error"], "Confirmation at the box is not available");
    assert_eq!(server.phase(), Phase::Idle);
    // The export needs no presence.
    assert!(matches!(
        server.send("GET", BACKUP, true, None, b""),
        Reply::BackupExport(_)
    ));
}

#[test]
fn a_restore_while_pairing_is_409_and_a_second_job_while_one_runs_is_409() {
    let server = Server::new();
    lock(&server.home).pairing_mut().opened(u64::MAX);
    let (status, body) = server.refusal("POST", BACKUP, true, Some(OCTET));
    assert_eq!(
        (status, body),
        (409, json!({"error": "Pairing in progress"}))
    );
    lock(&server.home).pairing_mut().ended(u64::MAX);
    lock(&server.home).pairing_mut().finished();

    let Reply::BackupExport(first) = server.send("GET", BACKUP, true, None, b"") else {
        panic!("export")
    };
    for (method, content_type) in [("GET", None), ("POST", Some(OCTET))] {
        let (status, body) = server.refusal(method, BACKUP, true, content_type);
        assert_eq!(
            (status, body),
            (409, json!({"error": "Backup already in progress"})),
            "{method}"
        );
    }
    drop(first);
    // A restore running refuses an export too.
    let Reply::BackupRestore { ticket, .. } = server.send("POST", BACKUP, true, Some(OCTET), b"")
    else {
        panic!("restore")
    };
    let (status, body) = server.refusal("GET", BACKUP, true, None);
    assert_eq!(
        (status, body),
        (409, json!({"error": "Backup already in progress"}))
    );
    drop(ticket);
    assert_eq!(server.phase(), Phase::Idle);
}

#[test]
fn both_routes_answer_409_while_a_restore_waits_for_the_restart() {
    let server = Server::new();
    let Reply::BackupRestore { ticket, .. } = server.send("POST", BACKUP, true, Some(OCTET), b"")
    else {
        panic!("restore")
    };
    let archive = {
        let files = MemFiles::with(&[("zones.json", "[]")]);
        let home = Home::new(0);
        let mut reader = home
            .backup_begin(false)
            .unwrap()
            .export_with_iterations(
                Some("a%20password"),
                Box::new(files),
                Box::new(Clock::new()),
                &mut |b: &mut [u8]| {
                    b.fill(3);
                    Ok(())
                },
                1000,
            )
            .unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut bytes).unwrap();
        bytes
    };
    let response = ticket.restore(
        false,
        Some(archive.len() as u64),
        Some("a%20password"),
        &mut std::io::Cursor::new(archive),
        &mut MemFiles::new(),
        &mut Clock::new(),
    );
    assert_eq!(response.status, 200);
    assert_eq!(server.phase(), Phase::Pending);
    for (method, content_type) in [
        ("GET", None),
        ("POST", Some(OCTET)),
        ("POST", Some("text/plain")),
    ] {
        let (status, body) = server.refusal(method, BACKUP, true, content_type);
        assert_eq!(
            (status, body),
            (409, json!({"error": "A restore waits for the restart"})),
            "{method} {content_type:?}"
        );
    }
}

#[test]
fn other_methods_are_405_and_the_old_backup_route_is_404() {
    let server = Server::new();
    for method in ["PUT", "DELETE", "PATCH"] {
        let (status, body) = server.refusal(method, BACKUP, true, Some("application/json"));
        assert_eq!(
            (status, body),
            (405, json!({"error": "Method not allowed"})),
            "{method}"
        );
    }
    for method in ["GET", "POST"] {
        let (status, _) = server.refusal(
            method,
            "/api/v1/system/backup",
            true,
            Some("application/json"),
        );
        assert_eq!(status, 404, "{method} /api/v1/system/backup");
    }
    assert!(ROUTES.contains(&("GET", BACKUP)));
    assert!(ROUTES.contains(&("POST", BACKUP)));
    assert!(
        ROUTES
            .iter()
            .all(|(_, path)| *path != "/api/v1/system/backup")
    );
}

#[test]
fn the_plain_handler_cannot_stream_and_answers_501_on_both_routes() {
    let mut home = Home::new(T0);
    let mut random = counter();
    for (method, content_type) in [
        ("GET", None),
        ("POST", Some(OCTET)),
        ("POST", Some("application/json")),
    ] {
        let response = handle(
            &mut home,
            &Request {
                method,
                path: BACKUP,
                host: Some(HOST),
                port: PORT,
                reach: Reach::Loopback,
                content_type,
                authorization: None,
                content_length: None,
                body: b"",
            },
            T0,
            &mut random,
        );
        assert_eq!(response.status, 501, "{method}");
        assert_eq!(
            response.body.as_deref(),
            Some(r#"{"error":"Backups need the streaming handler"}"#)
        );
    }
    assert_eq!(home.backup_state().phase(), Phase::Idle);
}

// ---- The Saver ----

fn add_zone(home: &Mutex<Home>, name: &str) {
    let mut random = counter();
    let mut home = lock(home);
    let response = handle(
        &mut home,
        &Request {
            method: "POST",
            path: "/api/v1/zones",
            host: Some(HOST),
            port: PORT,
            reach: Reach::Loopback,
            content_type: Some("application/json"),
            authorization: None,
            content_length: None,
            body: json!({"name": name}).to_string().as_bytes(),
        },
        T0,
        &mut random,
    );
    assert_eq!(response.status, 201, "{response:?}");
}

#[test]
fn a_due_file_is_not_written_while_an_export_reads_the_files_and_is_written_after() {
    let home = Mutex::new(Home::new(T0));
    let storage = MemStorage::default();
    let mut saver = Saver::new(&storage, T0);
    add_zone(&home, "Garage");
    let state = lock(&home).backup_state();
    let ticket = lock(&home).backup_begin(false).unwrap();
    // Preparing: the Saver still writes (the key derivation takes seconds).
    assert_eq!(state.phase(), Phase::Preparing);
    // Exporting: it starts no write.
    assert!(ticket.hold_for_export());
    saver.tick(&home, T0 + 1, false).unwrap();
    assert_eq!(storage.writes("zones.json"), 0, "held while exporting");
    saver.tick(&home, T0 + 2, false).unwrap();
    assert_eq!(storage.writes("zones.json"), 0);
    assert!(storage.text("zones.json").is_none());
    drop(ticket);
    saver.tick(&home, T0 + 3, false).unwrap();
    assert_eq!(
        storage.writes("zones.json"),
        1,
        "written at the next turn after the export"
    );
    assert!(storage.text("zones.json").unwrap().contains("Garage"));
}

#[test]
fn a_stop_writes_whatever_an_export_does() {
    let home = Mutex::new(Home::new(T0));
    let storage = MemStorage::default();
    let mut saver = Saver::new(&storage, T0);
    add_zone(&home, "Garage");
    let ticket = lock(&home).backup_begin(false).unwrap();
    assert!(ticket.hold_for_export());
    saver.tick(&home, T0 + 1, true).unwrap();
    assert_eq!(storage.writes("zones.json"), 1);
}

#[test]
fn the_export_switch_and_the_savers_write_start_are_one_critical_section() {
    let home = Home::new(T0);
    let state = home.backup_state();
    // The Saver started first: the export waits for the end of the write.
    let writing = state.try_write().expect("no export yet");
    let ticket = home.backup_begin(false).unwrap();
    assert!(!ticket.hold_for_export(), "the Saver is writing");
    assert_eq!(state.phase(), Phase::Preparing);
    drop(writing);
    // The export switched first: the Saver cannot start between the check and the hold.
    assert!(ticket.hold_for_export());
    assert!(state.try_write().is_none(), "refused once the export holds");
    drop(ticket);
    assert!(state.try_write().is_some());
}
