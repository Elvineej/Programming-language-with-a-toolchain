# Plan: `import elya/replay`

Spec: `docs/superpowers/specs/2026-10-24-elya-std-replay-design.md`.
Baseline: 1229 passed, 121 suites (730c301).

## Task 1: red tests
`tests/std_replay.rs` (4 tests, root, counted twice: +8, +2 suites) and
`native_std_replay.rs` (1 test, +1 suite). Red: E0600 "no module `elya/replay`".
Prediction: **1238 passed, 124 suites**.

## Task 2: the module (`std/replay.elya`) and its loading (`modules.rs`)
## Task 3: gate, control, review

## Outcome

Red as predicted; green at 1238 / 124 as predicted; the review found no defect
(limitations recorded in the spec's section 3).
