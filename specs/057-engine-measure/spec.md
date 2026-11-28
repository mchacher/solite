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
