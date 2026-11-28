# Plan 057

- [ ] 1. Mac: a test file measuring R1's parts and R2's layouts (QuickJS's `memory_usage()`), and R3
      (the 7 recipes in a shared context); targeted runs only.
- [ ] 2. QEMU: the same parts and layouts in the script phase of the QEMU image (feature-gated),
      all probes in one QEMU run.
- [ ] 3. The figures in the result and `docs/memory-budget.md`; the proposal (R4).
- [ ] 4. `validate.sh --pre-merge`; review; the proposal to the user.

## Test plan
| Module | Scenario | Expected |
| --- | --- | --- |
| core (test) | R1 parts, R2 layouts, R3 recipes | figures printed, recipes loaded or reasons |
| QEMU | R1 parts, R2 layouts | figure lines in the log, the run's checks unchanged |
