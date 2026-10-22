# Auto-run log

Scheduled runs working on branch `auto/elya` record their state here, newest entry first.
A run whose top entry says IN PROGRESS and started less than 3 hours ago is still active.

## 2026-10-22 15:22 UTC — IN PROGRESS (interactive session, at the maintainer's "Start next task")

- The 2026-10-17 01:53 entry never marked DONE; its last commit (7ac69a6) is 10-17 04:19,
  days ago. Every scheduled run since has stopped on the usage limit at start. This
  session takes over at 7ac69a6; it re-dates this entry before it is 3 hours old while
  still working, and marks it DONE or hands it over before it stops.
- Working on: HANDOFF step 1, multi-shot resumes over a deep chain.

## 2026-10-17 01:53 UTC — ended without a DONE mark (taken over 2026-10-22 15:22)

- The 2026-10-16 17:53 run never marked DONE; its last commit (7dc40b5) is 20:01, over
  3 hours ago, so this run takes over at 7dc40b5.
- LLVM 18 dev headers and libzstd-dev installed via apt. Baseline 1054 / 101.
- **Done: `with any` natively** (1789c12, HANDOFF step 1). Spec
  `docs/superpowers/specs/2026-10-17-elya-native-any-handlers-design.md`. After row
  specialization a `with any` handle is expanded into a plain handle, one clause per
  handled operation at its answer type; literals through a runtime reflection table
  with the evaluator's failure words. `examples/09_record_any.elya` runs natively.
  Gate 1069 / 102 (predicted 1067 before the review; +2 review tests). Sweep: 700
  programs, no disagreement or crash; of the 9 `with any` programs, 6 now agree and 3
  are refused by name (2 `Float`, 1 non-regular type), as predicted.
  - **Decisions (mine; please review):** expansion (monomorphizing the clause) rather
    than a run-time operation value; an operation value used other than by `op.*` or
    `==` is refused natively by name (no corpus program does it; HANDOFF step 5);
    decoding at an effect's own parameter stays a failure natively too (parity with
    the evaluator's S2, though natively the type is known).
  - **Changed expectation by this decision:** tests/core_lowering.rs
    `an_operation_generic_handler_is_refused_at_lowering` became
    `an_operation_generic_handler_lowers_marked_any`.
  - Review: 2 high (a generic helper at the answer type; `o == o` with a function in
    an argument) + 1 medium, fixed test-first.
  - `graphify` (CLAUDE.md) is not installed here and `graphify-out/` does not exist,
    so no graph update was run.
- Windows-gate debt added: 1789c12 (new runtime C: `elya_any_*`).
- **Done: unary operators natively; wrapping and bit arithmetic** (a6905a5, HANDOFF
  step 1). Spec `docs/superpowers/specs/2026-10-17-elya-unary-and-wrapping-design.md`.
  `-e`/`!e` lower in Core as `0 - e`/`e == False`; new pure builtins
  `int.wrapping_add/sub/mul`, `int.and/or/xor`, `int.shl/shr/ushr`. Gate 1079 / 105
  (predicted 1077, +2 from the review's fix).
  - **Decisions (mine; please review):** named builtins, not operators (`^`, `<<`,
    `&`, `|`); Rust's names; an out-of-range shift amount fails by name rather than
    being masked; no wrapping integer type.
  - Review: 1 low finding fixed (E0208's help now wraps a builtin at its arity);
    MIN not writable as a literal and the leading-`-` line continuation recorded as
    HANDOFF step 5.
  - Plan deviation: the native unary test was written after Core's lowering, so it
    was not red first natively (its root counterpart was; K1 shows it has teeth).
- Windows-gate debt added: a6905a5.
- **Done: record and replay natively** (95f4680, HANDOFF step 1). Spec
  `docs/superpowers/specs/2026-10-17-elya-native-record-replay-design.md`.
  `ELYA_RECORD=<file>` / `ELYA_REPLAY=<file>` on a native binary; the evaluator's
  trace format and failure words, so traces move both ways between `elya run` and
  native runs. Gate 1086 / 106 (predicted 1084, +2 review tests).
  - **Decisions (mine; please review):** environment variables (not a build flag);
    a malformed trace exits 1 and an unreadable/unwritable one exits 2, matching
    the CLI.
  - Review: 3 medium (unchecked writes, Unicode trim, non-UTF-8 traces) + 1 low
    (NUL bytes), fixed test-first. Found (older): `--replay` drops a failing run's
    output and `--record` puts it on stderr -- now HANDOFF step 5.
- Windows-gate debt added: 95f4680 (runtime C; the Windows `elya_start` now calls
  the trace init/finish too).
- Still active at 04:25 UTC, on HANDOFF step 1 (multi-shot resumes over a deep chain).

## 2026-10-16 17:53 UTC — ended without a DONE mark (taken over 2026-10-17 01:53)

- The 12:30 entry never marked DONE; its last commit (2b7ce18) is 14:33, over 3 hours
  ago, so this run takes over at 2b7ce18. LLVM 18 dev headers installed via apt.
  Baseline 1033 / 99.
- **Done: deep non-tail recursion natively** (94ee05e, HANDOFF step 1). Spec
  `docs/superpowers/specs/2026-10-16-elya-native-stack-design.md`. Native programs run on
  their own 1 GiB stack (`elya_start`); `len` over 1 000 000 and a non-tail resume over
  1 000 000 run (they died with an unnamed SIGSEGV past about 130 000 frames); an overflow
  is now `elya: native stack overflow: ...`, exit 1. Review: 2 medium + 2 low findings;
  M1 and L1 fixed test-first. Gate 1039 / 99 (predicted 1039).
  - **Decisions (mine; please review):** a FIXED 1 GiB stack (not sized from the
    machine's memory, for reproducibility), `ELYA_STACK_MIB` overrides, no silent
    fallback. **Consequence (review M2):** under `ulimit -v` below about 1 GiB, or strict
    overcommit on a small machine, every native program stops with `elya: cannot
    reserve a stack of 1024 MiB (ELYA_STACK_MIB sets a smaller one)`. If you prefer a
    fallback or a smaller default, say so.
  - Elya functions are now mangled `elya.<name>` (a function named `alloc`, `println` or
    `start` failed to link against the runtime's own symbol).
  - Existing native tests run their binaries with `ELYA_STACK_MIB=8` (the stack they were
    written against), so tail-call regressions still overflow.
- **Done: structural equality** (9bb6f5d, HANDOFF step 1). Spec
  `docs/superpowers/specs/2026-10-16-elya-structural-equality-design.md`. `==` on data
  only; comparing functions is the **new diagnostic E0436** (via equality type
  variables, SML-style); iterative in the evaluator, `elya_eq` natively. Review: 1 high
  (two pre-existing tests broke) + 3 low, fixed. Gate 1049 / 101.
  - **Decisions (mine; please review):** E0436 rather than identity equality or a
    run-time error on functions; `with any` operation values are data (N3 kept);
    phantom type parameters at function types are rejected.
  - **Changed expectation by this decision:** lib.rs
    `rejects_eq_on_an_unrepresentable_operand_type` now builds its operand by hand (the
    ADT it used compiles); the expected refusal is unchanged.
- Windows-gate debt added: 9bb6f5d.
- **Done: a repeatable differential sweep** (9176644, HANDOFF step 1). Spec
  `docs/superpowers/specs/2026-10-16-elya-differential-sweep-design.md`.
  `python3 scripts/sweep.py` (needs Python 3; not part of the gate). New CLI flag
  **`elya run --value`** (decision: mine; prints main's value as a native binary does).
  `elya run` now keeps output printed before a runtime error (was dropped). Result at
  9bb6f5d: 409 checked programs, 382 identical, **no disagreement and no native crash**;
  found and fixed `elya build` panicking (debug build) on warning-only programs. Top
  native gap: `with any` (9 programs), now HANDOFF step 1. Gate 1054 / 101.
- Windows-gate debt added: 9176644.
- PR #23 was merged by the maintainer (2026-10-16 18:32, at ab791b6); a new draft PR is
  open from `auto/elya`.
- Windows-gate debt added: 94ee05e (**priority**: the Windows `elya_start` path --
  CreateThread reservation, SetThreadStackGuarantee, the vectored handler -- has never
  run).

## 2026-10-16 12:30 UTC — ended without a DONE mark (taken over 2026-10-16 17:53)

- The 2026-10-15 17:53 run's session resumed after its usage limit. The 05:53 run's last
  commit (0d1d0dc) is 07:55, over 3 hours ago, so this run takes over at 0d1d0dc.
- PR #23 was marked ready for review by the maintainer (2026-10-16 12:29); it stays open.
- Baseline at 7cbef8a: 998 / 93.
- **Done: HANDOFF step 1, a function's own row in Core** (adb66f5). Spec
  `docs/superpowers/specs/2026-10-16-elya-own-row-in-core-design.md`. **Decision (mine;
  please review):** the function's type is recorded in the typed table at its
  declaration span. **Changed expectations by this decision:** the node-count tripwire
  (5 -> 6), seven `typed_inference` snapshots (one added line each), and mono.rs's two
  row-only refusal tests (now clones). Found: N7 part 2's K4 "crash" was a native stack
  overflow; deep non-tail recursion natively is the new HANDOFF step 5. Review: 52
  probes, no finding. Gate 1001 / 93 (predicted 1001).
- Windows-gate debt added: adb66f5.
- **Done: failure as an effect and scope cancellation** (4e2ca0b, HANDOFF step 1). Spec
  `docs/superpowers/specs/2026-10-16-elya-failure-and-cancellation-design.md`.
  **Decisions (mine; please review):** bottom as an empty type plus an empty match (no
  new type rule); **new diagnostic E0209** for any name declared twice (functions,
  types, constructors, effects were silent overwrites). `examples/10_failure.elya`.
  Review: 1 finding (the duplicate names), fixed test-first. Gate 1010 / 95.
- Windows-gate debt added: 4e2ca0b.
- **Done: computed callees and functions as values natively** (05f0676, HANDOFF step 1).
  Spec `docs/superpowers/specs/2026-10-16-elya-callees-and-function-values-design.md`.
  **Changed expectations by this decision:** two lib.rs refusal tests now assert
  compilation (named in the commit). Review: 1 finding (a widened row on the inner
  call), fixed test-first. Gate 1012 / 95.
- Windows-gate debt added: 05f0676.
- **Done: argument records past three parameters** (742a840, HANDOFF step 1). Spec
  `docs/superpowers/specs/2026-10-16-elya-argument-records-design.md`. **Decision (mine;
  please review):** one rule for every kind (more than three parameters: two kept, the
  rest in one record), not per-kind or win64-only spilling. **Changed expectations by
  this decision:** five lib.rs arity-refusal tests now assert compilation. Review: 1
  high finding (record keys), fixed test-first. Gate 1015 / 95.
- Windows-gate debt added: 742a840 (**priority**: the point of the slice is win64's
  tail-call cap).
- **Done: record and replay from the command line** (a90d362, HANDOFF step 1). Spec
  `docs/superpowers/specs/2026-10-16-elya-cli-record-replay-design.md`. **Decisions
  (mine; please review):** the trace is the HOST's answers (`time.*`), not user effects;
  text format `elya trace 1` + `question = answer`; plain `elya run` with extra
  arguments is now a usage error (exit 2). Review: 1 medium + several low findings,
  fixed. Gate 1033 / 99.
- Windows-gate debt added: a90d362.

## 2026-10-16 05:53 UTC — ended without a DONE mark (taken over 2026-10-16 12:30)

- The 2026-10-15 21:43 entry never marked DONE; its last commit (7c951d2) is from 23:17,
  over 3 hours ago, so this run takes over. LLVM 18 dev headers were missing again;
  installed via apt (after `apt-get update`). Baseline 941 / 91.
- **Done: HANDOFF step 1, async step 2 part 2 (timers and a poll loop).**
  - 1d9f565: `examples/08_timers.elya` (sleep, a timer list, `Clock` under a virtual
    and a real clock, a deadlock named), snapshot, crosscheck row, native same-bytes
    test. Gate 944 / 91 (predicted 944).
  - 61a84f9: review findings fixed test-first (expired timers starved by a busy queue;
    Windows `Sleep` returning early; a value that depended on real time). Gate 946 / 91.
  - Plan prediction that was wrong, recorded in the spec: control T1 (virtual clock
    never advances) changes the elapsed line, not the answer order.
- **Decisions (mine, rule 2; please review):** the clocks are generic in their answer;
  a deadline is taken when the scheduler dequeues `Sleeping` (one lap late at most).
- **Found:** an effectful function with five parameters is refused natively (the win64
  tail-call cap). It is the new HANDOFF step 5.
- **Windows-gate debt added:** e84c105, 7c951d2, 61a84f9 (runtime.c `_WIN32` time
  branch, now looping on `GetTickCount64`), 1d9f565.
- **Done: HANDOFF step 1 (old step 2), operation-generic handlers `with any`.**
  - Spec `docs/superpowers/specs/2026-10-16-elya-any-handlers-design.md`; plan
    `docs/superpowers/plans/2026-10-16-elya-any-handlers.md`.
  - Commits:
    - fe8b10c parse;
    - ae7d7c1 types and E0435;
    - 564ca33 E0434;
    - 17ac4c8 evaluator and literals;
    - 16c03fd native refusal by name;
    - cf474e8 `examples/09_record_any.elya`;
    - cd70f3e and 6798543 review fixes.
  - Gate: 998 / 93. Every task matched its prediction, or the commit says why it did
    not (tests added).
  - **Decisions for review (rule 2; spec section 1):**
    - the syntax `with any { op(o) -> ... }`;
    - `IO` is never handled;
    - the answer type is rigid per clause;
    - five `op.*` builtins;
    - Elya literal syntax is the encoding;
    - the handled effects must be data (E0434), but a forwarding handler's tail is
      checked at run time, because a call's row unification makes it the caller's
      ambient (measured);
    - evaluator only for now.
  - **Two independent reviews** found 4 soundness holes. All are fixed test-first.
    One is PRE-EXISTING for plain handlers: an escaped `resume` that re-enters a
    clause calling a parameter was typed without that parameter's row. It is fixed
    in `infer_handle` too and pinned in `tests/resume_row.rs`.
  - Changed expectation (this slice's own): the 09 snapshot spells `Unit`, not `()`.
- Next: HANDOFF step 1, a function's own row in Core.

## 2026-10-15 21:43 UTC — ended without a DONE mark (interactive session, at the maintainer's "Continue")

- The interactive session that fixed d07afea is working on `auto/elya` now, so scheduled
  runs started within 3 hours of this entry exit, as the protocol says. It marks this
  entry DONE, or hands it over, before it stops.
- **Done: native strings** (cc044d4 and cf6b08b, HANDOFF step 2, taken before step 1 on
  purpose). Spec `docs/superpowers/specs/2026-10-15-elya-native-strings-design.md`.
  - **What:** `<>`, `==`/`!=` on strings, string-literal patterns and `int.to_string`,
    natively.
  - **Decisions (mine, rule 2; please review):**
    - `int.to_string` is a pure builtin (D1);
    - a builtin used as a value is E0208 (D6).
  - **Changed test expectations, on purpose:** three lib.rs unit tests went from
    refusal to compilation. They are named in cf6b08b.
  - **Gate:** 934 / 89 (predicted 934). Review: no correctness bug; its 4 findings were
    fixed test-first.
- Windows-gate debt added: d07afea, cc044d4, cf6b08b (cf6b08b adds runtime.c functions;
  portable C only).
- Next: HANDOFF step 1, async step 2 part 2 (the poll loop).

## 2026-10-15 21:35 UTC — interactive session (not a scheduled run), DONE

**A silent miscompile in N7 part 2 (988348e). FIXED in d07afea.** The fix is on this
branch: an instance key renders a row as its convention. It was tested red to green, with
negative control C1. Gate 920 / 87 (predicted 920). Windows-gate debt: d07afea. Nothing
below is left for the next run except the "computed callee" note.

- **What fails.** This program prints `x` then 42 under `elya run`. Built natively at
  4b9d7c6, it stops with "elya: match failed (no arm matched)":

  ```
  type Box(a) { B(a) }
  fn call(b) { match b { B(f) -> f() } }
  pub fn main() -> Int { call(B(fn() { io.println("x")  42 })) }
  ```

  Same class: a generic `run_all` over a `List` holding one printing closure and one pure
  one (evaluator `a`, 56).

- **Cause.** In `crates/codegen/src/mono.rs`, `canon` with `rows == None` (the INSTANCE
  key) drops row tails but keeps LABELS, on the premise that only labels change a field's
  convention. `IO` is a label that does NOT change the convention: `cps::needs_cps` treats
  it as direct. So `main`'s `Box(fn() / {IO} -> Int)` and `call`'s `Box(fn() / {|r} -> Int)`
  become two instances with different constructor tags.

- **Fix.** In an instance key, render each function row as its convention only (whether
  `needs_cps` holds), not its labels. One value has the same convention at every use, so
  the binder field types stay right.

- **Tests.** Three rows in `TYPE_VARS`, `f3-*`. They don't print: an annotation puts `IO`
  in the type. All three failed with "match failed" in both corpus tests before the fix.
  - The fix was made on `claude/io-instance-key`, then fast-forwarded here.
  - C1 (the convention marker dropped too): only `f3-one-type-at-both-conventions` fails,
    by D16's refusal name. Reverted, `cmp` clean.
  - Re-running the ~700-program sweep: the disagreement is gone, and nothing else moved.

- **How it was found.** A differential sweep of ~700 programs (earlier review corpora plus
  `examples/`) at 4b9d7c6: native against the evaluator. The two programs above were the
  only native results that disagreed with the evaluator. Every other result matched,
  failed the same way on both sides, timed out in the evaluator, or was a named refusal.

- **Refusals worth a slice.** 11 programs are refused natively as "computed callee", for
  example `(compose(f, g))(x)` or `(mk())(1)`. In a parallel implementation (local, not
  pushed), Core lowering A-normalized every computed callee (`let $k = e in $k(args)`), and
  those 11 programs compiled and matched the evaluator.

- **The previous run (17:53) ended at 19:53 UTC on the usage limit** without a DONE mark.

## 2026-10-15 17:53 UTC — ended without a DONE mark (usage limit at 19:53 UTC)

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
- **Done: backtracking example + deep multi-shot chains** (865322c, HANDOFF step 2): `examples/07_search.elya`;
  100 000-frame multi-shot copy natively, linear. Gate 920 / 87.
- Windows-gate debt added: 865322c.

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
