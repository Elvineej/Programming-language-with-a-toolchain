# Plan: a block's length is not a stack bound

Spec `docs/superpowers/specs/2026-10-30-elya-long-blocks-design.md`. Gate before: 1315
passed, 136 suites (fa24c18).

1. Red tests first.
   - `tests/long_blocks.rs`, one test: a 100 000-statement `main` checked, lowered,
     dropped and run on a 1 MiB thread. Predicted red: the drop overflows (an abort,
     "has overflowed its stack").
   - `crates/codegen/tests/native_long_blocks.rs`, two tests on 8 MiB threads: the
     20 000-statement `main`, and the long blocks in every place (D4). Predicted red:
     an abort in `anf` or `mono` (the first pass), at a few thousand statements.
2. D2: `impl Drop for CoreExpr`. Code that moves out of a `CoreExpr` will not compile
   past it; each such site takes the field by `clone` or `std::mem::take` instead.
3. D1, pass by pass in pipeline order, each step re-running the native test under
   `gdb` to find the next pass that overflows: `anf`, `mono`, `specialize`, `any`,
   `instantiate_types`, `patterns`, `spill`, the collectors, the emitters. A shared
   helper in `src/core.rs` reads a spine (`let_spine`) and rebuilds one.
4. Controls:
   - K1, `Drop` back to derived: the root test aborts;
   - K2, one pass's spine loop back to recursion (`mono::rewrite_let`): the native
     tests abort, in that pass;
   - K3, the dependency chain broken in the 20 000-statement test's expected value:
     it fails on the value, not the stack (the test reads the value).
5. D3: measure the nesting bound (a call nested N deep; a block in a block N deep)
   for `check` and `build`, on the CLI, and record it in the spec.

Predicted gate: 1315 + 2 (the root test, twice) + 2 (native) = **1319 passed, 139
suites** (the root suite twice, the native one once).

Measured: red as predicted (the root test in the drop; the native tests in `anf`).
The red root test also found the evaluator's `Scope` chain (spec §2), fixed the same
way; its control is K1b.

The review's fixes (spec §3): a row added to an existing table, the native tests
reworked in place (no new test), so the prediction stands: **1319 passed, 139
suites**.
