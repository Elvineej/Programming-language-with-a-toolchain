# `Float`, part 2: comparisons, negation and conversions

**Status:** done (2026-10-24, branch `auto/elya`, a scheduled run). HANDOFF
step 1. The design choices are Claude's under the maintainer's delegation (rule
2), recorded here.

## 0. Measured first (at 4928cef, gate 1221 / 119)

`elya check` on four small numeric programs:

| program | needs | today |
|---|---|---|
| newton: `sqrt(2)` by Newton's method, stopping when `abs(n -. g) < 1e-9` | a float comparison | E0400 (`<` is `Int` only) x3 |
| mean: the mean of a list of `Int`s | `Int` to `Float` | E0201 unknown builtin `float.of_int` |
| neg: `-x` on a `Float` variable | negation | E0400 (`-x` is `0 - x`, `Int`) |
| round: `float.round(2.5) + float.truncate(-2.7)` | `Float` to `Int` | E0201 x2 |

`+. -. *. /.` and `==`/`!=` on floats run on both sides since 6baf595; a `-`
glued to a literal is part of it (`-1.5`).

## 1. Decisions

- **D1. Float comparisons are their own operators: `<.`, `<=.`, `>.`, `>=.`**,
  like `+.`, with `<`'s precedence. They are IEEE ordered comparisons: any
  comparison with a NaN is `False` (so `!(x <. y)` is not `x >=. y`).
  - *Rejected:* overloading `<` by type. Inference would need a class or a
    defaulting rule for `a < b` on unknown types; the arithmetic operators
    already chose separate spellings.
- **D2. `-.e` negates a float.** A prefix `-.` (the binary `-.` token): IEEE
  negation, the sign flipped (`-.0.0` is `-0.0`; NaN stays NaN). Core lowers it
  as `-0.0 -. e`, which is exactly the negation for every value. As with `-`, a
  line starting with a glued `-.` starts a statement (layout D4 of 2026-10-23).
  - *Rejected:* `0.0 -. e`: it gives `0.0` for `0.0`.
- **D3. `float.of_int(n)` rounds to the nearest `Float`, ties to even** (the IEEE
  conversion; exact up to 2^53). Conversion to `Float` cannot fail, like `+.`.
- **D4. `float.truncate(x)` and `float.round(x)` give an `Int`** (toward zero;
  to nearest with halves away from zero, C's `round`). They fail by name where
  no `Int` is: "float.round: NaN has no Int value", and "float.round: <x> is
  outside Int's range" (`<x>` the float's literal text, as `float.to_string`
  writes it), as checked `Int` arithmetic fails by name.
  - *Rejected:* saturating (Rust's `as`). A silent clamp is the wrong-value
    class checked arithmetic was made to remove.
- All of it on both sides (evaluator and native), differential tests on the
  edges (NaN, infinities, -0.0, 2^53 + 1, 2^63, halves).

## 2. Evidence (planned)

- `tests/float_ops.rs` (root, counted twice): comparisons (with NaN and -0.0),
  `-.` (on 0.0, a variable, at a line's start), conversions and their failures;
  the four measured programs run.
- `crates/codegen/tests/native_float.rs` gains two differential tests.
- Negative controls: NaN comparisons as unordered (`fcmp ult`) natively: the
  NaN row disagrees; D2 as `0.0 -. e`: `-.0.0` prints `0.0`; the range check off
  natively: `float.truncate(1e19)` answers.

## 3. Outcome and review

Red as predicted (E0201/E0400; natively a compile error first). Gate 1229 / 121
as predicted. Controls K4-K6 (in `native_float.rs`'s header) each failed
distinctly. `float_to_int` takes no libm: `trunc`/`round` would need `-lm` on
Linux, so the runtime truncates by conversion and rounds by the exact fraction
(every double at or past 2^52 is an integer).

The independent review found no defect: halves, ±2^63 neighbours, subnormals,
NaN of either sign, precedence and lexing of `-.`, the layout rule, comparisons
and conversions inside CPS functions, closures and a `with any` recorder, record
and replay; all ten examples unchanged. Two older observations, recorded in
HANDOFF: exponent literals (`1e19`) do not lex; a `main` whose body ends in a
`let` runs in the evaluator but is refused natively ("block without tail
expression").
