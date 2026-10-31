# Plan: elya/async, part 2 (the examples on it)

Spec: `docs/superpowers/specs/2026-10-31-elya-async-module-design.md` (D5, section 3).

1. Measure: 06 and 08 rewritten on the module against their snapshots (06 the
   same; 08 one line, the deadlock's ids in ascending order).
2. Install them; 04 stays the written-out scheduler, its header points at the module.
3. Adapt the tests that read their sources: `native_codegen.rs` (06, 08),
   `native_trace.rs` (`build`), `tests/timers.rs`.

Predictions: no new tests; gate **1407 passed, 151 suites** (as at faeec2a). Two
expectations change, both 08's deadlock line: the snapshot and the native test's
literal. (The first gate run found a third: `modules.rs`'s single-file test, its
guard from 10 examples to 8.)

After the review: one root test more (the deadlock's ids sorted), so the gate
prediction is **1409 passed, 151 suites**.
