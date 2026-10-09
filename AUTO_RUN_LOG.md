# Auto-run log

Scheduled runs working on branch `auto/elya` record their state here, newest entry first.
A run whose top entry says IN PROGRESS and started less than 3 hours ago is still active.

## 2026-10-09 17:53 UTC — IN PROGRESS

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
- Next: HANDOFF step 1 (resume carries relayed and return-clause effects).
- Windows-gate debt (check.ps1 not run): 786df15, 1a59091.
