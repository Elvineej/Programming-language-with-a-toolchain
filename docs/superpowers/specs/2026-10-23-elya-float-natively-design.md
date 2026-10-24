# `Float` natively

**Status:** done (2026-10-23, branch `auto/elya`, a scheduled run). HANDOFF
step 1. The design choices are Claude's under the maintainer's delegation (rule 2),
recorded here.

## 0. Measured first (at 9233648, gate 1184 / 113)

What the evaluator has (read from `eval.rs`, `types.rs`, `literal.rs`):

- literals (`1.5`; `-1.5` is a literal since 2026-10-23; no exponent syntax),
  `+. -. *. /.` (IEEE, no failure: `1.0 /. 0.0` is `inf`, `0.0 /. 0.0` is `nan`);
- `==`/`!=` with IEEE meaning (a NaN is unequal to itself, `-0.0 == 0.0`), also
  inside declared data and inside an operation's arguments (`o == o` is `False`
  when an argument is a NaN);
- NO comparisons (`<` and friends are `Int` only), no unary minus (`-x` is
  `Int`'s), no conversion to or from `Int`, no builtin that writes a float: the
  only text is `op.show` / `op.encode` of a `with any` operation;
- the text: Rust's shortest round-trip `Display`, never an exponent, `.0`
  appended when integral, and the words `inf`, `-inf`, `nan` (a NaN's sign is
  dropped). Measured through `op.show` on both edge and ordinary values:

| value | text |
|---|---|
| `1.5` | `1.5` |
| `0.0 -. 0.0` | `0.0` |
| `-0.0` | `-0.0` |
| `0.1 +. 0.2` | `0.30000000000000004` |
| `1.0 /. 0.0`, `-1.0 /. 0.0`, `0.0 /. 0.0` | `inf`, `-inf`, `nan` |
| `1e22 *. 1e19` | `100000000000000000000000000000000000000000.0` |
| `9007199254740993.0` (2^53 + 1) | `9007199254740992.0` |
| `1.0 /. 3.0` | `0.3333333333333333` |
| `0.0000001` | `0.0000001` |
| the smallest subnormal | `0.` then 323 zeros then `5` |

- decoding (`op.decode`) reads `[-]digits.digits` with Rust's `f64::from_str`
  (`1.` and `-.5` are accepted), and `inf`, `-inf`, `nan`.

Natively: Core refuses the literal (`Unsupported("Float")`); the sweep has 3
programs refused for it. The runtime's literal parser already scans a float
(`LIT_FLOAT`) but keeps no value; nothing prints one.

## 1. Decisions

- **D1. A `Float` is its IEEE-754 bits in an untraced word.** Everywhere a value
  lives natively (registers, fields, captures, frames, spills) a float is an
  `i64` holding the bits, exactly like an `Int`; arithmetic bitcasts to `double`,
  computes, and bitcasts back. Core gains `CoreLit::Float(bits)` (bits, so a
  literal is `Eq` and hashable, and `-0.0` and every NaN payload survive).
  - *Rejected:* boxing (a heap block per float, as strings): an allocation per
    operation and a traced word where no pointer is.
- **D2. Equality is IEEE's, also inside data.** `==`/`!=` on two floats is
  `fcmp oeq` / `une`. Inside declared data `elya_eq` compares words by bits, which
  is wrong for both NaN and `-0.0`, so the back end hands the runtime a second
  table, one float mask per tag (bit `i` set iff field `i` is a `Float`),
  through a new `elya_eq_init`; `elya_eq` compares those fields as doubles. The
  collector's descriptor rows and `gc_mark` are unchanged.
  - `o == o` over an operation (`elya_any_check`) is false when its walk meets a
    NaN, as the evaluator's structural `==` is.
  - *Rejected:* widening the descriptor rows (`gc_mark` is kept byte-identical by
    standing rule) or a float tag in the pointer mask's high bits (it would cap
    arity silently).
- **D3. One printer, proven against Rust.** `runtime.c` gains the float printer
  the literal syntax needs: the shortest digit string that reads back to the same
  double, the closest such string when several are equally short, written as a
  plain decimal (no exponent), `.0` when integral, `inf`/`-inf`/`nan`. It tries
  each length from 1 to 17 digits, the correctly rounded string (`%.*e`) first and
  then its two neighbours in the last digit (the closest string can fail to read
  back near a power of two where the gap below is half the gap above), and keeps
  the first that `strtod` reads back exactly.
  - Proof by execution: a C harness linked with `runtime.c` prints the bits of
    every edge value of §0 and 200,000 random bit patterns (a fixed seed), and the
    test compares each line with the evaluator's text.
  - *Rejected:* a port of Ryu or Grisu (thousands of lines and tables, for a
    printer that is not on a hot path); `%.17g` (not shortest: `0.1` would be
    `0.10000000000000001`).
- **D4. Decoding reads with `strtod`** on the text the scanner already accepts,
  which is correctly rounded as Rust's `from_str` (glibc; the Windows CRT has been
  since 2015, and the Windows gate must confirm). The literal keeps its bits.
- **D5. `float.to_string(x) -> String`, on both sides,** pure, writing exactly the
  literal text (so a float is observable without `with any`). `int.to_string`'s
  sibling; arity 1.
  - `float` joins the builtin modules: `import elya/float` is accepted, and a user
    module imported under the name `float` is E0604 (modules part 1 D2), as for
    `int`.
  - *Not in this step, recorded as open:* comparisons on floats, unary minus on a
    float expression, conversions between `Int` and `Float`.

## 2. Evidence (planned)

- `tests/float_text.rs` (both configurations): `float.to_string` in the evaluator
  on the table of §0; its type (pure, `Float -> String`, arity).
- `crates/codegen/tests/native_float.rs` (execution, differential against the
  evaluator): arithmetic (`inf`, `nan` by division); equality on floats, inside
  declared data and lists (`Box(nan) == Box(nan)` false, `Box(-0.0) ==
  Box(0.0)` true); floats in fields, captures, CPS frames across a perform and in a
  resumed answer; `with any` showing, encoding and decoding floats (`inf`, `-0.0`,
  `1.`, the malformed words) and `o == o` with a NaN argument; the printer harness
  (D3).
- Negative controls (in `native_float.rs`'s header): equality inside data by bits
  (D2), the neighbour step of the printer off (D3, the harness must find a
  difference), `elya_any_check` ignoring NaN (D2).

## 3. Results

Gate: **1193 passed, 116 suites**, as predicted. The seven planned tests were red
first as predicted (E0201 twice; `Unsupported("Float")` four times; the harness
did not link). Controls K1-K3 are in `native_float.rs`'s header, each failing one
distinct test; K2 showed the neighbour step is needed (45 values next to a power
of two).

Found on the way, by the harness: `%.*e` rounds a tie half to even, while Rust's
shortest form rounds it half up (2^-25 is `0.0000000298023223876953125` and
writes `...95313`; 58 of the first run's values). The printer now rounds each
candidate from the exact decimal expansion (`%.780e`), half up.

Also changed by D2: `o == o` over an operation (`any.rs`'s `op_eq`) checks its
arguments last first and stops at the first difference, as the evaluator's walk
does (a NaN can now be that difference; before, every argument was checked).

### The review (2026-10-23)

An independent review found no defect. It ran its own printer harness on 475,036
further values (random short decimals, integers, every m*2^k including ties and
all subnormals, powers of ten +-2 ulp, 9...9 x 10^e carries; also under ASan and
UBSan), about 40 decode inputs (accepted and rejected, with the same words), floats
through generic functions, closures, mixed data, CPS frames, multi-shot resumes and
return clauses, equality at type variables, `o == o` with NaN and function
arguments in both orders, and a collection over 200,000 floats whose bits look like
heap addresses. Two notes, recorded: the AST's debug printer writes `1.0` as `1`
(older); the printer's `%.780e` and `strtod` are exact on glibc, and the Windows
gate must confirm the UCRT.

Sweep (`scripts/sweep.py`, 886 programs): 490 same, no disagreement; the 3 `Float`
build errors are gone.

Windows: `runtime.c` changed (`elya_float_text`, `elya_float_to_string`,
`elya_eq_init` and `elya_eq`'s float fields, `with any` floats).
