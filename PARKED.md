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
