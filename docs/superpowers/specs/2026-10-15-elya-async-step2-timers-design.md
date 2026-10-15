# Async step 2, part 2: waiting for time, and a poll loop

**Status:** in progress (2026-10-15). HANDOFF step 1 (ROADMAP 1b), on top of
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
