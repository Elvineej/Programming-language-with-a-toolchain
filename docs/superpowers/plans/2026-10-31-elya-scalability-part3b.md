# Plan: compiler scalability, part 3b (a block's binding is a root until its last reader)

Spec: `docs/superpowers/specs/2026-10-31-elya-scalability-design.md` (D13).
Base: `main` after part 3a (dfe9c10).

## Steps

1. Measure (debug, `emit_ir` apart), at 250 / 500 / 1000 statements: a block
   of strings, each made from the one before, 126K / 503K / 2.0M IR lines; the
   same under one name, the same; in an effectful function (the CPS emitter),
   128K / 506K / 2.0M; with a site every 100 statements, 49K / 108K / 216K;
   every string live to the end (a control: liveness cannot shorten it), 317K
   / 1.26M / 5.0M. Every allocation rooted every pointer in scope.
2. A stress mode in `runtime.c` (outside `gc_mark`): `ELYA_GC_STRESS=N`
   collects at every Nth allocation; what it frees it poisons and keeps,
   never handed out again; a block it freed, met while marking (a root, the
   current handler, a field), stops the program. The new stress test's programs pass under it on part 3a as
   it stands (its rooting complete there).
3. `Deaths` (lib.rs): a block's bindings and the statement each is read last
   by, off `free_vars`; `lower_block` and the CPS emitter's two block loops
   retire a binding once that statement is bound (`Env::retire`; the CPS
   emitter also drops it from the bindings `root_live` pushes).
   `bind_local` and `unbind_local` keep a retired binding retired, parked or
   restored (found writing the tests: a dead outer binding shadowed by an
   inner block was rooted again, its value perhaps freed).
4. Tests:
   - `crates/codegen/tests/native_scalability.rs` (+4): strings down a block,
     under one name, unread under one name (retired at once, then parked),
     in an effectful function; IR lines and probes at n and 2n, linear.
   - `crates/codegen/tests/native_gc_stress.rs` (new, 1): twelve programs
     run natively under `ELYA_GC_STRESS=1`, each answering as the evaluator
     does: a binding read after allocations and by the end, one only the end
     reads, a closure's capture, shadowing (a live outer name, a dead one, an
     inner block), a string across a perform and one dead before it, a
     multi-shot resume, a block in a match arm, a string grown through a tail
     call, a dead large string shadowed by an inner block or followed by a
     site, and a block in value position in an effectful function, run
     straight and through resumptions (the review's).
   - Unit tests in lib.rs (+2): the deaths of a small block; parking keeps a
     binding retired or rooted.
5. Controls (each reverted, compared byte for byte):
   - K15, the block's end not counted as a reader: the stress test fails
     (`match failed`: the list was freed);
   - K16, K15 with nothing poisoned: still fails;
   - K17, `lower_block` retiring nothing: three scalability tests fail (3.98);
   - K18, the CPS loops retiring nothing: the effectful one fails (3.96);
   - K19, parking rooting a retired binding again: the unit test and the
     stress test fail ("a reachable value names a freed block");
   - K20, retiring one statement early: the stress test fails (the same);
   - K21, the CPS emitter keeping retired bindings for `root_live`: the
     stress test fails (the same);
   - K28, the CPS value-position loop alone keeping them (the review's M18b):
     the stress test fails (the same), on the review's program;
   - K29, that loop retiring every death by the binding just made (the
     review's M18a, a wrong index): the stress test fails.
6. The whole codegen suite under `ELYA_GC_STRESS=1` with the change, a
   watchdog stopping any program past 60 s (collecting at every allocation
   makes a long run quadratic): 23 of 321 tests fail, 20 because a program
   was stopped, 2 because they count collections, 1 because it times itself.
   None answers wrongly; no root names a freed block (the check then read
   the roots only; it now runs on everything grayed).
7. An independent review: nothing unsound (1205 sweep programs, 1200
   generated and 26 adversarial ones, base and change alike under stress;
   600 modules' IR, the change's root calls a subsequence of base's). From
   it: `cargo fmt`; the CPS value-position loop tested (K28, K29); the stress
   check on everything grayed, not the roots alone (a freed block marked once
   was missed); the comments on shadowed bindings; one figure corrected (every
   string live is unchanged, as a control should be); the cost of reading a
   resumption's deaths again recorded in the spec.

## Predictions

- Gate: 1436 + 4 + 1 + 2 = **1443 passed, 155 suites**.
- `gc_mark` unchanged (637 bytes); the runtime changes only under
  `ELYA_GC_STRESS`.
- At 2000 / 4000 / 8000 statements (debug): strings 18K / 36K / 72K IR lines,
  emit 0.28 / 0.41 / 0.95 s (17.6 s and 8.0M lines at 2000 before); in an
  effectful function 26K / 52K / 104K lines; every string live, the same as
  before (n^2).
