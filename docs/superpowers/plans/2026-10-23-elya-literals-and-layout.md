# Plan: literals at the edges, and a layout rule

Spec: `docs/superpowers/specs/2026-10-23-elya-literals-and-layout-design.md`.
Baseline: 1115 passed, 107 suites (3702b6f).

## Task 0: the corpus scan (spec §0, done before the spec)

A one-off script over `scripts/sweep.py`'s `collect()`: a token regex, a bracket
stack, and a report of every token that starts a line, is an infix operator or `(`,
and follows a token that can end an expression (a literal, a name that is not a
keyword, `)`, `}`, `]`). Result on 810 programs: none, at any nesting.

## Task 1: red tests

`tests/literals_layout.rs` (front end; counted twice), seven tests:

1. `min_is_a_literal`: `-9223372036854775808` is MIN; `/ 1000000000000` is
   -9223372; `-(-9223372036854775808)` fails with `integer overflow`.
   Predicted red: the parse asserts, E0001.
2. `an_out_of_range_literal_is_e0002`: `9223372036854775808`,
   `99999999999999999999`, `x - 9223372036854775808`, `-(9223372036854775808)`, and
   `9223372036854775808 -> ...` as a pattern each give E0002 and no E0001.
   Red: E0001 on the first.
3. `negative_literals_are_patterns`: m5 (10), m6 (7), MIN as a pattern.
   Red: E0100 "expected a pattern".
4. `a_line_starting_with_a_prefix_minus_or_a_paren_is_a_new_statement`: l1 is -12,
   l2 is 4. Red: E0200 unresolved `y`.
5. `an_operator_starting_a_line_is_e0101`: l3, `x` newline `- 1`, `* 2`, `== 1`,
   `&& b`. Red: l3 checks clean.
6. `continuations_that_stay`: l4 (4), l5 (2), l6 (2), and a block inside call
   arguments laid out again (`id({ let a = 1` newline `-a })` is -1). Red: only the
   last row, E0200 unresolved `a`.
7. `the_fold_is_on_the_literal`: `- 5` parses to `Int(-5)`, `-(5)` to a unary
   node, `-1.5` to `Float(-1.5)`; `op.show` of a MIN argument, pasted back into a
   program, is MIN. Red: `- 5` is unary.

`crates/codegen/tests/native_literals.rs` (execution, the `native_arith.rs` harness;
counted once), two tests:

8. `literals_agree`: MIN, MIN / 10^12, `-(-MIN)`'s overflow, m5, m6, a MIN pattern.
   Red: the front end rejects the first program.
9. `layout_agrees`: l1, l2, the block in call arguments. Red: the same.

Prediction: **1131 passed, 110 suites** (+7 x 2, +2; one new root test binary in
both configurations, one native).

## Task 2: the literal (D1-D3)

- `lex.rs`: `Int(u128)`, the digits saturated (`parse::<u128>()` or `u128::MAX`);
  never a lexer error on range.
- `parse.rs`: a helper `int_literal(neg, magnitude, span)` that range-checks (E0002)
  and folds; `atom`'s `Minus` arm looks at the next token: an `Int` or `Float`
  literal is folded, anything else stays unary. The pattern parser accepts `-`
  followed by an `Int`.

## Task 3: the layout (D4)

- `Parser` keeps the source text and a `braces: bool` (layout on), set in `block`,
  `match_expr`'s arms and `handle_expr`'s clauses, cleared in parenthesised
  expressions, call arguments and `resume(...)`, each restored after.
- In `expr`'s loop, before a postfix `(` or an infix operator: if layout is on and
  the text between the previous token and this one holds a newline, stop; for an
  infix operator other than a `-` glued to its operand, report E0101 first.

## Task 4: gate, controls, review, sweep

- `sh scripts/check.sh`: 1131 / 110.
- Controls: K1 the fold off in expressions (test 1 red), K2 the range bound one
  too wide (test 2 red), K3 the layout stop off (test 4 red), K4 the glued-`-` test
  off, every `-` an E0101 (test 4 red with E0101, distinct from K3's E0200).
- An independent review of the diff; `scripts/sweep.py` (no disagreement, the same
  counts as at baseline).
