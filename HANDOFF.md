# Handoff — where Elya is, and the next five steps

Written 2026-10-05 for whichever agent picks this up next. Read this, then `CLAUDE.md`,
then `PARKED.md`. The newest spec in `docs/superpowers/specs/` shows the house style.

## State at handoff

- `main` (`66fb01b`) has slice 5b-8 (native effect handlers), 5c-1 (handler clause
  resolution) and a cleanup.
- Open PRs, stacked, merge **in order**:
  - **#5** slice 5c-2 (E0204 one clause per op, E0205 no fn named like an op, locals
    shadow ops), base `main`.
  - **#6** slice 5c-3 (E0206 an op may only be called), base is #5's branch.
    **Retarget #6 to `main` after #5 merges**, or it merges into the wrong branch.
- Linux gate at the tip of #6: 647 passed, 63 suites. Windows CI green through #5; #6's
  run starts on push.
- Diagnostics added in 5c: E0202 duplicate op name, E0203 clause names no declared op
  (or has the wrong arity), E0204, E0205, E0206.

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

## The next five steps

### 1. Land the stack and confirm `main`

Get #5 merged, then #6 retargeted to `main` and merged. The maintainer does both. Then
fetch `main`, run the gate on it (expect 647), and ask the maintainer to run
`scripts/check.ps1` locally once: CI uses the LLVM 18.1.8 release archive, not their vcpkg
LLVM. Update `README.md`'s status section for 5b-8 and 5c-1..3. *Done when* `main` is
green on both and the README is current.

### 2. Slice 5b-9: native effectful closures

Natively refused today: "effectful lambda", "effectful closure call", and effectful calls
inside a `match`, all "(not yet compiled natively)" (PARKED, Task 8 follow-ups).

- This is the biggest check/run/build split left. 5c-3's own fix-it (`fn() { ping() }`)
  runs but does not build.
- Approach: give lambdas the CPS convention when their row names a user effect (the
  selective-CPS key, D16), with closure calls through `$cont`.
- Keep `gc_mark` byte-identical (637 bytes, measured with the `awk` command in PARKED/A7).
- Watch MAX_PARAMS (5) on win64.
- Measure each refused shape first and record which stay refused.

### 3. Slice 5b-10: lift D17 (nested handles, handles inside effectful code)

PARKED, "`elya_current_handler` depends on today's handle refusals":

- A perform must switch the global to the handler *outside* the clause's handle before
  jumping to the clause.
- The tail-resume install in `clause_tail` becomes load-bearing.
- First write a red test with **two live handlers where the wrong one is observable**.
  Today no test isolates the install (Task 12 control 1a), and A6 is not a witness in
  this design.

### 4. Evaluator: non-tail recursion under a handler is quadratic

PARKED: about 4× time per doubling (n=4000 takes 1.7 s; n=100 000 did not finish in 10
minutes). This caps every differential test's N. Profile (frame capture copies the
continuation?), predict the complexity, fix, and pin it with a timing-free test that
counts steps or allocations. The evaluator is the reference semantics, so the
differential corpora must stay green unchanged.

### 5. N7: runtime polymorphism (spec first, stop for decisions)

- Lifts the `Ty::Var` "unrepresentable type" refusal (5c-2's n4 hits it) and D16's
  conservative refusal of effect-polymorphic functions at user effects.
- The roadmap direction is specialisation in core code plus dictionary passing above it,
  with `@specialize`/`@share` and an enforced code-size budget.
- Write the measured table and the open questions, then ask the maintainer before
  planning. This is a large arc; plan it as several slices.

Also open, unscheduled: native multi-shot (`with multi` is refused natively),
PIC/PIE linking (needs a Windows run), and a named diagnosis for a native stack overflow
in tests other than A9.
