# Async step 2, part 1: structured concurrency

**Status:** done (2026-10-15). HANDOFF step 1 (ROADMAP 1c). Design choices are Claude's
under the maintainer's delegation (rule 2), recorded here. Part 2 -- real I/O readiness
through a runtime poll loop (ROADMAP 1b) -- is its own step: it needs runtime builtins
and a decision on what an I/O source is, neither of which this part touches.

## 0. Measured first

The natural program (`examples/06_structured.elya`'s draft, a scheduler over `spawn`,
`await`, `yld` and a joining `scope`) at `d173257`: E0423 at every
`spawn(fn() { fib(n - 1) })` -- the lambda closed `fib`'s row before `spawn` reached it.
Fixed first as its own change (recursion through a lambda, spec
`2026-10-15-elya-handle-recursion-design.md` §5). After it: the evaluator prints the
expected interleaving and 25555; natively the same bytes and value, with no back-end
change (N7 part 2 made the generic `List`/`Pair` queue native the same day).

## 1. What "structured" means here

- **A scope joins its children.** `scope(body)` runs `body` as task 0 under a round-robin
  scheduler and returns only when the queue is empty: every task started in the scope --
  by the body or by another task in it -- has finished. A child cannot outlive its scope,
  so from the outside a scope is an ordinary call (`pfib` opens one per level).
- **Handles, not promises.** `spawn` answers a task id; `await(id)` suspends until that
  task's answer is in the scope's result table. Ids are local to a scope.
- **Suspension is still an effect.** `fib` performs `Async` and nothing else marks it; the
  same `worker` would run under any other `Async` handler.

## 2. Choices

- **The scheduler stays a library-level handler (chosen)**, not a runtime primitive:
  it is 40 lines of Elya, it is what ROADMAP 1 promises (a handler gives suspension its
  meaning), and every part of it -- escaped resumes stored in a queue, generic lists, deep
  recursion through handles and lambdas -- is now exercised natively by one program.
- **Waiting is polling the queue (chosen for now).** An `Awaiting` task whose child has
  not finished goes back to the end of the queue. That is O(tasks) per wake-up, fine for
  an example; a wait list keyed by id is the scheduler's own business and changes no
  language rule. Deadlock (awaiting an id that never finishes) loops forever, as an
  unstructured `await` on a lost future would; detecting it (a full pass of only
  `Awaiting` tasks) is a scheduler improvement left for part 2, where blocking on I/O
  makes "nothing can run" a real state.
- **No cancellation yet.** A structured scope would cancel its children when the body
  fails; Elya has no failure effect in the example set yet. Recorded for part 2.

## 3. Evidence

- `tests/examples.rs::structured_concurrency` (snapshot), `crosscheck`'s effectful list,
  `native_codegen.rs::the_structured_concurrency_example_runs_natively` (same bytes, 25555).
- Plan: `docs/superpowers/plans/2026-10-15-elya-async-step2-structured.md`.
