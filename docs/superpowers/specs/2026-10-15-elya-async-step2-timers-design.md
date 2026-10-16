# Async step 2, part 2: waiting for time, and a poll loop

**Status:** done (2026-10-15 to 2026-10-16). HANDOFF step 1 (ROADMAP 1b), on top of
`examples/06_structured.elya`. The design choices are Claude's under the maintainer's
delegation (rule 2), recorded here.

## 0. Measured first (at cf8e8f9)

**The draft scheduler** is the natural program: `examples/08_timers.elya`'s draft, which
is 06's scheduler plus `sleep(ms)`, a timer list, a `Clock` effect and deadlock
detection. It checks, runs in the evaluator and runs natively with the same bytes. No
compiler change was needed.
- **Its output:** three fetches of 30, 10 and 20 ms answer in deadline order, and two
  tasks awaiting each other are reported by name.
- **What made the native run possible:** strings are native now (cf6b08b), so the
  native binary prints its own times.

**What is missing is time itself.** `time.now_ms()` is E0201: no builtin reads a
clock, and none can wait.

## 1. Decisions

- **D1. The first I/O source is a timer.** A task that sleeps is parked, as are the tasks
  waiting on it. When nothing can run, the scheduler waits until the earliest deadline:
  the "poll" over the one kind of source there is, the select-with-a-timeout shape every
  event loop has.
  - **Deferred:** pipes and sockets. They need a readiness builtin per platform, and
    the Windows gate cannot be run here.
- **D2. The scheduler never reads time; it performs `Clock`** (`now`, `wait_until`).
  Time is a handler:
  - **`virtual_clock`:** moves time only when the scheduler waits. It is exact and
    instant, so the example's output and the tests are deterministic.
  - **`real_clock`:** reads and waits on the machine's clock.

  The same scheduler and tasks run under either, unchanged. This is ROADMAP priority 3
  (handler-based testing) meeting priority 1: a test needs no mock and no sleep.
- **D3. Two builtins, both performing `IO`:**
  - `time.now_ms() -> Int`: monotonic milliseconds since an arbitrary start.
  - `time.sleep_ms(ms: Int) -> Unit`: blocks for at least `ms`; a non-positive `ms`
    returns at once.

  Both exist in the evaluator and natively. The runtime uses `clock_gettime(CLOCK_MONOTONIC)`
  and `nanosleep`, or `GetTickCount64` and `Sleep` on Windows.
  - *Rejected:* a single `time.poll(deadline)`. The scheduler would still need `now` to
    compute deadlines, and two primitive builtins compose better.
- **D4. Deadlock is detected, not looped on.** If no timer is pending and every queued
  task awaits a task that has not finished, the scope ends with `Deadlock(ids)`. The
  example says so and answers -1. Part 1 looped forever here.
- **D5. Cancellation stays with HANDOFF step 4** (a failure effect).

## 2. Evidence (planned)

- `tests/timers.rs`:
  - the builtins' types and their `IO`;
  - `now_ms` is monotonic, and `sleep_ms(20)` advances it by at least 20;
  - E0208 when either is used as a value.
- `examples/08_timers.elya`, with its snapshot. Its real-clock part prints only what
  real time cannot change: the answer order, and whether at least 30 ms elapsed.
- `native_codegen.rs`: the example natively prints the evaluator's bytes, and the
  builtins natively advance the clock by at least the requested time.

## 3. The example (task 3, 2026-10-16)

`examples/08_timers.elya`. The draft named in §0 was not committed, so the example was
rewritten from 06's scheduler. Measured on the way:

- **A native arity refusal.** The first version's `run(q, timers, results, next,
  stalled)` is an effectful function of five parameters: natively "effectful function
  takes more than four parameters" (the win64 `musttail` cap, `MAX_PARAMS`). The
  scheduler's state besides its queue is now one value, `Sched(timers, results, next)`.
  Lifting the cap is not this slice's (it needs the Windows gate); recorded in HANDOFF.
- **Exact rows.** `fetches` and `deadlocked` perform only `Async` themselves (the `Log`
  is inside the spawned lambdas), and `real_clock` is `/ {Log, IO}`.
- **Output.** Virtual clock: b, c, a, and "60 ms of requests in 30 virtual ms". The
  deadlock names tasks 1 0 2. The real clock: b, c, a and "at least 30 real ms". The
  native binary prints the same bytes and answers 600361, the evaluator's value.

**Controls** (each reverted, `cmp` clean):
- **T1** (the virtual clock ignores `wait_until`): the snapshot fails, but on the
  elapsed line ("in 0 virtual ms"), NOT the order. The plan predicted request order;
  that was wrong, because the order comes from the timer list's insertion, not from
  the clock. So a second control was added:
- **T1b** (`insert` ignores deadlines): answers in request order a, b, c.
- **T2** (deadlock detection off): `elya run` never finishes (killed by `timeout 10`,
  exit 124), already in the virtual part. The poll loop itself depends on the stall
  count: a body awaiting sleeping tasks is "stuck" until a timer fires.
- **T3** was task 2's (`sleep_ms` returning at once).

## 4. Review (2026-10-16)

An independent review found no correctness bug, and these risks, fixed test-first:
- **Starvation.** Timers were looked at only when every task was stuck, so a busy queue
  starved a sleeper whose deadline had passed (a 1 ms sleeper woke after ~485 ms
  behind 3000 yields). `run` now polls the earliest timer every turn, and `turn` is the
  round robin. Test: `an_expired_timer_fires_while_other_tasks_run` (tests/timers.rs)
  runs the example's scheduler under a `ticking` clock. Red first, with the predicted
  order reversed ("spinner done" first). The red run is the control.
- **Windows sleeps.** `Sleep` may return early and `GetTickCount64` moves in 10-16 ms
  steps, so `elya_time_sleep_ms` now waits on the same clock until it has moved by `ms`.
  This can't be built here (Windows-gate debt).
- **A timing-dependent value.** The answer used to fold the real elapsed time into
  `r / 1000`. `timed` now answers `Timed(total, ms)`; the clocks are generic in their
  answer. The value is 60361 at any speed.
- **Wording.** The deadlock line names every task that can never finish, not only a
  cycle: "tasks 1 0 2 can never finish".

Left as they are, by decision:
- A deadline is taken when the scheduler dequeues the `Sleeping` step, up to one lap late
  under a real clock. A task cannot read the clock (no `Clock` in its row), and one lap
  is the scheduler's resolution anyway.
- The evaluator's clock origin is set by the first time builtin; native's by the first
  `now_ms`. Both are "an arbitrary start" (D3).
