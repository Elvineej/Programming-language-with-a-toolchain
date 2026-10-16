# Record and replay from the command line -- plan

Spec: `docs/superpowers/specs/2026-10-16-elya-cli-record-replay-design.md`. Branch
`auto/elya` at `6e5794f`. Baseline Linux gate: 1015 passed, 95 suites.

- `tests/trace.rs` (new suite: +2 suites): five tests, counted twice: +10. Red: the API
  does not exist (a compile error).
- `crates/cli/tests/run_cli.rs` (new suite, both configurations: +2 suites): one test,
  +2. Red: `elya run --record` is a usage error.

Predicted gate: 1015 + 10 + 2 = **1027 passed, 99 suites**.

Measured 1027 / 99. Review: two trace tests (+4) and one CLI test (+2): **1033 passed, 99 suites**.
