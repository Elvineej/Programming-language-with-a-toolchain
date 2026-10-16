# Argument records -- plan

Spec: `docs/superpowers/specs/2026-10-16-elya-argument-records-design.md`. Branch
`auto/elya` at the callees commit's log (gate 1012 / 95).

- `native_codegen.rs::ARITY` (a1-a7, value and differential; red: every row refused by
  an arity cap): +2.
- `spill.rs` unit test `a_long_parameter_list_keeps_two_and_one_record`: +1.
- Changed expectations, by this decision (lib.rs, count unchanged): the five arity
  refusal tests now assert compilation (renamed), and the six-parameter one also pins
  `MAX_SOURCE_PARAMS + 2 <= MAX_PARAMS` at compile time.

Predicted gate: 1012 + 2 + 1 = **1015 passed, 95 suites**.

Measured 1015 / 95. Review: three corpus rows (no count change): **1015 passed, 95 suites**.
