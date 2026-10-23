# One wording for the one-shot trap, and the sweep's smaller gaps

**Status:** done (2026-10-23, branch `auto/elya`). HANDOFF step 1. The design
choices are Claude's under the maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at 9327f74, the lock on ebc2635)

The 2026-10-22 sweep (`scripts/sweep.py`, 735 programs) listed four
`both-fail-differently` programs, seven refusals and four build errors. Each gap was
measured again by hand:

- **The one-shot trap (4 programs: 0077, 0083, 0323, 0524).** Both sides fail with
  exit 1 after the same output, in different words.
  - The evaluator says `[E0425] continuation resumed more than once`, labelled "this
    handler is one-shot — use `with multi` for multi-shot".
  - Native code says `elya: resume: a one-shot continuation was resumed twice`.

  The sweep calls two failures the same only when the native line, after `elya: `,
  is inside the evaluator's message. Every other native trap follows that rule:
  `integer overflow`, `division by zero`, `int.shl: a shift by 64 is outside 0..=63`.
  The label is also wrong for 0077: its effect is not declared `multi`, so `with multi`
  is E0427 there ("a one-shot effect cannot be handled with `multi`", the 4d-1
  conformance rule).
- **A constructor used as a function value.**
  - Natively, a constructor with fields used as a value SEGFAULTS, even when its type
    is not parametric:
    - `type W { Wrap(Int) }` then `let f = Wrap  match f(5) { Wrap(h) -> h }`: the
      evaluator says 5, native code says SIGSEGV;
    - `fn ap(f, x) { f(x) }` then `ap(Wrap, 5)`: the same;
    - `map(xs, Wrap)`: the same.
  - Core lowers a bare constructor name to `Ctor(name, [])`, a nullary construction
    (5b-4 §3.2). With fields, that node is a one-word block typed as a function. A
    call jumps through its tag word.
  - With a parametric type (`map(xs, Some)`, `let f = Some`, `ap2(P, 3, True)`), the
    construction misses `mono`'s instance table and is refused, "parametric ADT".
  - The sweep saw only the parametric case (1 program); nothing in the corpus used a
    monomorphic constructor as a value.
- **An alias of an effect-polymorphic local (1 program: 0155).**
  - `let mk2 = mk` then `mk2(1)` used at a user effect is refused, "effect-polymorphic
    function used at a user effect". So are two chained aliases of a local
    (`ap2 = ap`, `ap3 = ap2`) and an alias of an alias of a top-level function
    (`let t = twice  let t2 = t`).
  - The evaluator gives 8, 608 and 14. A type-only alias (`let id2 = id` used at
    `Int` and `Bool`) already works natively (11).
  - The cause: `specialize` clones a local per reference, by the row each reference is
    instantiated at. The alias's own binding references `mk` at the alias's GENERAL
    type (a row variable), so the clone is direct. `mk2`'s uses at `S` find nothing
    to clone.
- **A user effect named `IO` (1 program: 0495).** The checker accepts
  `effect IO { fn write(s: String) -> Unit }`, and the evaluator runs it. Core refuses
  it, "effect IO", because every back-end pass reads "a user effect" as
  `label != "IO"`.

  In the checker the name is already ambiguous: a handler removes `IO` from the
  handled row and then adds it back as the residual "never dispatched to a handler"
  (`types.rs`, the handle rule). So a clause `IO.write` handles the user's operation,
  while the built-in `io.println` still prints past it.
- **`Float` in Core (3 programs).**
  - The evaluator has `Float` literals, `+. -. *. /.`, comparisons, `==` (NaN is
    unequal to itself) and `with any` encoding.
  - Natively, Core refuses the literal.
  - Making it native needs a float word representation, IEEE arithmetic, and a printer
    that writes Rust's `{}` for `f64` (shortest round-trip, no exponent) in C. That
    printer is how `op.encode` and `--value` show a float.
- **Not gaps:**
  - a `Bool` main (2 programs) is refused by `--value` on both sides by design
    (2026-10-16);
  - `too many type instances` and the operation value (3) are HANDOFF step 2.

## 1. Decisions

- **D1. One wording: "a one-shot continuation was resumed twice", on both sides.**
  - It names what was resumed (the one-shot kind) and when it stops (the second
    resume, which is where both sides trap).
  - The evaluator keeps E0425 and its span. Its label becomes "this handler is
    one-shot: only a `with multi` handler of an `effect multi` resumes more than
    once". That is right for both shapes the sweep found: an effect not declared
    `multi`, and a `multi` effect under a plain `with`.
  - Native code prints `elya: a one-shot continuation was resumed twice`. The
    `resume: ` prefix goes: like the other traps, the native line is the evaluator's
    message.
  - *Options:*
    - (a) Native takes the evaluator's words. "More than once" says less than "a
      one-shot ... twice".
    - (b) A longer native message carrying the help. That is not the evaluator's
      message, so the sweep's rule would still fail.
    - (c) **Taken:** native's words on both sides, and a corrected label.
- **D2. A constructor used as a value is a closure natively.**
  - `anf` (the pass that already wraps a top-level function used as a value) gains a
    third rule: `Ctor(C, [])` whose type is a function `fn(t1..tn) -> T` becomes
    `fn($eta.k.0..) { C($eta.k.0..) }` at that type.
  - The inner construction is an ordinary saturated `Ctor` typed `T`, so `mono`
    instantiates a parametric `T` like any other construction.
  - Core keeps lowering the source as written (its origin proofs are unchanged); the
    rewrite is native-only, like rule 2.
  - *Options:*
    - (a) Eta-expand in Core lowering. Its lookup cross-check would then need an
      exemption for synthesized nodes.
    - (b) Emit a constructor closure in the emitters. That is two emitters, and
      `mono` would still not see a construction.
    - (c) **Taken:** `anf`.
- **D3. An alias of a row-polymorphic local is substituted natively** (narrowed by
  the review, §3).
  - `anf` gains a fourth rule: `let x = y in b`, where `y` is a local whose type at
    the binding has an open row (or is a `with any` operation) and no top-level
    function is named `y`, becomes `b` with every `x` renamed `y`. Each occurrence keeps its own
    type, so `specialize` and `mono` see `y` referenced at exactly the instantiation
    each use needs. A chain of aliases resolves to its root.
  - It is sound because the evaluator's `let x = y` evaluates nothing (a variable is
    a value) and binds the same value. HM gave `x` an instance of `y`'s scheme, so
    each use of `x` is also an instance of `y`.
  - **Capture:** if anything in `b` binds `y` again (a `let`, a parameter, a pattern,
    a clause parameter, a return binder), the `let` is kept as it is. That case still
    meets the existing refusal by name.
  - *Options:*
    - (a) Teach `specialize` to clone through alias bindings. That is a second
      reference graph, for a binding that does nothing.
    - (b) **Taken:** substitution in `anf`.
- **D4. A user effect named `IO` is E0209 at resolve.** The words are "effect `IO` is
  built in", with the help "rename this effect: `IO` is the effect `io.*` performs".
  - E0209 is "a name declared twice"; here the language declared it first. Both sides
    now refuse the program at `check`.
  - Core's "effect IO" refusal stays, as a guard behind it. Its test still passes:
    `a_user_declared_io_effect_is_refused_at_lowering` lowers past the diagnostics.
  - *Option:* support a user `IO` natively. That would need every
    `label != "IO"` in the back end to tell two effects with one name apart, for a
    program the checker already reads ambiguously.
- **D5. `Float` natively is its own step,** recorded in HANDOFF's open list with
  what was measured. The review's two native wrong values (§3) took the window's
  new step. The C printer that
  must match Rust's `f64` `{}` is the whole risk, and it is not a "smaller gap".

## 2. Evidence

Each test was red first, as the plan predicted:
- `crates/codegen/tests/native_codegen.rs`:
  - `the_one_shot_trap_says_the_same_words_on_both_sides` (D1, the sweep's four
    programs). Red: `"resume: a one-shot continuation was resumed twice"` against
    `"continuation resumed more than once"` on all four.
  - `CTOR_VALUES` (D2, 13 rows, two tests: run natively, and against the
    evaluator). Red: v1-v3, v8-v10 and v13 SIGSEGV; v4-v7, v11 and v12 were
    refused, "parametric ADT". v12 builds 5 000 `Some`s through the wrapper
    (12 497 500).
  - `ALIASES` (D3, 8 rows, two tests). Red: w1-w4 refused, "effect-polymorphic
    function used at a user effect". w5-w8 were green before; they guard the
    substitution (a type-only alias, the rebound target, one alias at two
    effects, an alias of a parameter).
- `crates/codegen/src/anf.rs`: `a_constructor_used_as_a_value_is_wrapped` and
  `an_alias_whose_target_is_rebound_is_kept`. The second holds its own positive
  control (the same program with the rebinding removed loses its `let`).
- `tests/effect_syntax.rs`: `a_user_effect_named_io_is_refused`. It holds the words,
  the help, `check` failing, and a negative control (`effect Io` checks).

**Changed expectations, by D1:**
- the native trap's words in `a_second_resume_traps_natively_and_errors_in_the_evaluator`,
  `a_plain_handler_over_a_multi_effect_is_one_shot_natively` and
  `a_one_shot_continuation_saved_in_a_multi_shot_segment_fails_on_both_sides`
  (`elya: resume: ...` to `elya: a one-shot ...`);
- `resuming_twice_in_a_one_shot_handler_is_e0425` (`tests/effects_run.rs`) now also
  asserts the message and the label. Red first on the message.

**Changed expectation, by D3 (found by the first gate run):**
`any::tests::an_operation_value_used_as_a_value_is_refused`'s first clause stored the
operation by a bare alias (`let p = o  resume(op.perform(p))`). The alias is now
substituted, so that program compiles, and it agrees with the evaluator. The clause
now stores `o` through an `if` (`let p = if True { o } else { o }`), which is still
refused.
- The alias itself is a new test,
  `native_any.rs::an_aliased_operation_value_runs_natively` (`let p = o  let q = p`,
  then `op.show(q)` and `op.perform(p)`: the evaluator's two lines and 80).
- It was red under K3: "operation value used other than by op.* or ==".

**Changed test, by D3 (found by the second gate run):**
`an_alias_of_a_generic_local_used_at_a_user_effect_is_refused_by_name` pinned the deep
callee guard's refusal on the 2026-10-08 review's program, which is w1 here and now
runs (8).
- The guard stays, because an alias whose body rebinds its target is kept. So the test
  now rebinds it: `let k = { let mk = 0  mk2(1 + mk) }`. The evaluator still gives 8,
  and native code still refuses it by name.
- Its name and purpose (the guard's words) are unchanged.

Also probed by hand, all agreeing with the evaluator:
- a constructor value inside a generic function (`fn wrap_all(xs) { map(xs, Some) }`
  at `Int` and `Bool`);
- an alias of `Some` used at two types;
- a constructor captured by an effectful closure;
- a constructor value in a `with multi` run;
- `(Wrap)(5)` and `let f = (Wrap)`;
- a returned constructor value (`pick(True)(1)`);
- a linear type's constructor.

Controls (each reverted, `cmp` against the kept files):
- **K1**, rule 3 off: the 13 rows fail as before the change, the seven
  monomorphic ones by SIGSEGV.
- **K2**, rule 4 without the capture check: the capture unit test fails ("the alias
  was substituted under a rebinding of its target"). The compiler PANICS on w6 in
  inkwell: `ap2(fn() { ap })` renamed under `let ap = 5` calls an `Int`.
- **K3**, rule 4 off: w1-w4 refused by name again.
- **K4**, the old label: `resuming_twice_in_a_one_shot_handler_is_e0425` fails on
  the label.

The sweep (`scripts/sweep.py`, 758 programs collected with the new tests):

| result | before (735) | after (758) |
|---|---|---|
| same | 405 | 429 |
| both-fail | 14 | 18 |
| both-fail-differently | 4 | 0 |
| refused | 7 | 6 |
| build-error | 4 | 3 |

There is no `differ`, crash or panic.
- The four one-shot programs are now `both-fail`.
- `map(xs, Some)` (0395) and the alias 0155 are `same`, and `effect IO` (0495) is
  `not-checked`.
- The one alias refusal left is the rebinding guard's own program (0183), as intended.
- The other refusals are step 2's (`too many type instances`, the operation value
  passed to a function or captured) and the `Bool` main by design.
- The build errors are the three `Float` programs (D5).

After the review narrowed D3 (§3), the sweep collected 771 programs: 442 same, 18
both-fail, 6 refused, 3 build errors, and still no `differ`, crash or panic.

Gate (Linux): **1100 passed, 106 suites** (predicted 1099; see the plan), the same
after the review's fixes. The review's programs are rows of an existing corpus.

## 3. Review (2026-10-23)

An independent review ran:
- 4 500 fuzzed programs: aliases, alias chains and shadowing by every binder kind;
  constructors with 1 to 6 fields; multi-shot handlers;
- a 120-program matrix of an alias's source × its depth × where an upcast happens;
- examples 01-10;
- GC stress on the constructor programs.

D1, D2 and D4 held. **D3 as first written had three regressions, all fixed before
the commit** by narrowing the rule. Each is now an `ALIASES` row (w9-w21), and each
was red against the first version:
- **F1 (high, a wrong value).** An alias of a TOP-LEVEL function, then a sub-effecting
  upcast of its result (`let m = mk  let q: fn() -> fn() / {E} -> Int = m`): native
  gave 5 where the evaluator gives 15.
  - The rename made the later use a rule-2 wrapper of `mk`, whose result layer
    nothing coerces.
  - When `m` was a local, the upcast adapter coerced it at the use.
  - Fix: `y` must be a local, so an alias of a top-level function stays one (w9-w11).
  - The alias-of-an-alias case w3 still runs: the first alias is wrapped once and
    stays a local, and the second is renamed.
- **F2 (medium, new refusals).** An annotated upcast through an alias
  (`let f: fn(Int) / {E} -> Int = p`) became a call of `p` at the effectful type, which
  the adapter does not coerce at a callee.
  - Fix: `y`'s type at the binding must have an open row. An annotation cannot write
    a row variable, so an annotated alias keeps its `let` (w12, w13).
- **F3 (high: wrong values, compiler panics, LLVM verification failures).** Renaming
  to a local that SHADOWS a top-level function (`let inc = fn(z) { z * 2 }  let g = inc`)
  planted the name inside lambdas and handle bodies. There closure conversion reads
  a free name as the top-level function whenever one has that name: 2004 for 1007,
  and inkwell panics in fuzzed seeds.
  - That reading is a pre-existing bug: the same program without the alias gives
    2004 natively before this slice too (now HANDOFF step 1).
  - Fix: no top-level function may be named `y` (w14-w21).
- Controls on the narrowed rule (each reverted, `cmp` after):
  - **K6**, no top-level-name check: w17 gives 2004 (for 1007), w20 6 (for 202).
  - **K7**, closed rows allowed: w12 refused by name.
  - **K8**, the first version (any local or top-level `y`): the compiler panics in
    inkwell on the `ALIASES` corpus.
  - The rows of F1 and F3 that use a closed-row target are guarded twice: by the
    open-row check and by the local/top-level checks. No row depends on "`y` is a
    local" alone, which also excludes names that are neither, such as an
    operation used as a value.
- The review's fuzzers, re-run on the narrowed rule:
  - the 800 programs of its first generator: 717 agree, 48 fail on both sides in
    the same words, 35 are refused by name, with no mismatch and no crash;
  - 500 of its second: 382 agree, 35 fail on both sides, 76 are refused, with no
    mismatch.
  - The second's 7 build failures (seeds 1074, 1096, 1151, 1203, 1392, 1437, 1443)
    each also fail on the build before the slice. 1437 used to be refused
    "parametric ADT" first; it now reaches the same shadowing panic (HANDOFF step 1).
- **Low, not changed:** two `effect IO` declarations give three diagnostics. All three
  are true ("built in" twice, "declared more than once").

**Pre-existing, recorded for the next step (none is caused by this slice):**
- a local that shadows a top-level function, used free in a lambda or a handle body,
  runs the top-level function (wrong values), or panics the compiler in inkwell;
- rule 2's wrapper does not coerce a result-layer upcast (`ap2(mk)`: 5 for 15);
- fuzzed seeds that fail LLVM verification or crash on both builds.

The reviewer's scripts and programs are kept in the session scratchpad, not the repo.
