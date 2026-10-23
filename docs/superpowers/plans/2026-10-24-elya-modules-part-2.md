# Plan: modules, part 2

Spec: `docs/superpowers/specs/2026-10-24-elya-modules-part-2-design.md`.
Baseline: 1209 passed, 119 suites (37846fa).

## Task 1: red tests

Four tests in `tests/modules.rs` (spec §2), red: E0606 "not checked yet" where a
list is written, and no error where none is.
Prediction: **1217 passed, 119 suites** (+8, root crate twice). The part-1 test
of E0606 ("not checked yet") changes by D2: its program's list now is checked;
recorded as a changed expectation.

## Task 2: interfaces (`types.rs`)

`infer_with_interfaces`: the diagnostics, affine sites, and each top-level
function's generalized type.

## Task 3: the check (`modules.rs`, `lib.rs`)

`Program::grants` (import span, list, target, importing file); `merge` checks
E0607/E0608; `check_grants` after inference walks each target's `pub` types by
polarity (D1) and reports E0606.

## Task 4: gate, controls, review
