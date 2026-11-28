# Roadmap

What gets built next, and why, toward the true north in [CLAUDE.md](../CLAUDE.md). The place for
the future and for how far the work has got: the docs that describe the tool speak of the present
only. Planned and reviewed with `/solite-roadmap`: briefly after every feature, fully at every
milestone. A new need, at any time: one line in the inbox.

## Current milestone

**E. Solite in the box**. The user can then: see the garage of milestone B run inside the
Dongle Max itself, Mac switched off: the sensors report, the motion switches the relay, the
interface is served by the box.
Why now: the real unknowns are on the target (the ESP32's memory, the serial link to the radio,
Ethernet, storage in flash), and an autonomous box is the true north; the user asked to reach the
target quickly. Done when: the flashed Dongle Max runs the garage with the real devices for a
day, its original firmware saved and restorable.

Hardware: the Dongle Max (being delivered); the SNZB-02, a Sonoff motion sensor and the ZBMINIR2.
Flashing needs the user's explicit yes, once the original firmware is saved (decision D6).

| # | Increment | Done when | Status |
| - | --------- | --------- | ------ |
| E1 | Prepare the target, before the delivery: how the Dongle Max's ESP32 is wired (pins to the radio, the Ethernet chip, the Wi-Fi antenna, the button, the LEDs), how it is flashed and how its original firmware is saved and restored; the whole engine run in the firmware under QEMU with the simulated radio, its memory measured; the memory budget per item written in `docs/`, a test with every bound at its maximum, the placement fixed in `sdkconfig` (the engine in PSRAM, one TLS session at a time), the network settled: one image holds Wi-Fi and Ethernet, only one is active (setting `auto | wifi | ethernet`, `auto` by default: a cable at boot means Ethernet, else Wi-Fi; no switching while running), Wi-Fi built first, the memory budget set by the larger of the two (Wi-Fi), not their sum (first: removes the memory risk and the unknowns of the board without hardware) | A written hardware sheet, and the engine's memory line in QEMU for the realistic home | built (spec 015: hardware sheet, memory budget, worst-case test, partition table, network choice in the core; spec 016: the engine in the firmware under QEMU, peak 312 KB for the realistic home) |
| E2 | The real hardware from the Mac, the day of the delivery, nothing flashed: the pending demos of A3 to A5 and B2 to B8 (radio over TCP, SNZB-02, motion sensor, ZBMINIR2), defects fixed; then the stacked branches merged into `main`, pushed with the user's yes, the first CI run (second: proves EZSP in Rust on the real radio before the box) | The hand in front of the motion sensor switches the ZBMINIR2 from the Mac | demos run 2026-11-21, merged (pull requests 1 and 2); `setValue` defect open |
| E3a | The box's first flash (spec 020): `scripts/box.sh` identifies the Dongle Max, saves its original firmware, restores it once, flashes the box's image and a measuring image (the realistic home, saves written to the flash and timed), checks their boot log; the free memory at boot, the engine's peak, the FAT partition and the save stall measured on the box | The backup restored and the radio seen again over TCP; both images boot; the figures are in `docs/memory-budget.md` | script and tests built (spec 020); the run on the box needs the user's yes for each write |
| E3b | The radio over the serial link (UART): the engine on the real radio in the box; the serial link's buffers measured | A motion sensor paired and its measures seen, the radio driven by the box | done on the box 2026-11-22 (spec 021): radio at 115200 baud, network resumed, SNZB-03PR2 paired and its measures read. Not run: the 10-minute capture (4 minutes), a second boot, the booting-marker and power-cut cases, the restart ladder, a forming of a network, the ZBMINIR2 |
| E3c | The network (Wi-Fi; Ethernet is E3e), the HTTP and WebSocket API, the interface served by the box (precompressed), configured by `scripts/demo-garage.sh`; the internal RAM with Wi-Fi up and the 33 requests of an equipment with 16 + 16 bindings measured | The interface of the box opened from the Mac, the garage set up through the API | done 2026-11-22: garage set up through the API, motion switched the ZBMINIR2, `/ws` live in a browser, pairing by `box.sh pair`; the 33 requests never `Busy` but over 10 s in seven runs of eight, one request lost in two (inbox); lost link, clock line and error cases not run (see spec 023) |
| E3d | With no cable and no Wi-Fi settings the box opens its own access point `Solite-XXXX` with a page to pick the network (settings also written through the serial console, `box.sh wifi`, specs 022 and 023); a Wi-Fi that stays unreachable is retried, then the access point reopens after a few minutes; the pairing route (D28) answers on the box behind the token (spec 029, D49) | The garage runs in the box for a day, Mac off | |
| E3e | Ethernet (the IP101, RMII, pins of `docs/hardware.md`): a cable at boot under `auto` means Ethernet, the same server; the Ethernet memory figure measured | The box's interface opened over a cable, Wi-Fi settings absent | |

### Order of work while the box is away (from 2026-11-25)

E3d and E3e need the box, so the work goes on in the core and the interface, tested on the Mac, in
this order (each through `/solite-feature` or `/solite-fix`):

1. A batch of core fixes: done (spec 028).
2. J4: the password at commissioning: built (spec 029); its box checks are in the list below.
3. J2: equipments from the interface (detail page, bindings one by one, enable and disable): built (spec 030).
4. The colour cluster `0x0300` with a simulated bulb: built (spec 031); then J5 (signed updates, the core part): built (spec 032), its box check is in the list below.

### Tests to run with the box in hand

One session, the user present: `sh scripts/quiet.sh --tail 60 sh scripts/box-session.sh
--yes-flash --backup <the backup>` (spec 033), typed by the user (their D6 yes, D55), with
`SOLITE_HOME_LATITUDE`, `SOLITE_HOME_LONGITUDE` and `SOLITE_HOME_TIMEZONE` set; add
`--with-gestures` for the person's checks, `--factory-reset` for J4's reset. One pass, saves held
and flushed once per step (D39), `--keep-storage`. Each item says the step that runs it, or that it
stays a person's. The first run is expected to show route or timing mismatches (`/solite-fix`).

- Spec 025: the flash (B2), `measure-api.sh` inside `saves --budget 5` (B6), the boot log with
  its budget (B3). Run by the script.
- Spec 026: the full backup exported and its restore with a wrong password refused (spec 035), the location and time zone set, the sun times
  (B7), kept after a restart (B8). Run by the script. By day, motion does not switch the ZBMINIR2
  on with "Inactive during day": a person's (H3).
- Spec 027: the time zone then `GET /api/v1/system/timezone` (B7). Run by the script. The home
  lock per batch of measures timed: not run (the log line does not exist).
- Spec 023: the 4 requests in progress and the 5th `Busy`, a client that stops reading (B7): run
  by the script. The lost link (H4), a 1970 date after a boot with the access point off (H5), the
  unpowered relay (H6): a person's. A wrong password and `wifi --forget`: a person's, outside the
  script (they cut the box off the network). The console's long and unknown lines: not run
  (`box.sh` sends no raw line; inbox).
- Spec 021: a second boot (B5) run by the script; the power cut (H1), the ZBMINIR2 (H2): a
  person's; a forming of the network: not run (it needs `storage` erased).
- The 33 requests over 10 s and the lost request: B6 prints the total and the slowest; where the
  time goes stays to read in the serial log (E3d's day-long run waits for it).
- Spec 028: devices' dates and status after the clock (B7). Run by the script (the ZBMINIR2
  powered at boot).
- Spec 029 (J4): the setup within 10 minutes of a power-on inside `saves --budget 3`, one login
  timed, 401 without a token, `/ws` 4001 without the token and live with it, the pairing route
  behind the token, the 429 (B4); no token in the serial log (B5); the factory reset with
  `--factory-reset` (B10). Run by the script. The browser's login, live page and Discovery's "Add a
  device" (H7), a software restart asks to log in again (H8): a person's.
- Spec 031 (needs a real Zigbee 3.0 colour bulb, which the user may not have; from the Mac over
  the Dongle Max's radio, nothing flashed): pair it; `reporting: ... light_color`; `color_temp`
  and `color` shown; `POST .../orders/color_temp {"value":250}` and
  `.../orders/color {"value":{"x":0.2,"y":0.6}}` change the bulb and are read back; a hue and
  saturation order; an order while the bulb is off (does it apply?); the proposal `light_color`.
  Note the bulb's model and what was seen in spec 031. A person's (H11).
- Spec 032 (J5): the USB flash with the key (B2, it brings the rollback bootloader), a flipped
  signature refused, the next version installed from a signed file, running and confirmed, a
  panicking image installed and rolled back (B8), the `heap:` line of the updated image (B9). Run
  by the script. An unplug during the minute after an update (H9) and the Firmware update card's
  error (H10): a person's. The stack high-water marks of an API worker and of the OTA thread just
  after an install: not run (B9 reads a fresh boot). Each attempt (the flipped signature, the
  image with one byte changed, the valid image) logs `update: thread stack <n> of 16384 bytes
  never used` and no `stack overflow` line; before the fix of 2026-11-25 an API worker overflowed
  in the signature check and the box restarted (D60).
- Spec 035 (the full backup), the box check, the user present: a first run of the script checks
  B7.1 (the export, then a restore with a wrong password refused). The rest is a person's, on the
  radio image with devices paired: export a backup with a password from the Settings page (the
  export takes about 11 to 14 s: the serial log shows `backup: key derived in <n> ms`, then
  `backup: thread stack <n> of 10240 bytes never used`, and no `task_wdt` or `assert failed`
  line; before the fix of 2026-11-25 the box stopped on `assert failed: ...cache_utils.c:127`
  after 70 to 88 s, D59); restore it on the same box (the
  box restarts, log in again, the devices, rooms, equipments and recipes are back); factory-reset
  the box, or flash a second one while the first is off, restore the file there; count the devices
  that report again without pairing, and those that need pairing (D57's best effort), and note the
  numbers in spec 035; read the flash write count of the restore (`box.sh log`).
- Spec 036 (the supervised engine), the box check, the user present, one pass, no loop (D39), on
  the radio image of the spec, flashed with the user's yes: `GET /api/v1/system/supervisor` after
  the flash shows `bootCause` `power-on` and `mode` `normal`; one `panic` line typed by the person
  on the serial console (`box.sh` sends no raw line) restarts the box once, and the route then
  shows `bootCause` `panic` and one restart in the hour; the flash writes of that incident are at
  most one save of the files due plus the boot marker written at the next boot (`box.sh log`). The
  safe mode and the watchdog itself are not forced on the box: they are the Mac's tests. The route
  check stays a person's (the box session does not read it).
- Spec 039 (the box pulls a release), the box check, the user present, one pass, no loop (D39).
  (1) With a test key: the radio image built with a test public key (`SOLITE_UPDATE_PUBLIC_KEY`),
  which is a USB flash and **needs the user's explicit yes** (D6, D55), and a flash back with an
  image of the release key afterwards, or the box refuses real releases. A Mac HTTP server on the
  local network (`python3 -m http.server`) serves a `manifest.json` and a `.sup` made with
  `solite-host --update-sign` and that test key, a version above the box's; `PUT /settings`
  `update.manifestUrl` to it; `POST /system/version/check` (200, `updateAvailable` true), then an
  empty `POST /system/update`: the answer, the restart, the probation and its confirmation a
  minute later. (2) The real address (the default setting; the repository must be public): until a
  release exists GitHub answers 404 at once, so `POST /system/version/check` answers 500 `Update
  check failed: ...manifest.json: 404` after the TLS handshake, which proves the certificate
  bundle and the TLS memory; once a release exists, the redirects to the asset host too. Read, in
  `box.sh log --image radio`, the `update-check: thread stack <n> of 20480 bytes never used` and
  `update-pull: ...` lines (D68's 20 KB, by reasoning until then; the QEMU run left the figure in
  `docs/memory-budget.md` before the handshake) and the `heap:` line's internal RAM minimum during
  the check, then fill the TLS row of `docs/memory-budget.md`. The automatic check (`update.autoCheck`
  `"true"`) is read from the log 10 minutes after a boot.
- Spec 040 (rotation, the card, the resume), the box check, the user present, one pass, no loop
  (D39), with the test-key image of spec 039's item (1) already flashed. (1) **The resume**: a
  small Python server on the LAN serves the manifest and the `.sup` and closes the connection after
  N bytes on the first request, honouring `Range` on the next; `POST /system/update` (empty body):
  the log's `update: download cut at <count> of <size> bytes (...), resuming (1/3)` line, the
  install, the restart. (2) **The card, from the browser**: Check for updates, Update to x.y.z, the
  elapsed count, the restart, the login screen after it, the new version shown. (3) **The
  rotation**, which needs two box images (two test keys, A and B) and a USB flash, **the user's
  explicit yes** (D6, D55): the box on the image of key A takes a bridge signed by A that carries
  key B (from the LAN server), confirms it, then refuses a file signed by A (`Invalid signature`)
  and accepts one signed by B; the Mac's chain test (`crates/host/tests/rotation.rs`) proves the
  logic, this proves the key the radio image compiles. (4) **Observation (a) of spec 039's first
  run**: a check right after a restart, the log line naming the address it read (`update: check:
  latest <x>, running <y> (<url>)`), to see which server answered. Observation (b), a PUT of one
  setting that seemed to delete `home.name` and the location, was not reproduced on the Mac
  (`crates/host/tests/settings_merge.rs`, across a restart); the interface sends the whole map
  anyway, and the storage-loss line of the inbox (2026-11-25) stays open.
- Then E3d (access point `Solite-XXXX`, retry) and E3e (Ethernet), and the box's button wired
  (its pin; a short press confirms through `ButtonPresence`, a long press calls
  `Home::factory_reset`; spec 029, D48).

## Next milestone

**J. The autonomous box**. The user can then: set Solite up from the browser only, on the box:
add a device, make the rooms and equipments, set the recipes, protect it with their own
password, and update it from afar.
Why next: a box that only `curl` can configure is not autonomous. Done when: the box runs a month
in a real annex without intervention (the true north's test).

| # | Increment | Done when | Status |
| - | --------- | --------- | ------ |
| J1 | Add a device from the interface: pairing while serving, per integration (Administration › Discovery), the device appears live and named; it has no room of its own (the room is its equipment's, J2) | The ZBMINIR2 is added without a terminal | |
| J2 | Rooms and equipments from the interface (Sowel's screens ported under Administration): the equipment is the entry point (type, name, room, 1 or N compatible devices, bound as Sowel's interface binds); never from a device's row (D61) | The garage and its equipments made with the mouse, the light from the Equipments page with the ZBMINIR2 picked | built 2026-11-25 (spec 030): an equipment's page (its bindings added and removed one by one, the missing ones offered, its devices changed, enabled or disabled, deleted) and a device's page; an equipment can be created with no device; box check not needed (interface only); 2026-11-25 (spec 037): a device's row no longer creates an equipment, and Sowel's top bar is on every page |
| J3 | Recipes from the interface: the list, the form built from the slots as Sowel's, enable and disable, the log | motion-light set without `curl` | done 2026-11-23 (spec 024): the flow run once on the box over the API; a recipe settles when what it uses changes (D38); the browser demo is the user's |
| J4 | A password at commissioning, confirmed by the box's button (EN 18031: no common default password); factory reset | The box asks for a password on first start | built 2026-11-25 (spec 029): Sowel's `/auth` routes and bearer token on the API and `/ws`, PBKDF2 in `auth.json`, factory reset, first-start and login screens; the confirmation is the 10 minutes after a power-on until the button is wired; box check pending |
| J5 | Updates from afar: signed firmware, written to the inactive partition, automatic return to the old one if the new does not start | An update installed from the interface, and one that fails rolled back; a rollback to a release older than spec 031 cannot read a `devices.json` holding a `json` data (D51): the rollback handles it | built 2026-11-25 (spec 032): a signed update file (Ed25519 over a 128-byte header carrying the image's SHA-256, key given at build time, D52) streamed into the inactive slot through `POST /api/v1/system/update`, a probation (confirmed after 1 minute healthy, rolled back after 5, D54), `--update-keygen` and `--update-sign`, a Firmware update card; box check pending |
| J5b | The box updates itself from a GitHub release, in three steps: (1) the CI publishes a signed release; (2) the box reads `manifest.json` at `releases/latest/download/manifest.json` and downloads the update (Sowel's version check: `POST /api/v1/system/version/check`, `latest`, `updateAvailable`, `releaseUrl` filled from GitHub releases); (3) key rotation (several keys or a bridging release, a revocation list, what a box does when its key is lost, D64) | A box on release N installs release N+1 with no file from the user | step 1 built 2026-11-25 (spec 038): `scripts/release.sh`, `release.yml`, `--update-verify`, `docs/release.md`; the first real release is pending the user's key (public key committed, environment `release` and its secret set); step 2 built 2026-11-26 (spec 039, on the Mac and in QEMU; TLS and the real GitHub address not yet run on the box, see "Tests to run with the box in hand"): the check, the pull, the automatic check (off by default), the setting `update.manifestUrl`; step 3 built 2026-11-26 (spec 040, on the Mac and in QEMU; D70 to D72): the bridging release (`release/update-key.bridge`, two secrets, `docs/release.md`'s procedure and "A leaked key"), the Settings page's update card (check, notes link, Update to x.y.z, polling), and a cut download resumed with a `Range` request. Remains on the box (the list below): the resume against a LAN server that cuts once, the card from the browser, the rotation with two test-key images (a USB flash, the user's yes), and the first real release; the key rotation's logic is checked on the Mac by `crates/host/tests/rotation.rs` |
| J6 | Home settings: the location (sun times, so "inactive during day" works), the time zone; export and import of the configuration | The light no longer switches on in daylight | built 2026-11-24 (spec 026): settings, sun times on the root zone, Settings page, export and import of the settings and zones; the box run is pending (inbox) |

## Built, waiting for the real hardware

Milestones A and B are built and shown on the simulator; their demos on the real devices are E2.

**A. The radio speaks Rust**. The user can then: from the Mac, `solite-host` forms its own
Zigbee network through the Dongle Max, a real sensor (Sonoff SNZB-02) joins it, and its
temperature arrives live, still after a restart, without pairing again. The firmware boots in an
emulated ESP32 (Espressif's QEMU) with its free memory shown, and the share of Zigbee2MQTT
definitions the box can reuse is known for the leading brands.
Why first: it removes the three main risks of the framing note (EZSP in Rust, device coverage,
the ESP32's memory) before anything is built on them. Done when: the SNZB-02, paired from the Mac,
reports its temperature for a day, restart included; the coverage report and the QEMU memory
figures exist.

Hardware: the Dongle Max and a Sonoff SNZB-02, both arriving 2026-11-21. A1 and A2 need none;
A3's code is written and tested without hardware, its demo needs the Dongle Max. Nothing is needed
on the Sowel side.

| # | Increment | Done when | Status |
| - | --------- | --------- | ------ |
| A1 | Zigbee2MQTT coverage of the leading brands: a tool in `tools/` reports, per brand, the share of definitions that are declarative, Tuya datapoint tables, or code, and the share a generic reading of the standard ZCL clusters would catch (first: no hardware, measures the device-coverage risk) | The report exists, with the SNZB-02's line | done (spec 002) |
| A2 | The firmware boots in Espressif's QEMU on the Mac and shows its banner and free memory (internal RAM, PSRAM); part of `sh scripts/validate.sh` (second: no hardware, measures the memory risk from day one) | The banner and the memory figures show on the Mac | done (spec 003) |
| A3 | Talk to the radio: ASH and EZSP in Rust, in the core, tested against Silicon Labs' reference frames; the host connects to the Dongle Max over TCP and shows the EmberZNet version and the network state | The version shows, with the real Dongle Max | built (spec 004), demo pending |
| A4 | Form a network and let the SNZB-02 join: network key and channel kept in a file; the host shows the joining device's address, manufacturer and model | The SNZB-02 shows with its name after a press on its button | built (spec 005), demo pending |
| A5 | Read its temperature and survive a restart: temperature, humidity and battery shown at each report, through a generic reading of the standard clusters; after a restart of the host the network resumes and the sensor keeps reporting | Readings scroll, then come back after a restart | built (spec 006), demo pending |

**B. The engine on the Mac**. The user can then: on the Mac, with the Dongle Max, pair sensors
and a relay, put them in rooms, and see a motion sensor switch the light on (`motion-light`, as in
Sowel); everything readable through an API in Sowel's format, and live in the interface.
Why next: it proves the heart of the true north, a minimal Sowel automating on its own, before it
moves into the box (E). Done when: a real Sonoff motion sensor switches a real Sonoff ZBMINIR2
relay through `motion-light`, each room's state shows in the interface, and all of it survives a
restart.

Hardware: the Dongle Max, the SNZB-02, a Sonoff motion sensor and a Sonoff ZBMINIR2 relay (the
user has them). Approved by the user on 2026-11-21, to be built in an autonomous run as far as
possible without hardware.

| # | Increment | Done when | Status |
| - | --------- | --------- | ------ |
| B1 | Simulated radio: the host replays a scenario (a sensor joins, temperatures, motion) without the dongle (first: makes everything after it testable without hardware, in CI) | `--sim <scenario>` shows the same lines as the real radio | done (spec 007), on the branch of A3 to A5 |
| B2 | Devices in Sowel's format: a paired device becomes a Sowel `Device` with its data read generically (temperature, humidity, battery, motion, on/off, level), stored on disk, read with `GET /api/v1/devices`; memory ceilings tested from here (second: Sowel's contract, D3, is the base of the rest) | The JSON matches Sowel's, for the real SNZB-02 | built (spec 008), real SNZB-02 pending |
| B3 | Alarm sensors: IAS zone devices (the Sonoff motion sensor) are enrolled; their motion, contact, leak or smoke becomes device data | The motion sensor's `occupancy` turns true in the API | built (spec 009), real sensor pending |
| B4 | Equipments and zones: create a room, put an equipment in it (sensor, light, switch), with Sowel's zone aggregation (average temperature, motion in the room) | The API shows "Garage: 19.5 °C, motion" | built (spec 010) |
| B5 | Orders: switch a light or relay on and off through the API, on the real ZBMINIR2 | The relay obeys | built (spec 011), real ZBMINIR2 pending |
| B6 | Recipe `motion-light`, same name, parameters and behaviour as Sowel's (native Rust at first, a script recipe since spec 046) | Motion switches the relay on, then off after the delay | built (spec 012); on the Dongle Max, the garage's instance runs the script `motion-light` 1.1.2 (2026-11-27) |
| B7 | Live events: WebSocket in Sowel's format (`/ws`, topics, 200 ms batches) | A client sees the temperature, the motion and the light change as they happen | built (spec 013) |
| B8 | Interface: served by the host, shows the rooms and their equipments live | The temperature changes in the browser | built (spec 014), real Dongle Max pending |

## Later milestones

Decided with the user on 2026-11-26 after a comparison with Sowel's base features (recipes
themselves are a separate subject). Planned when their turn comes, in this order. The letter D is
skipped: decisions are D1, D2...

| Rank | Milestone | Content | Note |
| ---- | --------- | ------- | ---- |
| 1 | **F. Script recipes** | F1 (spec 041, done, its wave on the box 2026-11-27): recipes as JavaScript scripts in Sowel's script-recipe format, run by QuickJS on the host and the box, reaching only their slots, `motion-light` (the port of Sowel's recipe; the native one is gone since spec 046), measured (D73 to D77; WebAssembly measured and dropped). F2 (spec 042, done, its box check 2026-11-27: the recipe key made, `motion-light` installed from a signed file): a script downloaded from a catalogue or uploaded, signed with a key of its own, installed, updated and removed without a new firmware, admitted by the memory really free, a Resources card. F3: Sowel's recipes brought as scripts (`switch-light` built, spec 052; `motion-light-dimmable` built, spec 054; `schedule-on-off`, `state-trigger-light`, `presence-thermostat` and `state-watch` built, spec 055; the next ones...) | Raised to first place by the user. F1 built and merged (spec 041); R7's wave made on the box on 2026-11-27 |
| 2 | **I1. Targeted recognition** | Data and order categories for common devices, with their clusters: `power`, `energy`, `current` (spec 048), `action` of a button (spec 049, generic command names) and `shutter_position` with `shutter_move` and `set_shutter_position` (spec 050, Window Covering `0x0102`), `setpoint` with `set_setpoint` and a thermostat's `local_temperature` (spec 051, Thermostat `0x0201`) are done; a contact's door or window is said by its equipment, the user's pick, not by the device (user, 2026-11-28) | Solite knows 24 of Sowel's about 75 categories (specs 048 to 051 are done parts of I1; the device-specific names of `action` stay to do); a thermostat is fed its temperature and setpoint, not yet its power or mode |
| 3 | **I2. Equipment types without rules** | Binding rules and zone counts of Sowel for `water_heater`, `heater`, `water_valve`, `vmc`, then the others (23 types accepted without rules) | As a real device needs one, not before |
| 4 | **C1. Minimal modes** | Sowel's modes (day, night, away), their routes and events; no calendar and no triggers at first | Added to the roadmap; a prerequisite of proposals (the rest of C) and of G |
| 5 | **G. Link with Sowel** | The box's equipments in Sowel (depends on `sowel-plugin-solite`, which first publishes the box's devices, then its equipments and zones once Sowel lets a plugin publish them) | After the modes. The Sowel issue is opened early (draft ready) |
| 6 | **Monitoring finishes** | `GET /api/v1/health`, a supervisor card in the interface, Sowel's `system.update.available`, `.progress` and `.error` events | Lines of the inbox |
| 7 | **H. Daily use** | 24 h history (bounded by constants), MQTT export, mini PWA, languages beyond French and English, radio firmware update, recipe logs and state kept across a restart | After the rest |
| 8 | **Notifications** | Telegram, web push | Not a priority: stays in the inbox until the user says otherwise |

Rest of **C** (pair a sensor, pick the room, accept the proposal) follows C1 and the recipes;
rest of **I** (Zigbee2MQTT definitions export with the Tuya datapoint interpreter, Philips'
library, hand-written Legrand, Sonoff and Aqara definitions, alarm devices beyond motion, contact,
leak and smoke) is planned after I2.

Outside Solite's scope, reached through Sowel with G when at all: energy and the solar arbiter,
cameras, shared access, several users and MFA, the integrations store, AI recipes and MCP.
Beyond the true north, not planned: Matter bridge; product track (decision D5).

## Inbox

New needs not yet placed. Anyone adds a line, at any time; the next review places them. "Note"
is the framing note, `inputs/note-de-cadrage-mini-sowel.md`.

| Date | Need | Came from |
| ---- | ---- | --------- |
| 2026-11-28 | `sh scripts/box.sh log` restarts the Dongle Max when it opens the port on macOS, though DTR and RTS are set low before the open: it is not read-only. Make it so (pyserial's open order, or `stty`), or say so in `CLAUDE.md`'s commands. | Spec 056, box run |
| 2026-11-28 | **The recipe form as Sowel's**: slots grouped by their `group`, and an equipment slot offering the equipments of every zone (spec 024 offers the room's only), so `state-trigger-light` can watch a gate in another zone. | Spec 055 |
| 2026-11-28 | **Recipe actions, as Sowel's**: a recipe's `actions` (for `presence-thermostat`, `set_mode`, the mode switch on its card) through the script API, the recipe routes and the interface; served without them today. | Spec 055 |
| 2026-11-28 | **The worst-case memory test is noisy and close to its ceiling**: 1,045,449 bytes of 1,048,576 (3 KB), while the figure moves by about 9 KB run to run (1,036,365 to 1,055,135 on the same code). Find the noise (threads, allocator, timing) and make the figure steady, or win back margin (the streamed `GET /equipments` is 443 KB of it) before the next recipe. | Spec 055 |
| 2026-11-28 | Align Solite's API with Sowel's as close to 100 % as possible: every route, its path, verb and JSON as Sowel's (`../sowel/docs/technical/api-reference.md`, `src/shared/types.ts`), the deviations of `docs/sowel-subset.md` removed one by one. First: the recipes served under Sowel's `/plugins` routes, as Sowel does; putting them elsewhere was a mistake. | The user, 2026-11-28 |
| 2026-11-27 | **The box installs on its own the recipes it proposes**: a new box has no recipe until it reaches the catalogue or receives a file (spec 046 removed the native `motion-light`); when it proposes a recipe for a paired sensor and a room, it installs it itself from the catalogue, with the user's yes | User, 2026-11-27 (spec 046, question 4) |
| 2026-11-27 | **Bug, the box restarted once by its interrupt watchdog** (`rst:0x8 (TG1WDT_SYS_RESET)`, `PRO CPU PC=0x40080080` = `_WindowOverflow8`, `APP CPU PC=0x40081f3a` = `handle_livelock_int`): about 60 s after a boot of the B7.6 image, while the update route drained a 2.3 MB body it had refused (`Version 0.1.1 is not newer than 0.1.1`, right after four connections ended with 408). Not reproduced in two identical runs. The signature reads as a task on core 0 spilling its register windows to a stack in PSRAM while the flash cache is off (the other core held in the livelock handler): a PSRAM-stacked task (the API workers, spec 023) doing or meeting a flash operation. Read on 2026-11-27 (no fix yet): ruled out, a PSRAM-stacked task calling a flash operation (ESP-IDF asserts the caller's stack is internal, `spi_flash/cache_utils.c:127`, and would have printed it, as D59's crash did), the boot marker's removal (600 s, not 60), a thread with a default stack (ESP-IDF's default is internal RAM, `pthread.c:215`), and a worker interrupted by a flash operation on the other core (ESP-IDF parks it through its IPC task before the cache goes off). Likely: the ESP32's cache livelock under heavy PSRAM traffic from both cores (the 2.3 MB received into Wi-Fi and lwIP buffers in PSRAM), which ESP-IDF's ECO3 workaround (`highint_hdl.S`, the interrupt watchdog's `handle_livelock_int`) eases but may not remove; no panic print came before the hardware reset. Decided with the user: watch it in normal use (the supervisor counts each restart; frequent ones would end in safe mode), no stress loop on the box; spec 045 (the `script` stack in PSRAM, more PSRAM traffic) goes ahead with it watched (user, 2026-11-27): seen again during its box check, the PSRAM stack is reconsidered | Box, 2026-11-27 |
| 2026-11-27 | The interface's tests ended once with an unhandled error after all 541 passed (`ReferenceError: document is not defined` in `Modal.tsx:36`'s cleanup, from `AddBindingModal.test.tsx`) during a full validate on a loaded Mac; three runs alone passed clean: a modal's effect cleanup runs after the test environment is torn down. Make the test wait for the unmount | A failed validate, 2026-11-27 |
| 2026-11-27 | The rotation of the recipe key (spec 042): a key that leaks is replaced by a firmware that carries the new public key, then the scripts are signed again; its design, like spec 040's bridge for the firmware key, is a line of its own. The recipe key itself, the `mchacher/solite-recipes` repository, its CI and its secret are the user's to make (F2's box check R7 needs them) | Spec 042 |
| 2026-11-27 | Scripts in the backup archive (spec 042 leaves the files of `scripts/` out, D54: an older release refuses an archive with an unknown file): a restore on another box leaves the restored instances stopped (`Failed to restore: Recipe not found: <id>`) until the recipe is installed there | Spec 042 |
| 2026-11-27 | Sowel issue: the signed script file (`SOLITERS`, a header of 128 bytes) and the catalogue (`recipes.json`) as a format common to Sowel and Solite, so that the two share one distribution of script recipes (Sowel's plugin store serves TypeScript packages, not scripts) | Spec 042 |
| 2026-11-26 | The gate's refusals of the streamed routes (401, 403, 409, 411, 503, the update's 413: `close_after_answer` in `firmware/src/http_server.rs`) still close with the body unread: the client sees a reset, not the answer (an upload while another update installs shows a network error in the interface) | Review of the drain fix |
| 2026-11-26 | The HTTP server's task waits about 2 s (5 s before the fix of 2026-11-27) for each request whose body never comes, and every other request waits behind it, `GET /` included (4.8, 9.8, 14.9 and 19.8 s with 1 to 4 such requests at 5 s; 1.8, 3.8, 5.9 and 7.9 s at 2 s, measured on the box on 2026-11-27). The wait is shorter; a single-task httpd still serializes them. To check on the box: the same 1 to 4 requests should now cost about 2 s each | Box session 2026-11-26 |
| 2026-11-26 | `box-session.sh` shows each step's line as it runs (the person sees nothing for minutes today) | Box session 2026-11-26 |
| 2026-11-26 | The session's gestures left undone on 2026-11-26 (H3, H4, H5, H8, H9) | Box session 2026-11-26 |
| 2026-11-25 | `box.sh` sends one raw line to the radio image's console, so the session checks spec 023's long and unknown console lines | spec 033 |
| 2026-11-25 | The box's `storage` lost its files once (wifi.json, auth.json, the home: 28 KB used afterwards) between two flashes with `--keep-storage`, after two panics during the backup export; the mount printed no `storage: formatted` line. Not reproduced (a normal reboot keeps everything). Find out whether a reset in the middle of a FAT write can empty the partition, and whether the firmware must log or refuse a silent loss: a month alone with power cuts depends on it | Box run 2026-11-25 |
| 2026-11-25 | Discovery offers no next step after pairing now that a device's row creates no equipment (D61): a Sowel-style hint ("add it to an equipment", a link to Equipments) | Spec 037 |
| 2026-11-25 | An update takes 31 to 37 s for 2.1 MB (about 65 KB/s) and a refused image 33 s: acceptable, noted for the release download (E.g. a progress for the interface) | Box run 2026-11-25 |
| 2026-11-21 | The 3 `motion-light` instances of the realistic home queue 48 start orders against `MAX_UNATTENDED` = 32: 16 are dropped with a warning at every start; decide whether the queue bound or the start sync of recipes changes | Spec 016 review |
| 2026-11-21 | Optimise the engine back under 300 KB (stream the home's loading first) | D27 |
| 2026-11-22 | Zone reorder (Sowel's up and down arrows): needs Sowel's `PUT /api/v1/zones/reorder` in the core first | Spec 018 |
| 2026-11-22 | mDNS on the box (`solite-xxxxxx.local` without the router's help) | Spec 023 |
| 2026-11-22 | The server in QEMU through `open_eth`, so the serving path is tested without the box | Spec 023 |
| 2026-11-22 | The realistic home measured on the radio image with Wi-Fi and the server up (the radio image seeded from `firmware/fixtures/`), so the "Internal RAM with Wi-Fi up" row stops being a subtraction for it | Spec 023 |
| 2026-11-22 | The 33 requests of `measure-api.sh` on the box: eight runs, no `Busy`, totals 9.0 to 22.3 s against 10 s (rising over the runs), each creation 0.15 to 0.65 s (GETs about 0.05 s), likely the save to flash at every write; in two runs one request got no answer at all (curl status 000; 60 zone creations and deletions in a row lost none). Find where the time goes and why a connection is lost, before E3d's day-long run | Spec 023 run |
| 2026-11-22 | Investigate on the box the `devices.json` save (98 KB): it blocks the radio loop 3.8 s, 51 saves in 172 s in the realistic home. Accepted: a message may be lost while a save runs; what counts is the performance in normal running. Leads: one file per device, the radio on its own core, apart from the engine and the writer (a flash write stalls both cores for tens of ms, and the serial chip holds 128 bytes: about 11 ms at 115200 baud, so a large buffer is needed), where the time goes (erase, write, rename); UART interrupt in internal RAM (E3b) | Spec 020 |
| 2026-11-22 | `box.sh backup` prints `tr: stdout: Broken pipe` after a valid backup (cosmetic); the radio and the Wi-Fi of E3 settle the original firmware version, unread in E3a | Spec 020 |
| 2026-11-22 | The Devices page's search, sort and battery, LQI, last-seen columns | Spec 019 |
| 2026-11-22 | Sowel's `GET /api/v1/integrations`, so Discovery's tabs come from the core rather than a list in the interface | Spec 019 |
| 2026-11-22 | Remove the box's pairing windows at a power-on boot and from the serial console's `pair` (D32, D36) once the button is wired: the authenticated route exists (spec 029, D49) | Spec 021, D36, spec 029 |
| 2026-11-22 | Flash encryption (or another protection) for the keys of `/data/zigbee.json`, written in plain text on a flash that is not encrypted | Spec 021 |
| 2026-11-22 | The original firmware's network (one channel and PAN, not recorded here) was resumed by the box, so the box and the Mac host may share one radio network, with a question of who owns it: the box's `zigbee.json` only exists once the box forms or resumes, check what it saved (not checked: no serial port used) | Spec 021 |
| 2026-11-22 | A channel choice for a multi-channel device (Sowel's candidate picker in `DeviceSelector`) | Spec 019 |
| 2026-11-22 | Spec 023 review: move the /ws welcome text into the core's live::session (the host and the firmware each write it today) | Spec 023 |
| 2026-11-22 | Run the pending box checks of spec 023: the lost link (access point off), no 1970 date on the flash after a boot with the access point off, a wrong password and `wifi --forget`, the 4 requests in progress and the 5th `Busy`, the unpowered relay, the console's long and unknown lines, a client that stops reading, the power-on cases with the access point or its Internet off | Spec 023 |
| 2026-11-22 | One broadcast point in the core, `Home::publish(Fact)`, to fixed subscribers held as fields of `Home` (no dynamic allocation, synchronous under the home lock, tested on the Mac; about 250 to 350 lines, mostly moved). Today about twenty direct hub calls and three recipe-queue calls sit in `measures`, `aggregate`, `check_statuses` and `report`, and the data-to-bindings walk is written twice (`home/mod.rs:411-423`, `:437-449`). Do it with the first new consumer (history, notifications, MQTT), not before | Architecture review 2026-11-22 |
| 2026-11-22 | A plain GET can recompute zones and emit events (`api/mod.rs:278-280`): kept in J3 (spec 024, R6: it is when a silent device is seen offline); looked at again in J6 (spec 026): kept, and the aggregation, GET included, also emits `sunlight.changed` when the sun data changed | Architecture review 2026-11-22 |
| 2026-11-24 | Run spec 025's checks on the box, once, the user present (D39): the radio image flashed with `--keep-storage`; `SOLITE_PASSWORD=<password> box.sh saves --budget 5 -- sh -c 'sh scripts/measure-api.sh "$SOLITE_BOX_ADDRESS"'` (33 requests, one flush, at most 5 writes); then one `box.sh log --image radio --seconds 60 --budget 1` (the files read back, only the marker written); note the counts in spec 025 | Spec 025 |
| 2026-11-24 | Run spec 026's check on the box, once, the user present, after their yes to flash (D6, D39): radio image flashed `--keep-storage`; inside `box.sh saves --budget 3`, one `PUT /api/v1/settings` (the annex's location and time zone) and one `GET /api/v1/system/backup` and its `POST` with a wrong password, refused (spec 035; the full backup's own check is in the list of the tests to run); then `GET /api/v1/zones/aggregation` shows the local sun times; by day, motion does not switch the ZBMINIR2 on with "Inactive during day"; note the counts in spec 026 | Spec 026 |
| 2026-11-24 | Sowel alignment, what is left of D41 (D44): `GET /api/v1/settings/timezones` has no Sowel counterpart; `GET /api/v1/system/timezone` answers `source: "env"` for the setting, `fallback` with a location, and the offset now | Spec 026, spec 027 |
| 2026-11-24 | Local time in the logs, from `home.timezone` (the header's home time is spec 037's) | Spec 026, spec 037 |
| 2026-11-24 | Run spec 027's check on the box, once, the user present, after their yes to flash (D6, D39): radio image flashed `--keep-storage`; inside `box.sh saves --budget 1`, one `PUT /api/v1/settings` of the time zone, then `GET /api/v1/system/timezone`; the home lock per batch of measures timed on the box (a log line to add); note the figures in spec 027 and `docs/memory-budget.md` | Spec 027 |
| 2026-11-24 | Behaviour for more of Sowel's types (door, gate, awning, water valve, heater, meters): their binding rules, Sowel's zone counts (`doorsTotal`, `waterValvesTotal`...), and the rules of the seven equipment fields refused on `PUT` (D43) | Spec 027 |
| 2026-11-24 | Proposals exposed when milestone C starts (pair a sensor, pick the room): `equipments::propose` is internal until then, Sowel has no route for it | Spec 027 |
| 2026-11-22 | Two-core rule, kept as is (engine, radio, UART interrupt and saver on core 1; Wi-Fi, lwIP, HTTP server and workers, live thread and console on core 0): a future history or MQTT thread goes on core 0 at priority 5 at most; the engine never goes to core 0 (Wi-Fi 23, lwIP 18) and gets no higher priority without a blocking read (idle-task watchdog on core 1) | Architecture review 2026-11-22 |
| 2026-11-25 | Password change (Sowel's `PUT /api/v1/me/password`) from the interface | Spec 029 |
| 2026-11-25 | A factory reset that also erases the Wi-Fi settings (`wifi.json`) and leaves the Zigbee network (forgets `zigbee.json`'s keys), so EN 18031-2's deletion is whole | Spec 029, D48 |
| 2026-11-25 | The ESP32's SHA accelerator for the password hash, through an adapter, to raise PBKDF2's iterations | Spec 029, D47 |
| 2026-11-25 | Restore the radio's NWK frame counter and child table (and weigh writing the EUI64) on a restored network, so no device drops the new coordinator's commands or needs pairing again (D57), if the box check of the full backup shows devices lost | Spec 035, D57 |
| 2026-11-25 | A rate limit per client address on the box (one counter today: a device of the home network can keep the owner from logging in while it tries) | Spec 029, D47 |
| 2026-11-25 | Read `ColorCapabilities` (`0x400A`) at the pairing setup, so a tunable white bulb gets `color_temp` and no `color` data or order (today every `0x0300` device gets both, D51) | Spec 031 |
| 2026-11-25 | Zigbee2MQTT's other forms of a `color` order: `{"hex": "#RRGGBB"}` and `{"r","g","b"}` (converted to XY), the form a colour picker gives | Spec 031 |
| 2026-11-25 | A colour and colour-temperature control on the equipment page, once Sowel's `LightControl` has one to port (Sowel issue below) | Spec 031 |
| 2026-11-25 | Sign `SHA256SUMS` (or the manifest) with the release key, so the full-flash image's checksum is as trustworthy as the `.sup` | Spec 038, D63 |
| 2026-11-25 | Sowel's `system.update.available`, `system.update.progress` and `system.update.error` events on `/ws` (the upload's answer carries the outcome today) | Spec 032 |
| 2026-11-25 | ESP32 Secure Boot V2 and flash encryption (eFuses, irreversible, D52): signed bootloader and application checked by the ROM; `box.sh identify` stops on a box that has them | Spec 032, D52 |
| 2026-11-25 | Decide what the QEMU ceiling measures: the engine without the simulator (the box's image has none) or the bench with it; if without, take the simulator's allocations out of the phase figures (`firmware/src/simulated.rs:228-259`, `measure.rs:73-86`) and set a new ceiling from the measured figure | Fix of the 363 KB peak |
| 2026-11-25 | Stream the home's loading for real: `Storage` returns a reader and `devices.json` and `equipments.json` are parsed with `from_reader` (the texts of 94 KB and 71 KB are read whole today, `engine/storage.rs:145-195`) | Fix of the 363 KB peak |
| 2026-11-25 | An internal-RAM floor measured on the box with Wi-Fi up and the realistic home, read in the `heap:` line (the largest block is now in it), written in `docs/memory-budget.md` | Fix of the 363 KB peak |
| 2026-11-25 | A notification of a restart or of the safe mode (Telegram, web push, Sowel), once a notification channel exists | Spec 036, D58 |
| 2026-11-25 | A supervisor card or a banner in the interface that shows `GET /api/v1/system/supervisor` (mode, last incident, restarts) | Spec 036, D58 |
| 2026-11-25 | Sowel's `GET /api/v1/health` (its anonymous liveness shape, for an uptime monitor) | Spec 036, D58 |
| 2026-11-25 | Presence counted from the last power-on across restarts, so a restore or a factory reset is a way out of the safe mode (today only the update, a power-on or a USB flash is) | Spec 036, D58 |
| 2026-11-25 | Supervise the boot before the storage: a crash while the data partition mounts or a pending restore is applied (`firmware/src/main.rs`, `mount_storage`) happens before the supervisor boots, so it is neither counted nor stopped by the safe mode | Spec 036 review |
| 2026-11-26 | Pull a given version, or the intermediate release a `minVersion` asks for, from the box (today the user uploads it from its GitHub page) | spec 039 |
| 2026-11-26 | Sowel's `system.update.*` events for the progress of a check and a pull (the answer waits for the end today) | spec 039 |
| 2026-11-26 | `scripts/release.sh` refuses a manifest over 12 KB (the box reads at most 16 KB, spec 038's notes are not bounded) | spec 039 |
| 2026-11-26 | Revisit the default of `update.autoCheck` (off, D67) once the TLS session's memory is measured on the box | spec 039 |
| 2026-11-28 | **Schneider Electric devices** (Wiser, Odace: plugs, switches, dimmers, thermostats and radiator valves): measure first what the generic ZCL recognition (I1) already covers with `tools/z2m-coverage`, then add only the manufacturer-specific parts (their own clusters and quirks) | User, 2026-11-28 |
| 2026-11-28 | **A script call of 306 ms on the box** (`GET /system/resources`: `motion-light`, `maxUs` 306,251, against the 200 ms call limit), seen after spec 045's box check (two recipes, 5 install cycles of the second). Measured again on 2026-11-28 after a restart: an instance's start 32 to 41 ms, 8 instances on one recipe 53 ms at the most (average 13 to 27 ms), so neither the load nor the PSRAM stack makes it; the call's time includes its wait, so the likely cause is a call of `motion-light` waiting while the other recipe's install or uninstall held the home. To check: time a call during an install, and whether the 200 ms limit (which counts the script's own run) should count that wait | Box, 2026-11-28 |
| 2026-11-28 | **A flaky host test under load**: `wifi_ctrl_c_at_the_password_prompt_restores_the_echo_and_sends_nothing` (`crates/host/tests/box_script.rs`) failed twice in `sh scripts/validate.sh` (the terminal's echo still off after Ctrl-C), passes alone and in other validate runs | Session, 2026-11-28 |

## Sowel issues to open

Needs on the Sowel side, for the user to open as Sowel issues (never changed from here).

| Date | Issue | Came from |
| ---- | ----- | --------- |
| 2026-11-21 | motion-light enters override when a zone event arrives between its ON and the light's echo (stale state read) | Spec 012 |
| 2026-11-21 | motion-light's failsafe leaves override mode set; the button's ON arms no off timer | Spec 012 |
| 2026-11-21 | motion-light validates an order aliased `state` but sends by category first; the `disableWhenDaylight` slot speaks of offsets the code does not read; timers beyond 24.8 days fire at once | Spec 012 |
| 2026-11-21 | `POST /recipe-instances/:id/actions` on an unknown id answers 400 `Instance not running` instead of 404 | Spec 012 |
| 2026-11-24 | A `home.timezone` setting (an IANA name), read before `TZ` and `tz-lookup`, so a home without a `TZ` variable (Solite's box) shares the key with Sowel (D41) | Spec 026 |
| 2026-11-25 | Dates other than the devices' taken during the box's clock wait stay in 1970: recipe log lines and state keys (`timerExpiresAt`, `failsafeExpiresAt`) at the boot's start, equipments and zones created through the API in the first seconds, a device's `createdAt`/`updatedAt` read in 1970 from storage moved to about the boot's date (D45 moves the devices' only) | Spec 028 |
| 2026-11-25 | A colour and colour-temperature control in `LightControl` for `light_color` (today brightness only), which Solite would port | Spec 031 |
| 2026-11-25 | Check the Zigbee2MQTT plugin's colour: `flattenExposes` seems to turn the composite `color` expose into data `x` and `y` (category `generic`), so a colour bulb would not get `color` / `light_color` as `constants.ts` and `binding-candidates.test.ts` expect | Spec 031 |
| 2026-11-26 | Script recipes' draft (`roadmap/ai-script-recipes/spec.md`): a recipe `id` (Solite's scripts have one), the zone slot rule, `onMotionReport` (a safety auto-off reset by fresh motion) and `validate(params, home)` (checking bindings), which Solite's `motion-light` script needs | Spec 041 |

## Done
- 2026-11-27: spec 046 built (D81): the native `motion-light` recipe and its crate removed, no recipe is built in; `motion-light` is Sowel's 1.1.2 as a script recipe (Sowel's id, info and validation texts, at most 8 instances); the memory ceilings and the QEMU phase measure script instances; the Mac launchers pass `--recipe-script`; B11 checks the garage's own instance. Done on 2026-11-27: the Mac demo, and on the box the garage running `motion-light` 1.1.2 from a signed file, B11 green.

| Date | Increment | Spec |
| ---- | --------- | ---- |
| 2026-11-20 | Walking skeleton: the tool starts and shows its name on the Mac, the ESP32 and in the browser | 001 |
| 2026-11-20 | A1: Zigbee2MQTT coverage of the leading brands | 002 |
| 2026-11-20 | A2: the firmware boots in QEMU and shows its memory | 003 |
| 2026-11-21 | E1: the target prepared: hardware sheet, memory budget, partition table, network choice (spec 015); the engine run in the firmware under QEMU, memory measured (spec 016) | 015, 016 |

## Log

Every change to this roadmap: date, what changed, why.

- 2026-11-20: roadmap created at setup; inbox filled with the needs of the framing note and of the
  setup.
- 2026-11-20: license chosen (AGPL-3.0-only, decision D8): its inbox item removed.
- 2026-11-20: remote and CI connected (decision D9): the CI inbox item removed.
- 2026-11-20: milestone A planned; every inbox item placed (A1 to A5, B to H, or beyond the true
  north; none dropped). The coverage measure is limited to the leading Zigbee brands (the user's
  wish, widened to the leaders beyond their first list: Philips Hue, Tuya and its white labels,
  Aqara, IKEA, Innr, Sonoff, Schneider, Legrand, Ledvance, Heiman, Lexman...). QEMU added (A2)
  so the memory risk is measured before the hardware arrives. CI on the remote is not an
  increment: it comes with `/add-remote`.
- 2026-11-20: A1 done (spec 002), in an autonomous run approved by the user; three needs from its
  report added to the inbox (for milestone B).
- 2026-11-20: A2 done (spec 003), autonomous run: firmware revision 3 with PSRAM, 362 KB internal
  and 2045 KB PSRAM free at start in QEMU; one inbox line (first CI run with QEMU).
- 2026-11-21: milestone B planned with the user (six increments, B1 to B6); the ZBMINIR2 relay
  and a Sonoff motion sensor replace a bulb; modes moved to C; the definitions export and the
  hand-written definitions grouped in a new milestone I (after C), as the A1 report shows the
  generic reading covers more; two inbox lines from the A1 report placed (B2, I).
- 2026-11-21: B3 inserted (alarm sensors' enrollment, split out of B2 by the user); the next
  increments renumbered B4 to B7. A1 to A5 and B1 built in an autonomous run (A3 to A5 waiting for
  their hardware demos).
- 2026-11-21: B7 split into the live events (B7, spec 013) and the interface (B8), decided in
  the autonomous run: each is a demo of its own. B5 and B6 built (specs 011, 012). The memory
  ceiling test of the realistic home reaches 198 KB of 200 KB in the worst case with B7: the inbox item
  "memory budget of the box" is due before more features land in the engine.
- 2026-11-21: B8 built (spec 014): milestone B is built on the simulator, its demos on the real
  Dongle Max, motion sensor and ZBMINIR2 pending. A full review with `/solite-roadmap` is due
  (milestone reached, inbox of 18 items, the memory ceiling reached) before milestone C or E.
- 2026-11-21: the engine's memory ceiling raised from 200 KB to 300 KB with the user (decision D20), after
  an estimate of the whole box; its optimisations added to the inbox.
- 2026-11-21: full review with the user after milestone B: the user wants Solite on the target quickly.
  Current milestone E (Solite in the box: E1 prepare the target, E2 the real hardware from the
  Mac, E3 flashed), next J (the autonomous box: configuration screens, password, updates, home
  settings; done when a month in an annex), then C, I, H, F, G. A and B wait for their hardware
  demos (E2). Every inbox item placed; the Sowel issues moved to their own list.
- 2026-11-21: E2 demos run on the real Dongle Max (USB relayed by `socat`), a SNZB-03PR2 and a
  ZBMINIR2: radio, pairing, resume, readings, API, orders, motion-light and interface worked
  (specs 004 to 014 updated). Four defects: battery shown 0 % (not reproduced since: the sensor reports 200, 100 %,
  so the 0 was probably a first report after joining; watch), `setValue` refused by the radio
  (EmberStatus 0x70, one of the two set while forming the network; the start-up one is accepted;
  settle it at the next network forming), `scripts/demo-garage.sh` and the IPv4/IPv6 port clash
  fixed (branch fix/e2-defects). Merge pending.
- 2026-11-21: Wi-Fi decided with the user and placed (inbox line removed): one image with Wi-Fi and
  Ethernet, exclusive, chosen at boot (`auto | wifi | ethernet`), Wi-Fi first; access point
  `Solite-XXXX` to give the network without a screen. E1 and E3 changed; E2's branches merged.
- 2026-11-21: E1's first half built (spec 015). Worst case measured at 987 KB (ceiling 1 MB, D23). The
  budget shows the internal RAM as the tight resource: about 156 KB are left for the engine's small
  allocations (most of its bytes), against about 130 KB for the realistic home; spec 016 measures
  it, and `SPIRAM_MALLOC_ALWAYSINTERNAL` is the knob. The `sdkconfig` cannot carry the partition
  table (esp-idf-sys ignores it, D25): it is given at flashing.
- 2026-11-21: E1's second half built (spec 016), in an autonomous run: the engine's loop is in the
  core, the firmware mounts its FAT partition and, in the QEMU image, runs the realistic home on the
  simulated radio. Peak 312 KB (ceiling 350 KB, D27; upper bound with the simulator inside), internal RAM
  never below 107 KB free; E1 is built. One inbox line: stream the loading of the home (its files are read
  whole: the largest transient of the peak); the figure is measured again on the box in E3.
- 2026-11-21 (review of spec 016): the ceiling of the QEMU run is 350 KB (D27; the Mac test stays at
  300 KB). Inbox: the 94 KB `devices.json` save is synchronous in the radio loop, its stall is measured on
  the box (E3); the 16 dropped start orders of the motion-light instances are a line of their own. A
  save writes `<name>.tmp`, renames it to `<name>.new` once complete, and only `.new` is recovered.
- 2026-11-21: `scripts/quiet.sh` added as a strict rule for Claude and every agent (a command's
  output goes to a log, one verdict line is read), so the agents' context stays small; the same
  practice was filed as Sowel issue 1291. The commit hook checks the main folder, so one branch at
  a time is worked on while an agent builds there.
- 2026-11-22: spec 019 specified and built (UI): J1 and J2 corrected after a misreading of Sowel's model
  (specs 017 and 018 let the user put a device in a room; in Sowel a device never has a room, the
  equipment does). The interface gets Sowel's Administration section (Devices, Equipments, Topology,
  Discovery per integration), "Create equipment" on a device's row, and the equipment form binds from a
  plan computed in the browser (D30). Inbox lines added from spec 019.
- 2026-11-22: E3 cut by the user into E3a to E3d. Spec 020 (E3a) specified and built: `scripts/box.sh`,
  the measuring image (feature `flash-saves`), decision D31 (esptool replaces espflash, the simulator
  runs on the box in the measuring image only). The inbox line of the `devices.json` save stall is
  removed (measured by E3a); the line of the 33 requests goes to E3c. The run on the box (backup,
  restore, flash, figures) waits for the user's yes for each write.
- 2026-11-22: spec 021 (E3b) specified and built: the radio image (feature `radio`) drives the EFR32 over
  UART1 and runs the engine on it, with a speed probe, a restart of a wedged radio, pairing for 180 s at a
  power-on boot, the `serial:` and `heap:` lines, `box.sh --image radio`; decision D32. Inbox lines added
  (the boot window, a device removal, the keys of `zigbee.json`, `RADIO_BAUD`). The run on the box (baud
  found, the sensor paired, the 0x70 outcome, the figures) is the next entry.
- 2026-11-22: spec 021 (E3b) run on the box: the radio answered at 115200 baud, the first speed tried (EZSP 13,
  EmberZNet 7.4.5.0); the box resumed a network made by another program (one channel and PAN, not recorded here) and formed none,
  so the `EmberStatus 0x70` question of `setValue` stays open; pairing opened 180 s at boot and closed; the SNZB-03PR2
  joined 0.75 s after the opening and its battery, motion and luminosity were read; GPIO17 works as TX with the
  PSRAM; serial ring highest use 262 B, internal RAM minimum 234 KB. Not run: 10 minutes of capture (4 run), a second
  boot, the booting-marker and power-cut cases, the restart ladder, a forming, the ZBMINIR2. `RADIO_BAUD` stays
  115200 (its inbox line removed). The box stays on the radio image.
- 2026-11-22: spec 022 done (serving rules in the core, `crates/ui-embed`, scripts on an address, `measure-api.sh`); E3c continues with spec 023 (firmware). New need in the inbox: drop the mandatory backup of the original firmware. The inbox holds more than five items: a full review (`/solite-roadmap`) is due.
- 2026-11-22: spec 023 (E3c, firmware) specified: E3c reads Wi-Fi only, Ethernet moves to the new E3e after E3d; E3d's settings route is the serial console. Five inbox lines added from the spec (mDNS, the server in QEMU, the bearer token on `/ws`, the pairing window after `wifi`, the realistic home measured with Wi-Fi up).
- 2026-11-22: spec 023 done (E3c done, box run partial): the box joined Wi-Fi and served the API and the
  interface; the garage, the 33 requests, the lost link, the clock line and the error cases not run (inbox).
  Decision D35: the backup is optional for `box.sh flash` (its inbox line removed). The living spec's
  sentence saying `box.sh wifi` does not reset the box is corrected in the same change.
- 2026-11-22: architecture review (read-only, Opus): sound, nothing to fix before J3. Six lines added to the
  inbox (one broadcast point, missing events, stale recipe ids, GET side effects, the home lock, the
  two-core rule).
- 2026-11-22: spec 023 completed on the box: garage set up, motion switched the ZBMINIR2, `/ws` fixed
  (D37) and seen live in Chromium, `box.sh pair` added (D36), `motionSince` waits for the clock. The
  inbox lines on the power-on needed to pair are removed (D36 answers them), the J4 pairing line
  rewritten; the 33 requests measured over 10 s (inbox line replaces the one asking to measure); the
  pending box checks narrowed to what was not run; new line: a `lastSeen` taken before the clock stays 1970.
- 2026-11-23: spec 024 (J3) built: recipes from a room's page; a recipe settles when what it uses
  changes (D38). The stale-ids inbox line is done and removed; the missing-events line keeps its
  `set_device_zone` half; the GET line is kept for J6. New lines: a deleted motion sensor is not
  settled; the flash-write budget rule for box tests (D39, proposed).
- 2026-11-24: spec 025 built on the Mac (the user away): saves held and flushed from the serial
  console in one `box.sh saves` session, every flash write counted (`flash: <n> writes since
  boot`, `--budget`), definitions coalesced over 2 s (D40), `measure-api.sh` on the Mac; D39
  finalised. The budget-rule inbox line is done and removed; a line for its run on the box is
  added.
- 2026-11-24: spec 026 (J6) built on the Mac (the user away): the home's settings (Sowel's keys and
  `home.timezone`, D41), the sun times on the root zone so "Inactive during day" works, a Settings
  page, export and import of the settings and zones (D42). New lines: its run on the box, a full
  backup, a current-time pill; the GET-recompute line looked at again and kept.
- 2026-11-24: spec 027 (milestone I, aligned on Sowel) built on the Mac (the user away): the Sowel
  contract check (snapshot, `docs/sowel-subset.md`, D43), Sowel's 31 equipment types and its
  optional equipment fields, `light_color`'s rules, the proposal of a type and bindings in the
  core, `GET /api/v1/system/timezone` (D44); the home lock per batch measured on the Mac (70 µs).
  Lines removed: the binding plan in the core, the home lock measure; the D41 line narrowed. New
  lines: the colour cluster, the box run, more types with behaviour, proposals exposed.
- 2026-11-25: order of work while the box is away written (core fixes, J4, J2, colour cluster, J5
  core part; E3d and E3e wait for the box); the tests to run with the box in hand gathered in one
  list (specs 021, 023, 025, 026, 027, the 33 requests, E3d, E3e, J4's button).
- 2026-11-25: spec 028 (core fixes) built: device dates taken before the clock moved by its jump
  (D45); a motion-light button without `action` pruned instead of disabling the recipe, a running
  motion-light follows its zone's motion sensors (D46); device removal (done by spec 024) and a
  device's zone change (no event, as Sowel) closed. Lines removed: the clock wait's `lastSeen`,
  the device removal, `set_device_zone`, the deleted motion sensor, the button's device. New line:
  the other dates of the clock wait.
- 2026-11-25: spec 029 (J4) built on the Mac (the user away): one account set at first start,
  Sowel's `/auth` routes and bearer token on the API and `/ws`, PBKDF2 in `auth.json`, sessions in
  RAM, factory reset, first-start and login screens (D47 to D49). Lines removed: the bearer on
  `/ws`; rewritten: the box's pairing windows. New lines: password change, a whole factory reset,
  the SHA accelerator, a rate limit per address; J4's box checks added to the list.
- 2026-11-25: spec 030 (J2) built on the Mac (the user away): the equipment page (bindings one by
  one, the missing ones, change of devices, an Activation row, delete) and the device page, an
  equipment created with no device (D50). Lines removed: the equipment detail page, bindings one by
  one, enable and disable; narrowed: the device detail page to the Devices page's search, sort and
  columns.
- 2026-11-25: spec 031 built on the Mac (the user away): the colour cluster `0x0300` read and
  driven (colour temperature, colour as Zigbee2MQTT's `json` object, XY and hue/saturation
  orders), a colour bulb proposed as `light_color`, a simulated colour bulb and
  `scenarios/color-light.json` (D51). Line removed: read the colour cluster. Added: the
  capabilities, `hex` orders, a colour control (and its Sowel issue), the Zigbee2MQTT plugin's
  colour (Sowel issue), the real-bulb check in the tests to run, J5's rollback note.
- 2026-11-25: spec 032 (J5) built on the Mac (the user away; the firmware and interface parts
  built, the box not run): a signed update file (Ed25519, 128-byte header, key at build time)
  streamed into the inactive slot, a probation with rollback, `--update-keygen` and `--update-sign`,
  a Firmware update card (D52 to D54). J5's rollback note is met. Added: the box check in the tests
  to run (one USB flash, one OTA install, one image that panics), and the inbox lines for a release
  feed, Sowel's `system.update.*` events, Secure Boot V2, publishing a release, key rotation.
- 2026-11-25: the QEMU peak of 363 KB (D27's ceiling 350 KB) came from the simulator's memory file, written whole by `to_string_pretty` (its text passed 32 KB with spec 031, the buffer doubled to 64 KB). Written in a stream now: 327 KB in QEMU, ceiling and D27 unchanged. Inbox: the QEMU ceiling's scope, the home's loading in a stream, interned device texts, an internal-RAM threshold on the box.
- 2026-11-25: spec 033 built on the Mac (the user away, the box not run): `scripts/box-session.sh`
  runs the tests with the box in hand in one command, on the user's `--yes-flash` (D55); the list
  above says, item by item, the step that runs it or that it stays a person's; `validate.sh`
  prints the firmware images' share of the slot. New inbox line: a raw console line in `box.sh`.
- 2026-11-25: spec 034 built on the Mac (the user away, the box not run): the radio image's `heap:` line prints the largest free block of internal RAM and PSRAM; the devices read back share the catalog's texts and are read in one pass. The 50 devices hold 65 KB instead of 72 KB (Mac), QEMU 159 KB held and 311 KB peak instead of 176 and 327, internal RAM minimum 110 KB instead of 95; the JSON is unchanged.
- 2026-11-25: spec 035 built on the Mac (the user away, the box not run): the full backup, one password-protected file (`GET/POST /api/v1/system/backup`, D56) holding the home, the Zigbee network and, on request, the Wi-Fi settings, streamed both ways through a 4 KB buffer, validated whole before a restore replaces the home at the next start; the radio forms the restored network and leaves the one it runs (D57); a Backup card on the Settings page. D42's document and `/api/v1/backup` are gone. Inbox lines of the full backup and of D42's alignment removed; new line: the radio's frame counter and child table on a restored network. The box check is in the list above.
- 2026-11-25: spec 036 built on the Mac (the user away, the box not run): the radio image restarts the box when its engine ends on an error, with the reason kept in RTC memory (no flash write); a boot-loop guard (3 counted restarts in an hour or 6 in a day) and a safe mode that keeps the API and the update up, retried after 1 h, 4 h, then 24 h; the engine thread on the task watchdog (20 s, restarts the chip); `GET /api/v1/system/supervisor` (D58). The box check is in "Tests to run with the box in hand"; four inbox lines added (a notification, an interface card, Sowel's `/health`, presence across restarts).
- 2026-11-25: fix of spec 035 on the Mac (seen on the box, not rerun there): the box's export stopped the chip (`assert failed: ...cache_utils.c:127`) at its first file read, 70 to 88 s after the request (the derivation's share unknown), because it ran on an API worker whose stack is in PSRAM; the export and the restore run on a `backup` thread with a 10 KB internal-RAM stack, the box's files behind the core's `Guarded`, and each derivation logs its time (D59).
- 2026-11-25: fix of spec 032 on the Mac (seen on the box, not rerun there): an update upload (a signed file whose image had one byte changed) restarted the box on `A stack overflow in task http-worker`: the installation ran on an API worker (10 KB of stack), and the header's Ed25519 check alone uses 9.2 KB of an Xtensa stack (measured in QEMU); every upload overflowed, valid or not. The installation runs on an `update` thread with a 16 KB internal-RAM stack, which logs its unused stack, and the QEMU run installs three files on that thread and fails below 2 KB unused (D60).

- 2026-11-25: first run of the night's work on the box (radio image flashed with `--keep-storage`, the Wi-Fi put back with `box.sh wifi --from-file`): the boot, the supervisor, J4's account (hash 1.1 to 1.4 s), the 33 requests (1.85 s, 0 Busy, 4 writes), settings and time zone, the pairing behind the token, the full backup (export 15 s, restore: 415, 400 wrong password, 400 tampered file, 200, 409, a clean restart), J5 (a valid 0.1.1 installed in 36.6 s and confirmed after 60 s; a 0.1.2 that panics rolled back to 0.1.1; a changed image refused in 33 s). Found and fixed on the box: the backup export panicked (a flash read from a PSRAM stack, PR 34, D59) and every update upload overflowed the 10 KB API worker's stack (Ed25519, PR 36, D60). Not run: the factory reset, the watchdog and the reboot loop, the cases of specs 021 and 023, E3d, E3e. Four lines added to the inbox.
- 2026-11-25: spec 037 built on the Mac (the user away), from the author's notes on the box: no
  "Create equipment" on a device's row (D61), Sowel's top bar on every page (home name and page,
  sun times or a "Set the location" link, home time when the browser is elsewhere, the link
  indicator in place of the Banner, D62). The sunrise of 08:14 for Paris on 2026-11-25 is right:
  Sowel's suncalc and an independent calculation agree (07:58 is Paris's sunrise around
  14 November); a test pins it. Lines removed: the sun times check; narrowed: the current-time
  pill line to the logs. New line: a hint after pairing.
- 2026-11-25: spec 038 built on the Mac (the user away; nothing tagged, no secret created): the CI publishes a signed release (`scripts/release.sh`, `.github/workflows/release.yml`, `solite-host --update-verify`, `docs/release.md`, D63 and D64). J5b added (the box updates itself from a release, three steps; step 1 built, the first real release pending the user's key); the inbox lines "Publishing a release", "Fetch updates from a release feed" and "Key rotation" folded into J5b's steps 1 to 3; added: sign `SHA256SUMS`.
- 2026-11-26: spec 039 built on the Mac and in QEMU (the user away; nothing tagged, no secret created, the box not touched): the box and the host check a release's `manifest.json` and pull the `.sup` it names (`POST /system/version/check`, an empty `POST /system/update`, `GET /system/version` from the cache), through spec 032's installation; the core follows the redirects and bounds every size and time, the box fetches over TLS with ESP-IDF's common certificate bundle on a thread of 20 KB (D65 to D69); an automatic check, off by default, with the settings `update.manifestUrl` and `update.autoCheck`. J5b's step 2 built; TLS and the real GitHub address are in "Tests to run with the box in hand". Six inbox lines added (the interface's button and notes, resuming a download, a given version, Sowel's update events, `release.sh` and the manifest's size, the default of `update.autoCheck`).
- 2026-11-26: spec 040 built on the Mac and in QEMU (the user away; nothing tagged, no secret created, the box not touched): J5b's step 3. A key rotation is a bridging release declared by `release/update-key.bridge`, with the next key checked, two secrets in the CI and a procedure and a leaked-key section in `docs/release.md`, its logic proved by a Mac chain test with two host builds (D70); the Settings page's update card checks, links the release notes, updates after a confirmation and polls through the restart, and every save of the page sends the whole settings map (D71); a cut download resumes with a `Range` request, three tries at most (D72), and QEMU's pull check includes one cut and its resume. Two observations of spec 039's box run settled: (a) pinned by a test and logged with the address, (b) not reproduced. Inbox lines resolved and removed: the interface's button and notes, and resuming a cut download. The box checks are in "Tests to run with the box in hand".
- 2026-11-26: second run on the box (radio image of main with a test update key): the release download of spec 039 works (a version check against a Mac server: 0.1 s, `latest 0.1.1`; a pull with an empty body installed 0.1.1 in 36 s and the box came back on it) and TLS reaches GitHub (certificate validated, redirect followed, 404 as no release exists yet, update-check thread stack 14.7 KB unused of 20 KB, update-pull 9.9 KB unused); the resume of spec 040 works (the server cut the image after 900,000 bytes, the box asked again with a Range request, got 206 and finished: 39 s). Not run on the box: the Settings update card in a browser, the key rotation (needs two test-key images and a USB flash), the first check right after a restart (observation a of spec 040), the internal RAM minimum during a TLS session. The real release feed (tag, secret, key) waits for the author.
- 2026-11-26: priorities after milestone J set with the user, from a comparison of Sowel's base features with Solite's subset (recipes apart): WebAssembly recipes first (F), then targeted recognition (I1), equipment types without rules as devices need them (I2), minimal modes added (C1), the link with Sowel after the modes (G), the monitoring finishes, history and MQTT (H), notifications not a priority. The later milestones are now a ranked table. Inbox lines kept: they are the sources of I2, C1 and the monitoring finishes.
- 2026-11-26: spec 041 (F1) checked on the box in two passes. The first, the user's flash, ran the
  script on the garage (the light on and off) but read 47 KB of internal RAM at the lowest and calls
  of 83 ms at the most. The fixes (the recipe's info as one text, no listen-set copy per instance,
  the allocator's own count, 200 ms per call, the CPU at 240 MHz and the flash and PSRAM at 80 MHz,
  D77) gave 74 KB at the lowest and 36 ms at the most in the second pass, flashed by Claude with the
  user's leave. R7 is met in two halves (the wave in the first pass, the RAM in the second): F1
  merged, its row "done (box check pending)" until a wave on the final image; the box-test item of
  spec 041 replaced by that inbox line. New lines: the PSRAM in the
  probation, the restore's and the server's defects of the session (B7.1, B7.6), a Restart button,
  the session's live lines, the gestures left undone.
- 2026-11-26: fix `fix/refused-upload-drain`: a restore or update refused while its body is being read drops the rest of the body (about 10 s at most) before the connection closes, so the client gets the 400 and not a reset (box session B7.1); the two inbox lines (2026-11-25 update, 2026-11-26 restore) replaced by one for the gate's refusals. To check on the box: a full-size image with a bad header uploaded from the interface (curl stops sending on an early answer and hides a slow drain), and the `http: refused body drained` line.
- 2026-11-26: spec 043 built (the user away): a Restart button (`POST /api/v1/system/restart`, refused during a probation, allowed in safe mode, 501 on the host) and the update's probation checks the PSRAM (the OTA thread starts without it so the probation ticks). The inbox's two lines for them are done. Box check pending: the button, the boot cause `asked`.
- 2026-11-27: spec 041's wave made by the user on the box (the light on and off): F1 done. Fix `fix/mains-device-ping`: the garage's relay showed `offline` while it worked; a quiet mains device is now pinged before it is called offline, as Zigbee2MQTT does; on the box (2026-11-27, the image flashed with `--keep-storage`) the relay stayed online for 20 minutes without a report, its `lastSeen` moving every 9 minutes.
- 2026-11-27: inbox lines added: the script recipes' internal RAM moved to PSRAM before F2, the `script` thread's 32 KB first (the user's decision); the worst-case memory test's thin margin.
- 2026-11-27: spec 044 built (D80): 8 data a device, 8 + 8 bindings an equipment, 20 log lines a recipe instance, params kept at 2 KB (the user's choice); the engine's worst case falls from 1018 KB to 676 KB. Checked on the box the same day: the garage loaded unchanged after a flash with `--keep-storage`, and the user's wave switched the spots on. The inbox line on the worst-case test's margin keeps only the question of what its peak counts.
- 2026-11-27: the inbox line on the HTTP server's wait for a body that never comes reworded: the
  wait is 2 s instead of 5 s (`HTTPD_RECV_WAIT_SECS`, server started with a raw `httpd_config_t`);
  the structural limit of a single-task httpd remains.
- 2026-11-27: B7.6 checked on the box (1.8 to 7.9 s with 1 to 4 bodies that never come, against 4.8 to 19.8 s) and merged; a 2.3 MB refused upload drained in 5.4 to 6.3 s. One interrupt-watchdog restart seen during that test, not reproduced: an inbox line (bug); spec 045 (the `script` thread's stack in PSRAM) waits for it.
- 2026-11-27: spec 042 built (F2, D78): script recipes installed from a catalogue or a signed file (`SOLITERS`, the recipe key), updated and removed on a `recipe` thread in internal RAM, admitted by the PSRAM free (512 KB reserve) and the internal RAM's largest block, a Resources card and a Plugins page; `motion-light-script` leaves the radio image; `release.sh` refuses to build without `release/recipe-key.pub`. The inbox lines on admission and the Resources card are done; "Before F2" is spec 045's line. Box check R7 pending: the user's recipe key, the signed test files and a flash with `--yes-flash`; the garage's `motion-light-script` instance stays stopped from the F2 flash until the recipe is installed.
- 2026-11-27: the interrupt-watchdog restart read without the box; kept as an inbox line to watch in normal use (the user's choice), spec 045 waiting for it.
- 2026-11-27: spec 047 done: the sidebar's Administration block and Log out pinned at the bottom of the column, as in Sowel (the inbox line placed and done).
- 2026-11-27: spec 045 built (D83): the `script` thread's 32 KB stack in PSRAM, the `recipe` thread made once at start (an install needs a 16 KB internal block, not 36 KB), a flash call from a PSRAM stack refused in `storage.rs`; the box check (2 recipes, 8 instances of each, 5 uninstall-install cycles) pending.
- 2026-11-27: spec 045 done (D83): the box's `script` thread's stack in PSRAM, the `recipe` thread made once; on the box, two recipes of 8 instances and 5 install cycles, internal RAM at least 64.6 KB, every install accepted.
- 2026-11-27: spec 048 done (I1, first part): `power`, `energy`, `current` of a metering plug, their scale read once; buttons, shutters, thermostats and `contact_window` remain in I1.
- 2026-11-28: spec 049 done (I1, second part): a button's `action` from its On/Off, Level and Scenes commands (generic names); shutters, thermostats and `contact_window` remain in I1.
- 2026-11-28: spec 050 done (I1, third part): a shutter's `position` and its `state` and `position` orders (Window Covering); Solite has no zone orders (`allShuttersClose`), so none reaches the shutter; thermostats and `contact_window` remain in I1.
- 2026-11-28: spec 051 done (I1, fourth part): a thermostat's `local_temperature` and `setpoint`, and the `setpoint` order as a Write Attributes (Thermostat); `contact_window` remains in I1.
- 2026-11-28: the host's radio test starts the program once untimed before its timed runs (a freshly linked binary waited for macOS's check, 5 s on a new Mac); the inbox line placed and done.
- 2026-11-28: the recipe catalogue `mchacher/solite-recipes` made (its first release holds `recipes.json` and `motion-light` 1.1.2 signed; its CI secret is the recipe key); release 0.2.0 (after the box's test build 0.1.2, whose inbox line goes); I1's `contact_window` dropped: the equipment says door or window (user).
- 2026-11-28: 0.2.0 tagged, its build failed in the release workflow (QuickJS's bindings before ESP-IDF's download), never published; fixed (#71), released as 0.2.1.
