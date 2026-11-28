//! Worst case of the engine's memory (spec 015 R3, decision D23). One home with every memory bound
//! of the engine at its maximum, under a ceiling of 1 MB. The realistic home (300 KB, D20) is
//! `memory_ceiling.rs`; the two are never added: the box is in one state or the other. The Mac's
//! 64-bit figure is an upper estimate of the ESP32's 32-bit one. This file holds its own counting
//! allocator, so nothing else of the crate's tests may live in it; its two tests take a lock, so
//! that the second never runs while the first measures.
//!
//! Every `pub const MAX_*` of `crates/core/src`, as `file::NAME`, and of `crates/recipe-sdk/src`
//! (spec 041), as `recipe-sdk/src/file::NAME`, and how the test reaches it
//! (`every_memory_bound_is_listed_in_the_header` fails when one is missing from this list):
//!
//! - `devices/registry.rs::MAX_DEVICES`: 64 devices; a 65th is refused at the end.
//! - `devices/registry.rs::MAX_DATA`: 8 data on each device, keys of 32 characters.
//! - `devices/registry.rs::MAX_ORDERS`: 4 orders on each device.
//! - `devices/registry.rs::MAX_NAME_CHARS`: device names of 64 characters.
//! - `zones/mod.rs::MAX_ZONES`: root plus 31 zones through the API; a 33rd is refused.
//! - `zones/mod.rs::MAX_NAME_CHARS`: zone names of 64 characters.
//! - `zones/mod.rs::MAX_ICON_CHARS`: zone icons of 32 characters.
//! - `zones/mod.rs::MAX_DESCRIPTION_CHARS`: zone descriptions of 500 characters.
//! - `equipments/mod.rs::MAX_EQUIPMENTS`: 64 equipments through the API; a 65th is refused.
//! - `equipments/mod.rs::MAX_BINDINGS`: 8 data and 8 order bindings on 32 equipments, from 4
//!   devices each (which hold more, 32 data and 16 orders, so the bound is what cuts); the 8 data
//!   bindings are bound on all 64.
//! - `equipments/mod.rs::MAX_ALIAS_CHARS`: aliases of 32 characters (the device keys), more
//!   distinct ones than the table that shares aliases holds, so none is shared.
//! - `equipments/extras.rs::MAX_EXTRAS_BYTES`, `equipments/extras.rs::MAX_EXTRAS_TOTAL_BYTES`,
//!   `equipments/extras.rs::MAX_ID_CHARS`: Sowel's optional equipment fields, 64 sets of 128
//!   bytes (8192 in all, a vehicle id of 64 characters and a padded object in each), held beside
//!   the home since the API cannot set an object; the 1024 bytes of one equipment are far under
//!   the total's share, the total is the bound that counts.
//! - `recipes/mod.rs::MAX_INSTANCES`: 16 instances, 4 of the `motion-light` script and 4 of each of
//!   three copies of it under other ids (a script recipe holds 4, D85); a 17th is refused.
//! - `recipes/mod.rs::MAX_PARAMS_BYTES`: params of 2 KB each (nested padding, as
//!   `recipe_params_memory.rs`); the API refuses more, so this is the largest accepted size.
//! - `recipes/mod.rs::MAX_LOG_ENTRIES`: 20 entries in each instance's log, by button presses.
//! - `recipes/mod.rs::MAX_LOG_MESSAGE_BYTES`: the log is measured as it is, then the worst case
//!   (every entry at 200 bytes) is added, as `memory_ceiling.rs` does.
//! - `recipes/mod.rs::MAX_SCRIPT_RECIPES`, `recipes/mod.rs::MAX_INSTANCES_PER_SCRIPT`,
//!   `recipes/script/limits.rs::MAX_SCRIPT_BYTES`,
//!   `recipes/script/limits.rs::MAX_TIMERS`, `recipes/script/limits.rs::MAX_LOG_LINES_PER_CALL`,
//!   `recipes/script/limits.rs::MAX_ALIAS_BYTES`, `recipes/script/limits.rs::MAX_END_LINE_BYTES`
//!   (the end of a script's call, checked by the engine: aliases of 64 bytes, log lines of 1600),
//!   `recipes/script/slots.rs::MAX_SLOT_EQUIPMENTS`, `recipes/script/slots.rs::MAX_SLOTS`,
//!   `recipes/script/slots.rs::MAX_TEXT_BYTES` (a `text` slot's value, 64 bytes, kept in the
//!   instance's params, which `MAX_PARAMS_BYTES` already bounds; `motion-light` has no `text` slot)
//!   (spec 041, script recipes): the script runtimes (4 of 224 KB on the box, 896 KB) are QuickJS's
//!   own memory, which the counting allocator below does not see (QuickJS allocates through the
//!   C library), on their own line of the budget. The engine's worst case runs 16 instances of four
//!   loaded `motion-light` scripts (spec 046, D81): it counts their Rust side (slots, listen sets,
//!   states, logs), not their runtimes; the script's own worst case is the third test of this
//!   file (`scripts_pushed_to_their_limit_with_four_instances_each`), which loads
//!   `MAX_SCRIPT_RECIPES` scripts, each with a ballast that takes it to about 85 % of its limit, and
//!   starts `MAX_INSTANCES_PER_SCRIPT` instances in each. The Mac's limit is `SYSTEM_MEMORY_LIMIT`,
//!   twice the box's (64-bit pointers): the line it prints is the Mac's, bounded by 2 times that
//!   limit; the box's 4 x 224 KB is checked in QEMU. The bounds are
//!   tested in `script_recipes.rs`.
//! - `recipes/mod.rs::MAX_CARRIED_EVENTS`: events left for the next turn when scripts ran out of
//!   time (at most 128, the home's own queue bound); empty while no script is slow, as in this
//!   state.
//! - The `motion-light` script's own bounds (`recipes/scripts/motion-light.js`, `validate`, spec
//!   046): 16 lights and 16 buttons in each instance.
//! - `recipe-sdk/src/bounds.rs::MAX_LISTEN_ZONES`, `recipe-sdk/src/bounds.rs::MAX_LISTEN_EQUIPMENTS`,
//!   `recipe-sdk/src/bounds.rs::MAX_LISTEN_DEVICES` (spec 041): the 16 instances' listen sets, each
//!   at 1 zone and 32 equipments (16 lights, 16 buttons) and the motion sources of the zone; the
//!   engine keeps each distinct entry once and 2 bytes per entry in each instance, measured here
//!   in the live bytes.
//! - `recipe-sdk/src/bounds.rs::MAX_STATE_BYTES`: the state text each of the 16 instances keeps
//!   (about 110 bytes with the three keys of `motion-light`), measured in the live bytes.
//! - `recipe-sdk/src/bounds.rs::MAX_ZONE_EQUIPMENT_IDS`,
//!   `recipe-sdk/src/bounds.rs::MAX_VALUE_TEXT_BYTES`, `recipe-sdk/src/bounds.rs::MAX_LOG_LINES_PER_CALL`,
//!   `recipe-sdk/src/bounds.rs::MAX_LOG_LINE_BYTES`: texts that live for one call of a recipe (a view
//!   of an equipment, the ids of a zone, the lines a call logs); the calls of the worst case's
//!   instances are in the peak, each bound is tested in `solite-recipe-sdk`.
//! - `recipe-sdk/src/bounds.rs::MAX_PARAMS_DEPTH`: the nesting of params, checked before they are
//!   parsed a second time; not memory of its own (the params of 2 KB nested as deep as it allows
//!   are what `recipe_params_memory.rs` measures).
//! - `home/events.rs::MAX_EVENTS`: more reports than the queue holds, not handed over (128 kept).
//! - `home/events.rs::MAX_TEXT_BYTES`: a second queue, held alive, of 128 events whose text is
//!   cut at 32 bytes (no measure carries a text, so the home's own queue never holds one).
//! - `orders/queue.rs::MAX_ORDERS`: 2 orders of the API waiting in the queue.
//! - `orders/queue.rs::MAX_UNATTENDED`: 32 orders of the core waiting, 16 commands each.
//! - `live/mod.rs::MAX_CLIENTS`: 4 clients connected; a 5th is refused.
//! - `live/mod.rs::MAX_QUEUED_EVENTS`: a count of 256 events; the bytes bound below is reached
//!   first (an event is far larger than 32 bytes), so the count is never the limit.
//! - `live/mod.rs::MAX_QUEUED_BYTES`: each client's queue full, and one batch in flight.
//! - `api/mod.rs::MAX_BODY_BYTES`: one request body of this size held for each of 4 HTTP workers
//!   (the worker count is the host's, 4, as `orders/queue.rs` says).
//! - `radio/measures.rs::MAX_MEASURES_PER_FRAME`: each button press carries 16 measures.
//! - `radio/ash.rs::MAX_PAYLOAD`, `radio/ash.rs::MAX_PENDING`, `radio/ash.rs::MAX_DELIVERED`,
//!   `radio/commands.rs::MAX_COMMANDS`: the radio's queues are not driven here (they need a
//!   transport); their bytes are an estimate by arithmetic, `RADIO_ESTIMATE_BYTES`, added to the
//!   peak.
//! - `radio/ash.rs::MAX_TIMEOUTS`: a count of attempts, no memory.
//! - `radio/serial.rs::MAX_RADIO_RESTARTS`: at most 3 restart times kept (24 bytes), no memory
//!   worth driving here.
//! - `radio/ezsp.rs::MAX_PROTOCOL_VERSION`: a protocol version number, no memory.
//! - `network.rs::MAX_SETTING_BYTES`: the network setting is read from 256 bytes at most; one
//!   short-lived text at boot, far below the rest, not part of the running engine.
//! - `network.rs::MAX_WIFI_BYTES`, `network.rs::MAX_CONSOLE_LINE`: the Wi-Fi settings text (256
//!   bytes) and a console line (512 bytes) are short-lived texts read at boot or from the serial
//!   console, far below the rest, not part of the running engine.
//! - `api/slots.rs::MAX_WORKERS`: the HTTP workers' count, 4, the one `api/mod.rs::MAX_BODY_BYTES`
//!   multiplies.
//! - `live/session.rs::MAX_INCOMING_BYTES`: a client's incoming frame, 1 KB, inside the 2 KB read
//!   buffer counted with `live/mod.rs::MAX_QUEUED_BYTES`.
//! - `settings/mod.rs::MAX_HOME_NAME_CHARS`, `settings/mod.rs::MAX_MANIFEST_URL_CHARS`: the nine
//!   settings at their longest (a name of 64 characters, numbers at the length of their pattern,
//!   a manifest address and a catalogue address of 512 characters), `settings.json` written.
//! - `recipes/install/catalogue.rs::MAX_CATALOGUE_BYTES`, `recipes/install/catalogue.rs::MAX_ENTRIES`,
//!   `recipes/install/catalogue.rs::MAX_ENTRY_URL_CHARS`,
//!   `recipes/install/catalogue.rs::MAX_ENTRY_REPO_CHARS`, `recipes/install/catalogue.rs::MAX_ENTRY_TAGS`,
//!   `recipes/install/catalogue.rs::MAX_ENTRY_TAG_CHARS`,
//!   `recipes/install/catalogue.rs::MAX_ENTRY_LABEL_CHARS` (spec 059: an entry's `repo`, tags,
//!   `category` and `author`, inside the same 32 KB of the catalogue), `recipes/install/file.rs::MAX_FILE_BYTES`,
//!   `recipes/install/file.rs::MAX_SOURCE_BYTES`, `recipes/install/mod.rs::MAX_REFUSED`,
//!   `recipes/script/load.rs::MAX_INSTALLED_ID_CHARS` (spec 042, installed script recipes): the
//!   catalogue is read whole (32 KB) and held as 32 entries of at most that many bytes; a recipe
//!   job holds the signed file (32 KB) and its source (32 KB) for the length of the job, one job
//!   at a time (`Releases`' mark); the refused files are 8 names and reasons. By arithmetic, not
//!   driven here (`tests/recipe_install.rs` tests each bound): `INSTALL_ESTIMATE_BYTES`, added to
//!   the peak, rounded up to 152 KB (they sit in PSRAM, beside the engine's, and the two scripts'
//!   runtimes on their own line).
//! - `update/release.rs::MAX_MANIFEST_BYTES`, `update/release.rs::MAX_URL_CHARS`,
//!   `update/release.rs::MAX_REDIRECTS` (spec 039): a manifest of 16 KB is held by one check or
//!   pull at a time (one job, `Releases`' mark), beside the cache's one entry of about 2 KB; both
//!   are far under the engine's worst case and counted in `docs/memory-budget.md`, not driven
//!   here (`tests/release.rs` tests each bound); the redirects are a count, not memory.
//! - `update/release.rs::MAX_ETAG_CHARS` (spec 040): the first answer's `ETag` is kept for a
//!   resume, at most 256 characters (a longer one is dropped); 256 bytes for the length of a
//!   pull, no more than the manifest's share, not driven here (`tests/release.rs`).
//! - `auth/mod.rs::MAX_SESSIONS`, `auth/mod.rs::MAX_ATTEMPTS`: 8 sessions of 72 bytes and 10
//!   attempt times of 8 bytes, in RAM only (spec 029); about 700 bytes in all, far below the
//!   rest, so the worst case does not drive them (`tests/auth.rs` tests each bound).
//! - `auth/mod.rs::MAX_PASSWORD_CHARS`, `auth/mod.rs::MAX_PASSWORD_BYTES`,
//!   `auth/mod.rs::MAX_USERNAME_CHARS`, `auth/mod.rs::MAX_DISPLAY_NAME_CHARS`: the account's
//!   fields, inside a request body (`api/mod.rs::MAX_BODY_BYTES`, counted above); the hash itself
//!   keeps under 1 KB of stack and nothing on the heap that grows.
//! - `auth/mod.rs::MAX_ITERATIONS`: the most iterations a stored hash may declare, a count of
//!   rounds, no memory.
//! - `backup/mod.rs::MAX_ARCHIVE_BYTES`, `backup/mod.rs::MAX_FILE_BYTES`: the worst case's files as
//!   saved (devices, zones, equipments, recipes, settings) are summed: each under the file bound,
//!   all under the archive bound. The archive itself is streamed through one 4 KB buffer, never
//!   held (`full_backup_memory.rs` measures 1 MB each way under 32 KB), so it is not built here.
//! - `backup/mod.rs::MAX_FILES`, `backup/mod.rs::MAX_MANIFEST_BYTES`,
//!   `backup/mod.rs::MAX_BACKUP_ITERATIONS`, `backup/mod.rs::MAX_EXPORT_MS`: a count of files (the
//!   seven names), the manifest's text (under 4 KB, on the heap of one HTTP worker, as the
//!   update's buffer), a count of rounds and a time limit: no memory worth driving here.
//! - `update/mod.rs::MAX_UPLOAD_BYTES`: the longest update file an upload may declare; it is
//!   streamed through one 4096-byte buffer, never held (`update_memory.rs` measures a 1 MB
//!   installation under 32 KB), so the worst case does not drive it.
//! - `supervisor.rs::MAX_KEPT`: six restart times (48 bytes) in the supervisor's record, which
//!   lives in RTC memory (96 bytes in all), not in the heap: nothing to drive here.

mod worst_case;

use peak_alloc::PeakAlloc;
use serde_json::json;
use solite_core::api::{Reply, handle, handle_stream};
use solite_core::backup::{MAX_ARCHIVE_BYTES, MAX_FILE_BYTES};
use solite_core::devices::{DeviceWithDetails, Devices, Id, MAX_DEVICES};
use solite_core::equipments::{
    Alias, Extras, Field, MAX_BINDINGS, MAX_EXTRAS_BYTES, MAX_EXTRAS_TOTAL_BYTES, MAX_ID_CHARS,
    Piece, RawObject,
};
use solite_core::home::events::{Event, EventQueue, EventValue, MAX_EVENTS, MAX_TEXT_BYTES};
use solite_core::home::{Home, ListCursor};
use solite_core::live::{MAX_CLIENTS, MAX_QUEUED_BYTES};
use solite_core::orders::{
    DeviceCommand, MAX_ORDERS as MAX_API_ORDERS, MAX_UNATTENDED, OrderRequest, OrderSource,
};
use solite_core::radio::measures::{Measure, Value as MeasureValue};
use solite_core::radio::zcl::Command;
use solite_core::radio::{MeasureEvent, Sender};
use solite_core::recipes::{MAX_INSTANCES, MAX_INSTANCES_PER_SCRIPT, MAX_LOG_ENTRIES};
use solite_core::settings::MAX_HOME_NAME_CHARS;
use solite_core::zones::MAX_ZONES;
use worst_case::{T0, created_id, device, key, request};

/// The process counts every thread's bytes: this file holds the engine's test alone, so that no
/// other test's thread (its end, its output, its frees) moves the peak; the checks that share its
/// home are in `memory_worst_case_checks.rs`.
#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

/// The ceiling of the engine with every bound at its maximum, decision D23.
const CEILING_BYTES: usize = 1024 * 1024;
const MAX_EQUIPMENTS_API: usize = 64;
/// HTTP workers of the host, each reading one body at most `api::MAX_BODY_BYTES` long.
const HTTP_WORKERS: usize = 4;
/// The radio's queues, by arithmetic and not measured: about 97 frames of up to 223 bytes (the
/// inbox 64, the callbacks 16, the ASH pending 8 and delivered 8, the one being read) with their
/// 24-byte headers, about 24 KB; the coordinator's records (32 devices in pairing, 64 addresses,
/// 64 activities, 32 answered, 32 ignored) about 15 KB; the commands (8 in flight, 16 results, 16
/// reads, 64 routes) about 5 KB. Rounded up to 48 KB.
const RADIO_ESTIMATE_BYTES: usize = 48 * 1024;
/// The recipe installs (spec 042), by arithmetic: the catalogue's 32 KB body while it is parsed and
/// its 32 entries held (about 48 KB), one job's signed file and source (64 KB), the refused
/// files' names and reasons (under 2 KB). Rounded up to 152 KB (spec 053: 128 KB before the files grew to 32 KB; the sum is 146 KB).
const INSTALL_ESTIMATE_BYTES: usize = 152 * 1024;

fn eui(index: usize) -> [u8; 8] {
    [
        0x00,
        0x12,
        0x4B,
        0x00,
        0x12,
        0x34,
        0x56,
        u8::try_from(index).unwrap_or(0),
    ]
}

fn voltage(device: usize, value: f64) -> MeasureEvent {
    MeasureEvent {
        sender: Sender::Eui64(eui(device)),
        endpoint: 1,
        measures: vec![Measure {
            kind: "voltage",
            value: MeasureValue::Number(value),
            unit: "mV",
            device_value: None,
        }],
    }
}

#[test]
fn the_engine_with_every_bound_at_its_maximum_stays_under_1_mb() {
    let mut calls = 0u32;
    let mut random = move |buffer: &mut [u8]| -> Result<(), String> {
        calls += 1;
        buffer.fill(0x5A);
        buffer[..4].copy_from_slice(&calls.to_be_bytes());
        Ok(())
    };
    let base = ALLOCATOR.current_usage();
    ALLOCATOR.reset_peak_usage();

    // Devices: 64, each with 8 data and 4 orders.
    let mut counter = 0u32;
    let list: Vec<DeviceWithDetails> = (0..MAX_DEVICES).map(|i| device(i, &mut counter)).collect();
    let devices = Devices::from_list(list);
    assert_eq!(devices.list().len(), MAX_DEVICES);
    let device_ids: Vec<String> = devices
        .list()
        .iter()
        .map(|d| d.device.id.to_string())
        .collect();
    let mut home = Home::from_devices(devices, T0);

    // Zones: the root and 31 others, with the longest name, icon and description.
    let mut zone_ids = Vec::new();
    for i in 0..MAX_ZONES - 1 {
        let body = json!({
            "name": format!("{:z<64}", format!("Zone {i:02}")),
            "icon": "i".repeat(32),
            "description": "d".repeat(500),
        })
        .to_string();
        let created = handle(
            &mut home,
            &request("POST", "/api/v1/zones", body.as_bytes()),
            T0,
            &mut random,
        );
        assert_eq!(created.status, 201, "{created:?}");
        zone_ids.push(created_id(&created));
    }
    assert_eq!(home.zones().list().len(), MAX_ZONES);

    // Equipments: 64. 0 to 15 are the lights (one device each, in the first zone), 16 to 31 the
    // buttons (one device each), 32 to 63 bind four devices each, so 16 order candidates, 8 bound.
    for i in 0..MAX_EQUIPMENTS_API {
        let (kind, zone, devices): (&str, &String, Vec<&String>) = if i < 16 {
            ("light_onoff", &zone_ids[0], vec![&device_ids[i]])
        } else if i < 32 {
            // Buttons of the first zone: the script's slot wants a button of the instance's zone.
            ("button", &zone_ids[0], vec![&device_ids[i]])
        } else {
            let first = 32 + (i - 32) % 8 * 4;
            (
                "sensor",
                &zone_ids[i % 31],
                (first..first + 4).map(|d| &device_ids[d]).collect(),
            )
        };
        let body = json!({
            "name": format!("{:E<64}", format!("Equipment {i:02}")),
            "type": kind,
            "zoneId": zone,
            "deviceIds": devices,
        })
        .to_string();
        let created = handle(
            &mut home,
            &request("POST", "/api/v1/equipments", body.as_bytes()),
            T0,
            &mut random,
        );
        assert_eq!(created.status, 201, "{created:?}");
    }
    assert_eq!(home.equipments().list().len(), MAX_EQUIPMENTS_API);
    assert!(
        home.equipments()
            .list()
            .iter()
            .all(|e| e.data_bindings.len() == MAX_BINDINGS)
    );
    assert!(
        home.equipments().list()[32..]
            .iter()
            .all(|e| e.order_bindings.len() == MAX_BINDINGS)
    );
    let equipment_ids: Vec<String> = home
        .equipments()
        .list()
        .iter()
        .map(|e| e.equipment.id.to_string())
        .collect();
    let lights: Vec<String> = equipment_ids[..16].to_vec();
    let buttons: Vec<String> = equipment_ids[16..32].to_vec();

    // The buttons' `voltage` is rebound as `action`.
    for button in &buttons {
        let (binding, data) = {
            let item = home.equipments().get(button).unwrap();
            let binding = item
                .data_bindings
                .iter()
                .find(|b| b.alias == "voltage")
                .unwrap();
            (binding.id.to_string(), binding.device_data_id.to_string())
        };
        let path = format!("/api/v1/equipments/{button}/data-bindings/{binding}");
        assert_eq!(
            handle(&mut home, &request("DELETE", &path, b""), T0, &mut random).status,
            204
        );
        let path = format!("/api/v1/equipments/{button}/data-bindings");
        let body = format!(r#"{{"deviceDataId":"{data}","alias":"action"}}"#);
        assert_eq!(
            handle(
                &mut home,
                &request("POST", &path, body.as_bytes()),
                T0,
                &mut random
            )
            .status,
            201
        );
    }

    // The optional fields of Sowel's equipments (spec 027) at their total bound: 64 equipments of
    // 128 bytes each, 8192 in all, each with a vehicle id of the longest length and an object
    // padded to the size. They are held here beside the home, not in it (the API cannot set an
    // object, a file can), so their heap bytes are counted the same.
    let extras: Vec<Box<Extras>> = (0..MAX_EQUIPMENTS_API)
        .map(|i| {
            let mut extras = Extras::default();
            extras.set(
                Field::UsualVehicleId,
                Some(Piece::Text("v".repeat(MAX_ID_CHARS).into())),
            );
            let pad = |n: usize| RawObject::new(&format!(r#"{{"a":"{}"}}"#, "p".repeat(n)));
            let mut padding = 0;
            loop {
                extras.set(Field::EnergyProfile, pad(padding).map(Piece::Object));
                match extras.json_len().cmp(&128) {
                    std::cmp::Ordering::Less => padding += 1,
                    _ => break,
                }
            }
            assert_eq!(extras.json_len(), 128, "equipment {i}");
            Box::new(extras)
        })
        .collect();
    let extras_json_bytes: usize = extras.iter().map(|e| e.json_len()).sum();
    assert_eq!(extras_json_bytes, MAX_EXTRAS_TOTAL_BYTES);
    assert!(extras.iter().all(|e| e.json_len() <= MAX_EXTRAS_BYTES));

    // Recipes: 16 instances, each with 16 lights, 16 buttons and params padded to 2 KB. A script
    // recipe holds 4 instances (D85): the `motion-light` script and three copies of it under other ids.
    // The Rust side of the instances is what this counts; QuickJS's own memory is not seen here.
    let motion_light = include_str!("../../../recipes/scripts/motion-light.js");
    let recipe_ids = [
        "motion-light",
        "motion-light-b",
        "motion-light-c",
        "motion-light-d",
    ];
    for id in recipe_ids {
        let text = motion_light.replace("id: \"motion-light\"", &format!("id: \"{id}\""));
        let clock: solite_core::recipes::ScriptClock = std::sync::Arc::new(|| 0);
        home.load_script_recipe(
            &text,
            std::sync::Arc::new(solite_core::recipes::script::SystemRuntimes),
            clock,
        )
        .unwrap();
    }
    let mut instance_ids = Vec::new();
    for n in 0..MAX_INSTANCES {
        let mut params = json!({
            "zone": zone_ids[0], "lights": lights, "buttons": buttons,
            "timeout": "20s", "maxOnDuration": "5m"
        })
        .to_string();
        params.pop();
        let mut i = 0;
        while params.len() + 20 < 2048 {
            params.push_str(&format!(r#","{i}":{{"a":0}}"#));
            i += 1;
        }
        params.push('}');
        assert!(
            params.len() <= 2048 && params.len() > 2000,
            "{}",
            params.len()
        );
        let recipe_id = recipe_ids[n / MAX_INSTANCES_PER_SCRIPT];
        let body = format!(r#"{{"recipeId":"{recipe_id}","params":{params}}}"#);
        let created = handle(
            &mut home,
            &request("POST", "/api/v1/recipe-instances", body.as_bytes()),
            T0,
            &mut random,
        );
        assert_eq!(created.status, 201, "{created:?}");
        instance_ids.push(created_id(&created));
    }
    // The lights keep their bindings: since spec 024 a light that loses its toggle binding
    // disables the instance (R4), so the long error lines of a missing binding no longer come;
    // the logs fill with the switching lines, and the worst case (every entry at 200 bytes) is
    // added to the peak below.
    // A button is pressed until every log is full; each press is a frame of 16 measures.
    let press = MeasureEvent {
        sender: Sender::Eui64(eui(16)),
        endpoint: 1,
        measures: (0..solite_core::radio::measures::MAX_MEASURES_PER_FRAME)
            .map(|i| Measure {
                kind: if i == 0 { "voltage" } else { "battery" },
                value: MeasureValue::Number(3000.0),
                unit: "mV",
                device_value: None,
            })
            .collect(),
    };
    let mut now = T0;
    for _ in 0..60 {
        now += 1000;
        home.measures(&press, now, &mut random).unwrap();
        home.recompute(now);
        home.run_recipes(now, now);
    }
    // The queue of events full: more reports than it holds, not handed over.
    for _ in 0..MAX_EVENTS + 2 {
        home.measures(&press, now, &mut random).unwrap();
    }
    assert_eq!(home.events().len(), MAX_EVENTS);

    // The events' text at its bound: a queue held alive, outside the home's own.
    let mut texts = EventQueue::default();
    let long = "t".repeat(MAX_TEXT_BYTES * 4);
    for i in 0..MAX_EVENTS {
        texts.push(Event::Equipment {
            equipment: Id::from_random([u8::try_from(i).unwrap_or(0); 16]),
            alias: Alias::new(&key("event", i, 0)),
            value: EventValue::from_json(&json!(long)),
        });
    }
    assert!(matches!(
        texts.iter().next(),
        Some(Event::Equipment { value: EventValue::Text(t), .. }) if t.len() == MAX_TEXT_BYTES
    ));

    // The queue of orders: the API's and the core's, full. 16 commands for each.
    let order = |alias: &str| OrderRequest {
        equipment: Id::from_random([1; 16]),
        alias: alias.into(),
        value: json!("ON"),
        source: Some(OrderSource::Recipe {
            instance_id: Id::from_random([2; 16]),
            recipe_name: "Motion light".into(),
            recipe_id: "motion-light".into(),
        }),
        commands: (0..16)
            .map(|i| DeviceCommand {
                eui64: eui(i),
                nwk: Some(0x1234),
                endpoint: 1,
                command: Command::On,
            })
            .collect(),
    };
    let queue = home.orders();
    let mut waits = Vec::new();
    for _ in 0..MAX_API_ORDERS {
        waits.push(queue.push_waited(order("state"), T0).expect("room"));
    }
    // The recipes already queued orders of their own: the queue is filled up to its bound.
    while queue.push_unattended(order("state"), T0).is_ok() {}
    assert_eq!(queue.unattended_len(), MAX_UNATTENDED);
    assert!(
        queue.push_waited(order("state"), T0).is_err(),
        "a third order of the API"
    );

    // The HTTP workers' bodies, at the largest the core accepts, and the radio's queues.
    let bodies: Vec<Vec<u8>> = (0..HTTP_WORKERS)
        .map(|_| vec![b' '; solite_core::api::MAX_BODY_BYTES])
        .collect();
    let radio = vec![0xA5u8; RADIO_ESTIMATE_BYTES];

    // Live clients: all four, listening to everything, queues full (a report from a new device
    // away from overflowing), and a batch being written by each: an event taken and held, while
    // the queue fills up again.
    let hub = home.live();
    let clients: Vec<_> = (0..MAX_CLIENTS)
        .map(|_| hub.connect().expect("a free slot"))
        .collect();
    for client in &clients {
        assert!(hub.subscribe(
            *client,
            r#"{"type":"subscribe","topics":["devices","equipments","zones","recipes"]}"#
        ));
    }
    let mut report = 0u32;
    let mut fill_queues = |home: &mut Home| {
        while hub.queued(clients[0]).1 + 750 <= MAX_QUEUED_BYTES {
            report += 1;
            let device = usize::try_from(report).unwrap_or(0) % 32;
            home.measures(
                &voltage(device, 2000.0 + f64::from(report)),
                now,
                &mut |b| {
                    b.fill(0x33);
                    Ok(())
                },
            )
            .unwrap();
            assert!(report < 2000, "the queues never filled");
        }
    };
    fill_queues(&mut home);
    let mut in_flight = Vec::new();
    for client in &clients {
        assert_eq!(hub.close_request(*client), None, "not overflowed");
        let (events, bytes) = hub.queued(*client);
        assert!(bytes + 750 > MAX_QUEUED_BYTES, "full: {bytes} bytes");
        assert_eq!(hub.begin_batch(*client), events);
        in_flight.push(hub.pop(*client).expect("an event to write"));
    }
    fill_queues(&mut home);
    let (queued_events, queued_bytes) = hub.queued(clients[0]);

    // The logs: `MAX_LOG_ENTRIES` entries each; the worst case (every entry 200 bytes) is added to the peak.
    let mut log_bytes = 0usize;
    for instance in &instance_ids {
        let log = home.recipes().log(instance, MAX_LOG_ENTRIES);
        assert_eq!(log.len(), MAX_LOG_ENTRIES);
        log_bytes += log.iter().map(|e| e.message.len() + 56).sum::<usize>();
    }
    let worst_log_bytes: usize = MAX_INSTANCES * MAX_LOG_ENTRIES * (200 + 56);

    // The long lists are streamed, one chunk alive at a time; the others are whole, one at a
    // time.
    let mut sizes = Vec::new();
    for path in ["/api/v1/equipments", "/api/v1/devices"] {
        let reply = handle_stream(&mut home, &request("GET", path, b""), T0, &mut random);
        let Reply::List(mut cursor) = reply else {
            panic!("{path} is streamed");
        };
        let mut total = 0;
        while let Some(chunk) = cursor.next_chunk(&home).unwrap() {
            total += chunk.len();
        }
        sizes.push(total);
    }
    for path in ["/api/v1/zones", "/api/v1/zones/aggregation"] {
        let answer = handle(&mut home, &request("GET", path, b""), T0, &mut random);
        assert_eq!(answer.status, 200, "{path}");
    }
    // The save path, chunk by chunk.
    let mut cursors = [
        ListCursor::saved_devices(&home),
        home.take_zones_cursor().expect("zones to write"),
        home.take_equipments_cursor().expect("equipments to write"),
    ];
    // The bytes of each file as saved: the archive of spec 035 must hold them (R9).
    let mut file_bytes: Vec<usize> = Vec::new();
    for cursor in &mut cursors {
        let mut bytes = 0;
        while let Some(chunk) = cursor.next_chunk(&home).unwrap() {
            bytes += chunk.len();
        }
        file_bytes.push(bytes);
    }
    let mut cursor = ListCursor::recipe_instances(&home);
    while cursor.next_chunk(&home).unwrap().is_some() {}
    let mut cursor = home
        .take_recipes_cursor()
        .expect("the recipes are to be written");
    let mut recipes_bytes = 0;
    while let Some(chunk) = cursor.next_chunk(&home).unwrap() {
        recipes_bytes += chunk.len();
    }
    file_bytes.push(recipes_bytes);

    // The settings (spec 026, 039): the eight keys at their longest, `settings.json` written, the sun
    // computed.
    let settings = json!({
        "home.name": "n".repeat(MAX_HOME_NAME_CHARS),
        "home.latitude": "-89.123456789012",
        "home.longitude": "-179.123456789012",
        "home.timezone": "America/Los_Angeles",
        "home.sunriseOffset": "-180",
        "home.sunsetOffset": "-180",
        "update.manifestUrl": format!("https://example.com/{}", "m".repeat(512 - 20)),
        "update.autoCheck": "false",
    })
    .to_string();
    let put = handle(
        &mut home,
        &request("PUT", "/api/v1/settings", settings.as_bytes()),
        T0,
        &mut random,
    );
    assert_eq!(put.status, 200, "{put:?}");
    let mut cursor = home
        .take_settings_cursor()
        .expect("the settings are to be written");
    let mut settings_json = String::new();
    while let Some(chunk) = cursor.next_chunk(&home).unwrap() {
        settings_json.push_str(&chunk);
    }

    // The worst case's files fit the full backup (spec 035, R9): each under `MAX_FILE_BYTES`,
    // all with `settings.json` under `MAX_ARCHIVE_BYTES`.
    file_bytes.push(settings_json.len());
    assert!(
        file_bytes.iter().all(|bytes| *bytes < MAX_FILE_BYTES),
        "a file of the worst case does not fit a backup: {file_bytes:?}"
    );
    assert!(
        file_bytes.iter().sum::<usize>() < MAX_ARCHIVE_BYTES,
        "the worst case's files do not fit a backup: {file_bytes:?}"
    );

    let measured = ALLOCATOR.peak_usage().saturating_sub(base);
    let peak = measured + RADIO_ESTIMATE_BYTES.saturating_sub(radio.len()) + INSTALL_ESTIMATE_BYTES;
    let worst = peak + worst_log_bytes.saturating_sub(log_bytes);

    // One past each collection's bound: refused, and the peak does not grow.
    let before = ALLOCATOR.peak_usage();
    let zone = handle(
        &mut home,
        &request("POST", "/api/v1/zones", br#"{"name":"One too many"}"#),
        T0,
        &mut random,
    );
    assert_eq!(zone.status, 400, "{zone:?}");
    let body = json!({"name": "Too many", "type": "sensor", "zoneId": zone_ids[0]}).to_string();
    let equipment = handle(
        &mut home,
        &request("POST", "/api/v1/equipments", body.as_bytes()),
        T0,
        &mut random,
    );
    assert_eq!(equipment.status, 400, "{equipment:?}");
    let body = json!({
        "recipeId": "motion-light-b",
        "params": {"zone": zone_ids[0], "lights": lights}
    })
    .to_string();
    let recipe = handle(
        &mut home,
        &request("POST", "/api/v1/recipe-instances", body.as_bytes()),
        T0,
        &mut random,
    );
    assert_eq!(recipe.status, 400, "{recipe:?}");
    assert_eq!(hub.connect(), None, "a fifth client");
    let notes = home
        .devices
        .identified(
            eui(200),
            Some("Manufacturer"),
            Some("Model"),
            T0,
            &mut random,
        )
        .unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(home.devices.list().len(), MAX_DEVICES);
    assert_eq!(home.zones().list().len(), MAX_ZONES);
    assert_eq!(home.equipments().list().len(), MAX_EQUIPMENTS_API);
    assert_eq!(
        ALLOCATOR.peak_usage(),
        before,
        "an item past a bound made the peak grow"
    );

    // Everything built above stays alive until here.
    assert_eq!(in_flight.len(), MAX_CLIENTS);
    assert_eq!(waits.len(), MAX_API_ORDERS);
    assert_eq!(bodies.len(), HTTP_WORKERS);
    assert_eq!(texts.len(), MAX_EVENTS);
    assert_eq!(extras.len(), MAX_EQUIPMENTS_API);
    println!(
        "memory worst case: measured peak {measured} bytes ({} KB), with the radio's estimate {peak} bytes ({} KB), with every log entry at 200 bytes {worst} bytes ({} KB), of which the optional equipment fields {extras_json_bytes} bytes of JSON; {MAX_CLIENTS} clients of {queued_events} events and {queued_bytes} bytes; logs {log_bytes} bytes; GET /equipments {} bytes, GET /devices {} bytes (streamed); settings.json {} bytes; ceiling {CEILING_BYTES}",
        measured / 1024,
        peak / 1024,
        worst / 1024,
        sizes[0],
        sizes[1],
        settings_json.len(),
    );
    assert!(settings_json.len() < 1024, "{settings_json}");
    assert!(worst < CEILING_BYTES, "worst case {worst} bytes");
}
