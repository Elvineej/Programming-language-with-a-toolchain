# Parked

- Consider RelocMode::PIC + PIE linking on all platforms. Needs a Windows gate run before landing.
- Before fast-forwarding main to the 5b-8 branch: run scripts/check.ps1 on Windows and get exit 0. Commits from 65b08e4 onward have only been gated on Linux.
  - The ≤4 source-param cap for CPS functions (5 + continuation) is untested on win64, where >5 tailcc params caused a fatal abort. Verify it there.

## Unqualified handler clauses (front-end/evaluator disagreement)

The front end accepts a clause written `op(...) -> …` with no effect name
(`OpClause.effect == None`), and inference resolves its effect through the
op-name registry. But the evaluator's `handler_handles` (`src/eval.rs`)
matches a clause only when `effect == Some(name)`, so such a clause is never
dispatched and `elya run` ends in E0300 "unhandled effect". Core lowering
refuses it by name, `Unsupported("unqualified handler clause")`, since
8374687.

Measured repro (2026-10-02, at e2efa00):

```
effect Ask { fn ask() -> Int }
fn one() { ask() }
pub fn main() -> Int {
  handle { one() } with {
    ask() -> resume(2)
    return(x) -> x
  }
}
```

- `elya check` → `ok`, exit 0
- `elya run` → `[E0300] Error: internal: unhandled effect `ask` reached the machine`, exit 1
- `elya build` → `error: core lowering failed: Unsupported("unqualified handler clause")`, exit 1
- Control: the same program with `Ask.ask() -> resume(2)` runs, exit 0.

Decided direction, to be done in its own slice after 5b-8: keep the
unqualified syntax, have inference record the resolved effect, and make the
evaluator and lowering both read it. An op name shared by two handled effects
gets rejected as ambiguous by the checker.

## Front end: should a fn sharing a name with an effect op be an error?

Measured: accepted today, evaluator performs the op. Both inference
(`infer_call` checks the op registry before the environment) and the evaluator
(`CalleeSlot::Operation` from `op_table`) resolve an op name before any
variable, so the function is unreachable by that name. Native lowering matches
them through `CoreKind::Perform` (plan D11).

Measured repro (2026-10-03, at adbd854):

```
effect E { fn ping() -> Int }
fn ping() -> Int { 5 }
fn user() -> Int { ping() }
pub fn main() {
  let v = handle { user() } with {
    E.ping() -> resume(1)
    return(x) -> x
  }
  if v == 1 { io.println("evaluator PERFORMED the op") } else { io.println("evaluator CALLED fn ping") }
}
```

- `elya check` → `ok`, exit 0
- `elya run` → `evaluator PERFORMED the op`, exit 0

Ops-first also beats a local binding: `let ping = …; ping()` performs the op.
Matches inference + evaluator; probably should be local-first in the language.
Decide with the collision question. Measured (2026-10-03, at 40c09a0 + 7b-1):

```
effect E { fn ping() -> Int }
fn user() -> Int {
  let ping = fn() { 5 }
  ping()
}
pub fn main() {
  let v = handle { user() } with {
    E.ping() -> resume(1)
    return(x) -> x
  }
  if v == 1 { io.println("evaluator PERFORMED the op") } else { io.println("evaluator CALLED the local") }
}
```

- `elya check` → `ok`, exit 0
- `elya run` → `evaluator PERFORMED the op`, exit 0

## Slice close-out doc fixes

- Spec §6.3 says both existing guards "must be extended" to cover the frame
  tag; b370191 left both unchanged and added a separate frame guard of the same
  shape. (A8's own row was rewritten by plan D10 on 2026-10-03; §6.3's sentence
  still says "extended".)

## Op → effect index: one helper, one copy left

Plan D11 adds `ast::op_effects` for lowering: the third construction of the op →
effect index, after `Infer.ops` (richer: it carries signatures, like
`effect_multi` it stays separate) and the evaluator's `op_table`. Redirecting
`op_table` to the helper touches the reference semantics, so it was not done in
5b-8; do it, gated, in a later slice.

## HIGH: shadowed heap bindings are not rooted by the direct emitter (pre-existing, found 2026-10-03)

`lower_expr`'s `Let` saves a shadowed binding in a Rust local (`prev`) and roots only the
`env` view, so an outer heap binding hidden by an inner `let` of the same name is NOT on the
shadow stack while the inner scope allocates. A collection frees it; after the inner scope
the outer name reads a reused block. Measured at 831b023+ (the last commit before 7b-3, and
unchanged by 7b-3): prints `33792`, expected `30049` (`7 + 30000 + 42`).

```
type L { Nil, Cons(Int, L) }
fn churn(n, acc) { if n == 0 { acc } else { let g = Cons(n, Cons(n, Cons(n, Nil)))  churn(n - 1, acc + 1) } }
fn head(l) { match l { Nil -> 0  Cons(h, _) -> h } }
pub fn main() -> Int {
  let s = Cons(42, Nil)
  let r = { let s = Cons(7, Nil)  let z = churn(30000, 0)  head(s) + z }
  r + head(s)
}
```

The CPS emitter (7b-3) roots by binding index (`St::binds`), not by the name view, so its own
allocations do not have this hole -- but the direct code it calls into does. Fix in its own
slice: root every live binding (shadowed ones included), with this program as the red test.

## 7b-3 review follow-ups (Task 8 and later)

- Resumption functions are declared and emitted for EVERY call site, including sites inside
  clause bodies, which 7b-3 never emits. A site whose path runs through `resume(..)` would
  stop the build with "resume (Task 8)". Every such program the reviewer could build is
  refused earlier today; Task 8 must emit clause bodies and revisit this.
- D16's refusal is conservative: `fn ap(f) { f(1) + get() }` is refused at every use that
  names a user effect, even with a pure lambda (its generic signature has an open row). Lift
  with N7.

## Evaluator: non-tail recursion under a handler is quadratic (found 2026-10-03)

`deep(n)` (non-tail recursion that keeps a `Cons` per level) under a `handle` takes 0.12 s at
n = 1000, 0.43 s at 2000, 1.70 s at 4000 (debug `elya run`): ~4x per doubling, and n = 100 000
did not finish in 10 minutes. The 7b-3 deep-frames test uses n = 4000 because of this.
