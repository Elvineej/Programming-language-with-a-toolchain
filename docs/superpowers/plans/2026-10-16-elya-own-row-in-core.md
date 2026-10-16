# A function's own row in Core -- plan

Spec: `docs/superpowers/specs/2026-10-16-elya-own-row-in-core-design.md`. Branch
`auto/elya` at `7cbef8a`. Baseline Linux gate: 998 passed, 93 suites.

- `tests/core_lowering.rs::a_function_carries_its_own_row` (red: no field): +2.
- `native_codegen.rs::OWN_ROW` (r10, r24, r27, g1; value and differential in one test;
  red: refused today): +1.
- Changed expectations, by this decision: the two `mono.rs` row-only refusal tests now
  assert the clone (count unchanged); `src/types.rs`'s node-count tripwire 5 -> 6 and its
  all-`Int` check skips the declaration entry (count unchanged).

Predicted gate: 998 + 2 + 1 = **1001 passed, 93 suites**.

Measured: **1001 passed, 93 suites**. Also changed (additive, recorded in the spec): the seven
`typed_inference` snapshots gain one line per declaration.
