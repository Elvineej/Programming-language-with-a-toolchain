# Auto-run log

Scheduled runs working on branch `auto/elya` record their state here, newest entry first.
A run whose top entry says IN PROGRESS and started less than 3 hours ago is still active.

## 2026-10-15 17:53 UTC — IN PROGRESS

- Previous run (2026-10-10 09:49) never marked DONE; over 3 hours old, so this run takes over.
  PR #22 was merged into `main` (c2a169f); `auto/elya` fast-forwarded to it. A new draft PR
  will carry this run's work.
- LLVM 18 dev headers missing again; installed `llvm-18-dev libzstd-dev libpolly-18-dev`
  via apt. Codegen gate runs. Baseline 886 / 85.
- **Done: N7 part 2, type variables and parametric ADTs natively** (988348e), HANDOFF
  step 1. Spec `docs/superpowers/specs/2026-10-15-elya-n7b-type-monomorphization-design.md`.
  **Language/compiler decisions (mine, rule 2; please review):** whole-program
  monomorphization (not tagged ints or dictionaries; the collector stays precise, `Int`
  stays 64-bit); ambiguous type variables default to `Unit`; a type variable visible
  only in a function's own effect row is refused natively by name (now HANDOFF step 5).
  Two existing refusal tests now assert compilation (named in the commit). Gate 900 / 85
  (predicted 899, +1 from the review). Review: 2 findings (closures in parametric types
  miscompiled; phantom effect parameter regression), both fixed test-first.
- Windows-gate debt added: 988348e.
- **Done: recursion through a handle body** (4c692e2), a prerequisite of HANDOFF steps 1-2
  (PARKED since 5b-10). Spec `docs/superpowers/specs/2026-10-15-elya-handle-recursion-design.md`.
  **Decision (mine; please review):** a recursive call directly in a handle body includes
  the member's row instead of unifying with it. Consequence by design: a function
  annotated with the effect that used to leak (`/ {S}` where all S is handled inside) is
  now E0423 under the exact-row rule. Gate 909 / 87. Review: 2 soundness holes in my
  first version, fixed test-first (also closes a pre-existing hole of the same shape).
- Windows-gate debt added: 4c692e2.
- **Done: recursion through a lambda** (bce002c), same spec §5: a lambda that includes a
  recursive member's row closes with the group. **Two existing test expectations changed
  by this decision** (row_soundness `go`: E0420 instead of E0423; `go` under its handler
  now accepted, 64) -- named in the commit; please review. Review: 1 root cause (a
  member's later relay), fixed test-first; it also closed a hole present at 9eb8d5f.
  Gate 913 / 87.
- Windows-gate debt added: bce002c.
- **Done: async step 2 part 1, structured concurrency** (511d6ed): `examples/06_structured.elya`
  (spawn/await/scope as handlers; same bytes natively). Spec
  `docs/superpowers/specs/2026-10-15-elya-async-step2-structured-design.md`. Part 2 (I/O
  poll loop) is now HANDOFF step 1. Gate 916 / 87.
- Windows-gate debt added: 511d6ed.

## 2026-10-10 09:49 UTC — ended without a DONE mark (taken over 2026-10-15 17:53)

- Previous run (01:53) never marked DONE; over 3 hours old, so this run takes over.
  Its work is on `auto/elya`: native multi-shot handlers (01a7e36, fd32b81; gate 845 / 79).
- LLVM 18 dev headers (and libzstd-dev) were missing again; installed via apt. Codegen gate runs.
- **Done: evaluator linear continuations** (dfd83ca), HANDOFF step 1. Spec
  `docs/superpowers/specs/2026-10-10-elya-eval-linear-continuations-design.md`. Gate 859 / 81
  (predicted 859). Review: no findings.
- **Done: checked Int arithmetic; `/` and `%` natively** (8080b9f), prerequisite of replay.
  Spec `docs/superpowers/specs/2026-10-10-elya-checked-integer-arithmetic-design.md`.
  **Language decision (mine, per rule 2; please review):** Int overflow is a named runtime
  error on both sides (was: evaluator panic / native wrap). Gate 869 / 83. Review: no findings.
- **Done: nested and literal patterns natively** (c684b0b), prerequisite of replay. Spec
  `docs/superpowers/specs/2026-10-10-elya-native-pattern-compilation-design.md`. Review found
  exponential code size in my first design; redesigned test-first (linear). Gate 873 / 83.
- **Done: exact replay / handler-based testing** (9524cb3), HANDOFF step 1.
  `examples/05_replay.elya`. Gate 886 / 85.
- Next: HANDOFF step 1 (N7 part 2: type variables natively). (Done 2026-10-15.)
- Noticed, recorded in HANDOFF (not fixed): the parser continues a call across a newline
  (`f()` then a line starting `(` is `f()(...)`) -- a layout rule is a language decision.
- Windows-gate debt added: dfd83ca, 8080b9f, c684b0b, 9524cb3 (8080b9f adds runtime.c
  traps and LLVM overflow intrinsics -- worth a Windows run).

## 2026-10-10 01:53 UTC — ended without a DONE mark (taken over 09:49)

- **Done: native multi-shot handlers** (01a7e36, fd32b81), HANDOFF step 1 at the time.
  Spec `docs/superpowers/specs/2026-10-10-elya-native-multi-shot-design.md`. Gate 845 / 79.
- Windows-gate debt added: 01a7e36, fd32b81.

- Previous run (2026-10-09 17:53) never marked DONE; it is over 3 hours old, so this run
  takes over. Its work (786df15, 1a59091, b56566f) is intact on `auto/elya`.

## 2026-10-09 17:53 UTC — ended without a DONE mark (taken over 2026-10-10 01:53)

- Branch `auto/elya` created from `origin/main` (5799ba9, #21 merged). Draft PR #22.
- LLVM 18 dev headers were missing in the container; installed `llvm-18-dev` via apt, so
  the codegen gate runs.
- **Done: N7 part 1** (786df15): convention specialization + upcast adapter, native.
  Spec `docs/superpowers/specs/2026-10-09-elya-n7a-convention-specialization-design.md`.
  Gate 796 passed / 77 suites (baseline 783). Fixed two pre-existing native miscompiles.
  Independent review: 1 regression + 2 pre-existing issues, all pinned red and fixed.
- **Done: async step 1** (1a59091): function types in declarations + `examples/04_async.elya`
  (scheduler as a handler; same bytes natively). Gate 816 / 79 suites. Review: no
  regression; re-found the parked resume-row soundness gap (now HANDOFF step 1).
- **Done: resume-row completion** (b56566f): resume carries relayed, return-clause and
  re-entered-clause effects; `wrap(f)` accepted. Gate 836 / 79. Review: 2 holes in the
  first version fixed; 1 over-rejection pinned as known limitation (rows unify by
  equality -- see PARKED; a decision candidate for later: inclusion constraints).
- Next: HANDOFF step 1 (native multi-shot handlers).
- Windows-gate debt (check.ps1 not run): 786df15, 1a59091, b56566f.
