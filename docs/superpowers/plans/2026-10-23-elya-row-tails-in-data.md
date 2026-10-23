# Plan: upcasts through data, natively

Spec: `docs/superpowers/specs/2026-10-23-elya-row-tails-in-data-design.md`.
Baseline: 1193 passed, 116 suites (fb56487).

## Task 1: red tests

`crates/codegen/tests/native_upcast_data.rs` (the `native_arith.rs` harness;
counted once):
1. `wrapped_results_inside_data_agree`: r1 and top_val. Predicted red: r1 prints a
   pointer where the evaluator prints 15.
2. `call_results_upcast_through_data_agree`: let_lam, list, nest. Red: "match
   failed (no arm matched)".
3. `a_long_list_is_upcast_once`: a million closures in a list built by a `let`
   lambda, summed through an effectful function. Red: "match failed".
4. `bound_values_upcast_through_data_agree`: the call's result bound by `let`
   first, then passed. Red: measured when written.

Prediction: **1197 passed, 117 suites** (+4; one new native suite).

## Task 2: D1 (`anf.rs`)

`inner_type`: the row variables of the own result that the parameters and the
own row do not mention take the use's rows (`bind_tails`).

## Task 3: D2 and D3 (`specialize.rs`)

- `Adapter::coerce`: `(Con(T, a), Con(T, b))` calls a generated `$coerce.T.k`
  (cached by the pair), built by matching each constructor and coercing its
  fields; the functions are appended to the module.
- `coercible`: data of the same declared type is coercible.
- `conv_eq`: data types agree only with the same name.

## Task 4: gate, controls, review, sweep

`sh scripts/check.sh` at 1197 / 117; the three controls; a review; the sweep.
