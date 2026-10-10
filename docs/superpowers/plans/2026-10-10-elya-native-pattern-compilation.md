# Native pattern compilation -- plan

Spec: `docs/superpowers/specs/2026-10-10-elya-native-pattern-compilation-design.md`.
Branch `auto/elya` at `6b6307d`. Baseline Linux gate: 869 passed, 83 suites.

## Task 1 -- red tests

`crates/codegen/tests/native_codegen.rs` (once each):
- `nested_and_literal_patterns_run_natively` -- p1..p7, each native value equal to the
  evaluator's and to a pinned value. [red: p1 "nested constructor pattern"]
- `a_failed_nested_arm_does_not_shadow_an_outer_name` -- p4 alone: 509 natively and in
  the evaluator. [red: refused]
(p6, the 100 001-element tail loop through a compiled match, is a corpus row: a stack that
grew would crash it natively.)
+2.

## Task 2 -- the pass (D1-D4)

Predicted final gate: 869 + 2 = **871 passed, 83 suites**. Controls K1-K3 as the spec says.

## Outcome

First version gated at 871 (as predicted); the independent review then found exponential
code size. Fixed test-first (`a_compiled_match_grows_linearly_with_its_arms` red: killed
out of memory; `no_node_is_shared_in_the_output`), corpus rows p8/p9 added. Final gate
predicted 871 + 2 unit tests = **873 passed, 83 suites**.
