# Integer arithmetic is exact or fails by name -- plan

Spec: `docs/superpowers/specs/2026-10-10-elya-checked-integer-arithmetic-design.md`.
Branch `auto/elya` at `2e37b1c`. Baseline Linux gate: 859 passed, 81 suites.

## Task 1 -- evaluator (red first)

`tests/int_arith.rs` (root; counts twice):
- `integer_overflow_is_a_named_error` -- MAX+1, MIN-1, MAX*2, MIN/-1, -(MIN) each E0300
  "integer overflow", CEK and tree-walker. [red: Rust panic]
- `min_remainder_minus_one_is_zero` -- 0 on both evaluators. [red: panic]
- `division_truncates_toward_zero` -- -3, -1, 1, 3, 1. Pin, green before.
- `a_zero_divisor_is_named` -- pin, green before.
+8.

## Task 2 -- native (red first)

`crates/codegen/tests/native_codegen.rs` (once each):
- `integer_division_runs_natively` -- a corpus (truncation, signs, `MIN % -1`, `/` and `%`
  inside an effectful function after a perform, i.e. through the CPS emitter) agrees with
  the evaluator. [red: refused "Div"]
- `integer_traps_are_named_on_both_sides` -- div0, rem0, add/sub/mul overflow, MIN/-1:
  exit 1 and the named stderr natively, E0300 with the same words in the evaluator.
  [red: refused / wraps]
- Unit `rejects_div_specifically` -> `div_lowers` (0 net).
+2.

Predicted final gate: 859 + 8 + 2 = **869 passed, 83 suites**.
