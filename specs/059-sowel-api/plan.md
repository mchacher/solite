# Plan: Spec 059

Two pull requests. PR 1 (steps 1 to 6): the recipes under `/plugins`, core, interface, docs. PR 2
(steps 7 and 8): the backup path and the box run. Build after spec 058 has merged (it works in
`crates/core/src/recipes/script/` and `firmware/`, which steps 5 and 8 touch lightly).

## Steps
- [x] 1. Catalogue: `repo`, `category`, `author`, `tags` in `Entry`, bounds, the `repo` checks
      (unique, `owner/name`, derived `solite-recipes/<id>`), tests.
- [x] 2. Core views and routes: `installed_plugins()` and `store_entries()` in `install_ops.rs`,
      `GET /plugins`, `GET /plugins/store` in `api/recipes.rs`, `ROUTES`; `installedAt` from the
      script store's file date.
- [x] 3. The jobs: `POST /plugins/install` (JSON by `repo`, octet-stream by file), `/plugins/store/refresh`
      (`{count, source}`), `/plugins/:id/update`, `/plugins/:id/uninstall`; safe mode's exception;
      the five old routes removed; the existing `recipe_install.rs` tests moved to the new paths.
- [x] 4. Host and firmware path matches (`serve.rs`, `http_server.rs`): the new names; the
      host's `box_session_script.rs` and the scripts (`box-session.sh`) on the new paths.
- [x] 5. Interface: `recipeStore.ts` types and paths, `PluginsPage`, `useRecipeUpdates`; Vitest.
- [x] 6. Docs: `sowel-subset.md`, snapshot rewrite, living spec, D89, architecture, roadmap;
      `sh scripts/validate.sh` (QEMU included); PR 1.
- [x] 7. `/backup` at Sowel's path (core route, host and firmware matches, the backup card, the
      backup tests, `box-session.sh`), docs; validate; PR 2.
- [ ] 8. On the box (the user runs `box-session.sh`): install, update, remove through the new
      paths; a backup and its restore; the internal RAM's lowest point not lower than spec 056's.

## Test plan
| Module | Scenario | Expected |
| --- | --- | --- |
| catalogue | an entry with `repo`, `category`, `author`, `tags` | kept, served as Sowel's `StoreEntry` |
| catalogue | a 0.3 catalogue (no `repo`) | reads; `repo` is `solite-recipes/<id>` |
| catalogue | duplicate `repo`; `repo` without a slash | refused as a whole, `Invalid catalogue: <id>: repo` |
| api plugins | `GET /plugins` with one installed recipe, a newer one in the catalogue | one `PluginInfo`, `type: "recipe"`, `latestVersion` set, `status: "connected"` |
| api plugins | `GET /plugins/store`, 2 entries, 1 installed | 1 entry, `compatible`, `tier: "official"`, no `installed`, no `url` |
| api plugins | an entry with `minVersion` above the running one | `compatible: false`, `compatReason: "Requires Solite >= x"` |
| api plugins | `POST /plugins/store/refresh` | `{count, source: "remote"}`; a second one within a minute 429; a failed read `source: "local"` |
| api plugins | `POST /plugins/install {repo}` | 200 `{"success": true}`; the recipe moves from the store to `GET /plugins` |
| api plugins | install with an unknown `repo`, a malformed `repo`, an installed `repo`, a body without `repo` | 404 `Plugin not found`, 400, 409 `Recipe already installed`, 400 |
| api plugins | `POST /plugins/install` with a signed file (octet-stream) | installed, `{"success": true}` |
| api plugins | `POST /plugins/:id/update`, with and without a body | updated, `{"success": true}`; nothing newer 400 |
| api plugins | `POST /plugins/:id/uninstall` | removed, instances kept stopped |
| api plugins | the five old routes | 404 or 405, never a recipe job |
| api recipes | `GET /recipes/store` | 404 `Recipe not found` (as `/recipes/:id`) |
| safe mode | `POST /plugins/:id/uninstall` and `/system/update` in safe mode | pass; `POST /plugins/install` 503 |
| backup | export and restore at `/backup`; `/system/backup` | as before; the old path 404 |
| contract | `sowel_contract.rs` with Sowel's repo and the snapshot | green; no recipe or backup route in Deviations |
| ui | Plugins page: list, install, update, remove, refresh; the sidebar's count | as before, on `/plugins` |
| ui | a refused recipe | its reason read from `/system/resources` |
| scripts | `box-session.sh --api-only <port>` on the Mac | green with the new paths |
| box | install, update, remove, backup, restore | pass; internal RAM lowest over 64 KB |

## Validation
- `sh scripts/validate.sh` green, QEMU included.
- `cargo test -p solite-core --features script-recipes --test sowel_contract` with Sowel's
  repository beside this one.
