//! `GET /api/v1/system/supervisor` and the safe mode's 503 (spec 036 R5 and R9), through the gate.

mod support;

use std::sync::Mutex;

use serde_json::{Value, json};
use solite_core::api::{Reach, Reply, Request, handle_guarded};
use solite_core::auth::{Account, Language, PasswordHash};
use solite_core::devices::Id;
use solite_core::home::Home;
use solite_core::supervisor::{Mode, Status};
use support::{HOST, PORT, T0, counter};

const SUPERVISOR: &str = "/api/v1/system/supervisor";
const SAFE_MODE_ERROR: &str = "Safe mode: the engine is stopped";

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
            password: PasswordHash::new("correct horse battery", [7; 16]),
        }));
        let mut server = Server {
            home: Mutex::new(home),
            random: Mutex::new(counter()),
            token: String::new(),
        };
        let body = br#"{"username":"admin","password":"correct horse battery"}"#;
        let (status, login) = whole(server.send("POST", "/api/v1/auth/login", false, body));
        assert_eq!(status, 200, "{login}");
        server.token = login["accessToken"].as_str().unwrap().to_string();
        server
    }

    fn set_mode(&self, mode: Mode) {
        let mut home = self.home.lock().unwrap();
        home.set_supervision(Status {
            mode,
            boot_cause: "panic",
            last_incident: Some("panic"),
            restarts_last_hour: 3,
            restarts_last_day: 3,
        });
    }

    fn send(&self, method: &str, path: &str, with_token: bool, body: &[u8]) -> Reply {
        let authorization = format!("Bearer {}", self.token);
        let content_type = if path.ends_with("/backup") || path.ends_with("/system/update") {
            "application/octet-stream"
        } else {
            "application/json"
        };
        let mut random = |buffer: &mut [u8]| (self.random.lock().unwrap())(buffer);
        handle_guarded(
            &self.home,
            &Request {
                method,
                path,
                host: Some(HOST),
                port: PORT,
                reach: Reach::Loopback,
                content_type: Some(content_type),
                authorization: with_token.then_some(authorization.as_str()),
                content_length: Some(body.len() as u64),
                body,
            },
            T0,
            &mut random,
        )
    }
}

fn whole(reply: Reply) -> (u16, Value) {
    match reply {
        Reply::Whole(response) => (
            response.status,
            response
                .body
                .map_or(Value::Null, |t| serde_json::from_str(&t).unwrap()),
        ),
        other => panic!("not a whole answer: {other:?}"),
    }
}

#[test]
fn the_supervisor_route_answers_the_default_status_then_the_one_set() {
    let server = Server::new();
    let (status, body) = whole(server.send("GET", SUPERVISOR, true, b""));
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({"mode": "normal", "bootCause": "power-on", "lastIncident": null,
               "restartsLastHour": 0, "restartsLastDay": 0})
    );
    server.set_mode(Mode::Safe);
    let (status, body) = whole(server.send("GET", SUPERVISOR, true, b""));
    assert_eq!(status, 200, "a GET is served in safe mode");
    assert_eq!(
        body,
        json!({"mode": "safe", "bootCause": "panic", "lastIncident": "panic",
               "restartsLastHour": 3, "restartsLastDay": 3})
    );
}

#[test]
fn another_method_on_the_supervisor_route_is_405_and_no_token_is_401() {
    let server = Server::new();
    for method in ["POST", "PUT", "DELETE"] {
        let (status, _) = whole(server.send(method, SUPERVISOR, true, b"{}"));
        assert_eq!(status, 405, "{method}");
    }
    let (status, body) = whole(server.send("GET", SUPERVISOR, false, b""));
    assert_eq!(status, 401);
    assert_eq!(body, json!({"error": "Authentication required"}));
}

#[test]
fn in_safe_mode_a_change_is_refused_with_503_and_a_read_or_a_login_answers_as_usual() {
    let server = Server::new();
    for mode in [Mode::Safe, Mode::SafeBare] {
        server.set_mode(mode);
        for (method, path) in [
            ("POST", "/api/v1/zones"),
            ("PUT", "/api/v1/settings"),
            ("POST", "/api/v1/system/factory-reset"),
            ("POST", "/api/v1/backup"),
            ("DELETE", "/api/v1/zones/x"),
        ] {
            let (status, body) = whole(server.send(method, path, true, br#"{"name":"A"}"#));
            assert_eq!(status, 503, "{mode:?} {method} {path}");
            assert_eq!(body, json!({"error": SAFE_MODE_ERROR}));
        }
        let (status, _) = whole(server.send("GET", "/api/v1/zones", true, b""));
        assert_eq!(status, 200, "{mode:?}");
        let login = br#"{"username":"admin","password":"correct horse battery"}"#;
        let (status, _) = whole(server.send("POST", "/api/v1/auth/login", false, login));
        assert_eq!(status, 200, "{mode:?}");
    }
}

#[test]
fn in_safe_mode_the_update_is_left_to_the_streaming_caller() {
    let server = Server::new();
    server.set_mode(Mode::Safe);
    let reply = server.send("POST", "/api/v1/system/update", true, b"x");
    assert!(matches!(reply, Reply::Update), "{reply:?}");
    // An empty body pulls the latest release (spec 039): let through as well.
    let reply = server.send("POST", "/api/v1/system/update", true, b"");
    assert!(matches!(reply, Reply::UpdatePull(_)), "{reply:?}");
}

#[test]
fn in_safe_mode_a_request_without_a_token_is_still_401_not_503() {
    let server = Server::new();
    server.set_mode(Mode::Safe);
    let (status, _) = whole(server.send("POST", "/api/v1/zones", false, b"{}"));
    assert_eq!(status, 401);
}

#[test]
fn in_normal_mode_a_zone_can_be_created() {
    let server = Server::new();
    let (status, body) = whole(server.send("POST", "/api/v1/zones", true, br#"{"name":"Attic"}"#));
    assert_eq!(status, 201, "{body}");
}

#[test]
fn in_safe_mode_the_restart_goes_through_and_another_post_is_503() {
    let server = Server::new();
    for mode in [Mode::Safe, Mode::SafeBare] {
        server.set_mode(mode);
        let reply = server.send("POST", "/api/v1/system/restart", true, b"");
        assert!(matches!(reply, Reply::Restart), "{mode:?}: {reply:?}");
        let (status, body) = whole(server.send("POST", "/api/v1/zones", true, b"{}"));
        assert_eq!(status, 503, "{mode:?}");
        assert_eq!(body, json!({"error": SAFE_MODE_ERROR}));
    }
}
