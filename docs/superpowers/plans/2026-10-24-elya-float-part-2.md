# Plan: `Float`, part 2

Spec: `docs/superpowers/specs/2026-10-24-elya-float-part-2-design.md`.
Baseline: 1221 passed, 119 suites (4928cef).

## Task 1: red tests
`tests/float_ops.rs` (3 tests, root, counted twice: new suite +2) and two in
`native_float.rs`. Red: the parse or E0201/E0400 errors of §0.
Prediction: **1229 passed, 121 suites** (+8).

## Task 2: front end (lexer, parser, types, resolve, core, evaluator)
## Task 3: native (both emitters; `elya_float_to_int` in runtime.c)
## Task 4: gate, controls, review
