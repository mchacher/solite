# Plan: Spec 056

## Steps
- [x] 1. R1: the install's step lines (firmware), seen in QEMU.
- [x] 2. R3: the catalogue typed (core), tests.
- [x] 3. R2: review of internal buffers; the threshold at 512; QEMU memory figures before/after.
- [x] 4. Living spec, memory budget, D86; validate.
- [x] 5. On the box: OTA of a release (0.3.1, then 0.3.2), installs and removals of 4 recipes, the lowest figure;
      pairing and reading a device.

## Test plan
| Module | Scenario | Expected |
| --- | --- | --- |
| catalogue | a catalogue of 32 entries | typed, same API answers |
| catalogue | unknown fields | ignored, as before |
| firmware | QEMU boot and script phase | passes; internal figure written |
| box | install, update, remove 4 recipes | lowest internal over 64 KB |

## Validation
- `sh scripts/validate.sh` green, QEMU included.
