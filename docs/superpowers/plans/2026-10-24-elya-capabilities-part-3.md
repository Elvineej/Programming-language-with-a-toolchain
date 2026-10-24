# Plan: capabilities, part 3 (attenuation in the standard library)

Spec: `docs/superpowers/specs/2026-10-24-elya-capabilities-part-3-design.md`.
Baseline: 1264 passed, 127 suites (dc22437).

## Task 1: red tests

`tests/std_capabilities.rs` (5 tests, twice: +10, +2 suites), red: E0600 "no
module `elya/console`" (tests 1-5; test 3 and 4 red as E0600 for `elya/host`).
`crates/codegen/tests/native_std_capabilities.rs` (1 test, +1 suite), red:
the front end rejects (E0600).

Prediction: **1275 passed, 130 suites**.

## Task 2: `std/console.elya`, `std/clock.elya`, `std/host.elya`; `STD_SOURCES`
## Task 3: gate, controls, review
