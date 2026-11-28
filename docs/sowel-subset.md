# Sowel subset

What Solite has of Sowel's model and API, and why; and the names it has that Sowel does not, each
kept by a decision. Solite is a subset of Sowel: what it has is Sowel's (names, values,
semantics, JSON; Sowel's `src/shared/types.ts` wins), and what is outside the subset is refused
or kept untouched (decision D43).

`crates/core/tests/sowel_contract.rs` reads this file: the first column of each table below must
list exactly what the code has, and every name of Solite outside Sowel must be in
**Deviations**. It compares Solite with `contract/sowel-snapshot.json` (taken from Sowel's
repository) and, when it is present, with Sowel's repository itself (`SOWEL_REPO`, else
`../sowel`). The JSON fields of devices, zones, the zone aggregation, equipments, bindings and
recipes are checked against Sowel's interfaces from the API's real answers; they need no table.
Every one of Sowel's 31 equipment types is accepted, stored and served; Sowel's optional
equipment fields are kept.

To rewrite the snapshot after Sowel moved: `SOWEL_SNAPSHOT=write cargo test -p solite-core
--features script-recipes --test sowel_contract` with Sowel's repository beside this one.

## Data categories

The categories the device catalog gives a device's data (Sowel's `DataCategory`).

| Category | Why |
| -------- | --- |
| `temperature` | Temperature cluster (0x0402); a thermostat's LocalTemperature (Thermostat 0x0201), key `local_temperature` |
| `humidity` | Humidity cluster (0x0405) |
| `pressure` | Pressure cluster (0x0403) |
| `luminosity` | Illuminance cluster (0x0400) |
| `motion` | Occupancy cluster (0x0406) and IAS motion zones |
| `contact_door` | IAS contact zones (Sowel's Zigbee2MQTT plugin gives `contact` this category) |
| `water_leak` | IAS water leak zones |
| `smoke` | IAS fire zones |
| `light_state` | On/off cluster (0x0006) |
| `light_brightness` | Level cluster (0x0008) |
| `light_color_temp` | Color Control cluster (0x0300), colour temperature in mireds |
| `light_color` | Color Control cluster (0x0300), the `color` object (type `json`) |
| `battery` | Power configuration cluster (0x0001), percentage |
| `voltage` | Power configuration cluster (0x0001), battery voltage |
| `action` | A remote's On/Off (0x0006), Level (0x0008) and Scenes (0x0005) commands, the command's generic name (type `text`) |
| `power` | Electrical Measurement cluster (0x0B04) ActivePower, else Metering (0x0702) InstantaneousDemand, in W |
| `energy` | Metering cluster (0x0702) CurrentSummationDelivered, in kWh |
| `current` | Electrical Measurement cluster (0x0B04) RMSCurrent, in A |
| `shutter_position` | Window Covering cluster (0x0102) CurrentPositionLiftPercentage, as `100 - lift` (100 open), in % |
| `setpoint` | Thermostat cluster (0x0201) OccupiedHeatingSetpoint, in °C |
| `generic` | IAS zone flags (`tamper`, `battery_low`) |

## Order categories

The categories of a device's orders (Sowel's `OrderCategory`).

| Category | Why |
| -------- | --- |
| `light_toggle` | On/off cluster: on, off, toggle |
| `set_brightness` | Level cluster: move to level |
| `set_color_temp` | Color Control cluster: move to color temperature |
| `set_color` | Color Control cluster: move to color, move to hue and saturation |
| `shutter_move` | Window Covering cluster (0x0102): `OPEN` (up), `CLOSE` (down), `STOP` (type `text`) |
| `set_shutter_position` | Window Covering cluster: go to lift percentage `100 - value`, 0 to 100 (100 open) |
| `set_setpoint` | Thermostat cluster (0x0201): write OccupiedHeatingSetpoint, 5 to 30 °C |

## Events

The events Solite sends on `/ws` (Sowel's `EngineEvent`); the others are not sent (D18).

| Event | Why |
| ----- | --- |
| `device.discovered` | A device joined |
| `device.removed` | A device deleted |
| `device.status_changed` | Online, offline |
| `device.data.updated` | A measure |
| `zone.created` | Zone routes |
| `zone.updated` | Zone routes |
| `zone.removed` | Zone routes |
| `zone.data.changed` | Zone aggregation |
| `equipment.created` | Equipment routes |
| `equipment.updated` | Equipment routes, bindings |
| `equipment.removed` | Equipment routes |
| `equipment.data.changed` | A bound measure |
| `equipment.status.changed` | Equipment status |
| `equipment.order.executed` | An order answered |
| `equipment.order.failed` | An order failed |
| `recipe.instance.created` | Recipe routes |
| `recipe.instance.removed` | Recipe routes |
| `recipe.instance.started` | Recipe routes |
| `recipe.instance.stopped` | Recipe routes |
| `recipe.instance.state.changed` | A recipe instance's state |
| `recipe.instance.error` | A recipe that cannot run |
| `settings.changed` | Settings routes |
| `sunlight.changed` | Sun times on the root zone |

## Settings keys

| Key | Why |
| --- | --- |
| `home.name` | The home's name (Sowel's interface writes it) |
| `home.latitude` | Sun times |
| `home.longitude` | Sun times |
| `home.timezone` | Local time (Sowel's global key; D41, D44) |
| `home.sunriseOffset` | The `motion-light` script's "Inactive during day" |
| `home.sunsetOffset` | The `motion-light` script's "Inactive during day" |
| `update.manifestUrl` | Where the box reads the latest release's manifest (Solite's own, D67) |
| `update.autoCheck` | The automatic check for a release, off by default (Solite's own, D67) |
| `recipes.catalogue` | Where the box reads the catalogue of script recipes (Solite's own, D78) |

## Routes

The routes of `api::ROUTES` (the host and the box also serve `GET /ws`, Sowel's, and the
interface's files).

| Route | Why |
| ----- | --- |
| `GET /api/v1/devices` | Devices |
| `GET /api/v1/devices/:id` | Devices |
| `PUT /api/v1/devices/:id` | Devices |
| `DELETE /api/v1/devices/:id` | Devices |
| `GET /api/v1/zones` | Zones |
| `POST /api/v1/zones` | Zones |
| `GET /api/v1/zones/aggregation` | Zone aggregation |
| `GET /api/v1/zones/:id` | Zones |
| `PUT /api/v1/zones/:id` | Zones |
| `DELETE /api/v1/zones/:id` | Zones |
| `GET /api/v1/equipments` | Equipments |
| `POST /api/v1/equipments` | Equipments |
| `GET /api/v1/equipments/:id` | Equipments |
| `PUT /api/v1/equipments/:id` | Equipments |
| `DELETE /api/v1/equipments/:id` | Equipments |
| `POST /api/v1/equipments/:id/data-bindings` | Bindings |
| `DELETE /api/v1/equipments/:id/data-bindings/:bindingId` | Bindings |
| `POST /api/v1/equipments/:id/order-bindings` | Bindings |
| `DELETE /api/v1/equipments/:id/order-bindings/:bindingId` | Bindings |
| `POST /api/v1/equipments/:id/orders/:alias` | Orders |
| `GET /api/v1/recipes` | Recipes |
| `GET /api/v1/recipes/:id` | Recipes |
| `GET /api/v1/recipe-instances` | Recipe instances |
| `POST /api/v1/recipe-instances` | Recipe instances |
| `PUT /api/v1/recipe-instances/:id` | Recipe instances |
| `DELETE /api/v1/recipe-instances/:id` | Recipe instances |
| `POST /api/v1/recipe-instances/:id/enable` | Recipe instances |
| `POST /api/v1/recipe-instances/:id/disable` | Recipe instances |
| `POST /api/v1/recipe-instances/:id/actions` | The `motion-light` script's actions |
| `GET /api/v1/recipe-instances/:id/log` | Recipe journal |
| `GET /api/v1/settings` | Settings |
| `PUT /api/v1/settings` | Settings |
| `GET /api/v1/settings/timezones` | The time zones the box knows (deviation, D41) |
| `GET /api/v1/system/timezone` | The home's time zone |
| `GET /api/v1/integrations/solite/permit-join` | Pairing window (deviation, D28; behind the token, D49) |
| `POST /api/v1/integrations/solite/permit-join` | Pairing window (deviation, D28; behind the token, D49) |
| `GET /api/v1/auth/status` | First start or login (Sowel's, D47) |
| `POST /api/v1/auth/setup` | The one account, confirmed at the box (Sowel's, D47, D48) |
| `POST /api/v1/auth/login` | Tokens (Sowel's, D47) |
| `POST /api/v1/auth/refresh` | New tokens (Sowel's, D47) |
| `POST /api/v1/auth/logout` | Closes the session (Sowel's, D47) |
| `POST /api/v1/system/factory-reset` | Erases the home and the account (deviation, D48) |
| `GET /api/v1/backup` | The full backup archive (Sowel's path, Solite's file, D56, D89) |
| `POST /api/v1/backup` | Restores the full backup archive (Sowel's path, Solite's file, D56, D89) |
| `GET /api/v1/system/version` | The running version and the last check's latest release (Sowel's `VersionInfo`, a subset, D53, D65) |
| `POST /api/v1/system/version/check` | Checks for a release now (Sowel's route, D65) |
| `POST /api/v1/system/update` | Installs the signed update file in its body, or, with no body, pulls the latest release (Sowel's route takes no body, D53, D65) |
| `POST /api/v1/system/restart` | Restarts the box (Sowel's route, Solite's answer, D79) |
| `GET /api/v1/system/supervisor` | The supervisor's status: mode, boot cause, last incident, restarts (deviation, D58) |
| `GET /api/v1/system/resources` | The memories, the storage and the script recipes' rows (deviation, D78) |
| `GET /api/v1/plugins` | The installed script recipes, Sowel's `PluginInfo` (D89) |
| `GET /api/v1/plugins/store` | The catalogue's script recipes not installed, Sowel's store entries (D89) |
| `POST /api/v1/plugins/store/refresh` | Reads the catalogue again, answers `{count, source}` (D89) |
| `POST /api/v1/plugins/install` | Installs a script recipe from the catalogue (`{repo}`) or from a signed file (D89) |
| `POST /api/v1/plugins/:id/update` | Updates an installed script recipe to the catalogue's newer version (D89) |
| `POST /api/v1/plugins/:id/uninstall` | Removes an installed script recipe (D89) |

## Equipment types with behaviour

The types with Sowel's binding rules (`equipments::rules`) and proposed by recognition; the other
23 are accepted, stored and served, without rules.

| Type | Why |
| ---- | --- |
| `light_onoff` | Relays and on/off lights (ZBMINIR2) |
| `light_dimmable` | Dimmers |
| `light_color` | Colour bulbs, the most common Zigbee lights (Philips, IKEA, Innr, Ledvance) |
| `shutter` | Roller shutters |
| `switch` | Plugs |
| `sensor` | Temperature, motion, contact, leak, smoke sensors |
| `button` | Remotes |
| `thermostat` | Thermostats |

## Deviations

Names Solite has and Sowel does not, each kept by a decision. Differences of value (bounds,
error texts, fields refused) are listed in the decisions themselves (D13, D14, D16 to D18, D41 to
D44, D47 to D49, D53, D56, D58, D65 to D67, D78 to D80, D89).

| Kind | Name | Decision |
| ---- | ---- | -------- |
| route | `GET /api/v1/settings/timezones` | D41 |
| route | `GET /api/v1/integrations/solite/permit-join` | D28 |
| route | `POST /api/v1/integrations/solite/permit-join` | D28 |
| route | `POST /api/v1/system/factory-reset` | D48 |
| route | `GET /api/v1/system/supervisor` | D58 |
| route | `GET /api/v1/system/resources` | D78 |
| setting | `update.manifestUrl` | D67 |
| setting | `update.autoCheck` | D67 |
| setting | `recipes.catalogue` | D78 |

Differences of value in the authentication (D47 to D49), none a new name: one account, always
`admin` (no other user, no `/me`, no `/users`, no API token, no MFA: `trustedDeviceToken` is
accepted and ignored); `preferences` has `language` only; opaque tokens (32 hex characters), not
JWTs, and logout closes the access token too; passwords of 8 characters or more (Sowel: 6); setup
answers 403 `{"error", "confirmRequired": true}` until a person at the box confirmed (D48); one
rate limit for the box (10 attempts per minute on setup and login together); `/ws` reads the token
from the first `bearer.` subprotocol other than `bearer.none`, and the box echoes `bearer.none`
only, so a client that offers `bearer.<token>` alone gets no subprotocol echoed there (D49).

Differences of value in the updates (D53, D65), none a new name: `POST /api/v1/system/update` takes
the signed update file as its body (`application/octet-stream`) **or** an empty body, which pulls
the latest release; Sowel's route takes no body. It answers Sowel's `{success, message}`, its
`{error}` shape, its 409 `Update already in progress` and its 400 `No update available`, when the
update is installed (Sowel answers at once and reports its progress by events, which Solite does
not have). `POST /api/v1/system/version/check` answers Sowel's `VersionInfo`, its 429 (60 s between
two checks, Sowel 10 s, no `Retry-After` header) and its 500 `Update check failed: ...`;
`GET /api/v1/system/version` answers `current`, `latest`, `updateAvailable` and `releaseUrl` from
the last check (`null` and `false` before one), without Sowel's two Docker fields
(`dockerAvailable`, `composeManaged`); `updateAvailable` is false when the release's `minVersion` is
above the running version (Sowel has no such rule). The settings `update.manifestUrl` and
`update.autoCheck` are Solite's own (D67).

Differences of value in the restart (D79), none a new name or shape: `POST /api/v1/system/restart` is Sowel's
route (it restarts Sowel's container through docker, answers `{success, message}` and refuses with 400, 403, 409 and 503
of its own); Solite's restarts the box, answers the same shape, 200 `{"success": true, "message": "Restarting"}`, and refuses with 409 while an update, an
update check, a backup, a recipe install or a catalogue refresh (`Recipe install already in
progress`, `Recipe catalogue refresh already in progress`) or a probation is on (401 without a
session, as every route).

Differences of value in the safe mode (D58), none a new name: while the supervisor holds the engine
stopped, every request that is not a `GET` answers 503 `{"error": "Safe mode: the engine is
stopped"}`, except the account's routes (`/auth/*`), `POST /api/v1/system/update` and `POST
/api/v1/system/version/check`, `POST /api/v1/system/restart` (D79) and `POST
/api/v1/plugins/:id/uninstall` (D78, D89, the way out of a script that breaks the engine). Sowel has no
such mode.

## Script recipes

Script recipes (spec 041, D73 to D76) follow Sowel's script-recipe draft (`roadmap/ai-script-recipes/spec.md`
in Sowel's repository: "The idea", "Script API (first cut)", "Rules enforced by the host"), which has no
code in Sowel yet. They add no route, field or event: a script recipe is served through Sowel's
recipe routes, with Sowel's `RecipeInfo`, `RecipeSlotDef`, `RecipeInstance` and `RecipeLogEntry`.
What differs from the draft is the script API's own (this section is prose, not a table the contract
test reads), each kept by D75 or D74:

- **Added to the draft**: the script's `id` (1 to 64 characters among `a-z`, `0-9`, `-`, 1 to 32
  when installed; never `store` or `install`, reserved since spec 042 when the store routes sat
  under `/recipes`; the draft has none) and an optional `version` (shown in the start line only;
  `x.y.z` and the signed header's when installed, spec 042); `onMotionReport(fn)`, called
  with the value and the equipment's id on every motion report of a motion sensor of the zone, so
  that a safety auto-off can be reset by fresh motion; `cancel(id)` for the timers;
  `console.log` as `log`; `validate(params, home)` with a second argument, `home.equipment(id)` giving
  the `name`, `type`, `zoneId` (Sowel's `Equipment` fields), `orders` and `data` of an equipment
  named in the params, and `home.zone(id)` giving `{ id }` for a zone named in the params that
  exists, else `null` (the draft's `validate` has
  `params` only, and no way to check bindings). A Sowel issue proposes `id`, the zone slot rule,
  `onMotionReport` and `validate(params, home)` for the draft.
- **Added** (spec 054): `now().localTime`, the home's time of day as `HH:MM` in its time zone,
  where a Sowel recipe reads the server's `new Date()`; and a slot's `group`, kept in the info.
- **Left out**: `mode()` (no modes), the tariff period in `now()`, `energy.claim/release`, history
  reads, the observation mode, a per-instance order rate limit, `state.alarm` semantics; a script
  that calls `mode`, `energy` or `history` gets `Not available in Solite: <name>`.
- **Added** (spec 055): `now().localWeekday`, the home's weekday (0 = Sunday, `null` while the clock is not set), as `localTime`; a slot of type `data-key` (a text of at most 64 bytes naming a data of the equipment of the first single `equipment` slot; the form offers its aliases) and one of type `text` (a text of at most 64 bytes, checked by the recipe's own
  `validate`, where Sowel checks nothing) and a slot's `hiddenWhen` (`{slot, equals}`, kept in the
  info; only the interface uses it).
- **Functions have no source**: QuickJS is built without keeping the source text of functions
  (`JS_NO_FUNCTION_SOURCE`, D74): `Function.prototype.toString` of a script's function answers
  `function f() { [native code] }`, where Sowel's engine gives the source.
- **Synchronous host calls**: `get`, `order` and the others answer at once (a superset of the draft,
  where they resolve through promises); a promise's `then` still runs after the call.
- **One runtime per recipe**: the instances of a script recipe share one QuickJS runtime and context
  (the draft: one per instance), so a script's top-level variables are shared and what belongs to
  one instance lives inside `start`.
- **An overrun fails the recipe**: a call over its time, memory or stack stops every instance of the
  recipe, which comes back after a minute (the draft stops the instance with `state.alarm`).
- **Validation is the host's**: the texts are Solite's own, `Invalid params: <text>` (Sowel has no
  generic slot validation: each of its recipes checks itself), then the script's `validate`.
- **`onZone(alias, fn)`** is called on every `zone.data.changed` of the instance's zone with the
  aggregate's current value, for `motion`, `luminosity` and `motionSensors`; **`get` and `order`**
  take a slot name only (never an id), read the data of category `light_state` first for the alias
  `state`, and send through a `light_toggle` or `toggle_power` binding first, as Sowel's helpers do.

## Installed script recipes

Script recipes installed from a catalogue or a signed file (spec 042, D78) are served at Sowel's
`/plugins` routes (spec 059, D89): a recipe is a package of `type: "recipe"`, as in Sowel's plugin
store. Differences of value, none a new name:

- `GET /api/v1/plugins` lists the script recipes the box runs (the built-in scripts of
  `--recipe-script` too) as `PluginInfo`: `enabled` is always `true`, `status` `connected`,
  `deviceCount` and `offlineDeviceCount` 0, `source` `registry`; `installedAt` is the install's
  time while the box runs, and `null` when the clock was not set at the install or the box
  restarted since (Sowel: always a date). A recipe refused at start is not listed (its reason is in
  `GET /system/resources`).
- `GET /api/v1/plugins/store` is Sowel's bare list without the installed ones; `isOfficial` is
  always `true` and `tier` `official` (the one catalogue is the author's); `compatReason` reads
  `Requires Solite >= x`, where Sowel's reads `Requires Sowel >= x`. The catalogue's `repo`
  (`owner/name`), `category`, `author` and `tags` are optional: an entry without a `repo` gets
  `solite-recipes/<id>`. A `category` that is not one of Sowel's (`lighting`, `climate`, `water`,
  `schedule`, `safety`, `energy`) is dropped, and `GET /api/v1/plugins` gives an installed recipe
  without one the category `other`, as Sowel does.
- `POST /api/v1/plugins/store/refresh` answers `{count, source}`, `source` `remote` (a read) or
  `local` (a failed read keeps the held list, the reason is in the log and in `GET
  /system/resources`), and at most once a minute (429, Sowel 10 s).
- `POST /api/v1/plugins/install` takes `{repo}` (`confirmed` and `expectedSha256` are accepted and
  ignored: the signature is the check) **or** a signed file as the body
  (`application/octet-stream`; Sowel's takes no file, as for `/system/update`, D53), and answers
  `{"success": true}`; 404 `Plugin not found`, 400 `Invalid 'repo' field (expected owner/repo)`, 409
  `Recipe already installed`.
- `POST /api/v1/plugins/:id/update` and `/uninstall` answer `{"success": true}`; their statuses
  are Solite's (404 unknown, 409 while another job runs or when nothing is newer, Sowel 500). An
  uninstall removes the recipe's file and keeps its instances, as Sowel's.
- `GET` and `POST /api/v1/backup` are Sowel's paths with Solite's file (D56): a password-protected
  archive in `X-Backup-Password`, not Sowel's ZIP. Sowel's `/backup/local` and
  `/backup/restore-local` are not served.

The setting `recipes.catalogue` is Solite's, as `update.manifestUrl` (D67). The file format
(`SOLITERS`, a header of 128 bytes signed with the recipe key) and the catalogue's (`recipes.json`)
are Solite's; a Sowel issue proposes them as a common format.
