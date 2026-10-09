# Handoff — where Elya is, and the next five steps

Written 2026-10-05 for whichever agent picks this up next. Read this, then `CLAUDE.md`,
then `PARKED.md`. The newest spec in `docs/superpowers/specs/` shows the house style.

## State at handoff (updated 2026-10-08)

- `main` has slices 5b-8, 5c-1..3, 5b-9a, the CEK example, the PolyForm Strict license and
  this file. **Open PR #15** lands the tail-in-match fix and slice 5b-9b on `main` (Windows
  CI green on its tip). Slice 5b-10 (this file's newest Done entry) is on branch
  `claude/slice-5b10-nested-handles`, stacked on #15's branch: merge #15 first, then
  retarget 5b-10's PR to `main`.
- History was rewritten on 2026-10-05 (the maintainer's request) so no commit carries a
  university address; every branch was force-pushed. Do not push old local branches.
- The repo is private until the maintainer flips it public (only they can).
- Parked, unmerged: `claude/outside-edit-tail-in-match` -- edits from another writer (not
  this session); superseded by `claude/tail-in-match`, kept until the maintainer says.
- Linux gate at the 5b-10 tip: 677 passed, 67 suites. `gc_mark` 637 bytes.

## How work is done here (non-negotiable)

1. **Measure first.** Run the real program through `elya check | run | build` and write
   the table into the spec before designing (see any 5c spec, §0).
2. **Language decisions belong to the maintainer.** Ask, offering options with a
   recommendation, then record the answer in the spec. Never decide the language silently.
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

## The next five steps

### 1. Front end: `resume` carries its handle's row (soundness; found by 5b-10)

PARKED, "`resume` is typed effect-free". A program that checks clean stops in the evaluator
with "unhandled effect `t` reached the machine": a lambda that resumes is typed pure, but
calling it runs the rest of the handled body. Under deep handlers `resume(v)` should carry
the handle's OUTER row (what the resumed computation may still perform). Measure first (the
PARKED program, m10 from the 5b-10 spec, every corpus that resumes inside a lambda), red
tests in `tests/` (a type error where today `check` is clean), then the fix in
`src/types.rs`. Expect m10 to compile natively afterwards and
`an_escaped_resume_of_a_leaking_handle_is_refused_by_name` to flip (an approved
expected-value change: say so in the commit). Any existing program the fix rejects is a
language decision: stop and ask the maintainer.

### 2. Evaluator: non-tail recursion under a handler is quadratic

PARKED: about 4× time per doubling (n=4000 takes 1.7 s; n=100 000 did not finish in 10
minutes). This caps every differential test's N. Profile (frame capture copies the
continuation?), predict the complexity, fix, and pin it with a timing-free test that
counts steps or allocations. The evaluator is the reference semantics, so the
differential corpora must stay green unchanged.

### 3. N7: runtime polymorphism (spec first, stop for decisions)

- Lifts the `Ty::Var` "unrepresentable type" refusal (5c-2's n4 hits it) and D16's
  conservative refusal of effect-polymorphic functions at user effects (5b-9a's s4).
- The roadmap direction is specialisation in core code plus dictionary passing above it,
  with `@specialize`/`@share` and an enforced code-size budget.
- Write the measured table and the open questions, then ask the maintainer before
  planning. This is a large arc; plan it as several slices.

### 4. Native multi-shot handlers (`with multi`)

Refused natively by name since 5b-8 (A2); the evaluator runs them. A multi-shot resume
re-runs a captured continuation, but native frames are consumed in place -- and since 5b-10
a resume also MUTATES its handler frame (`next`, `parent`): a second resume needs the frame
chain, handler frames included, COPIED first, and the one-shot word replaced by a
copy-on-resume rule. Measure first (the evaluator's multi-shot corpus, e.g.
`multi_shot_collects_both_branches`, through `build`), then a spec with the copy cost
stated and gated against A4's live-set instrument. Ask the maintainer whether multi-shot is
worth native support before N7.

### 5. Native strings: `<>` and an Int-to-String builtin

`<>` (string concatenation) runs in the evaluator but is refused natively by name (5b-8
§9.1), and there is no way to turn an `Int` into a `String` at all -- which is why the
Elya CEK example prints fixed strings instead of its results. Measure first (every
corpus and example that uses `<>`, through `build`). The runtime needs a concatenation
that allocates a fresh string block (one descriptor row already exists for strings) and
an integer formatter. The builtin's NAME and module (`int.to_string`? `show`?) is a
language decision: ask the maintainer, offering options. Then let the CEK example print
its answers.

Also open, unscheduled: two front-end findings from 5b-10 (PARKED): recursion through a
handle body is rejected by inference, and a handle with clauses for only some of its
effect's ops checks clean (a language question: error, or forward and put the rest in the
row). A perform walks the handler chain, so a program whose handler stack really grows
(a resume inside a fresh handle every iteration) pays O(depth) per perform; the evaluator
is no faster there. Also: the Elya CEK machine's next versions (a parser for its terms, a step
counter as an Elya effect); PIC/PIE linking (needs a Windows run), and a named diagnosis for
a native stack overflow in tests other than A9.
