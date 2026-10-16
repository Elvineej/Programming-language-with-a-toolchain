# Failure and cancellation -- plan

Spec: `docs/superpowers/specs/2026-10-16-elya-failure-and-cancellation-design.md`. Branch
`auto/elya` at `fcd9cd4`. Baseline Linux gate: 1001 passed, 93 suites.

- `tests/failure.rs` (new suite, +2 suites): two tests, counted twice: +4.
- `tests/examples.rs::failure_and_cancellation`: +2.
- `native_codegen.rs::the_failure_example_runs_natively`: +1.

Predicted gate: 1001 + 4 + 2 + 1 = **1008 passed, 95 suites**.

Measured 1008 / 95. Review: E0209 test (+2): **1010 passed, 95 suites**.
