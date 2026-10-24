# Plan: block tails and float exponents

Spec: `docs/superpowers/specs/2026-10-24-elya-block-tails-and-exponents-design.md`.
Baseline: 1275 passed, 130 suites (a92b546).

## Task 1: red tests

`tests/block_tails_and_exponents.rs`, four tests (twice: +8, +2 suites):
1. `a_block_ending_in_let_is_e0103` -- red: E0400s and clean checks.
2. `an_empty_block_is_e0103` -- red: clean checks.
3. `exponent_literals_read_as_their_floats` -- red: E0200 (`e19` names).
4. `exponent_literal_mistakes_are_named` -- red: E0200, no E0003/E0002.
`native_float.rs`: `exponent_literals_run_natively` (+1) -- red: E0200.

Prediction: **1284 passed, 132 suites**.

## Task 2: D1 (parser), D2 (lexer)
## Task 3: gate, controls, review

## Outcome

Red as predicted; green at 1284 / 132 as predicted. The review's F1-F2 added
assertions to an existing new test (no count change).
