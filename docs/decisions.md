# Decisions

Every choice that shapes the project, with its reasons, so whoever comes next understands it.
Newest last. A decision is never edited: a new one replaces it and says so.

## D1: Stack

- **Date**: 2026-11-20
- **Choice**:
  - **Core and host program**: Rust stable (pinned in `rust-toolchain.toml`), a Cargo workspace
    (`crates/core`, `crates/host`). Checks: `cargo fmt`, `cargo clippy -D warnings`, `cargo test`;
    the compiler is the type check.
  - **ESP32 firmware**: Rust with its standard library (`std`) on ESP-IDF, through `esp-idf-svc`;
    the Xtensa toolchain installed by `espup` (`firmware/rust-toolchain.toml`, channel `esp`);
    ESP-IDF pinned in `firmware/.cargo/config.toml` (v5.5.5 at setup). A Cargo project of its own
    in `firmware/`, outside the Mac workspace, using the same `crates/core`.
  - **Interface**: Preact (with `preact/compat`, so React components of Sowel work unchanged),
    TypeScript strict, Vite, Tailwind 4. Checks: Prettier, ESLint (typescript-eslint and Sowel's
    design rules), `tsc`, Vitest, a first-load size budget.
  - **One command**: `sh scripts/validate.sh` (all), `--fast` for the pre-commit hook.
  - **Tests**: Rust integration tests in `crates/<crate>/tests/`, unit tests in-module; UI tests
    beside the components.
- **Target architecture** (from the framing note, `inputs/note-de-cadrage-mini-sowel.md`):

  ```
  ┌──────────────────────────── ESP32 (Rust) ────────────────────────────┐
  │  Sowel core (no hardware dependency): devices, equipments, zones,    │
  │  modes, WebAssembly recipes, recipe proposals                        │
  │      │              │               │                │               │
  │  Radio adapter  Storage adapter  HTTP / WS adapter  OTA adapter      │
  └──────┼──────────────────────────────┼────────────────────────────────┘
         │ UART (EZSP)                  │ Ethernet PoE / Wi-Fi
  ┌──────┴───────────┐          ┌───────┴────────────────────────────┐
  │ EFR32MG24        │          │ Mini PWA, sowel-plugin-mini,       │
  │ EmberZNet (NCP)  │          │ signed updates                     │
  └──────┬───────────┘          └────────────────────────────────────┘
         │ Zigbee: sensors, lights, plugs, shutters
  ```

  The core reaches the hardware only through the four adapters. On the Mac, `crates/host` gives
  it a simulated radio (replayed scenarios) or the real radio over TCP (the Dongle Max's original
  firmware) or USB (an EFR32 dongle); on the box, `firmware/` gives it the real ones. The
  interface is built into the firmware and served by the box. Recipes run in `wasmi`; device
  definitions come from an export of Zigbee2MQTT's converters (a Node tool in `tools/`, run in CI).
- **Why**: ESP-IDF already provides what the true north needs to run a month without
  intervention: OTA on two partitions with automatic rollback, the Dongle Max's Ethernet, Wi-Fi,
  an HTTP and WebSocket server, flash storage. With `std`, Rust is written the same way on the Mac
  and on the ESP32, which matters for a user new to Rust, and keeps about 90 % of the work on the
  Mac. The interface stays in TypeScript, which the user knows, and can reuse Sowel's components.
- **Alternative considered**: Rust `no_std` with `esp-hal` and Embassy, the path Espressif now
  backs: pure Rust, smaller binaries, finer memory control; but OTA with rollback and the HTTP
  server must be assembled, Ethernet on the classic ESP32 is marked "unstable" in `esp-hal` 1.2,
  and `no_std` is harder for a Rust beginner. The cost of the choice: `esp-idf-svc` is maintained
  by the esp-rs community with little paid time from Espressif, and the first build downloads
  ESP-IDF (a few GB). Because the core has no ESP-IDF dependency, moving to `esp-hal` would change
  `firmware/` only.

## D2: Git and hosting

- **Date**: 2026-11-20
- **Choice**: local git, branch `main`; remote: none yet, `/add-remote` connects one later (GitHub,
  beside `mchacher/sowel`, with GitHub Actions running `sh scripts/validate.sh`).
- **Why**: the project starts on one computer; the remote and its CI come when the user wants to
  publish. Commits use the same identity as Sowel, set for this repository only.

## D3: Sowel contract (equipment types: replaced by D43)

- **Date**: 2026-11-20
- **Choice**: everything mini-Sowel exposes takes the names, types and JSON of Sowel's API; the
  reference is Sowel's `src/shared/types.ts`, which wins over Sowel's docs where they disagree
  (for example `zone.data.changed` carries `{zoneId, aggregatedData}`, as in `types.ts`, not
  `{zoneId, key, value}` as in `api-reference.md`). Equipment types are limited to light, shutter,
  sensor, switch (a plug is a `switch`), thermostat and button. mini-Sowel is a repository separate
  from Sowel; what it needs on the Sowel side is an ordinary Sowel issue.
- **Why**: the same contract is what lets `sowel-plugin-mini` show the box's equipments in Sowel,
  and lets Sowel's recipes and components be ported without translation.

## D4: Design system

- **Date**: 2026-11-20
- **Choice**: Sowel's design system (tokens, Tailwind theme, design rules, components), copied
  from the Sowel repository and adapted to the box: system fonts instead of Inter and JetBrains
  Mono, Preact instead of React, Lucide icons imported by name only, a first-load budget of
  170 KB gzip. The starter kit's design system is not used. Details: `docs/design-system.md`.
- **Why**: mini-Sowel is part of the Sowel family and becomes part of Sowel's interface when
  linked; the kit's look is unrelated. The box serves its pages from flash: the budget and the
  fonts follow from the framing note (100 to 170 KB gzip, system fonts).

## D5: Trajectory and hardware

- **Date**: 2026-11-20
- **Choice**: a personal open-source project with the door to a product left open. The hardware
  is the Sonoff Dongle Max (ESP32-D0WDR2-V3, EFR32MG24, 16 MB flash, 2 MB PSRAM, Ethernet PoE,
  Wi-Fi, USB); only its firmware is replaced. The product track (ESP32-S3 board, secure boot,
  backup keys, OEM, CRA) is out of the current trajectory.
- **Why**: the framing note left "personal or product from the start" open; the user chose
  personal first. The classic ESP32 holds a single key digest (no backup keys): accepted for a
  personal project, a known limit for a product.

## D6: Development hardware stays on its original firmware

- **Date**: 2026-11-20
- **Choice**: the firmware is built for the ESP32 from the start, but the Dongle Max is not
  flashed until the user decides it; its original firmware exposes the radio over TCP, which the
  host program uses for development.
- **Why**: flashing the dongle removes the TCP radio the first development steps rely on.

## D7: License

- **Date**: 2026-11-20
- **Choice**: none chosen at setup; the Cargo manifests carry no `license` field. The choice is in
  the roadmap inbox (Sowel is AGPL-3.0).
- **Why**: the license is the author's decision, and it weighs on the product door left open in
  D5.

## D8: License (replaces D7)

- **Date**: 2026-11-20
- **Choice**: GNU Affero General Public License v3.0 only (`AGPL-3.0-only`), the same as Sowel:
  `LICENSE` at the root, `license` in every Cargo manifest and in `ui/package.json`.
- **Why**: mini-Sowel is part of the Sowel family and reuses Sowel's code (design system,
  components, recipes); one license keeps that reuse simple. As the author holds the copyright,
  the product door of D5 stays open (a separate license can be granted for a product).

## D9: Remote and CI (replaces the remote part of D2)

- **Date**: 2026-11-20
- **Choice**: remote `origin` on GitHub, `https://github.com/mchacher/solite.git` (renamed from mini-sowel on 2026-11-21), beside
  Sowel. CI: GitHub Actions, `.github/workflows/ci.yml`, runs `sh scripts/validate.sh` (cut on 2026-11-21 into three parallel jobs, `--only rust`,
  `--only ui`, `--only firmware`, so the slowest sets the time) on every
  pull request and on every push to `main` (without the slow checks, which run every night: D34), with the Rust version of `rust-toolchain.toml`, Node
  24, and the ESP32 toolchain (espup v0.19.0, ldproxy); ESP-IDF's download is cached. Pull
  requests are made with `gh pr create`; merges stay squash merges after the user's yes.
- **Why**: a backup and a public home for the open-source project (D5), and a CI that checks
  every change, the firmware build included, on a machine other than the author's.

## D10: Node tools in `tools/`

- **Date**: 2026-11-20
- **Choice**: the Node tools of D1 live in `tools/<name>/`, each with its own `package.json` and
  lock file, outside the interface; TypeScript run directly by Node 24 (type stripping, no build
  step), checked by `tsc` and `node:test`, formatted by Prettier with the interface's settings,
  and part of `sh scripts/validate.sh` and of the CI. A library a tool measures or exports is
  pinned to an exact version. The first one is `tools/z2m-coverage` (spec 002).
- **Why**: a tool's dependencies (Zigbee2MQTT's library, about 5 000 definitions) must not weigh
  on the interface's packages or budget; Node runs TypeScript the user already knows without a
  build step; pinning makes a measure repeatable.

## D11: Unsafe code in the firmware's adapters only

- **Date**: 2026-11-20
- **Choice**: the core and the host forbid `unsafe` code. The firmware denies it
  (`unsafe_code = "deny"` in `firmware/Cargo.toml`, instead of `forbid`), and allows it only on
  the small functions that call ESP-IDF's C functions with no safe wrapper in `esp-idf-svc` (the
  first: reading the heap sizes, spec 003), each with a `SAFETY` comment saying why the call is
  sound. The firmware targets ESP32 chips of revision 3 and later (`CONFIG_ESP32_REV_MIN_3`).
- **Why**: Rust cannot check C code, so every call into ESP-IDF's C API is `unsafe`; `forbid`
  cannot be lifted even on one function. Keeping such calls few, small and commented keeps the
  rest of the firmware checked by the compiler. Below revision 3 the PSRAM needs a workaround the
  C compiler applies and Rust code would not get; the Dongle Max's ESP32-D0WDR2-V3 is revision 3.

## D12: Small crates in the host when std cannot do it

- **Date**: 2026-11-21
- **Choice**: the host may use a small, widely used crate when the standard library cannot do
  the job portably; the first is `ctrlc` (spec 005), so that Ctrl-C during pairing closes the
  radio's joining. Randomness is read from the system (`/dev/urandom`) without a crate. The core
  stays free of operating-system crates; it uses `log` only.
- **Why**: leaving joining open on the radio after the user stopped the host would let a device
  join a network nobody watches; std has no signal handling. Each crate added is named in a
  spec and its review.

## D13: The device API on the Mac, and its divergences from Sowel

- **Date**: 2026-11-21
- **Choice**: the host serves Sowel's device routes (spec 008) on 127.0.0.1 only, without
  authentication, with a `Host` header check; Sowel's bearer token comes with the box's
  commissioning password (milestone E). The routing and the JSON live in the core, so the box
  reuses them; the host uses the `tiny_http` crate (the user's choice; it widens D12, which
  allowed a crate only where std cannot do the job). The core gains `serde` and `serde_json`
  (CLAUDE.md names them for JSON). Divergences from Sowel, kept on purpose: devices in pairing
  order (Sowel: by name), data in a fixed order (Sowel: by key), dates with `T` and milliseconds
  (Sowel's SQLite: a space, no milliseconds), data ids random (Sowel: derived), a stricter `PUT`,
  405 and 413 with `{"error"}` bodies, `zoneId` `null` until zones exist, no `rawExpose`. Values
  are written to disk every 10 minutes and on stop, definitions at once (the user's choice).
- **Why**: the Mac is a development host, reached from the same machine; a token without a
  commissioning flow would be a fake guard. The box's flash must last: writing every measure
  would wear it out within months. The divergences do not change what a Sowel client reads (the
  same fields, types and values).

## D14: Zones and equipments, and their divergences from Sowel (replaces D13's "zoneId null"; types and optional fields: replaced by D43)

- **Date**: 2026-11-21
- **Choice**: zones, equipments, bindings and the zone aggregation follow Sowel's types, routes,
  messages and rules, read from Sowel's code (spec 010). Devices now take a zone, so D13's "`zoneId`
  `null` until zones exist" no longer holds. Divergences kept on purpose: only the six equipment
  kinds of D3; no computed data, timed actions, energy flags, zone orders or reorder; bounds (32
  zones, 64 equipments, 16 bindings) and lengths (names 64 characters, Sowel 100); `PUT /devices`
  checks the zone (404) and deleting a zone returns its devices to no zone (Sowel does neither);
  `POST /equipments` always returns the details; validation errors as `{"error"}` texts; unknown
  fields refused on zone and binding routes, ignored on equipment routes (Sowel's clients send
  extra flags).
- **Why**: the box has bounded memory, and a Sowel client must read the same fields, values and
  rules; the stricter checks only refuse what would leave dangling references.

## D15: Long lists and files streamed in chunks

- **Date**: 2026-11-21
- **Choice**: the long API lists (`GET /devices`, `GET /equipments`) and the store's files are
  produced by a cursor in the core, a few items at a time (at most 4), each chunk under the lock
  and written out with the lock released (HTTP chunked transfer on the host; the box's HTTP server
  sends chunks the same way). The memory ceiling stays 200 KB for a realistic home (50 devices,
  10 zones, 50 equipments), model and largest response in flight included; measured 269 KB with
  whole responses, 150 KB with chunks, 124 KB once the model stopped storing what it can derive
  (parent ids, constant fields, shared aliases): the model itself went from 132 KB to 103 KB;
  128 KB with chunks of 4 items and the aggregation's index, so four slow clients holding a chunk
  each add about 48 KB.
- **Why**: building a whole response held the model plus a 143 KB string, and up to four requests
  run at once, so the peak grew with the home and with the clients; on the box's 2 MB of PSRAM,
  shared with WebSocket clients, recipes, TLS and OTA, that growth would end in a restart, against
  the true north (a month without intervention). Discussed with the user, who chose it.

## D16: Orders, and their divergences from Sowel

- **Date**: 2026-11-21
- **Choice**: equipment orders follow Sowel's route, messages and value rules (spec 011), and go
  through a queue the radio loop serves, so HTTP never touches the radio. Divergences kept on
  purpose: no order outcome events yet; a value Sowel would pass to its integration unchecked is
  refused here (400); every order is answered within 12 s and dropped, never sent late, once its
  deadline passed; a retry only for a missing answer or a busy radio; brightness 255 counts as
  254; after an order the state is read back from the device instead of being set from the order
  (Zigbee2MQTT sets it optimistically). At most 2 orders at once (one being sent, one waiting), so reads keep being served.
- **Why**: a light that switches after the client was told it failed is worse than no answer; the
  ZBMINIR2 is not set up for reporting by Zigbee2MQTT, so its state must be read; the box serves
  few HTTP clients at once and must stay readable while orders wait.

## D17: Recipes in native Rust first, and motion-light's divergences from Sowel

- **Date**: 2026-11-21
- **Choice**: until milestone F (recipes loaded at run time, `wasmi`, D1), recipes are written in
  Rust in the core, with Sowel's id, slots, messages, behaviour and routes; `motion-light` 1.1.2
  first, its quirks kept as Sowel has them and filed as Sowel issues. Divergences: recipes read
  compact, filtered engine events (not Sowel's full events); the zone aggregate is recomputed once
  per batch of measures, not per measure; orders from the API go before recipe orders, and the
  limit of 2 orders at once (D16) counts orders from the API only, recipe orders having their own
  limit (32); at most 16 instances, 16 lights and 16 buttons each, 50 log lines of at most 200
  bytes; logs and state are not kept across a restart; no 24.8-day timer limit; numbers Sowel
  would pass through (negative or fractional durations) are clamped; the body error text is
  `Invalid body`; `isDaylight` is always `null`; no `recipe.instance.*` events yet.
- **Why**: the milestone's demo needs one recipe now, and WebAssembly is a milestone of its own;
  the user is Sowel's author, so a quirk is fixed in Sowel first; the bounds keep the box's memory
  known; a person waiting at the API goes before an automation.

## D18: Live events, and their divergences from Sowel

- **Date**: 2026-11-21
- **Choice**: `GET /ws` follows Sowel's WebSocket (spec 013): the `bearer.` subprotocol echoed,
  subscribe by topics, arrays every 200 ms, Sowel's dedup and event fields. It replaces two
  points of D17: `recipe.instance.*` events are sent, and an order carries its source (the
  recipe) to its `equipment.order.executed` or `equipment.order.failed` event (D16's outcome
  events beyond these two stay unsent). Divergences: at most 4 clients (503); incoming messages
  of at most 1 KB (1009); a client's queue bounded (256 events, 8 KB), closed 1013 past it;
  pings every 30 s, a client silent for 60 s or unwritable for 5 s dropped; the token not
  checked (D13); the interface's development server accepted as an origin; not sent:
  `device.heartbeat`,
  `equipment.order.outcome|unconfirmed`, `equipment.timed_action.*`, `system.*`, `mode.*`,
  `activity.*`, and the optional `sourceTimestamp` and `startedAt`; deleting a device sends no
  `equipment.updated` for the bindings it drops; the aggregate is also recomputed every 60 s
  without a change (staleness), which can give the recipes a zone event too. Three small crates join the
  host (D12): `tungstenite` for the frames, `socket2` for the socket's buffer, `httparse` for a
  front door that keeps `/ws` on sockets the host owns (with their timeouts) and passes every
  other request to `tiny_http` on a loopback port: `tiny_http` cannot give an upgraded socket a
  timeout, and a listener timeout stops its accept loop on Linux. The box will use its own server
  (E), so this stays on the Mac.
- **Why**: Sowel's interface works unchanged against it; the bounds keep the box's memory known
  and a sleeping laptop from holding a slot for good; the host has no async runtime.

## D19: The interface ports Sowel's leaf components, not its pages

- **Date**: 2026-11-21
- **Choice**: the interface (spec 014) ports Sowel's small components and stores unchanged where
  possible (light control, sensor values, zone pills and lights line, status badge, the zone,
  equipment and aggregation stores, the API client) and writes its own page, WebSocket store and
  `t()` (French and English, Sowel's keys, `react-i18next` aliased to it); no router (the room in
  the address's hash); the host serves `ui/dist` from disk on the Mac. Target: first load at most
  80 KB gzip.
- **Why**: Sowel's home page pulls about 35 modules, an 11-store WebSocket, i18next with 67 KB of
  locales and a router, over the 170 KB budget; redrawing would break "ported, not redrawn" (D4);
  the box will embed the files (E).

## D20: The engine's memory ceiling is 300 KB for a realistic home

- **Date**: 2026-11-21
- **Choice**: the ceiling test of a realistic home (50 devices, 10 zones, 50 equipments, 3 recipe
  instances with full logs, a live client with a full queue, the largest answer in flight) allows
  300 KB, measured on the Mac; it replaces D15's 200 KB. Every optimisation found stays in the
  roadmap's inbox, and the figure is read again on the box (QEMU, then the Dongle Max).
- **Why**: milestone B brought the engine to 187 KB (198 KB in the worst case), with no room for
  the next feature. An estimate of the whole box (2026-11-21) leaves about 1.4 MB of PSRAM free in
  the likely case: the engine can take 300 KB, while the risks lie in the internal SRAM, the
  WebAssembly recipes and bounds no test checks together. Approved by the user.


## D21: The tool is named Solite

- **Date**: 2026-11-21
- **Choice**: the tool, until then mini-Sowel, is named **Solite**, pronounced "so-light": Sowel
  is "so well", Solite is "so light", the light version of Sowel that runs on its own (and its
  first recipe switches a light). Crates `solite-core`, `solite-host`, `solite-sim`,
  `solite-firmware`; skills `/solite-*`; the logo is Sowel's smiling house with three short rays
  above the roof, in the logo's own blues. A radio variant, if one comes, is "Solite Thread", not
  a new name. Earlier specs and decisions keep the old name: they are history.
- **Why**: "mini-Sowel" only said "a smaller Sowel"; Solite keeps Sowel's play on words and
  says what the box is. Checked on 2026-11-21: the crates are free; the common domains are taken
  (the site goes under Sowel's); trademarks named SOLITE exist in the United States in other
  classes (luggage, building materials; lighting abandoned or cancelled): a search in TMview and
  WIPO, classes 9 and 42, comes before a public release. Chosen by the user.

## D22: Solite's logo is Sowel's revised house with an antenna

- **Date**: 2026-11-21
- **Choice**: the logo is Sowel's revised mark (Sowel spec 210, commit `5da84dc1`: the house, the
  face and the wink, in its two weights) with an antenna on the roof, a ball and one radio wave on
  each side, and the lowercase wordmark "solite" in Nunito 700, outlined as paths. It replaces
  D21's "three short rays above the roof".
- **Why**: chosen by the user among five concepts: the antenna says what the box is (a small
  radio box) while the house and its smile stay Sowel's; outlined glyphs keep web fonts off the
  box. Nunito is under the SIL Open Font License (`ui/src/lib/NUNITO-OFL.txt`); the glyphs are
  outlined again from Google Fonts' file before a public release.

## D23: The worst case has its own ceiling of 1 MB, and small allocations go to internal RAM first

- **Date**: 2026-11-21
- **Choice**: a second test, `memory_worst_case.rs`, builds a home with every bound of the core at
  its maximum and keeps its peak under 1 MB (measured 987 KB with every recipe log entry at 200
  bytes); D20's 300 KB stays the ceiling of the realistic home, the two never added. A test fails
  when a `pub const MAX_*` of the core is missing from its header. In the firmware,
  `SPIRAM_MALLOC_ALWAYSINTERNAL=4096` keeps allocations of up to 4 KB in internal RAM,
  `SPIRAM_MALLOC_RESERVE_INTERNAL=65536` keeps 64 KB of it for the network and DMA, and mbedTLS
  takes its buffers from PSRAM.
- **Why**: the realistic home says nothing about the day every bound is reached, and a bound added
  later must not escape the budget. The placement is ESP-IDF's usual one for a Wi-Fi application
  with PSRAM; the budget (`docs/memory-budget.md`) shows it fits on paper, with the internal RAM
  as the tight resource (most of the engine's bytes are small allocations). The figures are
  estimates until the box measures them, as D20's are. Approved by the user.

## D24: One network link, chosen at boot, Wi-Fi first

- **Date**: 2026-11-21
- **Choice**: one image holds Wi-Fi and Ethernet, and only one link is active, chosen once at
  boot from the setting `{"link": "auto" | "wifi" | "ethernet"}` and the cable: `auto` (the
  default, and the fallback for a missing or wrong setting) means Ethernet with a cable at boot,
  else Wi-Fi; nothing switches while running. The choice is a function of the core
  (`network::choose`); the firmware starts the link. Values are case-sensitive, as Sowel's enums.
  Wi-Fi is built first.
- **Why**: two links at once would add their memory (the budget takes the larger, Wi-Fi), and a
  switch while running is a source of states nobody tests. Wi-Fi first because QEMU has no Wi-Fi
  to test against and the box needs one to be reachable without a cable. Approved by the user.

## D25: The partition table has two application slots and a data partition

- **Date**: 2026-11-21
- **Choice**: `firmware/partitions.csv`, 16 MB, no factory slot: `nvs` 24 KB, `otadata` 8 KB,
  `phy_init` 4 KB, `ota_0` and `ota_1` of 4 MB each, `storage` (FAT with wear levelling, built
  into ESP-IDF) of about 7.9 MB. With `otadata` empty the bootloader starts `ota_0`. The table is
  not set in `sdkconfig.defaults`: `esp-idf-sys` ignores a custom table there (its README says to
  leave `CONFIG_PARTITION_TABLE_*` out), so the table reaches the box through `espflash
  --partition-table` and QEMU through `scripts/qemu.sh`, which builds its image from the same
  file and places the application at the offset of `ota_0` that the built table gives.
- **Why**: two slots allow an update that can fall back, and a data partition holds the home's
  files. The slots are 4 MB because the firmware is about 1 to 2 MB and the interface and
  recipes need room. FAT rather than LittleFS: it is in ESP-IDF, with no extra component. The
  table is cheap to change before the first flash and needs a full re-flash after. Chosen in
  autonomy, to be confirmed by the user at the demo.

## D26: The engine's loop is in the core, and the simulator is built into the QEMU image

- **Date**: 2026-11-21
- **Choice**: the radio loop and the saver of the host move to `crates/core/src/engine/`, generic
  over the radio transport, a `Storage` trait (the engine's four named files, written chunk by
  chunk, D15), clock closures, a random closure and an output closure; the host (files in a
  folder) and the firmware (the FAT partition) implement `Storage`. `crates/sim` builds for the
  ESP32: the firmware has a cargo feature `sim`, which embeds the simulator, the realistic home
  of D20 and its scenario (`firmware/fixtures/`) and runs the engine on them in a thread of 32 KB
  of stack; without it the image is the box's and starts no radio. This replaces spec 007's "the
  simulator is never built for the ESP32": it is built for the image QEMU runs, not for the box.
  The engine's heap peak and the internal RAM minimum are read by the firmware with ESP-IDF's
  local-minimum monitor of the free heap, phase by phase (load the home, build the simulator,
  run), and `scripts/qemu.sh` fails above 350 KB (D27, first 300 KB of D20) or below 64 KB of internal RAM free
  (spec 015's reserve).
- **Why**: the loop had to run on the box before the box is flashed, and moving it keeps one loop
  for both programs (the host's tests are its safety net). D20 said the 300 KB is "read again on
  the box (QEMU, then the Dongle Max)": the QEMU figure is an upper bound, with the simulator
  inside, and the box measures it again in E3. The monitor's figures are the sum of the low
  watermarks of the internal RAM and the PSRAM, which may have been reached at different
  moments: over the whole run the sum gave 329 KB for the first run, phase by phase 287 KB; both
  readings are upper bounds of the true peak, the per-phase one the tighter and never below it,
  so the gate uses the phases. A bound holds for what the run exercised: the realistic home,
  the scenario of `firmware/fixtures/` (motion, light orders), no network. Final run: 312 KB
  by phases, heap 170 KB at the end. `SPIRAM_MALLOC_ALWAYSINTERNAL` was
  tried at 512 as the first remedy (D23's knob): the total heap does not shrink when allocations
  move between the two memories, and the peak read 359 KB (whole-run sum), so it stays at 4096.
  What the peak holds: the 32 KB stack, the home loaded from its files (text and parsed data,
  about 260 KB at the highest), then the simulator; the loading is the largest transient and
  is the first optimisation to make if the ceiling is crossed.

## D27: The engine's ceiling is 350 KB in QEMU, replacing D20's 300 KB as the gate

- **Date**: 2026-11-21
- **Choice**: the heap peak of the engine in QEMU (`scripts/qemu.sh`) may reach 350 KB; D20's
  300 KB was the gate until then. Approved by the user on 2026-11-21, pending optimisation: the
  inbox of `docs/roadmap.md` has the line to bring the engine back under 300 KB, loading the
  home by streaming first. The Mac test (`memory_ceiling.rs`: 182 KB, 194 KB with every log full)
  stays at 300 KB, and D23's 1 MB for the worst case is unchanged.
- **Why**: once the scenario plays at its own pace, with motion and light orders (spec 016
  review), the QEMU run reads 312 KB by phases: the second phase (the recipes' start and the
  simulator's build) is the largest at 312 KB, the load of the home 258 KB, the run 197 KB, the
  identification 186 KB. Both readings of the monitor (whole run, per phase) are upper bounds, the
  per-phase one the tighter, and the figure holds for what the run exercised. Internal RAM stays
  at 107 KB free at the lowest, above the 64 KB floor. The measured run differs from the box: its
  saves are dropped (the last one written to flash), the simulator's memory is kept in RAM, and the
  scenario is clocked by its waits, so no File buffer or wear-levelling write is in the figure.

## D28: Pairing while serving

- **Date**: 2026-11-22
- **Choice**: pairing opens and closes while serving (spec 017), through
  `GET` and `POST /api/v1/integrations/solite/permit-join`, under Sowel's integration namespace
  (Sowel has no pairing route: in Sowel, Zigbee2MQTT pairs). The body takes Zigbee2MQTT's `time`
  (seconds, 0 closes, 180 when missing); `POST` answers 202 with the state asked for
  (`permitJoin`, `remaining`, and `error` when the radio refused the last opening), the radio
  loop applies it at its next turn. One window at a time; a new `POST` restarts it. No live event
  for the window (only Sowel's events, D18: the interface reads the state with `GET`). `--serve`
  starts the network as `--pair` does, forming one on a radio that has none, so a fresh box or
  simulator is set up from the browser alone; D18's `device.discovered` is sent. The window is in
  the core (`home/pairing.rs`, `engine::run`), so the box gets it with the API it serves.
  **Constraint**: the route has no authentication (D13): on the box (E3) it stays disabled
  (404) until the password of J4 exists.
- **Why**: J1 of the roadmap: add a device from the interface with no terminal, where `--pair`
  could not run together with `--serve`. The route under `/integrations/solite` follows Sowel's
  grouping of what an integration does; 202 with the state asked for avoids a new wait mechanism
  for a rare action whose refusal is read with `GET`.

## D29: Equipments from the interface

- **Date**: 2026-11-22
- **Choice**: Sowel's zone and equipment screens are ported (spec 018) with D3's seven types and
  without bindings edited one by one. The equipment form creates the equipment and its bindings in
  one `POST /api/v1/equipments` with `deviceIds` (alias = data key, the core's auto-binding of
  spec 010), not as Sowel's interface does, which computes a binding plan in the browser
  (`autoCreateBindings`, `bindingUtils.ts`) and sends one request per binding. The zone form has
  no "None" parent: Solite's tree has one root (spec 010 R7.10), and `parentId: null` means the
  root on `POST` and is refused on `PUT`.
- **Why**: J2 of the roadmap: the garage and its equipments made with the mouse. One request is
  atomic (no half-made equipment when a request fails) and runs the core's tested rules; the
  browser-side plan is over 800 lines of Sowel code the box's interface has no room for (170 KB
  budget).

## D30: The equipment is the entry point

- **Date**: 2026-11-22
- **Choice**: replaces D29's way of binding (D29's screens and zone form stay). The interface never
  gives a device a room (`Device.zoneId` is kept in the API for Sowel's format, D3, and read by
  nothing). The equipment form needs at least one device, since Solite has no equipment page to
  complete one. It binds as Sowel's interface does, from a plan computed in the browser (Sowel's
  `bindingUtils.ts` and `binding-candidates.ts`, ported for D3's seven types) and one request per
  binding, the equipment deleted again if one is refused. `POST /equipments` with `deviceIds` keeps
  Sowel's `createWithAutoBindings` (every key) for the API's clients. Sowel's Administration section
  is ported, with a Discovery screen per integration in the place of Sowel's Integrations entry.
- **Why**: Sowel's model (`equipments.md`, "Equipment vs Device"); the plan sits beside the selector
  that uses the same rule (Sowel's architecture: one module decides compatibility and bindings); the
  core keeps Sowel's route as Sowel's server has it, and the realistic home of D20, its firmware
  fixtures and memory figures are untouched. **D29's two arguments, reversed**: (1) "the browser-side
  plan is over 800 lines of Sowel code the box's interface has no room for": only the part for D3's
  seven types is ported (about 250 lines, about 5 KB gzip), and the first load stays far under the 170 KB
  budget (51.8 KB gzip, checked with `npm run check:bundle`, against a gate of 60 KB for this spec);
  (2) "one request is atomic": atomicity is given up for Sowel's bindings; a refused binding deletes the
  equipment again, and the half-made equipment is visible for the time of the requests (as in Sowel).
  The equipment made by the old way, every key bound, was wrong for the type, which outweighs atomicity.
  **Risk on the box**: one creation sends up to 33 sequential requests (1 + 16 + 16), one connection
  each, to an ESP32 that answers 503 `Busy` past 4 requests at once; sequential requests stay under it,
  but neither the Mac tests nor QEMU measure the box's time or connection behaviour; the hardware demo
  (from the Mac) and E3 on the box are the checks.

## D31: The simulator runs on the box in a measuring image; esptool replaces espflash

- **Date**: 2026-11-22
- **Choice**: replaces the part of D26 that kept the simulator out of the box ("not for the box") and
  the part of D25 that gave the partition table to the box "through `espflash --partition-table`".
  The simulator now also runs on the Dongle Max, in a third image, the measuring image (cargo feature
  `flash-saves`, which turns on `sim`): the realistic home with every save written to the flash and
  timed, to measure on the real ESP32 what QEMU could only bound. The box's own image never holds the
  simulator. Every write to the box goes through `sh scripts/box.sh`, which uses `esptool.py` for each
  step (identify, backup, restore, flash) and refuses to flash without a backup that was restored once;
  espflash is dropped, and the `runner` line of `firmware/.cargo/config.toml` that flashed with it is
  removed, so `cargo run` in `firmware/` flashes nothing. The EFR32 radio stays wired but not driven
  (no file of `firmware/src/` names GPIO12 or GPIO15).
  The prompt of `box.sh` reads the end of the box's MAC from `/dev/tty`: that stops a pipe and a
  script's plain standard input, not a pseudo-terminal. The guarantee is the user's explicit yes
  in the conversation plus the MAC typed, not the terminal alone.
- **Why**: QEMU's flash takes a quarter of an hour for the saves the box does in seconds, and says
  nothing of the box's timing; a measuring image answers (free memory at boot, the engine's peak, the
  FAT partition, the save stall) without waiting for the radio's serial link. One tool to fake in the
  tests, the same image parts as `scripts/qemu.sh` (bootloader, table from `partitions.csv`, `ota_0`
  offset), and a runner that skipped every check was a write path to the box (D6).

## D32: The radio over the UART in a radio image, pairing at boot

- **Date**: 2026-11-22
- **Choice**: a fourth image (feature `radio`) drives the EFR32 over UART1 (TX GPIO17, RX GPIO13,
  8N1, no flow control), a 4 KB receive ring and the interrupt in internal RAM; the radio and the
  engine share one thread on core 1 with 32 KB of stack. The interrupt setting is in
  `sdkconfig.radio.defaults`, built in `target/radio`, so the other images keep their settings.
  The speed is probed at each boot, the fixed `RADIO_BAUD` first, then 115200, 460800, 921600.
  The core describes the pins by role (reset active low, bootloader active high, never set
  active) and the reset sequence; the firmware runs it after the ESP32's boot, each level written
  before the pin becomes an output, GPIO12 low before GPIO15 (both strapping pins). GPIO12 and
  GPIO15 are named in `firmware/src/radio_uart.rs` only (a test). Pairing opens for 180 s at a
  power-on boot when the marker `/data/booting` (removed after 10 minutes of running) is absent: a
  crash, a watchdog, a brown-out or a boot loop opens nothing. The Zigbee backup is
  `/data/zigbee.json`, the network and trust-centre keys in plain text on a flash that is not
  encrypted, accepted until flash encryption is decided. Only a radio error restarts the radio
  (reset, probe, resume, never a forming nor a window), at most 3 times in 10 minutes; a storage
  error stops the loop. The radio image is a development image, in no public release before J4.
  Replaces D31's "the EFR32 radio stays wired but not driven" and D26's "without it the image is
  the box's and starts no radio" for the radio image.
- **Why**: the simplest bounded way to pair without a network (a console command shares the log's
  port, the button's pull-up is unknown); the probe finds a radio at another speed for one try's
  cost; the ring holds more than the radio's ASH window, and the interrupt keeps the FIFO (11 ms at
  115200) drained while a flash write stalls the caches; logic in the core is tested on the Mac.

## D33: The radio image serves the API on the local network, over Wi-Fi

- **Date**: 2026-11-22
- **Choice**: the radio image (feature `radio`) joins the Wi-Fi network of `/data/wifi.json`
  (written by `box.sh wifi` through the firmware's serial console; never NVS, never git) and serves
  Sowel's API, `/ws` and the interface on port 80, with no authentication. A `Host` is
  accepted when it is a private IPv4 address (10/8, 172.16/12, 192.168/16, 127/8), `localhost` or
  the box's name `solite-<6 hex>` (alone or with `.local`, `.lan`, `.home`, `.home.arpa`); the
  pairing route answers 404 (D28). ESP-IDF's HTTP server through `esp-idf-svc`, 4 workers by its
  asynchronous requests (their stacks and 16 KB body buffers in PSRAM), at most 12 connections in
  a socket table of 16, TCP keep-alive; on `/ws` the refusals are close
  codes after the upgrade (4003, 1013), the development origins of `live::session::origin_allowed`
  (`http://localhost:5173`, `http://127.0.0.1:5173`) are accepted as the host does (decided by the
  user, 2026-11-22: a development page on a machine of the home's network can open the live events),
  only `bearer.none` is echoed, and a dead client is found by
  TCP keep-alive instead of 60 s of silence; a client that cannot be written to holds the server's
  task up to 5 s. The interface is embedded at build time, gzip only, from flash. Wi-Fi power save
  off; SNTP with `pool.ntp.org`; the engine's saves wait at most 20 s for the clock, the radio and
  the pairing window of a power-on do not. `flate2` is in the firmware's build (never its code).
  The setting `ethernet` starts no link in this image. Built in two specs: the rules in the core,
  the packing and the scripts (spec 022), the firmware (spec 023).
- **Why**: the true north's box is reached from the home's network; the radio image is a
  development image, in no public release (D32), so an open API on the local network is
  accepted, also while the box is the family's annex controller (the user, 2026-11-22), on two
  conditions: the box stays on the home's own network (not a guest network), and no router port is
  forwarded to it. The guards are the Mac host's (Host check against DNS rebinding, JSON-only
  bodies against forms, Origin on `/ws`), and the pairing route stays off. The settings file
  survives a flash with `--keep-storage`, which NVS would not.

## D34: The slow checks run every night and on the Mac, not on each pull request

- **Date**: 2026-11-22
- **Choice**: the slow checks are the Rust tests marked `#[ignore = "slow ..."]` (each waits a
  production delay: an order's 12 s deadline, the 10 s wait for the network, the 10 s limit of
  `measure-api.sh`, a minute of the garage at speed 10) and the engine's run in QEMU. No test is
  ignored for another reason. `sh scripts/validate.sh` runs them (`cargo test -- --include-ignored`
  and `scripts/qemu.sh`), so the Mac has the full verdict;
  `--skip-slow` leaves them out, and Claude's commit hook runs it. The CI (`.github/workflows/ci.yml`, D9) runs `--skip-slow` on
  pull requests and pushes to `main`, and the full verdict every night at 02:00 UTC on `main` and
  by hand (Run workflow). The git pre-commit hook stays `--fast`. The firmware job caches the Xtensa
  toolchain espup installs, keyed by espup's version and `firmware/rust-toolchain.toml`. The tests
  of `serve.rs` and `ws.rs` pair the garage once per file and copy its folder into each test.
- **Why**: the pull-request CI took about 8 minutes, most of it waiting (real pairing windows,
  production timeouts, QEMU's run); the user chose (2026-11-22) a fast verdict on each pull
  request and the slow checks every night and on the Mac, where each one still runs.

## D35: The backup of the original firmware is optional for `flash`

- **Date**: 2026-11-22
- **Choice**: replaces, for `box.sh flash`, the mandatory backup and the MAC confirmation that
  followed D6 (spec 020, D31's "refuses to flash without a backup that was restored once").
  `flash` takes `--backup <file>` as an option: a backup given is still checked (16 MB, the name
  holds this box's MAC, its SHA-256 in a row of `docs/hardware.md`'s backup table with a restore
  date); without it the plan says the way back is Sonoff's own reflash. `flash` asks no question
  beyond the command itself; `backup` and `restore` keep the question of the MAC's last 6 hex
  digits. D6's rule that the box is not flashed without the user's yes stays, as a rule for Claude
  in `CLAUDE.md`.
- **Why**: the user does not need the original firmware's backup: Sonoff supplies its own reflash
  (its web flasher, or its Home Assistant add-on), which brings the box back to its original
  firmware. The backup and its proven restore stay available for whoever wants them.

## D36: The pairing window also opens from the serial console, never from the network

- **Date**: 2026-11-22
- **Choice**: the radio image's serial console (spec 022 R9) takes a third line, `pair`, sent by
  `sh scripts/box.sh pair` over the USB cable: the engine opens the pairing window for 180 s
  (`PAIR_AT_BOOT_SECONDS`), whatever the boot was and whether the marker `/data/booting` is there,
  and logs `pairing: open for 180 s` (or `pairing: failed: <reason>`). It completes D32's window
  at a power-on boot, which stays as it is, and keeps D28's rule for the box: the pairing route
  answers 404 on the network until J4's password. D32's "a console command shares the log's port"
  is no longer a reason against it: the console of D33 already reads that port. The window still
  opens only from what a person does by hand at the box (power, or the cable), never from the
  local network.
- **Why**: on the real box (2026-11-22), pairing the ZBMINIR2 needed a power-on more than 10
  minutes after the previous boot: opening the serial port resets the box and, within 10 minutes,
  the marker refuses the window; a software restart opens none. A line on the cable that already
  carries the Wi-Fi settings opens it at once, with no new input path and no exposure on the
  network (the API has no authentication, D13).

## D37: `/ws` is registered directly with ESP-IDF on the box

- **Date**: 2026-11-22
- **Choice**: the radio image registers `/ws` itself with `httpd_register_uri_handler`
  (`is_websocket`, the subprotocol `bearer.none`, `ws_post_handshake_cb`), with
  `CONFIG_HTTPD_WS_POST_HANDSHAKE_CB_SUPPORT=y` in `sdkconfig.radio.defaults` (checked by
  `validate.sh`), instead of `esp-idf-svc`'s `ws_handler`. The post-handshake callback, on the
  handshake's request, reads `Host` and `Origin`, takes a slot of the core's `LiveHub` or closes
  (4003, 4003, 1013), sets a session context released by ESP-IDF when it deletes the socket (which
  frees the slot) and sends the welcome. The live thread hands each frame to the server's task
  with `httpd_queue_work` and `httpd_ws_send_frame_async`, and waits for its result; a frame for a
  session already closed is not sent (its socket number may belong to a new connection), and a
  close asked by the live thread is a shutdown of the socket run on the server's task, for a
  session still open. The rest
  of the server stays `esp-idf-svc`'s. D33's other `/ws` rules are unchanged.
- **Why**: with ESP-IDF 5.5, the WebSocket handshake never calls the URI handler, only the frames
  that follow do, and `esp-idf-svc` 0.53 builds its session (its `New` case) on the handshake's
  call: on the box (2026-11-22) a browser got its 101 and then nothing, no welcome and no event.
  The post-handshake callback is ESP-IDF's own place for that work, on the request that still holds
  the headers.

## D38: A recipe settles when what it uses changes

- **Date**: 2026-11-23 (spec 024, decided in an autonomous run, for the user's review)
- **Choice**: right after an equipment is deleted or changed, a binding removed, a device removed
  or a zone deleted, the core settles the recipe instances: a deleted equipment's id leaves the
  settings of every instance that names it (warning line); every enabled instance is validated
  again and, when it fails, stopped and **disabled** (`Disabled: <reason>`, events `stopped` and
  `error`); a running one whose settings lost an id restarts with them; the instances of a deleted
  zone are deleted. `enable` refuses (400 `Invalid params: <reason>`) an instance that does not
  validate. It diverges from Sowel, which keeps the stale ids, lets the instance fail on each order
  and enables an instance that cannot start.
- **Why**: the box runs for a month with nobody reading its logs; an instance that looks enabled
  but sends orders to an equipment that is gone is a state the interface cannot show. Disabled
  with its reason, it shows as a switch off and a line in its journal, and it is never restarted
  in error at each boot. A deleted zone's instance could never be valid again (a new room has a
  new id) and would be shown nowhere, so it goes with its zone. The settle walks at most 16
  instances x 32 ids and never fails the request that caused it.

## D39: A budget of flash writes for the tests on the box

- **Date**: 2026-11-23, proposed in an autonomous run from the user's instruction; finalised
  2026-11-24 by spec 025, which gives it its means (D40)
- **Choice**: a test on the real box causes at most one flash write per file the flow changes, or
  one write and a restart to check that it persists. A run on the box is one pass of the flow being
  shown, never a loop: no repeated `measure-api.sh` (at most once per milestone), no burst or
  stress against the box. Loops, measures and stress run on the Mac (`--sim`) or under QEMU. The
  flow runs inside `sh scripts/box.sh saves --budget <n>` (saves held, one flush at the end), with
  `n` = 1 (the boot marker, written because opening the port resets the box) + the number of files
  the flow changes (4 at most: devices, zones, equipments, recipes) + 1 if the session lasts over
  10 minutes (the marker's removal); a restart to check persistence is one `box.sh log --budget 1`.
  Each spec that runs on the box writes in its result the `flash: <n> writes since boot` count of
  its run and the images it flashed.
- **Why**: the box's flash wears with writes (each save of a JSON file rewrites it, and a flash
  of an image rewrites megabytes); the box is meant to run for years in an annex, and the test
  runs should not spend its endurance. On 2026-11-22 eight runs of `measure-api.sh` and 60 zone
  creations and deletions cost hundreds of saves.

## D40: Saves held from the serial console, writes counted, definitions coalesced over 2 s

- **Date**: 2026-11-24 (spec 025, decided in an autonomous run, for the user's review)
- **Choice**: the radio image's serial console reads `saves hold` (nothing written, everything in
  RAM, forgotten at a restart) and `saves flush` (one write per changed file, then the hold ends),
  never from the network (as `pair`, D36). Because opening the serial port resets the box, the
  hold and its flush are sent in one session of the port by `box.sh saves`, which runs the test's
  command in between; there is no separate `flush` command. Every firmware image counts its writes
  to `/data` and logs `flash: <n> writes since boot`; `box.sh saves` and `box.sh log` check it
  against `--budget`. In the radio image and the host's `--serve`, a change of definitions is
  written 2 s (`DEFINITION_DELAY_MS`) after the saver first sees it unsaved, counted from the
  first change so that a stream of changes is still written every 2 s; a stop writes at once.
  This replaces D13's "definitions at once (the user's choice)": a power cut now loses at most the
  definitions of the last 2 s (and one loop turn), the values of the last 10 minutes as before, and
  everything since a hold. A delay of 0 gives back the former rule.
- **Why**: the 33 requests of the interface's largest creation gave 33 rewrites of
  `equipments.json`; with the delay they give one per 2 s, and inside a held test one in all. Two
  seconds is shorter than any action a person repeats by hand, and a power cut within two seconds
  of a change is rare; the hold and the counter make the budget of D39 something a script checks,
  not a promise.

## D41: The home's time zone is a setting picked from a fixed list, with POSIX rules in the core (the key: see D44)

- **Date**: 2026-11-24 (spec 026, decided in an autonomous run, for the user's review)
- **Choice**: Sowel's settings keys (`home.name`, `home.latitude`, `home.longitude`,
  `home.sunriseOffset`, `home.sunsetOffset`) plus Solite's own `home.timezone`, a name from a fixed
  list of 59 IANA names compiled in the core, each with its POSIX rule
  (`CET-1CEST,M3.5.0,M10.5.0/3`), served by Solite's own `GET /api/v1/settings/timezones`. The core
  parses the rule and converts UTC to local time itself; the sun times are a port of suncalc 2, as
  Sowel's sunlight manager uses, on the root zone only. Unknown keys and values that break a strict
  pattern are refused (Sowel accepts any key and parses with `parseFloat`); `""` is stored and reads as unset (as Sowel); a
  new location or time zone applies at once (no `system.restart_required`). Until this, the root
  zone's `isDaylight` was always `null` (D-history of spec 012).
- **Why**: Sowel takes its time zone from `TZ` or derives it from the location with `tz-lookup`,
  whose database does not fit the box, and the box has no `TZ` to set. A short list with today's
  rules covers the homes the box is for; a rule changed by law, or a zone without a fixed yearly
  rule (Casablanca, Cairo), is out until a list update. The sun times decide motion-light's
  "Inactive during day", the J6 test.

## D42: The backup is the home's settings and zones, as one JSON document of at most 16 KB (replaced by D56)

- **Date**: 2026-11-24 (spec 026, decided in an autonomous run, for the user's review)
- **Choice**: `GET /api/v1/backup` gives `{"version":1,"settings":{...},"zones":[...]}`;
  `POST /api/v1/backup` takes it as a JSON body, checks it whole, and only then replaces the
  settings whole and creates or updates the document's zones by id (none deleted; equipments,
  devices and recipes untouched; an unchanged zone not rewritten). Both ways at most
  `MAX_BODY_BYTES` (16384 bytes): an export beyond is refused with 413, so that every export can be
  imported back. Sowel's backup (its SQLite tables, a ZIP, a file upload) is not mirrored.
- **Why**: equipments, recipes and devices depend on the devices paired to one box and on its
  radio network's keys; a backup of them is a different feature (secrets, a body beyond the box's
  16 KB worker buffers, a streamed import). The settings and the rooms are what can be prepared on
  the Mac and moved to a box, and fit the existing bound. Merging zones rather than replacing them
  avoids deleting a room that holds equipments.

## D43: Solite is a checked subset of Sowel (replaces D3's equipment types and D14's kinds and flags)

- **Date**: 2026-11-24 (spec 027, decided in an autonomous run, for the user's review)
- **Choice**: every name and value Solite has is Sowel's (`src/shared/types.ts` wins), or a
  deviation listed with its decision in `docs/sowel-subset.md`; anything outside the subset is
  refused or kept untouched. A test (`crates/core/tests/sowel_contract.rs`, a step of
  `scripts/validate.sh`) checks it against a snapshot of Sowel checked in
  (`contract/sowel-snapshot.json`) and against Sowel's repository when present: a snapshot
  behind Sowel is a warning, Solite not fitting Sowel's repository a failure. `EquipmentType` has
  Sowel's 31 values; eight have Sowel's binding rules (D3's seven and `light_color`, the most
  common Zigbee light), the others none. Sowel's twelve optional equipment fields are kept as
  stored; `PUT` takes the four whose rules are shape only (`requireConfirmation`,
  `plugInHybrid`, `trackOrderOutcome`, `family`, with Sowel's `FamilyNotApplicable` and the
  separate-supply and local-production rules on the resulting type) and refuses the eight whose
  rules need what Solite lacks (`energyProfile`, `solarProfile`, `timedCommand`,
  `meteringParentId`, `usualVehicleId`, `separateSupply`, `localProduction`, and
  `invertDirection`, which Sowel applies to orders). The zone aggregation leaves out a `weather`
  equipment and a `water_valve`'s bindings as Sowel does, but has none of Sowel's own counts for
  the types without behaviour (`doorsTotal`, `displaysTotal`, `waterValvesTotal` stay 0), nor
  Sowel's stale rule for the electrical data of meters.
  Value-level differences kept: `null` not stored (Sowel's "absent / null = default"); the texts
  `Not supported: <field>` and `Invalid <field>` (Sowel answers with its schema validator's own
  body); a string field at most 64 characters; one equipment's fields at most 1024 bytes of JSON
  and all of them 8192 (Sowel has no bound), past which a `PUT` is refused and a field is dropped
  at load with a warning; an object compared by its bytes. Names kept as deviations: the routes
  of D28 (permit-join) and D41 (`/settings/timezones`); the backup document of D42 has Sowel's
  paths and its own body.
- **Why**: the user's rule (2026-11-24): Solite becomes an extension of the full Sowel, so a
  client, an export or a future import must read the same model; a check catches a drift that
  care alone misses. The bounds keep the worst case under D23's 1 MB (1004 KB with them).

## D44: Sowel's `GET /api/v1/system/timezone`, and `home.timezone` as a Sowel key (narrows D41)

- **Date**: 2026-11-24 (spec 027, decided in an autonomous run, for the user's review)
- **Choice**: Solite serves Sowel's `GET /api/v1/system/timezone` (`{tz, source, offsetHours}`).
  `home.timezone` is a name Sowel has (its `GLOBAL_READABLE_KEYS`, the keys every plugin may
  read), so it is no longer Solite's own key; Sowel's own time zone does not read it yet, and the
  Sowel issue asking it to stays open. Differences kept: with the setting, `source` is `env` (the
  setting holds the POSIX rule a `TZ` variable would hold; Sowel's three values are `env`, `auto`,
  `fallback`); without it, `fallback` even with a location (Sowel's `auto` needs `tz-lookup`'s
  database, too large for the box); `offsetHours` is the offset now, Sowel's is taken at start.
  `GET /api/v1/settings/timezones` stays (D41).
- **Why**: Sowel's interface reads that route (its current-time pill, the rain summary); the
  list route has no Sowel counterpart, and moving the list into the interface would duplicate the
  core's table of POSIX rules.

## D45: Device dates taken before the clock is set are moved by its jump

- **Date**: 2026-11-25 (spec 028, decided in an autonomous run, for the user's review)
- **Choice**: the home keeps the last wall time it was given (by each turn of the radio loop and
  each tick of the saver). When it was before 2020 and the time given is after, every device date
  before 2020 (`lastSeen`, `createdAt`, `updatedAt`, the data's `lastUpdated` and `lastChanged`)
  is moved forward by the difference. A `lastSeen`, `lastUpdated` or `lastChanged` read in 1970
  from storage is `null`. The shift marks the values as changed, never the definitions.
- **Why**: the box starts in 1970 and gets the time 20 s later; a device heard in between showed
  1970, turned `offline` at the jump and wrote 1970 to the flash. Shifted, its date is right
  within one turn of the loop and it stays `online`; `null` would lose the last message. The last
  wall time lives in the home, not in the loop, so a radio restart (which calls the loop again)
  does not lose the crossing. Marking only the values adds no flash write at boot. Other dates
  taken during the wait (recipe logs and state keys, equipments and zones) are not moved.

## D46: A motion-light keeps running when a button goes stale, and follows its zone's sensors (narrows D38)

- **Date**: 2026-11-25 (spec 028, decided in an autonomous run, for the user's review)
- **Choice**: when the recipes settle, a motion-light button whose equipment no longer has an
  `action` data binding leaves the settings (`Button removed from settings: <name>`), instead of
  the instance being disabled. A running instance restarts only when its lights lost an id; a
  change of its buttons alone, and the motion sources of its zone, are taken in place
  (`Runner::refresh`), its timers and state kept. The settle also runs after an equipment is
  created, a data binding added and a zone updated. Diverges from Sowel's motion-light, which
  resolves its sources once at its start.
- **Why**: a box that runs a month alone must not stop lighting the garage because a button's
  device was removed, nor switch a lit garage off (a restart's start sync) for it; and a motion
  sensor replaced, unbound or moved must count, or stop counting, for the safety auto-off without
  anyone restarting the recipe. Creation and the form still refuse a button without `action`.

## D47: One account, opaque tokens in RAM, PBKDF2-HMAC-SHA256 (narrows D13 and D33)

- **Date**: 2026-11-25 (spec 029, decided in an autonomous run, for the user's review)
- **Choice**: the home has one account, Sowel's admin, set up at first start (no default
  password, EN 18031-1). The password is hashed with PBKDF2-HMAC-SHA256 (RFC 8018), a 16-byte salt
  from the caller's random source, 10 000 iterations (stored with the hash, so a later change still
  checks the old ones), a 32-byte result, compared in constant time; 8 to 128 characters and at
  most 512 bytes (Sowel asks 6: 8 is NIST SP 800-63B's floor). The hash and the user are in
  `auth.json`, written by the saver like `settings.json` (one write per setup or reset, none per
  login); a file that cannot be read stops the start, it is never read as "no account", which would
  open the setup to anyone. A login or a setup opens a session: an access token (900 s) and a
  refresh token (30 days), each 16 random bytes as 32 hex characters, opaque (not a JWT), kept in
  RAM only, at most 8 (a 9th drops the first), lost at a restart. Sowel's five `/auth` routes and
  its gate's three errors are mirrored; setup and login together are limited to 10 attempts per
  minute for the whole box (the core does not see addresses), a 429 beyond, without hashing. Three
  RustCrypto crates enter the core (`sha2`, `hmac`, `pbkdf2`, no default features): std has no hash
  (D12), and a hash written by hand is worse than three audited crates.
- **Why**: a box on the home's network (D33) with no authentication is not acceptable for a product
  (EN 18031-1: no common default password, protection against brute force). The cost is reasoned
  for the ESP32: one iteration is two SHA-256 compressions of about 25 to 50 µs in software at 240
  MHz, so a hash takes 0.5 to 1 s on the HTTP worker that asked, never under the home's lock; the
  ESP32's SHA accelerator (more iterations) is an adapter for later. Tokens in RAM cost no flash
  write and a restart only asks the owner to log in again; opaque tokens need no JWT library, no
  clock agreement with a client and can be closed at logout (stricter than Sowel's JWT). One rate
  limit for the box accepts that a device of the home network can keep the owner from logging in
  while it tries, open sessions and their refresh working: accepted for a home network. Sowel's
  users, roles, API tokens, MFA and password change stay out (password change: roadmap inbox).

## D48: The confirmation at the box is an adapter, and the factory reset is Solite's route

- **Date**: 2026-11-25 (spec 029, decided in an autonomous run, for the user's review)
- **Choice**: the setup and the factory reset need a person at the box. The core defines the
  adapter `Presence` (`confirmed()`, `hint()`), held by the home (`Home::set_presence`), counting
  time on a monotonic uptime clock given to it, never the wall clock, which jumps when it is set;
  the default never confirms. The Mac host confirms always (`AlwaysConfirmed`: it serves on
  loopback, the person at the Mac is at the host). Until the box's button is wired, the box
  confirms during the 10 minutes after a power-on boot (`BootPresence`, the reset reasons of D32's
  pairing window, never after a software restart or a crash). `ButtonPresence` (press, confirmed
  for 120 s) is in the core for the firmware's button later. A refusal is 403
  `{"error": <hint>, "confirmRequired": true}`. `POST /api/v1/system/factory-reset` (Solite's, no
  Sowel counterpart) erases the recipe instances, equipments, devices, zones but the root (its name
  and description back to a first start's), the settings, the account and the sessions, as the
  API's deletions do (with their live events), and answers 409 while a pairing window is open. It
  keeps the Zigbee network and its keys (`zigbee.json`), the Wi-Fi settings and the network
  setting. `Home::factory_reset` is what the button's long press will call.
- **Why**: EN 18031-2 asks that the user can delete their data, and the first person on the home's
  network must not be able to take a box that was not set up. Confirmation by a physical action
  at the box is the answer; the real button (pin, short and long press) needs the box in hand, so
  the core takes the decision behind an adapter, tested here, and the box's 10 minutes after a
  power-on stand in for it. The factory reset leaves the radio network: forgetting its keys would
  make every paired device unreachable, and the forgotten devices come back when they announce
  themselves during a pairing window, so the deletion is partial (an inbox line).

## D49: The box's pairing route behind the token, and `/ws`'s token in the second subprotocol (replaces D28's constraint and D36's "never from the network", narrows D13 and D33)

- **Date**: 2026-11-25 (spec 029, decided in an autonomous run, for the user's review)
- **Choice**: every route of the API and `/ws` need Sowel's bearer token on the Mac host and on
  the box alike; the interface's files stay public. The box's pairing route
  (`/api/v1/integrations/solite/permit-join`) answers as on the Mac, behind the token: D28's
  "stays disabled (404) until J4's password exists" and D36's "never from the network" are
  replaced. The power-on window (D32) and the console's `pair` (D36) stay until the button is
  wired. On `/ws` the token is read from the `Authorization` header or from the first offered
  subprotocol `bearer.<token>` other than `bearer.none`; the interface offers `bearer.none` then
  `bearer.<access token>`, because ESP-IDF echoes only a fixed subprotocol and reads at most 49
  characters of the header: the box echoes `bearer.none` and reads the whole header itself. After
  the upgrade, no token closes 4001 `Authentication required`, a token that is not a live access
  token closes 4001 `Invalid token`; a session that ends later does not close an open socket (as
  Sowel, which checks the token at the handshake). A client that offers only `bearer.<token>`, as
  Sowel's own interface does, gets no subprotocol echoed on the box and its browser drops the
  socket (a deviation, `docs/sowel-subset.md`). The firmware sets ESP-IDF's `httpd_ws` log to
  errors only, so the offered subprotocols (the token) are never written to the serial log.
- **Why**: with a password and a token the reason for D28's and D36's rule (an API open to the
  home network) is gone, and the interface can open pairing from "Add a device" on the box as on
  the Mac. The token goes where a browser can put it (the subprotocol: a browser cannot set
  headers on a WebSocket), and the fixed echo is the one thing the box's server allows.

## D50: The equipment and device pages, Solite's Enabled row, an equipment created without a device (narrows D19 and D30)

- **Date**: 2026-11-25 (spec 030, decided in an autonomous run, for the user's review)
- **Choice**: Sowel's `EquipmentDetailPage` and `DeviceDetailPage` are ported, cut to what Solite
  serves (no history, no type panels, no raw expose), at `#/equipments/<id>` and `#/devices/<id>`
  (no router, D19's hash); D19's "leaf components, not its pages" is narrowed: whole pages are
  ported when they fit the budget. The equipment page adds one row Sowel's interface does not
  have, **Activation** (titled so, its summary "Enabled" or "Disabled"; Sowel's `ConfigRow` and `Switch`, `PUT {"enabled"}`, which Sowel's API
  takes), so a disabled equipment is enabled again without `curl`. The creation form no longer
  needs a device (D30's "at least one device, since Solite has no equipment page" goes): with none,
  the equipment is created alone and its page opens. "Change devices" removes the bindings then
  writes the creation form's plan, stopping at the first refusal (Sowel skips a refused removal),
  not atomic, as Sowel's.
- **Why**: J2: an equipment completed, trimmed or moved to another device from the mouse; Sowel's
  form allows an equipment without device; a refused removal skipped could leave a binding under
  an alias the new plan then fails to write.

## D51: The colour of a bulb as Zigbee2MQTT's object, Sowel's `json` type

- **Date**: 2026-11-25 (spec 031, decided in an autonomous run, for the user's review)
- **Choice**: a device with the Color Control cluster (`0x0300`) gets the data `color_temp`
  (number, `light_color_temp`, mired) and `color` (Sowel's `json` type, `light_color`), and the
  orders `color_temp` (`set_color_temp`, 150 to 500 mireds, Zigbee2MQTT's default range, not read
  from the bulb) and `color` (`json`, `set_color`). The `color` value is Zigbee2MQTT's object, at
  most `x`, `y` (0 to 1), `hue` (0 to 360), `saturation` (0 to 100), in two groups: the colour
  measures of one frame replace their fields, and a frame of one group drops the other group (no
  `color_mode` is kept). A `color` order takes `{x, y}` (Move to Color) or `{hue, saturation}`
  (Move to Hue and Saturation), options 0 as Zigbee2MQTT (a bulb that is off may ignore it).
  Reporting is set up for the colour temperature and X/Y; hue and saturation are read back after a
  hue and saturation order; a read-back carries up to four attributes in one request, so a colour
  comes back in one update. Every `0x0300` device gets both colour data and orders:
  `ColorCapabilities` is not read, so a tunable white bulb has a `color` that its bulb refuses.
  The memory worst case puts the four-field object (632 bytes on the Mac) on its 16 lights, not on
  its 64 devices: a colour bulb has 4 data where the test's devices have 16, and the 12 it lacks
  weigh more than its object. A `devices.json` holding `"type":"json"` is not read by an older
  release (the host stops, the file is kept): going back needs the device paired again or the file
  removed.
- **Why**: Sowel's model names the colour `color`, `light_color`, typed `json`
  (`src/shared/constants.ts`, `binding-candidates.test.ts`), with Zigbee2MQTT's payload; one
  update per colour keeps clients and recipes from seeing a half-changed colour; reading the
  capabilities is a new step of the pairing setup, left for later (inbox).

## D52: Updates are signed with Ed25519 over a header carrying the image's SHA-256; the key is given at build time

- **Date**: 2026-11-25 (spec 032, decided in an autonomous run, for the user's review)
- **Choice**: an update is one file (`.sup`): a 128-byte header (magic `SOLITEUP`, format 1,
  version as three u16, image length, the image's SHA-256), whose first 64 bytes are signed with
  Ed25519 (RFC 8032, checked with `ed25519-dalek`'s `verify_strict`, small-order keys refused),
  then the image. The program checks the signature and the version before writing anything, then
  hashes the image as it streams it into the inactive slot, and marks it bootable only when the
  digest matches. The public key is the build-time variable `SOLITE_UPDATE_PUBLIC_KEY` (64 hex),
  read by the host and the firmware with `option_env!`; a build without it refuses updates (503).
  The private key is made by `solite-host --update-keygen` into a file outside any git working
  tree, and used by `--update-sign`; it is never in the repository, and the tests make their key
  pairs at run time.
- **Why**: signing a small header rather than the image lets the box refuse a forged file before
  erasing a byte and verify a 4 MB image with a 4 KB buffer (Ed25519 needs its whole message;
  the prehashed variant brings nothing over a signed digest). Ed25519: small keys and signatures,
  fast verification without tables, a pure-Rust crate that builds for the Xtensa target, and
  `sha2` already in the core. The key at build time, not in a file of the box, so that a reset
  data partition cannot swap it. ESP-IDF's Secure Boot V2 (and flash encryption) would also
  protect against a USB flash, but it burns eFuses irreversibly and would end the Dongle Max's
  return to its original firmware (D6): it stays in the inbox, the update's own signature protects
  the channel the true north opens (updates from afar).
- **Rejected**: RSA-PSS (keys and signatures ten times larger, slower on the ESP32); ESP-IDF's
  signed app verification without Secure Boot (needs the ECDSA/RSA scheme tied to the
  bootloader's settings, and a flash to change); a key file on the data partition (a factory
  reset or a lost partition would change who can update the box).

## D53: Sowel's `POST /api/v1/system/update` takes the signed file as its body; `GET /api/v1/system/version` as a subset

- **Date**: 2026-11-25 (spec 032, decided in an autonomous run, for the user's review)
- **Choice**: the update is uploaded to Sowel's route `POST /api/v1/system/update`, behind the
  token of spec 029, with the signed file as its body (`application/octet-stream`), streamed
  without the 16 KB body limit of the other routes; it answers Sowel's `{success, message}`, and
  Sowel's errors `{error}` (409 `Update already in progress`, 503 when updates cannot run). One
  installation at a time; none while the running image is on probation (the inactive slot holds the
  image to fall back to) or while an installed update waits for the restart. A version equal to or
  older than the running one is refused (no downgrade through the API). `GET
  /api/v1/system/version` answers Sowel's `VersionInfo` without its two Docker fields, `latest`
  `null` and `updateAvailable` `false` (no release poll yet). Every refusal of the upload route
  closes the connection, so the image's bytes are never read as a next request.
- **Why**: the same path and answer as Sowel keeps one API for both (D3); Sowel's route takes no
  body because it pulls its Docker image itself, which a box without a release feed cannot do yet:
  the body is the deviation, recorded in `docs/sowel-subset.md`. A downgrade would let a stolen
  token reinstall an old signed image with a known flaw; going back is a USB flash with the
  user's yes (D6).

## D54: An updated image confirms itself after a healthy minute, or rolls back after five; a release reads what its successor wrote

- **Date**: 2026-11-25 (spec 032, decided in an autonomous run, for the user's review)
- **Choice**: an image booted from an update is on probation. Healthy means the engine's loop is
  running and the API server is listening (not the Wi-Fi link). Healthy for 60 s without a break:
  the image confirms itself; not confirmed 300 s after the start (monotonic clock): it is marked
  bad and the previous image becomes the next boot (the box restarts). A crash, a restart or a
  power cut before the confirmation rolls back at the next boot, by ESP-IDF's bootloader
  (`CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE`); that setting lives in the bootloader, which an update
  never replaces, so it reaches a box through one USB flash of a spec 032 image. From now on, a
  change of a stored file's format stays readable by the previous release (it adds, it does not
  rename or retype), because a rollback runs the previous release on the data its successor
  wrote. The first image that can roll back is spec 032's, which already reads `json` data (D51).
- **Why**: a running engine and a reachable API are what the user needs from the box; requiring
  the Wi-Fi link would roll back a good update when the router is off. A minute catches an image
  that crashes or stalls at start; five minutes leave room for a slow radio start. Without a USB
  flash first, a box keeps working and accepts updates but cannot roll back: said in the box run.

## D55: The box session's `--yes-flash` is the user's yes, given once on the command line, for that run (refines D6 and D39)

- **Date**: 2026-11-25 (spec 033, decided in an autonomous run, for the user's review)
- **Choice**: `scripts/box-session.sh` runs the roadmap's box checks in one pass. The user typing
  `--yes-flash` in their own terminal is their D6 yes for that run only: one USB flash of the radio
  image with `--keep-storage`, the API writes of the checks inside `box.sh saves` with D39's
  budgets, two signed updates and the rollback they cause. Claude never adds the flag to a command
  it runs, no agent message is the yes, and the script refuses the flag when `/dev/tty` cannot be
  opened. Without it, the script flashes nothing and writes nothing through the API (the read-only
  steps only). With it, the original firmware's backup is required (`--backup`, checked by
  `box.sh flash`), although `box.sh flash` alone keeps it optional (D35): an unattended run flashes
  only a box whose way back is proven. `storage` is never erased; no step is repeated or looped;
  a `saves` over its budget stops the run. The factory reset, which erases the user's home, needs
  its own flag, `--factory-reset`. The update key is made at run time and removed when the run
  ends; a password the run creates is kept in `$HOME/.config/solite/box-password` (0600), outside
  the repository, and `SOLITE_PASSWORD` overrides it. The home check sets the user's real location
  and time zone from `SOLITE_HOME_LATITUDE`, `SOLITE_HOME_LONGITUDE` and `SOLITE_HOME_TIMEZONE`
  (environment only, never in the repository), so specs 026 and 027 are checked end to end.
- **Why**: the checks of a session were run one command at a time from a conversation, each with
  its own yes; one yes for one pass keeps D6's intent (nothing is flashed without the user's
  decision) and D39's (one pass, counted writes) while the session costs one command and one
  report.

## D56: The full backup is one encrypted archive at `/api/v1/system/backup` (replaces D42)

- **Date**: 2026-11-25 (spec 035, decided in an autonomous run, for the user's review)
- **Choice**: `GET /api/v1/system/backup` streams one file, `POST` of it restores it; the password
  is in the header `X-Backup-Password` (percent-encoded UTF-8, 8 to 128 characters), never in the
  address or a body. `/api/v1/backup` (D42's document of the settings and zones, at Sowel's path
  with another shape) is removed and answers 404; Sowel's own backup (a ZIP of its SQLite tables)
  is not mirrored, and the sentence of D43 that the backup document "has Sowel's paths" no longer
  holds. The archive holds `devices.json`, `zones.json`, `equipments.json`, `recipes.json` and
  `settings.json` (plain), and the Zigbee network file and `wifi.json` (encrypted); never
  `auth.json` (the account belongs to the box at hand: a restore keeps it) nor the network setting
  (the box's hardware link); the Wi-Fi settings are restored only with `?wifi=true`, else the box
  keeps its own. Format 1: the magic `SOLITEBK`, the format, the iterations, a salt, a nonce and a
  key check; the files as chunks of at most 4096 bytes; a manifest (names, sizes, SHA-256 of the
  stored bytes) and an HMAC-SHA256 over everything before it. Keys: PBKDF2-HMAC-SHA256 of the
  password, 100,000 iterations at export (a restore accepts 10,000 to 400,000), split by two
  HMACs into an encryption key and an authentication key; ChaCha20 for the secrets, encrypt then
  MAC. Both ways are streamed through one 4 KB buffer (the archive is never held whole); a restore
  is validated whole into staged copies (`<name>.restore`) and refuses a wrong password, a
  truncated, altered, oversized or newer file with the live files untouched; the staged files
  replace the live ones at the next start (`restore.pending` names them), so the Saver, which
  writes the home in memory until the restart, never overwrites them. Bounds: 2 MiB the archive, 1
  MiB a file, 7 files, 120 s an export, one job at a time; the key derivation pauses every 5,000
  iterations for the box's watchdog.
- **Why**: devices, equipments and recipes depend on the devices paired to one box and on its
  radio network's keys: D42's document left all of them out, so a replaced box or a lost
  partition meant pairing everything again; D42 itself said that a full backup was "a different
  feature" (secrets, a body beyond the 16 KB buffers, a streamed import), and this is it. The file
  leaves the box and can be attacked offline, so the key takes ten times the account's iterations
  (D47) and the keys of the radio and the Wi-Fi password are encrypted. No AEAD (an authenticated
  cipher) is among the dependencies: `chacha20` of RustCrypto is pure Rust, `no_std`, and pulls
  only two small crates (`cipher`, `inout`) beside the `sha2` and `hmac` the core already has, and
  HMAC over the ciphertext (encrypt then MAC) gives the authentication that a hand-made AEAD
  would; AES would need a hardware driver or a larger crate. A path of Solite's own under
  `/system`, like the update's (D53), ends the use of Sowel's path with a different shape (the
  inbox line on D42's alignment). A restore applied at the start, not in place, keeps the
  running engine and its Saver out of the way and lets an interruption finish at the next start. The account is kept because its hash and tokens are the box's, and a restore must not
  lock the user out with a password they forgot.

## D57: A restore's Zigbee network: the radio leaves the network it runs and forms the restored one

- **Date**: 2026-11-25 (spec 035, decided in an autonomous run, for the user's review)
- **Choice**: a restore that brings the network file writes a one-shot mark, `zigbee.restore`
  (in the data folder, on the box and on the Mac). At the network start, with the mark, a radio that runs
  another network **leaves it** (EZSP `leaveNetwork`, then the stack's network-down status) and
  forms the restored one with the file's PAN, extended PAN, channel and keys (the extended PAN id
  of the file is in display order, the reverse of the frame's); a radio already on the restored
  network resumes it. The mark is removed once the network is up and kept when the start fails.
  The network left keeps its keys in `zigbee-<PAN>.json` (`network-<PAN>.json` on the Mac), set
  aside when the restore is applied. Without the mark, a radio on a network resumes it and is
  never left, as before; a radio with no network and a parsable file forms the file's network
  instead of a random one. This is **the one exception** to the rule (the living spec's "Pairs a device") that a network is never left: the user's
  restore asks for it. Not carried: the radio's address (EUI64: Zigbee 3.0 devices keep the trust
  centre's address; on an EmberZNet radio it is a token written once and never undone, so Solite
  does not write it), its NWK frame counter (a new radio starts at 0, and a device that remembers a
  higher one may drop its commands until it passes it) and its child table (end devices whose
  parent was the coordinator rejoin by themselves or need pairing again). It is a best effort:
  the box check of the full backup counts the devices that report again without pairing, and
  the roadmap keeps the frame counter and the child table as a line.
- **Why**: the point of a full backup is that the paired devices keep talking to the restored box;
  a box forms its own network at its first start, and a factory reset keeps it, so without
  leaving it the restored keys would sit unused. Setting the old network's keys aside rather
  than discarding them keeps the rule that a network's keys are never lost. The
  EUI64 is left alone because writing it cannot be undone; the counter and the child table need
  EZSP values Solite does not read today, so the measure on the box decides what is worth adding.

## D58: The radio image supervises its engine: a last rung that restarts the box, a boot-loop guard, a safe mode, a watchdog

- **Date**: 2026-11-25 (spec 036, decided in an autonomous run, for the user's review)
- **Choice**: everything below is decided in the spec; the user reviews it.
  - **The last rung**: when the engine thread ends on an error (the radio's ladder refused a 4th
    restart, no speed answers, the start failed, a storage or data error), the box restarts, one
    second after its log line, with the reason kept across the restart. A stop asked by the
    console or an update is not an error and wins over it.
  - **The reason lives in RTC slow memory, not in the flash**: 24 words in `.rtc_noinit`, with a
    checksum; it survives a software restart, a panic and a watchdog reset, and a power-on makes
    it garbage, which is the intended reset (a power cut is itself the reason). No NVS write, so
    a restart loop wears no flash (D39's spirit). Plain loads and stores only on the RTC words
    (the atomic read-modify-write instructions work on internal SRAM only).
  - **The budgets**: 3 counted restarts in an hour or 6 in a day (the supervisor clock, which
    counts the milliseconds run since the last power-on across restarts, because the wall clock is
    not set at boot) enter the safe mode. A **brown-out is shown and not counted** (a restart does
    not mend a power supply, and the safe mode would only stop the automations), nor are the
    restarts asked on purpose and the safe mode's own retry.
  - **The safe mode** keeps the way in (Wi-Fi, API, update) with the engine stopped, and is retried
    after **1 h, then 4 h, then 24 h** (24 h for every later one) by one restart with the cause
    `safe-mode-retry`. 3 counted restarts since it began make it the **bare safe mode**: the
    home's files are not even read (a home whose load crashes cannot loop); an empty home is
    served in RAM and the files are never touched.
  - **The 503 gate**: in safe mode every request that is not a `GET` answers 503 `{"error": "Safe
    mode: the engine is stopped"}`, except the account's routes and `/system/update`: nothing is
    accepted that would never run or never be saved. A restore and a factory reset need a
    person's presence, which only a power-on gives, so they are not ways out of it. Presence
    counted from the last power-on across restarts is an inbox line.
  - **Solite's own route**: `GET /api/v1/system/supervisor` (`mode`, `bootCause`, `lastIncident`,
    `restartsLastHour`, `restartsLastDay`), a deviation from Sowel, which has no supervisor, listed in
    `docs/sowel-subset.md`. The interface shows nothing of it in this spec.
  - **The watchdog**: the engine thread is subscribed to ESP-IDF's task watchdog with a **20 s**
    timeout that restarts the chip (`CONFIG_ESP_TASK_WDT_TIMEOUT_S=20`,
    `CONFIG_ESP_TASK_WDT_PANIC=y`, in the radio image's defaults only). It is fed at every radio read
    (every 50 ms), each turn of the loop, before each file the saver writes and before each step
    of the radio's ladder. The longest stall that cannot be fed is one file's flash write, the
    3.8 s save of a 98 KB `devices.json`: 20 s is over five times it, checked by a test. The idle
    tasks stay watched with the same timeout and now restart the chip.
  - **Panics**: the policy kept is the target's: `xtensa-esp32-espidf` aborts on a panic and
    ESP-IDF's panic handler prints and restarts the chip; the next boot reads the reset reason
    `panic` and counts it. No `catch_unwind` (it cannot catch under `abort`), no profile change.
    The console's `panic` line panics on purpose, to check this on the box: it is **the one
    exception** to the rule of no `panic!` outside tests and start-up code.
- **Why**: the true north asks a month in an annex without intervention. An engine that ended and
  left the ESP32 up doing nothing, or one stuck in a loop, is a box that looks alive and does
  nothing, and nobody is there to unplug it. A restart mends most such faults; a boot-loop guard
  keeps a persistent fault from restarting the box for ever and wearing the radio and the flash,
  and the safe mode keeps the one way to replace a bad image over the network (the update), which
  a restart loop would make unreachable.
- **Alternatives**: the reason in NVS or a file (a flash write per incident, for a reason a power
  cut erases anyway); a watchdog around each radio call only (the saver's flash writes are what
  blocks); a shorter timeout (the 3.8 s save leaves little margin under 5 s); restarting without a
  budget (a loop on a persistent fault); a safe mode with no retry (a transient fault would need a
  person).

## D59: The box's backup jobs run on a thread whose stack is in internal RAM, and its files sit behind a guard (refines D56)

- **Date**: 2026-11-25 (fix of spec 035, decided in an autonomous run, for the user's review)
- **Choice**: on the radio image, `GET` and `POST /api/v1/system/backup` run their whole answer
  (the key derivation, the files read or staged, the marker) on a `backup` thread started for the
  job, its 10 KB stack in internal RAM, pinned to core 0 with the API workers; the worker that took
  the request lends it and waits. The box's `BackupFiles` reach the core only behind the core's
  `backup::Guarded`, which asks before every call, and every read or write of a file it opened,
  whether the calling stack is in internal RAM, and turns a "no" into an error, never a flash
  access. Each derivation logs `backup: key derived in <n> ms`.
- **Why**: the export of spec 035 ran on an API worker, whose stack is in PSRAM
  (`CONFIG_SPIRAM_ALLOW_STACK_EXTERNAL_MEMORY`, spec 023). Every flash access, a read too, turns
  the caches off, and ESP-IDF stops the chip when the calling stack is in PSRAM (`assert failed:
  spi_flash_disable_interrupts_caches_and_other_cpu cache_utils.c:127`): the box answered 200,
  then restarted at its first file read; a restore would have done the same at its first staged
  write. The rule spec 032 kept for writes (the OTA thread) holds for reads too. The request
  also took 70 to 88 s before the panic against 11 to 14 s expected from the login's rate (same
  algorithm, equal on the Mac, the login on the same kind of PSRAM stack): the cause of that gap
  is not known, and the logged line measures the derivation alone on the box.
- **Alternatives**: a permanent thread (10 KB of internal RAM for the whole run, for a job of a
  few seconds a month); the OTA thread (6 KB, and its one-second probation tick would wait for a
  derivation of seconds); copying the files to RAM on a flash thread before the stream (up to
  2 MiB, against the streaming of D56); the workers' stacks in internal RAM (40 KB for every
  request, against D33's budget).

## D60: The box's update installation runs on a thread of 16 KB in internal RAM, checked in QEMU (refines D54, follows D59)

- **Date**: 2026-11-25 (fix of spec 032, decided in an autonomous run, for the user's review)
- **Choice**: on the radio image, `POST /api/v1/system/update` runs its whole answer (the body
  read, `Updates::install` with the header's signature check and the image's hash, the answer) on
  an `update` thread started for the request, its 16 KB stack in internal RAM, pinned to core 0
  with the API workers; the worker that took the request lends it and waits (`job_thread.rs`,
  shared with D59's `backup` thread). The flash calls still go to the OTA thread. The thread logs
  `update: thread stack <n> of 16384 bytes never used` after every attempt. The image QEMU runs
  installs three update files (valid, one image byte changed, one signature byte changed) on the
  same thread at boot, and `scripts/qemu.sh` fails when a status differs or fewer than 2048 bytes
  of its stack stay unused.
- **Why**: the box restarted on `A stack overflow in task http-worker` at an upload. The
  installation ran on an API worker, 10 KB of stack with the request's own frames. Without
  `precomputed-tables` (the core takes `ed25519-dalek` without default features), the Ed25519
  check builds two lookup tables of 1280 bytes and two 256-byte digit arrays on the stack: on the
  ESP32 (measured in QEMU, release build) the check alone uses 9.2 KB of a stack and the whole
  installation 9.8 KB, against 0.76 KB for a bare thread; on a thread of 10 KB the installation
  uses all of it and the chip stops. Every upload with a well-formed header ran the check, so
  every upload overflowed, a valid one too. 16 KB leaves about 6 KB for the request's frames (the
  body reader and the answer never nest under the check) and for the compiler's drift; the QEMU
  check catches a drift below 2 KB on every validate run. Internal RAM, as for D59: the job is
  short, and a stack in PSRAM is slower for the hash of a 2 MB image.
- **Alternatives**: larger worker stacks in PSRAM (4 × 6 KB more for every request, for a job of a
  few seconds a release; a worker's own frames vary by route); the check on the OTA thread (6 KB,
  its probation tick would wait, and it would need 16 KB for the whole run); `precomputed-tables`
  (saves one table, about 1.3 KB of stack, and adds a static table to the image: not enough
  alone); a Mac test of the stack (a Mac thread has at least 16 KB and other frame sizes: it
  proves nothing about the ESP32).

## D61: No equipment is created from a device (replaces spec 019's "Create equipment" on a device's row)

- **Date**: 2026-11-25 (spec 037, decided in an autonomous run on the author's note from the box)
- **Choice**: the Devices page's rows, which Discovery also shows, offer no "Create equipment": a
  device with no equipment shows "—" in the Equipment column and delete is its only action, as
  Sowel's `DeviceList` (`EquipmentCell`). An equipment is created from the Equipments page or a
  room's "Add equipment", whose form picks the devices. The form's props that only the device's
  button used (type, name and devices given, the type guessed from the devices read) go with it.
  D30's "the equipment is the entry point" now holds without exception. Discovery offers no next
  step after pairing (an inbox line asks for a hint toward Equipments).
- **Why**: the author, on the box (2026-11-25): in Sowel a device is never the entry point; two
  paths to the same form made the interface differ from Sowel's for no gain the true north needs.
- **Alternatives**: keep the button (refused by the author); a link from the row to the
  Equipments page (not Sowel's; left to the inbox line).

## D62: Sowel's top bar, with Solite's switch point and a location link

- **Date**: 2026-11-25 (spec 037, decided in an autonomous run)
- **Choice**: Sowel's top bar is ported (`TopBarFrame`, the header of `AppLayout` with its
  breadcrumb and phone title, `SunTimes`, `HomeTimeNote`, `ConnectionStatus`, `useTopBarState`,
  `useTimezone`, Sowel commit `2d360726`), fed by `GET /settings` (`home.name`, the location),
  `GET /system/timezone` and the root zone's sun times. Differences from Sowel: the switch point is
  `md` (Solite's sidebar, design-system.md), not `lg`; the link indicator replaces Solite's
  "Connection lost" strip (`Banner`) with the same 3 s rule, and `useLive` now says `connecting`
  at each attempt; where Sowel shows nothing without sun times, Solite shows a "Set the location"
  link to Settings when no location is set (nothing when only the clock is missing); no alarms,
  updates or restart pills, no user block (Solite has no data behind them; Log out stays under
  Administration). The home's time is Sowel's `HomeTimeNote`: shown only when the browser sits in
  another time zone than the home's; the `CurrentTimePill` named in the inbox no longer exists in
  Sowel.
- **Why**: the author noted the missing bar on the box; a box ships without a location, which
  "Inactive during day" needs, so the bar says where to set it; Sowel's quiet clock rule avoids a
  second clock beside the device's own.
- **Alternatives**: an always-shown clock (Sowel's older `CurrentTimePill`, which Sowel replaced);
  keeping the Banner beside the indicator (two signals for one fact); Sowel's `lg` switch point
  (would change Solite's sidebar too).
## D63: A release is made by one script and published by a tag-triggered workflow (spec 038)

- **Date**: 2026-11-25 (decided in an autonomous run, for the user's review)
- **Choice**: `scripts/release.sh` turns a version, a signing key and the public key into the files
  of a release (the signed `.sup`, a full-flash image, `SHA256SUMS`, `manifest.json`), refusing a
  release that is not signed, not signed by the key matching the public key, or whose version is
  not the tree's (the three version files). `.github/workflows/release.yml` runs it on a pushed tag
  `vX.Y.Z` and publishes the GitHub release; run by hand it is a dry run with a throwaway key that
  publishes nothing. Details:
  - the public key is a committed file, `release/update-key.pub` (not a repository variable);
  - the notes are the section of `docs/release-notes.md` for the version (also the annotated tag's
    message), not read from the tag;
  - one full-flash image (`esptool merge_bin`, written at `0x0`) rather than three files with
    offsets;
  - the release is built in `firmware/target/release-radio`, so its key never lands in
    `target/radio`, which `box.sh flash` takes;
  - the private key is an environment secret (`release`, deployment rule: tags `v*`), read by the
    `sign` job only; that job builds only the host program, whose Rust dependencies' `build.rs`
    are far fewer than the radio image's npm, ESP-IDF, cmake and pip, and they run in an earlier
    step than the one that holds the secret (on Linux a build tool of the same user could read a
    running step's environment); the radio image is built by the `firmware` job, which has no secret, and handed
    over as an artifact that `sign` accepts only if it carries the committed public key; the job
    that writes the release has no secret and the only `contents: write`; the caches are restored,
    never saved, by a release run (no `setup-node` `cache: npm`, which saves on a miss, and no npm
    cache at all);
    one concurrency group per kind of run (tag, manual), so a dry run never cancels a tag run;
  - the tag must be on `main` (checked by the `firmware` job before its build, and again by `sign`), put on the merge commit of the release's
    pull request;
  - `SHA256SUMS` is not signed (the `.sup` is, and the box checks it); signing it is in the inbox;
  - `solite-host --update-verify` proves the script's output with the box's own checks.
- **Why**: a release that exists, is signed and is reproducible from a tag is what the true north
  asks ("signed binaries", users flashing from a public release). Each rule closes a way to publish
  a wrong or unsigned file: a key that does not match the image's public key would give a release no
  box accepts.
- **Alternatives**: the public key as a repository variable (not reproducible from the tag, not
  nameable in the docs); the notes from the tag message (not reviewed in the release commit); three
  files with offsets (mistypeable); a repository secret instead of an environment secret (a branch's
  workflow could read it).

## D64: Key custody, and what a rotation means (spec 038)

- **Date**: 2026-11-25 (decided in an autonomous run, for the user's review)
- **Choice**: the key is made outside the repository (`--update-keygen`), its public half committed,
  its private half kept in a password manager and an offline copy and given to the CI as the
  environment secret (`docs/release.md`). The update file's format 1 has one key per build and no
  field for a next key: its reserved bytes (18-19 and 56-63) are checked to be zero. A rotation is
  therefore a **bridging release** signed with the old key and carrying the new public key; from it
  on, releases are signed with the new key. A leaked key means a new key and the same bridge, or a
  USB flash. The format and the order of the bridge are designed in step 3 of the roadmap's J5b.
- **Why**: the public key is compiled into the image (D52), so only a release signed by the old key
  can bring the new one to a running box; writing that now would fix a format nothing reads yet.
- **Alternatives**: several public keys per build and a revocation list (a larger image and a
  format change, for a need not yet met); a rotation field in format 1 (the reserved bytes are
  refused today, so a box that has not the new reader would reject the file).

## D65: The update route has two forms, and the box checks and pulls a release itself (spec 039, refines D53, D58)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: `POST /api/v1/system/update` takes the signed file as its body (spec 032's upload,
  `application/octet-stream`) **or** an empty body, which pulls the latest release: the manifest
  (cached ten minutes), the `.sup` it names, installed through the same pipeline. `POST
  /api/v1/system/version/check` (Sowel's route) checks now; `GET /api/v1/system/version` answers
  the last check. Differences of value from Sowel: the check's interval is 60 s (Sowel 10 s) and no
  `Retry-After` header is sent; the pull answers when it is installed (Sowel answers at once and
  reports progress by events); `updateAvailable` is false when the release's `minVersion` is above
  the running version. The safe mode's exceptions (D58) become the account's routes,
  `/system/update` and `/system/version/check`. The core decides the form from a new field of its
  request, `content_length` (the declared length, `Some(0)` for a request with neither
  `Content-Length` nor `Transfer-Encoding`, `None` for a chunked body): `Some(0)` is the pull. The
  box's server gives that value on the update route only (a missing length there is the pull; a
  chunked update stays 411; the backup restore keeps its 411), and the two tests that sent an
  empty upload to the core moved to a declared length.
- **Why**: one route for the interface and for Sowel's clients; a box in safe mode must still be
  able to find the release that fixes it.
- **Alternatives**: a new route for the pull (Sowel's clients call `/system/update` with no body);
  events for the progress (Solite has none yet).

## D66: Where the box may fetch from (spec 039)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: `https://` to any host; `http://` only to an IPv4 literal of `10/8`, `172.16/12`,
  `192.168/16` or `127/8`, or to `localhost`. Redirects are followed by the core (five at most,
  never from `https` to `http`), not by the platform's client (the box's `EspHttpConnection` is
  set to follow none): one rule, tested on the Mac, for both adapters. The file is signed and its
  hash checked, so its integrity does not rest on TLS. The box verifies the server with ESP-IDF's
  certificate bundle, set to its common list (the authorities of most of the web, GitHub's
  included) rather than the full one, which keeps the image smaller.
- **Why**: TLS keeps a public network from seeing or blocking which version a box takes; a name
  could resolve to the Internet, hence the literal-only rule for plain http, which a test server on
  the local network uses.
- **Alternatives**: any scheme to any host (an `http://` name on the Internet); only `https` (no
  test server on the local network).

## D67: The automatic check is off by default, and its two settings are Solite's own (spec 039)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: the settings `update.manifestUrl` (default the GitHub address of the latest release's
  manifest) and `update.autoCheck` (`"true"` turns it on; off by default) join the settings of spec
  026. The automatic check runs 10 minutes after the start, then every 24 hours, only reports and
  never installs.
- **Why**: a box contacts the Internet only when its user asked, until the TLS session's memory has
  been measured on the box.
- **Alternatives**: on by default (the box talks to GitHub unasked); no automatic check (a user who
  wants it has no way).

## D68: The check and the pull run on a thread of 20 KB in internal RAM (spec 039, follows D59, D60)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: on the box, a check and a pull run on a thread started for the job (`update-check`,
  `update-pull`), 20 KB of internal RAM each, one at a time. ESP-IDF's `esp-tls` runs mbedTLS in the
  calling task and its HTTPS examples give such a task 8 KB; the installation's Ed25519 check takes
  9.2 KB (D60) and runs after the handshake returned. The HTTP client's buffers are 4096 bytes to
  receive (GitHub's redirect `Location`, a signed address of several hundred characters, is over
  the default 512) and 1024 to send, both in internal RAM. The API worker that took the request
  lends it and waits, as for D59 and D60; the automatic check's thread (started once a minute at
  most, from the console watch thread) is not waited for.
- **Measured** (2026-11-26): QEMU's pull of a 64 KB file through one redirect, from an in-memory
  fetcher on this thread, leaves 9412 of 20480 bytes unused; `scripts/qemu.sh` fails below 8192,
  the room left for the handshake, which QEMU cannot run. The radio image's application grows from
  2,193,632 to 2,328,192 bytes (+134,560, 52 % to 55 % of the slot): the TLS client, the
  certificate bundle and the release code, the split not measured. The handshake's share of the
  stack and the internal RAM minimum during a check are read on the box (the roadmap's list).
- **Why**: the API's workers and the engine keep their stacks; a deep handshake never runs on a
  stack that was not sized for it.
- **Alternatives**: the API worker's own stack (not sized for TLS); a larger stack (internal RAM is
  the tight resource).

## D69: The host fetches `https` through the system's `curl`, no TLS library (spec 039)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: the host's fetch adapter does `http://` with the standard library and `https://`
  through the `curl` program (`-q`, `-g` so `[ ] { }` in a URL are never expanded, HTTP/1.1, no redirect followed, the same timeouts; a `chunked` header in its output is ignored, the body being decoded); without it
  the error reads `https needs the curl program`. No TLS library joins the workspace.
- **Why**: the host is a development and demo tool; a TLS library adds a large dependency tree to
  audit for a path the box does not use (it has ESP-IDF's mbedTLS).
- **Alternatives**: `rustls` or `native-tls` in the host (a dependency tree and a build time for
  a development tool).

## D70: A key rotation is a bridging release declared by a committed file (spec 040, refines D64)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: the bridge is declared by `release/update-key.bridge` (`version` and `previous`, the old
  public key) committed in the release's pull request beside the new `release/update-key.pub`. The
  image carries the new public key; the `.sup` is signed with the old private key; the new private
  key is given too (`--next-key-file` or `--next-key-env`) and must match `release/update-key.pub`,
  so that boxes never move to a key nobody holds. The script checks, after signing, that the old key
  accepts the file and the new key refuses it. The manifest has no field for a bridge: a box accepts
  the file by its compiled (old) key and trusts the key its new image carries once confirmed. The CI
  holds two secrets for the time of the bridge, `SOLITE_UPDATE_KEY_PREVIOUS` (old) and
  `SOLITE_UPDATE_KEY` (new), read by the `sign` job only. The procedure and the leaked-key case are
  in `docs/release.md`; the Mac's chain test (`crates/host/tests/rotation.rs`, two host builds) checks
  the logic, the on-box run (two test-key images, a USB flash) the compiled key.
- **Why**: a committed file makes the bridge reviewable in the pull request and refusable by its
  version when it is left behind; a flag or a secret alone would leave no trace in the tree. The box
  needs nothing new, so no format changes and no old box is left behind by one.
- **Alternatives**: a next-key field in the manifest or the update file (a format change that a box
  without the new reader rejects, D64); several public keys per build and a revocation list (a
  larger image for a need a bridge meets); the bridge decided by the tag name (nothing to review).

## D71: The update card links to the release page, and the settings are always sent whole (spec 040)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: the Settings page's update card shows the check, the latest version, "Update to vX"
  with a confirmation, and a link to `releaseUrl` for the release notes: the API does not expose the
  notes (the cache keeps none, a manifest's notes could take 16 KB) and `VersionInfo` is unchanged.
  After the click the card polls `GET /api/v1/system/version` every 3 s, never two at once, each
  with a 5 s timeout; it ends at 3 minutes (the pull answered 200) or 11 minutes (the connection
  dropped during the request: the 600 s download and a restart), from the **last** poll: the same
  version means the update was not kept, no answer means "reload the page". Every save of the page
  (the Home card, the automatic check, the manifest's address) reads `GET /api/v1/settings` and
  sends the whole map with its own keys changed.
- **Why**: the pull's request waits for the whole install, unlike Sowel's overlay, so the card
  counts seconds rather than a countdown and ends on what the last answer said. The whole-map save
  does not rely on the server's merge rule: the first run of spec 039 on the box saw keys vanish
  after a PUT of one key (not reproduced on the Mac, `settings_merge.rs`; the storage-loss line of the
  roadmap is the likelier cause), and Sowel's own Settings page sends its whole form. It costs one
  GET per save, and a value changed in another tab in the same second can be written back.
- **Alternatives**: the notes in `VersionInfo` or in the card (a field Sowel does not have, memory
  held for text); a full-screen overlay as Sowel's (the sessions end at the restart, the card keeps
  the context); a PUT of the changed keys only (relies on the merge).

## D72: A cut download resumes with a Range request, at most three times (spec 040, refines D65)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: during a pull, a read that fails or a body that ends before `image.size` bytes is
  resumed: the cut body is dropped (one TLS session at a time), the core waits 2, 4 then 8 s
  (`RESUME_WAIT_MS` doubled, through `Pacing::wait_ms`), fetches `image.url` again from the start of
  its redirects (a GitHub asset's signed address expires within minutes) with `Range: bytes=<count>-`,
  and continues the same reader (the same hash, count and slot) when the 206's `Content-Range` is
  exactly the rest, its `Content-Length` too and its `ETag` (when both answers have one, at most 256
  characters) the first one's. At most 3 attempts within the 600 s; a resumed request that fails
  (adapter error) or answers a 5xx counts as one attempt. A 200, a 416, a changed range, size or
  `ETag` fails cleanly (`the server does not resume (200 to a range request)`, `the file changed on
  the server`, the cached manifest dropped for the last two), as any other status. A read error
  once the count already equals `image.size` is the end of the body, not a cut. The QEMU rule for
  the pull thread becomes `resumes 1` and 1024 unused bytes (it was 8192, D68), the resumed request
  touching an 8 KB array for the handshake; 8336 unused measured.
- **Why**: a Wi-Fi blip or an access point's roam lasts seconds and the pull of 2 MB takes about
  35 s, which should not be lost; three tries over 14 s cover that, and a longer outage is better
  met by a later pull than by a request holding the update thread and the TLS memory. Nothing is
  kept on flash across pulls, and the final count, hash and signature checks decide as before. The
  pull does not restart from byte 0 inside the request: that would erase and rewrite the 4 MB slot a
  second time on a server that just proved unreliable; a clean failure is shorter to reason about
  and the user's next click starts over.
- **Alternatives**: no resume (spec 039); a resume kept across pulls in flash (flash wear and a
  state to invalidate); an unbounded retry (holds the thread and the memory); a restart from byte 0
  in the request (above).

## D73: Recipes outside the firmware are JavaScript scripts run by QuickJS (spec 041, replaces D1's `wasmi`)

- **Date**: 2026-11-26 (decided with the user)
- **Choice**: a recipe that is not built in is one JavaScript text in Sowel's script-recipe format
  (Sowel's `roadmap/ai-script-recipes/spec.md`: `recipe({ api: 1, slots, start({ on, get, order,
  after, now, state, log }) })`), run by QuickJS-ng through the crate `rquickjs` (0.14.0, its C
  compiled by the build), behind the core's feature `script-recipes`. Solite adds `id`, `version`,
  `onMotionReport`, `cancel`, `console.log` and an optional `validate` to the draft; the differences
  are in `docs/sowel-subset.md` and proposed to Sowel. D1's "recipes run in `wasmi`" no longer holds.
- **Why**: measured (spec 041's `research.md`), a WebAssembly recipe costs about 1.1 MB in `wasmi`
  as built and 376 KB after a week of shrinking (its interpreter triples the code), 150 to 215 KB
  in WAMR (a C runtime of its own); a QuickJS recipe costs about 60 KB of PSRAM and 0.5 KB of internal
  RAM on the 32-bit target, its limits (memory, time, stack) work there, and Sowel runs its own
  script recipes in QuickJS, so one recipe can run in both (the true north's "extension of Sowel").
  JavaScript is the author's language and the one AI writes best.
- **Alternatives**: `wasmi` (too much memory); WAMR (half the memory of `wasmi`, still three times
  QuickJS's per recipe, a C runtime, modules to shrink by hand); Berry (10 KB per recipe, but a niche
  language Sowel does not run); Rhai (pure Rust, about 120 KB per recipe); a declarative JSON format
  (cannot hold motion-light's state machine; rejected by Sowel's own draft); native recipes delivered
  by firmware updates only (no recipe without a release).

## D74: One QuickJS runtime per script recipe, its bounds and where it lives on the box (spec 041; the `script` thread's internal stack: replaced by D83; 16 KB of source, 2 recipes and 8 instances: replaced by D85)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: one runtime and one context per script recipe, shared by its instances (Sowel's draft:
  one per instance). Bounds: 2 script recipes (at most 8 instances of each, a ninth is refused with 400), 16 KB of
  source, a memory limit of 224 KB per runtime on the box (448 KB on the Mac, whose 64-bit pointers
  make everything larger; QuickJS's own limit, counting with the allocator's usable size; 448 KB for
  the two, under a 512 KB ceiling), 1000 ms to load a script, 200 ms per call (the interrupt handler
  on a monotonic clock the engine is given; 100 ms at first, raised on the box's measure, 24 ms on average and
  83 ms at the most for `motion-light-script` at 160 MHz, with the user's agreement), 300 ms of script calls per turn of the engine's loop, 16 KB of
  JavaScript stack; QuickJS's small-object arena is off (`rquickjs-sys` vendored under
  `vendor/rquickjs-sys` with a two-line `#ifndef` patch: about 85 KB per runtime saved); the garbage
  collector's threshold kept under ¾ of the limit, and a collection after a call that leaves more than
  85 % of the limit held or 32 KB more than the last collection left (what the runtime holds is the
  box's allocator's own count, read after each call; on the Mac, where the system's allocator
  counts nothing, QuickJS's accounting, which walks the heap); QuickJS is built with `JS_NO_FUNCTION_SOURCE`, a patch of the same vendored
  crate: a script's functions do not keep their source text (`Function.prototype.toString` answers
  `function f() { [native code] }`), which saves the source of every function of every runtime and
  of the shim. On the box the firmware's allocator puts the
  runtime in PSRAM (`heap_caps_*`, its `unsafe` under D11), holds each runtime to its limit itself and
  tells a refusal in a flag of that runtime (QuickJS has no limit of its own on the box); every call
  into a script runs on a stack in internal RAM: the engine thread's, and a `script` thread for the
  API's non-`GET` requests (but the account's `/api/v1/auth/*`, which reach no script and stay on the worker, so that a login's password hash holds no other write), made once at boot (D59, D60's lesson; a thread per request needed a
  free 32 KB block each time, which fragmentation could refuse for every write). A call over its time, an allocation refused (caught or not), a stack overflow
  or a forged handle fails the whole recipe: its instances stop, and it restarts after 1 minute, at
  most 3 times in 24 hours, then on a person's action. The core reaches the call's home from QuickJS's
  callbacks through the crate `scoped-tls-hkt` (no `unsafe` in the core: D11 kept).
- **Why**: a runtime per instance costs about 60 KB each where a shared one costs about 5 KB per
  extra instance; the price is that one instance's runaway stops the recipe's other instances, which
  the restart brings back. PSRAM keeps the internal RAM (the tight resource) free; the measures of
  spec 041 set the figures: measured in QEMU, `motion-light-script` loaded holds 107 KB and each
  instance 8-10 KB, so 8 instances need about 190 KB; the first limit, 128 KB, ran out of memory at
  load, and the load takes 212 ms in QEMU (a clock slowed 50 times) whence its own limit. Limits are
  not raised again without the user.
- **Alternatives**: a runtime per instance (memory); the default allocator on the box (every QuickJS
  block is under 4 KB and lands in internal RAM: three runtimes exhaust it); a counting allocator in
  the core (needs `unsafe`, against D11).

## D75: A script reaches only the equipments of its own settings (spec 041)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: `on`, `get` and `order` take a slot name, never an id; the engine maps the name to the
  ids of that instance's validated settings; the engine's functions are captured by the shim and not
  reachable from the script's globals; a handle that is not one of the recipe's instances fails the
  recipe; the ids a script's `validate` may read the facts of are the ones the params under check
  name, kept in Rust (any other text a `validate` looks up, a zone id, a time or an option, gives nothing and fails nothing); the end of each call (state, listen set, timers, log lines) is checked in Rust
  whatever the script did to the objects the shim builds it from. Instances of one script recipe
  share its runtime and its handles, so they can reach each other: the isolation is between recipes
  and from the equipments outside an instance's settings. The settings are checked by the engine
  from the slots before the script runs, with Solite's own texts (Sowel has no generic check).
- **Why**: Sowel's draft rule ("a script cannot reach an equipment the user did not put in a slot"),
  enforced by the host, not by trust; it holds before scripts are downloaded (F2), so a script from
  outside meets it from the first day.
- **Alternatives**: ids in the API (no isolation); isolation left to F2 (a script written now would
  break then).

## D76: Where scripts come from before they are downloaded (spec 041)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: on the Mac, `--recipe-script <file.js>` (a developer's flag, unsigned); on the box,
  the scripts of `recipes/scripts/` built into the radio image, so every release carries QuickJS
  (about 752 KB of flash, measured) and `motion-light-script`. Downloading, signing (with a key of its own,
  not the firmware's) and removing a script are F2.
- **Why**: the box test needs the scripts on the box, and the radio image is the one the box and the
  releases run; a second image only for the test would be a variant to keep. The flash fits (the
  4 MB slot stays under the 75 % warning: the radio image takes 69 % of it).
- **Alternatives**: scripts out of releases until F2 (a box image of its own for the test); a route
  to upload a script now (it is F2's, with its signature).

## D77: The box's CPU at 240 MHz, its flash and PSRAM at 80 MHz (spec 041)

- **Date**: 2026-11-26 (decided in an autonomous run, the user away, with their leave to flash)
- **Choice**: every firmware image sets the CPU to 240 MHz (ESP-IDF's default 160), the flash to
  80 MHz and the PSRAM to 80 MHz (40 each by default), in `firmware/sdkconfig.defaults`. The flash
  stays in DIO mode. The chip is an ESP32-D0WDR2-V3, whose PSRAM ESP-IDF drives at 80 MHz with the
  flash at 80 MHz.
- **Why**: QuickJS interprets the script recipes from the flash, through the cache, with its heap in
  PSRAM: on the box, a call of `motion-light-script` took 24 ms on average and 83 ms at the most at
  160 MHz and 40 MHz; 13.9 ms and 36 ms after the change (with the RAM fixes of spec 041). The
  application's PSRAM start-up sets the flash's clock too (ESP-IDF's `psram_gpio_config`, its mode with
  the flash and the PSRAM at 80 MHz), so an image installed over the air runs both at 80 MHz whatever
  the bootloader's setting. The risk: `CONFIG_SPIRAM_IGNORE_NOTFOUND` lets a box whose PSRAM fails
  at 80 MHz boot without it, and the update's probation does not look at the PSRAM (an inbox item
  of the roadmap); the box's boot log of 2026-11-26 shows the PSRAM found at 80 MHz, its memory test
  passed, and the CPU at 240 MHz.
- **Alternatives**: QIO flash (faster still, but it needs the flash's write-protect and hold pins
  wired as data lines, not checked on the Dongle Max); QuickJS compiled at `-O2` (a larger image,
  more cache misses; not measured); keeping 160 MHz and a slower limit per call.

## D78: Script recipes installed from a catalogue or a signed file, checked at every start (spec 042; the per-job `recipe` thread on the box: replaced by D83; the 16 KB file: replaced by D85)

- **Date**: 2026-11-27
- **Choice**: refines D76. A script recipe is installed from a catalogue (`recipes.json`, at
  `mchacher/solite-recipes`' latest release by default, the setting `recipes.catalogue`) or from an
  upload, as a file of at most 16 KB signed with a recipe key of its own (magic `SOLITERS`, never the
  firmware's key: a leak of one does not give the other), kept as `scripts/<id>.srs`, checked at
  install and again at every start, admitted by the PSRAM free now (a runtime's 224 KB limit plus a
  512 KB reserve) and by the internal RAM's largest block (a job thread and its TLS session; the uninstall, since 2026-11-27, needs only the thread's 20 KB). One
  job at a time, with the release check and pull (one TLS session). No flash access from a task whose
  stack is in PSRAM: the download, the write and the removal run on a `recipe` thread in internal
  RAM, the load and unload on the `script` thread. No script is built into the radio image. Not in
  the backup (D54's old releases refuse an unknown file). The uninstall is allowed in safe mode (the
  way out of a script that breaks the engine, as D79's restart). It deletes the recipe's instances
  (settings and logs) with the file, as a delete does: kept and stopped, an instance stayed in its
  zone as a ghost (the second box demo, 2026-11-27); the page's confirmation names their count. The routes are Solite's own, under
  `/recipes/*`, and the ids `store` and `install` are reserved so that they never meet Sowel's
  `/recipes/:recipeId`. The factory reset removes the installed recipes.
- **Why**: a new recipe needed a new firmware (D76); the user ranked its removal first. Sowel's
  plugin store serves TypeScript packages, not scripts, so its routes are not reused. A key of its
  own lets a catalogue's CI sign recipes without holding the firmware's key.
- **Alternatives**: scripts in the firmware's release (a release per recipe); one key for both
  (a leaked recipe key would sign firmware); the signed file streamed (a new streamed route, which
  the API's workers cannot take: their stacks are in PSRAM); the memory checked by the lowest free
  since boot (it never comes back up after a passing peak); the scripts in the backup (D54).
- **Deviations from Sowel**: the routes and the setting above (`docs/sowel-subset.md`); `GET
  /api/v1/system/resources` (the Resources card).

## D79: A restart route, allowed in safe mode, refused during a probation; the probation checks the PSRAM (spec 043)

- **Date**: 2026-11-26 (decided in an autonomous run, for the user's review)
- **Choice**: `POST /api/v1/system/restart`, the administrator's, Sowel's route name with Solite's values
  (Sowel restarts its container): Sowel's answer, 200 `{"success": true, "message": "Restarting"}`, then the update's restart path (`restart_after_update`, the boot's cause `asked`). It is
  one more exception to the safe mode's 503, beside D58's and D65's (`auth/*`, `system/update`,
  `system/version/check`): restarting is the way out, and the cause `asked` clears the count. It is
  refused with 409 during an update, a check or a backup (their texts) and during a probation (a reset
  would make the bootloader roll the update back); allowed when an update or a restore already waits
  for its restart. The probation's health becomes `{ engine, api, psram }`: an image without its PSRAM
  is rolled back at 300 s; the OTA thread starts without its chunk buffer so that the probation ticks.
- **Why**: the person in front of the interface had no way to restart the box; since D77 (PSRAM at
  80 MHz, `CONFIG_SPIRAM_IGNORE_NOTFOUND`) a box could boot without its PSRAM, be confirmed, and
  never run the memory budget.
- **Alternatives**: a restart during a probation that confirms it first (hides a bad image); the
  restart refused in safe mode (the person would need a USB flash or a power-on to leave it); the
  PSRAM read once at boot instead of in the probation (a confirmed image is not the probation's case).

## D80: Tighter bounds: 8 data a device, 8 + 8 bindings an equipment, 20 log lines an instance (spec 044)

- **Date**: 2026-11-27
- **Choice**: `devices::MAX_DATA` 16 to 8, `equipments::MAX_BINDINGS` 16 to 8 (8 data and 8 order
  bindings; the interface's own bound too), `recipes::MAX_LOG_ENTRIES` 50 to 20. Unchanged: 4 orders
  a device, 2 KB of params an instance, 200 bytes a log message, the counts of devices, equipments,
  zones and instances. A home stored with the old bounds is cut at load and the cut is logged once
  per device or equipment: a device keeps its first 8 data; an equipment first loses its dangling
  bindings (data or order gone, alias taken), then keeps its first 8 valid ones of each kind, and is
  written once. The "more than 8 data, <key> not added" line is logged once per device and key, for the last 16 such pairs remembered (a pair forgotten past them, or a device deleted, is said again). A
  value a Sowel equipment holds beyond these bounds cannot be held by the box as is (a difference of
  value, beside D13, D14).
- **Why**: the worst case (every bound at its maximum) measured 1,042,032 bytes against the 1 MiB
  ceiling, 6.5 KB of room, and went over once on a loaded Mac; the coming features (I1, the modes,
  the history) add to it. A real device has 3 to 8 data and a light 1 to 4 bindings of each kind;
  the last 20 lines say what a recipe did. The worst case falls to 676 KB, measured on the Mac (693,067 bytes, every log entry at 200 bytes). The richest device
  of today's catalog, an IAS motion multi-sensor, has exactly 8 data (occupancy, temperature,
  humidity, illuminance, battery, voltage, tamper, battery low): its margin is zero, accepted.
- **Alternatives**: params 2 KB to 1 KB (about 16 KB gained, but `motion-light` would have been cut
  to 8 lights and 8 buttons and the script recipes' list slots to 8: refused, params stay at 2 KB);
  raising the 1 MiB ceiling (the PSRAM is shared with the radio's and the script recipes' shares);
  a bound of 12 (about half the gain).

## D81: No recipe is built in: `motion-light` is a script recipe, Sowel's 1.1.2 (replaces D17's native recipes and D46's pruned button, spec 046; the 8 instances: replaced by D85)

- **Date**: 2026-11-27
- **Choice**: the native (Rust) `motion-light` recipe and its crate are removed; the engine
  provides no recipe, every recipe is a script loaded at start (installed from the catalogue or a
  signed file, or given to the Mac's host by `--recipe-script`). The recipe SDK stays (the
  engine's contract). `motion-light` is the script `recipes/scripts/motion-light.js`, version
  `1.1.2`: Sowel's id, slots, info (French included, served byte for byte as the native served it,
  a whole `min` written `0`) and validation texts, the script's own `validate` run before the
  host's check of the slots (a `validate` that throws or runs out of time gives way to the host's
  text; it reads, of what the params name, an equipment's `name`, `type`, `zoneId` and binding
  aliases and whether a zone exists, so that `Zone not found`, `... is type "<type>" but this
  recipe requires light_onoff` and `... does not belong to the selected zone` are Sowel's, in
  Sowel's order). The principle: **the recipes are Sowel's, only adapted for performance to Solite's
  context**. Five behaviours are the engine's own and stay as the script does them (they replace
  D17's "the native settle" and D46's pruned button): a deleted equipment leaves the settings by
  the generic settle; a button that loses its `action` binding disables the instance (D46 kept
  running and dropped the button); the legacy `light` key is not read; a refused order's reason is
  logged; the 2 s echo window is counted on the wall clock. At most 8 instances of `motion-light`
  (D74's bound of one script recipe; D17's 16); `MAX_INSTANCES` (16) still bounds all recipes.
  A home saved with native instances finds its recipe again once the script is installed: until
  then they stay stopped with `Failed to restore: Recipe not found: motion-light`, settings kept.
  What the memory ceilings measure (effect on D20 and D27): the Mac's tests (`memory_ceiling.rs`,
  300 KB, `memory_worst_case.rs`, 1 MiB) run script instances and count the Rust side (QuickJS
  allocates through the C library and is not seen): the realistic home peaks at 238 KB, 243 KB
  with every log entry at 200 bytes (163 and 173 KB with the native recipe), the worst case at 880
  KB with every log entry at 200 bytes (804 KB), 16 instances over two script recipes; the
  runtimes are a line of their own in `docs/memory-budget.md` (224 KB each on the box). The QEMU
  gate of D27 (350 KB, the `engine:` line) still runs before any script is loaded, so it measures
  the home with its three `motion-light` instances kept stopped (peak 274 KB, 312 KB with the
  native recipe running); the script phase then loads
  `motion-light`, starts the three kept instances, plays the scenario and measures them running
  (`recipes: script motion-light ...`, `script sizes: ...`). A second gate holds that running
  case: the whole heap (internal RAM and PSRAM, from before the engine started) with the script
  loaded and the three instances running is at most 574 KB, D27's 350 KB plus the 224 KB ceiling of
  one script runtime (`heap peak` of the `recipes: script` line, checked in `fw_check_script_phase`;
  measured 535 KB, 565 KB with the depth script's runtime added); D27 itself is unchanged. The radio image is 34 KB smaller (2932 KB, 71 % of its slot).
- **Why**: since F2 (spec 042) the radio image carried no script but still carried the native
  recipe's code, and two implementations of one behaviour were tested against each other. One
  recipe, one place it comes from, and a box that gets its recipes from the catalogue; the native
  code and its 16-instance bound would have kept a second set of rules alive. The script's
  differences are what Solite's engine needs, none a rule of the recipe.
- **Alternatives**: an id alias `motion-light-script` to `motion-light` at load (no box but the
  author's runs a script instance, F2 not being released); keeping the native recipe as the
  fallback (the two would drift); the box installing on its own the recipes it proposes (a roadmap
  line, not part of this change).

## D82: The operator's secrets have an encrypted copy in a private repository (completes D64)

- **Date**: 2026-11-27
- **Choice**: the two private keys (`~/keys/solite.update-key`, `~/keys/solite.recipe-key`), the
  box's password and its Wi-Fi settings (`~/.config/solite/box-password`, `wifi.env`) are kept as one
  archive encrypted with `gpg` (symmetric, AES-256) in the private repository `mchacher/solite-ops`.
  Its passphrase lives only in the operator's password manager; its README says how to restore the
  files (mode 600) on a new computer and how to update the archive. This repository holds none of
  them, encrypted or not; the release CI does not read `solite-ops` (its secrets are typed into
  GitHub, D64). The archive is D64's offline copy.
- **Why**: a second computer had to sign recipes and reach the box; the password manager alone
  does not restore files with their paths and modes, and a copy that needs both the repository and
  the passphrase survives the loss of one computer without exposing a key.
- **Alternatives**: an encrypted volume on a disk (D64's first wording: one copy, in one place);
  the keys in the password manager only (typed back by hand, mode and path by hand); a secret store
  of a cloud service (one more account holding the keys).

## D83: The `script` thread's stack in PSRAM, the `recipe` thread made once (replaces D74's internal stack for the script thread and D78's per-job recipe thread on the box, spec 045)

- **Date**: 2026-11-27
- **Choice**: the permanent `script` thread's 32 KB stack is in PSRAM (`MALLOC_CAP_SPIRAM`, as the
  API workers', spec 023), and the `recipe` thread (20 KB, internal RAM) is made once at start and
  runs the recipe jobs and the catalogue refreshes one at a time, instead of one thread per job.
  The core's admission (`RecipeInstalls::job_thread_kept`) then counts only the TLS session's 16 KB
  for an install, an update or a refresh, and nothing for an uninstall; the Mac and the QEMU run,
  which make a thread per job, keep the 36 KB and the 20 KB. A thread that cannot be made at start
  is logged and a recipe job then runs on a thread made for it (internal RAM), the admission counting its 20 KB as before. A PSRAM stack is allowed because nothing the
  `script` thread runs touches the flash (D59: a flash access, a read too, turns the caches and so
  the PSRAM off): the core's router for every non-`GET` request but `/auth/*` reaches no `Storage`
  (the saves are written by the engine's saver), and the recipe jobs' load, swap and unload steps
  are memory only. As a guard, every flash call of the box's storage (`storage.rs`) first checks
  that the caller's stack is not in PSRAM (`esp_ptr_external_ram`) and returns the error `flash
  access from a PSRAM stack refused`, logged once per boot, instead of the chip stopping.
  `job_thread::run` and `spawn` take where the stack lives (`StackPlace`); every other thread stays
  internal. `CONFIG_SPIRAM_ALLOW_STACK_EXTERNAL_MEMORY` is set in `sdkconfig.defaults` too, so the
  QEMU run measures the same PSRAM stack.
- **Why**: measured on the box with two script recipes, the internal RAM fell to 44 KB (floor
  64 KB, D27) and, after installs and uninstalls, the largest free block fell to 27 KB and an install
  was refused (36 KB needed: the per-job 20 KB stack and 16 KB of TLS session). The `script` thread's
  32 KB was the largest permanent share; the per-job 20 KB block the one that fragmentation refused.
- **Alternatives**: the Rust side's small blocks of a script recipe or an instance in PSRAM
  (`SPIRAM_MALLOC_ALWAYSINTERNAL`), only if the minimum falls under about 80 KB; the engine thread's
  stack in PSRAM (it writes the flash: refused, D59).

## D84: The CI's caches survive a version bump; the release workflow builds in `target/radio`

- **Date**: 2026-11-28
- **Choice**: the ESP-IDF download cache (`firmware/.embuild`) keeps its exact key (the IDF's
  settings and `firmware/Cargo.lock`) but falls back (`restore-keys`) to the last download of the
  same `firmware/.cargo/config.toml`, so a release's version bump, which changes `Cargo.lock`,
  no longer downloads ESP-IDF again; the Rust cache of the firmware also holds `target/radio`,
  the radio image's own folder, which had none. The release workflow's runner builds the radio
  image in `target/radio` (`SOLITE_RELEASE_TARGET_DIR`) to restore that cache; on a computer
  `scripts/release.sh` keeps `target/release-radio` (D63's reason: `box.sh flash` takes
  `target/radio`, and the runner has no box).
- **Why**: the firmware job took about 7 minutes, mostly in builds with no cache for the radio
  image, and the release of 0.2.0 found no ESP-IDF cache after its version bump (and failed in
  the window that left, #71).
- **Alternatives**: a cache key without `Cargo.lock` (a new `esp-idf-sys` could need tools the
  old download lacks: the fallback restores, the build completes the download, the exact key is
  saved); the release built from the CI's artifact (the build must stay in the job with no
  secret, D63).


## D85: Script recipes up to 32 KB, 4 recipes of 4 instances each (replaces D74's and D78's 16 KB, and D74's 2 recipes and 8 instances, spec 053)

- **Date**: 2026-11-28
- **Choice**: a script recipe's source is at most 32 KB (`MAX_SCRIPT_BYTES`); a request body at most
  32 KB and the 128 bytes of the signed file's header (`MAX_BODY_BYTES` = 32 KB + `HEADER_BYTES`,
  so that a signed file of 32 KB of source passes; `MAX_FILE_BYTES` is the body bound and
  `MAX_SOURCE_BYTES` 32 KB), the catalogue's `size` at most that file; at most 4 script recipes
  loaded (`MAX_SCRIPT_RECIPES`, 2 before), each with at most 4 instances
  (`MAX_INSTANCES_PER_SCRIPT`, 8 before), 16 in all (`MAX_INSTANCES`, unchanged = 4 x 4). The memory
  limit of one runtime stays 224 KB (D74).
- **Why**: Sowel's larger recipes ported to scripts are about 17 to 26 KB (`motion-light` is 15.3 KB,
  at the old bound). More, larger scripts with fewer instances each is the user's choice
  (2026-11-28). The limit was tried at 208 KB, to make 4 runtimes fit the PSRAM budget (4 x 224 KB
  is 54 KB over on paper): the 32 KB script then ran out of memory when loaded (QEMU), though it
  holds 139,992 bytes once loaded and 180,668 (peak 183,828) with 4 instances, so the load itself
  peaks between 208 and 224 KB.
- **Measured**: the engine's worst case (16 instances, 4 bodies of 32 KB, installs estimated at
  152 KB) 842 KB peak, 1019 KB with the log entries and the estimates, under the 1 MiB ceiling by
  4.9 KB (the installs' estimate, a sum of 146 KB, was rounded to 160 KB at first, which crossed the
  ceiling); the PSRAM arithmetic of the 4 runtimes at their limit: `docs/memory-budget.md`.
- **Alternatives**: a limit under 224 KB (the load of a 32 KB script needs more); 3 recipes (the
  arithmetic would fit: 3 x 224 KB); the installs' admission rule (a runtime's limit and a 512 KB
  reserve free in PSRAM) keeps the real PSRAM from running out meanwhile.


## D86: Allocations over 512 bytes go to PSRAM (replaces D23's 4096-byte threshold, spec 056)

- **Date**: 2026-11-28
- **Choice**: `CONFIG_SPIRAM_MALLOC_ALWAYSINTERNAL=512` (4096 before). A `malloc` of at most 512 bytes
  stays in internal RAM, a larger one goes to PSRAM. The 64 KB reserve (`RESERVE_INTERNAL`) is
  unchanged. The recipe catalogue is held as typed entries, once per read, instead of a JSON block
  parsed at each request. Each step of a recipe job logs the internal RAM (`recipe install: ...`).
- **Why**: on the box with 4 script recipes the internal RAM fell to 28 KB at its lowest across
  installs (85 KB at rest), under the 64 KB floor. QuickJS and mbedTLS were already in PSRAM; what
  remained was the Rust heap's small blocks (the engine's data, each recipe's Rust side, an
  install's transient blocks), all internal at 4096.
- **Review of what must stay internal** (spec 056, D59): the UART driver's rings, its object and
  its semaphores take `MALLOC_CAP_INTERNAL` when `CONFIG_UART_ISR_IN_IRAM=y` (the radio image sets
  it), whatever the threshold; Wi-Fi's own buffers, FreeRTOS objects, thread stacks (pthread's
  default caps) and the job threads (`MallocCap::Internal`) are allocated with explicit caps;
  lwIP and the FAT layer already prefer PSRAM (`SPIRAM_TRY_ALLOCATE_WIFI_LWIP`, `ff_memalloc`);
  the flash read and write copy a buffer outside the internal RAM through a bounce buffer
  (`esp_flash_read` and `esp_flash_write`), as they already did for the 32 KB files. No buffer given
  to a driver by `malloc` was found; nothing needed a new explicit capability.
- **Alternatives**: 256 (the next step if 512 is not enough); a custom allocator in Rust for the
  install only (unstable `allocator_api`); a shared QuickJS runtime (saves PSRAM, not internal RAM:
  spec 056's part 2).
- **Measured**: QEMU, 50 devices and 3 recipes: internal free after the engine's load 233 KB
  (min 135) at 4096, 276 KB (min 166) at 512. The box's figures: spec 056's result.

## D87: Tests by stage, scaled to the change

- **Date**: 2026-11-28
- **Context**: on 2026-11-28 the full verdict (10 to 13 minutes, QEMU included) ran about twelve
  times on the Mac, the run without the slow checks (4 to 5 minutes) about ten times, and each QEMU
  probe cost 4 minutes of emulation: most of a change's time went to checks that its files could
  not fail. The pull request's CI (5 to 6 minutes, three jobs side by side) was not the cost.
- **Choice**: three stages. While developing: targeted tests, then `validate.sh --changed` (the quick
  checks and the tests of the touched groups; no firmware build, no QEMU). Before the merge:
  `validate.sh --pre-merge`, run by the pull request's CI, scaled by `scripts/scope.sh` from the
  change's files: docs only, the quick checks; normal, the touched groups without the slow checks;
  critical (firmware, memory tests, update, backup, radio, storage, recipe installs and scripts,
  dependencies, CI and release scripts), the touched groups in full, QEMU included. main (`--skip-slow`)
  and the nightly run (full) keep every group, the net for what the scope misses; the full run also
  comes before a box session. The commit hook stays `--fast`.
- **Rejected**: the full verdict before every commit (D9's first form): its cost grew with the
  firmware images and QEMU until it set the pace of the work.
