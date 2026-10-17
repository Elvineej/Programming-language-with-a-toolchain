# Unary operators natively, and explicit wrapping and bit arithmetic

**Status:** in progress (2026-10-17, branch `auto/elya`). HANDOFF step 1. The design
choices are Claude's under the maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at b701e89)

- Core refuses every unary operator (`LowerError::Unsupported("Unary")`), so
  `elya build` refuses `-x` and `!b`. The 2026-10-17 sweep (700 programs) shows no
  `Unary` refusal: every program meant to run natively avoids them (`0 - 1`,
  `if b { False } else { True }`). The gap is the language's, not the corpus's: the
  evaluator runs `-x` (checked: `-MIN` is `integer overflow`) and `!b`.
- `Int` arithmetic is checked (spec 2026-10-10): `+`, `-`, `*` fail by name on
  overflow, on both sides. So a hash or a generator in Elya cannot be written the
  usual way: FNV-1a multiplies by `1099511628211` modulo 2^64, xorshift needs `^`,
  `<<`, `>>`. `examples/09_record_any.elya`'s generator is a 31-bit LCG kept small
  enough not to overflow, by hand. There is no bit operation at all.

## 1. Decisions

- **D1. Unary operators lower in Core, as what they mean.** `-e` is `0 - e` and `!e`
  is `e == False`, both on the unary node's span.
  - `0 - e` overflows exactly when `e` is `MIN`, as `checked_neg` does, with the same
    words (`integer overflow`): the back end's checked subtraction already traps.
  - `e == False` is `Bool` equality, which both emitters compile.
  - So nothing downstream of Core gains a node kind.
  - *Rejected:* a `CoreKind::Unary`. Every pass would need a case, for two operators
    that are already expressible.
- **D2. Wrapping is explicit, by name; checked stays the default.** New builtins, all
  `fn(Int, Int) -> Int` and pure:
  - `int.wrapping_add`, `int.wrapping_sub`, `int.wrapping_mul`: two's complement,
    modulo 2^64.
  - `int.and`, `int.or`, `int.xor`: bitwise.
  - `int.shl(x, n)`, `int.shr(x, n)` (arithmetic: the sign is kept),
    `int.ushr(x, n)` (logical: zeros come in). The amount must be in `0..=63`.
    Anything else fails by name, on both sides: `int.shl: a shift by 64 is outside
    0..=63`.
    *Rejected:* masking the amount (`n & 63`, as Rust's wrapping shifts and the x86
    instruction do). That makes `x << 64 == x` silently; a named failure is the
    stricter option, as for division by zero.
  - *Rejected:* operators (`^`, `<<`, `&`, `|`). `|` and `&` are close to `||` and
    `&&`, which Elya already has, and the operators would need precedences. Named
    builtins say what they do, and "wrapping" is visible at the call.
  - *Rejected:* a wrapping integer type. That is a second numeric type, conversions,
    and literals typed by context, for what a hash function does in three calls.
  - Names follow Rust's (`wrapping_mul`), which is what a reader searching for it
    would type.
- **D3. Natively**, the wrapping operations are plain LLVM `add`/`sub`/`mul` (no
  overflow check), the bit operations `and`/`or`/`xor`/`shl`/`ashr`/`lshr` after a
  range check of the amount that traps through the runtime with the evaluator's
  words.

## 2. Evidence (planned)

- `tests/int_wrapping.rs` (root): the types (each builtin's signature, a misuse is a
  type error), the evaluator (the boundary values, each shift-amount failure), and
  Core lowering of `-x` and `!b` (D1).
- `crates/codegen/tests/native_arith.rs`: unary operators and every builtin through
  the evaluator and natively, the same output or the same failure; an FNV-1a hash
  and a xorshift generator.
- Negative controls (recorded in the native test's header): `-e` lowered as `0 + e`;
  `int.shr` compiled as the logical shift; the shift range check off.

## 3. Results

Negative controls K1-K3 recorded in `native_arith.rs`'s header; each failed a distinct
test, files restored byte for byte. Plan deviation: `unary_operators_agree` was
written after Core's D1 lowering, so it was not red first natively (its root
counterpart `unary_operators_lower_to_core` was red with `Unsupported("Unary")`, and
K1 shows the native test has teeth).

Gate: 1077 passed, 105 suites, as predicted; 1079 after the review's fix (one root
test, red first).

### The review (2026-10-17)

About 95 differential programs, all agreeing. Findings:
- **L1, fixed test-first.** E0208's help wrapped every builtin used as a value in a
  one-parameter function (`fn(x) { int.xor(x) }`, itself an arity error; also wrong
  for `time.now_ms`). It now writes the builtin's own arity
  (`resolve::builtin_arity`).
- Older than this slice, recorded in HANDOFF: a line starting with `-` continues the
  previous line as a binary minus (the known layout question); `MIN` cannot be
  written as a literal (`-9223372036854775808` is E0001 "not a valid Elya token",
  misleading for an out-of-range literal); the one-shot trap's wording (step 4).
