# Captures by scope, and a wrapped function's result coerced

**Status:** done (2026-10-23, branch `auto/elya`). HANDOFF step 1. The design
choices are Claude's under the maintainer's delegation (rule 2), recorded here. Both
bugs were found by the 2026-10-22 slice's review
(`2026-10-22-elya-one-shot-words-and-sweep-gaps-design.md` §3), on the build before
that slice too.

## 0. Measured first (at 2d8f62c)

**A local that shadows a top-level function.** `fn inc(x) { x + 1000 }`, and in `main`:

| program | the shape | evaluator | native |
|---|---|---|---|
| h1 | `let a = inc(1)  let inc = fn(z) { z * 2 }  let f = fn(q) { inc(q) }` | 1007 | **2004** |
| h4 | the same `inc` bound by a pattern (`B(inc) -> ..`) | 1007 | **2004** |
| h8 | a clause parameter `inc` captured by a lambda | 1004 | refused, "function used as a value" |
| h9 | `let inc = 7` captured by a lambda | 1011 | refused, the same |
| q1 | `let inc = fn ..` in a handle body, passed to `map` | 15 | **compiler panic** (inkwell) |
| p0 | `let mk = mk  let mk = fn ..` in a handle body | 8 | **compiler panic** |
| h2, h3, h10 | a parameter `inc`; no free use of the top-level `inc` | agree | agree |
| h5-h7 | the shadowing `inc` used in a handle body, clause or return clause, outside any lambda | agree | agree |

- The cause, `closure.rs::collect_lambdas`: a lambda's capture list leaves out every
  name free in the WHOLE enclosing function ("module-level, so not a capture").
- In h1 the top-level `inc` is used free (`inc(1)`), so the local `inc` the lambda
  uses is dropped from its captures. The lifted body then finds no local `inc` and
  calls the top-level one, or refuses a top-level function used as a value.
- The comment above it claims the rule "gets a local that shadows a top-level name
  right". That holds only when the top-level name is not ALSO used free.

**A wrapped top-level function whose result is upcast.** `fn mk() -> fn() -> Int { fn() { 5 } }`:

| program | where `mk` goes | evaluator | native |
|---|---|---|---|
| u1 | a parameter `fn() -> fn() / {E} -> Int` | 15 | **5** |
| u2 | joined with `fn() { fn() { e() } }` by an `if` | 510 | **5** |
| u3 | a field `fn() -> fn() / {E} -> Int` | 15 | **5** |
| u4 | two result layers (`fn() -> fn() -> fn() / {E} -> Int`) | 15 | **139897419601040** |
| u5 | `fn mk(n) -> fn(Int) -> Int` at `fn(Int) -> fn(Int) / {E} -> Int` | 13 | **3** |
| u6 | the same through a local lambda `fn() { mk() }` | 15 | 15 |

- The cause: `anf` rule 2 wraps `mk` as `fn() { mk() }` at the USE's type.
  `inner_type` corrects the inner call's row, but not its result: the call of `mk` is
  typed as returning an effectful closure, while `mk` returns a direct one.
- `specialize` coerces a result upcast only at a call of a LOCAL binder (u6 works
  that way).

## 1. Decisions

- **D1. A lambda captures by scope.** `collect_in` threads the names bound around
  the lambda: the function's parameters, `let`s, patterns, enclosing lambdas'
  parameters, clause parameters and the clause continuation, return binders. A name
  free in the lambda is a capture iff it is bound there. Otherwise it is
  module-level.
  - This is the rule the CPS passes already use (`cps.rs::saved_at` and
    `handler_saved` look a name up in the scope at that point).
  - *Option:* rename every local that shadows a top-level function before closure
    conversion. That hides the rule instead of fixing it, and every later pass would
    still be free to rebind the name.
- **D2. A wrapper's inner call has the function's own result type.** When `f`'s own
  row is closed, `inner_type` also instantiates `f`'s own result, by matching its
  signature against the use's type.
  - If that differs from the use's result type, the wrapper is
    `fn(a..) { let $eta.k.r = f(a..)  $eta.k.r }`: the call at `f`'s own result, the
    variable at the use's.
  - `specialize` already coerces a value use of a tracked local binder whose type
    disagrees with its binding (the adapter that makes u6 right). The `let` makes the
    result such a binder.
  - It applies only when the instantiated own result is ground (no type variable, no
    open row). Otherwise the use's type is kept, as today.
  - *Option:* teach `specialize` to coerce calls of top-level functions as it does
    calls of locals. Rule 2 is the only place a top-level function's call is typed
    from a use rather than from its own signature, so the fix belongs there.

## 2. Evidence

Each test was red first, as the plan predicted:
- `SHADOWED_TOP_LEVEL` (12 rows, two tests). Red: the compiler panicked in inkwell on
  q1 and p0, which aborts the test run before the other rows report.
- `WRAPPER_RESULTS` (6 rows, two tests). Red: u1-u3 gave 5, u4 a pointer
  (140534282720400) and u5 3.
- `closure.rs::a_shadowing_local_is_captured_where_the_top_level_name_is_also_used`.
  Red: no capture of `inc`.
- `anf.rs::a_wrappers_inner_call_has_the_functions_own_result`. Red: the inner call
  was typed at the use's result.

After the change all 18 rows agree with the evaluator, including q1 and p0 (the
review's panics): D1 alone fixed all 12 shadowing rows.

Controls (each reverted, `cmp` after):
- **K1**, D1 reverted (`closure.rs` as before): the compiler panics in inkwell on the
  shadowing corpus again.
- **K2**, the own result computed but not bound (the inner call typed at it, the
  wrapper's result at the use's): u1-u3 5, u4 a pointer, u5 3, as before the change.
  The `let` is what lets `specialize` coerce.
- **K3**, bound but the binding typed at the use's result: the same five wrong values.

The 2026-10-22 review's fuzzers, re-run on the same seeds:

| run | before this slice | after |
|---|---|---|
| second generator, seeds 1000-1499 | 382 agree, 76 refused, 7 build failures | 453 agree, 6 refused, **0 build failures** |
| first generator, seeds 0-799 | 717 agree, 35 refused | 736 agree, 12 refused |

There is no mismatch in either run.
- The seven build failures (inkwell panics at seeds 1074, 1151, 1392 and 1437; LLVM
  verification failures at 1096, 1203 and 1443) were all the shadowing bug of D1.
- So were the 64 + 21 "function used as a value" refusals: a local shadowing a
  top-level function, used as a value inside a lambda.
- What is still refused is "effect-polymorphic function used at a user effect".

The sweep (`scripts/sweep.py`, 789 programs): 459 same, 18 both-fail, 6 refused, 3
build errors (`Float`), and no `differ`, crash or panic. One `differ-timing` program
reads the real clock (noise, by the sweep's rule).

Gate (Linux): **1106 passed, 106 suites**, as predicted.

## 3. Review (2026-10-23)

An independent review ran:
- the requested tests, then the whole `elya-codegen` suite single-threaded;
- 3 700 fuzzed programs (the previous review's two generators, and a new one aimed
  at this slice), 900 of them under a stress runtime that collects at every
  allocation and poisons freed blocks;
- 25 hand-written D1 programs and 41 D2 programs.

There was no high finding, and no mismatch or crash in the fuzzers. On D1 it argued
that the new captures are always a superset of the old ones (the only names added
are locals shadowing a top-level function used elsewhere), so nothing is
under-captured. The scope structure matches the CPS passes' and `anf`'s.

Findings:
- **F1 (medium, older than the slice, fixed here).** A wrapper inside a
  row-polymorphic function:
  `fn wrap_it(g) { call2(mk, g) }` gave 5 for 15, and 1505 for 1515 at two rows.
  - At `anf` time the use's result still has an open row, which counts as direct,
    so a convention test made no `let`. `specialize` closed the row later.
  - Fix: the wrapper binds whenever the two result TYPES differ. A `let` that
    turns out not to be needed coerces nothing.
  - These are rows u7 and u8, red first. The other D2 programs still agree.
  - Control K2 now targets this type test.
- **F2 (medium/low, older than the slice, open).** A row variable inside DATA in the
  function's own result:
  `fn mko() -> fn() -> Option(fn() -> Int)` used at
  `fn() -> fn() / {E} -> Option(fn() / {E} -> Int)`. Native prints a pointer for 15.
  - `mono::match_tys` never binds a row tail, so that own result is never ground,
    and the use-typed path runs.
  - The reviewer's suggested fix is to match row tails against the use's rows too.
    Recorded in HANDOFF.
- **F3 (low, fixed).** Three stale comments: `free_vars_under` in `closure.rs`; rule
  4's rationale in `anf.rs`, which now says the condition is conservative; and rule
  2's module doc, which now names the `let` form.
