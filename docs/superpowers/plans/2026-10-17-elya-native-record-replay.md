# Record and replay natively -- plan

Spec: `docs/superpowers/specs/2026-10-17-elya-native-record-replay-design.md`. Branch
`auto/elya` at `005b0d2`. Baseline Linux gate: 1079 passed, 105 suites.

1. `crates/codegen/tests/native_trace.rs` (native: counts once, +1 suite), red first
   (every test: the binary ignores `ELYA_RECORD`/`ELYA_REPLAY`, so no trace is
   written, a replay reads the live clock, and no failure is named):
   - `an_evaluator_trace_replays_natively`
   - `a_native_trace_replays_in_both`
   - `replay_failures_have_the_evaluators_words`
   - `a_failing_native_run_still_writes_its_trace`
   - `the_environment_is_checked_before_the_program_runs`
   +5.
2. Runtime: `elya_trace_init` before the program thread, the two time functions,
   `elya_trace_finish` after it (D1-D5).
3. Negative controls.

Predicted gate: 1079 + 5 = **1084 passed, 106 suites**.

Measured: **1084 passed, 106 suites**, as predicted; the review added two tests (1086).
