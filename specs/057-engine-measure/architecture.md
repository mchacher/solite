# Architecture 057

- No engine change. The measures use rquickjs directly in a test of `crates/core` (with the shim
  from `crates/core/src/recipes/script/shim.js` and the recipes of `recipes/scripts/`), and in the
  QEMU image's script phase (`firmware/src/script_phase.rs`, feature `sim,scripts`), through the
  firmware's PSRAM allocator so the figures are the target's.
- The shared-context layout wraps a source as `(function () { <source> })();` evaluated in one
  context whose `recipe()` registers into a list; nothing of it reaches `ScriptRecipe`.
