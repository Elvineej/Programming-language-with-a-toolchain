# Plan: aliases that rebind their target

Spec: `docs/superpowers/specs/2026-10-24-elya-alias-renaming-design.md`.
Baseline: 1238 passed, 124 suites (c5c2a44).

## Task 1: red tests
`crates/codegen/tests/native_aliases.rs`: two tests (+2, +1 suite). Red: a1 and a4
refused by name. Prediction: **1241 passed, 125 suites** (+2 native, +1 unit
test pinning the guard).

## Task 2: D1 and D2 (`anf.rs`)
## Task 3: the changed tests (the guard's program runs now; the anf unit test)
## Task 4: gate, controls, review

## Outcome

Red as predicted; green at 1241 / 125 as predicted; the review found no defect.
