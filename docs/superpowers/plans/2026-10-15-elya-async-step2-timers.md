# Plan: async step 2, part 2 (timers, a poll loop)

Spec: `docs/superpowers/specs/2026-10-15-elya-async-step2-timers-design.md`. Baseline:
934 passed, 89 suites.

1. **Builtins in the front end and the evaluator.** Red `tests/timers.rs`, 3 tests: the
   types and `IO`, the clock advancing, and E0208. Then `resolve`, `infer_call` (both
   perform `IO`), and both evaluator paths. Predicted gate: 934 + 3 × 2 = **940**,
   **91** suites.
2. **Natively.** Add the runtime functions `elya_time_now_ms` and `elya_time_sleep_ms`,
   and builtin arms in the direct emitter and both CPS paths. Neither allocates. Red
   first: one native test that the clock advances. Predicted: **941**.
3. **The example.**
   - `examples/08_timers.elya`, plus `tests/examples.rs` (a snapshot, +1 root test ×2)
     and a native same-bytes test (+1).
   - Check whether `crosscheck` runs it. Predicted: **944**, **91** suites.
   - Controls:
     - T1: the virtual clock never advances (`wait_until` ignored). The snapshot
       changes: answers come out in request order.
     - T2: deadlock detection off. The example never finishes; that is measured with a
       timeout, not committed.
     - T3: `sleep_ms` returns at once. The clock test fails.
4. **Review**, then HANDOFF and the log.
