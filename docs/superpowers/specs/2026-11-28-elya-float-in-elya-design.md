# Float through the compiler in Elya

2026-11-28. Status: decided, not built. Step (4) of spec
`2026-11-28-elya-retiring-rust-design.md` T1. Design choices are Claude's
(rule 2).

## 0. Measured first

The checker in Elya types `Float` already (`TB(1)`, the `+.` family,
`float.*`'s signatures). What stops a float program in Elya:

- the parser in Elya keeps `EFloat` without its value (the lexer cannot say
  which double a literal is: Elya has no way to read one from text); its
  range errors (E0002) are decided exactly on the digits, without one;
- Core in Elya refuses `EFloat` by name ("Float in Elya"), where Rust's Core
  holds the literal's bits (`CoreLit::Float(u64)`) and prints them with
  `literal::float_text`, which is `float.to_string`'s text;
- the back end in Elya has no float operation, conversion or printing.

The runtime has the pieces natively: `float_parse.c` (correctly rounded,
checked bit for bit against Rust's `f64::from_str` on 3 million texts) and
`float_fmt.c` (shortest round trip, Ryu's method).

## 1. Decisions

**D1. Three builtins, pure:** `float.parse(text: String) -> Float` (a
literal's text as the lexer accepts it, `_` separators allowed; any other
text fails by name, `elya: float.parse: `<text>` is not a number`, status
1), `float.to_bits(x: Float) -> Int` and `float.of_bits(n: Int) -> Float`
(the IEEE-754 bits as a signed word, both ways, exactly). The evaluator uses
Rust's `f64::from_str`, `to_bits` and `from_bits`; natively `float_parse.c`
and bit casts. They are general (binary I/O, the maintainer's numerics list,
needs the bits), not only the compiler's.

**D2. The parser in Elya** keeps a literal's bits: `EFloat(Int)`, from
`float.to_bits(float.parse(text))`. Its range checks stay as they are (exact,
and E0002's words unchanged); a literal they accept parses to a finite double.

**D3. Core in Elya** lowers `EFloat(b)` to `LFloat(b)` and prints it as Rust
does, `float.to_string(float.of_bits(b))`; `-0.0`, which Rust's Core writes as
its own literal, likewise.

**D4. The back end in Elya** carries a `Float` as an `i64` word of its bits (as
the Rust back end does): `+. -. *. /.` and the comparisons through `bitcast`
to `double` and back (`fcmp oeq`/`une`/`olt`/... as Rust's `float_prim`);
`float.of_int` by `sitofp`; `float.truncate`/`round` through
`elya_float_to_int`; `float.to_string` through `elya_float_to_string`;
`float.parse` through a new `elya_float_parse`; `to_bits`/`of_bits` are the
word itself. A literal is its bits as an `i64` constant.

**D5. Evidence:** every float program of the corpus through the whole Elya
pipeline (the parser's AST dump, Core's dump, the back end's program) against
Rust's, byte for byte; focused programs for the rounding edges (the nearest
double of a midpoint, subnormals, `-0.0`, `inf`/`nan` from arithmetic) and
for each operation's NaN behaviour; the three builtins in both evaluators and
natively, including the failure.
