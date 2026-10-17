# A repeatable differential sweep -- plan

Spec: `docs/superpowers/specs/2026-10-16-elya-differential-sweep-design.md`. Branch
`auto/elya` at `45e577b`. Baseline Linux gate: 1049 passed, 101 suites.

1. `crates/cli/tests/run_cli.rs` (runs in both configurations: counts twice), red first:
   - `elya_run_keeps_output_written_before_a_runtime_error` (red: stdout empty): +2.
   - `elya_run_value_prints_main_as_a_native_binary_does` (red: a usage error): +2.
2. `elya::run_source_outcome`, `elya run` printing output before an error, `--value`.
3. `scripts/sweep.py`; run it; record the table and the ranked gaps in the spec.
4. Whatever the sweep finds that is a bug gets its own red test. Found: `elya build`
   panicked on a warning-only program -> `build_cli.rs::a_program_with_only_warnings_builds`
   (codegen configuration only: +1).

Predicted gate: 1049 + 4 + 1 = **1054 passed, 101 suites**.
