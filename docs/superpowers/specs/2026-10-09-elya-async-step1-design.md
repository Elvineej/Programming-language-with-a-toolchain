# Async step 1: function types in declarations, and a scheduler written as a handler

**Status:** done (2026-10-09). HANDOFF step 1 ("async as an effect", ROADMAP priority 1).
Design choices are Claude's under the maintainer's delegation (rule 2).

## 0. Measured first

At `5d4b14e` (`auto/elya`, after N7 part 1):

| Program | `check` | evaluator | native |
|---|---|---|---|
| a scheduler storing a task's rest in a constructor, `Paused(fn() -> Task)` | E0432 "a function type in a type or effect declaration is not supported yet" | -- | -- |
| `effect Async { fn fork(f: fn() -> Unit) -> Unit }` | E0432 (same) | -- | -- |
| a queue of closures through a parametric `Q(a)` | ok | 1 | refused: parametric ADT |
| `fork` as a multi-shot op returning twice (Unix-style) | ok | -- | refused: multi-shot |

A scheduler needs to KEEP suspended computations somewhere. The only place that is
natively supported today is a non-parametric constructor field, and fields could not be
function-typed. So the first step is a front-end one; once it was in, every scheduler
probe below ran natively without a back-end change (N7 part 1 had already made the
generic `each` and the upcasts work).

## 1. Function types in declarations

- **Where.** A constructor field, an operation parameter and an operation result may be
  `fn(A, ..) / {E, ..} -> R`.
- **Rows (the fork).** (a) An unwritten row means "any effects", as in a function's own
  annotations, which needs a row variable in the type declaration (a row parameter of
  `Task`, or an existential). (b) **An unwritten row is EMPTY (chosen).** A declaration has
  no row variables, so its rows are exactly what is written: `fn() -> Task` is pure,
  `fn() / {Log} -> Task` performs `Log`. (b) is the stricter option, needs no new kind of
  type parameter, and sub-effecting already lets a smaller closure into a field that
  admits more. Row parameters on types (`type Task(e)`) can come later without changing
  the meaning of anything written today.
- **Which effects.** `IO` and any declared effect, including one declared later in the
  file or the one being declared (`fork(f: fn() / {Async} -> Unit)` inside `Async`). An
  effect with type parameters cannot be written in a declaration row yet (its arguments
  would need syntax): E0432 says so by name. An unknown effect is E0432 "unknown effect".
- **Core.** `core::ann_to_ty` built `Ty::Con("fn", ..)` for such a field (harmless only
  while fields could not be functions); it now builds the same `Ty::Fn` inference does.
- **Native.** A closure in a field is a traced pointer, as before. One soundness gap
  opened, found by a probe and closed before the commit: a PATTERN binder can now hold a
  closure, and the upcast adapter and its guard tracked only `let`, parameter, clause and
  return binders. `match b { P(g) -> if c { g } else { fn(x) { lg(x) } } }` printed 4 where
  the evaluator printed 304. Pattern binders are now tracked with their field types
  (`specialize::pat_binder_types`; a parametric type's fields stay untracked, and are
  refused natively anyway).

## 2. The scheduler (`examples/04_async.elya`)

- `effect Async { fn fork(f: fn() / {Async, Log} -> Unit) -> Unit  fn yld() -> Unit }`.
- `task(body)` is an ordinary handler that turns a computation into a `Task`: `Done`,
  `Paused(rest)` at a `yld`, `Forked(child_body, rest)` at a `fork`. `rest` is
  `fn() { resume(Unit) }`: the continuation, stored in a field and resumed later, after
  the handler has returned (one-shot: each rest runs once).
- `run(queue)` is the round-robin loop, plain Elya over a `Queue` ADT.
- `each(f, n)` is one function, used in ordinary code with a pure argument and inside the
  workers with an argument that yields: no function colouring. Natively it is the
  original (pure use) plus one convention clone (N7 part 1).
- **Why `Forked` carries the child's BODY, not its task.** `fork(f) -> Forked(fn() {
  task(f) }, ..)` (the clause re-entering `task`) is E0423: recursion through a handle
  body is the parked inference gap (PARKED, "recursion through a handle body"). Handing the
  body to the scheduler, which calls `task` itself, is equivalent and is what a real
  runtime does anyway (spawn goes through the scheduler).

Output, the same bytes from the evaluator and natively, then 3 (tasks finished):

```
main: start
  ping
main
    pong
  ping
main
    pong
  ping
main: done
```

## 3. Evidence

- Gate: 816 passed, 79 suites (predicted 816: 8 new root tests twice, the example twice,
  2 native).
- `tests/fn_fields.rs` (8 rules), `FN_FIELDS` (8 programs, value and differential), the
  example natively (bytes and value) and in the evaluator (snapshot).
- The tree-walker oracle has no effects, so `crosscheck` now names its effectful examples
  (`EFFECTFUL_EXAMPLES`) and asserts the tree-walker REFUSES each, so the list cannot hide
  a regression; they are checked against the native backend instead.
- Negative controls, each reverted (`cmp`):
  - C2, an unwritten declaration row open: `an_unwritten_row_in_a_declaration_is_pure`
    checks clean, and the written-row test fails too (the written row was dropped);
  - C3, the declared-effect set not filled: the three rows naming effects fail (E0432);
  - C4, pattern binders untracked again: `an-upcast-of-a-pattern-binder` prints 4, not
    304.
- Expectation changed: `annotations.rs` `a_function_type_in_a_declaration_is_named_not_unknown`
  -> `a_function_type_in_a_declaration_checks` (the feature exists now).

## 4. The independent review

About 100 probes (sub-effecting into fields, one-shot violations through stored resumes,
GC stress with thousands of tasks and closure chains, refusals): no regression, no native
miscompile, no GC failure. One finding, PRE-EXISTING and parked (the resume-row gaps),
which this slice makes easy to reach: in `task(body)` with `body` UNANNOTATED (or with an
open row), a body effect relayed through `body`'s row variable is not carried by `resume`
(only its labels are added at the resume site, not its tail), and neither are the return
clause's effects. So `fn() { resume(Unit) }` is typed pure, fits `Paused(fn() -> Task)`,
and is called after the outer handler has returned: `check` is clean, the evaluator stops
on an unhandled effect, natively the program is refused by name or (with an unrelated
effect in the field's row) reaches the runtime's "no clause" guard. `examples/04_async.elya`
is sound because `body`'s row is written (closed). This is the next step (HANDOFF 1).

## 5. Next (async)

- Recursion through a handle body (PARKED) blocks the natural recursive `task`.
- Real I/O readiness: a runtime poll loop the scheduler handler consults (ROADMAP 1b).
- Structured concurrency: a scope handler that joins its children (ROADMAP 1c).
- Effect rows as type parameters (`type Task(e)`), so a task type can be generic in what
  its tasks perform.
