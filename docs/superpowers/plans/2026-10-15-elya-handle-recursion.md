# Recursion through a handle body -- plan

Spec: `docs/superpowers/specs/2026-10-15-elya-handle-recursion-design.md`. Branch
`auto/elya` at `9eb8d5f`. Baseline Linux gate: 900 passed, 85 suites.

- `tests/handle_recursion.rs` (new suite, both configurations: +2 suites): three tests,
  red under the negative control (two fail with E0420 / E0423; the rejection controls
  pass both ways). Root crate, counts twice: +6.
- `native_codegen.rs::recursion_through_a_handle_body_runs_natively`: five programs,
  value and differential (red at `9eb8d5f`: the front end rejects them): +1.

Predicted gate: 900 + 6 + 1 = **907 passed, 87 suites**.

Measured: **907 passed, 87 suites**. Review: F1 and F2 fixed red first, one test (+2,
counted twice). Final gate predicted **909 passed, 87 suites**.

Extension, recursion through a lambda: one test (+2), two `row_soundness` expectations
changed (count unchanged), two native corpus rows: predicted 911, measured 911. Its
review: one test (+2): predicted and measured **913 passed, 87 suites**.
