# Plan: elya/async, part 1 (the module)

Spec: `docs/superpowers/specs/2026-10-31-elya-async-module-design.md`. HANDOFF step 1.

1. Red: `tests/std_async.rs`, 7 tests, all E0600 "no module `elya/async`" today.
2. `src/std/async.elya`: 08's scheduler made generic (`Async(a, e)`, `AsyncStep(a, e)`,
   timers, the deadlock rule); `scope`, `schedule`; `src/modules.rs` registers it;
   `src/std/host.elya` gains `timed`.
3. Natively: `crates/codegen/tests/native_std_async.rs`, 3 tests (written once the
   evaluator's are green, as for row parameters).
4. Controls K1-K3 (deadlock rule, timer order, the host's sleep).

Predictions: gate 1385 + 2 x 7 + 3 = **1402 passed, 151 suites** (148 + 2 + 1);
measured 1402, 151. After the review: 2 root tests and 1 native more,
**1407 passed, 151 suites**.
No test expectation changes. `gc_mark` unchanged.

Part 2: `examples/04_async.elya`, `06_structured.elya`, `08_timers.elya` on the
module (their snapshots; `tests/timers.rs`, `native_trace.rs`, `native_codegen.rs`
and `crosscheck.rs` read them).
