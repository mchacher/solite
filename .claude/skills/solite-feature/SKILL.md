---
name: solite-feature
description: |
  Builds a new feature for Solite, spec first. Use when the user types
  /solite-feature, or asks to add, create or change something in the tool
  ("ajouter la recette motion-light", "gérer les volets", "add the zone aggregation",
  "ajouter une fonctionnalité", "je voudrais que le boîtier...").
argument-hint: "[the idea, in one sentence]"
---

# Solite Feature Workflow

Idea: $ARGUMENTS

The user decides **what**; you handle **how**. They review the spec before the code and the demo
after it, not the code itself: the checks and the review agent do that. They often cannot see when
a step was skipped, so **you never skip one**.

**Models** ([docs/agents.md](../../../docs/agents.md)): Phase 2's spec on Sonnet when it follows an
existing pattern, on Opus when it is hard (a new block, a choice of architecture or data model,
several options to weigh): say which, and ask the user to `/model opus` before it and
`/model sonnet` after the approval. Every agent you launch names its `model:`.

Follow EVERY phase below IN ORDER. Each phase ends with a GATE: a checklist that MUST be fully true
before going on. Gates marked **STOP** wait for the user's explicit yes ("yes", "oui", "go"),
whatever the permission mode, Auto included. Do NOT combine phases.

- **Language**: talk with the user in their language, plainly; write every file in English.
- **Rules**: everything about code style, structure and git is in `CLAUDE.md`. Read it; do not
  repeat it here. Templates and key paths: [reference.md](reference.md).
- **One feature per conversation.** If the user brings a second idea or a new need, add it to the
  roadmap inbox (see `CLAUDE.md`) and finish this one first.

---

## Phase 1: Understand & Clarify

### 1.1 Read, in this order

| File | Why |
| --- | --- |
| `CLAUDE.md` | Rules, commands, where things are |
| `docs/living-spec/README.md`, then only the pages it points to that the idea touches | What the tool does today: the feature must fit in it |
| `docs/specs-index.md` | A past spec may already cover part of the idea |
| `docs/decisions.md` | Choices already made, and why: do not reopen one silently |
| `docs/roadmap.md` | Where this feature sits, and what comes after it |
| `docs/architecture.md` | The blocks the feature will touch |
| `crates/core/src/` (start at `lib.rs`), and the adapter traits it defines | The model and the engine: the feature's logic goes here |
| The block the feature touches: `crates/host/src/`, `firmware/src/`, `ui/src/` | How the core is wired to the Mac, the ESP32, the browser |
| `docs/design-system.md`, if the feature changes the interface | Sowel's look, the embedded changes, the budget |
| In the Sowel repository (`../sowel`, see "Sowel contract" in `CLAUDE.md`): `src/shared/types.ts`, the page of `docs/technical/data-model/` for each entity the feature touches, and `docs/technical/api-reference.md` for an API change | The names, types and JSON the feature must mirror; `types.ts` wins over the docs. For a recipe: its `sowel-recipe-*` repository (slots, behaviour). If `../sowel` is missing, ask the user where Sowel is cloned |
| `inputs/note-de-cadrage-mini-sowel.md`, the sections the feature touches | The intent behind the roadmap item |

If the user put documents about this feature in `inputs/`, read them too, all of them.

**To save tokens**, do not read the table yourself: launch one `Explore` agent with
`model: "sonnet"`, give it the idea and this table, and ask for a brief of at most about 80
lines: what exists today that the idea touches, the past specs and decisions that bear on it,
the roadmap item, the Sowel types and fields to mirror (with their file), and the code to reuse
(`file:line`). Read yourself only the few files the brief points to as decisive.

### 1.2 Clarify until nothing is assumed

First restate the idea in two sentences: what changes for the user, and why. Then go through these
topics and ask, **at most three questions per message**, about what is unclear. Give your
recommended answer when you have one.

| Topic | What you need to know |
| --- | --- |
| **What** | The expected behaviour, described as the user would see it |
| **Why** | The problem it solves, for whom; how it serves the objective in the living spec |
| **Scope** | What is in; what is explicitly out (it goes to the roadmap) |
| **Data** | What goes in and out, formats, units; new data to store; where it comes from |
| **Rules** | Calculations, thresholds, choices the tool makes; their source (a standard, a document) |
| **Interface** | New screens, buttons, files, reports; changes to existing ones |
| **Edge cases** | Missing, empty or wrong data; very large inputs; the device or service unavailable |
| **Check** | How the user will know it works: the demo they expect to see, with which data |
| **Sowel contract** | Which Sowel types, fields, events or recipe slots it mirrors, with their file in the Sowel repository; any divergence from Sowel, and why (a divergence is a decision) |
| **Where it runs** | Mac only (host), ESP32 (firmware), interface, or several; which adapters it needs (radio, storage, HTTP/WS, OTA) |
| **Hardware** | Simulated radio, the Dongle Max's radio over TCP, a USB EFR32 dongle, or the box itself; which real device (model) is needed for the demo, and whether the user has it. Flashing the Dongle Max needs the user's explicit yes (decision D6) |
| **Memory and size** | What grows with devices, events or history, and its bound (the ESP32 has 2 MB of PSRAM); for the interface, its effect on the 170 KB gzip budget |
| **Security** | Signatures, keys, passwords, network exposure; no key in the repository |

Stop asking when you could write the spec without guessing. Ask the user if they have anything to
add before you write it.

### 1.3 Look for existing patterns

Search the code for something similar (a comparable screen, calculation, file reader) and reuse its
approach. Note what you found; it goes in `architecture.md`.

> **GATE 1**: all true before going on:
>
> - [ ] 1.1 Every file of the table read, and the `inputs/` documents if any
> - [ ] 1.2 Idea restated and confirmed by the user; every topic clear, or answered "not
>       applicable"
> - [ ] 1.3 Existing patterns searched; what to reuse noted

---

## Phase 2: Write the Spec

### 2.1 Spec folder

If this increment already has a spec folder (its row in `docs/specs-index.md` is not "done", for
example spec 001 written at setup), use it: read it again, complete it with what Phase 1 taught,
and keep its number. Otherwise:

```bash
ls specs/ | tail -1          # last number
mkdir specs/NNN-<short-name> # next number, three digits, kebab-case name
```

### 2.2 Three files, from [reference.md](reference.md)

| File | Content |
| --- | --- |
| `spec.md` | Context, goals, out of scope, requirements, **living spec delta**, acceptance criteria, edge cases, how the user will check it |
| `architecture.md` | Overview, data, the files that change and how, what is reused (1.3) |
| `plan.md` | Steps in order, the **test plan**, the validation checklist |

### 2.3 Living spec delta (MANDATORY)

`spec.md` says exactly what this feature adds or changes in `docs/living-spec/`, page by page, in
business words, including the sentences it makes false (search for them: see 3.4). A feature that changes nothing the user can see or rely on says so, with the
reason. The delta is applied first in Phase 3, so the living spec never lags behind the code.

### 2.4 Test plan, before any code (in `plan.md`)

For each module that gets new or changed logic, list the scenarios:

- **Nominal**: the normal case works as expected;
- **Edge cases**: those of 1.2, one scenario each (missing, empty, wrong, too large);
- **Unchanged behaviour**: what already worked still works, when the feature touches existing code.

Written as a table `Module | Scenario | Expected`. Every scenario becomes one test in Phase 3.

### 2.5 Index

Add the spec's row to `docs/specs-index.md`, status "in progress".

### 2.6 Spec review, by an agent that did not write it

Launch the `spec-reviewer` agent (Agent tool, `subagent_type: spec-reviewer`) on the spec folder.
Fix what it says must be fixed. Its questions for the user go into the summary below, under
**Open questions**. If you disagree with a finding, keep the spec and say why in the summary.

### 2.7 Present the summary, in the user's language, in this format (translate the labels)

```
## Spec NNN: <name>

**What changes for you**: <two sentences, no jargon>
**In scope**: <list>
**Out of scope**: <list, gone to the roadmap>
**How you will check it**: <the demo you will see at the end>
**Tests planned**: <number> scenarios on <modules>
**Living spec**: <what changes in docs/living-spec/, or "nothing, because ...">
**Open questions**: <from the spec review, or "none">

Shall I build it?
```

> **GATE 2 (STOP)**: all true before going on:
>
> - [ ] 2.1 `specs/NNN-<name>/` exists
> - [ ] 2.2 `spec.md`, `architecture.md`, `plan.md` written
> - [ ] 2.3 Living spec delta written, or "no change" with its reason
> - [ ] 2.4 Test plan in `plan.md`, one scenario per edge case of 1.2
> - [ ] 2.5 Row added to `docs/specs-index.md`
> - [ ] 2.6 `spec-reviewer` ran; its "to fix" items fixed, or the disagreement stated
> - [ ] 2.7 Summary presented in the format above, open questions included
> - [ ] The user explicitly said yes. If they asked for changes: spec updated, summary presented
>       again.

---

## Phase 3: Branch & Build

### 3.1 Branch (MANDATORY)

```bash
git checkout main
git pull                                  # only if there is a remote
git checkout -b feat/NNN-<short-name>
git branch --show-current                 # must NOT be main
```

### 3.2 Build in this order

1. **Living spec**: apply the delta to `docs/living-spec/`.
2. **Code**, in this order:
   1. types of the core, mirroring Sowel's (`serde`, Sowel's field names);
   2. logic in `crates/core`, behind adapter traits when it needs hardware;
   3. adapters: the simulated or Mac one in `crates/host` first, then the ESP32 one in
      `firmware/` if the spec asks for it;
   4. API or WebSocket, in Sowel's format, if the spec asks for it;
   5. interface in `ui/`, with Sowel's tokens and classes (port a Sowel component when one fits);
      take a screenshot of each screen changed, for the demo.
   If a step does not apply to the feature, write "not applicable" against it in `plan.md`.
3. **Tests**: see 3.3.
4. **Docs**: see 3.4.

**To save tokens**, steps 2 to 4 go to the `implementer` agent (Agent tool,
`subagent_type: implementer`, Sonnet, set in its file): it holds the strict rules (output through
`quiet.sh`, nothing outside the spec, no commit, stop after three identical failures) and the
report format. Give it the spec folder, not your conclusions, the branch, the files and lines to
start from, and a report path in the session's scratchpad. Read the report only when its one-line
answer is not enough. Take the work back on Opus only if it stops on the same failure (see
[docs/agents.md](../../../docs/agents.md)). Then you run the checks yourself.
**Build it yourself instead** when the brief would cost more than the work: a change of two or
three files, one that needs back-and-forth with the user, or a run on the box.

Do only what the spec says. No unrelated refactor, no extra option, no "while I was there". If the
spec turns out wrong or incomplete, stop, tell the user, update the spec, and get their yes again.

### 3.3 Tests (MANDATORY, never skipped)

- **One test per scenario** of the test plan. Check at the end that none is missing.
- **Look at the existing tests first** and follow them: structure, naming, how data is prepared.
- **What to test**: logic, calculations, rules, file reading and writing, anything that decides
  something. **What not to test**: trivial code with no decision in it.
- **Where**: Rust: `cargo test`, integration tests in `crates/<crate>/tests/<topic>.rs`, unit tests
  of private logic in a `#[cfg(test)] mod tests` beside it; test names say the behaviour
  (`motion_turns_the_light_on`). Interface: Vitest and Testing Library, `ui/src/**/<Name>.test.tsx`
  beside the component. The firmware has no tests: its logic lives in the core, tested on the Mac.
- **This project also tests**:
  - **Sowel's JSON**: a type exposed by the API is serialized in a test and compared with a JSON
    example written from Sowel's `types.ts` (field names, casing, enum values);
  - **Radio scenarios**: logic driven by devices is tested with the simulated radio, replaying a
    scenario (a sensor joins, a temperature, motion), never with a real device in `cargo test`;
  - **Bounds**: a collection that grows has a test at its bound (what happens one past it);
  - **Real hardware**: when the spec needs a real device or the box, the run on it is part of the
    demo, and its result (date, device, what was seen) is written in the spec's "How the user will
    check it".
- A test must fail if the feature is broken: check it once by breaking the code on purpose, then
  restore it.

```bash
cargo test -p solite-core --test <file name without .rs>   # one Rust test file
cd ui && npx vitest run src/<path>/<Name>.test.tsx              # one UI test file
```

### 3.4 Docs to update with the code, in the same branch

| If the feature... | Update |
| --- | --- |
| Changes what the tool does | `docs/living-spec/` (done in 3.2.1) |
| Changes the structure, adds a block or a dependency | `docs/architecture.md` |
| Involves a choice someone could question later | `docs/decisions.md`: a new decision |
| Was on the roadmap, or reveals new items | `docs/roadmap.md` and `docs/roadmap.html` |
| Changes how to install, run or use the tool | `README.md` |
| Changes a command or a rule Claude must know | `CLAUDE.md` |
| Adds a colour, a token or a component, or changes the interface's budget | `docs/design-system.md`, and the token or theme file in `ui/src/styles/` |
| Mirrors a new Sowel type, field or event, or diverges from Sowel | The spec names the Sowel file; a divergence gets a decision in `docs/decisions.md` |
| Needs something on the Sowel side | A line in the roadmap inbox: "Sowel issue: ...", for the user to open |

Then correct **what this change makes false** elsewhere. The table finds the docs the feature
adds to; it does not find a sentence, written by an earlier change, that this one makes untrue,
and the diff never shows it. Search for it: `grep -rn -i` in `docs/` (roadmap and
decisions excepted: they look ahead and record history), `README.md`, `CLAUDE.md`,
`inputs/README.md` and `.claude/` (skills and agents), for the names this feature touches (blocks,
files, commands, screens, settings) and for this increment's id and name. Rewrite in the present
tense every sentence that is no longer true; list them in the spec's "No longer true". A decision
this change makes obsolete gets a new decision that replaces it (`docs/decisions.md`), never an
edit. The rule and its check are in `CLAUDE.md` ("Docs say what is true today").

> **GATE 3**: all true before going on:
>
> - [ ] 3.1 On `feat/NNN-<name>` (checked with `git branch --show-current`)
> - [ ] 3.2 Living spec updated first; code in the order; nothing outside the spec
> - [ ] 3.3 Every test-plan scenario has its test; none missing
> - [ ] 3.4 Every doc of the table updated where it applies; what this change makes false searched
>   for and corrected

---

## Phase 4: Check

```bash
sh scripts/validate.sh --pre-merge   # scaled to the change; critical paths run in full, QEMU included (D87)
```

One command, one verdict: the docs check; Rust format and lint (core, host, firmware); Rust tests;
the interface's format, lint (Sowel's design rules, no raw colour), types, tests, build and size
budget; the ESP32 firmware build. **It
must be green.** Show the user the command and the last lines of its output, never just "it works".
If it fails: fix the cause and re-run. Never weaken a check, skip a test or delete one to get green.

> **GATE 4**: all true before going on:
>
> - [ ] Validate command green, output shown to the user
> - [ ] No check weakened, no test skipped or removed

---

## Phase 5: Agent Review (before the demo)

Launch the `code-reviewer` agent (Agent tool, `subagent_type: code-reviewer`): it did **not**
write the code, and it holds the review checklist (`.claude/agents/code-reviewer.md`). Tell it:
"feature", the branch, and the spec folder. It reports findings ranked by severity, each with
`file:line` and a concrete failure scenario.

Fix every blocking finding. For each other one, fix it or say why not. If code changed, re-run
Phase 4. Summarise the review to the user in a few lines: what was found, what was fixed.

> **GATE 5**: all true before going on:
>
> - [ ] `code-reviewer` ran on the branch diff, with the spec folder
> - [ ] Blocking findings fixed, or deferred with the user's explicit agreement
> - [ ] Phase 4 re-run and green if code changed
> - [ ] Review summarised to the user

---

## Phase 6: Demo

Show the user that it works, **in their terms**, as the spec's "How the user will check it" says:
run the tool and show the screen, the file produced, the number computed, with their data or the
spec's example. On the Mac: `cargo run -p solite-host` (with the simulated radio's scenario,
or the Dongle Max's radio over TCP, as the spec says), and show its output or the API's answers.
Interface: `cd ui && npm run dev`, open the screen, and show screenshots of every screen changed,
in the Hybrid and Dark themes. ESP32: the firmware build and, only with the user's yes to flash,
`cd firmware && cargo run --release` (flashes and opens the serial log). A run on the box is one
pass of the flow, never a loop, inside `sh scripts/box.sh saves --budget <n>` (decision D39), and
its `flash:` count goes into the spec's result. With real hardware, say
which device was used and write the result in the spec. Compare out loud with what the spec
promised. If something differs, say it plainly.

> **GATE 6 (STOP)**:
>
> - [ ] Demo shown, compared with the spec
> - [ ] The user says it is what they wanted. If not: back to the phase that fixes it (spec, code
>       or test), then Phases 4 to 6 again

---

## Phase 7: Close, Commit, Pull Request, Merge

### 7.1 Close the spec

Tick the acceptance criteria in `spec.md` and the steps in `plan.md`; set the row of
`docs/specs-index.md` to "done".

Then **review the roadmap**: follow Phase 3, *Quick review*, of
`.claude/skills/solite-roadmap/SKILL.md`. The demo is the best moment to ask
whether this feature gave the user a new need. The roadmap change goes into this feature's commit.

### 7.2 Commit

```bash
git add <the files of this feature, by name>
git commit -m "feat(<scope>): <what changes, in English>

<why, in one or two lines>"
```

Conventional commit. **No AI attribution line**: no `Co-Authored-By`, no "Generated with". If
the feature leaves `docs/living-spec/` unchanged, add the line `Living-Spec: none — <reason>`
(see `CLAUDE.md`): the commit is refused without it.

### 7.3 Pull request (only if there is a remote)

```bash
git push -u origin feat/NNN-<short-name>
```

On GitHub (decision D9): `gh pr create --base main --title "<title>" --body "<body>"`, then
`gh pr checks --watch` and report the CI result in plain words: a red CI blocks the merge. Title: `feat(<scope>): <description> (spec
NNN)`. Body: summary, living spec delta (or the `Living-Spec: none — <reason>` line), changes,
test plan with the validate output, review outcome, demo seen and approved. Share the link with the user. Without a remote, skip this step.

### 7.4 Merge, on approval only

Ask: "Shall I merge into main?" Only after an explicit yes:

```bash
git checkout main
git merge --squash feat/NNN-<short-name>
git commit -m "<the PR title>" -m "<the Living-Spec line, if the feature has one>"
git push                                                 # only if there is a remote
git branch -D feat/NNN-<short-name>                      # and the remote branch, if any
```

Confirm to the user in one sentence, and say what is next on the roadmap: the next increment, or
a full review with `/solite-roadmap` if one is due.

> **GATE 7 (STOP)**: all true at the end:
>
> - [ ] 7.1 Spec and plan ticked, index set to "done"; roadmap reviewed with the user
> - [ ] 7.2 Committed on the branch, no AI attribution
> - [ ] 7.3 Pushed and pull request shared, if there is a remote
> - [ ] 7.4 The user explicitly approved the merge; merged; branch deleted; `main` up to date

---

## Gate Summary

| Gate | Condition | If skipped |
| --- | --- | --- |
| 1 | Idea clear, nothing assumed | The wrong thing gets built |
| 2 | Spec, delta and test plan approved | The code drifts from what the user wants |
| 3 | Branch, living spec first, every scenario tested | `main` broken; the living spec lags behind |
| 4 | Validate command green | Broken checks reach `main` |
| 5 | Independent review, blockers fixed | Bugs and scope creep reach the user |
| 6 | Demo approved by the user | "Done" without the user seeing it work |
| 7 | Spec closed, roadmap reviewed, merge approved | No traceability; new needs lost; unauthorised merge |
