# Spec 057: what one script engine costs, measured (engine study, part 2)

Written in an autonomous run (user, 2026-11-28: "ok pour faire l'analyse au niveau moteur",
"enchaîne en autonomie"). Measures only: the engine does not change; the choice it prepares is the
user's, figures in hand.

## Context
Each script recipe has its own QuickJS runtime and context (spec 041, D74), held to
`SCRIPT_MEMORY_LIMIT` (224 KB) in PSRAM; `MAX_SCRIPT_RECIPES` is 4 (D85). An empty script with the
shim holds about 85.8 KB (`docs/memory-budget.md`), so 4 recipes spend about 344 KB on the same
fixed part. The study of spec 056 found that QuickJS bounds memory per runtime, not per context,
and that much of the fixed part may be per context (the intrinsics, the compiled shim), not per
runtime: one runtime with several contexts would then save little, and the large saving would need
several recipes in one context. Which of the two is true decides whether 6 to 8 recipes can fit;
it is not measured.

## Goals
- The fixed cost split into its parts: the runtime, a context with its intrinsics, the shim, a
  script, an instance; on the target (QEMU, 32-bit) and on the Mac.
- What 4 recipes would hold in three layouts: one runtime each (today), one runtime with a context
  each, one runtime and one context for all (each recipe in its own function scope).
- Whether Sowel's recipes (the 7 of `recipes/scripts/`) run unchanged in the shared-context layout.
- A proposal, with its cost in isolation and in bounds, for the user to decide.

## Out of scope
- Changing the engine, the limits or `MAX_SCRIPT_RECIPES` (a spec of its own, after the choice).

## Requirements
### R1: the parts of the fixed cost
- A measuring path, in tests on the Mac and in the QEMU run's script phase (no extra QEMU run),
  reads QuickJS's own count (`memory_usage()`: bytes used, objects, atoms, functions) after each
  of: an empty runtime; a context with the engine's intrinsics; the shim evaluated; an empty
  script's `recipe()`; `motion-light` loaded; one instance started.
### R2: three layouts, 4 recipes
- The same count for 4 recipes (`motion-light`, `state-watch`, `state-trigger-light`,
  `schedule-on-off`) with one instance each, in the three layouts of the goals. The shared-context
  layout wraps each source in its own function scope; the measure says what it holds and whether
  the four load and start.
### R3: Sowel's recipes in a shared context
- On the Mac, each of the 7 recipes loads and starts in the shared-context layout beside the
  others, its info unchanged; any that does not, with the reason (a top-level name, `this`, ...).
### R4: the proposal
- The figures in this spec's result and in `docs/memory-budget.md`; a proposal (how many recipes
  each layout allows within the PSRAM budget and the install admission rule, what isolation and
  which bound each keeps: the memory limit per recipe, the time limit, a failure's reach).

## Living spec delta
### docs/living-spec/README.md
- None: the engine does not change (the measuring path is in tests and the QEMU image only).
### docs/memory-budget.md
- The fixed cost's parts and the three layouts' figures.

## Acceptance criteria
- [ ] R1's six figures, QEMU and Mac, in the result.
- [ ] R2's three layouts measured in QEMU and on the Mac.
- [ ] R3: the 7 recipes in a shared context, each loaded and started or its reason.
- [ ] R4: the proposal written, the user's choice asked.
- [ ] `validate.sh --pre-merge` green (critical: QEMU included); no change to the box's image.

## Edge cases
- The measuring path must not reach the box's image (feature-gated, the QEMU image only): the
  images' symbol checks stay as they are.
- A layout that fails to load a recipe is a figure, not a failure of the spec: written with its
  reason.

## How the user will check it
The table of the result, and the proposal.

## Result
Measured 2026-11-28. QEMU: `sh scripts/qemu.sh`, the `engine study:` lines of the script phase (the
PSRAM allocator's own byte count, 32-bit). Mac: `cargo test -p solite-core --features script-recipes
--test engine_layouts -- --nocapture` (`malloc_size`, 64-bit, about 1.4 times the target's). Both run
the same code (`crates/core/tests/fixtures/engine_study.rs`): rquickjs directly, the engine's
intrinsics (`Date, Eval, Json, MapSet, Promise`), the shim text as the loader compacts it, a
stand-in for the engine's native host functions (nine JavaScript closures, about 3 KB), no limit
but the 16 KB stack (32 KB on the Mac, see the findings). Each line is after a collection.

### R1: the parts (bytes, each line cumulative; `motion-light` in its own runtime from the shim on)
| Part | QEMU | Mac |
| --- | --- | --- |
| Empty runtime | 19,104 | 29,048 |
| + context with the intrinsics | 53,444 (context +34,340) | 77,992 |
| + host stand-in | 56,204 | 81,968 |
| + shim evaluated | 91,448 (shim +35,244) | 129,176 |
| + empty `recipe()` | 92,024 | 129,992 |
| `motion-light` loaded | 113,912 (script +21.9 KB) | 161,096 |
| One instance started | 123,864 (+9,952) | 175,632 |
| Two instances started | 128,988 (+5,124) | 183,976 |

Per runtime: 19 KB. Per context: 34 KB (intrinsics) + 35 KB (the compiled shim and its closures) =
69 KB. So of the 92 KB fixed part, 21 % is the runtime, 79 % the context.

### R2: four recipes (`motion-light`, `state-watch`, `state-trigger-light`, `schedule-on-off`), one instance each
| Layout | QEMU | Mac | Saved against today (QEMU) |
| --- | --- | --- | --- |
| One runtime each (today) | 474,252 (largest 144 KB) | 677,528 | |
| One runtime, a context each | 353,616 | 496,248 | 120,636 (25 %, 40 KB per extra runtime) |
| One runtime, one context (shim compiled once as a factory) | 213,676 | 305,112 | 260,576 (55 %, 87 KB per extra recipe) |

In the shared layout the base (runtime, context, factory) is 85,836 bytes in QEMU and each recipe
adds 42.0, 26.9, 22.7 and 36.2 KB (script and one instance). All four load and start in every
layout. Atoms (names) are the one thing a runtime shares: 3,264 with a runtime each, 1,051 with
contexts, 1,054 shared.

### R3: Sowel's 7 recipes in one shared context (Mac)
All seven (`motion-light`, `state-watch`, `state-trigger-light`, `schedule-on-off`,
`motion-light-dimmable`, `presence-thermostat`, `switch-light`) load and start, and the info each
declares is byte for byte the one it declares alone. Together 425,800 bytes on the Mac (about 300 KB
in QEMU's counting). The recipes are unchanged: the wrapper gives each source its own `recipe`,
`console` and `Date` as parameters of its function.

### Findings that are not in the tables
- The shim is written for one recipe per context (`recipe()`, `console` and `Date` are globals, its
  entry point a non-configurable global): a shared context needs the shim as a factory (done here as
  a text transform, not in the engine) and a wrapper that passes the three names.
- At the engine's 16 KB stack limit the wrapped layout fails to load on the Mac (`Maximum call stack
  size exceeded`, 64-bit frames; 20 KB passes); in QEMU it loads at 16 KB. The engine's own path is
  unchanged at 16 KB.
- QuickJS counts per runtime (`memory_usage()`): no figure per context exists, so a limit per
  recipe in a shared runtime has nothing to read.

### R4: the proposal (arithmetic over the figures; the user decides, nothing is decided here)
PSRAM for scripts, realistic home: 2048 KB minus 610 KB for the rest minus the 512 KB reserve = about
926 KB. A recipe costs, with 1 instance, 119 KB (one runtime each), 77 KB more per recipe (contexts),
32 KB more per recipe (shared); with 4 instances add 30 KB. The admission rule (a runtime's limit plus
512 KB free) is applied to the groups below.

| Layout | Recipes that fit, 1 instance each | 4 instances each | Isolation and bounds kept |
| --- | --- | --- | --- |
| One runtime each (224 KB limit each) | 6 (the admission rule stops the 7th) | 5 | All: memory limit per recipe, 200 ms per call, a crash or out-of-memory reaches that recipe only, one recipe is updated or removed alone |
| One runtime, a context each (a group limit, say 640 KB) | 8 to 11, by the group's limit | 8 | Separate globals and prototypes; one memory limit for the group: a recipe that fills it makes the others' allocations fail, an out-of-memory fails the group; the time limit per call holds; no figure per recipe |
| One runtime, one context | 13 to 27 | 13 | Function scopes only: a script can reach the globals and the prototypes the others use; the shim's hiding of the engine's functions has to be redone; one failure or out-of-memory fails all; one recipe cannot be updated alone |

Worst-case home (1206 KB for the rest): the rule admits one runtime each layout A recipe only; the
paper ceiling is not met by any layout beyond that. Reading: the contexts layout buys 25 % for a
large loss of the per-recipe bound; the shared layout buys 55 % and the trust boundary, which the
recipes installed from a catalogue (spec 042) rely on. A middle path is to keep one runtime each and
raise `MAX_SCRIPT_RECIPES` to 5 or 6 on the realistic figure (5 with 4 instances each), or to share
one runtime only between the recipes of the catalogue that are signed by the project. Which one is
the user's choice.

### Not measured
The engine's native host functions (a stand-in of 3 KB is counted), the 7 recipes in QEMU, a call's
peak (the collector, events and timers running), the internal RAM, heap fragmentation after
installs and removals, the cost in time, recipes with long slot lists. The box is untouched.
