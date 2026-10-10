# Auto-run log

Scheduled runs working on branch `auto/elya` record their state here, newest entry first.
A run whose top entry says IN PROGRESS and started less than 3 hours ago is still active.

## 2026-10-10 09:49 UTC — IN PROGRESS

- Previous run (01:53) never marked DONE; over 3 hours old, so this run takes over.
  Its work is on `auto/elya`: native multi-shot handlers (01a7e36, fd32b81; gate 845 / 79).
- LLVM 18 dev headers (and libzstd-dev) were missing again; installed via apt. Codegen gate runs.
- **Done: evaluator linear continuations** (dfd83ca), HANDOFF step 1. Spec
  `docs/superpowers/specs/2026-10-10-elya-eval-linear-continuations-design.md`. Gate 859 / 81
  (predicted 859). Review: no findings.
- Next: HANDOFF step 1 (exact replay / handler-based testing, `examples/05_replay.elya`).
- Windows-gate debt added: dfd83ca.

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
