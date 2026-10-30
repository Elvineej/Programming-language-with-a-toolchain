# The occurs checks look inside effect rows

**Status:** in progress (2026-10-30, branch `auto/elya`, an interactive session).
Found while measuring HANDOFF step 1 (row parameters on declarations); fixed first,
because that step leans on these checks. The design choices are Claude's under the
maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at 6b443af)

Three-line programs crash the checker (`elya check`: "thread 'elya' has overflowed
its stack", exit 134) instead of reporting an error:

| program | what cycles |
|---|---|
| `effect E(t) { fn op(x: t) -> Unit }` / `fn f() { op(f) }` | `f`'s row contains `E(type of f)`: a row variable inside its own label's argument |
| the same effect / `fn g(k) { op(k)  k() }` | `k`'s type `fn() / {E(type of k) ..} -> ..`: a type variable inside a row of its own type |
| `effect Async(t) { fn spawn(f: t) -> Int }` / a scheduler `fn sched(body) { handle { body() } with { Async.spawn(f) -> resume(sched(f)) } }` / a task that spawns | the task type `t = fn() / {Async(t)} -> Int` |

`gdb`: `Infer::resolve` and `resolve_row` recursing without end. Both occurs checks
miss the case:
- `Infer::occurs` (E0401) walks a function type's parameters and result but not
  its row (`Ty::Fn(ps, _row, r)`), so `t := fn() / {E(t)} -> Int` is bound;
- `Infer::bind_row` (E0424) rejects only `v := {.. | v}`, not `v` inside a label's
  arguments.

Each cycle is a type that contains itself, which Elya does not have (no recursive
types except through a declared constructor), so the right answer is the existing
named error. (For `fn g(k) { op(k)  k() }` the cycle comes from the relay rule:
calling a parameter unifies its row with the caller's, so `g`'s row would hold
`E(type of k)` with `k`'s row being `g`'s. With inclusion instead of unification
the type would be finite.)

## 1. Decisions

- **D1. A type variable occurs in a function type's row too.** `occurs` walks the
  row's labels' arguments: `t := fn() / {E(t)} -> Int` is E0401 "infinite type".
- **D2. A row variable occurs in its row's labels' arguments too.** `bind_row`
  rejects `v := {E(fn() / {.. | v} -> ..)}` as E0424 "cyclic effect row".
- **D3. A poisoned row is settled.** Once a cycle is reported, the row is `ErrorRow`
  (it absorbs every label), and the inclusion fixpoint (`flush_inclusions`) skips
  it: adding a label to it "succeeded" every pass, so the loop never ended (the
  review's F1).
- *Rejected:* equi-recursive types (accepting the cycle). The async scheduler that
  needs one is HANDOFF step 1's job, with a row parameter instead: its task type is
  then finite (`Async(e)` with `e` the other effects, not the task).

## 2. Evidence

- `tests/occurs_in_rows.rs`: each measured program is a named error, checked on an
  8 MiB thread (a regression aborts the test binary); and the same effect without
  a cycle still runs.

Measured (2026-10-30):
- Red first as planned: the test binary aborted (stack overflow).
- After: `fn f() { op(f) }` and `fn g(k) { op(k)  k() }` are E0424 (`k`'s row and
  `g`'s are one, so the row check sees the cycle first); the scheduler is E0401.
- No other root test changed (503 tests, 60 suites).
- Controls: K1 brought back only the scheduler's overflow, K2 only the two row
  cycles': each check covers its own case.
- **Review (an independent agent), each finding fixed test-first:**
  - F1, high: the occurs checks turned some overflows into a hang with growing
    memory. A task that respawns itself (`fn worker() { spawn(fn() { worker() }) }`)
    poisons a row inside `flush_inclusions`, whose next passes "added" labels to
    the poisoned row forever. D3; test `a_cycle_found_in_the_inclusion_fixpoint_ends_it`
    (four programs, each checked under a 60-second limit); control K3.
  - F3, medium: `occurs` and `mentions_row` resolved again at every level: a
    hundred nested performs took 43.8 s (8.4 before). Each now resolves once and
    walks the result: 9.7 s. The growth that remains is older (8.4 s before the
    fix; 0.09 s at 25 deep, 0.68 at 50), and is HANDOFF's scalability step's.
  - F5, F6: the relayed case's wording (above, and in the test); a direct D1
    case (`fn g(k) { k(fn() { op(k) }) }`, E0401); the no-cycle control with an
    open, relayed row in a label's argument (`fn w(k) { spawn(fn() { k() }) }`).
  - No other path to a cyclic substitution was found (every binding stores a
    resolved term), and no program that ran before is refused.
