# Plan: row variables in signatures (part 1)

Spec `docs/superpowers/specs/2026-10-30-elya-row-variables-design.md` D1, D2. Gate
before: 1337 passed, 143 suites (8432b96).

1. Red tests first, `tests/row_variables.rs` (root crate), four tests:
   - `/ {e}` shared by a parameter and the function's row, run with a `Log` task:
     predicted red, E0100 "expected effect name";
   - `/ {Log | e}` adding a label: predicted red, E0001 (no `|` token);
   - one name is one row across positions: `first(f: fn() / {e} -> Int, g: fn()
     / {e} -> Int) -> fn() / {e} -> Int { f }` makes the caller handle what `g`
     performs (E0420 when it does not); unnamed, the program runs. Predicted red:
     E0100. (Two drafts were wrong and corrected before the code, each checked with
     the CLI: a call of a parameter relays its row, and E0421 counts a function's
     own labels only, so `/ {}` there was never "pure"; naming changes neither.)
   - the syntax's errors: two variables, a variable before a label, an uppercase
     name after `|`, a variable in a grant list: each a named parse error.
2. The lexer's `|` token; `parse::effect_row` reads a tail; `ast::row_tail`.
3. The checker: a per-signature map from a row variable's name to a row variable,
   used by `elaborate_ann` and the declared row (`check` of a function's
   `effect_row`). Declarations refuse a row variable for now ("part 2").
4. Controls, each reverted: K1, the names not shared (a fresh variable per
   mention): `first`'s program runs (no E0420); K2, the tail dropped (read
   as no row): `/ {Log | e}` refused (the body's `f()` performs more than `Log`).

A fifth test, added while writing the code: a declaration's row variable is
refused by name (it read as "unknown effect `e`").

Predicted gate: 1337 + 5 x 2 (root tests run twice) = **1347 passed, 145 suites**.

The review's fixes add four tests (nine in all): predicted gate 1337 + 9 x 2 =
**1355 passed, 145 suites**.
