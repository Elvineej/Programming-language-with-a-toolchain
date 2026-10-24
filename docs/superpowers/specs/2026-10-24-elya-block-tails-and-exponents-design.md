# Block tails and float exponents

**Status:** done (2026-10-24, branch `auto/elya`, a scheduled run). HANDOFF
step 1 (two gaps the Float part-2 review saw). The design choices are Claude's
under the maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at a92b546, gate 1275 / 130)

(a) A block whose last statement is a `let`:

| program | `elya run --value` | `elya build` |
|---|---|---|
| `pub fn main() { io.println("x")  let y = 2 }` | prints `x`, value 0 (`Unit`) | refused: "core lowering failed: block without tail expression" |
| `pub fn main() { let y = 2 }` | 0 | refused (the same) |
| a lambda `fn() { let q = 1 }` called, main `7` | 7 | refused (the same) |
| `fn f() -> Int { let y = 2 }` | E0400 type mismatch (twice) | -- |
| an `if` arm, a handler clause body ending in `let` at `Int` | E0400 | -- |

So the checker and the evaluator give such a block the value `Unit`, and the
native back end refuses it: the two sides disagree on accepted programs. An
empty block `{ }` is the same case (no tail). With the rule of D1 switched on,
the root crate's whole suite (both its packages) stays green: no test program
relies on a block without a value.

(b) Float literals: `1e19` is the integer `1` followed by the name `e19`
(E0200 "unresolved name `e19`"); there is no exponent, so `1.0e19` and
`5.0e-324` must be computed. `float.to_string` writes positional digits
(`1000000000000000000000000000000.0`, `0.0000000000003333333333333333`).

## 1. Decisions

- **D1. A block's value is its last expression: E0103 without one.** A block
  (a function body, an `if` arm, a lambda, a clause, a `{ }` expression) that
  ends in a `let` is E0103 "a block cannot end with `let`", labelled at the
  `let`; an empty block is E0103 "a block has no value". The help says to end
  it with the bound name, another expression, or `Unit`. Both sides then agree
  by construction (nothing reaches either).
  - *Rejected:* lowering a missing tail as `Unit` in Core. It would make both
    sides agree, but keeps a block whose value is silently `Unit`, which the
    measured E0400s show is a type error waiting at the use (`fn f() -> Int {
    let y = 2 }` was reported as a mismatch at the function, not at the `let`).
    The stricter rule costs nothing measured.
  - A `let` whose own value failed to parse is not reported again.
- **D2. Exponent literals, with the `.` required.** A float literal is digits,
  `.`, digits, and an optional exponent `e`/`E`, sign, digits (`1.0e19`,
  `5.0e-324`, `2.5E-3`; `_` separators as before). `1e19` is E0003 "an exponent
  needs a float literal" with the help `write 1.0e19` (it is read as that
  float, so checking goes on). A literal that is not finite (`1.0e400`) or is
  non-zero yet rounds to zero (`1.0e-400`) is E0002 "float literal out of
  range"; `0.0e-999` is zero, as written.
  - *Rejected:* `1e19` as a float. Every float literal has a `.` today, so a
    digit string stays an `Int` whatever follows; one rule for the reader.
  - Text unchanged: `float.to_string` and `op.show` keep writing positional
    digits (exact, shortest, never an exponent). They are the trace format
    (`op.encode`, the host trace) and the native printer is proven against
    Rust's on 200 000 values; an exponent form would change both for no
    reader that needs it.

## 2. Evidence (planned)

`tests/block_tails_and_exponents.rs` (root, twice): every block kind ending in
`let` is one E0103; an empty block is E0103; exponent literals' values and
texts; `1e19` E0003, `1.0e400` and `1.0e-400` E0002, `0.0e-999` zero.
`crates/codegen/tests/native_float.rs` gains one test: the exponent literals
natively, equal to the evaluator.

Negative controls: D1 off (no E0103): the E0103 tests fail and a `let`-ending
main checks clean; D2's range check off: `1.0e400` checks clean.

## 3. Outcome and review

Red as predicted (E0200 for `e19`, clean checks for the `let`-ending blocks);
gate 1284 / 132 as predicted. Controls K1 and K2 in the test file's header.

The independent review ran the whole workspace suite on the change and every
`.elya` file in the repository against the base: no valid program changed
meaning (`1.e5`, `1.0e`, `1.0e+`, `}else{` lex as before), the range and zero
checks are right at the edges (`2.4703282292062328e-324` rounds up to the
smallest subnormal, the exact tie below it is E0002), and evaluator and native
agree on ten exponent literals. Findings, fixed test-first (assertions added
to `exponent_literal_mistakes_are_named`): F1, `1e400` was E0003 only, with a
help suggesting `1.0e400` (itself E0002): now both; F2, `1_e5`'s help said
`1_.0e5`. Recorded, not changed: a lexer error on a block's last line (E0001
drops the character) leaves the `let` above it last, so E0103 is added.
