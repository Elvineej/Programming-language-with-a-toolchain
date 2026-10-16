# Computed callees and functions as values -- plan

Spec: `docs/superpowers/specs/2026-10-16-elya-callees-and-function-values-design.md`.
Branch `auto/elya` at `80183ed`. Baseline Linux gate: 1010 passed, 95 suites.

- `native_codegen.rs`: `CALLEES` (c1, c4, c3, c5, c6, c7, c8), value and differential:
  +2. Red: six refused by name; c8 (a shadowing local) passes both ways -- it is the
  guard that the pass leaves locals alone.
- Changed expectations, by this decision (lib.rs): `rejects_a_computed_callee_specifically`
  and `rejects_a_function_name_in_value_position` now assert compilation (renamed;
  count unchanged).

Predicted gate: 1010 + 2 = **1012 passed, 95 suites**.

Measured 1012 / 95. Review: three corpus rows (no count change); final **1012 passed, 95 suites**.
