# Slice 5c-2 — name rules: plan

Spec: `docs/superpowers/specs/2026-10-05-elya-slice-5c2-name-rules-design.md`.
Branch `claude/slice-5c2-name-rules` from `main` at `66fb01b`. Baseline Linux gate: 614
passed, 61 suites. Root-crate tests count twice (both gate configurations).

## Task 1 — E0204 and E0205 (resolve)

New file `tests/name_rules.rs` (same `codes` helper shape as `clause_resolution.rs`).
Red tests, today's result in brackets:
- `a_second_clause_for_one_op_is_e0204` (n1) [`[]`] -> `[E0204]`, text "`ask` already has a
  clause in this handler".
- `a_second_clause_spelled_differently_is_still_e0204` (`ask()` + `Ask.ask()`) [`[]`] -> `[E0204]`.
- `a_function_named_like_an_op_is_e0205` (n2) [`[]`] -> `[E0205]`, text "function `ping` has the
  name of an operation of `E`".

## Task 2 — locals before ops (resolve set, inference, evaluator, lowering)

Red tests (values via `run_module_value`):
- `a_let_bound_local_shadows_an_op` (n3) [1] -> 5.
- `a_parameter_shadows_an_op` (n4) [1] -> 5.
- `shadowing_ends_with_its_scope` (n5) [2] -> 6.

Expectation changes the decision requires (each replaced, red first, called out in the commit):
- core `an_op_wins_over_a_same_named_fn_...` -> `a_local_named_like_an_op_lowers_to_a_call`: n3's
  `ping()` lowers to an application, not a `Perform`.
- native `a_function_named_like_an_op_is_not_what_a_perform_calls` ->
  `a_local_named_like_an_op_is_called_natively`: n3, text "CALLED the local", equal to the
  evaluator; plus a new `shadowing_ends_with_its_scope_natively` (n5, 6).

Predicted final gate: 614 + 6 x 2 (new root file, both configs) + 1 (new native test) = 627
passed, 63 suites.

## Task 3 — controls, each reverted

- K1 E0204 off: n1 tests `[]`.
- K2 E0205 off: n2 test `[]`.
- K3 inference op-first again: n3/n4/n5 front end disagrees -- predicted: type error E0400
  (the op's `Int` vs the closure call) or the value tests fail; recorded as observed.
- K4 evaluator op-first again: value tests return the op's answers (1, 1, 2).
- K5 lowering ignores the resolver's set: core test fails (a `Perform`), native n3 prints
  "PERFORMED" and the differential fails.

## Task 4 — close-out

PARKED: both questions resolved. Independent review. Windows CI green.
