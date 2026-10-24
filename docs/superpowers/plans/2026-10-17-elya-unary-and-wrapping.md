# Unary operators natively, wrapping and bit arithmetic -- plan

Spec: `docs/superpowers/specs/2026-10-17-elya-unary-and-wrapping-design.md`. Branch
`auto/elya` at `b701e89`. Baseline Linux gate: 1069 passed, 102 suites.

1. `tests/int_wrapping.rs` (root: counts twice, +2 suites), red first:
   - `the_builtins_are_typed` (red: E0201 unknown builtin): +2.
   - `the_evaluator_wraps_and_shifts` (red: E0201): +2.
   - `unary_operators_lower_to_core` (red: `Unsupported("Unary")`): +2.
2. Resolve, types, both evaluators (D2); Core lowering (D1).
3. `crates/codegen/tests/native_arith.rs` (native: counts once, +1 suite), red first:
   - `unary_operators_agree` (red: core lowering fails): +1.
   - `wrapping_and_bits_agree` (red: E0201 at check): +1.
4. Both emitters and the runtime's shift trap (D3).
5. Negative controls.

Predicted gate: 1069 + 6 + 2 = **1077 passed, 105 suites**.
