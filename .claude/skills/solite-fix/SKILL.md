---
name: solite-fix
description: |
  Fixes a bug in Solite: reproduce it, pin it with a failing test, fix the
  cause, show it fixed. Use when the user types /solite-fix, or reports
  something wrong ("la lumière ne s'éteint pas", "le firmware redémarre en boucle", "ça plante quand...", "bug").
argument-hint: "[what goes wrong, in one sentence]"
---

# Solite Fix Workflow

Problem: $ARGUMENTS

A fix changes behaviour the user relies on: **prove the bug before touching the code, and prove
the fix after**. Follow EVERY phase IN ORDER; each ends with a GATE checklist that MUST be fully
true before going on. Gates marked **STOP** wait for the user's explicit yes, whatever the
permission mode. Talk in the user's language, plainly; files in English. Rules: `CLAUDE.md`.

---

## Phase 1: Understand & Reproduce

### 1.1 Read

`CLAUDE.md`, the part of `docs/living-spec/` that describes the behaviour, and `docs/specs-index.md`
for the spec that introduced it.

### 1.2 Ask, at most three questions per message

| Topic | What you need |
| --- | --- |
| **Steps** | What the user did, in order |
| **Expected** | What should have happened, and why they expect it |
| **Got** | What happened instead: message, wrong value, crash, screenshot |
| **Data** | The file or values that trigger it (a copy in `inputs/` is fine) |
| **Since when** | Always, or since a recent change |

### 1.3 Reproduce

Reproduce it yourself, with the user's data or the smallest case that shows it. If you cannot, say
what you tried and ask for more. Never fix a bug you have not seen.

### 1.4 Bug or feature?

Decide with the user which is right: the code, or the living spec. If the code does what the living
spec says, or the living spec is silent, this is a **change of behaviour**, not a fix: suggest
`/solite-feature`, and stop here.

> **GATE 1**: all true before going on:
>
> - [ ] 1.1 Living spec read for this behaviour
> - [ ] 1.2 Steps, expected, got and data known
> - [ ] 1.3 Bug reproduced by you
> - [ ] 1.4 Agreed with the user: it is a bug, and the expected behaviour is clear

---

## Phase 2: Pin It with a Test

```bash
git checkout main
git pull                                 # only if there is a remote
git checkout -b fix/<short-name>
git branch --show-current                # must NOT be main
```

Write a test that reproduces the bug, next to the existing tests of that code and in their style
(Rust: `cargo test`, in `crates/<crate>/tests/` or the module's `#[cfg(test)] mod tests`, with the
simulated radio when devices are involved; interface: Vitest, `ui/src/**/<Name>.test.tsx`; a bug
seen only on the ESP32: reproduce its logic in a core test, and say so). Run it: it must **fail**, and fail for the reason of the
bug, not for another one.

> **GATE 2**: all true before going on:
>
> - [ ] On `fix/<name>`
> - [ ] New test written, failing for the right reason (output shown)

---

## Phase 3: Fix the Cause

Output (strict rule): tests, builds and the full check run through `sh scripts/quiet.sh <command>`, never bare, and every agent prompt says so (see `CLAUDE.md`).
Models (table in `CLAUDE.md`): stay on Sonnet; a fix that needs a design decision is Opus's
(ask the user to `/model opus`), and any agent you launch names its `model:`.

1. Find the **cause**, not only the symptom. Explain it to the user in two plain sentences.
2. Fix it with the smallest change that removes the cause.
3. Look for the same mistake elsewhere in the code; tell the user what you found, and fix it in the
   same branch only if it is the same bug.
4. If the bug revealed something the living spec did not say, add it to `docs/living-spec/`. Then
   correct what the fix makes false elsewhere, with the search of the feature skill (3.4), on the
   names the fix touches.
5. Run the new test: it passes. Run `sh scripts/validate.sh --pre-merge`: everything green (D87). Never weaken a
   check or remove a test to get there.

> **GATE 3**: all true before going on:
>
> - [ ] Cause explained to the user
> - [ ] Smallest fix; same mistake searched elsewhere; what the fix makes false in the docs corrected
> - [ ] New test passing; validate command green, output shown

---

## Phase 4: Review & Show

1. Launch the `code-reviewer` agent (Agent tool, `subagent_type: code-reviewer`): it did not
   write the fix, and it holds the checklist. Tell it: "fix", the branch, and the bug as the user
   described it, with its cause. Fix blocking findings; re-run the checks if code changed.
2. Show the user the bug fixed, on their own case.

> **GATE 4 (STOP)**:
>
> - [ ] Review done, blocking findings fixed, checks green
> - [ ] The user saw it fixed on their case and agrees

---

## Phase 5: Commit & Merge, on Approval

```bash
git add <the files of this fix, by name>
git commit -m "fix(<scope>): <what was wrong, in English>

<the cause, in one line>"
```

No AI attribution line. If the fix leaves `docs/living-spec/` unchanged, add the line
`Living-Spec: none — <reason>` (see `CLAUDE.md`): the commit is refused without it. With a remote:
push and open a pull request (GitHub: `gh pr create --base main`) with the
cause, the fix, the test, and the living spec change or the `Living-Spec` line. Then
ask "Shall I merge into main?" and, only after an explicit yes, merge (`git checkout main && git
merge --squash fix/<name> && git commit -m "<the fix's commit title>"`), push `main` if there is a
remote, and delete the branch.

> **GATE 5 (STOP)**:
>
> - [ ] Committed, no AI attribution; pull request shared if there is a remote
> - [ ] The user explicitly approved the merge; merged; branch deleted

---

## Gate Summary

| Gate | Condition | If skipped |
| --- | --- | --- |
| 1 | Bug reproduced, expected behaviour agreed | A non-bug gets "fixed", or the wrong one |
| 2 | Failing test first | No proof the fix fixes anything |
| 3 | Cause fixed, checks green | The bug comes back, or another appears |
| 4 | Review and user check | A fix that breaks something else |
| 5 | Merge approved | Unauthorised change to `main` |
