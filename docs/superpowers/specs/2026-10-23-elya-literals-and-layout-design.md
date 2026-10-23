# Literals at the edges, and a layout rule

**Status:** in progress (2026-10-23, branch `auto/elya`, a scheduled run). HANDOFF
step 1. The design choices are Claude's under the maintainer's delegation (rule 2),
recorded here.

## 0. Measured first (at 3702b6f, gate 1115 / 107)

Each probe through `elya run --value` and `elya build` + the binary:

| program | evaluator | native |
|---|---|---|
| m1 `-9223372036854775808 / 1000000000000` | E0001 "unexpected character / not a valid Elya token" on the digits | (front end) |
| m2 `9223372036854775808` | E0001, the same words | (front end) |
| m3 `99999999999999999999` | E0001, the same words | (front end) |
| m4 `-(9223372036854775807) - 1` | -9223372036854775808 | the same |
| m5 `match x { -1 -> 10  _ -> 20 }` | E0100 "expected a pattern" | (front end) |
| m6 the same arms, one per line | E0100 "expected a pattern" (`5 - 1` read as the first arm's body) | (front end) |
| l1 `let y = 4` then a line `-x * y` | E0200 "unresolved name `y`" (read as `let y = 4 - x * y`) | (front end) |
| l2 `let h = g` then a line `(h)(x)` | E0200 "unresolved name `h`" (read as `let h = g(h)(x)`) | (front end) |
| l3 `let y = x` then a line `+ 1` | 4 (continued) | 4 |
| l4 `let y = x +` then a line `1` | 4 | 4 |
| l5 `g(x,` then a line `1)` | 2 | 2 |
| l6 `(x` then a line `- 1)` | 2 | 2 |

So `MIN` is writable only as arithmetic, every out-of-range literal is reported as a
bad *character*, a negative number cannot be a pattern, and a line starting with `-`
or `(` silently joins the line before.

The corpus (`scripts/sweep.py`'s collection, 810 programs): **no** line starts with
an infix operator or `(` after a line that ended an expression, at any nesting (a
one-off token scan, kept in the plan). Literals of 19 or more digits: only
`9223372036854775807` (MAX) and `0 - 9223372036854775807 - 1` (MIN, five files).
No negative literal pattern anywhere (none parses). So neither rule below changes
any existing program.

Elya's own data-literal syntax (`op.show`, `op.encode`, `op.decode`, spec
2026-10-16 D7) already writes a negative number as one literal (`-7`,
`-9223372036854775808`), and `op.decode` reads it with the sign.

## 1. Decisions

- **D1. A `-` directly before an integer literal is part of the literal.** The
  parser folds `-` followed by an integer literal token into one negative literal
  (`Expr::Int(-n)`), whatever the whitespace between them, as the data-literal
  syntax already does. So `-9223372036854775808` is `MIN`, and the source
  language and `op.show` write numbers the same way.
  - Float literals fold the same way (`-1.5` is `Expr::Float(-1.5)`); `-x` on a
    float already meant IEEE negation, so the value is the same, `-0.0`
    included.
  - Folding does not change any value: `-n` was `0 - n` in Core (spec 2026-10-17
    D1), equal to the literal for every `n` in range. `-(-n)` and `-(n)` stay
    unary (a parenthesised literal is not directly after the `-`).
  - *Rejected:* keep `-` unary and accept the single token `9223372036854775808`
    only under it (Rust's lint). It needs a literal that is out of range
    everywhere except one parent, and gives `-5` and `MIN` two different shapes.
- **D2. An integer literal out of range is its own error, `E0002`.** The lexer
  reads digits into an unsigned magnitude and never fails on range; the parser
  checks it: at most `9223372036854775807`, or `9223372036854775808` when folded
  with a `-`. Anything else is `E0002` "integer literal out of range", labelled on
  the literal, with the help "`Int` is 64-bit: from -9223372036854775808 to
  9223372036854775807". `x - 9223372036854775808` and `-(9223372036854775808)` are
  `E0002` (the literal is not folded there). E0001 stays for characters that are
  no token.
  - *Rejected:* wrapping or saturating an oversized literal. Arithmetic is checked
    (spec 2026-10-10); a literal that silently changes value would be the one
    unchecked number.
- **D3. Negative literals are patterns.** `-1 -> ...` and `-9223372036854775808 ->
  ...` match, by D1's fold and D2's range check, in the pattern parser too. The
  exhaustiveness checker already treats `Int` patterns as an open set.
  - Consequence (accepted, recorded): arms on ONE line, `0 -> 5 -1 -> 7`, read
    `5 - 1` as the first body, as `5 -1` reads anywhere else; it is a parse error
    at the second `->`, never a silent change. One arm per line works (D4).
- **D4. Layout: in braces, a newline before an infix operator or a call's `(`
  ends the expression.** Inside `{ ... }` (a block, a function or lambda body,
  `match` arms, a handler's clauses), when the next token is an infix operator or
  the `(` of a call and a newline separates it from the token before, the
  expression stops there. Then:
  - a line starting with `-` directly followed by its operand (no whitespace:
    `-x * y`, `-1`) or with `(` begins a new statement (l1 is `-12`, l2 is `4`);
  - any other infix operator starting a line, and a `-` followed by whitespace
    (`- 1`, the binary style), is `E0101` "an operator at the start of a line does
    not continue the line before", labelled on the operator, with the help "end the
    line before with `<op>`, or put the whole expression in parentheses".
  - Inside `( ... )` (a parenthesised expression, call arguments, `resume(...)`)
    newlines are whitespace again, so `(x` newline `- 1)` still continues (l6); a
    block inside parentheses is braces again.
  - *Why the whitespace after `-` matters:* without it, `a` newline `- b` (a
    continuation written in the leading-operator style) would silently become two
    statements, `a` and `-b`. With it, the binary style is an error that says how
    to continue, and the prefix style (`-x`, as in l1) is a statement.
  - *Rejected:* (a) continue across newlines everywhere (today: l1 and l2 are
    silently one expression); (b) end the expression before `-` and `(` only, and
    let `+`, `*`, ... continue: `total` newline `+ x` newline `- y` would be
    `total + x` and then `-y`; (c) make every line starting with `-` or `(` after
    an expression an error: it outlaws the plainest negative result, l1's `-x * y`
    after a `let`; (d) Go's rule (end at every newline after a token that can end
    an expression) applied inside parentheses too: it breaks l6, the only way left
    to write a leading-operator continuation.
  - Changed: l3 (a `+` starting a line, which continued) is now E0101. No corpus
    program is written that way (§0).
  - Not changed: a `.` (qualified names) is not an infix operator here; a trailing
    operator (`x +` newline `1`, l4) and a trailing comma (l5) continue as before.

## 2. Evidence (planned)

- `tests/literals_layout.rs` (front end, both configurations): MIN as a literal
  (value, `/`, `-(MIN)` overflowing by name), E0002 on m2, m3,
  `x - 9223372036854775808`, `-(9223372036854775808)` and an out-of-range pattern;
  negative patterns (m5, m6, MIN as a pattern); l1, l2, l4, l5, l6 values;
  E0101 for l3, a spaced `- 1`, `*`, `==` and `&&` at a line start; a block inside
  call arguments laid out again; the fold's AST (`- 5` is `Int(-5)`, `-(5)` is
  unary) and `op.show` of MIN pasted back as source.
- `crates/codegen/tests/native_literals.rs` (execution, differential): MIN
  programs, `-(MIN)`'s overflow, negative patterns, l1 and l2, each compiled, run
  and compared with the evaluator.
- Negative controls (recorded in the test files' headers): the fold off in
  expressions (D1), the range check one too wide (D2), the layout check off (D4),
  the whitespace test after `-` off (D4), each failing a distinct test.
