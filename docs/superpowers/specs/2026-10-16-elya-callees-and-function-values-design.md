# Computed callees and functions as values, natively

**Status:** done (2026-10-16). HANDOFF step 1. Design choices are Claude's under
the maintainer's delegation (rule 2), recorded here.

## 0. Measured first

At `80183ed`, natively (the evaluator runs each):

| Program | evaluator | native |
|---|---|---|
| c1 `(compose(f, g))(4) + (mk())(1)` | 52 | refused: computed callee |
| c3 `let g = double  apply(g, 4) + g(1)` | 10 | refused: function used as a value |
| c4 `(pick(1))(arg(2))`, both print | `callee`, `arg`, 3 | refused: computed callee |
| c5 `B(inc)`, a field holding a function | 42 | refused: function used as a value |
| c6 `twice(logd, 5)`, `logd` performs `L` | 23 | refused (same) |
| c7 `apply(id, True)`, `apply(id, 7)`: a generic function as a value | 7 | refused (same) |

c4 pins the evaluator's order: the callee is evaluated before the arguments.

## 1. Choice

- **Where:** a Core-to-Core pass in the back end (`crates/codegen/src/anf.rs`), run first
  in `build_module`, rather than in Core lowering: Core stays the faithful image of the
  source (its printer and snapshots unchanged), and the evaluator never sees the pass.
- **Computed callee -> a named one.** `App(e, args)` with a non-variable callee becomes
  `let $callee.k = e in $callee.k(args)`: evaluated first, as in the evaluator (c4), and
  then an ordinary local-closure call. (The parallel `claude/async` branch did the same
  in lowering and matched the evaluator.)
- **Function as a value -> a closure.** A top-level function named outside callee
  position, not shadowed by a local, becomes `fn($eta.k.0, ..) { f($eta.k.0, ..) }` at the
  variable's own type: a closure with no captures, whose inner call is an ordinary
  reference. So N7 specializes it -- a generic `id` used as a value at `fn(Bool) -> Bool`
  gets its clone, an effectful `logd` gets its convention -- with no new emitter path and
  `gc_mark` untouched (closures are an existing object kind). One closure allocation per
  use; a shared static closure per function is a later optimization.
- Alternative rejected: a static closure object per top-level function emitted by the
  back end. It would need one per instantiation anyway (after N7), a new kind of global
  the collector must not free, and a path for the convention -- everything the eta
  wrapper gets from passes that already exist.

## 2. Evidence

- `native_codegen.rs` `CALLEES` (value and differential): c1, c4 (order: 321), c3, c5,
  c6, c7, c8 (a shadowing local, the guard) and the review's r1-r3. Red at `80183ed`
  (refused by name) except c8.
- Changed expectations (`lib.rs`): the two refusal tests now assert compilation.
- Gate: 1012 passed, 95 suites (predicted 1012; the review added corpus rows only).
- Negative controls: the pass off -- every row but c8 is refused by name again;
  `inner_type` returning the use's type -- r1-r3 refused, "calling convention disagrees
  with the callee".

## 3. The independent review

About 60 probes (order, nesting, zero parameters, generic and effectful values, multi-shot,
shadowing by every binder, budgets, 1M-iteration tail loops of closure calls, 300x2000
closures in lists). Fixed, red first:

- **F1 (medium, a refusal by a misleading name).** A pure function passed where an
  effectful one is expected (`apply(inc, 0)` under a handler): the wrapper's inner call
  copied the use's WIDENED type, so the direct `inc` was called with the CPS convention.
  The inner call now has the function's own row when that row is closed (an open own
  row is a relay, which the use's type already instantiates; part 1 clones from it --
  narrowing it there broke a relaying `ap` used through `let a = ap`, caught by
  re-running the review's probes).
- **F2 (low).** `inc == inc` is refused natively (Eq on functions); the evaluator answers
  true. Each use allocates its own closure, so a pointer equality would differ: function
  equality belongs to HANDOFF's structural-equality step, which should make it a
  check-time error.
