# Async as a standard module: `elya/async`

**Status:** part 1 (the module) in progress (2026-10-31, on `main`, an interactive
session); part 2 rewrites the three examples on it (D5). HANDOFF step 1.
The design choices are Claude's under the maintainer's delegation (rule 2).

## 0. Measured first

The three async examples each carry their own scheduler, and each task type named
the program's other effects (`fn() / {Async, Log} -> Int`). Since row parameters
(499c048) a task's type can say "the other effects" (`{Async(e) | e}`). Measured with
the debug CLI at 5e3bb5c, evaluator and native binary:

| program | result |
|---|---|
| `examples/06_structured.elya` with `Async(e)`, `Step(e)`, `step`/`scope` at `{Async(e) \| e}` / `{e}` | output byte-identical to 06, both back ends |
| the same, `pfib` still declared `/ {Log}` | E0423: `pfib` never performs `Log` (the fixed task type forced it) |
| `effect Async(a, e)`: `spawn(f: fn() / {Async \| e} -> a) -> Int`, `await(id) -> a` | one scope's tasks return `Int`, another's `String`; both back ends |
| the scheduler as a user module, `import lib/async` | no grant needed (`scope`'s row is `e` alone); `async.scope(..)` qualified, operations unqualified; both back ends |
| 06's scheduler, two tasks awaiting each other | never ends (killed after 10 s) |
| `examples/08_timers.elya` made generic (`Step(a, e)`, `Timer(a, e)`, `Ending(a)`, `Sched(a, e)`) | output byte-identical to 08 (virtual clock, deadlock, real clock), both back ends |

- 04 forks tasks that return `Unit`; 06 adds `await` and a joining `scope`; 08 adds
  `sleep`, timers sorted by deadline, a `Clock { now, wait_until }` with a virtual
  and a real handler, and a deadlock rule (every queued task awaits one that
  cannot finish, and no timer is due).
- Every name a standard module declares carries the module's (capabilities part 3
  D1): types, effects and operations are one name space per program, and `spawn`,
  `yld`, `Clock` are names the examples themselves declare.

## 1. Decisions

- **D1. One module, `elya/async`, written in Elya: 08's scheduler made generic.**
  `effect Async(a, e) { fn async_spawn(f: fn() / {Async | e} -> a) -> Int;
  fn async_await(id: Int) -> a; fn async_yield() -> Unit; fn async_sleep(ms: Int) -> Unit }`.
  A scope's tasks share one result type `a`; a task is named by its `Int` id, as
  in 06 and 08. `e` is whatever else the tasks perform, handled outside the scope.
  An id names a task of the scope that spawned it: awaited inside a nested scope
  with the same `a`, the inner scope answers for its own task of that number
  (the review's R4; with another `a` the types refuse it, E0423). Typed handles
  would close this; they need an abstract type.
  - Rejected: a typed handle `AsyncTask(a)` (it needs an abstract type, which
    the language does not have yet); a separate `AsyncTimer` effect for `sleep`
    (a sleeping task would change type, and the scheduler must handle both).
- **D2. A scope answers how it ended:** `type AsyncEnded(a) { AsyncFinished(a),
  AsyncDeadlocked(AsyncList(Int)) }`. `AsyncFinished(v)` once every task started in
  it has finished, `v` the body's value; `AsyncDeadlocked(ids)` when every task
  left awaits one that can never finish and no timer is due (08's rule). A scope
  never loops without end on a deadlock (06's did, measured).
  - Rejected: answering a default value (06's `-1`): a deadlock would read as a
    result. A failure effect (spec 2026-10-16): the module would force a `Fail`
    on every caller.
- **D3. Time: deterministic by default; a clock is two functions.** The one
  scheduler, `async.schedule(body, now_of, wait_for)`, takes its clock as what the
  time is (`now_of(t)`, given the time it keeps) and the time after waiting for a
  deadline (`wait_for(t, d)`). `async.scope(body)` is it on a virtual clock: time
  moves only when every task sleeps, to the earliest deadline, so a scope is
  reproducible and pure apart from `e`; a task that waits by polling
  (`async_yield` in a loop until a sleeper acts) never lets that time move, and
  loops (the review's R5: on the machine's clock it ends). `elya/async_host`'s
  `async_host.timed(body)` is the scheduler on the machine's clock (`time.now_ms`,
  `time.sleep_ms`; its interface performs `IO`, part 3 D3): a module of its own,
  not part of `elya/host` (the review's R2).
  - *Changed after measuring the first draft* (an `AsyncClock` effect, handled
    inside `scope` by a virtual clock, performed by an `async.timed`): the clock
    effect reached the tasks' row (a call's tail is equated with the ambient it
    is poured into, so `e`, which is also `scope`'s row, held `AsyncClock`: E0420
    at `main`), and a module whose interface performs it needed
    `import elya/async / {AsyncClock}` at every import (E0606).
- **D4. The module's own types** are `AsyncList`, `AsyncPair`, `AsyncOption`,
  `AsyncTable`, `AsyncStep`, `AsyncTimer`, `AsyncReady`, `AsyncQueue`, `AsyncEntry`,
  `AsyncSched`, `AsyncEnded` (named by D1's rule); only
  `AsyncEnded` and `AsyncList` reach a caller.
- **D5. Proof:** `examples/04_async.elya`, `06_structured.elya` and
  `08_timers.elya` rewritten on `elya/async`, their outputs unchanged except where a
  decision says (06 and 08 report a deadlock by D2), in the evaluator and
  natively. `elya/async` itself is imported without a grant (a scope's row is its
  tasks' other effects alone); a dependency that spawns tasks performs `Async`, so
  spawning is a capability its importer grants (`import lib/dep / {Async}`;
  E0606 otherwise: corrected from the first draft's claim).

## 2. Evidence (part 1: the module)

- `src/std/async.elya` (08's scheduler made generic, its clock as two functions),
  `elya/host`'s `host.timed`.
- `tests/std_async.rs` (7, red first: E0600 "no module `elya/async`"): joining
  and the body's value; two result types and the tasks' `Log` outside; a
  deadlock by name (tasks 0, 1, 2); sleepers in deadline order; a clock given as
  functions (it says when it waits); the host's clock really sleeps; spawning as
  a granted capability (E0606 without the grant).
- `crates/codegen/tests/native_std_async.rs` (3): joining, the deadlock, the
  sleepers on both clocks; each binary prints what the evaluator prints.
- Controls K1-K3 (the test file's header): each failed distinctly.
- **Review (an independent agent), each finding fixed test-first or written
  down:**
  - R1, high: a scope written inline in a task that also performs the outer
    `Async` is refused natively by name ("calling convention disagrees with the
    callee"; the evaluator answers 4). Older than this module (a 14-line program
    without it fails the same way): HANDOFF's native-limits step. Pinned:
    `a_nested_scope_inline_is_refused_by_name_natively`.
  - R2, medium: `elya/host` imported `elya/async` for `host.timed`, so every
    program importing the host's handlers lost the names `Async`, `AsyncPair`,
    `async_yield`..., and the examples (each declares its own `Async`) could not.
    `timed` is now `elya/async_host`'s. Test `the_host_module_reserves_no_async_name`;
    control K4.
  - R3, medium: a task waiting in `async_await` was polled every round (an
    append over the queue each time) and the task table was a list: fib(15),
    about 2 000 tasks, took 236 s in the evaluator. Now a waiting task is parked
    on the entry of the task it waits for and woken when it answers, the queue is
    two lists, and the table a Braun tree by id:

    | program | first draft | now |
    |---|---|---|
    | fib(14), evaluator (debug) | over 60 s (30 s with parking alone) | 1.7 s |
    | fib(18), native | 27 s | 0.12 s |

    The deadlock rule became "no task ready, no timer, some task unanswered"
    (no counter). Test `many_tasks_are_scheduled_without_polling_the_waiting`.
  - R4, medium: an id across scopes (above, D1): written down.
  - R5, low: polling on the virtual clock (above, D3): written down.
  - R6, low: on the machine's clock a sleep's deadline was read when the
    scheduler next looked at the task, not when it slept (a sibling computing
    for 100 ms delayed a 50 ms sleep to 150 ms). Now it is read when the task
    sleeps. No test: it shows only on a clock that moves while tasks compute,
    and a real-time test flakes under load (`08_timers`, HANDOFF).
