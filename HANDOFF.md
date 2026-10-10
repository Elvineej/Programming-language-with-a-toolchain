# Handoff — where Elya is, and the next five steps

Written 2026-10-05 for whichever agent picks this up next. Read this, then `CLAUDE.md`,
then `PARKED.md`. The newest spec in `docs/superpowers/specs/` shows the house style.

## State at handoff (updated 2026-10-10, auto-run)

- `main` has everything through #21 (sub-effecting and checked annotations).
- Scheduled auto-runs work on branch `auto/elya` (one draft PR to `main`) and log in
  `AUTO_RUN_LOG.md`; the maintainer reviews on GitHub.
- History was rewritten on 2026-10-05 (the maintainer's request) so no commit carries a
  former e-mail address; every branch was force-pushed. Do not push old local branches.
- The repo is private until the maintainer flips it public (only they can).
- Parked, unmerged: `claude/outside-edit-tail-in-match` -- edits from another writer (not
  this session); superseded by `claude/tail-in-match`, kept until the maintainer says.
- Linux gate at the tip of `auto/elya`: 869 passed, 83 suites. The runtime gained
  `elya_cont_copy` (multi-shot); `gc_mark` is unchanged.
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

- **Async step 1: function types in declarations, and a scheduler written as a handler**
  (2026-10-09, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-09-elya-async-step1-design.md`. `fn(A) / {E} -> R` in
  constructor fields and operations (an unwritten declaration row is empty);
  `examples/04_async.elya` (fork, yield, a round-robin queue, one `each` for sync and
  async), the same bytes natively. Pattern binders are tracked by the native adapter.

- **`resume` carries relayed, return-clause and re-entered-clause effects** (2026-10-09,
  branch `auto/elya`): spec `docs/superpowers/specs/2026-10-09-elya-resume-row-complete-design.md`.
  Closes the last parked resume-row gaps (15 probe programs checked clean and then hit an
  unhandled effect); a lambda relaying an ENCLOSING parameter keeps its row open, so
  `wrap(f)`/`compose` are accepted (natively with annotations, via N7 part 1).

- **Native multi-shot handlers (`with multi`)** (2026-10-10, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-10-elya-native-multi-shot-design.md`. A resume of a
  `with multi` handle re-enters a COPY of its captured frames (`elya_cont_copy` in the
  runtime: next relinked, handler parents and `innermost` remapped); the original is never
  run. Keyed on the handler's `with multi` (`CoreHandle::multi`), so a plain `with` over a
  `multi` effect is one-shot natively, as in the evaluator. The live set stays bounded.

- **The evaluator's continuation costs linearly at any depth** (2026-10-10, branch
  `auto/elya`): spec `docs/superpowers/specs/2026-10-10-elya-eval-linear-continuations-design.md`.
  Two causes, not the handler: a per-step depth walk, and perform/resume copying frames.
  The continuation is now segmented at handlers (a meta-continuation): a perform walks
  handler boundaries and shares the frames above them. `Interp::cost()` pins it
  timing-free (`tests/eval_complexity.rs`); the PARKED shape runs at n = 100 000.

- **`Int` arithmetic is exact or fails by name; `/` and `%` natively** (2026-10-10, branch
  `auto/elya`, a prerequisite of the replay step): spec
  `docs/superpowers/specs/2026-10-10-elya-checked-integer-arithmetic-design.md`. Overflow,
  `MIN / -1` and a zero divisor stop both sides by the same name (the evaluator used to
  panic, native wrapped); `MIN % -1` is 0. Closes 5b-1 §11's deferred overflow question.

## The next five steps

### 1. Exact replay and handler-based testing (ROADMAP priority 3)

A `Record` handler that wraps a computation and logs every effect's answer (op name,
arguments, the value it resumed with), and a `Replay` handler that feeds a log back and
stops by name on the first divergence. Written in Elya as ordinary handlers over a demo
effect set first (needs a list of answers: an ADT log is enough; strings wait for native
`<>`), shown on a program whose result depends on its effects, then a test that swaps
handlers instead of mocking. Evaluator and native. Ship `examples/05_replay.elya` with
tests and a README section; the CLI flag (`elya run --record/--replay`) comes after real
I/O effects exist.

### 2. N7 part 2: type variables natively

`twice(f, x)` with `x: 'a`, and every unannotated `wrap`/`compose` (now accepted by the
front end), are "unrepresentable type" natively. Monomorphize type variables like rows
(N7 part 1's `specialize.rs` already matches signatures against uses), with the code-size
budget, or pass dictionaries -- decide in the spec. Parametric ADTs (`type Q(a)`) are the
same question for constructors. Also: aliases of generic locals (`let mk2 = mk`) are
refused by name.

### 3. Async step 2: structured concurrency and a poll loop

ROADMAP 1b/1c on top of `examples/04_async.elya`: a scope handler that joins its children
(a `spawn` returning a handle, `await` as an effect), then real I/O readiness through a
runtime poll loop the scheduler consults. Recursion through a handle body (PARKED) blocks
the natural recursive `task`: measure whether it is the next front-end fix.

### 4. A backtracking example, and handler frames in deep chains

Multi-shot runs natively now; show it: `examples/05_search.elya` (or the next free
number), an N-queens or subset-sum search written as an ordinary function over a
`multi` `Choose` effect, with handlers that collect all answers, the first answer, and a
count -- one search, three meanings, the same bytes natively. Then measure the one shape
the multi-shot review could not build: a captured chain holding MANY handler frames
(blocked by PARKED's "recursion through a handle body"); if step 3's front-end fix lands
first, add it to `MULTI_SHOT` and time the copy (it maps parents through a hash table, so
it should stay linear).

### 5. Native strings

`<>` is refused natively and there is no Int-to-String, so every native example prints a
number. Add the builtin (`int.to_string`, Claude's call) to the front end and evaluator,
then strings natively: concatenation in the runtime (GC-managed byte arrays), the builtin,
and `io.println` of a computed string. Measure first which examples and corpora this
unlocks (the CEK machine's output, step 2's replay log), and pin it with differential tests.

Also open, unscheduled: explicit wrapping arithmetic (`int.wrapping_mul` and friends, for
hashes and generators) and unary minus natively (Core refuses "Unary"); a two-parameter
lambda passed to a higher-order function is "unrepresentable" natively (N7 part 2?); the
one-shot trap's words differ (evaluator "continuation resumed more than once", native
"a one-shot continuation was resumed twice"); the parser continues a call across a
newline (`f()` then a line starting `(` is `f()(...)`) -- a layout rule is a language
decision; the roadmap's
other priorities (record/replay handlers, then capabilities once modules exist); the rest
of the soundness sweep (resume's row lacks return-clause and re-entered-clause effects;
recursion through a lambda the function handles around, PARKED); recursion through a
handle body is rejected by inference (PARKED). A perform walks the handler chain, so a program whose handler stack really grows
(a resume inside a fresh handle every iteration) pays O(depth) per perform; the evaluator
is no faster there. Also: the Elya CEK machine's next versions (a parser for its terms, a step
counter as an Elya effect); PIC/PIE linking (needs a Windows run), and a named diagnosis for
a native stack overflow in tests other than A9.
