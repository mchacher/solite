# Spec 059: the same API as Sowel, recipes first (they move under `/plugins`)

Written on 2026-11-28 from the user's request: "the API of Solite as close to 100 % as possible to
Sowel's; first, the recipes under Sowel's `/plugins`, as Sowel does; putting them elsewhere was a
mistake". Sowel files read: `src/api/routes/plugins.ts`, `src/packages/package-manager.ts`
(`getStore`, `refreshRegistryNow`), `src/packages/registry-types.ts`, `src/shared/types.ts`
(`PluginManifest`, `PluginInfo`, `PackageSource`, `PackageTier`, `RecipeInfo`), `src/api/routes/{recipes,
backup,system,zones,devices,equipments,auth,health}.ts`, `docs/technical/api-reference.md`.

## Context
Solite serves 55 routes (`api::ROUTES`). Against Sowel's route code (the inventory is in
`architecture.md`): 42 are Sowel's, same path and method (10 of them differ in value only, each kept
by a decision: `/auth/*`, `/system/update`, `/system/version*`, `/system/restart`,
`/system/timezone`); 13 are Solite's own, of which **five are the recipe store**
(`/recipes/store`, `/recipes/store/refresh`, `/recipes/install`, `/recipes/:id/update`,
`/recipes/:id/uninstall`, D78) and two are the full backup, which Sowel serves at another path
(`/system/backup`, Sowel's is `/backup`). Sowel does the store's job, for the same packages (a
recipe is a package of `type: "recipe"`), under `/api/v1/plugins`: list the installed
(`GET /plugins`), list the store (`GET /plugins/store`), refresh (`POST /plugins/store/refresh`),
install (`POST /plugins/install` `{repo}`), update and uninstall (`POST /plugins/:id/update`,
`/uninstall`). Sowel's `/api/v1/recipes` stays what it is: the definitions of the recipes that
run (`RecipeInfo`). A tool made for Sowel (the user's scripts, Sowel's own interface pointed at the
box) cannot install a recipe on the box today.

## Goals
- Every recipe route of Solite is a route of Sowel: same path, method, request body and answer
  shape, for the part Solite has. What Solite added beyond Sowel's JSON is removed, or lives in a
  Solite-only route that is already there (`/system/resources`).
- The box's interface follows (its one client); nothing in the repository calls an old path.
- `docs/sowel-subset.md`'s Deviations table loses the five recipe routes and the backup pair, and
  the contract test proves it.

## Out of scope
- The other deviations, one spec each as the roadmap row says: `GET /settings/timezones`,
  `/integrations/solite/permit-join` (Sowel has no native radio; its path is in Sowel's
  integration namespace), `/system/factory-reset`, `/system/supervisor`, `/system/resources`.
- Sowel routes the box does not need: personal sources (`/plugins/sources*`), `enable`/`disable`
  and `oauth/*` of a plugin, integrations' start/stop, users, `/me`, MFA, modes, history, charts,
  energy, calendar, dashboard, shared access, the timed action, `devices/suggest`,
  `devices/:id/raw`, `devices/battery-alerts`, `zones/reorder`, `zones/:id/orders/:orderKey`,
  `/health`, `/system/sunlight`, `/system/mode`, `/system/takeover`. They stay unserved (404), as
  today; the ones worth adding (`/health`, `/system/sunlight`, the zone reorder) are lines of the
  roadmap.
- Aliases for the old paths (R6).
- The catalogue's address and format version (`recipes.catalogue`, `recipes.json` format 1): a
  catalogue written for 0.3 still reads (R2).

## Requirements
### R1: `GET /api/v1/plugins`, the installed recipes
- An array of Sowel's `PluginInfo`, one per installed script recipe (the built-in scripts of the
  host's `--recipe-script` too): `manifest` (`id`, `name`, `version`, `description`, `icon` when
  the catalogue gave one, `repo`, `type: "recipe"`, `category` and `author` when known), `enabled:
  true`, `installedAt` (ISO 8601), `status: "connected"`, `deviceCount: 0`,
  `offlineDeviceCount: 0`, `source: "registry"`, and `latestVersion` when the catalogue holds a
  newer one than the installed file (Sowel's `getAvailableUpdateFor`).
- It never reaches the network: it reads what the box holds, as `GET /recipes/store` did (D78).
- A recipe refused at start is not listed; its reason stays in `GET /system/resources`
  (`scripts[].reason`), where D78 already put it.

### R2: `GET /api/v1/plugins/store`, the catalogue
- An array of Sowel's store entries (`PluginManifest` plus `compatible`, `compatReason?`,
  `isOfficial`, `tier`, `i18n?`, `tags?`): the catalogue's entries **not installed** (Sowel's
  `getStore` rule: the installed ones are in `GET /plugins`). `isOfficial: true`, `tier:
  "official"` (the one catalogue is the author's, signed, D78). `compatible: false` and
  `compatReason: "Requires Solite >= <minVersion>"` when the entry's `minVersion` is above the
  running version.
- `repo` of an entry: the catalogue's new optional `repo` (`owner/name`, the recipe's Sowel
  repository); an entry without one gets `solite-recipes/<id>`, so a 0.3 catalogue still reads and
  every entry has a unique `repo` that matches Sowel's `owner/repo` pattern. `category`, `tags` and
  `author`: new optional fields of the catalogue, passed through.
- What was there and goes, as Sowel has none of it: the wrapper object (`recipes`, `readAt`,
  `error`), `size`, `sha256`, `url`, `installed`, `refused`, `builtIn`. Where each goes: `readAt`
  and `error` to the refresh's answer and the log; `installed` to `GET /plugins`; `refused` to
  `GET /system/resources`.

### R3: `POST /api/v1/plugins/store/refresh`
- Reads the catalogue again; answers Sowel's `{count, source}` (`source: "remote"`). A failed read
  keeps the held list and answers `{count, source: "local"}` (Sowel's answer when its fetch
  fails), the reason logged. Solite's one difference of value stays: at most once a minute (429),
  D78.

### R4: `POST /api/v1/plugins/install`
- Body `{repo, confirmed?, expectedSha256?}` as Sowel's: the catalogue entry with that `repo` is
  downloaded, checked and installed as `POST /recipes/install {id}` did (D78). `confirmed` and
  `expectedSha256` are accepted and ignored (the signature is the check; Sowel's confirmations are
  for community and personal packages, which the box has none of). Answers `{"success": true}`
  (Sowel's answer for a recipe). A `repo` the catalogue does not hold: 404 `{"error": "Plugin not
  found"}`; a malformed one: 400 `{"error": "Invalid 'repo' field (expected owner/repo)"}`
  (Sowel's text).
- The signed file as the body (`Content-Type: application/octet-stream`) stays on the same path,
  as `/system/update` takes a file where Sowel's takes none (D53): a difference of value, listed
  in `docs/sowel-subset.md`, answered `{"success": true}` too.

### R5: `POST /api/v1/plugins/:id/update` and `/uninstall`
- As `POST /recipes/:id/update` and `/uninstall` were (D78: the catalogue's newer version; the
  file removed, the instances kept), at Sowel's paths. The update's body is optional (`{confirmed?,
  expectedSha256?}`, ignored). Answer `{"success": true}`. Solite's statuses stay (404 unknown, 409
  while another job runs, 400 nothing newer), a difference of value beside Sowel's 500.
- The safe mode's exception for the removal (D58: the way out of a script that breaks the engine)
  moves to the new path.

### R6: the old paths are gone
- `/recipes/store`, `/recipes/store/refresh`, `/recipes/install`, `/recipes/:id/update`,
  `/recipes/:id/uninstall` are removed: `GET /recipes/store` now meets `GET /recipes/:id` and
  answers 404 `Recipe not found`; the POST ones answer 405 or 404 as any unknown route. No alias:
  the box's interface is embedded in the same image and moves with the firmware (the choice and
  its cost are in `architecture.md`).
- A script recipe's `id` still may not be `store` or `install` (D75): a file in the field keeps
  loading, and no id has to change.

### R7: the full backup at Sowel's path
- `GET /api/v1/backup` and `POST /api/v1/backup` replace `/api/v1/system/backup` (Sowel's
  `src/api/routes/backup.ts`: export and restore of the whole system). The body, headers, statuses
  and file are Solite's (D56: a password-protected archive, not Sowel's ZIP), a difference of
  value in the same section of `docs/sowel-subset.md`. Sowel's `/backup/local` and
  `/backup/restore-local` are not served. The old path is removed, no alias.

### R8: the contract and the interface
- `docs/sowel-subset.md`: the routes table lists the new routes; the Deviations table loses the
  five recipe routes and the backup pair; the file body of the plugin install is written beside
  the other differences of value. `crates/core/tests/sowel_contract.rs` passes against the snapshot
  and Sowel's repository (the snapshot rewritten).
- The interface (`ui/src/api/recipeStore.ts` and what uses it: the Plugins page, the sidebar's
  update count, the backup card) calls the new paths and reads the new shapes; what it showed from
  the removed fields (size, a refusal, the catalogue's read time and error) comes from the answers
  named in R2.
- `scripts/box-session.sh`, `scripts/demo-garage.sh` and the host's tests that call the old
  paths call the new ones.

## Living spec delta
### docs/living-spec/README.md
- The routes table and "Runs script recipes": the store routes at `/plugins`, with the shapes of
  R1 to R5; the backup at `/api/v1/backup`; the safe-mode exception's new path; the catalogue's
  new optional fields (`repo`, `category`, `author`, `tags`).
### docs/sowel-subset.md
- Routes and Deviations tables (R8), the plugin install's file body, the backup's content.
### docs/decisions.md
- D89: the recipes under Sowel's `/plugins`, the old paths removed with no alias (replaces D78's
  routes, not its design), the backup at `/backup`.
### docs/architecture.md, docs/roadmap.md, docs/roadmap.html
- The routes' module comment; the roadmap row "Same API as Sowel": first increment done, what
  is left (the "Out of scope" list).

## Acceptance criteria
- [ ] The five recipe routes of D78 answer as unknown routes; the six of R1 to R5 answer with
      Sowel's shapes (a test of each, with a catalogue of two entries, one installed).
- [ ] `GET /plugins/store` lists only what is not installed; after an install the entry moves to
      `GET /plugins`, after an uninstall back.
- [ ] A client built from Sowel's `docs/technical/api-reference.md` (`curl`) lists, installs,
      updates and removes a recipe on the Mac simulation.
- [ ] The interface's Plugins page does what it did (install, update, remove, refresh, the update
      count in the sidebar) with the new routes (Vitest).
- [ ] `GET /backup` and `POST /backup` answer what `/system/backup` did (the existing backup tests
      moved to the new path); the old path is unknown.
- [ ] `sowel_contract.rs` green; the Deviations table has no recipe route and no backup route.
- [ ] `sh scripts/validate.sh` green, QEMU included.
- [ ] Box (the user runs `box-session.sh`): a recipe installed from the catalogue, updated and
      removed through the new paths; a backup downloaded and restored.

## Edge cases
- Two catalogue entries with the same `repo`, or a `repo` that is not `owner/name`: the catalogue
  is refused as a whole (`Invalid catalogue: <id>: repo`), as a duplicate `id` is.
- `POST /plugins/install` with the `repo` of an installed recipe: 409 `Recipe already installed`
  (as the id's today).
- `installedAt` when the file's date is FAT's start (no clock at install): answered as that date;
  the interface does not show it.
- A browser tab opened before the update still calls the old paths and gets 404 until reloaded;
  `/system/update` is unchanged, so an old tab can still update the box.
- A stored script named `store` or `install`: loads as before; its removal is
  `/plugins/<id>/uninstall`, which meets no fixed route.
- Safe mode: `POST /plugins/:id/uninstall` passes (R5); `POST /plugins/install` does not.

## How the user will check it
Open the box's Plugins page: install a recipe from the catalogue, update it, remove it (the same
as before). Then, from the Mac, with the token: `curl -H "Authorization: Bearer ..."
http://<box>/api/v1/plugins` and `.../plugins/store` give the arrays of Sowel's documentation.

## Decisions (2026-11-28)
- The user: the old `/recipes/...` paths are removed, no alias; the backup moves to `/backup` in
  this spec (its own encrypted file format kept, a difference of value listed in
  `docs/sowel-subset.md`); `GET /plugins/store` answers Sowel's bare list, the catalogue's read
  date and error move to `GET /system/resources` and the Resources card.
- The session: each entry of the `solite-recipes` catalogue gains its `repo` (and the box falls
  back to `solite-recipes/<id>` for an entry without one); `installedAt` is the install's time when
  the clock is set, else `null`. Built after spec 058 is merged (both touch `recipes/script/` and
  `firmware/`).
