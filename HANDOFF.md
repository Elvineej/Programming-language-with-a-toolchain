# Handoff — where Elya is, and the next five steps

Written 2026-10-05 for whichever agent picks this up next. Read this, then `CLAUDE.md`,
then `PARKED.md`. The newest spec in `docs/superpowers/specs/` shows the house style.

## State at handoff (updated 2026-10-09, auto-run)

- `main` has everything through #21 (sub-effecting and checked annotations).
- Scheduled auto-runs work on branch `auto/elya` (one draft PR to `main`) and log in
  `AUTO_RUN_LOG.md`; the maintainer reviews on GitHub.
- History was rewritten on 2026-10-05 (the maintainer's request) so no commit carries a
  former e-mail address; every branch was force-pushed. Do not push old local branches.
- The repo is private until the maintainer flips it public (only they can).
- Parked, unmerged: `claude/outside-edit-tail-in-match` -- edits from another writer (not
  this session); superseded by `claude/tail-in-match`, kept until the maintainer says.
- Linux gate at the N7-part-1 tip of `auto/elya`: 796 passed, 77 suites. The runtime is
  untouched (`gc_mark` unchanged).
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

- **N7 part 1: convention specialization and the upcast adapter (native)** (2026-10-09,
  branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-09-elya-n7a-convention-specialization-design.md`. A
  Core-to-Core pass (`crates/codegen/src/specialize.rs`): references that instantiate a
  row variable at a user effect go to clones with substituted rows (top-level and
  `let`-bound lambdas); upcasts are eta-expanded into adapter lambdas, at every covariant
  layer. Fixed on the way: a pre-existing native miscompile of an upcast in a result
  (2 for 3306) and of one at a callee's result (11 for 1511, found by the review).

## The next five steps

### 1. Async as an effect: a scheduler handler (ROADMAP priority 1)

An `Async` effect (`fork`, `yield`) and a round-robin scheduler written as an ordinary
Elya handler that keeps a queue of suspended continuations. Show that the same `map` works
for sync and async code (no function colouring). The evaluator first, then natively (a
queue of continuations needs escaped resumes, which work natively; `fork` may need
multi-shot or a second continuation: measure). Ship `examples/04_async.elya`, with tests
and a README section.

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

### 5. Front end: a lambda forwarding an enclosing parameter keeps its row open; then N7 part 2

`fn wrap(f) { fn(x) { f(x) + 1 } }` used at `{L}` (and `compose(f, g)` with one pure
argument) is E0423: a lambda forwarding an enclosing function's parameter forces that
parameter pure (PARKED by sub-effecting, "deferred to N7"). Natively nothing blocks the
open version any more (N7 part 1 specializes it). Measure the probe programs of the N7a
spec (q1, q9, q10, p16) first. Then N7 part 2: type variables (`twice(f, x)` with `x: 'a`
is "unrepresentable type"): monomorphize types like rows, with the code-size budget, or
dictionary passing -- decide in its spec. Also: aliases of generic locals (`let mk2 = mk`)
are refused by name.

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
