# Native strings: `<>`, `==` and `int.to_string`

**Status:** done (2026-10-15), after an independent review (§3). HANDOFF step 2, taken before step 1 on purpose: the
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
- **D6. A builtin used as a value is E0208** ("builtin `int.to_string` used as a value",
  with help on how to wrap it in a function). It used to pass `check`, then fail at run
  time ("must be called") or in the native build. That was already true of
  `io.println`. Refusing it matches how an operation used as a value is handled
  (E0206).
  - *Rejected:* making a builtin a first-class value by eta-expansion. It would
    work, but nothing needs it, and a lambda says the same thing explicitly.

## 2. Evidence (planned; as measured in §3)

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

## 3. As measured, and the review

- **Sweep.** On the ~700-program sweep, exactly the 17 programs refused before (16 for
  "Concat", 1 for string `==`) now compile, and all 17 match the evaluator byte for
  byte. No other native result changed.
- **Controls.**
  - C2 (lengths-only `==`) and C3 (no minus sign) failed exactly the rows predicted.
  - **C1** (the left operand not rooted while the right is lowered) failed nothing at
    first. A collection frees about 30,000 blocks, and the per-size free lists hand
    back the OLDEST dead block first, so a freed operand was almost never reused
    before it was read.
    - A program built to make the reuse certain became
      `a_left_operand_survives_a_collection_in_the_right_one`. Its left operand is a
      9-word literal, the only block of its size class, and its right operand churns
      and then allocates a same-size string. C1 fails it: 0 instead of 1, and the
      collector reports `live=0`.
- **Independent review: no correctness bug.** The reviewer re-ran the gate (930 / 89)
  and the sweep (the same 17). It also ran a torture runtime that collects on every
  allocation, poisons freed blocks and checks shadow-stack balance at exit; all 402
  programs that run under it are balanced and match the evaluator. Its findings, all
  fixed test-first:
  - **CPS roots (medium).** Controls C4 and C5 (no `root_live` around a CPS `<>` /
    `int.to_string`) failed nothing in my probes, and I had called them defence in
    depth. That was wrong. In a resumption function the resumed frame is dead, so the
    continuation and the frame's saved bindings are reachable only through what
    `root_live` pushes.
    - Two deterministic tests now pin them:
      `cps_live_values_survive_a_concat_in_a_resumption` (C4: 7021 for 20 000) and
      `cps_live_values_survive_int_to_string_in_a_resumption` (C5: 5242).
  - **A builtin used as a value passed `check` (low, existed before this change).**
    Now E0208 (D6).
  - **String-literal patterns compile natively.** `patterns.rs` turns a literal test
    into `==`. This was a side effect; it is now pinned by three corpus rows (flat,
    nested, and on a perform's answer). A non-exhaustive `String` match is still
    E0430.
  - **Docs:** the README's "strings have no operations" sentence; the `gc_root` doc
    comment, which my helpers had separated from its function; and the plan's test
    counts.
