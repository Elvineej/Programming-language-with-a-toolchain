# Native strings: `<>`, `==` and `int.to_string`

**Status:** in progress (2026-10-15). HANDOFF step 2, taken before step 1 on purpose: the
poll-loop example of step 1 has to print times and task ids, and nothing can turn an
`Int` into text today. The design choices are Claude's under the maintainer's
delegation (rule 2), recorded here.

## 0. Measured first (at 258dda6)

| Program | `check` | `run` | `build` |
|---|---|---|---|
| `io.println("ab" <> "cd")` | ok | `abcd` | "unsupported construct (Concat)" |
| `io.println(int.to_string(42))` | E0201 "unknown builtin `int.to_string`" | E0201 | (front end) |
| `if "a" == "a" { 1 } else { 0 }` | ok | 1 | "Eq on an unrepresentable operand type" |
| `greet(n) = "hi " <> n`, printed and compared | ok | `hi ada!` | "Concat" |

**The ~700-program sweep.** In the sweep used for d07afea, `<>` is the most common
native refusal of a real program (16). Only "no `main`" refusals are more frequent (23),
and those are test fragments. No example in `examples/` uses `<>`: every native example
prints a number, because nothing else could be built.

## 1. Decisions

- **D1. `int.to_string(n: Int) -> String`** is a pure builtin. It is module-qualified like
  `io.println`, and its output is decimal with a leading `-` for a negative number,
  `i64::MIN` included.
  - *Rejected:* a `show`/`Display` trait (Elya has no type classes), and string
    interpolation (new syntax; it can desugar to this builtin later).
- **D2. Representation unchanged.** A string is an immutable heap block `[tag][len][bytes,
  NUL]`, traced by a row that traces nothing. `a <> b` allocates a fresh string and copies
  both, at O(|a| + |b|). Ropes or a builder are deferred: nothing measured needs them.
- **D3. `==` and `!=` on `String`** are byte equality, as the evaluator's Rust `String`
  comparison is. Ordering (`<` on strings) stays a type error.
- **D4. Both are allocation sites for the collector**, and an un-rooted operand would be a
  silent use-after-free.
  - **Direct emitter:** the left operand is rooted while the right one is lowered (the
    right may allocate), then the environment is rooted across the call, as `Ctor`
    does.
  - **CPS emitter:** operands are already rooted by `operand`, and `root_live` covers the
    continuation and the bindings.
  - **Runtime:** `elya_str_concat` also pushes both operands while it allocates (defence in
    depth, as `elya_cont_copy` does).
- **D5. Portable runtime code only**: `memcpy`, `memcmp`, `snprintf("%lld")`. There are no
  platform calls, so the Windows gate needs nothing new.

## 2. Evidence (planned)

- `tests/strings.rs`: the front end and the evaluator for `int.to_string`. This covers
  its type, its purity, a misspelt builtin, and decimal output at `0`, a negative and
  `i64::MIN`.
- `crates/codegen/tests/native_codegen.rs`:
  - a strings corpus compared with the evaluator, text and value. It includes `<>` of
    computed strings, `==`/`!=`, and `int.to_string` inside an effectful function;
  - a collection test (thousands of concatenations kept live in a list while the
    collector runs);
  - a CPS test whose right operand performs an effect, so the left operand waits across
    a continuation site.
- **Three unit tests flip from refusal to compilation, on purpose (D1-D3):**
  `rejects_concat_by_name` and `rejects_eq_on_strings_by_name`. The third,
  `rejects_eq_on_an_unrepresentable_operand_type`, keeps guarding `==`'s allow-list with
  an ADT operand instead of a `String`.
