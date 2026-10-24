# Plan: a wrapped function's row shared with a parameter

Spec: `docs/superpowers/specs/2026-10-23-elya-wrapper-parameter-rows-design.md`.
Baseline: 1198 passed, 117 suites (99a84e2).

## Task 1: red test

`wrapped_results_sharing_a_parameter_row_agree` in `native_upcast_data.rs`
(native, counted once): the eight programs of §0. Predicted red: f2 prints a
pointer where the evaluator prints 16.

Prediction: **1199 passed, 117 suites** (+1).

## Task 2: D1 (`anf.rs`)

`inner_type`: bind the parameters' row variables from the use's parameters
(every position), then the own result's remaining free ones from the use's
result; parameters' unbound variables stay fixed.

## Task 3: gate, control, review

`sh scripts/check.sh` at 1199 / 117; the control; a review.

## Outcome

Red as predicted (f2, a pointer); green at 1199 / 117 as predicted; K4
distinct. The review's older finding became D2, two more programs in the same
test (count unchanged).
