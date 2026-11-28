# Living spec

What Solite does **today**, in business words. The source of truth: the code follows it, and
every change updates it in the same pull request (the "living spec delta" of its spec). Present
tense only: what it does not do is not written here, but in the roadmap.

## Purpose

Solite is a minimal Sowel in a Zigbee coordinator sold off the shelf, the Sonoff Dongle Max.
It automates an annex of a house (garage, pool house, workshop, outbuilding) or a small home on
its own, without a server, Docker or the cloud, by proposing the recipes itself; when a Sowel
exists, it becomes its extension. It keeps Sowel's concepts (devices, equipments, zones, recipes,
modes) with the same names, types and JSON as Sowel's API.

## Users

- **Its author**, for their own annexes and their family's homes.
- **Sowel users** who flash their own Dongle Max from a public release.

They pair Zigbee devices, make each one an equipment in its room, accept the proposed recipes, and look at the box from
its web interface or from Sowel.

## What the tool does

### Starts and shows its name

Each of the three programs starts and says which tool it is, with its version:

- **On the Mac**: `cargo run -p solite-host`, without `--radio` or `--sim`, prints `solite 0.1.0`
  and exits.
- **On the ESP32**: the firmware starts ESP-IDF and writes `solite 0.1.0` to the serial log,
  then its free memory: internal RAM (free, and largest free block) and PSRAM (free, of total),
  for example `memory: internal 245 KB free (largest block 110 KB), psram 2039 KB free of
  2048 KB`, or `psram none` without PSRAM. It runs on chips of revision 3 and later, like the
  Dongle Max's.
  After the memory line, the firmware mounts its data partition (FAT, at `/data`; it formats the
  partition when it cannot mount it, and logs `storage: formatted (<reason>)`), logs
  `storage: /data, <used> KB used of <size> KB`, reads its network setting from
  `/data/network.json` and logs its choice (`network: wifi (setting auto), no link started` in the
  box, sim and measuring images, which start no link; the radio image logs `network: wifi (setting
  auto)` and starts the link), and, in the image for the box, logs `radio: none`. The image runs on the Dongle
  Max as in QEMU; the measuring image (cargo feature `flash-saves`, which turns on `sim`) is built to
  run the realistic home on the box with every save written to the flash, and logs, instead of the count of
  saves run without the emulator's flash, `saves: <n> written to flash, longest <ms> ms (<file>,
  <KB> KB), total <ms> ms`: the time each save holds the radio loop. After the last save it logs
  `storage-after: /data, <used> KB used of <size> KB`, the use of the FAT partition by the home.
  The radio image (cargo feature `radio`, never with `sim`: `radio and sim are two images`) is built
  in its own target folder with its own ESP-IDF settings (`firmware/sdkconfig.radio.defaults`,
  which also puts the UART interrupt in internal RAM, turns on the WebSocket, sets the server's
  limits and the lwIP socket table, puts the workers' stacks and buffers in PSRAM and sets lwIP's core and
  SNTP's settings); the other images keep theirs. After the `network:` line the radio image reads
  `/data/wifi.json`, starts the Wi-Fi station (or logs why not) and SNTP, then starts one thread,
  `engine`, on core 1 with 32 KB of stack, which resets the radio, finds its serial speed, starts
  the radio's network and runs the engine's loop with the home read from `/data` (radio, network
  and the power-on pairing window at once; the engine's saves are held until the clock is set, 20 s
  at most), showing the measures and the pairing lines on the serial log. Once the home is loaded it
  serves Sowel's API, `/ws` and the interface on port 80 (see "Serves Sowel's API from the box").
  New lines: `solite-ui <n> files, <KB> KB`, `console: ready`, `wifi: ...`, `clock: ...`,
  `serving: ...`, `memory-network: ...`. A boot
  with a power-on reset opens pairing for 180 seconds, and so does the serial console's `pair` line
  at any time (see "Pairs a device"). Every 240 seconds it
  logs `serial: <baud> baud, rx buffer 4096 B, highest <n> B, full <k> times` and `heap: internal
  <n> KB free (min <m> KB), largest block <l> KB, psram <p> KB free (min <q> KB), largest block <r> KB, engine stack <s> KB unused of 32 KB, http stack <s> KB unused of <t> KB`.
  The radio image is a development image: its API is open on the local network (decision D33).
  It writes a change of definitions (a device, a zone, an equipment, a binding, a recipe instance)
  2 seconds after it first sees it unsaved, so a burst of changes gives one write per file; a stop
  (the console's restart) writes at once. A power cut loses at most the definitions of those last
  2 seconds (and one loop turn) and the devices' values of the last 10 minutes (decision D40). Its
  serial console also reads three lines (never from the network): `panic` (logs `console: panic asked`,
  then panics on purpose: the box restarts once with the cause `panic`, see "Supervises its
  engine"; the box check's fault), `saves hold` (logs `saves: held,
  nothing is written until saves flush or a restart`; from then on nothing is written, every change
  stays in RAM, and a restart, the serial port being opened included, forgets the hold) and `saves
  flush` (logs `saves: flush asked`; at its next turn, once the clock is set, the engine writes each
  file that has unsaved changes once, ends the hold and logs `saves: flushed, <k> files written`; a
  failed write is retried as any save). A stop still writes while held; a restart of the radio
  inside the engine does not. When the clock is set, the 10 minutes of the devices' values start
  from that moment (the saver started in 1970), so a boot writes no `devices.json` for its values
  alone.
  Every firmware image logs `flash: <n> writes since boot` after each write to `/data` (a save of a
  file, whatever its size, a removal, a rename, the boot marker's write and removal; counted once
  attempted, even when it fails; a 32-bit count that stops at its maximum).
- **On the Mac, in an emulated ESP32**: `sh scripts/qemu.sh` boots the firmware in Espressif's
  QEMU, with 2 MB of PSRAM, and runs the engine in it with the simulated radio and a realistic
  home (50 devices, 10 zones, 50 equipments, 3 `motion-light` instances, the files of
  `firmware/fixtures/`); it prints the banner, the memory line, the `storage:` and `network:`
  lines, then the `engine:` line (counts, heap used and peak, internal RAM and PSRAM free, with
  their minimum), and fails, with the last lines of the log, when the heap peak is above 350 KB, when a device of the home got no measure into the engine
  (`50 devices (50 reported)`), when the radio dropped a callback while the scenario played,
  when the internal RAM minimum free is below 64 KB, when the firmware panics or reboots, or when
  the `engine:` line does not show within 240 seconds. Before the engine, the image installs three
  update files of a 64 KB image, signed with a key pair made at boot, on the `update` thread of the
  radio image (16 KB of stack, the slot a counter in memory), and logs `update check: valid 200,
  image changed 400, signature changed 400` and the thread's `update: thread stack <n> of 16384
  bytes never used`; the run fails when a status differs or fewer than 2048 bytes stay unused.
  It then pulls a release once, from an in-memory fetch adapter (a manifest, one redirect, a signed
  file made at boot, whose first answer is cut after 40 KB and whose `Range` request is answered
  206), on an `update-pull` thread of the radio image (20 KB of stack), and logs
  `update pull check: status 200, resumes 1` and the thread's `update-pull: thread stack <n> of
  20480 bytes never used`; the run fails when the status differs, when the pull did not resume
  once, or when fewer than 1024 bytes stay unused. The resumed request touches an 8 KB array on the
  stack, standing for the TLS handshake a box runs there and the image, with no network, cannot
  (D68's margin); the signature check after the download is deeper still, so 1024 bytes is what
  is left at the deepest point (8336 unused measured).
  After the `engine:` line the image QEMU runs (built with the scripts, never flashed on the box)
  loads the `motion-light` script and starts the realistic home's three kept `motion-light`
  instances (stopped until then with `Recipe not found: motion-light`, as on a box that has not
  installed the recipe, and kept with their settings), runs the home's motion scenario again, then
  a script that recurses as deep as its stack allows, calling the engine at its
  deepest, on the engine thread and on a `script` job thread whose stack is in PSRAM as the
  box's, and logs `recipes: script
  motion-light <KB> KB source; psram +<KB> KB (peak +<KB> KB); internal +<KB> KB (min <KB> KB
  free); engine stack <n> of <n> unused; script thread stack <n> of <n> unused; heap peak <KB> KB`;
  the run fails when that line is missing, the three instances' peak passes 75 % of a script's
  memory limit, the internal RAM minimum is under 64 KB, either stack keeps less than 2 KB unused,
  or the heap peak passes 574 KB. The two gates measure two things: the `engine:` line's 350 KB is
  the engine with the home's three instances stopped (the engine runs with no script loaded); the script
  phase's 574 KB (350 KB for the engine and 224 KB, the ceiling of one script runtime) is the
  whole heap (internal RAM and PSRAM), free before the engine started minus the lowest free seen
  until the home's scenario has played again with the `motion-light` runtime loaded and the three
  instances running (the depth script comes after it).
  The script is the file `recipes/scripts/motion-light.js` read at build time by that
  phase only (the radio image carries no script). The phase also signs that script at boot with a
  throwaway key, installs it through an in-memory fetcher on a `recipe` thread of 20 KB, loads it
  again as at start, and logs `recipe: thread stack <n> of 20480 bytes never used`; the run fails
  when that line is missing or fewer than 2048 bytes stay unused.
  The measuring image (`flash-saves`) runs the same check at its boot, without the scripts. `sh scripts/validate.sh` runs it. The image
  holds the bootloader, the 16 MB partition table of `firmware/partitions.csv` (settings, two
  application slots of 4 MB, and a data partition of about 7.9 MB, blank at every run) and the
  application in the first slot, at the offset the built table gives. Allocations above 4 KB go
  to PSRAM; 64 KB of internal RAM are kept for the network and DMA, and the TLS library takes its
  buffers from PSRAM.
  The box's files are saved through a temporary file (`<name>.tmp`), synced then renamed to `<name>.new`: FatFS refuses a rename onto an existing file, so a save deletes the old one then renames `.new`; a read that finds `<name>` missing and `<name>.new` present recovers it and logs a line, and deletes any `.tmp` (a write that did not end, never promoted).
- **In the browser**: the home screen (see "Shows the home in the browser"), in Sowel's Hybrid or
  Dark theme as the system asks.

### Chooses its network

The box has one network link, Wi-Fi or Ethernet, set by `auto`, `wifi` or `ethernet` (read from
`{"link": "auto"}`, values in lowercase, other fields ignored). `auto` is the default, and the
setting a missing, empty, over-long (more than 256 bytes), invalid or unknown one falls back to,
with a one-line reason for the log, for example `network: setting unknown "WiFi", auto`. With
`auto`, a cable at boot means Ethernet, else Wi-Fi; `wifi` and `ethernet` hold whatever the
cable says. The choice is made once and holds until the next start. The core makes this choice
(`solite_core::network`). The radio image starts the Wi-Fi link when the choice is Wi-Fi and
`/data/wifi.json` is valid; the choice `ethernet` starts no link in this image, and the other
images start none. The Wi-Fi settings are a name and a password (bounded), the connection is
retried forever (1 s doubling to 60 s), and each failure names its reason.

### Talks to the radio

`cargo run -p solite-host -- --radio tcp://<address>:6638` connects to the radio of a Dongle
Max that runs its original firmware (its ESP32 relays the radio over TCP, port 6638, one client at
a time). It resets the radio, speaks Silicon Labs' ASH and EZSP protocols with it, and shows:

```
radio: tcp://192.168.1.50:6638
ezsp: protocol 13, stack type 2, EmberZNet 7.4.4.0
radio address: 0x8C6FB9FFFE123456
network: no network
```

The network state is one of: no network, joining, joined, joined with no parent, leaving, or
`unknown (<n>)`. It accepts radios speaking EZSP 13 to 19 (EmberZNet 7.2 and later). On a
failure it prints one line on the error output and exits 1: `radio: cannot connect to <address>:
<reason>`, `radio: no answer to reset`, `radio: no answer to command`, `radio: link lost`,
`radio: connection closed by the radio`,
`radio: error 0x<code>`, or the EZSP error (a version outside 13 to 19 is named); a malformed
`--radio` or `--sim` prints the expected form and exits 2.

The radio image of the firmware talks to the radio of the Dongle Max over the serial link (UART1:
TX GPIO17, RX GPIO13, 8 data bits, no parity, 1 stop bit, no flow control), with a receive ring of
4096 bytes and a transmit ring of 1024 bytes in internal RAM, and the interrupt in internal RAM so
that bytes keep arriving while a flash write stalls the caches. The radio's two lines, reset
(active low) and bootloader (active high, never set active), are described by role in the core,
with the hardware reset as a list of steps: bootloader inactive, reset active for 100 ms, reset
inactive, 1000 ms for the radio's application to start. The firmware runs them after its own boot,
each level written before the pin becomes an output (GPIO12 low, then GPIO15 high). The speed is
probed at each boot: 115200 first (the speed fixed in the core), then 460800 and 921600. A speed
that gets no answer to the reset within 5 seconds logs `radio: no answer at <baud> baud` and the
next is tried; any other failure stops the probe with its error. Found: `radio: uart <baud>
baud`, then the `ezsp:` and `radio address:` lines; found at a speed other than 115200 it also
warns `radio: answered at <baud> baud, RADIO_BAUD is 115200`. No answer at any speed logs
`radio: no answer at any baud rate (115200, 460800, 921600)`; the engine does not start and the
box restarts (`radio-silent`, see "Supervises its engine").

A radio error while the engine runs (link lost, an ERROR frame, a driver error) restarts the radio:
what is due is saved, an open pairing phase ends, the reset sequence runs, the speed is probed and
the network is resumed (never formed, no pairing window). At most 3 restarts in 10 minutes; past
that the image logs `radio: stopped after 3 restarts in 10 min: <last error>` and restarts the
box (`radio-stopped`). A radio that answers at no speed in a restart restarts the box too
(`radio-silent`). A storage error stops the loop without resetting the radio and restarts the box
(`engine-error`).

### Pairs a device

`cargo run -p solite-host -- --radio tcp://<address>:6638 --pair [seconds]` (180 by default,
1 to 254) starts the radio's Zigbee network and opens pairing:

- the first time, the radio **forms** a network on channel 15, with random keys and identifiers,
  and saves it in its own flash; the host writes a backup of it (identifiers and keys) to
  `solite-data/network.json` (`solite-data/sim/network.json` under `--sim`; `--data
  <dir>` for another folder), readable by its owner only,
  never printed; when that file already holds a network (put there by a restored full backup,
  see "Backs up and restores the whole box") and the radio has none, the radio forms **that**
  network, with its keys, instead of a new one: `network: formed (restored from the backup), ...`;
- a Mac host whose `network.json` parses and that is pointed at another, blank radio forms the old
  network on it: two coordinators with the same PAN and keys if the first radio still runs;
- after that, the radio **resumes** its network; a coordinator network another program made is
  used as it is, never destroyed, and named so; the one exception is a restore (next
  paragraph);
- a device that joins is shown with its address, then with the manufacturer and model it sends
  (an original Sonoff SNZB-02 sends `eWeLink TH01`); `device: <address> unknown unknown` when it
  does not answer, `device: <address> not identified (radio busy)` when the radio stays busy;
  beyond 32 devices in one session, `pairing: too many devices, <address> ignored`; a device
  already in the network that announces itself again is shown again;
- an alarm device (IAS zone) is enrolled right after its identification, while it is awake: the
  host reads its zone, writes its own address into it and answers its enrollment, as Zigbee2MQTT
  does (`, enrolled (motion)`, `, enrollment unconfirmed (motion)` or `, enrollment failed` ends
  its `reporting:` line); a failed or unconfirmed enrollment is tried once more when the device
  next speaks; enrollment requests are answered in every mode;
- after identification, the host sets up the device's reporting: it binds each standard cluster
  the device has (temperature, humidity, pressure, illuminance, occupancy, on/off, level, colour,
  power, thermostat)
  to itself and asks for reports (`reporting: <address> temperature, humidity, battery`, or
  `reporting: <address> failed: <clusters>`, both parts when some succeed and some fail,
  `reporting: <address> none` when the device has none of them); a failed setup is tried once more when the device
  next speaks, except when the device gave no description of its clusters (`reporting: <address>
  failed: no descriptor`: pair it again); then the measures it sends are shown;
- pairing closes at the end of the time (identifications, enrollments and reporting setups in
  progress finish first, at most 70 s; a setup still running then shows `reporting: <address> cut
  short`), or on Ctrl-C (exit 130).

**After a restore, a radio that runs another network leaves it.** A restore that brought a
network file (see "Backs up and restores the whole box") leaves a one-shot mark, `zigbee.restore`
(the same name on the Mac host), in the data folder. At the next network start, with the mark
present, a radio that runs a network other than the restored one **leaves it** (the usual case: a
box forms its own network at its first start, and a factory reset keeps it) and forms the
restored one, with its PAN, channel and keys; a radio already on the restored network resumes it.
The network it left keeps its keys in `zigbee-<PAN id>.json` (`network-<PAN id>.json` on the Mac
host), set aside when the restore is applied, so no network's keys are lost. The mark is removed
once the network is up; a start that fails keeps it for the next. It is the only case where Solite
leaves a network: without the mark a radio on a network resumes it and never leaves it. It is a
best effort: the radio's own address, its frame counter and its list of child devices are not
carried over, so a device that remembers the old radio's higher counter may ignore its commands
until it passes it, and an end device whose parent was the coordinator may need pairing again.

With `--pair`, the identification's `network: <state>` line is replaced by the `network:` line
below; `network: resumed (made by another program), ...` for a network the host has no backup
of. When a network is formed, the backup is written as soon as the radio accepts it, before the
steps after it; a backup of another network already in the folder is kept, renamed
`network-<PAN id>.json`. Warnings (a refused setting, a dropped callback) go to the error output.

```
network: formed, channel 15, PAN 0x1A2B, extended PAN 0xDDDDDDDDDDDDDDDD
pairing: open for 180 s
joined: 0x00124B0012345678 (0x5C3D)
device: 0x00124B0012345678 eWeLink TH01
pairing: closed
```

In the radio image of the firmware, the radio's network is started as `--pair` starts it (resumed,
or formed on channel 15 with the ESP32's hardware random generator) and, when formed, the backup
is written to `/data/zigbee.json` in the host's format before the next step; a `zigbee.json` of
another network is kept, renamed `zigbee-<PAN id>.json`. A radio with no network forms the network
of `/data/zigbee.json` when the file holds one (a restored backup). The file holds the network and
trust-centre keys in plain text, on a flash that is not encrypted. A marker file, `/data/booting`,
is written at each boot and removed after 10 minutes of running. Pairing opens for 180 seconds at
boot only when the reset is a power-on (power, or the USB bridge's reset) and no marker was there:
a crash, a watchdog, a brown-out or a boot loop opens nothing and logs `pairing: not opened at
boot (<reason>)`. The window is the home's pairing window; the image then only listens, and shows
`pairing: open for 180 s`, the devices' lines and `pairing: closed`; a radio that refuses to open
joining logs `pairing: failed: <reason>`. The same window opens for 180 seconds, whatever the boot
and the marker, when the firmware's serial console reads the line `pair` (sent by `sh scripts/box.sh
pair` over the USB cable; the console logs `pairing: asked from the console` and the engine takes
the request within half a second). Opening the serial port resets the box: when the previous boot was
less than 10 minutes before, the marker refuses that boot's window and the `pair` line opens it.
Pairing never opens from the network on the box (decision D36). A setting the radio refuses while the network forms is
logged as on the Mac.

While serving, pairing also opens from the API or the interface (see "Serves Sowel's device API"
and "Shows the home in the browser"); the devices that join are followed, identified and kept
exactly as with `--pair`, with the same lines.

New errors: `radio: network did not start`, `radio: the radio is not a coordinator`,
`radio: the radio refused <command> (status 0x<code>)`, `no random source: <reason>`,
`cannot write <path>`. Devices are kept as described below.

### Shows the measures

`cargo run -p solite-host -- --radio tcp://<address>:6638 --listen` resumes the radio's
network (never forms one: `radio: no network, pair a device first`) and shows every measure the
paired devices send, until Ctrl-C (exit 130):

```
network: resumed, channel 15, PAN 0x1A2B, extended PAN 0xDDDDDDDDDDDDDDDD
listening
0x00124B0012345678 temperature 19.52 °C
0x00124B0012345678 humidity 54.1 %
0x00124B0012345678 battery 100 %
```

Measures are read from the standard Zigbee clusters, without a device definition, and named as
in Sowel: `temperature` (°C), `humidity` (%), `pressure` (hPa), `luminosity` (lx), `motion`,
`light_state`, `light_brightness` (%), `light_color_temp` (mired), the parts of a colour
`color_x`, `color_y` (0 to 1, four decimals), `color_hue` (°), `color_saturation` (%), `battery`
(%), `voltage` (V), and `alarm`, `tamper` (IAS zone notifications, sent by enrolled alarm
devices), and `action` (a remote's On/Off, Level and Scenes commands, shown `action toggle`; its output clusters are bound and the `reporting:` line lists it as `action`); booleans are shown `motion true`, `light_state on`. A colour bulb's reporting is set up
for its colour temperature and X/Y (`light_color` in the `reporting:` line). The banner and the radio's lines come first, as with `--pair`.
Invalid values and
implausible ones (temperature outside −50 to 150 °C, humidity outside 0 to 100 %, a colour
temperature of 0 or above 1000 mireds, X or Y above `0xFEFF`, a hue or saturation of 255) are not
shown.
A device is shown by its 64-bit address, or by its network address when the radio does not know
it. The host answers each report as Zigbee requires (a default response). A sensor reports at most
every 30 s and at least every hour; its battery every few hours.

### Keeps devices, in Sowel's format

A paired device becomes a Sowel device (`Device` of Sowel's `types.ts`): `id` (UUID), `name`
(`TH01 5678` by default: the model and the end of the address; `Zigbee 5678` without a model),
`manufacturer` and `model` as sent, `ieeeAddress` and `sourceDeviceId` (`0x` + 16 lowercase hex
digits), `source` and `integrationId` `solite`, `zoneId` (`null`; an API client may set it, nothing reads it), `powerSource` (`mains`,
`battery` or `unknown`, from the device's announce), `status` (`online`; `offline` after 10
minutes of silence for a mains device, 25 hours otherwise; `unknown` before any message). As
Zigbee2MQTT's availability does, a mains device silent for 9 minutes is pinged (its Basic
cluster's ZCL version read), again 30 s later, then every 10 minutes while it stays silent: any
answer is a message, so a relay that keeps its state (it reports it once an hour at most) stays
online, and an offline one comes back once it answers; at most 4 pings a second.
The other fields:
`lastSeen`, `createdAt`, `updatedAt` (ISO 8601 with milliseconds). A date taken before the box's
clock is set (the box starts in 1970) is moved forward by the clock's jump when the clock is set
(`lastSeen`, `createdAt`, `updatedAt`, the data's `lastUpdated` and the `lastChanged` of the
equipment's data), so a device heard during the wait shows the real date and stays `online`. A
`lastSeen`, `lastUpdated` or `lastChanged` read in 1970 from storage (a former boot) is `null`.

Its data (`DeviceData`), one per measure, are named as Sowel's Zigbee2MQTT plugin names them:
`temperature` (°C), `humidity` (%), `pressure` (hPa), `illuminance` (luminosity, lx), `occupancy`
(motion), `state` (light_state), `brightness` (0 to 254), `color_temp` (light_color_temp, mired),
`color` (light_color, type `json`), `battery` (%), `voltage` (mV), and for a metering plug `power`
(W, Electrical Measurement `0x0B04` ActivePower, else Metering `0x0702` InstantaneousDemand; one
`power` when a device has both), `energy` (kWh, Metering CurrentSummationDelivered) and `current`
(A, Electrical Measurement RMSCurrent); each with its last `value` and `lastUpdated`. The
multiplier and divisor of these clusters are read once the reporting is set up (and again at the
next report when they are not known, for instance after a restart): a value received before is not
published, the next report carries it; a divisor of 0 or absent counts as 1, a multiplier absent as 1.
Power is a whole number of W, energy and current have 2 decimals. The mains voltage is not a data. A remote (an endpoint with On/Off, Level or Scenes among its output clusters and
On/Off not among its input clusters) is bound at pairing, on each of those output clusters, and
gets `action` (category `action`, type `text`): the generic name of the command it sends, `on`,
`off`, `toggle`, `brightness_move_up`, `brightness_move_down`, `brightness_step_up`,
`brightness_step_down`, `brightness_stop` or `recall_<scene>`; every command is a
`device.data.updated` event, even the same one twice. The names of a model's own buttons
(`single`, `double`...) are not known. A shutter (Window Covering `0x0102`) gets `position`
(shutter_position, %, 100 open, 0 closed: `100 - CurrentPositionLiftPercentage`; a lift above 100
is dropped), reported at most every 1 s, at least every 300 s, on a change of 1. A thermostat
(Thermostat `0x0201`) gets `local_temperature` (category temperature, °C: LocalTemperature, 0.01 °C
rounded to 0.1; `0x8000` dropped; a device that also has the Temperature cluster keeps its
`temperature` apart) and `setpoint` (category setpoint, °C: OccupiedHeatingSetpoint, same
rounding), reported every 30 to 600 s on a change of 0.1 °C for the first, every 1 to 600 s on a
change of 0.1 °C for the second. `color` is Zigbee2MQTT's colour object, at most `x`, `y` (0 to 1), `hue` (0 to 360)
and `saturation` (0 to 100): the colour measures of one message replace their fields once (one
`device.data.updated`), and a message carrying X/Y and no hue/saturation drops hue and saturation
(and the reverse), so the object never mixes two colours. Its orders (`DeviceOrder`): `state`
(light_toggle, `ON` / `OFF`) for an on/off device, `brightness` (set_brightness, 0 to 254) for a
dimmable one, `color_temp` (set_color_temp, 150 to 500) and `color` (set_color, type `json`) for a
colour one (every device with the colour cluster, a tunable white bulb included), and for a
shutter `state` (shutter_move, type `text`: `OPEN`, `CLOSE` or `STOP`, any case; anything else is
`Invalid value for state`) and `position` (set_shutter_position, 0 to 100 with 100 open: a whole
number, refused otherwise). They send the Window Covering commands Up, Down, Stop and Go To Lift
Percentage with `100 - value`; no position is published after an order, the shutter's reports
give it. A thermostat has the order `setpoint` (set_setpoint, a number from 5 to 30 °C, any
step, else `Invalid value for setpoint`): a Write Attributes of OccupiedHeatingSetpoint (int16,
value x 100), answered by a Write Attributes Response (or a Default Response); a refusal is the
order's error, and no setpoint is published after an order, the device's report gives it. A colour bulb
paired before this was possible keeps only its on/off and level data until paired again. A device
paired again keeps its id and name. Alarm devices (IAS zone: motion, contact, water leak, smoke sensors) also get the data of their
zone: `occupancy` (motion), `contact` (contact_door, true when closed), `water_leak`, `smoke`, and
`tamper` and `battery_low` (generic) once the device first reports them. Alarm devices paired
before this was possible keep only their battery data until paired again.

The host keeps them in `devices.json` in its data folder (owner-only): a device paired, renamed
or deleted is written within 2 seconds under `--serve` (a burst of changes gives one write) and at
once with `--pair`, the latest values every 10 minutes, and everything at once when the host stops,
so a restart keeps them. At most 64 devices, 8 data and 4 orders each (a stored device over the bound keeps its first 8 data, logged once, `devices: <name>: <n> data, <n - 8> not kept (at most 8)`); beyond: `pairing: device store
full (64), <address> ignored`. A `devices.json` that cannot be read stops the host
(`devices: <file>: cannot read: <reason>`), never overwritten. One host at a time per data folder.

### Serves Sowel's device API

`--serve [port]` (8080 by default), with `--radio` or `--sim`, starts the network as `--pair`
does (it forms one, with its backup, on a radio that has none; the errors and the `network:` line
are `--pair`'s, before `serving:`), listens, keeps the devices up to date, sends the orders it receives, and serves on `127.0.0.1` only (the box serves in its own section below; the port must also be free on `[::1]`, which a browser tries first for `localhost`: if another program has it, the host says so and exits 1), with Sowel's bearer token (see "Protects the home with a password"; a browser page
from another site is also refused by a `Host` check, and by an `Origin` check on `/ws`). With
`--recipe-script <file.js>` (repeatable) it also runs recipes written as scripts (see "Runs script
recipes"). The routes:

| Route | Answer |
| ----- | ------ |
| `GET /api/v1/devices` | the devices with their data and orders, in pairing order |
| `GET /api/v1/devices/:id` | one device; 404 `{"error":"Device not found"}` |
| `PUT /api/v1/devices/:id` `{"name"?, "zoneId"?}` | renames it (1 to 64 characters); `zoneId` (an id, or `null`; 404 `Zone not found`) is kept as in Sowel's format, read by nothing: a device is in a room only through its equipments; a name or zone change emits no live event, as Sowel's (the client reads the answer) |
| `DELETE /api/v1/devices/:id` | 204; the host forgets it (it stays in the Zigbee network) |
| `GET /api/v1/integrations/solite/permit-join` | 200 `{"permitJoin": <bool>, "remaining": <seconds>}`, plus `"error"` when the radio refused the last opening |
| `POST /api/v1/integrations/solite/permit-join` `{"time"?: 0..254}` | 202 with the same body, the state asked for: opens joining for `time` seconds (180 when missing; a window already open restarts), or closes it (`0`); 400 `Invalid field: time` for anything but an integer from 0 to 254, `Unknown field: <field>`, `Invalid JSON`; 404 `Integration not found` for another integration id; `DELETE /devices/:id` of a device being identified or set up answers 409 `Device is being paired` |
| `GET /api/v1/auth/status`, `POST /api/v1/auth/setup`, `/login`, `/refresh`, `/logout` | Sowel's authentication (see "Protects the home with a password") |
| `POST /api/v1/system/factory-reset` | 204: erases the home and the account (see "Protects the home with a password") |
| `GET /api/v1/system/version`, `POST /api/v1/system/version/check`, `POST /api/v1/system/update` | the version and the last check's latest release; a check now; the signed update file as the body, or an empty body to pull the latest release (see "Updates itself from a signed file") |
| `POST /api/v1/system/restart` | restarts the box (an empty body; see "Restarts on request" under "Updates itself from a signed file") |
| `GET /api/v1/backup`, `POST /api/v1/backup` | the full backup file, with the password in `X-Backup-Password`; its restore (see "Backs up and restores the whole box") |
| `POST /api/v1/equipments/:id/orders/:alias` `{"value": ...}` | sends the order to the bound devices (see "Organises the home") |
| `GET /ws` (WebSocket) | live events (see "Live events" below) |
| `GET /`, `GET /assets/<file>` | the interface (see "Shows the home in the browser") |
| `/api/v1/recipes`, `/api/v1/recipe-instances` | the recipe and its instances (see "Runs the motion-light recipe") |
| `GET /api/v1/plugins`, `GET /api/v1/plugins/store`, `POST /api/v1/plugins/store/refresh`, `POST /api/v1/plugins/install`, `POST /api/v1/plugins/:id/update`, `POST /api/v1/plugins/:id/uninstall` | Sowel's plugin routes, for the script recipes: the installed ones, the catalogue, their install, update and removal (see "Runs script recipes") |
| `GET /api/v1/system/resources` | the memories, the storage and the script recipes' rows (see "Shows its resources") |

Errors are `{"error": "<message>"}` (400, 401, 403, 404, 405, 409, 411, 413, 415, 426, 429; 500 `Update failed: <message>`, `Update check failed: <reason>` and `Download failed: <reason>`, and 501 on the update and check routes (501 on the backup routes too, through the plain handler); 502 for an order no device took; 503 `Busy` beyond 4 requests at
once, or beyond 2 orders from the API at once, or for a 5th live client). A request with a body is refused with 415 unless it is sent as
`application/json`, so a web page cannot send one from the browser; the exceptions are the update route and the backup restore, which take
`application/octet-stream` (a cross-origin form cannot send that type, and the token is required). The lists of devices and equipments are sent in chunks (HTTP chunked transfer), so
their size never weighs on memory; the files are written the same way. It prints `serving:
http://127.0.0.1:<port>`; `--verbose` also prints the measure lines. `--pair` adds the devices to
the same file. Ctrl-C writes the store and exits 130.

**The pairing window.** Joining stays closed until the route opens it; the radio loop applies the
request at its next turn (a request made before the first turn is applied at the first), the
window is open until its time ends, on `time` 0, or on Ctrl-C (joining closed, best effort). From
the opening, the coordinator follows the devices that join or announce themselves (at most 32 per
window; the devices of earlier windows are let go at each opening, and the ones still being set up
are kept), then, once the time is over, until their identifications, enrollments and reporting
setups are done (at most 70 s; `reporting: <address> cut short` for a setup cut), and only
listens again after that. `remaining` counts whole seconds, rounded up. The lines are printed
without `--verbose`: `pairing: open for <n> s`, `joined:`, `device:`, `reporting:`, `pairing:
closed`, `pairing: device store full (64), <address> ignored`, and `pairing: failed: <reason>`
when the radio refuses the opening (kept in `error` until the next `POST`; the window stays
closed and the engine goes on). A device deleted from the home that announces itself during a
window comes back as a new device; a setup still running at its deletion does not bring it back,
and a setup cut short at the cap is abandoned. The route needs the token (decision D49).

#### Live events

`GET /ws` is Sowel's WebSocket: the token comes in the `Authorization` header or in a `bearer.<token>` subprotocol (the first offered other than `bearer.none`); the first offered `bearer.` subprotocol is echoed back; after the upgrade, no token closes the socket with 4001 `Authentication required`, a token that is not a live access token with 4001 `Invalid token` (the `Host` and `Origin` checks, 4003, come first, the 5th client's 1013 after); a session that ends later does not close an open socket. The first message is `{"type":"connected","message":"Connected to Sowel engine",
"version":"0.1.0"}`. A client sends `{"type":"subscribe","topics":[...]}`, which replaces its
topics (`system` always included; unknown names ignored), and then receives, every 200 ms when
there is something, one JSON array of Sowel's events in the order they happened, keeping only
the last event of a device data, an equipment binding, a status or a zone within the 200 ms.
Fed topics: `devices` (discovered, data updated, status changed, removed), `zones` (created, updated,
removed, aggregate changed), `equipments` (created, updated, removed, data changed, status
changed, order executed or failed, with the recipe as source when a recipe sent it), `recipes`
(instance created, removed, started, stopped, state changed, error), `system` (settings changed, sunlight changed). At most 4 clients; a client
whose queue passes 256 events or 8 KB is closed (1013) and reconnects; one silent for 60 s, or
that cannot be written to for 5 s, is dropped; a message over 1 KB closes it (1009); an `Origin`
other than the API's own address or the interface's development server closes it (4003).
`GET /ws` without an upgrade answers 426 `Upgrade Required`, a WebSocket version other than 13
400 `Unsupported WebSocket version`, another method 405; an upgrade on any other path 400; Ctrl-C
closes each client with 1001. The host accepts at most 64 connections at once (a 65th is closed
without an answer); a request must arrive within 10 s of opening its connection, and each answer closes its
connection (`Connection: close`).

### Serves Sowel's API from the box

The radio image serves on port 80 of the local network, over IPv4. The box is kept on the home's
own network (not a guest network) with no router port forwarded to it (decided by the user,
2026-11-22, decision D33). The `Host` rule accepts private IPv4 addresses, the box's name
(`solite-<6 hex digits>`) and its suffixes. The routes and errors are those of the Mac host,
except:

- `GET` and `POST /api/v1/integrations/solite/permit-join` answer as on the Mac, behind the token
  (decision D49); pairing also opens at a power-on boot or from the serial console's `pair` line,
  over the USB cable (decision D36).
- On `/ws` the refusals come after the upgrade, as close codes: 4003 for a wrong host, 4003 for an
  origin not allowed, 1013 for a 5th client. The development origins (`http://localhost:5173`,
  `http://127.0.0.1:5173`) are accepted as on the Mac. The subprotocol `bearer.none` is echoed; the token is read from the second subprotocol (or the `Authorization` header), so a client that offers only `bearer.<token>` gets no subprotocol echoed and its browser drops the socket.
  A dead client is found by TCP keep-alive (about 45 s) instead of 60 s of silence.
- A body sent without a length is answered 411 `Length required` (the update route is the exception: with neither `Content-Length` nor `Transfer-Encoding` its body is empty, which pulls the latest release; a chunked upload to it is 411, and the backup restore keeps its 411); a body over 16 KB, 413 (except
  the update route's, streamed up to 4 MB and a header, see "Updates itself from a signed file", and the backup restore's, streamed up to 2 MiB); a body
  that stops for 2 s, 408 `Request timeout`, its connection closed; a request head over 4096 bytes,
  431 (the backup's password header can need 1536 of them). A method other than GET, POST, PUT, DELETE and PATCH is answered 405 by ESP-IDF itself.
- On the box the server is one task (ESP-IDF's): it waits at most 2 s for the next bytes of a request (an inactivity gap, so a large upload that keeps flowing is not cut), and a connection that sent its headers and never its body holds that task for 2 s, during which every other request waits. The task answers 408 and moves on.
- In safe mode (see "Supervises its engine") every request that is not a `GET` answers 503
  `{"error": "Safe mode: the engine is stopped"}`, except the account's routes (`/auth/*`),
  `POST /api/v1/system/update`, `POST /api/v1/system/version/check`, `POST /api/v1/system/restart` and `POST /api/v1/plugins/:id/uninstall` (the way out of a script that breaks the engine); a restore and a factory reset are refused too.
- At most 12 connections are open; beyond, the least recently used is closed. At most 4 requests
  are answered at once; a 5th gets 503 `{"error":"Busy"}`.

The interface is served from the firmware: every file is gzip (`Content-Encoding: gzip`), with the
cache headers of the Mac. The engine never waits for the network: a lost link is retried while the
garage runs.

### Protects the home with a password

A home with no password asks for one: no default password exists. The first person to open the
interface (or to call `POST /api/v1/auth/setup`) gives a user name, a display name and a password,
and a person at the box confirms (decisions D47 to D49).

- **One account**, Sowel's admin: an id, a `username` (1 to 64 characters, no control character), a
  `displayName`, a `language` (`fr` or `en`, `fr` when missing), a creation date. The password is
  8 to 128 characters and at most 512 bytes (400 `Invalid field: password` otherwise). It is
  hashed with PBKDF2-HMAC-SHA256 (a 16-byte random salt, 10 000 iterations stored with the hash)
  and compared in constant time; on the box a hash takes about a second, with the home's lock
  released, so the radio loop is never held by a login. The account and its hash are in
  `auth.json` (`/data` on the box, the data folder on the Mac), written by the saver within 2
  seconds, once per setup or reset, never per login; `{"version":1}` means no account. An
  `auth.json` that cannot be read, or whose version, user, scheme (`pbkdf2-sha256`), iterations
  (1 000 to 1 000 000), salt or hash is wrong, stops the start (`auth: <file>: cannot read:
  <reason>`), never read as "no account". A home from before this change has no `auth.json`: setup
  is required and nothing of the home is changed. The password and the tokens are never stored,
  logged, or sent but in the tokens' own answers.
- **The confirmation at the box.** The core asks an adapter, `Presence`, whether a person at the
  box confirmed. The Mac host (loopback only) confirms always. The box, until its button is wired,
  confirms during the 10 minutes after a power-on boot (power-on, external reset or USB, as the
  pairing window), never after a software restart or a crash; the refusal says "Unplug the box and
  plug it back in, then try again within 10 minutes". A button adapter (confirms 120 s after a
  press: "Press the button on the box, then try again") is in the core, not yet wired. A refusal is
  403 `{"error": "<what to do>", "confirmRequired": true}`; nothing is written.
- **Sowel's routes**, mirrored from `src/api/routes/auth.ts`. `GET /api/v1/auth/status` →
  `{"setupRequired": <bool>}`. `POST /api/v1/auth/setup` `{username, password, displayName,
  language?}` → 201 with Sowel's `AuthTokens` (`accessToken`, `refreshToken`, `expiresIn` 900,
  `user` as Sowel's `User` with `role` `admin`, `preferences.language`, `enabled`, `lastLoginAt`,
  `createdAt`); an account already there is 403 `Setup already completed` (checked before the body),
  two setups at once make one account. `POST /api/v1/auth/login` `{username, password}` → 200
  `AuthTokens`, else 401 `Invalid credentials` (no account included; a wrong user name costs the
  same hash as a wrong password). `POST /api/v1/auth/refresh` `{refreshToken}` → 200 a new
  `AuthTokens` (the old session is closed), else 401 `Invalid refresh token`. `POST
  /api/v1/auth/logout` `{refreshToken?}`, behind the gate → 204; the session of that refresh token
  is closed, its access token too. Bodies are checked as the other routes (`Invalid JSON`,
  `Unknown field: <f>`, `Invalid field: <f>`, `Missing field: <f>`, 415 without `Content-Type:
  application/json`).
- **Sessions** are opaque (not JWTs): an access token (32 hex characters, 15 minutes) and a refresh
  token (30 days). At most 8 are kept, in RAM only; a 9th login drops the one opened first, and a
  restart ends them all (the user logs in again; the account stays). A refresh token sent as a
  bearer is refused. `lastLoginAt` is the last login since the start. A session or an attempt dated
  before the box's clock was set moves with the clock's jump, and the account's creation date too
  (`auth.json` is written again).
- **The gate.** Every route of `/api/v1`, existing or not, but `/auth/status`, `/auth/setup`,
  `/auth/login` and `/auth/refresh`, goes through Sowel's gate before the 413 and 415 checks (an
  unauthenticated POST is 401, not 415 or 413), in Sowel's order: no account 403 `{"error":"Setup
  required","setupRequired":true}`; no `Authorization: Bearer <token>` 401 `Authentication
  required`; a token that is not a live access token 401 `Invalid or expired token`. The interface's
  files need no token. The `Host` check comes first (403 `Forbidden host`). The Mac host and the
  box apply the same rules, except that the box answers what it decides from the request's head
  before the gate: 411 `Length required`, 413 for a body announced over 32 KB and 128 bytes, 408 for a body that
  stops (see "Serves Sowel's API from the box").
- **Brute force.** Setup and login together: at most 10 attempts per minute for the whole box (the
  core does not see addresses), then 429 `Rate limit exceeded, retry in 1 minute` without hashing.
  Only an attempt that would hash counts (a 400, 403 or 415 does not). Open sessions and their
  refresh keep working while the limit is reached.
- **`/ws`** needs the token too (see "Live events").
- **Factory reset**: `POST /api/v1/system/factory-reset` (Solite's, an empty body or `{}`), behind
  the gate, then the confirmation (403 as above), then 409 `Pairing in progress` while the pairing
  window is open or a device is still being set up, or 409 `Recipe install already in progress`
  during a recipe job. It erases, with their live events, every
  recipe instance, the installed script recipes (the runtimes dropped, the files removed by the
  saver), equipment, device (forgotten: it stays in the Zigbee network) and zone but the
  root (its name and description back to a first start's), the settings, the account and every
  session, then answers 204; `auth.json` becomes `{"version":1}`. The API then answers 403 `Setup
  required`. It keeps the Zigbee network and its keys (`zigbee.json`), the Wi-Fi settings and the
  network setting, so the deletion is partial: forgotten devices come back when they announce
  themselves during a pairing window.

### Organises the home in zones and equipments

As in Sowel, nothing acts on a device directly: a **zone** holds **equipments**, and an equipment
binds device data and orders under **aliases**. A device has no room of its own: it is in the
rooms of the equipments that bind it. Zones and equipments are also made from the interface, and an
equipment is completed from it: its bindings added and removed one by one, its devices replaced,
enabled and disabled (see "Shows the home in the browser").

- **Zones**: a tree under the root zone `Maison` (id `00000000-0000-0000-0000-000000000001`,
  always present). `GET/POST /api/v1/zones`, `GET/PUT/DELETE /api/v1/zones/:id` with Sowel's
  rules and messages: the root cannot be deleted, nor a zone with child zones or equipments; a
  zone cannot move under itself or a descendant. Deleting a zone returns its devices to no zone.
  At most 32 zones (the root included).
- **Equipments**: a device removed takes its bindings away from the equipments, each of which then
  gets `equipment.updated`. Every one of Sowel's 31 types is accepted, stored, served and filtered
  by (`GET /api/v1/equipments?type=`); eight have Sowel's binding rules (`light_onoff`,
  `light_dimmable`, `light_color`, `shutter`, `sensor`, `switch`, `thermostat`, `button`), and
  `light_color` counts as a light in the zone aggregation; the other 23 have no rule and no zone
  count of their own (a `door` or a `display` adds nothing to `doorsTotal` or `displaysTotal`);
  as in Sowel, a `weather` equipment and a `water_valve`'s bindings stay out of the room's
  figures. `POST /api/v1/equipments {name, type, zoneId, deviceIds}` binds every
  data and order of the given devices under their key as alias (`temperature`, `state`...) (the
  interface binds otherwise: see "Shows the home in the browser");
  bindings are added or removed one by one under `.../data-bindings` and `.../order-bindings`.
  Each equipment shows its bindings with their values, `lastChanged`, `stale` (a temperature,
  humidity, pressure or luminosity silent for 65 minutes), and its `status`: `online`,
  `degraded` (a device offline among several, or a binding stale) or `offline` (all its devices
  offline, or no binding), with the reason. At most 64 equipments, 8 bindings of each kind (a stored equipment over the bound loses first its dangling bindings, then keeps its first 8 valid ones of each kind, logged once, `equipments: <name>: <n> data bindings, <n - 8> not kept (at most 8)`, and is written once).
- **Sowel's optional equipment fields** (`energyProfile`, `requireConfirmation`,
  `trackOrderOutcome`, `meteringParentId`, `separateSupply`, `localProduction`, `family`,
  `plugInHybrid`, `usualVehicleId`, `solarProfile`, `invertDirection`, `timedCommand`): kept in
  `equipments.json` as written and shown wherever the equipment is (`GET`, the `equipment.*`
  events); a `null` is not stored (the field is absent, Sowel's default). `PUT` takes
  `requireConfirmation`, `plugInHybrid` (booleans), `trackOrderOutcome` (a
  boolean or `null`) and `family` (one of Sowel's six families or `null`), else 400 `Invalid
  <field>`; Sowel's rules on the resulting type, with Sowel's body `{"error","message"}`: a family
  on a type other than `switch` is 400 `FamilyNotApplicable`, a switch made another type loses its
  family, `SeparateSupplyNotApplicable` and `LocalProductionNotApplicable` for a separate supply or
  local production on a house total, a production meter, a solar panel or a vehicle. The other
  eight fields are refused when given with a value other than the stored one (`false` counts as
  stored for the three flags whose default is `false`; an object is compared by its bytes): 400
  `Not supported: <field>`, nothing changed. `POST` ignores all twelve, as Sowel's create. One
  equipment's fields take at most 1024 bytes of JSON, all equipments' 8192: past either, 400
  `Equipment fields too large (1024 bytes each, 8192 in all)`; at load, a field of the wrong shape
  or past a bound is dropped with a warning.
- **Proposals**: from a paired device's data and orders, the core proposes an equipment type and
  its bindings, with Sowel's binding rules (every data and order the type may bind; the
  interface's form instead binds one channel of a lighting relay or shutter, Sowel's candidates): `light_color` (an on/off order and a colour
  data or order), `light_dimmable` (with a brightness), `light_onoff` (on/off only, the
  ZBMINIR2), a colour bulb (cluster `0x0300`) being `light_color` with `state`, `brightness`,
  `color_temp` and `color` bound both ways, `shutter`, `thermostat`, `button`, `sensor` (a measure of Sowel's sensor
  categories), or none (battery only); bindings under Sowel's aliases, `state_2` for a second
  `state`, at most 8 of each kind. Not exposed by the API (Sowel has no such route); nothing
  computes it per measure.
- **Orders**: `POST /api/v1/equipments/:id/orders/:alias {"value": ...}` sends the order bound
  under that alias to its devices, as Sowel: `state` takes `true`/`false`, `"on"`/`"off"`,
  `"true"`/`"false"`, `1`/`0` (any case, spaces ignored; empty means on) or `"toggle"`;
  `brightness` takes 0 to 254 (255 counts as 254); `color_temp` takes a whole number of mireds
  from 150 to 500 (a number or a numeric string); `color` takes `{"x": .., "y": ..}` (0 to 1) or
  `{"hue": .., "saturation": ..}` (0 to 360, 0 to 100), `x`/`y` first when both are given,
  anything else 400 `Invalid value for color`. The value is read back once the device
  acknowledges. A colour order is sent with ZCL options 0, as Zigbee2MQTT: a bulb that is off may
  ignore it while answering success. Answers: `{"success":true}` when a device
  acknowledged; 502 with the reason (`device did not answer`, `device refused the command
  (status 0x..)`, `radio busy`, `device address unknown`) otherwise; 404 `Equipment not found`,
  400 `Equipment is disabled`, 404 `Order alias not found: <alias>`, 400 `Invalid value for
  <alias>`. A device that does not answer is tried once more 2 s later; the answer comes within
  12 s; no attempt is sent with less than 4.5 s left, so an order that could not start in time
  is dropped, never sent late, and a late answer to the first attempt is never followed by a
  second one. The new state is
  read back from the device, then shows in its data, its equipment and the zone aggregation.
  Each device's last network address and endpoint are kept, so a relay is reachable after a
  restart before it speaks. Orders also come from recipes (below): those are never waited for,
  have their own bound (32), and an order from the API is sent before them.
- **Zone aggregation** (`GET /api/v1/zones/aggregation`), as Sowel computes it, over each zone and
  its sub-zones: average `temperature` (1 decimal), `humidity`, `luminosity`; `motion`, the
  number of motion sensors and `motionSince` (none while the clock is not set, that is before
  2020-01-01, as on a box that has not reached the time server: the first computation after the
  clock is set gives the time); open doors and windows; water leak, smoke; lights
  on and in total; shutters; equipments unavailable by category. The root zone also carries `sunrise`, `sunset` and `isDaylight` (see "Keeps the home's settings"); the other zones `null`. It is recomputed after every batch of measures, status change and edit.

Zones, equipments and the home's settings are kept in `zones.json`, `equipments.json` and `settings.json` beside `devices.json`, written
within 2 seconds of a change (one write for a burst of changes; at once on stop); on load, references to things that no longer exist are dropped or moved
under the root. A `zones.json`, `equipments.json` or `settings.json` that cannot be read stops the host, like
`devices.json`.

### Keeps the home's settings

The home's name, location, time zone and sun offsets, through Sowel's settings routes, and the sun
times computed from them, as Sowel's sunlight manager does.

- **Settings** (`GET /api/v1/settings`, `PUT /api/v1/settings`): Sowel's flat map of strings, nine
  keys: `home.name` (64 characters at most), `home.latitude` (−90 to 90) and `home.longitude`
  (−180 to 180), written `48.8566` (digits, an optional `-` and decimals; no exponent, `+` or
  `inf`), `home.timezone` (a key Sowel names among its global ones; a name of `GET
  /api/v1/settings/timezones`, Solite's own list, `UTC` when unset),
  `home.sunriseOffset` and `home.sunsetOffset` (whole minutes, −180 to 180; 30 and 45 when unset),
  and Solite's own `update.manifestUrl` (a URL the box may fetch from, 512 characters at most; the
  GitHub address of the latest release's manifest when unset or `""`) and `update.autoCheck`
  (`"true"` or `"false"`; off when unset), see "Updates itself from a signed file", and Solite's
  own `recipes.catalogue` (the address of the script recipes' catalogue, the same rule and 512
  characters at most; `""` or unset reads as the default), see "Runs script recipes".
  A PUT changes the keys it gives and keeps the others; `""` is kept and reads as unset (as Sowel); it is checked whole
  first: 400 `Invalid body`, `Unknown setting: <key>` or `Invalid value for <key>`, nothing
  changed; else `{"success":true}` and Sowel's `settings.changed` event with the keys. The settings
  are kept in `settings.json`, written within 2 seconds of a change, and not written when a PUT
  changes nothing; an entry of the file that breaks its rule (a time zone the list no longer has)
  is dropped at load, with a warning.
- **The home's time zone** (`GET /api/v1/system/timezone`, Sowel's): `{"tz","source",
  "offsetHours"}`; with `home.timezone` set, `source` is `env` and `offsetHours` the offset now
  (`1` in a Paris winter, `2` in summer, `5.5` in Kolkata); unset, `{"tz":"UTC","source":
  "fallback","offsetHours":0}`, a location set or not.
- **Time zones**: a fixed list of 59 names (`UTC`, Europe, the French overseas departments and
  territories, the Americas, Africa, Asia, Oceania), each with today's daylight-saving rule.
- **Sun times**: on the root zone of the zone aggregation, `sunrise` and `sunset` in local
  `HH:MM` (suncalc's times, as Sowel), and `isDaylight`, true from sunrise + the sunrise offset to
  sunset − the sunset offset. Where the sun neither rises nor sets that day (polar day or night),
  `"--:--"` and `isDaylight` from the sun's height. `null` without a location, or while the clock
  is not set (before 2020). Computed at most once a minute and at once after a setting changes;
  when they change, Sowel's `sunlight.changed` event, and the root zone's `zone.data.changed`; a
  change of day or night shows within about 2 minutes without any measure.
### Backs up and restores the whole box

One file, the **backup** (`.slb`, "Solite backup"), holds everything needed to rebuild a box: the
home (devices, rooms, equipments, recipe instances, settings), the Zigbee network (PAN, channel,
keys) and the Wi-Fi settings. A box replaced or a lost `storage` partition no longer means pairing
every device again and building the home again. Sowel's routes (`GET` and `POST /api/v1/backup`, D89), with Solite's file (a
password-protected archive, not Sowel's ZIP of its database, decision D56); Sowel's `/backup/local`
and `/backup/restore-local` answer 404, and so does the old `/api/v1/system/backup`; behind the
token:

- **Export**: `GET /api/v1/backup` with the password in the header `X-Backup-Password`
  (percent-encoded UTF-8, so any character fits; never in the address). 200
  `application/octet-stream`, `Content-Disposition: attachment; filename="solite-backup.slb"`, sent
  in chunks (the size is not known ahead). The password is 8 to 128 characters: missing, 400
  `Backup password required`; too short or too long, 400 `Backup password must be 8 to 128
  characters`. Another export or restore running: 409 `Backup already in progress`. These
  refusals are whole JSON answers, before the file starts.
- **Restore**: `POST /api/v1/backup` with the file as the body (`application/octet-stream`,
  415 otherwise; on the box a missing `Content-Length` is 411) and the password in the same
  header. `?wifi=true` also restores the Wi-Fi settings; without it, the box keeps its own.
  Before the body is read, in this order: the token, the content type, the confirmation at the box
  (as the factory reset: 403 `{"error": <hint>, "confirmRequired": true}`), then 409 `Pairing in
  progress` when the pairing window is open and 409 `Backup already in progress`; then a declared
  length over 2 MiB is 413 `Backup too large` and a missing password 400 `Backup password
  required`. Another method: 405. While a restore waits for the restart (below) both routes answer
  409 `A restore waits for the restart`. During a recipe job (an install, update or uninstall of a
  script recipe) both routes answer 409 `Backup already in progress`. The archive holds no script
  recipe file: a restore on the same box leaves the files of `scripts/`, so the restored instances
  start; on another box they stay stopped (`Failed to restore: Recipe not found: <id>`) until the
  recipe is installed.
- **What it holds**: `devices.json`, `zones.json`, `equipments.json`, `recipes.json` and
  `settings.json` (stored plain, the file's chunks authenticated), and the two secrets, the Zigbee
  network file and `wifi.json` (the box only), **encrypted**. Never in it: the account
  (`auth.json`: a restore keeps the box's own account, and its sessions end with the restart, so
  the user logs in again) and the network setting (`auto`, `wifi`, `ethernet`: the box's hardware
  link). A home file the box has never written is absent from the file.
- **The format**: the magic `SOLITEBK`, the format number (1), the key derivation's iterations
  (100,000 at export; a restore accepts 10,000 to 400,000), a random salt and nonce, a key check;
  then each file as chunks of at most 4096 bytes; then a manifest (JSON: the version, Solite's
  version, each file's name, size and SHA-256) and a 32-byte HMAC-SHA256 over every byte before
  it. The key is PBKDF2-HMAC-SHA256 of the password (ten times the account's iterations, because
  the file leaves the box and can be attacked offline: estimated at 11 to 14 seconds on the box,
  ten times the login, and logged `backup: key derived in <n> ms`, once per export or restore,
  a second on the Mac); the secrets are encrypted with ChaCha20 and the whole
  file authenticated (decision D56). The password is used as the UTF-8 bytes received, with no
  Unicode normalisation. A file made by a newer format is refused.
- **On the box, off the workers**: an export or a restore runs on a `backup` thread started for
  the job, its 10 KB stack in internal RAM, while the API worker that took the request waits: a
  flash access, a read too, is refused by ESP-IDF from a stack in PSRAM (the chip stops and
  restarts). The box's files reach the core only behind a guard that turns a call from a PSRAM
  stack into an error (the export's stream ends without its end, the restore answers 500), never
  a flash access. When the thread cannot start, the answer is 503 `Busy, try again`. The thread
  logs `backup: thread stack <n> of 10240 bytes never used` at its end (D59).
- **Streamed, never held whole**: both ways use one 4 KB buffer on the Mac host; the box's export holds two (the archive reader's and the HTTP server's send buffer), so a 1 MB file peaks under 32 KB
  of heap (measured by a test). An export is read from the files as last saved (at most 2 seconds
  behind the home in memory) and makes the Saver write nothing while it runs; if the Saver is
  writing a file when the export starts, it waits up to 5 seconds, else 503 `Busy saving, try
  again`; an export ends after 120 seconds whatever the client. A failure mid-stream ends the
  answer without its end: the client keeps a truncated file, which a restore refuses. During the
  key derivation the box pauses 10 ms every 5,000 iterations, so its watchdog keeps being fed.
- **A restore is checked whole before it changes anything**: the file is read once and written as
  staged copies (`<name>.restore`, readable by their owner only) beside the live files, checking
  as it goes. Refused, with nothing changed: a wrong password (400 `Wrong backup password`, before
  any file is staged), a file that is not a backup (400 `Invalid backup: not a Solite backup`), a
  newer format (400 `Backup made by a newer version of Solite`), a body that ends early (400
  `Invalid backup: truncated`), a changed byte anywhere or bytes after the end (400 `Invalid
  backup: <reason>`), an unknown or repeated file name, a secret stored plain, a file over 1 MiB,
  more than 7 files, a chunk over 4096 bytes, iterations out of range, a body over 2 MiB (413
  `Backup too large`). The staged copies are then removed. The refusal is answered first, then the box reads and drops
  what is left of the declared body (about 10 s at most, plus one receive wait of 5 s) before it
  closes the connection: the client gets the refusal's answer, not a reset (not on the 413, where
  the client is not made to send the file). A refused file may have written up to
  its size in staged copies to the flash before its check fails.
- **Applied at the next start**: once everything holds, the staged copies are synced and a marker
  file, `restore.pending`, names what to do for each file (`promote <name>` or `remove <name>`).
  The answer is 200 `{"success":true,"message":"Backup restored: the box restarts, log in again
  after it","files":<n>,"wifi":<bool>}`; the box restarts right after (the update's restart:
  the Saver flushes, then a software reset). The Mac host answers `Backup restored: restart the
  host to load it`, prints `backup: restored <n> files; restart the host to load them` and applies
  the files at its next start. At every start, before anything reads the data folder
  (`backup::apply_pending`), the staged copies are renamed over the live files, then the marker
  is removed, logged `backup: <n> files restored` (`<n>`, like `files` in the answer, counts the
  files restored, not the home files removed); a power cut at any point is finished at the
  next start. A staged copy without a marker (a refused or interrupted restore) is removed; a
  marker that does not parse or names a file outside the list is discarded whole, with a warning.
  The home is the backup's, not a mix: a home file the backup does not hold is removed, with its
  leftover temporary copies; an absent network or Wi-Fi file leaves the box's own. `?wifi=true` on
  the Mac host (no Wi-Fi file there) is ignored: `"wifi": false`. Restoring the same file twice
  gives the same result.
- **The Zigbee network after a restore** (best effort, decision D57): the radio forms the
  network of the backup, with its keys, and leaves the one it runs; see "Pairs a device". Before a
  restored network file replaces one of another network, the live one is kept as
  `zigbee-<PAN id>.json`, so no network's keys are lost. What the radio itself holds is not in
  the file (its own address, its frame counter, its list of child devices): some devices may
  need pairing again.
- **Never restore a backup onto a second box while the first one still runs**: two coordinators
  with the same PAN and keys would answer the same devices (the same warning as the Mac host and
  the box sharing one radio network). Switch the first one off, or factory-reset it, first.

### Checks the Sowel contract

Solite is a subset of Sowel: its names and values are Sowel's (`types.ts` wins), or a deviation
kept by a decision. `sh scripts/validate.sh` runs the check (step `Sowel contract`,
`crates/core/tests/sowel_contract.rs`); it compares Solite's code with
`contract/sowel-snapshot.json` (Sowel's unions, interface fields, recipe slot types, events,
`home.*` settings keys and routes, with the Sowel commit) and, when Sowel's repository is present
(`SOWEL_REPO`, else `../sowel`), with that repository too: a snapshot that differs is a warning,
Solite not fitting Sowel's repository is a failure. It fails on: an equipment type of Sowel's
missing in Solite or one Sowel does not have; a Sowel equipment field Solite does not keep; a data
or order category, data type, status, power source, family, event, settings key, route or JSON
field of a device, zone, aggregation, equipment, binding or recipe that Sowel does not have and
that `docs/sowel-subset.md` does not list as a deviation; a deviation row that is stale or whose
decision does not exist; a table of `docs/sowel-subset.md` that differs from the code. Its last
line: `sowel contract: OK (...)`, naming what it was checked against.

### Runs the motion-light recipe

`motion-light` is a **script recipe** (next section), the port of Sowel's `sowel-recipe-motion-light`
1.1.2 (`recipes/scripts/motion-light.js`): Sowel's id, settings, messages and behaviour, version
`1.1.2`, through Sowel's routes. No recipe is built in: the box runs it once it is installed from
the catalogue or from a file, and the host (Mac) when given `--recipe-script
recipes/scripts/motion-light.js`. With nothing installed `GET /api/v1/recipes` lists `[]` and an
instance of `motion-light` is refused 404 `Recipe not found: motion-light`. Five behaviours are
Solite's engine's own, the adaptations of decision D81: a deleted equipment leaves the settings by
the generic settle; a button that loses its `action` data binding disables the instance; the legacy
`light` setting is not read; a refused order's reason is logged; the 2 s echo window is counted on
the wall clock.

- **What it does**: when motion is seen in its zone, it switches its lights on (`light_onoff`
  equipments of that zone); after the timeout (`10m` by default) with no motion, it switches them
  off. With a lux threshold, it does not switch on above it, and switches off above it plus 10 %.
  A light switched off by hand while motion lasts, or on by hand above the threshold, puts it in
  override until the zone has been still for the timeout; a light switched on by hand with no
  motion goes off after the timeout. The safety auto-off forces the lights off after its duration
  without fresh motion. A button press switches the lights off (override) or on. "Inactive during
  day" keeps the lights off, on motion and at the start, while the root zone's `isDaylight` is
  true (sunrise + offset to sunset − offset, from the home's settings); with no location, or the
  clock not set, it is `null`, read as night. At every start it sets the lights to what the zone calls for (off with no
  motion).
- **Routes**: `GET /api/v1/recipes`, `GET /api/v1/recipes/:id`; `GET`/`POST
  /api/v1/recipe-instances`, `PUT`/`DELETE /api/v1/recipe-instances/:id`, `POST .../enable`,
  `.../disable`, `.../actions`, `GET .../log?limit=`; texts and statuses as Sowel's (`Invalid
  params: <reason>`, `Recipe not found: <id>`, `Instance not found`, `Instance not running`,
  `Invalid body`).
- **Its log**: the last 20 lines of each instance, newest first, each message at most 200 bytes (`?limit=` over 20 gives the 20 kept);
  its state (`overrideMode`, `timerExpiresAt`, `failsafeExpiresAt`) is shown with the instance.
- **What survives a restart**: the instances, in `recipes.json` beside the other files, written
  within 2 seconds of a change; each enabled one is started again (one that no longer validates logs `Failed to restore:
  <reason>` and stays stopped). An instance whose recipe is not installed stays stopped with
  `Failed to restore: Recipe not found: motion-light`, keeps its settings, and starts as soon as
  the recipe is installed (or at the next start); an instance with only the legacy `light` setting
  stays stopped with `Failed to restore: At least one light is required`. Its log and state start
  empty. A `recipes.json` that cannot be
  read stops the host; an entry beyond 16, or with an id already seen, is dropped.
- **Validation texts**: a refused setting answers Sowel's 1.1.2 text, the script's own `validate`
  running before the host's check of the slots: `Zone parameter is required`, `At least one light
  is required`, `At most 16 lights`, `Light equipment not found: <id>`, `Light "<name>" has no
  "state" order binding`, `Invalid duration format: <v>. Use e.g. "10m", "30s", "1h"`, `luxThreshold
  must be a non-negative number`, `At most 16 buttons`, `Button equipment not found: <id>`, `Button
  "<name>" has no "action" data binding`, and, in Sowel's order, `Zone not found: <id>`, `Light "<name>"
  is type "<type>" but this recipe requires light_onoff` and `Light "<name>" does not belong to
  the selected zone`. A script's `validate` sees, for what the params name, `home.equipment(id)`
  (`name`, `type`, `zoneId`, the aliases of its `orders` and `data`) and `home.zone(id)` (`null`
  for a zone that does not exist); the host's texts come only for what the script lets through.
- **When what it uses changes** (decisions D38 and D81; Sowel keeps the stale ids): right after an
  equipment is created, changed or deleted, a data binding added or removed, an order binding
  removed, a device removed, or a zone updated through its route or deleted, the instances settle. A deleted equipment leaves the settings of every instance that names it (its
  id taken out of the lights and the buttons), with the
  warning `Equipment removed from settings: <name>` in its log, and the change is written. Every
  enabled instance is then checked as a creation is: one that no longer validates is stopped,
  disabled and written, with the error `Disabled: <reason>` (`At least one light is required`, a
  light no longer in the zone or without its `state` order binding...) and the events
  `recipe.instance.stopped` (when it ran) and `recipe.instance.error`; it sends no order after
  that. A running one whose lights lost an id and still validates starts again with them (its
  start line, its state reset). A button that no longer has an `action` data binding (its
  device removed, its binding deleted) disables the instance, `Disabled: Button "<name>" has no
  "action" data binding`; a button that no longer exists leaves the settings like any equipment.
  `POST` and `PUT /recipe-instances` refuse a button without an `action` binding. A running
  instance that is not restarted resolves its motion sources again at each settle (the enabled
  equipments of its zone and the zones below it), keeping its timers and state: a sensor deleted,
  unbound, disabled or moved out stops resetting the safety auto-off, one added or moved in
  starts to. An instance whose zone is deleted is deleted with its log
  (`recipe.instance.removed`). A renamed equipment, or a change nothing uses, leaves the instances
  untouched; a disabled instance is only pruned. `POST .../enable` on an instance whose settings
  no longer validate answers 400 `Invalid params: <reason>` and leaves it disabled.
- **From the interface**: the instances are made, changed, enabled, disabled and deleted, and
  their log read, from a room's page (see "Shows the home in the browser").
- **Bounds**: 4 instances of `motion-light`, all zones together (D85; the 16 instances of all
  recipes together stay the bound of the engine), 16 lights and 16 buttons each, settings of at most 2 KB and nested at
  most 8 levels deep (deeper: `Invalid params: params nested too deeply`), 128 events waiting for
  the recipes.

### Runs the switch-light recipe

`switch-light` is a script recipe, the port of Sowel's `sowel-recipe-switch-light` 1.0.0
(`recipes/scripts/switch-light.js`): Sowel's id, name, icon, settings, French texts, validation
texts and behaviour, version `1.0.0`, from the catalogue (or a signed file), or on the Mac through
`--recipe-script recipes/scripts/switch-light.js`.

- **Settings**: `zone`, `lights` (`light_onoff`, `light_dimmable` or `light_color` equipments of
  the zone, each with a `state` order binding), `buttons` (button equipments with an `action` data
  binding), and the optional `maxOnDuration`.
- **What it does**: each press of one of its buttons (an `action` event, spec 049: a remote's
  `toggle`, `on`, `off`...) switches its lights off when any is on, on when none is; each press is
  one toggle, two presses in a row toggle twice. A button that is not in its settings is ignored.
  With `maxOnDuration`, the lights it switched on, or that came on, are forced off after that
  duration (warning `Failsafe: lights forced off after <duration> max on duration`); a light
  switched off by any means with none left on cancels the countdown. The countdown is shown as the
  instance's `failsafeExpiresAt` state. Nothing is sent when the instance starts.
- **Validation texts** (Sowel's, in its order): `Zone parameter is required`, `Zone not found:
  <id>`, `At least one light is required`, `Light equipment not found: <id>`, `Light "<name>" does
  not belong to the selected zone`, `Light "<name>" has no "state" order binding`, `At least one
  button is required`, `Button equipment not found: <id>`, `Button "<name>" has no "action" data
  binding`, `Invalid duration format: <v>. Use e.g. "10m", "30s", "1h"`; a light of another type
  is refused by the slot's constraint.
- **Bounds**: those of any script recipe (4 instances, 32 KB of source, 224 KB of memory); 4
  instances of a script of 32 KB use about 242 KB on the Mac.

### Runs the motion-light-dimmable recipe

`motion-light-dimmable` is a script recipe, the port of Sowel's `sowel-recipe-motion-light-dimmable`
1.0.2 (`recipes/scripts/motion-light-dimmable.js`): Sowel's id, name, icon, settings (a slot may
carry Sowel's `group`, served in the info, with the French `groups`), French texts, validation
texts and behaviour, version `1.0.2`, from the catalogue (or a signed file), or on the Mac through
`--recipe-script recipes/scripts/motion-light-dimmable.js`. It is `motion-light`'s behaviour for
dimmable lights (`light_dimmable` or `light_color`), with a brightness.

- **Settings**: those of `motion-light` (`zone`, `lights`, `timeout`, `luxThreshold`,
  `maxOnDuration`, `buttons`, `disableWhenDaylight`), and `brightness` (1 to 254, the level when
  no slot matches) and three time slots `slotNStart`, `slotNEnd` (`HH:MM`) and `slotNBrightness`
  (all three or none).
- **What it does**: motion in its zone switches its lights on, then sets their brightness to that
  of the first slot that holds the time (a slot can pass midnight; `end` is excluded), else to
  `brightness`; with no `brightness` nothing but the switch-on is sent. The time is the home's
  (`now().localTime`, its time zone: the one adaptation of Sowel's behaviour, which reads the
  server's clock). The brightness is chosen when the lights are switched on, not while they stay
  on. The off timer, the lux threshold, the safety auto-off, the daylight rule, the buttons and
  the override are `motion-light`'s (the echo window is 5 s); a brightness reported with the
  lights on, other than the one sent and after the window, puts the instance in override. While
  the clock is not set (the box before its first sync) `now().localTime` is `null` and no slot
  holds the time: the plain `brightness` is used.
- **Validation texts** (Sowel's, in its order): those of `motion-light` for the zone, the lights
  (`... requires light_dimmable or light_color`), the duration, `luxThreshold` and the buttons,
  then `brightness must be between 1 and 254`, `Slot <n>: start, end, and brightness must all be
  provided or all omitted`, `slot<n>Start must be in HH:MM format`, `slot<n>End must be in HH:MM
  format`, `slot<n>Brightness must be between 1 and 254`.
- **Bounds**: those of any script recipe; 4 instances use about 218 KB on the Mac.

### Runs the schedule-on-off recipe

`schedule-on-off` is a script recipe, the port of Sowel's `sowel-recipe-schedule-on-off` 2.0.2
(`recipes/scripts/schedule-on-off.js`): Sowel's id, name, icon `CalendarClock`, settings (with their
`group`s `slot1` to `slot3` and `hiddenWhen`), French texts, validation texts and behaviour, version
`2.0.2`.

- **Settings**: `zone`, `equipments` (on/off equipments: `switch`, `light_onoff`, `light_dimmable`,
  `light_color`, `water_valve`, `pool_pump`, `water_heater`) and, for each of up to 3 windows N, a
  start and an end: `slotN_start_kind` / `slotN_end_kind` (`time`, `sunrise` or `sunset`), `slotN_*_time`
  (`HH:MM`, for a fixed time) and `slotN_*_offset` (minutes, -180 to 180, for the sun); window 1 is
  required, windows 2 and 3 take both ends or none.
- **What it does**: at the start of a window every equipment gets `state` `ON`, at its end `OFF`
  (a window may pass midnight); the instance's state holds `status` (`idle` or `running`),
  `currentSlot`, `nextStart` and `nextEnd`. Times are the home's (its time zone). A sun boundary uses
  the day's sunrise or sunset plus its offset, read again when it fires and at midnight; where the
  sun gives no time that day it is skipped, with a warning logged once. Equipments are named by the
  first 8 characters of their id in the log (a script does not read names).
- **Validation texts** (Sowel's, in its order): `Zone is required`, `At least one equipment is
  required`, `Slot 1 start and end are required (a fixed time, or sunrise/sunset)`, `Slot <n> start
  and end must both be set`.

### Runs the state-trigger-light recipe

`state-trigger-light` is a script recipe, the port of Sowel's `sowel-recipe-state-trigger-light`
0.2.3 (`recipes/scripts/state-trigger-light.js`): Sowel's id, name, icon `Bell`, settings, French
texts, validation texts and behaviour, version `0.2.3`.

- **Settings**: `zone`, `trigger` (an equipment of any zone, with a `state` data), `stateValue` (a
  text), `lights` (`light_onoff`, in the zone or its sub-zones), `duration` (default `5m`) and
  `nightOnly` (default true).
- **What it does**: when the trigger's `state` turns to exactly `stateValue` (after any other value:
  a re-report of the target does not fire), and it is night (no daylight, or the sun unknown) unless
  `nightOnly` is off, and no light is already on, the lights are switched on for `duration`, then
  off; a light switched off by hand (after the 2 s echo window) cancels the timer; the end time is
  kept in the state as `expiresAt` and an instance started again resumes it.
- **Validation texts** (Sowel's, in its order): `Zone is required`, `Zone not found`, `Trigger
  equipment is required`, `Trigger equipment not found`, `Trigger "<name>" has no "state" data
  binding`, `Target state value is required`, `At least one light is required`, `Light <id> not
  found`, `"<name>" is not a light_onoff equipment`, `Trigger equipment cannot also be a light
  controlled by this recipe`, the duration texts and `Duration must be > 0`.

### Runs the presence-thermostat recipe

`presence-thermostat` is a script recipe, the port of Sowel's `sowel-recipe-presence-thermostat`
1.2.0 (`recipes/scripts/presence-thermostat.js`): Sowel's id, name, icon `Thermometer`, settings
(with their `group`s `night`, `preheat` and `cocoon`), French and English texts, validation texts and
behaviour, version `1.2.0`. The `set_mode` action of Sowel's thermostat card does not exist (the
info has no `actions`; `POST .../actions` is ignored).

- **Settings**: `zone`, `thermostat` (a `thermostat` equipment of the zone with a `setpoint`
  order), `comfortTemp`, `ecoTemp`, `timeout` (default `30m`), and optionally a night window
  (`nightTemp`, `nightStart`, `nightEnd`, all three), a weekday preheat window (`preheatStart`,
  `preheatEnd`), a weekend one (`weekendPreheatStart`, `weekendPreheatEnd`) and the cocoon
  (`buttons`, `button` equipments with an `action` data, and `cocoonTemp`, both).
- **What it does**: the setpoint is `comfortTemp` while the zone has motion, `ecoTemp` after `timeout`
  with none; at the start it is set to what the zone calls for. A night window holds `nightTemp`
  whatever the presence and ends with comfort (preheat or motion) or eco. The weekday window (Monday
  to Friday) and the weekend window (Saturday, Sunday) hold comfort without motion, checked every
  minute; a button press holds `cocoonTemp` until the next press (it never times out). A setpoint
  changed by hand (neither the recipe's echo nor within 5 s of its order) suspends the recipe until
  the zone has been still for `timeout`, which ends with eco. The state holds `currentMode` (`eco`,
  `comfort`, `cocoon`, `night`), `overrideMode`, `cocoonMode` and `timerExpiresAt`. Hours and
  weekdays are the home's (`now().localTime`, `localWeekday`); while the clock is not set no night or
  preheat window is active.
- **Validation texts** (Sowel's, in its order): `Zone parameter is required`, `Zone not found:
  <id>`, `Thermostat parameter is required`, `Thermostat equipment not found: <id>`, `Thermostat
  "<name>" does not belong to the selected zone`, `Thermostat "<name>" has no "setpoint" order
  binding`, `comfortTemp is required` and `must be a number` (and `ecoTemp`), the duration text, the
  night, preheat and cocoon texts (`nightTemp requires nightStart and nightEnd`, `preheatStart and
  preheatEnd must both be provided or both omitted`, `cocoonTemp is required when buttons are
  configured`...), `Button equipment not found: <id>`, `Button "<name>" has no "action" data
  binding`.

### Runs the state-watch recipe

`state-watch` is a script recipe, the port of Sowel's `sowel-recipe-state-watch` 3.1.0
(`recipes/scripts/state-watch.js`): Sowel's id, name, icon `Eye`, settings, French texts, validation
texts and behaviour, version `3.1.0`.

- **Settings**: `zone`, `equipment` (of the zone), `dataKey` (a data alias of the equipment, a
  `data-key` slot), `watchValue` (a text, compared with the value as text: `true`, `open`),
  optionally `delay` and a check window `checkStart` / `checkEnd` (both or none, it may pass
  midnight, the home's time).
- **What it does**: while the data equals `watchValue` and the time is inside the window (always,
  with none), an alarm is raised once the `delay` has passed (at once with none): the state holds
  `alarm`, `alarmSince`, `watchStartedAt` (during the delay) and `summary`, a French line
  (`<key> : <value>`, with `après 10 min` and `de 21:00 à 06:00`, `... en alarme` while the
  alarm lasts; `Erreur A01` for a value `error_A01`). A value that leaves, or a window that closes,
  clears it. The window's edges re-evaluate. While the clock is not set the window counts as
  closed. The summary does not use the data's category (a script cannot read it): Sowel's labels
  by category (`Sur batterie`, `Ouvert`...) are not used.
- **Validation texts** (Sowel's, in its order): `Zone parameter is required`, `Zone not found:
  <id>`, `Equipment parameter is required`, `Equipment not found: <id>`, `Equipment "<name>" does
  not belong to the selected zone`, `Data key parameter is required`, `Equipment "<name>" has no
  data binding with alias "<key>"`, `Watch value parameter is required`, the duration text, `The
  check window needs both a start and an end time`, `Invalid start time: <v>. Use HH:MM (e.g.,
  "21:00")`, `Invalid end time: ...`.

### Runs script recipes

A recipe can also be a **script**: one JavaScript text in Sowel's script-recipe format (Sowel's
`roadmap/ai-script-recipes/spec.md`), run by QuickJS, a small JavaScript engine built into the host
and the box. To the API it is a recipe: it is listed by `GET /recipes` (in load
order), its instances are made, changed, enabled, disabled and deleted through the
same routes and from a room's page, with the same log, state and events.

- **Its form**: `recipe({ api: 1, id, name, description, slots, icon?, i18n?, version?, validate?,
  start })`, called once. `api` must be 1; `id` is 1 to 64 lower-case letters, digits and `-` (1 to 32 for an installed
  script), not one already loaded, and never `store` or `install` (`Script recipe invalid: id <id>
  is reserved`, kept from the days the store routes sat under `/recipes`). An
  installed script must declare `version` as `x.y.z` (three numbers of 0 to 65535: `Script recipe
  invalid: version must be x.y.z`); the version of a script given by `--recipe-script` stays free. The slots are Sowel's (`zone`, `equipment`, `number`, `duration`, `time`,
  `boolean`, `select`, `text` and `data-key`: a text of at most 64 bytes, `Invalid params: <slot> must be a text` or
  `<slot>: at most 64 bytes`; a `data-key` names a data of the equipment of the recipe's first single `equipment` slot, which the form offers and the script's `validate` checks), written as an object by slot id, each with Sowel's optional `group`
  and `hiddenWhen` (`{slot, equals}`, served in the info, used by the interface to hide a field); the slot `zone` is added first when missing.
  The settings are checked by the script's own `validate` first, if it has one, so that a refusal
  it makes carries its text (spec 046); then by the program from the slots (`Invalid params:
  <slot> is required`, `... must be a number between <min> and <max>`, `...: invalid duration`,
  `... must be one of ...`, `...: equipment not found`, `...: <name> is not a <type>`, `...: <name>
  is not in the zone`, `...: at most 16 equipments`). A `validate` that throws, or runs out of its
  200 ms, on settings of the wrong shape gives way to the program's check, whose text is the
  answer: a script never makes a setting pass that the slots refuse. A whole number in a slot's
  `min` or `max` is served without a decimal point (`0`, as Sowel's TypeScript recipes).
- **What a script may do**: `on(slot, alias, fn)` when a data of an equipment of a slot is updated;
  `onZone(alias, fn)` at each change of its zone's aggregate (`motion`, `luminosity`,
  `motionSensors`); `onMotionReport(fn)` at each report of a motion sensor of the zone, which
  Sowel's draft does not have; `get(slot, alias)`; `order(slot, alias, value)`, to every enabled
  equipment of the slot, answering the ones refused; timers `after` (at least 100 ms), `every` (at
  least 1 s), `at("HH:MM")` in the home's time zone, `cancel`, at most 16; `now()` (`time`, the home's time of day `localTime` as `HH:MM` in its time zone (UTC when it has none, `null` while the clock is not set) and its weekday `localWeekday` (0 = Sunday, as `Date.getDay()`, `null` then too), and the sun);
  `state.set/get/delete` (at most 512 bytes, served as the instance's `state`, kept in memory only: it goes when the instance stops, as Sowel's `stop()` clears it, and a script has no stop hook); `log`, `log.warn`,
  `log.error`, `console.log` (at most 20 lines per call). It names equipments only through its
  slots: a call with another name answers `Unknown slot`, and a script has no way to reach an
  equipment the user did not put in its settings. Instances of the same script recipe share its
  runtime and its handles, so they can reach each other: the isolation is between recipes (each has
  its own runtime) and from the equipments outside an instance's settings. An `equipment` or `zone`
  slot cannot have a default (`Script recipe invalid: slot <id>: a default is not allowed for type
  <type>`). A function of a script has no source text: `Function.prototype.toString` answers
  `[native code]` (a difference from Sowel). `mode`, `energy` and
  `history` answer `Not available in Solite`. Its top-level variables are shared by all its
  instances; what belongs to one instance lives inside `start`.
- **Where scripts come from**: on the Mac, `solite-host ... --serve --recipe-script <file.js>`, up to
  2 times, loads each script before the instances start and prints `recipes: script <id> <version>
  from <file> (<KB> KB of source, <KB> KB in its runtime)`; a file that is missing or refused
  (`Script recipe invalid: <reason>`), a third script, or the flag without `--serve` stop the host
  with exit status 2. On both, a script is **installed**: a signed file (below) kept in the data
  folder as `scripts/<id>.srs`, loaded at every start after any `--recipe-script` and before the
  instances start. No script is built into the box's radio image.
- **The signed file (`.srs`)**: a header of 128 bytes, then the script's UTF-8 source, the whole
  at most 32 KB of source and the header (32,896 bytes). Header: magic `SOLITERS`, format `1` (u32), the version (three
  u16), 2 reserved bytes at zero, the source's length (u32), its SHA-256, 8 reserved bytes at
  zero, and the Ed25519 signature (64 bytes) over the first 64. It is signed with the **recipe
  key**, not the firmware's, whose public half is built in (`SOLITE_RECIPE_PUBLIC_KEY`, from
  `release/recipe-key.pub`). Refused with 400, in this order: a length under 129 or over 32,896 bytes,
  the magic, the format (`format 2 is not supported`), the reserved bytes, the length field, the
  SHA-256, the signature (`verify_strict`), then the script's own checks; its `version` must equal
  the header's (`version differs from the signed one`). A build without a valid recipe key answers
  every install, update and refresh with 503 `Recipe installs are not configured` (an invalid key
  is logged `recipes: invalid recipe key, installs refused`). `solite-host --recipe-sign <file.js>
  --key <file> --out <file.srs>` signs (exit 2 for a script with no `x.y.z` version),
  `--recipe-verify <file.srs>` checks a file as the box does (exit 1 with the reason); the key
  pair is made with `--update-keygen`.
- **The catalogue**: `recipes.json`, unsigned (each file is signed and pinned by its SHA-256):
  `{"format": 1, "recipes": [{id, version, name, description, i18n?, icon?, size, sha256, url,
  minVersion?, repo?, category?, author?, tags?}]}`, at most 32 entries and 32 KB (`repo`: `owner/name`, at
  most 100 characters, unique, `solite-recipes/<id>` when absent; `category` and `author` at most
  64 characters, and a `category` outside Sowel's six (`lighting`, `climate`, `water`, `schedule`, `safety`, `energy`) is dropped; at most 8 tags of 32 characters), `size` at most 32,896 bytes; refused as a whole, the
  last good list kept, when over a bound, not JSON, of another format, an entry's `i18n` is not a JSON object, an entry's `repo` is malformed or repeated (`Invalid catalogue: <id>: repo`), or an entry's `url` breaks
  the update URL rule (https, or http on the home network). Its address is the setting
  `recipes.catalogue` (at most 512 characters, the same rule, 400 at the save; empty or unset
  means `https://github.com/mchacher/solite-recipes/releases/latest/download/recipes.json`). It is
  held in RAM only, as entries read once per catalogue read (each entry's `i18n` kept as its raw JSON text and served
  as it came, `groups` and `options` included; any other unknown field dropped), not parsed again at each request.
- **The install's steps in the log** (the box): each step of a recipe job logs `recipe install:
  <step> internal <free>/<lowest>`, the internal RAM free now and its lowest since boot (on the box; QEMU's figures are per-phase minima), in bytes; a step that fails logs its line too;
  the steps are `catalogue downloaded`, `catalogue read`, `download`, `check`, `load`, `store` (an
  install), `update load`, `update store` (an update), `removal unloaded`, `removal stored` (a
  removal). The engine's allocations of 512 bytes or less stay in internal RAM, larger ones are in
  PSRAM (`CONFIG_SPIRAM_MALLOC_ALWAYSINTERNAL=512`, D86).
- **Routes** (Sowel's `/plugins` routes, administrator, D89; the old `/recipes/store`,
  `/recipes/install`, `/recipes/:id/update` and `/recipes/:id/uninstall` are gone, `GET
  /recipes/store` meets `GET /recipes/:id` and answers 404 `Recipe not found`):
  `GET /api/v1/plugins` answers what the box holds, never the network: Sowel's `PluginInfo` of
  each script recipe running (the `--recipe-script` ones too): `{"manifest": {id, name, version,
  description, icon?, repo, "type": "recipe", category (`other` when the catalogue gives none), author?}, "enabled": true, "installedAt"
  (ISO 8601, `null` when the clock was not set at the install or the box restarted since),
  "status": "connected", "deviceCount": 0, "offlineDeviceCount": 0, "latestVersion"? (the held
  catalogue holds a newer one), "source": "registry"}`; a file refused at start is not listed (its
  reason is in `GET /system/resources`). `GET /api/v1/plugins/store` answers Sowel's bare list of the
  catalogue's entries that are neither installed nor built in: the manifest fields with `compatible`,
  `compatReason` (`Requires Solite >= <x.y.z>` when `minVersion` is above the running version),
  `isOfficial` true, `tier` `official`, `i18n?` served whole and `tags?`; `[]` before any read. `POST
  /api/v1/plugins/store/refresh` reads the catalogue again and answers `{"count", "source":
  "remote"}` (at most once a minute: 429; 503 `The clock is not set yet: reading the recipe
  catalogue needs the date`; a failed read keeps the list and answers `source: "local"`, the
  reason in the log and in `catalogue.error` of `GET /system/resources`). `POST
  /api/v1/plugins/install` with `{"repo": "owner/name", "confirmed"?, "expectedSha256"?}` (the
  catalogue's entry of that repository; the last two are ignored: the signature is the check) or an
  `application/octet-stream` body (a signed file of at most 32,896 bytes: 32 KB of source and its header), 200
  `{"success": true}`; a body with no valid `repo` is 400 `Invalid 'repo' field (expected
  owner/repo)`, a repository the catalogue lacks 404 `Plugin not found`, an installed one 409
  `Recipe already installed`. `POST /api/v1/plugins/:id/update` (the body is optional and ignored;
  the catalogue's version must be newer: 409 `<id> <x.y.z> is not newer than <x.y.z>`; an upload of a signed file
  of an installed recipe updates it the same way; an upload's id is read from the `id: "..."` text of its `recipe({...})` call without loading it, 400 when there is none, and the load refuses a script that declares another) and `POST /api/v1/plugins/:id/uninstall` answer 200 `{"success": true}`.
  Install is refused, in order: the build's key (503), the one-job rule (409, below), the internal
  RAM (503), a `repo` the catalogue lacks (404 `Plugin not found`), the id of a `--recipe-script`
  (409 `<id> is built in`), a loaded id (409 `Recipe already installed`), four script recipes
  loaded (409 `Too many script recipes (at most 4)`), a `minVersion` above the firmware (409 `<id>
  <x.y.z> needs Solite <minVersion> or later`), an unreachable catalogue or a failed download
  (502 with the reason; the download is resumed as an update's, at most three times), a size,
  SHA-256 or version that differs from the entry (400), the file's checks (400), the memory (409).
- **One job at a time**: an install, update or uninstall (a **recipe job**) and a catalogue refresh
  hold the release check's single session. A recipe job is refused with 409 while a recipe job
  (`Recipe install already in progress`), a refresh (`Recipe catalogue refresh already in
  progress`), an update check (`Update check already in progress`), a pull, an update received or
  installed (`Update already in progress`), an update installed and waiting (`An update is
  installed and waits for the restart`), a backup or a restore (`Backup already in progress`) or a
  restore waiting (`A restore waits for the restart`) is under way; a refresh while a recipe job, a
  refresh, a check or a pull. A check, a pull, an upload, a backup, a restore, a restart and a
  factory reset are refused while a recipe job runs (a restart and a check also during a refresh),
  each with its own text.
- **Install, update, uninstall**: an install loads the script, writes `scripts/<id>.srs` (one
  flash write) and starts the instances kept for that id (stopped by a restore or an
  image that carries no such script, with `Failed to restore: Recipe not found: <id>`); a write
  that fails unloads it again (500 `Could not save the recipe`). An update stops the instances,
  drops the old runtime, loads the new file and writes it over the old one, then starts the
  instances with their settings; a new version that does not load leaves the old one running (400
  with the reason); a write that fails gives the old one back (500); an old file that does not
  load again fails the recipe as a script that fails (500 with both reasons). An uninstall
  keeps the recipe's instances, as Sowel's (stopped, enabled or not as they were, `Recipe not
  found: <id>` in their log, still listed with their settings; an install of the same id starts
  the enabled ones again, as at start), drops the runtime and removes the file (a file that failed its checks too; 500 `Could not remove the recipe`), 404
  without a file, 409 `<id> is built in` for a `--recipe-script`.
- **At start** the files of `scripts/` are read in the order of their ids (a lone `<id>.srs.new`
  left by a power cut is promoted, a `.tmp` ignored) and go through the same checks; the enabled
  instances of the loaded recipes then start. A file refused (changed, format 2, a third file, a
  the id of a `--recipe-script`, not enough memory) is left in place, logged `recipes: script <id>: <reason>` and
  shown `refused` with its reason; an uninstall removes it, an install or an upload of that id
  replaces it. In the safe mode no file is loaded (`Safe mode: not loaded`); the uninstall goes
  through, install, update and refresh answer 503. The factory reset removes the installed
  recipes: the runtimes dropped, the files removed by the saver.
- **Admission by memory**: a recipe is admitted when the PSRAM free now, less a runtime's limit
  (224 KB), is at least 512 KB (409 `Not enough memory for <id>: <n> KB free, <m> KB needed`;
  checked at install, update and start); before a recipe job or a refresh starts its thread, the
  largest block of internal RAM must hold 16 KB for the TLS session (503 `Not enough internal
  memory: largest free block <n> KB, <m> KB needed`); on the box the `recipe` thread (20 KB) is
  made once at start, so an install, an update or a refresh needs only those 16 KB and an uninstall,
  which opens no TLS session, needs none; where a thread is made for each job (the QEMU run), the
  thread's 20 KB count too (36 KB, 20 KB for an uninstall). When the box's `recipe` thread could
  not be made at start (no 20 KB block), it logs it, and each recipe job runs on a thread made for it, the admission then counting its 20 KB (36 KB for an install, update or refresh, 20 KB for an uninstall). The Mac reads no memory: its admission always passes.
- **Where it runs on the box**: the download, the checks, the write and the removal run on the
  `recipe` thread, made once at start, whose stack is in internal RAM (the flash is never touched
  from a task whose stack is in PSRAM), one job at a time; the load and unload of a runtime on the
  `script` thread, whose stack is in PSRAM (nothing it runs touches the flash). The scripts are not
  in the full backup: a restore on the same box leaves the files (the restored instances start),
  on another box the restored instances stay stopped until the recipe is installed.
- **Bounds**: 4 scripts of at most 32 KB, at most 4 instances of each (a fifth is refused: 400 `Too
  many instances of <recipe id> (max 4)`; on restore the fifth and later fail to restore with that
  text and stay stopped); each runs in its own QuickJS runtime of at most 224 KB on the box (448 KB
  on the Mac, whose pointers are twice as large), shared by its instances; loading a script stops
  after 1000 ms (`Script recipe invalid: the script took more than 1000 ms when loaded`); a call
  into a script stops after 200 ms, its stack at 16 KB; no event or timer goes to a script once
  300 ms of a turn of the engine's loop are spent on scripts (the one under way finishes: one call
  per instance, or per equipment holding a reporting motion sensor, each of at most 200 ms), the rest
  waiting for the next turn; a failed script's restart, one recipe per turn, counts in those
  300 ms, and its load may take up to its 1000 ms limit (that turn is longer). A call that leaves
  the runtime over 85 % of its memory limit, or 32 KB more than the last collection left, is
  followed by a garbage collection. The engine checks the end of each call (the state is a JSON
  object of at most 512 bytes, at most 16 timers, aliases of at most 64 bytes, at most 21 log lines):
  a script that makes it otherwise fails the recipe. A fifth instance never runs, whichever way it
  is started (a restart after a failure leaves it stopped, `Failed to restart: Too many instances
  of <recipe id> (max 4)`). On the box the scripts' memory is in PSRAM and every call into a script
  runs on a stack of 32 KB: the engine thread's, in internal RAM, or the `script` thread's, made at
  boot, in PSRAM, for the API's requests that change something, but the account's routes
  `/api/v1/auth/*`, which stay on the worker. The box's storage refuses a flash access made from a
  PSRAM stack (`flash access from a PSRAM stack refused`, logged once per boot) instead of letting
  the chip stop.
- **When a script fails** (a call that throws): `Script error: <message>` in the instance's log, and
  the instance runs on (`Failed to start: Script error: ...` if it was the start); a throw inside a
  promise's `then` that no `catch` handled is `Script error: <message>` too, and a stack overflow
  there fails the recipe. **When a script
  recipe fails** (a call over its time, its memory or its stack): its running instances stop with
  `Script recipe failed: <reason>` and the events `recipe.instance.stopped` and
  `recipe.instance.error`; they stay enabled and are not written, the rest runs on, and a settle
  leaves them alone. A minute later the recipe is started again with its enabled instances (`Script
  recipe restarted (<n> of 3 in 24 h)`), at most 3 times in 24 hours; then it waits for `POST
  .../enable` on one of its instances, a create or change of one, or a restart, which start all its
  enabled instances (a recipe that fails again at once answers 500 `Script recipe failed:
  <reason>`).
- **On the box**: the radio image logs `recipes: script <id> ...` for each installed file at start and, with each `heap:`
  line, `scripts: <id> <n> instances, <n> calls, average <µs> µs, max <µs> µs, heap <KB> KB (peak
  <KB> KB)`.

### Shows its resources

`GET /api/v1/system/resources` (a Solite route, administrator) answers `{"internal": {"free",
"minFree", "largestBlock"} | null, "psram": {"free", "minFree", "total", "reserve"} | null,
"storage": {"used", "total" | null}, "scripts": [{"id", "version" | null, "source": "builtIn" |
"installed", "state": "ready" | "failed" | "refused", "reason" | null, "held", "limit",
"instances", "calls", "averageUs", "maxUs"}], "catalogue": {"readAt" (ISO 8601) | null, "error" |
null}}`, bytes and microseconds as numbers; `catalogue` is when the recipe catalogue was last read
and why the last read failed (they were in the store's answer before D89). `builtIn` is a
script given by `--recipe-script`; a refused file has `held` 0. The Mac answers `internal` and
`psram` `null` and `storage.total` `null` (`used`: the files of its data folder). The storage
figures are never read by the request (a worker's stack is in PSRAM and must not touch the flash):
they are read at start and after each write or removal, by the thread that did it, and kept.
Settings has a "Resources" card (the two memories as a bar each, with the lowest free and the PSRAM
reserve marked, the storage, a row per script recipe); the Plugins page of Administration lists
the recipes (see "Shows the home in the browser").

### Shows the home in the browser

With `--serve`, `http://127.0.0.1:<port>/` opens the interface (built by `npm run build` in
`ui/`, served from `ui/dist`, or from the folder given by `--ui <folder>`). Without it, the API
still answers, `/` answers 404 `Interface not built: run npm run build in ui/`, and the host
prints `interface: <folder> not found, run npm run build in ui/`. `cd ui && npm run dev` serves
it on `http://localhost:5173` with the API of a host running on port 8080. The interface pairs
the devices (Discovery), makes the rooms (Topology) and the equipments (Equipments, a room's "Add
equipment"), sets the recipes (a room's Recipes panel) and the home's location and time zone (Settings), as well
as showing them. It also shows an equipment's page (its values, devices and bindings, completed or
changed one by one, enabled or disabled) and a device's page (its data and orders). The interface also opens at
`http://<box address>/` or `http://solite-xxxxxx/` (when the router resolves DHCP names), from the
copy embedded in the radio image at build time.

- **First start and login**: the interface first reads `GET /auth/status`. With no account it shows
  the **first-start screen** (Sowel's setup page): the Solite mark, "Set up your Solite", user
  name, display name, password and confirmation; "Passwords do not match" and "At least 8
  characters" (8 to 128 characters, counted as the host counts them) are checked before sending;
  the host's refusal (for example the confirmation at the box) shows under the form; on success the
  home shows. With an account and no stored token it shows the **login screen** (user name,
  password, "Log in"; "Invalid credentials" or the host's text on a refusal). The tokens are kept in
  `localStorage` (`sowel_access_token`, `sowel_refresh_token`); every request carries `Authorization:
  Bearer <access>`; a 401 asks `POST /auth/refresh` once (one at a time) and repeats the request, a
  refresh refused clears the tokens and shows the login screen, a 403 `setupRequired` shows the
  first-start screen. `/ws` offers `bearer.none` and `bearer.<access>`; a close 4001 refreshes the
  token and reconnects.
- **Sidebar**: on a phone, behind the top bar's menu button, in a drawer that closes when a room is
  picked; the Solite mark (Sowel's smiling house with an antenna) and the wordmark "solite" above the rooms as a tree, which scrolls alone, then, pinned at the bottom of the column under a line, the Administration section: Devices, Equipments,
  Topology, Discovery, Plugins (with the count of installed recipes the catalogue has newer, as a badge), Settings; open on an administration page (also on an equipment's or a device's page, Equipments or Devices marked), opened or closed by its title; under it, **Log out** (`POST /auth/logout`, the tokens cleared, the login screen); the room shown is kept in the address
  (`#/zone/<id>`); at first the first room with equipments. The Solite mark is in the sidebar and the drawer, not in the phone's top bar.
- **Top bar**: on every page of the home, the home's name (the Settings' name, "Home" when none, a
  grey placeholder while it loads) and where you are (the room's path, or the administration
  page); on the right the home's time when the browser sits in another time zone, the sunrise and
  sunset of the root zone with a sun by day and a moon by night ("Set the location", a link to
  Settings, when no location is set; nothing while the box's clock is not set), and the link
  indicator: nothing while connected and for 3 s after, then "Connecting..." or "Disconnected". On
  a phone the bar holds the menu button, the home's name with the page under it, and the
  indicators as icons. The name and the time zone are read again when the connection opens and
  after Settings saves.
- **The room**: its name; Sowel's pills (temperature, humidity, luminosity, motion) and lights
  line ("1 light on"); its equipments in Sowel's order: a light with its power button, a sensor
  with its values, any other equipment with its name, icon and status badge; a disabled one
  dimmed without control; the equipment's name opens its page (the light's button and the values
  keep working as before); an "Add equipment" button opens the equipment form with the room chosen.
  "No room yet: create one in the Topology page" when only the home's root exists and it holds no equipment; "No equipment
  in this room".
- **The room's recipes**: under its equipments, the **Recipes** panel with the number of instances whose zone is the room and a "+" that lists the recipes
  (name, description); picking one opens the form under the panel. Each instance is a row: the
  recipe's icon and name (French from the recipe's `i18n.fr`), the names of its lights (three,
  then `+N`), a timer pill counting down to the off timer and an "Override" pill while enabled,
  and a switch that enables or disables it (dimmed when disabled; a refusal shows its text for
  5 s). A click on its name opens its journal: the log newest first, `HH:MM` and message, errors
  and warnings in their colours, read again every 5 s while open ("No entries"; "The journal could
  not be read" when the first read fails), with a **Settings** button (none for a recipe the
  host does not know). Settings opens the form in a dialog, filled from the instance, with Save
  and Delete (confirmed). "No recipe in <room>" and an "Add a recipe" link when the room has none. With no recipe installed on the host the panel
  has no "+" and says "No recipe is installed" with a link to the Plugins page of Administration.
  An instance whose recipe id is absent from the recipe list (a home restored on another box, or
  saved with a recipe since uninstalled) shows itself as it is, in the room whether or not another
  recipe is installed: named by its recipe id, dimmed, its switch off and disabled (it cannot
  run), a "Recipe not installed" label and a Delete (confirmed) in the row.
  **The form** is built from the recipe's slots, in their order, each with its name (`*` when
  required) and description: the zone is the room; an equipment list is a checkbox per equipment
  of the room of the slot's type; a duration is a number of minutes ("min"), sent as `"<n>m"`; a
  number, a checkbox for a boolean, a select for `select`, a text field otherwise; defaults from
  the slots (the timeout 10). As Sowel's, the lux threshold shows only when the room has a
  luminosity, the buttons only when the home has a button equipment. A required slot left empty
  shows "<slot> is required" and sends nothing; a refusal shows the host's text under the form,
  which stays open. Every `recipe.instance.*` event reads the instances again (a burst read once,
  then once more after 1 s).
- **Devices**: `#/devices`: every device by name, with its status, name (editable), manufacturer,
  model and Equipment column ("Room › Equipment" for each equipment that binds it, the room a link
  to its page, the equipment's name a link to its own page; "—" when none); delete (confirmed; a refusal shows its text for 5 s); "Add devices" opens
  Discovery. A click on a row opens the device's page, except on the name editor, delete and the
  links, which keep their action. A device has no room select.
- **Device page** (`#/devices/<id>`): a back link "Devices"; a radio icon, the name (editable, as
  on the list) and the status badge (online, offline, unknown), read like the list's, so a rename
  and a status change show at once. The information: manufacturer and model, the source device id,
  the IEEE address, when it was last seen ("never" when it was not) and when it was created; a
  field the device does not give is left out. The **Data**: key, category, value with its unit ("—"
  when none) and when it was updated, sorted by category then key ("No data" when empty); the
  **Orders** (only when the device has some): key, type and range (min to max with the unit, or
  the values of an enumeration). It follows `device.data.updated` live, and a
  `device.removed` for it shows "Device not found". **Delete** (confirmed; the device stays
  in the Zigbee network) goes back to the list; a refusal (for example "Device is being paired")
  shows its text. An unknown device shows "Device not found" with the back link; another failure
  shows its text and "Retry". A malformed or empty id (`#/devices/%`, `#/devices/`) shows the
  list.
- **Discovery** (`#/discovery/<integration>`, Zigbee's is `#/discovery/solite`): the integration's
  name as a heading (Zigbee), a tab bar only past one integration; "Add a device" opens pairing for 180 s (a countdown from the route's
  `remaining`, and "Stop"); the devices discovered while the screen is open appear first, marked
  "New", as rows of the Devices list; leaving the screen empties it. It follows
  `device.discovered`, `device.removed` and `device.status_changed` live.
- **Topology** (`#/zones`): the tree from `Maison`; add a zone (under the root) or a sub-zone; edit
  its name, parent (never under itself or a descendant) and description; delete it (confirmed;
  refused with Sowel's text when it has sub-zones or equipments, shown 5 s). The root is renamed but never deleted or moved.
- **Equipments** (`#/equipments`): grouped by room, each with its name (a link to the equipment's
  page), type, status and number of devices; "Add equipment" in two steps: type (light on/off,
  dimmable light, colour light, shutter, sensor, switch, thermostat, button), name and room, then
  the devices (those ticked first, then the compatible ones, all on demand); none is needed: with
  none ticked, "Create" makes the equipment without bindings and its page opens, the hint under the
  list saying devices can be added later from that page; devices ticked that give nothing the type
  uses still block "Create". Only what the type uses is bound, under Sowel's names
  (`state`, `brightness`, `position`, `battery`; `battery_2` for a second one), one request per
  binding after the equipment's; if one is refused the equipment is deleted again and the form
  shows the text; edit the name and room; delete (confirmed). An equipment of a type the form does not
  offer (made through the API, `door`...) keeps its type when edited, shown as written; the room is chosen there. A refusal keeps the form open with its
  text. At most 64 devices chosen; a warning when they give more than
  8 values, of which only 8 are bound.
- **Equipment page** (`#/equipments/<id>`): a back link to the equipment's room ("Equipments" when
  its room is unknown); the header: the type's icon, the name, the status badge (with its reason),
  and under it the type, the room and "Disabled" when it is, with an **Edit** button (name and
  room, the type locked). **Controls**, only while it is enabled: a light, plug or switch whose
  on/off order is bound has its power button (`ON`/`OFF`; a refusal shows its text for 5 s); an
  equipment with sensor data shows the values the room's row shows, also when disabled. The
  **Configuration** card, in this order:
  - **Activation**: "Enabled" or "Disabled", and, opened, a switch that sends `enabled` to the host
    and waits for its answer; a refusal shows the host's text in the row and the switch goes
    back.
  - **Devices**: closed at first, the number of devices bound; opened, each with its name (a link
    to its page), when it was last seen ("never" when it was not) and "<n> data · <m> orders"
    bound to it; "No device" when none. **Change devices** (or "Add" with none) opens a dialog with
    the creation form's device list for the equipment's type, nothing ticked at first: "Save"
    removes every binding, then writes what the type uses from the devices ticked (under Sowel's
    names, at most 8 of each kind), one request after the other. The first refusal stops it and
    shows the host's text; what was written so far stays (the bindings list shows it). "Save" stays
    disabled with no device ticked or when the devices give nothing the type uses; for a type with
    no binding rules (one of Sowel's 23 others) it says "This type is completed binding by
    binding".
  - **Bindings**: closed at first, the total count; opened, the data bindings (alias, category,
    device, key, value with its unit, "—" when none) and the order bindings (alias, type, device,
    key), each list with an "Add" button and, on each row, a remove button (on hover, always on a
    touch screen) that removes it at once, without confirmation; a refusal shows its text for 5 s
    under the list; "None" for an empty list. "Add" opens a dialog: the devices, then the device's
    data (or orders) with their key, category or type and value; a click picks one and fills the
    alias with Sowel's name for it (`state`, `brightness`, `battery`...), editable. "Add" sends it
    with the alias trimmed; nothing is sent for an empty alias, an alias the equipment already
    uses in that kind ("already in use") or one over 32 characters ("At most 32 characters"); the
    host's refusal (the 17th binding, a device item gone) shows in the dialog, which stays open.
    When the devices bound give data or orders the type uses and the equipment does not bind
    (read only once the section is opened, one device after the other, a device that cannot be
    read skipped), a banner "<n> new data points available" has a "Review" button: a dialog lists
    them all ticked (alias, data or order, device, key), "Add <n>" writes the ticked ones one after
    the other, the first refusal stops it and shows its text, and what was written leaves the list.
    Removing a binding a recipe needs disables that recipe (the core's rule: a motion light whose
    light loses its `state` order binding is disabled), and putting the binding back does not
    enable it again: it is enabled again from the room's Recipes panel.
  - **Delete equipment** (confirmed): goes back to the list; a refusal shows its text.

  An id the equipments do not hold shows "Equipment not found" with the back link (nothing before
  the first read; when the equipments cannot be read, the error and "Retry"); an equipment deleted
  while its page is open shows the same. Values, status, bindings and the enabled state follow the
  equipment events. A malformed or empty id (`#/equipments/%`, `#/equipments/`) shows the list.
  The dialogs close on Escape and on a click outside.
- **Settings** (`#/settings`): the **Home** card (Sowel's): name, latitude, longitude (a typed `,`
  becomes `.`), sunrise and sunset offsets, and the time zone, picked in the list (`UTC` when
  unset); Save shows when something changed and sends the whole settings map, read just before
  (`GET /api/v1/settings`) with its own six keys changed, so no other key is dropped (every save of
  the page does the same, D71); a refusal shows the host's text. Under it, **Today**: sunrise, sunset and "Day" or "Night", live from the root zone; "Set
  the location to get the sun times" when unknown. The **Backup** card: under "Export", a password and its confirmation (8 to 128 characters, the two
  equal; else the button stays disabled and the reason shows under it) and an "Export" button
  ("Exporting..." while it runs, a few seconds on the box), which downloads
  `solite-backup-<date>.slb` or shows the host's refusal; under "Restore", a file input (`.slb`),
  the backup's password, a checkbox "Also restore the Wi-Fi settings" and a "Restore" button,
  disabled until a file and a password are given, which opens a confirmation dialog (the devices,
  rooms, equipments, recipes, settings and the Zigbee network are replaced, the account is kept,
  the box restarts; one more line when the Wi-Fi box is ticked), then "Restoring..." during the
  upload, and the answer's message or the host's refusal ("Wrong backup password", the
  confirmation's hint). At the foot of the page, the **Factory reset** card: what is erased, a
  "Factory reset" button and a confirmation dialog (what is erased, "Erase everything"); success
  clears the tokens and shows the first-start screen, a refusal (the confirmation's text, "Pairing in
  progress") shows the host's text. Between the Backup and Factory reset cards, the **Firmware
  update** card (Sowel's version badge and update overlay, inside the card): "Running version
  x.y.z" (`GET /api/v1/system/version`) and, under it, one of four lines from `latest` and
  `updateAvailable`: "No check since the box started"; "Version x.y.z is available", with a
  "Release notes" link to the release page (`releaseUrl`, a new tab, absent when null) and an
  "Update to vx.y.z" button; "Version x.y.z cannot be installed directly from this version: see
  its release notes" (its `minVersion` is above the running one, no button); "Up to date (latest
  x.y.z)". The versions are compared as three numbers. "Check for updates" posts
  `/api/v1/system/version/check` with a spinner and refreshes the card; every refusal (429, 503,
  500, 409) shows the server's text under the buttons. The release notes are not in the API. "Update
  to vx.y.z" opens a confirmation (the box downloads and installs, restarts in about a minute, goes
  back by itself if the new version does not start, the user logs in again; Cancel sends nothing)
  and Update posts `/api/v1/system/update` with no body. While it installs the buttons are
  disabled and "Downloading and installing... <n> s" counts the seconds. Then the card polls `GET
  /api/v1/system/version` every 3 s (never two at once, each with a 5 s timeout, stopped when the
  card unmounts): after the 200 "Installed x.y.z, the box restarts...", a `current` other than the
  one before gives "Now running x.y.z"; the restart ends the sessions, so the next answer is a 401
  and the login screen shows, and the new version shows after logging in. At 3 minutes the card
  says "The box still runs x.y.z: the new version was not kept (see the box's log)" when the last
  poll answered with the same version, else "The box has not answered for 3 minutes: reload the
  page". A network error during the request itself ("The connection was lost; waiting for the
  box...") polls for up to 11 minutes (the 600 s download and a restart): "Now running x.y.z", or
  at the end "The update did not complete: the box still runs x.y.z" or the reload text. Any other
  refusal (400 `No update available`, `Version ... needs ...`, `Download does not match the
  manifest`, 409, 413, 429, 500 `Download failed: <reason>`, 503) shows the server's text and the
  card returns to its state. A checkbox "Check for a new version once a day" is `update.autoCheck`,
  saved at once; under "Advanced", a field for `update.manifestUrl` (the default GitHub address as
  placeholder, `""` returns to it) with its Save; the refusals show. Under its own heading,
  "Install from a file": a file input (`.sup`), an "Install" button, disabled until a file is
  chosen and during the upload ("Installing..."), then the answer's `message` (success token) or
  `error` (danger token) under it; after a success, "The box restarts; reload the page in a
  minute." The upload is disabled during a pull or a check, and they are during an upload. The file
  goes as the body, `Content-Type: application/octet-stream`, with the bearer token. French and English labels.
  The **Resources** card (Settings, after the Restart card) shows `GET /api/v1/system/resources` (the internal RAM and the PSRAM as a bar each, with the
  lowest free and the PSRAM reserve marked, the storage used, when the recipe catalogue was read and why a read failed, one row per script recipe with its
  version, source, memory held against its limit, instances and call times; a row hidden when its
  figure is `null`). French and English.
  The **Plugins** page (`#/plugins`, Administration, after Discovery; Sowel's page cut to script
  recipes, with no category filter): two tabs with their counts, **Installed** (each installed
  recipe of `GET /api/v1/plugins` with its name and version, Update when it names a `latestVersion`, Remove (two
  clicks: "Confirm?"; the recipe's instances stay, stopped, as Sowel's uninstall keeps them; no Remove for a recipe of the host's own `--recipe-script`, read from the resources' `builtIn` rows; the
  files refused at start with their reason, read from the resources, with Remove) and **Store**
  (`GET /api/v1/plugins/store`, with Install by the entry's `repo`, disabled with the entry's `compatReason` when it is `compatible: false`; the refusal of a file of that id
  when there is one; the error of the last read, from the resources), a search field over the id, name and description of the tab shown; it offers no upload of a signed
  file (that stays the API's route, `POST /api/v1/plugins/install`, for the developer). It asks the refresh when it
  opens and shows the list held when that answers 429; the server's own words show for a refusal.
  French and English.
  The **Restart** card (after the Firmware update card): a "Restart" button, a confirmation ("Restart
  the box? It is back in about a minute."; Cancel sends nothing), then `POST /api/v1/system/restart`
  and a message while the box is away; the page polls `GET /api/v1/system/version` every 2 s (never
  two at once) and reloads once it answers again after being away, a 401 included (the session ends with the restart, the
  reload shows the login). After 90 s without an answer, a message asks to reload by hand. A refusal
  (409, 501, ...) shows the server's text and the button is enabled again. French and English.
- **Live**: values, motion and lights change as the events arrive (spec 013's `/ws`); when the
  connection is not open for 3 s (also at the first load) the top bar's link indicator says so:
  "Connecting..." while it tries, "Disconnected" between tries; the interface reconnects (1 s, 2 s, 4 s... up to
  30 s) and reloads everything when it is back.
- **A light's button** sends the order (`ON` or `OFF`); the button waits, and the light shows its
  new state when the device confirms it; a refusal shows its text for 5 s.
- **Language**: French when the browser is in French, English otherwise (the first-start and login screens included).
- The pages carry headers that forbid showing them inside another site's page.

### Runs without the dongle

`--sim <scenario>` replaces `--radio tcp://<address>:6638` in every command (identify, `--pair`,
`--listen`, `--serve`): a simulated radio and simulated devices, speaking the same protocols as the real
ones, play a scenario file; the lines are the same, `radio: sim <scenario>` first. The simulated
radio runs on the Mac, in the emulated ESP32, and in the measuring image, which is built for the Dongle Max.
`--speed <1..100>` runs the scenario's clock faster (the pairing time is not affected).

A scenario (JSON, at most 1 MB) names the radio (EZSP version 13 to 19, stack version, address,
`"network": "none"` or `"formed"`), up to 64 devices and up to 10 000 events: `join` (as soon as
pairing is open) and `report` (measures by Sowel category, allowed per kind), at times such as
`40s` or `1m30s`. Its errors name the file and the line, or the device or event and its field.

| Kind | Identity sent | Sends |
| ---- | ------------- | ----- |
| `temperature-humidity` | eWeLink TH01 (Sonoff SNZB-02) | temperature, humidity, battery; asleep except 10 s after joining or reporting |
| `motion` | eWeLink MS01 (Sonoff SNZB-03) | enrolled at pairing; then `alarm true` on motion, and `alarm false` 60 s of scenario time later |
| `relay` | SONOFF ZBMINIR2 | `light_state on` / `off` after an On, Off or Toggle command |
| `color-light` | TEST CLR01 (synthetic colour bulb, mains) | `light_state`, `light_brightness`, `light_color_temp`, `color_x`/`color_y` (and `color_hue`/`color_saturation` when read) after its commands (on/off, level, colour temperature, XY, hue and saturation, applied even when off); a report takes `light_state`, `color_temp` (150 to 500) and `color` (`{"x", "y"}`, 0 to 1, one message) |
| `button` | TEST BTN01 (synthetic remote, battery, output clusters On/Off and Level) | once bound, the command a report's `action` names (`on`, `off`, `toggle`, `brightness_move_up`, `brightness_move_down`), the same one twice if the scenario says so |
| `shutter` | TEST SHT01 (synthetic shutter, mains) | answers Up, Down, Stop and Go To Lift Percentage by moving 10 points a second towards its target (1 s of scenario time, shortened by `--speed`), reporting each step; starts closed; a report takes `position` (0 to 100, 100 open) and puts it there at once |
| `thermostat` | TEST THM01 (synthetic thermostat, mains) | answers the read of its temperature and setpoint and the Write Attributes of the setpoint (the write is answered, then reported); a report takes `temperature` (the measured one) and `setpoint` (5 to 30 °C) |
| `plug` | TEST PLG01 (synthetic metering plug, mains) | `light_state` after its commands; answers the reads of the scale (power divisor 10, current and energy divisor 1000, multipliers 1); a report takes `light_state`, `power` (W), `current` (A) and `energy` (kWh) |

The simulated radio keeps its network and its devices in `solite-data/sim/` (or `--data`),
apart from the real dongle's backup, so a second run resumes. Shipped scenarios:
`scenarios/snzb02.json` (one sensor), `scenarios/garage.json` (a sensor, a motion sensor, a
relay), `scenarios/color-light.json` (one colour bulb), `scenarios/button.json` (one remote), `scenarios/button-light.json` (a remote and a relay, for `switch-light`), `scenarios/plug.json` (one metering plug) `scenarios/shutter.json` (one shutter) and `scenarios/thermostat.json` (one thermostat).

`SOLITE_PASSWORD=<password> sh scripts/demo-garage.sh [<port> | <address>[:<port>]]` sets the garage up through the API (the two scripts read the password from `SOLITE_PASSWORD`, never from an argument, and the user name from `SOLITE_USER`, `admin` when unset: they ask `/auth/status`, set the account up when it is required (display name = user name), else log in, and send the token on every request; without `SOLITE_PASSWORD`: `<script>: set SOLITE_PASSWORD (the password of the box)`, exit 2; a refused login or setup prints the host's text, exit 1; the password is never printed): a
number alone is a port on `127.0.0.1`, anything else an address (port 80 by default); nothing
listening: `demo-garage: no host answers at <address>:<port> ...`, exit 1.
`SOLITE_PASSWORD=<password> sh scripts/measure-api.sh <address>[:<port>] [--keep]` sends, one after the other, the 17 requests
of an equipment with 8 data and 8 order bindings (the same data and order under 8 aliases),
times each one (its login is not counted), and prints `measure: 17 requests, <k> answered 201, <b> Busy, total <s> s, slowest
<t> s (request <i>)`; it exits 0 only when the 17 answered 201, none was 503 and the total is at
most 10 s, else 1 with the first answer that was not 201. It needs a device with an order (`no
device with an order: pair a relay first`, exit 1), refuses when a zone `Measure 33` exists, and
deletes its zone and equipment afterwards (kept with `--keep`). It is run on the Mac against the
simulator (`--sim scenarios/garage.json --serve`, then `SOLITE_PASSWORD=<password> sh scripts/measure-api.sh 8080`); against
the box at most once per milestone, inside `box.sh saves` (decision D39), and no script loops it.

### Updates itself from a signed file

A box is updated from a **signed update file** (`.sup`, "Solite update package") sent to Sowel's
`POST /api/v1/system/update`, or pulled by the box itself from the latest release (see "From a
release"). The file is a header of exactly 128 bytes, then the image (the ESP32
application image of the slot, as `esptool elf2image` writes it, the same as `scripts/box.sh` and `scripts/release.sh` make). Integers are little-endian.

| Offset | Bytes | Field |
| ------ | ----- | ----- |
| 0 | 8 | magic, ASCII `SOLITEUP` |
| 8 | 4 | format, `1` |
| 12, 14, 16 | 2 each | version: major, minor, patch |
| 18 | 2 | reserved, 0 |
| 20 | 4 | image length in bytes, at least 1 |
| 24 | 32 | SHA-256 of the image |
| 56 | 8 | reserved, 0 |
| 64 | 64 | Ed25519 signature (RFC 8032) over bytes 0 to 63 |

- **The key.** The signature is checked against a public key given at build time, in the
  environment variable `SOLITE_UPDATE_PUBLIC_KEY` (64 hexadecimal characters, either case), read by
  the host and the firmware; the core takes it as a value. No private key, seed or test key is in
  the repository. A build without the variable has no update key: the route answers 503 `Updates are
  not configured: this build has no update key` and the start logs `update: no key, updates
  refused`. A value that is not 64 hex characters, not an Ed25519 point or a weak key is the same
  case, logged `update: invalid key, updates refused`. With a key, the start logs `update: key
  <first 8 hex characters>`.
- **The checks, before anything is written**, in this order: magic (400 `Not a Solite update`),
  format (400 `Unsupported update format <n>`), reserved bytes zero and length at least 1 (400
  `Malformed update header`), signature (400 `Invalid signature`; a non-canonical signature is
  refused), version strictly newer than the running one, compared as (major, minor, patch) (400
  `Version <x.y.z> is not newer than <running>`), image length within the slot (413 `Image too
  large for the update slot`; the slot holds 4 MB). A file shorter than 128 bytes is 400
  `Truncated update`. No downgrade goes through the API: an equal or older version is refused, a
  downgrade is a flash.
- **Streaming.** The image is never held whole. It is read at most 4096 bytes at a time into one
  buffer, hashed and written to the inactive slot, so the memory of an installation is the header,
  that buffer and the hash state, whatever the image's size (1 MB peaks under 32 KB of heap in the
  tests). A source that ends early or fails: 400 `Truncated update`; one more byte than declared:
  400 `Update longer than declared`; a SHA-256 that differs from the header's: 400 `Image does not
  match its signature`; a write or finish that fails (the platform finds the image invalid, for
  instance): 500 `Update failed: <message>`, logged `update: failed: <message>`. Each of these
  drops what was written: the slot is not bootable and the boot record is unchanged, so a power
  cut at any point before the end leaves the running image as the one that boots. Only when the
  whole image matches does the slot become the next boot, on probation; the answer is 200
  `{"success": true, "message": "Installed <x.y.z>, restarting"}`, logged `update: <x.y.z>
  installed, <KB> KB`.
- **One at a time.** An installation in progress: 409 `Update already in progress`; an update
  installed and waiting for the restart: 409 `An update is installed and waits for the restart`;
  the running image still on probation: 409 `The running image is not confirmed yet` (the inactive
  slot holds the image to fall back to). No key: 503. A recipe job (see "Runs script recipes") or
  a catalogue refresh shares the session: a check is refused while either runs (`Update check
  already in progress`), a pull (`Update already in progress`), and an upload while a recipe job
  runs (`Update already in progress`).
- **Probation.** A newly booted image is on probation. It is confirmed (the running image is kept
  from then on, logged `update: running image confirmed`) once it has run healthy for 60 s without
  a break; a break restarts the minute. Healthy means the engine's loop is running, the API
  server is listening and the PSRAM is there (its size read from the heap, D79; every Solite image
  expects it); the Wi-Fi link is not required, so a router that is off does not roll back a
  good update (decision D54). The reason an image is not healthy is logged once, when it changes
  (`update: unhealthy: no PSRAM`, `engine stopped` or `API not listening`). The OTA thread starts
  without PSRAM too, with no chunk buffer: every update call then answers 503 `No memory for an
  update` and the probation ticks, so an image without its PSRAM is rolled back at 300 s. The host
  has no PSRAM to expect and always reports it present. An image not confirmed 300 s after its start is rolled back (logged
  `update: not confirmed in time, rolling back`): the previous image becomes the next boot, and the
  box restarts into it. A confirmation or a rollback that fails is logged (`update: confirm
  failed: ...`, `update: roll back failed: ...`) and tried again at the next tick, about one
  second later. The time is the monotonic clock, so a jump of the wall clock changes nothing.
  Installations are refused while on probation (above) and accepted once confirmed.
- **A crash, a restart or a power cut before the confirmation** is rolled back by the bootloader
  at the next boot (ESP-IDF's app rollback, `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE`). Unplugging
  the box during the minute after an update brings the old image back; the update is then simply
  installed again. The setting belongs to the **bootloader**, which an update over the air never
  replaces: it reaches the box only through `scripts/box.sh flash`. Until the box has been
  flashed once over USB with an image that carries it, a new image is never put on probation
  (its state stays new, nothing is pending) and nothing rolls back. The sim and QEMU images get
  the setting too; with `otadata` erased they boot `ota_0` as before.
- **The routes**, behind the token (401 and 403 as every route, before any body byte is read):
  `GET /api/v1/system/version` answers 200 `{"current": "<version>", "latest": <x.y.z or null>,
  "updateAvailable": <bool>, "releaseUrl": <url or null>}` (Sowel's `VersionInfo` without its two
  Docker fields) from the last check, never from the network: `latest` is `null` and
  `updateAvailable` false before any successful check; `POST /api/v1/system/version/check` checks
  now (see "From a release"); `POST /api/v1/system/update` takes the signed file as its body, sent
  as `application/octet-stream` (415 `Unsupported Media Type` otherwise), **or** an empty body
  (`Content-Length: 0`, or neither `Content-Length` nor `Transfer-Encoding`; any content type),
  which pulls the latest release (a deviation from Sowel's route, which takes no body, decisions
  D53 and D65). Another method on any of them: 405. The body limit (32 KB and 128 bytes) does not apply to the
  update route; a declared `Content-Length` over 128 bytes + 4 MB + 1 is 413 before any read, and
  on the box a chunked upload is 411. The body ends at `Content-Length` bytes, so the check for a
  longer file never waits on the socket. Every refusal of this route that leaves body bytes unread
  (401, 403, 405, 409, 411, 413, 415, 503, a header refused, an error mid-image) closes the
  connection after its answer, so the image's bytes are never read as a next request. A refusal
  that comes while the body is still being read (a header refused, an error mid-image) is answered
  first, then the box reads and drops what is left of the declared body, about 10 s at most (plus
  one receive wait of 5 s), before it closes: the client gets the refusal's answer whatever was left of its upload, not a reset (not
  on the 413, where the client is not made to send the file). Through the
  core's plain request handler (tests and tools, which cannot stream) the update and check routes
  answer 501 `Updates need the streaming handler`.
- **From a release.** The box reads the release's `manifest.json` (see "Publishing a release"),
  says whether a newer release exists, and downloads and installs it on request, through the same
  installation as the upload (signature, streamed hash into the inactive slot, no downgrade,
  probation, rollback).
  - **The manifest.** Fetched whole, at most 16 KB (`Manifest too large`); `format` must be 1
    (`Unsupported manifest format <n>`); `version`, `minVersion` (null, absent or `x.y.z`),
    `image.size` (129 bytes to 128 + 4 MB), `image.sha256` (64 hex characters) and `image.url`
    are checked (`Invalid manifest: <field>`); the notes and the date are not kept. One entry is
    kept (the cache): the version, the minimum version, the file's address, size and hash, and the
    release page, which for a GitHub asset address is `https://github.com/<owner>/<repo>/releases/tag/<tag>`
    and `null` for any other. A failed check keeps the previous entry.
  - **The check.** `POST /api/v1/system/version/check` (no body read, no content type needed)
    checks now and answers 200 with the `VersionInfo` above. `updateAvailable` is true when `latest`
    is newer than the running version and `minVersion` is not newer than it. Refusals, in order:
    the clock not set 503 `The clock is not set yet: checking a release needs the date`; a check
    or a download in progress 409 `Update check already in progress`; a check started less than 60
    s ago 429 `Please wait <n>s before checking again`; a failure 500 `Update check failed:
    <reason>` (`<url>: <status>`, `too many redirects`, `redirect without Location`, `plain http
    is refused for <host>`, `redirect from https to http refused`, `invalid URL`, `timed out`,
    `took longer than <n> s`, a connection error, or the manifest's message), logged `update: check
    failed: <reason>`; a success logs `update: check: latest <x.y.z>, running <x.y.z> (<manifest
    address>)`, so the log says which server answered. The cache is in RAM only: a fresh start has
    `latest` null, and a check always reaches the network, the cache filled or not. The setting
    `update.manifestUrl` names the manifest; the clock must be set (a date before 2020 is not).
  - **Where the box may fetch from.** Every address (the setting, `image.url`, a redirect) is at
    most 2048 visible ASCII characters: `http` or `https`, a host of letters, digits, `-` and `.`
    (no user, no IPv6), an optional port, an optional path; anything else is `invalid URL`.
    `https://` to any host; `http://` only to an IPv4 literal of `10/8`, `172.16/12`, `192.168/16`
    or `127/8` (four decimal numbers, no leading zero) or to `localhost`, so a test server on the
    local network works and a name or public address does not (D66). Redirects (301, 302, 303,
    307, 308) are followed with a GET, five at most, never from `https` to `http`; a `Location` is
    an absolute address or a path starting with one `/`. Nothing but a GET is sent, with
    `User-Agent: solite/<version>`, no cookie and no authentication header.
  - **The pull.** `POST /api/v1/system/update` with an empty body, in this order: the clock not
    set 503; a check or another pull in progress 409 `Update already in progress`; no key 503; an
    installation in progress, an update waiting for the restart or the running image on probation:
    the 409s above, before any network access (and an upload meanwhile gets them); then the
    manifest (the cache when its check is younger than 10 minutes, else a check now, with the check's
    messages and its 60 s: a second one within a minute of a failed one is 429, without a new
    connection); `latest` not newer 400 `No update available`; `minVersion` above the running
    version 400 `Version <latest> needs <minVersion> or later first (running <current>): install
    <minVersion> before it` (the intermediate release is uploaded from its GitHub page, then the
    latest pulled); an image over the slot 413. The file is then fetched: a declared length that
    differs from `image.size` is 400 `Download size differs from the manifest` before any write;
    the body is read through a reader bounded to `image.size` + 1 bytes that hashes and counts every
    byte, given to the installation as its source; the header's length and version must agree with
    the manifest (400 `Download does not match the manifest`, before anything is written), then the
    installation's checks run, and after the image, before the slot becomes the next boot, the
    count and SHA-256 must equal the manifest's (else the same 400, the slot dropped). A body longer
    than announced is 400 `Update longer than declared`. Every read waits at most 30 s, the whole
    download 10 minutes (a check alone 15 s and 30 s). A connection that drops, a read error, a
    timeout, the total time passed or a body that ends early drops the slot as a truncated upload
    does and answers 500 `Download failed: <reason>` (`connection lost`, `timed out`, `took longer
    than 600 s`, the adapter's error), logged `update: download failed: <reason>`; the next attempt
    starts over from byte 0, once the resumes below are spent or refused. The answer is the
    upload's 200 `{"success": true, "message": "Installed <x.y.z>, restarting"}`, sent when the
    file is installed. Between reads the download hands the CPU back every 64 KB.
  - **A cut download resumes.** After the download's first answer, a read that fails (reset, or
    no byte for 30 s) or a body that ends before `image.size` bytes does not fail the pull at
    once: the cut body is dropped (its connection closed), the core waits, and fetches `image.url`
    again from the start of its redirects (a GitHub asset's signed address expires within
    minutes), every request carrying `Range: bytes=<bytes already read>-`. A 206 whose
    `Content-Range` is exactly `bytes <count>-<size - 1>/<size>`, whose `Content-Length` (when
    given) is the rest and whose `ETag` (when both answers have one, at most 256 characters) is
    the first answer's continues the same installation: the same hash, the same count, the same
    slot; nothing is kept on flash across pulls. At most 3 resumed requests per pull, waiting 2, 4
    then 8 s before each, all within the 600 s (the total time is never resumed); a resumed
    request that fails, answers a 5xx or is cut again counts as one attempt. Each logs `update:
    download cut at <count> of <size> bytes (<reason>), resuming (<n>/3)`. The pull fails cleanly
    (the slot dropped, 500 `Download failed: <reason>`) and the next one starts over from byte 0
    when: the attempts are spent (`<reason> (after 3 resumes)`); the server answers 200 to the
    range request (`the server does not resume (200 to a range request)`); it answers 416 or a
    206 that is not the rest asked for or whose `ETag` changed (`the file changed on the server`,
    and the cached manifest is dropped, so the next pull reads it again); or another status
    (`<url>: <status>`). Whatever the number of resumes, the count must equal `image.size` and the
    hash the manifest's, then the signature check runs, before the slot becomes the next boot
    (D72).
  - **The automatic check.** The setting `update.autoCheck` turns it on (`"true"`; off when unset,
    `""` or `"false"`, D67). On, a check runs 10 minutes after the start, then every 24 hours, when
    the clock is set and no check or pull runs (otherwise it waits for the next minute); it fills
    the cache like the route, logs the same lines, never installs, and its failures wait for the
    next day without counting for the 60 s of the route. Turning it on after the first 10 minutes
    checks at the next minute.
  - **The settings.** `update.manifestUrl` is a URL of the rule above, 512 characters at most
    (default `https://github.com/mchacher/solite/releases/latest/download/manifest.json`; another
    value 400 `Invalid value for update.manifestUrl`); `update.autoCheck` is `"true"` or `"false"`
    (400 `Invalid value for update.autoCheck`). Both are in `settings.json`; the previous release
    drops them with its warning and writes the file again.
- **The host** keeps its slots as files in `<data>/update/`: `slot-0.bin`, `slot-1.bin` and
  `boot.json` (`{"boot": 0|1, "pending": <bool>}`, absent meaning slot 0, not pending), each
  written through a temporary file and a rename. It does not run the image it stores nor restart:
  after an installation it prints `update: <x.y.z> written to slot <n>; restart the host to put it
  on probation`. At start, a `boot.json` with `pending: true` puts the host on probation, logged
  `update: slot <n> on probation`, and `update: running image confirmed` follows a minute later. The
  `update:` lines are printed without `--verbose`. A stalled upload ends within 30 s (the door's
  idle read timeout), the installation counts as truncated and the next upload is accepted. The
  host fetches `http://` by itself (a body delimited by `Content-Length` or by the connection's
  end; a chunked answer is refused, `chunked answers are not supported over http`) and `https://`
  through the system's `curl` program (`https needs the curl program` without it; no TLS library
  is in the workspace, D69); a check or a pull runs on the API worker that took the request, the
  automatic check on a thread of its own, asked once a minute.
- **The box** (radio image) serves the same route. One OTA thread, with a 6 KB stack in internal
  RAM, does every call that touches the flash (the API workers' stacks are in PSRAM and never
  touch the flash, a read neither), receives them through a channel one request at a time, and ticks the probation
  every second (and runs the supervisor's tick); a chunk crosses in one 4096-byte buffer in PSRAM.
  The installation itself (the body read, the header's signature check, the image's hash, the
  answer) runs on an `update` thread started for the request, its 16 KB stack in internal RAM,
  while the API worker that took the request waits: the signature check alone needs about 9 KB of
  stack, more than a worker's 10 KB holds with the request's own frames. When the thread cannot
  start, the answer is 503 `Busy, try again` and the connection closes. The thread logs `update:
  thread stack <n> of 16384 bytes never used` at the end of every attempt, accepted or refused
  (D60). A check (`POST /system/version/check`, or the automatic one) and a pull (an empty `POST
  /system/update`) run the same way on an `update-check` or an `update-pull` thread, 20 KB of
  stack in internal RAM (D68), one at a time (the 409s of "From a release"): the worker that took
  the request waits for the thread, the automatic check's thread is not waited for (the box asks
  whether it is due about once a minute), and neither holds the home's lock while it waits on the
  network. The box fetches through ESP-IDF's HTTP client, https verified with ESP-IDF's common
  certificate bundle (the authorities of most of the web, GitHub's included), redirects followed
  by the core, one TLS session at a time; an empty body on the update route reaches the core with
  a declared length of 0 (a chunked update stays 411, and so does the backup restore without a
  length). The threads log `<name>: thread stack <n> of 20480 bytes never used`; a thread that
  cannot start answers 503 `Busy, try again`. After a success of an upload or of a pull the answer is sent, then the engine stops (its
  final save runs) and the box restarts, logged `restarting`. Its start lines are the host's. A restart the supervisor makes before the confirmation
  (a panic, a watchdog, the last rung of the ladder) rolls the image back like any other restart,
  and an image that boots into the safe mode is not healthy (its engine's loop does not run): it
  rolls back 300 s after its start like any image that does not confirm.
- **Restarts on request.** `POST /api/v1/system/restart` (Sowel's route, Solite's answer, D79; the session's, 401 without;
  an empty body) answers Sowel's 200 `{"success": true, "message": "Restarting"}`, then the box restarts as after an update: 500 ms
  for the answer to leave, the engine stopped (its final save runs, 10 s at most), then the boot's
  cause is `asked` (D58: not a crash, the restart count and the safe mode clear). A second request
  while a restart is under way is answered the same, and the box restarts once. Refused with 409: while
  an update is received or installed (`Update already in progress`), while an update check runs
  (`Update check already in progress`), while a backup or a restore runs (`Backup already in
  progress`), while a recipe job (`Recipe install already in progress`) or a catalogue refresh
  (`Recipe catalogue refresh already in progress`) runs, and while the running image is on probation (`The update is being checked, restart in
  a minute`: a reset would roll the update back). Allowed when an update is installed and waits for its restart, or a
  restore waits for its: the box is about to restart. The safe mode lets the route through (the way
  out of it; the engine is already stopped, no wait). An open pairing window is closed by the restart.
  The Mac's host answers 501 `{"error": "The host does not restart itself"}`.

### Supervises its engine

The radio image watches its own engine. When the engine thread ends on an error, or stops answering,
the whole box restarts and says why; too many restarts put the box in a **safe mode** that keeps the
way in. Nothing of this writes the flash.

- **The reasons.** Every restart has a reason, a short text kept across the restart: `radio-stopped`
  (the radio's restart ladder refused a 4th restart in 10 minutes), `radio-silent` (the radio answers
  at no speed), `radio-start` (any other radio failure before the loop runs), `engine-error` (the
  home cannot be loaded, or the loop ended on a storage or data error), `panic` (the previous run
  ended on a panic), `watchdog` (a watchdog reset the chip), `brown-out` (the supply dropped),
  `unexpected` (a software or other reset that nothing of Solite asked for), `asked` (a restart
  asked on purpose: the console's restart, an installed update, a restore, an update's rollback)
  and `safe-mode-retry`. A power-on (power, reset button, USB reset) is not a restart: its cause
  is `power-on`, and it clears everything the supervisor kept. A brown-out is shown but never
  counted, nor are `asked` and `safe-mode-retry`.
- **The last rung.** The radio's ladder runs first (at most 3 restarts of the radio in 10 minutes).
  When it ends (`radio-stopped`, or `radio-silent`), when the loop ends on another error, or when
  the start fails, the engine saves what is due once (an error is logged and does not stop the
  restart), writes the reason, logs `supervisor: restarting the box after <reason>: <detail>`
  and restarts the box one second later. A stop asked by the console or an update is not an error:
  nothing is done, the asked restart wins.
- **The record.** 24 words (96 bytes) in the ESP32's RTC slow memory, in a section the boot does not
  initialise: it survives a software restart, a panic and a watchdog reset, and is ignored after a
  power-on or when its magic number, version, checksum, a reason or its count is wrong. It holds
  the pending reason, the last incident, the supervisor clock (milliseconds run since the last
  power-on, across restarts, written every 10 s), the start of the safe mode, the times of the last
  6 counted restarts and two counters. No NVS, no file.
- **The boot-loop guard.** Each boot logs `supervisor: boot after <cause>, <n> restarts in the last
  hour, <m> in the last day` (plus `, safe mode` or `, bare safe mode`). With 3 counted restarts
  within the last 60 minutes of the supervisor clock, or 6 within 24 hours, the box enters the safe
  mode and logs `supervisor: safe mode: the engine stays stopped, the API and updates stay up;
  retried in <t>`. A crash in the safe mode does not end it; 3 counted restarts since it began make
  it the **bare safe mode**, where the home's files are not even read.
- **The safe mode.** The console, the update thread, the Wi-Fi station and the server run as usual;
  the engine thread does not start (no radio reset, no loop, no saver, no boot marker, no pairing,
  no recipe). The home is loaded and served, so one can log in, look for a release and install an update; a home that
  cannot be loaded is replaced in RAM by an empty one (logged), and the stored files are never
  touched. Every request that is not a `GET` is refused with 503 but the account's, the update's, the release check's (`POST /api/v1/system/version/check`),
  the restart's (`POST /api/v1/system/restart`, D79) and a script recipe's uninstall (`POST /api/v1/plugins/:id/uninstall`);
  no installed script recipe is loaded (each is shown `refused`, `Safe mode: not loaded`);
  a restore and a factory reset need the presence a power-on gives, so they are refused too: the
  restart and the update are the ways out over the network (a restart asked for clears the safe
  mode), otherwise a USB flash or a power-on. The first-password
  setup follows the reset reason as always, so a safe mode never opens it. The safe mode is
  retried: after 1 hour the first time since the power-on, 4 hours the second, 24 hours from the
  third on, the box restarts once with the cause `safe-mode-retry`, the restart times cleared; a
  fault that persists brings the safe mode back after 3 more restarts. A power-on or a restart
  asked also ends it.
- **Restarts asked.** Before each restart Solite asks for on purpose, the firmware writes `asked`:
  the next boot counts nothing and ends a safe mode.
- **Panics.** A panic aborts (the target's strategy); ESP-IDF prints the panic and restarts the
  chip, and the next boot reads the reset reason `panic` and counts it. Nothing catches a panic.
- **The watchdog.** The engine thread is subscribed to the task watchdog, whose timeout is 20
  seconds and which restarts the chip. The engine feeds it at every read of the radio's serial link
  (every 50 ms at most, so every radio wait), at each turn of its loop, before each file the saver
  writes and before each step of the radio's restart ladder. What blocks without reading the radio
  is a flash write, the longest known being the 3.8 s save of a 98 KB `devices.json`. The two idle
  tasks stay watched with the same timeout and restart the chip too.
- **The status.** `GET /api/v1/system/supervisor`, behind the token, answers 200 `{"mode":
  "normal"|"safe"|"safe-bare", "bootCause": "<reason>"|"power-on", "lastIncident":
  "<reason>"|null, "restartsLastHour": <n>, "restartsLastDay": <n>}`; `lastIncident` is the last
  reason other than `asked` and `safe-mode-retry` since the power-on. Another method: 405. The
  host program answers the default (`normal`, `power-on`, `null`, 0, 0). A deviation from Sowel,
  which has no supervisor (decision D58).
- **Not supervised.** The host program (it has no restart of its own), the image QEMU runs and the
  box's image without the radio: only the radio image is.

### Publishing a release

A release is made by the CI from a tag, and by `scripts/release.sh` (which the CI runs) on the Mac.
The steps a maintainer follows are in `docs/release.md` (the `/solite-release` skill points there).

1. **Key, once.** `cargo run -p solite-host -- --update-keygen <file>` writes a new signing key
   (32 random bytes, as 64 hex characters and a newline) to `<file>`, created new with mode `0600`,
   and prints `public key: <64 hex>`. It refuses an existing file and a path inside a git working
   tree (a `.git` in the file's folder or above), exit 2 with the reason. Keep the file outside the
   repository, offline (an encrypted volume or a password manager), and back it up: a lost key
   means a box can only be updated by a USB flash. `*.update-key` and `*.sup` are ignored by git.
   The public key is committed as `release/update-key.pub`; the private key is the secret
   `SOLITE_UPDATE_KEY` of the GitHub environment `release` (deployment rule: tags `v*` only).
2. **Tag.** A tag `vX.Y.Z` on the merge commit of `main` starts `.github/workflows/release.yml`.
   Its `firmware` job (no environment, no secret, `contents: read`) refuses a missing
   `release/update-key.pub` and a tagged commit that is not on `main` before its build, then runs
   `sh scripts/release.sh X.Y.Z --build-only` and uploads `app.bin` and `flash.bin`. Its `sign` job
   (the environment `release`, `needs: firmware`) refuses (before the host is built, an
   `::error::`) an empty secret (`SOLITE_UPDATE_KEY_PREVIOUS` too when `release/update-key.bridge`
   is in the tagged tree, point 7), a missing `release/update-key.pub` and a tagged commit that is not
   on `main`; it builds the host program only (no ESP32 toolchain, no npm), downloads the artifact
   and runs `sh scripts/release.sh X.Y.Z --key-env SOLITE_UPDATE_KEY --prebuilt <folder>`. Its
   `publish` job (`needs: sign`), the only one with
   `contents: write`, makes the GitHub release (not a draft, not a pre-release; an existing release
   for the tag makes it fail: a release is never replaced). Run by hand (Run workflow), the
   workflow is a dry run: a throwaway key, the script, and only `manifest.json` and `SHA256SUMS`
   kept as an artifact; nothing is published and no secret is read (with a bridge file in the tree,
   the dry run makes a second throwaway key and a bridge file of its own and runs the bridge's
   checks with them, point 7). The concurrency group is
   `release-<event>`: a dry run never cancels a tag run, and a second tag run waits for the first.
3. **The script.** `sh scripts/release.sh <x.y.z|vx.y.z> (--key-file <path> | --key-env <NAME>)
   [--prebuilt <folder>] [--public-key-file <path>] [--out <folder>] [--notes <file>]
   [--min-version <x.y.z>] [--repo <owner/name>] [--next-key-file <path> | --next-key-env <NAME>]
   [--bridge-file <path>]`, or `sh scripts/release.sh <x.y.z> --build-only
   [--public-key-file <path>] [--out <folder>] [--notes <file>]`. It refuses (exit 2, nothing built) a version that is not `x.y.z` or that is not
   the `version` of `Cargo.toml`'s `[workspace.package]`, of `firmware/Cargo.toml` and of
   `ui/package.json`; no key option, both, a key variable that is unset or empty, or a key file
   that is missing or inside a git working tree or the output folder; a public key file
   (`release/update-key.pub` by default) that is missing or not 64 hex characters; no section
   `## <version>` in `docs/release-notes.md` (or an empty one); a non-empty output folder
   (`target/solite-release/<version>` by default); a `--min-version` that is not lower than the
   version. A probe (16 bytes signed and verified) stops a key that does not match the public key
   (exit 1) before the build. `--build-only` takes no key (a key option is refused, exit 2), makes
   every check above that needs none, builds, and writes only `app.bin` and `flash.bin` to the
   output folder (the line `release: built, not signed (--build-only)`); no probe, no signing.
   `--prebuilt <folder>` takes `app.bin` and `flash.bin` from that folder instead of building (it
   needs a key; a missing file is exit 2) and refuses (exit 1, `the prebuilt image does not carry
   <public key file>'s key`) an `app.bin` that does not hold the public key's 64 hex characters.
   The build is the radio image with `SOLITE_UPDATE_PUBLIC_KEY` set to
   the public key and `SOLITE_RECIPE_PUBLIC_KEY` to the recipe key's (`release/recipe-key.pub`; the script and the
   workflow refuse to build without that file: `release: release/recipe-key.pub is missing: make the recipe key first`, exit 2), in `firmware/target/release-radio` (never `target/radio`), checked as
   `box.sh flash --image radio` checks it, made into the application image (`esptool elf2image`,
   as `box.sh`) and a full-flash image (`esptool merge_bin`: bootloader at `0x1000`, partition
   table at `0x8000`, application at the `ota_0` offset, from `0x0` padded with `0xFF`). The
   application is signed (the line `signed: no application descriptor` is refused here) and the
   signed file is verified with `--update-verify` and the public key. The radio build, before it
   is accepted, must hold the public key's text (`grep`). The signing key is never in the
   environment of a job that builds with third-party tools (the `firmware` job has no secret, the
   `sign` job builds only the host program), and the script takes it from the environment before
   assigning any variable (`--key-env` and `--next-key-env` read it with `printenv`, then unset it,
   both keys of a bridge), so no command it
   runs inherits it; it is neither on a command line, in the output nor in the log; the staging
   folder and the log are searched for it before anything is kept. A signal (Ctrl-C) stops the
   script's whole process group (the build's children included) and removes its folders.
4. **What a release holds**, in the output folder and as the GitHub release's assets:
   `solite-<version>.sup` (the signed update file), `solite-<version>-flash.bin` (the full-flash
   image for a first USB flash, written at `0x0` after a whole erase), `SHA256SUMS` (`<sha256>`,
   two spaces, the name; sorted; `shasum -a 256 -c` reads it; not signed) and `manifest.json`;
   the folder also holds `notes.md` (the release's text on GitHub, not an asset).
5. **`manifest.json`**, at `https://github.com/<repo>/releases/latest/download/manifest.json`:
   `{"format": 1, "version", "releasedAt" (UTC, ISO 8601 to the second), "notes", "minVersion"
   (null, or `x.y.z`: a box below it goes through an intermediate release), "image": {"name",
   "size", "sha256" (of the `.sup`), "url"}}`, two spaces of indent. Solite's own JSON, not
   Sowel's (decision D63). A running box reads it (see "Updates itself from a signed file", "From a
   release").
6. **Install** from the Settings page's Firmware update card (Update to x.y.z, or a `.sup` chosen
   from a file; see "Shows the home in the browser"),
   or `POST` the `.sup` to `/api/v1/system/update` with the token. A box never flashed with Solite
   takes the full-flash image over USB (`docs/release.md`).
7. **A bridging release** rotates the signing key (D70; the steps are in `docs/release.md`). It is
   declared by the file `release/update-key.bridge` in the release's tree (`--bridge-file <path>`
   overrides the path): `version <x.y.z>` and `previous <64 hex>` once each, `#` lines and blank
   lines ignored, else `release/update-key.bridge: <reason>`. The image carries the new public key
   (`release/update-key.pub`, as always); the `.sup` is signed with the old private key
   (`--key-file` / `--key-env`), and the new private key is given too (`--next-key-file` or
   `--next-key-env`, read like the signing key, 64 hex characters), so that boxes never move to a
   key nobody holds. The script refuses, before the build and with nothing written: a bridge for
   another version (`a bridge is one release; remove the file after it`); a `previous` that is not
   64 hex characters or is the current public key; a signing key that does not match `previous`; a
   missing next key; a next key that does not match `release/update-key.pub`; a `--next-key-*`
   without a bridge file, or with `--build-only`. After signing, `--update-verify` must accept the
   `.sup` with `previous` and refuse it with the new key (else `the bridge's file verifies with the
   new key: nothing is published`); the script prints `release: key bridge: signed with
   <previous8>, the image carries <pub8>`. The manifest has no field for a bridge: a box accepts the
   bridge by its compiled (old) key and trusts the key its new image carries once it is confirmed,
   refusing the old key's files (`Invalid signature`). In `release.yml`, the `sign` job with the
   bridge file in the tagged tree needs the environment secrets `SOLITE_UPDATE_KEY_PREVIOUS` (the
   old private key) and `SOLITE_UPDATE_KEY` (the new one), read by that job only, and runs the
   script with `--key-env SOLITE_UPDATE_KEY_PREVIOUS --next-key-env SOLITE_UPDATE_KEY`; an empty
   `SOLITE_UPDATE_KEY_PREVIOUS` is an `::error::` and no release. Without the file, the command is
   the normal one. The Mac's chain test (`crates/host/tests/rotation.rs`) builds the host with two
   keys and checks that a box before the bridge refuses a file signed by the new key and the box
   after it refuses one signed by the old.

`solite-host --update-sign <image> --version <x.y.z> --key <file> --out <file>.sup` writes the file
above and prints `signed: <out>, version <x.y.z>, <bytes> bytes, key <first 8 hex>` (the bytes are
the image's). It refuses (exit 2, nothing written) an empty image or one over 4 MB, a version that
is not `x.y.z` of numbers up to 65535, a key file that is not 64 hex characters, and an existing
`<out>`. When the image is an ESP32 application image (first byte `0xE9`, magic `0xABCD5432` at
offset 32), the version of its application descriptor (offset 48, 32 bytes) must equal
`--version` (else exit 2); an image without a descriptor (the Mac demo's bytes) is signed with the
line `signed: no application descriptor, version not checked`. The core only verifies; the signing
is in the host.

`solite-host --update-verify <file> --public-key <64 hex>` checks a signed file as the box does,
without installing it: the header (as the box, with a running version of `0.0.0` and the slot's
capacity), then the image's length and SHA-256 against the header. It prints `verified: <file>,
version <x.y.z>, <n> bytes, key <first 8 hex>` (exit 0). A public key that is not 64 hex characters
or not a valid Ed25519 key, an unreadable file, a file shorter than the 128-byte header, a refused
header, an image shorter or longer than the header says, or a digest that differs: exit 2 with
`update-verify: <reason>`. One of `--update-verify` and `--public-key` without the other: usage, exit 2.

### Flashes the box and brings it back

`sh scripts/box.sh <command> [--port <device>] [--baud <rate>]` works on the Dongle Max over USB,
with `esptool.py`, `espefuse.py` and `gen_esp32part.py` taken from the `PATH`, else from ESP-IDF's
Python environment in `firmware/.embuild/`. The port is `--port`, or the only USB bridge found
(none or several: exit 2 with the list); the baud rate is 921600 by default.

- `identify` (read-only) prints the chip and its revision, the MAC address, the flash size, secure
  boot and flash encryption. It stops (exit 3, `box: STOP, <reason>; nothing is written to this
  box`) on a box it cannot flash: secure boot on, flash encryption on, a chip revision below 3 or a
  flash that is not 16 MB. When the box does not answer (it is not in download mode), exit 4 and
  the manual way (hold the button while plugging the cable) is printed.
- `backup --out <folder>` reads the whole flash twice, keeps the file `dongle-max-<mac>-<date>.bin`
  outside the repository (mode 0600) and its `.sha256` only when the two reads are equal, 16 MB long
  and not one repeated byte, and prints the row for `docs/hardware.md`.
- `restore <file>` writes the backup back, after checking its size, its SHA-256 and that the MAC of
  its name is the connected box's.
- `flash --image box|sim|radio [--backup <file>]` takes the backup as an option (decision D35: the
  way back to the original firmware is Sonoff's own reflash, and the plan says so when no backup is
  given); a backup given is refused unless it is 16 MB, its name holds this box's MAC and its
  SHA-256 is in the backup table of `docs/hardware.md` with a restore date. It checks the built
  image is the one asked for
  and fits the `ota_0` slot, erases `nvs`, `otadata` and `storage` (unless `--keep-storage`, radio
  image only), writes the bootloader, the table
  of `firmware/partitions.csv` and the application, and resets the box. The bootloader it writes
  carries the app rollback (`CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE`, see "Updates itself from a
  signed file"): a box that has never been flashed with it never puts an update on probation, and an
  update over the air cannot bring it. The radio image is read
  from `firmware/target/radio`, a build without the marker `radio: uart ` is refused with the
  build command, a radio build without the embedded interface (its ELF lacks the marker
  `solite-ui `) is refused with the build command, and the box and sim images refuse a radio build; the box is not reset at the end
  (`--after no_reset`): the first boot, and its pairing window, is the one `log --image radio`
  captures.
- `log --image box|sim|radio [--seconds <n>] [--replug] [--expect-kept]` resets the box, captures its
  serial output and checks it as `scripts/qemu.sh` checks QEMU's (banner, memory, `storage:`,
  `network:`, then `radio: none` or the `engine:` and `saves:` lines); it also fails on `psram none`,
  a panic or a boot loop, and with `--expect-kept` on `storage: formatted`. `--from-file <log>`
  checks a captured log, `--raw` captures without checks. With `--image radio` it captures 300
  seconds by default and shows the serial output as it comes; it checks the banner, `memory:`,
  `storage:` and `network:` lines, then `radio: uart`, `ezsp:`, a `network: formed|resumed` line,
  `pairing: open for` or `pairing: not opened`, and the `serial:` and `heap:` lines; it fails on
  `no answer at any baud rate`, `radio: stopped after`, `pairing: failed:` (named), `task_wdt`, a
  panic, a second banner (a reboot), `wifi: settings refused` (named) and a `heap:` internal minimum
  below 64 KB. Its `network:` line is `network: wifi (setting <s>)` (or `network: ethernet (setting
  <s>), not in this image, no link started`), without `no link started` when Wi-Fi is the choice;
  after it, a `wifi:` line (none is required when the choice is `ethernet`), and, once `wifi:
  connected` shows, a `serving: http://` line; it prints the last `wifi:`, `clock:`, `serving:` and
  `memory-network:` lines. A box with no Wi-Fi settings passes. The box and sim images' checks
  still require `no link started`.
- `flash --image radio --keep-storage` erases `nvs` and `otadata` only and keeps
  `storage` (the home, the network's backup and the Wi-Fi settings); its plan says `keep storage
  (the home, the network's backup and the Wi-Fi settings)`, then `WARNING: keep it only over a radio
  image: a sim image's /data is a test home`, and the radio image's warning about the
  erase of `zigbee.json` is not printed. Without it, the plan is as above and the warning ends
  `(use --keep-storage to keep them)`. With `--image box` or `sim` it is refused (`--keep-storage is
  for the radio image: the sim image seeds /data`, exit 2). The radio image's plan also prints
  `the box drives this radio's network itself: never start a Mac host with this network's backup on
  another radio while the box runs`: one network, one coordinator at a time. The same holds for a
  full backup (see "Backs up and restores the whole box"): never restore one onto a second box
  while the first one still runs.
- `wifi [--forget]` asks on the terminal (`/dev/tty`, never standard input; no terminal: `box: no
  terminal to ask for the Wi-Fi settings, nothing done`, exit 1) for the network's name (shown as
  typed) and its password (echo off, restored on any exit, Ctrl-C included, nothing sent then). It
  opens the serial port, which resets the box (the DTR and RTS lines of the USB-serial chip), waits
  for the console's `console: ready` line (12 s at most), sends one line, `wifi-set <JSON>`, to the
  firmware's serial console, and reads the answer: `wifi: settings saved` (the box then saves its files and restarts) then, within 60 s, `wifi: connected, address <IPv4>`
  (printed as `box: Wi-Fi connected, address <IPv4>, name <host name>; open http://<IPv4>/`, exit
  0, else the last `wifi:` line and exit 1); `wifi: settings refused (<reason>)` (printed, exit 1);
  no answer within 15 s: `box: no answer from Solite's console on <port>: is the radio image
  running?`, exit 4 (an image without the console, such as the box's original firmware, answers
  nothing and nothing changes on the box). The JSON is built from the two lines read on standard
  input: the password is never in a process list, a file, a log or the output; the name is not
  printed either. `wifi --forget` sends `wifi-forget` and reads `wifi: settings removed` (exit 0, or
  exit 4 with no answer).
- `pair` opens the serial port (which resets the box), waits for `console: ready` as `wifi` does,
  sends the line `pair` and reads the answer within 15 s: `pairing: open for 180 s` (printed as
  `box: pairing is open (open for 180 s): put the device in pairing mode now`, exit 0);
  `pairing: failed: <reason>` (printed, exit 1); the console's `pairing: asked from the console`
  with no window after it: `box: the console took the line but the engine opened no pairing window
  within 15 s: is the radio running?`, exit 1; no answer: the message of `wifi`, exit 4. It asks
  nothing and needs no backup: it writes nothing to the flash but the boot marker of the reset it
  causes.
- `saves [--budget <n>] [--no-flush] [-- <command> [<args>]]` runs one test on the box with its saves
  held, in one session of the serial port (opening the port resets the box, so a hold and its
  flush are never sent in two sessions): it opens the port, keeps every line read from then on in
  a log under `$TMPDIR/solite-box/` (path printed), waits for `console: ready`, sends `saves hold`
  and reads `saves: held` within 15 s (none: the message of `wifi`, exit 4), waits for
  `serving: http://<address>/` (60 s; none: `box: the box serves nothing within 60 s: the command is
  not run, the saves stay held until the next restart`, exit 1) and prints `box: saves held;
  serving at http://<address>/`. It then runs the command with `SOLITE_BOX_ADDRESS=<address>`, or,
  without one, asks on the terminal `box: saves held: run the test, then press Enter to flush`
  (neither a command nor a terminal: `box: saves needs a command after -- or a terminal`, exit 2,
  the port never opened). Then it sends `saves flush` and reads `saves: flushed, <k> files written`
  within 30 s (printed `box: saves flushed (<k> files written)`; only `saves: flush asked`: `box:
  the console took the line but the engine flushed nothing within 30 s: is the engine running, and
  the clock set?`, exit 1); with `--no-flush` it sends nothing and prints `box: saves still held:
  the changes stay in RAM and are lost at the next restart`. It prints the last `flash:` count of
  the session, `box: flash: <m> writes since boot` (0 when none), and with `--budget <n>` fails
  when `m > n` (`box: flash: <m> writes since boot, over the budget of <n>`, exit 1). A second
  `console: ready` after the hold (the box restarted) gives `box: the box restarted during the
  session: the hold was lost and the count is not the session's`, exit 1, no flush. A command that
  fails, or cannot start (`box: cannot run the command: <reason>`, exit 127), still gets the flush
  and the count, then `box: the command failed (exit <c>)`, exit 1; the command runs from the
  caller's folder. Ctrl-C at the terminal (or a SIGTERM to its Python helper), from the hold on,
  stops the command, sends the flush and exits 130; a SIGTERM to `box.sh` alone ends the script,
  and its helper still flushes once the command ends.
- `log --budget <n>` (any image, from the port or `--from-file`), after the log's checks, prints
  the last `flash:` line (`flash: 0 writes since boot` when there is none) and fails (exit 1, the
  message of `saves`) when its count is over `n`; a `--budget` that is not a whole number is a
  usage error (exit 2).
- `backup`, `restore` and `flash` print what they will do; `backup` and `restore` then ask, on the
  terminal (never on standard input), for the last 6 hex digits of the box's MAC address; anything
  else, or no terminal, writes nothing. `flash` asks nothing beyond the command itself (D35).
  `--dry-run` prints the plan only. No command writes an eFuse.
- Only `firmware/src/radio_uart.rs` names GPIO12 and GPIO15 (the radio's bootloader and reset
  lines, both strapping pins of the ESP32), GPIO13 and GPIO17 (the UART): a test checks it. The box
  and measuring images leave the radio untouched.

#### Runs a box session in one command

`sh scripts/box-session.sh [--yes-flash --backup <file>] [--factory-reset] [--with-gestures]
[--port <device>] [--address <address>[:<port>]]` runs, in one pass and with no prompt between the
steps, every check of the roadmap's "Tests to run with the box in hand" that needs no hand gesture,
through `box.sh` (decision D55). `--yes-flash` is the user's yes of decision D6 for that run: typed
by the user in their terminal (refused when `/dev/tty` cannot be opened: `box-session: --yes-flash
is typed by the user in a terminal`, exit 2), never added by an agent; it needs `--backup <file>`
(or `SOLITE_BOX_BACKUP`), passed to `box.sh flash`, which checks it (missing: exit 2, nothing run).
Without it nothing is flashed and nothing is written through the API: preflight, the image size,
`identify` and the boot log only, every other step `skipped (needs --yes-flash)`.

- **Steps** <!-- check-docs: allow -->, in a fixed order: P1 preflight (it warns when the working tree has changes under
  `crates/` or `firmware/`); P2 an update key made with `--update-keygen` in the run folder; P3 the
  radio image built with that key (and with the recipe key's public half when `release/recipe-key.pub` exists); P4 a next version (patch + 1) and a rollback image (patch + 2,
  panics at start) built in a copy of `HEAD` outside the repository, signed, and a copy with one
  signature byte flipped; P5 the radio image's size against the `ota_0` slot; B1 `identify`; B2
  `box.sh flash --image radio --keep-storage --backup <file>` (`storage` is never erased); B3
  `box.sh log --image radio --budget 1` with the internal RAM free and largest block, the PSRAM
  free, the internal minimum and the storage used, read from the serial log; B4 the
  authentication checks inside `box.sh saves --budget 3`; B5 the second boot (`network: resumed`,
  one banner, no token in the serial log); B6 `measure-api.sh` inside `saves --budget 5`; B7 the
  home checks inside `saves --budget 4`; B8 the update checks, outside `saves` (the box restarts);
  B9 `box.sh log --raw --seconds 300 --budget 1` and its `heap:` line; B10 the factory reset inside
  `saves --budget 7`, only with `--factory-reset` (it erases the home); B11 the script group: when `motion-light` is not installed it uploads the signed file given by
  `--recipe-file <file.srs>` (else the group is `skipped (not installed: --recipe-file)`, and so
  is its serial-lines check); it then takes the garage's own enabled `motion-light` instance
  (`skipped` without one) and checks it by a wave: the light on within 10 s, then off within the
  instance's timeout plus 30 s (an unreadable timeout, as "1h30m", skips the off check with a
  line); nothing of the home is copied, changed or deleted; at the group's end the recipe is kept
  while an instance of it exists on the box (the garage's own), and only an upload that no
  instance uses is uninstalled again. A step whose dependency failed is
  `skipped (after <id> <outcome>)`.
- **The checks**: authentication (status, setup or login, one login timed, at most 2 s, 401
  without a token, `/ws` closed 4001 without a token and live with it, the pairing route behind the
  token, the 429 of the 11th attempt in a minute); home (the full backup exported, and its restore with a wrong password refused (400, nothing
  changed), the
  location and time zone set from `SOLITE_HOME_LATITUDE`, `SOLITE_HOME_LONGITUDE` and
  `SOLITE_HOME_TIMEZONE` and read back, the sun times after the clock is set, `GET
  /api/v1/system/timezone`, the devices' dates and status, and on the box the 5th request `Busy`
  and a client that stops reading); update (401 without a token, a flipped signature refused, the
  next version installed and running within 180 s with the settings kept, confirmed after 75 s, the
  rollback image installed and rolled back after a restart is proven); reset (204, setup required,
  setup again). Every group closes, as soon as it holds a token, the pairing window that each power-on boot
  opens, so no device joins during a counted step.
- **Stops**: `identify` refusing the box, a failed flash, a failed boot after it, a failed rollback
  (the report names the way back, a USB flash) or a `saves` over its budget stop the box steps:
  the later ones are `skipped (stopped after <id>)`. No step is repeated.
- **Secrets**: the password is `SOLITE_PASSWORD`, else the password file (`$SOLITE_PASSWORD_FILE`,
  by default `$HOME/.config/solite/box-password`, `<run folder>/password` with `--api-only`); a box with no account is set up with a password
  generated into that file (0600, never overwritten), named on the verdict line, never printed. The
  key is removed when the run ends, however it ends.
- **Output**: one line per step, `box-session: <id> <name>: OK|KO|skipped (<reason>)`, a group's
  checks on lines of their own, then `box-session: <k> OK, <m> KO, <s> skipped; log: <path>`;
  every command's output is in `session.log` of the run folder (`$TMPDIR/solite-box-session/<date>`,
  0700, refused inside the repository). Exit 0 with no KO, 1 with one, 2 for a usage error.
- **Gestures**: eleven checks that need a person (a power cut, a hand in front of the sensor, the
  access point off, the relay unplugged, the browser, an unplug during an update, a colour bulb)
  are `skipped (needs a person: <what>)`, or, with `--with-gestures`, asked one by one on the
  terminal after the automatic steps (`y` OK, `n` KO, `s` skipped).
- `--api-only <address>[:<port>]` runs the same groups and `measure-api.sh` against the Mac host's
  `--serve`, with no box and no firmware build (the host program makes the key and signs 16 bytes
  for the refused signature); the box-only checks are `skipped (box only)`. Only `127.0.0.1`,
  `localhost` and `::1` are accepted (`box-session: --api-only is for the Mac host (127.0.0.1,
  localhost, ::1): <address> refused`, exit 2).

`sh scripts/validate.sh` prints, after the firmware builds, each image's size and its share of the
4 MB `ota_0` slot (`<image>: <n> KB, <p> % of the 4096 KB ota_0 slot`), with `WARNING: above 75 %
of the slot` past three quarters.

### Measures the coverage of Zigbee2MQTT's definitions

A tool on the Mac, `tools/z2m-coverage` (`npm run report` in its folder), loads the pinned version
of Zigbee2MQTT's device library (`zigbee-herdsman-converters`) and writes
`tools/z2m-coverage/report.md`. For each leading Zigbee brand (Philips Hue, IKEA, Aqara, Tuya and
its white labels, Sonoff, Schneider Electric, Legrand and others), the report gives:

- the share of definitions the box can reuse by export: **declarative** (data and calls to the
  library's shared code) and **Tuya tables** (datapoint tables read by the shared Tuya
  interpreter);
- the share that needs code: **brand code** (a manufacturer's library file) or **device code**
  (written in the device's own file), and among it the share whose code only binds the device and
  sets up its reporting;
- the share a generic reading of the standard Zigbee clusters handles (temperature, humidity,
  on/off, level, colour, occupancy, alarms, battery, covers, thermostat, metering), in full, or
  for its main function with only the manufacturer's extras missed;
- the number of shared functions of the library the box needs to cover the brand.

It also gives totals, the line of a few devices of interest (the Sonoff SNZB-02 among them), and
how the measure is made and what it cannot see. Nothing of the tool runs in the box.

### Stays within a memory budget

The engine stays under 300 KB on the Mac for a realistic home (50 devices, 10 zones, 50 equipments, 3
recipe instances with full logs, a live client with a full queue, the largest answer in flight).
In the emulated ESP32 it stays under 350 KB for the realistic home with the fixture scenario
playing on the simulated radio inside the image (no network, no HTTP client, logs as the scenario
fills them). The same image pulls a release from an in-memory fetch adapter on a thread of 20 KB
and must leave 8 KB of that stack unused, the room for the TLS handshake the box runs. Script
recipes have a ceiling of their own, 896 KB for the 4 of them at their limit (224 KB each), counted beside the
engine's in both the realistic home and the worst case. It stays
under 1 MB with every bound of the core at its maximum (64 devices of 8 data, 32 zones, 64
equipments of 8 bindings and 8192 bytes of Sowel's optional equipment fields, 16 recipe instances of 16 lights and 16 buttons, 4 live clients with
full queues, the orders queues full). Both are checked by tests on the Mac, whose 64-bit figures
are upper estimates of the ESP32's; a test fails when a bound is added to the core without being
counted in the worst case. The tooling measures the engine's figures on the Dongle Max with the
measuring image; they are written in `docs/memory-budget.md`, "On the box". The recipe jobs
(a download and its checks, a write, a removal) run on the `recipe` thread of 20 KB of internal RAM,
made once at start, one job at a time, and an install is admitted only when the PSRAM and the largest internal block leave
the reserves of "Runs script recipes". The radio image logs
its own `heap:` line (the minimums since boot, the largest free block of each memory, the
fragmentation's indicator, the engine stack's and the workers' stack's high-water marks) and, once, `memory-network:` (a snapshot with the server idle, for information),
and `box.sh
log --image radio` fails when its internal RAM minimum is below the 64 KB reserve.
`docs/memory-budget.md` has a row for the internal RAM with Wi-Fi up (the `memory-network:`
snapshot measured on the box; the network's cost as a difference of two `heap:` minima and the
projection for the realistic home are not computed) and one for the 17 requests' time on the box,
not measured (it needs a relay paired). `docs/memory-budget.md` also has the time the engine holds the home lock for one batch of
measures, measured on the Mac. It shares the box's 2 MB
of PSRAM and its internal RAM item by item, and `docs/hardware.md` is the sheet of the Dongle Max's
wiring and its risks.

No mode exists in the tool; no recipe is built in (`motion-light` is a script recipe, installed from
the catalogue or a file); the host keeps the devices,
zones, equipments, recipe instances, the account and the network backup, makes and restores the full backup, serves Sowel's device, zone,
equipment and recipe API, its live events and the interface (above), executes equipment orders, runs the recipe instances,
and talks to the radio to identify it, start its network, pair devices and receive their
measures; the firmware's radio image does the same over the serial link and serves the API, the
live events and the interface over Wi-Fi, behind the home's password.

## History

| Spec | Change |
| ---- | ------ |
| 001 | Walking skeleton: the tool starts and shows its name |
| 002 | Measures the coverage of Zigbee2MQTT's definitions for the leading brands |
| 003 | The firmware shows its free memory and boots in QEMU on the Mac |
| 004 | The host identifies the Dongle Max's radio over TCP (ASH, EZSP) |
| 005 | The radio runs a Zigbee network; the host pairs a device and shows what it is |
| 006 | The host sets up reporting and shows the measures of the standard clusters (`--listen`) |
| 007 | A simulated radio and devices play scenario files (`--sim`) |
| 008 | Devices kept in Sowel's format and served by Sowel's device API (`--serve`) |
| 009 | Alarm sensors enrolled; their motion, contact, leak or smoke becomes device data |
| 010 | Zones, equipments and zone aggregation, with Sowel's routes |
| 011 | Equipment orders sent to the Zigbee devices (`POST .../orders/:alias`) |
| 012 | The `motion-light` recipe, with Sowel's recipe routes |
| 013 | Live events on Sowel's WebSocket (`/ws`) |
| 014 | The interface: the rooms and their equipments, live, served by the host |
| 015 | The Dongle Max's hardware sheet, the memory budget and its worst-case test, the partition table and memory placement, the network choice at boot |
| 016 | The engine's loop and saver in the core, run by the firmware on its data partition with the simulated radio under QEMU, memory measured |
| 017 | Pairing while serving, from the API and the interface's devices page: new devices appear live and named |
| 018 | Rooms and equipments made from the interface: the Topology and Equipments pages, Sowel's forms |
| 019 | The Administration section (Devices, Equipments, Topology, Discovery per integration); a device has no room; equipments made from a device's row, bound as Sowel's interface binds |
| 020 | `scripts/box.sh` identifies the Dongle Max, saves and restores its original firmware, flashes Solite's images with the user's typed confirmation and checks their boot log; the measuring image times the saves on the box |
| 021 | The firmware's radio image drives the radio over the serial link, runs the engine on the box, and pairs a device at boot |
| 022 | The rules for serving on the local network in the core, the interface packed for the flash, the scripts on an address, the 33 requests measured |
| 023 | The radio image joins Wi-Fi and serves Sowel's API, its live events and the interface from its flash |
| 024 | Recipes from a room's page (list, form from the slots, enable and disable, journal, settings, delete); a recipe settles when what it uses changes |
| 025 | Saves held and flushed from the serial console and `box.sh saves`, every flash write counted and checked against a budget, a burst of definition changes written once |
| 026 | The home's settings (location, time zone, sun offsets) and the sun times on the root zone, so "Inactive during day" works; a Settings page; export and import of the settings and zones |
| 027 | The Sowel contract checked (snapshot, subset and deviations); Sowel's 31 equipment types and optional equipment fields; `light_color`; a paired device's type and bindings proposed in the core; `GET /api/v1/system/timezone` |
| 028 | Core fixes: the dates of a device taken before the clock is set move by its jump; a motion-light button without an `action` binding leaves the settings instead of disabling the recipe; a running motion-light follows its zone's motion sensors as they change |
| 029 | A password at commissioning, confirmed at the box: one account, Sowel's `/auth` routes and bearer token on every route and on `/ws`, the box's pairing route behind the token, a factory reset |
| 031 | The colour cluster: a colour bulb's colour temperature and colour read, its colour orders sent, proposed as `light_color`; a simulated colour bulb |
| 032 | Signed updates: a signed update file streamed into the inactive slot, a probation with automatic rollback, `--update-keygen` and `--update-sign`, a Firmware update card |
| 035 | The full backup: one password-protected file with the home, the Zigbee network and the Wi-Fi settings, streamed both ways, checked whole before a restore replaces the home at the next start; the radio forms the restored network; a Backup card |
| 036 | The supervised engine: the box restarts when its engine ends on an error, with the reason kept in RTC memory; a boot-loop guard and a safe mode that keeps the API and updates up; the engine on the task watchdog; `GET /api/v1/system/supervisor` |
| 038 | Release publication: `scripts/release.sh` and `.github/workflows/release.yml` make a signed release from a tag (the `.sup`, a full-flash image, `SHA256SUMS`, `manifest.json`); `solite-host --update-verify`; the key's custody in `docs/release.md` |
| 041 | Script recipes: recipes in JavaScript, in Sowel's script-recipe format, run by QuickJS on the host and the box, reaching only their slots; `motion-light-script`; `--recipe-script`; bounds, a failing script recipe stopped then restarted; the cost measured on the Mac, in QEMU and on the box |
| 042 | Script recipes installed, updated and removed without a new firmware: a signed `.srs` file (the recipe key), a catalogue, admission by memory, a Resources card and a Plugins page; the script leaves the radio image |
| 043 | A Restart button and route (`POST /api/v1/system/restart`, allowed in safe mode, refused during a probation, 501 on the host); the update's probation confirms an image only with its PSRAM and logs why it is not healthy; the OTA thread starts without PSRAM |
| 044 | Tighter bounds: 8 data a device, 8 data and 8 order bindings an equipment, 20 log lines a recipe instance; a home stored with the old bounds is cut at load, logged once |
| 046 | The native `motion-light` recipe removed: no recipe is built in, `motion-light` is Sowel's 1.1.2 as a script recipe (Sowel's id, info and validation texts, at most 4 instances since spec 053), the memory ceilings measure scripts, the Mac launchers pass `--recipe-script` |
