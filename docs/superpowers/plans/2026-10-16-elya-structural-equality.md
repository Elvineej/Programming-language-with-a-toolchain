# Structural equality -- plan

Spec: `docs/superpowers/specs/2026-10-16-elya-structural-equality-design.md`. Branch
`auto/elya` at `1ce1a9d`. Baseline Linux gate: 1039 passed, 99 suites.

1. `tests/structural_eq.rs` (root crate: counts twice), red first:
   - `comparing_what_is_not_data_is_e0436` (red: every rejected row checks clean): +2.
   - `the_evaluator_compares_long_lists_iteratively` (red: host stack overflow, the test
     binary aborts): +2.
2. Types: marked (equality) variables, the binding and instantiation
   rules, E0436 (D1). Evaluator: the iterative comparison (D2).
3. `native_codegen.rs::EQUALITY` value and differential (red: refused by name): +2.
4. Runtime `elya_eq`, the emitter calls in `lib.rs` and `cps_emit.rs` (D3).
5. Negative controls K1-K3.

Predicted gate: 1039 + 4 + 2 = **1045 passed, 100 suites** (one new test file).

First full gate: red (two pre-existing tests, the review's H1). After the review's fixes
(two more root tests, both red first): predicted **1049 passed, 100 suites**.

Measured: **1049 passed, 101 suites** (the suite count was mispredicted: a root test file
counts once per configuration).
