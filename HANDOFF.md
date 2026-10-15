# Handoff — where Elya is, and the next five steps

Written 2026-10-05 for whichever agent picks this up next. Read this, then `CLAUDE.md`,
then `PARKED.md`. The newest spec in `docs/superpowers/specs/` shows the house style.

## State at handoff (updated 2026-10-10)

- `main` has everything through #15 (5b-9b). The stacked PRs #16-#19 were merged into
  each other's branches, not into `main`, so `main` still lacks 5b-10, the three
  soundness fixes, E0207 and sub-effecting. **One PR lands all of it:** #20, branch
  `claude/sub-effecting` -> `main`. From now on: one PR at a time, always based on `main`.
- Waiting on the maintainer's merge, in order: #20 (sub-effecting and everything before
  it), #21 (`claude/annotations`), then the async PR (`claude/async`, built on the
  annotations commit). Each targets `main` and shows only its own commits once the one
  before it merges.
- History was rewritten on 2026-10-05 (the maintainer's request) so no commit carries a
  university address; every branch was force-pushed. Do not push old local branches.
- The repo is private until the maintainer flips it public (only they can).
- Parked, unmerged: `claude/outside-edit-tail-in-match` -- edits from another writer (not
  this session); superseded by `claude/tail-in-match`, kept until the maintainer says.
- Linux gate at the async tip: 812 passed, 79 suites. `gc_mark` 637 bytes.
- **Direction (2026-10-09):** `docs/ROADMAP.md` -- the problems Elya is for (async without
  colouring, per-dependency capabilities, exact replay and handler-based testing) and the
  language decisions taken. The maintainer delegated all choices (rule 2).

## How work is done here (non-negotiable)

1. **Measure first.** Run the real program through `elya check | run | build` and write
   the table into the spec before designing (see any 5c spec, §0).
2. **Language decisions: delegated to Claude (the maintainer, 2026-10-09: "From now on u
   handle all including the choices", aiming for a language with real advantages --
   high-tech and experimental, but WORKING; unconventional implementations are fine).**
   Decide, write the options and the reason into the spec, and say so in the PR. Never
   decide silently, and never trade away soundness or the gate for novelty.
3. **Spec, then plan (with numeric predictions), then red tests, then code.** Every new
   test must fail first with the predicted value.
4. **Negative controls:** break each rule on purpose, observe a *distinct* failure,
   revert, and confirm `git diff --quiet`. Record them in the test file's header. A
   control that fails nothing is a finding (5c-2's K3 produced a type test that way).
5. **Gate:** `sh scripts/check.sh`, exit 0, with the *predicted* pass count. Root-crate
   tests count **twice** (both gate configurations); native tests once. If cargo stops
   early, the counts are partial, so use `--no-fail-fast` when diagnosing.
6. **An independent review agent** reads every slice's diff before the final commit. Fix
   its findings test-first.
7. **Commits:**
   - Explicit `git add` paths, never `-A`.
   - Write the message to a file and use `git commit -F` (backticks in `-m` get executed).
   - End with the `Co-Authored-By` / `Claude-Session` lines.
   - Never amend, force-push or delete branches without asking.
8. **Never change an existing test's expected value** unless a maintainer decision
   requires it, and call each one out in the commit message.
9. **MSRV is 1.75** (clippy enforces it): no `is_none_or`, no let-chains.
10. **Windows:**
    - `.github/workflows/windows-gate.yml` runs `scripts/check.ps1` on every push to
      `claude/**`.
    - The Actions logs API is blocked from the cloud session. Read the results with
      `gh api repos/Elvineej/Programming-language-with-a-toolchain/commits/<sha>/check-runs`,
      then that run's `/annotations` (a `notice` carries `passed=N failed=M`; failures
      carry the log tail).
11. **The agent cannot mark PRs ready or merge them** (blocked by the permission system,
    including via the browser). Open PRs ready for review and let the maintainer click
    Merge.
12. **Only Claude writes this file** (the maintainer's rule, 2026-10-05). Other agents and
    tools (Cline, other models) read it but never edit it; they report progress and Claude
    records it here.
13. **Keep this file a rolling window of five** (the maintainer's standing rule). When a
    step is finished, move it to "Done" with its PR or commit, renumber the rest, and
    write a NEW step 5, so the next agent always sees five steps ahead. Update the State
    section in the same commit.

## Done

- **Land the stack and confirm `main`** (2026-10-05): #5, #6, #7 merged; `main` gated at
  647 after the merges and the email rewrite; README status updated (PR #8). Still open
  from it: the maintainer runs `scripts/check.ps1` locally once (vcpkg LLVM, not CI's).
- **Slice 5b-9a: effectful code inside a `match`, natively** (2026-10-05): spec
  `docs/superpowers/specs/2026-10-05-elya-slice-5b9a-effectful-match-design.md`; gate 650.
  Also fixed: a pre-existing direct-emitter crash on a `match` arm after a catch-all.

- **Type-safety fix: inference's call graph** (2026-10-05, PR #10): calls inside match arms,
  lambdas and handlers were no edge, so a caller could be generalised before its callee
  (`forall a` results; ill-typed programs checked clean, depending on declaration order).
- **A CEK machine written in Elya** (2026-10-05, the maintainer's request):
  `examples/03_cek.elya` -- a lambda calculus (de Bruijn indices, integers, `+`, a zero
  test, the Z combinator), run by `elya run` (to N = 2,000 in tests; 10,000 measured) and
  natively (to N = 1,000; deeper waits on step 1). `crosscheck` now runs examples on a
  64 MiB thread (the tree-walker oracle recurses on the host stack).

- **Tail calls inside a `match` arm** (2026-10-05, branch `claude/tail-in-match`): the
  direct emitter's `lower_tail` had no `Match` case, so a call in an arm was an ordinary
  call (pre-existing since 5b-4; red on Windows `main`). `lower_match` now has a value
  mode and a tail mode. A million-deep loop and list walk, and the Elya CEK machine at
  N = 100,000, run natively. Supersedes the parked `claude/outside-edit-tail-in-match`.

- **Slice 5b-9b: effectful lambdas and closure calls, natively** (2026-10-05, branch
  `claude/slice-5b9b-effectful-lambdas`): CPS-convention lifted bodies that continue their
  enclosing scope's binding indices; effectful closure calls as site calls or tail jumps.
  Spec `docs/superpowers/specs/2026-10-05-elya-slice-5b9b-effectful-lambdas-design.md`.

- **Slice 5b-10: nested handles and handles inside effectful code, natively (D17 lifted)**
  (2026-10-08, branch `claude/slice-5b10-nested-handles`): spec
  `docs/superpowers/specs/2026-10-08-elya-slice-5b10-nested-handles-design.md`. A handle
  that leaks an effect is a CPS handle (a continuation site; its clauses and return clause
  CPS regions); handler frames gained a `parent` word; performs walk parents; resumes
  re-install their frame at the resume. Found on the way (parked): `resume` is typed
  effect-free (a soundness hole), partial handles check clean, recursion through a handle
  body is rejected by inference.

- **`resume` carries its handle's row** (2026-10-08, branch `claude/resume-row`): spec
  `docs/superpowers/specs/2026-10-08-elya-resume-row-design.md`. Closes the soundness hole
  5b-10 found; the first version opened a new one (tail unification), caught by the
  independent review and fixed test-first. m10's T-carrying variant now compiles natively.

- **Effects dropped on a closed row** (2026-10-08, branch `claude/row-conflicts`): spec
  `docs/superpowers/specs/2026-10-08-elya-row-conflicts-design.md`. Now E0423; a second
  half (an environment rule for lambda tails) was reverted after the review found two
  regressions -- sub-effecting is now a parked language question.

- **Partial handlers are an error, `E0207`** (2026-10-09, branch `claude/partial-handlers`,
  the maintainer's choice): `tests/handler_coverage.rs`. `effect_syntax`'s A3 program
  gained the `State.set` clause it lacked; the three partial-handler native rows went (the
  programs no longer pass the front end).

- **Sub-effecting for function values** (2026-10-09, branch `claude/sub-effecting`, the
  maintainer's decision; design Claude's): spec
  `docs/superpowers/specs/2026-10-09-elya-sub-effecting-design.md`. Phantom tails (Koka's
  open/close), inclusions at calls instead of unification, flushed before rows close. On
  ~840 probe programs nothing changed from accepted to rejected, and every newly accepted
  one runs in the evaluator. Natively a direct closure used at an effectful type is
  refused by name (it miscompiled: 2 for 13).

- **Type annotations are checked** (2026-10-09, branch `claude/annotations`): spec
  `docs/superpowers/specs/2026-10-09-elya-annotations-design.md`. Parameter, return,
  `let` and lambda annotations used to be DISCARDED by the parser; function types
  `fn(A) / {E} -> R` are new syntax. Flexible per-function type variables; an unwritten
  row is an upcast tail where a value is produced and a real variable where consumed. The
  review found a native GC unsoundness through a generalized annotation variable (3395 for
  42), fixed first.

- **Async without function colouring, with N7's specialization** (2026-10-10, branch
  `claude/async`; old steps 1 and 2). Spec:
  `docs/superpowers/specs/2026-10-09-elya-async-design.md`.
  - **The example:** `examples/04_async.elya` is a round-robin scheduler written as an
    Elya handler, with `fork` and ONE `each` used by sync code and by yielding tasks. It
    runs natively with the evaluator's output.
  - **Front end:**
    - function types in type and effect declarations;
    - deferred lambda closing, where the value row includes the ambient and is
      generalized with the inclusion copied.
  - **Native** (`crates/codegen/src/specialize.rs`):
    - convention clones (`$poly`);
    - type-variable monomorphization;
    - parametric ADT instances keyed up to rows;
    - computed callees A-normalized.
  - **Review6 found** a tag miscompile (one instantiation became several types), a build
    that never finished on a non-regular type, and the deferral over-rejecting six
    programs base accepted. All three were fixed test-first, with controls.
  - **Not done from old step 1:** the closure adapter (now step 5).

## The next five steps

### 1. `resume` carries the rows its body relays (soundness)

PARKED, now reachable in the scheduler shape: an UNANNOTATED `run(task)` whose task
performs an outer `T` checks clean and the evaluator stops on an unhandled effect
(review6's s03); review2's a1 is refused natively for the same reason. `resume` adds the
handled body's labels but not a tail it only relays. Tying the resume site to that tail
by equality (as a call of a parameter does) forces an unannotated scheduler's tasks to be
`Async`-only -- the queue's closed `{IO}` closes the relayed row -- so it needs a real
design: an upper-bounded inclusion (`w ⊆ {IO}`), or the clause rows known before the
clauses are typed (the return clause's and re-entered clauses' effects are the same
family). Measure s03, s04, a1 and `tests/async_effect.rs` first; nothing that checks
today may start failing without a named reason.

### 2. Native multi-shot handlers (`with multi`)

Refused natively by name since 5b-8 (A2); the evaluator runs them. A multi-shot resume
re-runs a captured continuation, but native frames are consumed in place -- and since 5b-10
a resume also MUTATES its handler frame (`next`, `parent`): a second resume needs the frame
chain, handler frames included, COPIED first, and the one-shot word replaced by a
copy-on-resume rule. Measure first (the evaluator's multi-shot corpus, e.g.
`multi_shot_collects_both_branches`, through `build`), then a spec with the copy cost
stated and gated against A4's live-set instrument. Multi-shot is what a native
probabilistic or backtracking handler needs (ROADMAP), so it is worth doing natively.

### 3. Evaluator: non-tail recursion under a handler is quadratic

PARKED: about 4× time per doubling (n=4000 takes 1.7 s; n=100 000 did not finish in 10
minutes). This caps every differential test's N. Profile (frame capture copies the
continuation?), predict the complexity, fix, and pin it with a timing-free test that
counts steps or allocations. The evaluator is the reference semantics, so the
differential corpora must stay green unchanged.

### 4. Exact replay and handler-based testing (ROADMAP priority 3)

A `Record` handler that wraps a computation and logs every effect's answer (op name,
arguments, the value it resumed with), and a `Replay` handler that feeds a log back and
stops by name on the first divergence. Written in Elya as ordinary handlers over a demo
effect set first (needs a list of answers: an ADT log is enough; strings wait for native
`<>`), shown on a program whose result depends on its effects, then a test that swaps
handlers instead of mocking. Evaluator and native. Ship `examples/05_replay.elya` with
tests and a README section; the CLI flag (`elya run --record/--replay`) comes after real
I/O effects exist.

### 5. The closure adapter, and what specialization still refuses (native)

**The adapter.** A direct closure used where an effectful function is expected is
refused by name, "direct function used where an effectful one is expected". This is
D16: its code has the direct convention, and the caller calls it as CPS. Wrap it at the
upcast in an adapter closure, `adapt(clos, args.., k) = k(code(clos.inner, args..))`,
with one descriptor row. That accepts review6's x1 and the j01/j03/j07 shapes (a `let`,
a field, a pattern).

**Same pass, the specialization leftovers (PARKED):**
- a generic function used as a value: clone it at the value's type;
- shadowing a generic name keeps the original (d04): make `refs` scope-aware;
- f05's misleading refusal name;
- let-polymorphic lambdas: clone per instantiation, or refuse them more clearly.

**Rules:** measure through `build` first and keep `gc_mark` byte-identical.

Also open, unscheduled: native strings (`<>` refused natively, no Int-to-String; the
builtin's name is Claude's call now -- `int.to_string` is the natural one); the roadmap's
other priorities (record/replay handlers, then capabilities once modules exist); the rest
of the soundness sweep (resume's row lacks return-clause and re-entered-clause effects;
recursion through a lambda the function handles around, PARKED); recursion through a
handle body is rejected by inference (PARKED). A perform walks the handler chain, so a program whose handler stack really grows
(a resume inside a fresh handle every iteration) pays O(depth) per perform; the evaluator
is no faster there. Also: the Elya CEK machine's next versions (a parser for its terms, a step
counter as an Elya effect); PIC/PIE linking (needs a Windows run), and a named diagnosis for
a native stack overflow in tests other than A9.
