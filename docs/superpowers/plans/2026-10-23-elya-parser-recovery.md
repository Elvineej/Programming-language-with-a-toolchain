# Plan: parser error recovery inside a body

Spec: `docs/superpowers/specs/2026-10-23-elya-parser-recovery-design.md`.
Baseline: 1199 passed, 117 suites (1a9003c).

## Task 1: red tests

`tests/parse_recovery.rs` (root crate, counted twice):
1. `one_mistake_in_a_body_is_one_error` (p1-p4, p7): red, the E0200 cascade.
2. `a_truncated_declaration_is_an_error` (p5, p6): red, p6 checks clean.
3. `a_lambda_body_recovers_on_its_own` (a mistake inside a lambda; the rest of
   the enclosing function and a later function checked): red, E0200 cascade.
4. `a_lambda_fn_does_not_restart_the_module` (D4): a mistake before a lambda's
   `fn` in a body; red until D2/D4 (today the parse resumes at the lambda).

Prediction: **1207 passed, 119 suites** (+8: four tests counted twice; the new
suite is counted in both configurations, +2).

## Task 2: D1, D3, D4 (`parse.rs`)
## Task 3: D2 (`ast.rs` `Expr::Error`, `parse.rs`, and every match on `Expr`)
## Task 4: gate, controls, review
