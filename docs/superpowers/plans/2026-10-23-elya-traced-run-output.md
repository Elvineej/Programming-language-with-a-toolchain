# Plan: a failing traced run keeps its output on stdout

Spec: `docs/superpowers/specs/2026-10-23-elya-traced-run-output-design.md`.
Baseline: 1139 passed, 110 suites (0cf65fd).

## Task 1: red tests

`crates/cli/tests/run_cli.rs` (counted twice):
1. `a_failing_recorded_run_keeps_its_output_on_stdout`: stdout `x\n`, stderr has
   `division by zero` and does not start with `x`. Predicted red: stdout empty.
2. `a_failing_replayed_run_keeps_its_output_on_stdout`: the replay of test 1's
   trace prints `x\n`; a diverging replay prints `y\n` with "replay diverged" on
   stderr. Red: stdout empty.

`crates/cli/tests/build_cli.rs` (codegen, counted once):
3. `a_traced_run_prints_what_the_native_binary_prints`: for the failing record,
   its replay, the diverging replay and a successful replay, the CLI's stdout and
   exit code equal the native binary's. Red: stdout empty vs `x`.

Prediction: **1144 passed, 110 suites** (+2 x 2, +1).

## Task 2: the fix

`src/lib.rs`: `record_source`, `replay_source` returning `RunOutcome` (a shared
`outcome` helper with `run_source_outcome`); the old functions as wrappers.
`crates/cli/src/main.rs`: `trace_cmd` prints `output` then the error.

## Task 3: gate, control, review

`sh scripts/check.sh` at 1144 / 110; the control of spec §2; a review of the diff.
