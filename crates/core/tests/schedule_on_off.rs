//! `schedule-on-off`, Sowel's recipe 2.0.2 as the script `recipes/scripts/schedule-on-off.js`
//! (spec 055): its info (pinned from Sowel's source), Sowel's validation texts, the windows in the
//! home's time zone (one across midnight), a boundary that follows the sun and its re-arm the next
//! day, a sun that is not known, and the memory of 4 instances.
//!
//! These tests need the `script-recipes` feature (the host program turns it on, so
//! `cargo test --workspace` runs them).

mod support;

use std::sync::Arc;

use serde_json::{Value, json};
use solite_core::devices::Id;
use solite_core::radio::zcl::Command;
use solite_core::recipes::native::{NativeHost, RecipeName};
use solite_core::recipes::script::{
    NewRuntime, SCRIPT_MEMORY_LIMIT, ScriptRecipe, ScriptRuntimes, SystemRuntimes,
};
use solite_core::recipes::{MAX_INSTANCES_PER_SCRIPT, ScriptClock};
use solite_recipe_sdk::{Clock, Recipe};
use support::*;

const SCRIPT: &str = include_str!("../../../recipes/scripts/schedule-on-off.js");
const SECOND: u64 = 1000;
const BASE: &str = "/api/v1/recipe-instances";
const GHOST: &str = "11111111-2222-4333-8444-555555555555";
/// 2026-11-21 00:00:00 UTC.
const MIDNIGHT: u64 = T0 - (9 * HOUR + 12 * MINUTE + 3 * SECOND + 120);

/// The instant of a Paris time of the day (an hour ahead of UTC in November), on the 21st; hours
/// from 1 to 23.
fn paris(hour: u64, minute: u64) -> u64 {
    MIDNIGHT + (hour - 1) * HOUR + minute * MINUTE
}

struct World {
    rig: Rig,
    g: Garage,
}

impl World {
    /// The garage, the script loaded, the home in Paris; `located` gives it its coordinates (so
    /// that the sun is known).
    fn new(now: u64, located: bool) -> World {
        let mut rig = Rig::new();
        rig.now = now;
        let g = rig.garage();
        let clock: ScriptClock = Arc::new(|| 0);
        rig.home
            .load_script_recipe(SCRIPT, Arc::new(SystemRuntimes), clock)
            .unwrap();
        let mut settings = json!({"home.timezone": "Europe/Paris"});
        if located {
            settings["home.latitude"] = json!("48.8566");
            settings["home.longitude"] = json!("2.3522");
        }
        let (status, _) = rig.put("/api/v1/settings", settings);
        assert_eq!(status, 200);
        rig.sent();
        World { rig, g }
    }

    fn create(&mut self, params: Value) -> (u16, Value) {
        self.rig.post(
            BASE,
            json!({"recipeId": "schedule-on-off", "params": params}),
        )
    }

    fn start(&mut self, extra: Value) -> String {
        let mut params = json!({"zone": self.g.zone, "equipments": [self.g.light]});
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        let (status, body) = self.create(params);
        assert_eq!(status, 201, "{body}");
        body["id"].as_str().unwrap().to_string()
    }

    /// Time moves to `at` (a turn at the end, a second after).
    fn go(&mut self, at: u64) {
        self.rig.now = at;
        self.rig.advance(SECOND);
    }

    fn sent(&mut self) -> Vec<Command> {
        self.rig.sent()
    }

    fn state(&mut self, id: &str) -> Value {
        self.rig.state(id)
    }

    fn log(&mut self, id: &str) -> String {
        self.rig.messages(id).join("\n")
    }
}

fn refused(w: &mut World, params: Value) -> (u16, String) {
    let (status, body) = w.create(params);
    (status, body["error"].as_str().unwrap_or("").to_string())
}

fn invalid(message: &str) -> (u16, String) {
    (400, format!("Invalid params: {message}"))
}

// ---- info ----

#[test]
fn the_recipe_is_served_with_sowels_info_byte_for_byte() {
    let mut w = World::new(T0, false);
    let response = w.rig.raw("GET", "/api/v1/recipes/schedule-on-off", "");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body.unwrap(),
        include_str!("pinned/recipe_schedule_on_off.json")
    );
}

#[test]
fn the_list_of_recipes_holds_the_same_info_as_the_single_answer() {
    let mut w = World::new(T0, false);
    let (status, list) = w.rig.get("/api/v1/recipes");
    assert_eq!(status, 200);
    let pinned: Value =
        serde_json::from_str(include_str!("pinned/recipe_schedule_on_off.json")).unwrap();
    let entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "schedule-on-off")
        .unwrap();
    assert_eq!(entry, &pinned);
}

// ---- validation ----

#[test]
fn every_refused_setting_answers_sowels_text_in_sowels_order() {
    let mut w = World::new(T0, false);
    let (zone, light) = (w.g.zone.clone(), w.g.light.clone());
    let ok = json!({"zone": zone, "equipments": [light], "slot1_start_time": "08:00", "slot1_end_time": "09:00"});
    let with = |extra: Value| {
        let mut params = ok.clone();
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        params
    };
    let cases: Vec<(Value, String)> = vec![
        (json!({}), "Zone is required".into()),
        (
            json!({"zone": zone}),
            "At least one equipment is required".into(),
        ),
        (
            json!({"zone": zone, "equipments": []}),
            "At least one equipment is required".into(),
        ),
        (
            json!({"zone": zone, "equipments": [light]}),
            "Slot 1 start and end are required (a fixed time, or sunrise/sunset)".into(),
        ),
        (
            with(json!({"slot1_end_time": "9h"})),
            "Slot 1 start and end are required (a fixed time, or sunrise/sunset)".into(),
        ),
        (
            with(json!({"slot1_start_time": "24:00"})),
            "Slot 1 start and end are required (a fixed time, or sunrise/sunset)".into(),
        ),
        (
            with(json!({"slot2_start_time": "10:00"})),
            "Slot 2 start and end must both be set".into(),
        ),
        (
            with(json!({"slot3_end_kind": "sunset"})),
            "Slot 3 start and end must both be set".into(),
        ),
    ];
    for (params, message) in cases {
        assert_eq!(
            refused(&mut w, params.clone()),
            invalid(&message),
            "{params}"
        );
    }
    // A slot 2 with both ends, and a sun window in slot 1, are accepted.
    let (status, body) = w.create(with(
        json!({"slot2_start_time": "10:00", "slot2_end_time": "11:00"}),
    ));
    assert_eq!(status, 201, "{body}");
    let (status, body) = w.create(json!({
        "zone": zone, "equipments": [light],
        "slot1_start_kind": "sunrise", "slot1_start_offset": -30,
        "slot1_end_kind": "sunset",
    }));
    assert_eq!(status, 201, "{body}");
    // The host's own checks of the slots: an offset beyond 180, an unknown kind, an unknown id.
    assert_eq!(
        refused(
            &mut w,
            with(json!({"slot1_start_kind": "sunrise", "slot1_start_offset": 181}))
        ),
        invalid("slot1_start_offset must be a number between -180 and 180")
    );
    assert_eq!(
        refused(&mut w, with(json!({"slot1_start_kind": "noon"}))),
        invalid("slot1_start_kind must be one of time, sunrise, sunset")
    );
    assert_eq!(refused(&mut w, with(json!({"equipments": [GHOST]}))).0, 400);
}

// ---- behaviour ----

#[test]
fn a_window_switches_on_and_off_at_the_homes_local_times() {
    // 08:00 to 09:30 in Paris is 07:00 to 08:30 UTC.
    let mut w = World::new(MIDNIGHT + 5 * HOUR, false);
    let id = w.start(json!({"slot1_start_time": "08:00", "slot1_end_time": "09:30"}));
    let state = w.state(&id);
    assert_eq!(state["status"], "idle");
    assert_eq!(state["nextStart"], "08:00");
    assert_eq!(state["nextEnd"], "09:30");
    assert!(
        w.log(&id)
            .contains("Recette démarrée : 1 équipement(s), 1 créneau(x) [08:00 → 09:30]")
    );
    w.go(paris(7, 59));
    assert_eq!(w.sent(), vec![]);
    w.go(paris(8, 0));
    assert_eq!(w.sent(), vec![Command::On]);
    let state = w.state(&id);
    assert_eq!(state["status"], "running");
    assert_eq!(state["currentSlot"], "08:00 → 09:30");
    assert!(w.log(&id).contains("Marche créneau 08:00 → 09:30"));
    w.go(paris(9, 30));
    assert_eq!(w.sent(), vec![Command::Off]);
    let state = w.state(&id);
    assert_eq!(state["status"], "idle");
    assert_eq!(state["currentSlot"], Value::Null);
    assert!(w.log(&id).contains("Arrêt créneau 08:00 → 09:30"));
    // The next day, the same.
    w.go(paris(8, 0) + 24 * HOUR);
    assert_eq!(w.sent(), vec![Command::On]);
}

#[test]
fn a_window_across_midnight_switches_on_in_the_evening_and_off_in_the_morning() {
    let mut w = World::new(MIDNIGHT + 12 * HOUR, false);
    let id = w.start(json!({"slot1_start_time": "22:00", "slot1_end_time": "06:00"}));
    w.go(paris(22, 0));
    assert_eq!(w.sent(), vec![Command::On]);
    // Next day, 06:00 Paris is 05:00 UTC.
    w.go(MIDNIGHT + 24 * HOUR + 5 * HOUR);
    assert_eq!(w.sent(), vec![Command::Off]);
    assert!(w.log(&id).contains("Arrêt créneau 22:00 → 06:00"));
}

#[test]
fn a_boundary_that_follows_the_sun_fires_at_the_sun_time_with_its_offset_and_the_next_day() {
    let mut w = World::new(MIDNIGHT + 12 * HOUR, true);
    let id = w.start(json!({
        "slot1_start_kind": "sunset", "slot1_start_offset": -15,
        "slot1_end_time": "23:00",
    }));
    let state = w.state(&id);
    let next = state["nextStart"].as_str().unwrap().to_string();
    // Sunset in Paris on the 21st of November is about 16:56: minus 15 minutes.
    assert!(next.starts_with("16:") || next.starts_with("17:"), "{next}");
    assert!(w.log(&id).contains("coucher -15min → 23:00"));
    let at = |day: u64, time: &str| {
        let h: u64 = time[..2].parse().unwrap();
        let m: u64 = time[3..].parse().unwrap();
        paris(h, m) + day * 24 * HOUR
    };
    w.go(at(0, &next) - 2 * MINUTE);
    assert_eq!(w.sent(), vec![]);
    w.go(at(0, &next));
    assert_eq!(w.sent(), vec![Command::On]);
    // After midnight the sun's time of the new day is read again.
    w.go(MIDNIGHT + 23 * HOUR + MINUTE); // 00:01 on the 22nd in Paris
    assert_eq!(w.sent(), vec![Command::Off], "23:00");
    let next2 = w.state(&id)["nextStart"].as_str().unwrap().to_string();
    w.go(at(1, &next2));
    assert_eq!(
        w.sent(),
        vec![Command::On],
        "the sun's time of the next day"
    );
}

#[test]
fn without_the_sun_a_sun_boundary_is_skipped_with_a_warning() {
    let mut w = World::new(MIDNIGHT + 12 * HOUR, false);
    let id = w.start(json!({"slot1_start_kind": "sunrise", "slot1_end_time": "23:00"}));
    assert_eq!(w.state(&id)["nextStart"], Value::Null);
    assert!(
        w.log(&id)
            .contains("Créneau 1 : lever indisponible aujourd'hui, borne ignorée")
    );
    w.go(paris(23, 0));
    assert_eq!(w.sent(), vec![Command::Off]);
}

// ---- limits ----

/// A roomy limit: the Mac counts with 64-bit pointers, the box's figure is measured in QEMU.
struct Roomy;

impl ScriptRuntimes for Roomy {
    fn new_runtime(&self) -> Result<NewRuntime, String> {
        SystemRuntimes.new_runtime()
    }

    fn memory_limit(&self) -> usize {
        1024 * 1024
    }
}

#[test]
fn four_instances_of_the_script_stay_within_its_memory() {
    let w = World::new(MIDNIGHT + 12 * HOUR, true);
    let runtimes: Arc<dyn ScriptRuntimes> = Arc::new(Roomy);
    let clock: ScriptClock = Arc::new(|| 0);
    let (mut recipe, _) = ScriptRecipe::load(SCRIPT, &runtimes, &clock).unwrap();
    let names = RecipeName {
        id: "schedule-on-off".into(),
        name: "Schedule On/Off".into(),
    };
    let clock = Clock {
        mono: w.rig.now,
        wall: w.rig.now,
    };
    // Three windows, six boundaries, all settings used.
    let params = json!({
        "zone": w.g.zone, "equipments": [w.g.light],
        "slot1_start_kind": "sunrise", "slot1_start_offset": -30, "slot1_end_kind": "sunset", "slot1_end_offset": 15,
        "slot2_start_kind": "time", "slot2_start_time": "12:00", "slot2_end_kind": "time", "slot2_end_time": "13:00",
        "slot3_start_kind": "time", "slot3_start_time": "20:00", "slot3_end_kind": "time", "slot3_end_time": "23:30",
    });
    let empty = recipe.held_bytes();
    let host = NativeHost::new(&w.rig.home, Id::ROOT_ZONE, &names, clock.mono, clock.wall);
    for handle in 0..MAX_INSTANCES_PER_SCRIPT as u32 {
        recipe.start(&host, handle, &params, clock).unwrap();
    }
    assert_eq!(recipe.instance_count(), MAX_INSTANCES_PER_SCRIPT);
    // Every boundary fires once over a day.
    for step in 1..=4u64 {
        let clock = Clock {
            mono: clock.mono + step * 7 * HOUR,
            wall: clock.wall + step * 7 * HOUR,
        };
        let host = NativeHost::new(&w.rig.home, Id::ROOT_ZONE, &names, clock.mono, clock.wall);
        for handle in 0..MAX_INSTANCES_PER_SCRIPT as u32 {
            recipe.tick(&host, handle, clock).unwrap();
        }
    }
    let held = recipe.held_bytes();
    println!(
        "schedule-on-off: {} bytes of source, {empty} bytes loaded, {held} bytes with {MAX_INSTANCES_PER_SCRIPT} instances (the box's limit is {SCRIPT_MEMORY_LIMIT})",
        SCRIPT.len()
    );
    assert!(SCRIPT.len() < 32 * 1024, "{}", SCRIPT.len());
    assert!(held < SCRIPT_MEMORY_LIMIT, "{held}");
}
