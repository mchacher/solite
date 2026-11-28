# Architecture: Spec 059

## Inventory: Solite's 55 routes against Sowel's
Sowel's routes read from `src/api/routes/*.ts` (types: `src/shared/types.ts`, which wins over the
docs). "Same" is path and method as Sowel's; "value" is a difference of status, text or bound that
a decision keeps (listed in `docs/sowel-subset.md`).

| Solite route | Sowel's equivalent | Verdict |
| --- | --- | --- |
| `GET/PUT/DELETE /devices/:id`, `GET /devices` | same (`devices.ts`) | same |
| `GET /zones`, `POST /zones`, `GET/PUT/DELETE /zones/:id`, `GET /zones/aggregation` | same (`zones.ts`) | same |
| `GET/POST /equipments`, `GET/PUT/DELETE /equipments/:id`, `POST /equipments/:id/data-bindings`, `DELETE .../data-bindings/:bindingId`, same for `order-bindings`, `POST /equipments/:id/orders/:alias` | same (`equipments.ts`) | same |
| `GET /recipes`, `GET /recipes/:id` | same (`recipes.ts`, `RecipeInfo`) | same |
| `GET/POST /recipe-instances`, `PUT/DELETE /recipe-instances/:id`, `POST .../enable`, `.../disable`, `.../actions`, `GET .../log` | same (`recipes.ts`) | same |
| `GET/PUT /settings` | same (`settings.ts`) | same (Solite's keys are a subset plus 3 own) |
| `GET /system/timezone` | same (`system.ts`) | value (D41, D44: `source`, `fallback`) |
| `GET /auth/status`, `POST /auth/setup`, `/login`, `/refresh`, `/logout` | same (`auth.ts`) | value (D47 to D49: one account, opaque tokens, setup confirmed at the box) |
| `GET /system/version`, `POST /system/version/check`, `POST /system/update`, `POST /system/restart` | same (`system.ts`) | value (D53, D65, D79: no Docker fields, a file body on update, the box restarts) |
| `GET /ws` | same (`ws`, Sowel's `EngineEvent`) | same (subset of events, D18) |
| `GET /recipes/store` | `GET /plugins/store` (`plugins.ts`, `getStore()` = `StoreEntry[]`) | **different path and shape**: Solite wraps `{recipes, readAt, error}` and adds `size`, `sha256`, `url`, `installed`, `refused`, `builtIn`; Sowel's is a bare array without installed ones |
| (none: Solite's `installed` field) | `GET /plugins` (`PluginInfo[]`, installed integrations and recipes) | **Sowel-only and needed** (R1) |
| `POST /recipes/store/refresh` | `POST /plugins/store/refresh` -> `{count, source}` | **different path and shape** (Solite answers the store) |
| `POST /recipes/install` `{id}` or a signed file | `POST /plugins/install` `{repo, confirmed?, expectedSha256?}` -> `{success: true}` for a recipe | **different path, body and answer**; the file body is Solite's own, kept |
| `POST /recipes/:id/update` | `POST /plugins/:id/update` `{confirmed?, expectedSha256?}` -> `{success: true}` | **different path** |
| `POST /recipes/:id/uninstall` | `POST /plugins/:id/uninstall` -> `{success: true}` | **different path** |
| `GET /system/backup`, `POST /system/backup` | `GET /backup` (ZIP), `POST /backup` (ZIP upload) | **different path**; the content is Solite's (D56) |
| `GET /settings/timezones` | none | Solite-only (D41), later spec |
| `GET/POST /integrations/solite/permit-join` | none (Zigbee2MQTT's own integration settings, no route) | Solite-only (D28), kept |
| `POST /system/factory-reset` | none | Solite-only (D48), later spec |
| `GET /system/supervisor`, `GET /system/resources` | none (Sowel has `GET /health`, `GET /system/mode`) | Solite-only (D58, D78), later spec |

Sowel's routes Solite does not serve, and whether they are needed (all unserved, 404, and not in
this spec): `GET /plugins/sources`, `POST /plugins/sources`, `POST /plugins/sources/remove`,
`/plugins/:id/enable|disable`, `/plugins/:id/oauth/*` (no: the box has one catalogue, no OAuth);
`GET /integrations`, `/integrations/:id/start|stop|refresh|restart` (no); `GET /health` and `GET
/system/sunlight` (worth adding: roadmap lines); `GET /system/mode`, `POST /system/takeover` (no);
`PUT /zones/reorder`, `POST /zones/:id/orders/:orderKey` (a later spec: the interface's
reordering); `GET /devices/battery-alerts`, `/devices/suggest`, `/devices/:id/raw`, `POST/DELETE
/equipments/:id/timed-action` (no); `/me*`, `/users*`, MFA, tokens (no: one account); modes,
history, charts, energy, calendar, dashboard, button actions, shared access, MQTT and
notification publishers, push, logs, audit, camera (out of the true north).

## Where the change goes
- `crates/core/src/api/mod.rs`: `ROUTES` loses the five `/recipes/*` store routes and
  `/system/backup`, gains `GET /plugins`, `GET /plugins/store`, `POST /plugins/store/refresh`,
  `POST /plugins/install`, `POST /plugins/:id/update`, `POST /plugins/:id/uninstall`, `GET` and
  `POST /backup`. `recipe_job_path` (line ~570) matches `["plugins", "install"]`,
  `["plugins", "store", "refresh"]`, `["plugins", id, "update"|"uninstall"]`; the route's first
  segment `plugins` goes to `recipes.rs`; the backup path check (line ~481) and the safe mode's
  exceptions (line ~482 of the living spec: the removal's path) follow.
- `crates/core/src/api/recipes.rs`: `GET /plugins` and `GET /plugins/store` serialise the views
  below straight into the body (no tree of `Value`: the internal RAM, spec 056).
- `crates/core/src/home/install_ops.rs`: `recipe_store()` becomes two views: `installed_plugins()`
  (the installed files: `Vec<PluginInfoView>`, `latestVersion` from the catalogue) and
  `store_entries()` (the catalogue minus the installed, with `compatible`). The `StoreView`
  wrapper, `refused` and `built_in` leave the answer; `refused` is already in
  `GET /system/resources`.
- `crates/core/src/recipes/install/catalogue.rs`: `Entry` gains optional `repo`, `category`,
  `author`, `tags` (bounds written beside them: `repo` 100 characters, 8 tags of 32); `parse`
  checks `repo` (owner/name, unique) and derives `solite-recipes/<id>` when absent.
  `recipes/install/mod.rs`, `job.rs`: the install is looked up by `repo`; `{id}` is no longer read
  from the JSON body (the `Op::Install` carries the id found from the `repo`).
- `crates/core/src/api/mod.rs` (body of `POST /plugins/install`): `application/json` is the
  catalogue install, `application/octet-stream` the file (as today's `/recipes/install`).
- `installedAt`: the file's modification time in `scripts/` (FAT keeps one); the storage adapter
  trait gets no new method if the existing listing already carries it, else one `modified()` on
  the script store (host and firmware, `firmware/src/` only to add the one accessor).
- `crates/host/src/serve.rs`, `firmware/src/http_server.rs` (comments and the path matches of the
  recipe job and backup routes at ~784): follow the core's names. **Build after spec 058 is
  merged** (it works in `crates/core/src/recipes/script/` and `firmware/`).
- `ui/src/api/recipeStore.ts` (paths, types: `PluginInfo`, `StoreEntry` of Sowel's shape, the
  install by `repo`), `ui/src/components/plugins/PluginsPage.tsx` (the Installed tab reads `GET
  /plugins`, the Store tab `GET /plugins/store`; the size and the refusal shown from
  `/system/resources`), `ui/src/components/layout/useRecipeUpdates.ts` (the count: the entries of
  `GET /plugins` with `latestVersion`), the backup card's path (`ui/src/api/` or its component).
- `scripts/box-session.sh` (lines ~906, ~956), `scripts/demo-garage.sh` if it installs, and the
  host's `crates/host/tests/box_session_script.rs` (~2404, ~2418), `crates/core/tests/recipe_install.rs`,
  `memory_worst_case.rs`.
- `docs/sowel-subset.md`, `contract/sowel-snapshot.json` (rewritten by the contract test with
  Sowel's repository beside), `docs/living-spec/README.md`, `docs/decisions.md` (D89),
  `docs/architecture.md`, `docs/roadmap.md`, `docs/roadmap.html`.

## Sowel files each change mirrors
| Change | Sowel file |
| --- | --- |
| `GET /plugins` and `PluginInfo` | `src/api/routes/plugins.ts` (the recipe branch of the listing), `src/shared/types.ts` (`PluginInfo`, `PluginManifest`, `PackageSource`) |
| `GET /plugins/store`, `StoreEntry` | `src/packages/package-manager.ts` (`getStore`, `StoreEntry`), `src/packages/registry-types.ts` (`RegistryEntry`) |
| `POST /plugins/store/refresh` | `plugins.ts`, `package-manager.ts` (`refreshRegistryNow`) |
| `POST /plugins/install`, `:id/update`, `:id/uninstall` | `plugins.ts` (`installBodySchema`, `updateBodySchema`, the recipe branches) |
| `/backup` | `src/api/routes/backup.ts` |

## The old paths: alias for one release, or removed
Recommendation: **removed, no alias**.
- The only client is the interface, built into the same image as the firmware (`ui/dist`,
  precompressed): after an update the box serves the new interface with the new routes at once. The
  one case an old path is called is a browser tab opened before the update, until it is reloaded
  (the page shows an error, `/system/update` keeps working). Scripts in the repository are changed
  in this spec.
- An alias costs what the box is short of: a second set of routes in the `match` and in the
  exceptions of the safe mode, both answer shapes built (the old wrapper is the larger JSON), and
  a deviation row kept in `sowel-subset.md` for a release, which the contract test must then
  tolerate and a later change must remember to remove.
- If the user prefers an alias anyway: `POST /recipes/:id/update` and `/uninstall` can answer by
  calling the new handler (same answers, almost free); the store, install and refresh aliases
  carry the old shapes and are the expensive ones.

## Memory
`GET /plugins` and `/plugins/store` are streamed by the same cursor as the other lists when the
core already has one for them (`ListCursor`), else serialised straight into the body, never built
as a `Value` (spec 056). 8 recipes at most are installed (4 script recipes + refused) and 32 are in
the catalogue: the stores' answers are about 10 KB at most.
