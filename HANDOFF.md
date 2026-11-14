# Handoff — where Elya is, and the next five steps

Written 2026-10-05 for whichever agent picks this up next. Read this, then `CLAUDE.md`,
then `PARKED.md`. The newest spec in `docs/superpowers/specs/` shows the house style.

## State at handoff (updated 2026-11-14, an interactive session on `main`)

- **Work lands on `main` directly** (the maintainer, 2026-10-30: "Commit in the main
  tho"). `main` has everything: #24 (2026-10-24), then `auto/elya`'s 15 commits since,
  merged at 731ce70 (strings by byte through row variables part 1; PR #25 shows them
  merged). `auto/elya` stays at 68fdc9f, not deleted; nothing new goes there.
- Scheduled auto-runs and interactive sessions commit to `main` after a green gate
  and log in `AUTO_RUN_LOG.md`; the maintainer reviews on GitHub.
- History was rewritten on 2026-10-05 (the maintainer's request) so no commit carries a
  former e-mail address; every branch was force-pushed. Do not push old local branches.
- The repo is public (seen 2026-10-30; the maintainer flipped it).
- **The maintainer's working copy is on their PC, `E:\Elya\Elya-project\Programming-language-with-a-toolchain`**
  (2026-11-05: "keep working on the project in E: from now on"; working notes
  beside it). A cloud session cannot open that drive in a shell, only write
  files to it, and not into `.git`: it builds and gates in its own workspace,
  pushes to GitHub, and at each checkpoint writes the changed files into that
  clone's working tree. `E:\Elya\elya-sync.bat` (fetch, set back a file
  that already holds what `origin/main` holds, fast-forward `main`; log
  `E:\Elya\elya-sync.log`) brings its history up; `E:\Elya\elya-test.bat`
  runs the gate there (log `E:\Elya\elya-test.log`). Neither has run yet.
  The PC's linked shell (a Linux VM, 2026-11-12) cannot mount `E:` either, and
  cannot reach rust-lang.org: a session puts Ubuntu's `rustc-1.91` and
  `cargo-1.91` (`apt-get download`, `dpkg -x`, no root) under its home and
  clones from GitHub there; the root crate builds and passes (no LLVM 18 in
  that Ubuntu, so not the codegen suite). The VM's home goes with the session.
- Parked, unmerged: `claude/outside-edit-tail-in-match` -- edits from another writer (not
  this session); superseded by `claude/tail-in-match`, kept until the maintainer says.
- Linux gate at the tip of `main`: 1533 passed, 164 suites, and configuration C
  (the boards on QEMU; `check.sh` fails without its tools unless
  `ELYA_BOARDS=skip`) 5 passed, 1 suite. **Look at the Windows
  gate after every push** (`gh run list`): it was red from cf3f028 to 145b662
  unseen (a checker frame of 16 KiB overflowing a 2 MiB test thread; fixed in
  12f91bd, spec 2026-11-13-elya-front-end-frames). Native programs run on
  their own 1 GiB stack (`elya_start`); the CLI compiles on a 1 GiB thread too
  (since 2026-10-24). In October the runtime
  gained `elya_cont_copy` (multi-shot), the string functions and the two time
  functions, and a handle's body ends at the shared end frame (`gc_collect` grays the
  current handler as a root). Since 2026-11-12 the collector is `gc.c` (paged, freestanding;
  spec 2026-11-12-elya-paged-collector), the runtime compiled `-O2` once per process.
  Since 2026-11-13 `runtime.c` is freestanding too: the machine only through
  `elya_platform_*` hooks (`platform_hosted.c` on a host; a board gives `write` and
  `exit`), Float text by `float_fmt.c` / `float_parse.c`, no C library function in
  the IR (spec 2026-11-13-elya-platform-layer). Since fe2639d native code is
  optimised (`default<O2>`; spec 2026-11-13-elya-optimised-code; the Windows
  gate was red from it until fad7056). Since 52bf757 and 6620efb an Elya program runs on
  a Cortex-M3 and a 32-bit RISC-V board under QEMU (`compile_module_for` with
  `Machine::cortex_m3()` or `Machine::rv32imac()`, the `boards` feature,
  `crates/codegen/boards/`; specs 2026-11-13-elya-boards,
  2026-11-14-elya-riscv-board).
- **Work is divided with ChatGPT** (the maintainer, 2026-11-13): Claude gives the
  ChatGPT (Codex) app on the PC self-contained jobs in `E:\Elya\chatgpt-jobs\<job>`,
  never the repository itself, picks its reasoning effort by the job's difficulty,
  and checks every result here before it lands (the spec names what was checked).
  The PC has no LLVM 18 and no QEMU: its jobs cannot build the native back end.
  When the PC's link is down, or Codex is out of credit (the maintainer,
  2026-11-14: "switch to chat"), ChatGPT on the web (the maintainer's browser, the
  Claude extension) can read the public repository and review; it cannot write
  files. Every job at the most effort offered (the maintainer, 2026-11-14: GPT-6,
  thinking effort High on that account); the easier parts to it, the harder
  here, and its answers checked against the source (spec 2026-11-13-elya-boards
  section 3 shows why: two of its research claims were wrong).
  `elya_eq` takes a fourth argument (the first closure tag) since 2026-10-23.
- Interactive sessions use the same `AUTO_RUN_LOG.md` lock: an IN PROGRESS entry
  under 3 hours old makes a scheduled run exit.
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
      `main` (since 2026-10-30) and `claude/**`.
    - The Actions logs API is blocked from the cloud session. Read the results with
      `gh api repos/Elvineej/Programming-language-with-a-toolchain/commits/<sha>/check-runs`,
      then that run's `/annotations` (a `notice` carries `passed=N failed=M`; failures
      carry the log tail).
11. **Commit to `main`, gated** (the maintainer, 2026-10-30). Push `main` only with
    `sh scripts/check.sh` green at the predicted count, as a fast-forward (never a
    force-push). The agent still cannot mark PRs ready or merge them through GitHub
    (blocked by the permission system); a branch reaches `main` by a local merge and a
    push, as `auto/elya` did at 731ce70.
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

- **Nested and literal patterns natively** (2026-10-10, branch `auto/elya`, a prerequisite
  of the replay step): spec `docs/superpowers/specs/2026-10-10-elya-native-pattern-compilation-design.md`.
  A Core-to-Core pass (`crates/codegen/src/patterns.rs`) compiles them to flat matches
  (test, then destructure: linear size). The first version copied failure continuations
  and the review measured exponential code size; fixed test-first.

- **Exact replay and handler-based testing** (2026-10-10, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-10-elya-replay-design.md`. `examples/05_replay.elya`:
  `record`, `replay` (stops by name on the first divergence, rejects leftovers) and a test
  double, as ordinary handlers over one program; the same bytes natively.

- **N7 part 2: type variables and parametric ADTs natively** (2026-10-15, branch
  `auto/elya`): spec `docs/superpowers/specs/2026-10-15-elya-n7b-type-monomorphization-design.md`.
  Whole-program monomorphization (`crates/codegen/src/mono.rs`): generic functions and
  generic `let` values are cloned per instantiation (originals not emitted), ambiguous
  variables default to `Unit`, and parametric types become ground instances built
  lazily (non-regular types work) AFTER part 1's rows. A variable only in a function's
  own row is refused by name (defaulting it was measured unsound: a crash). The review
  found closures inside parametric types miscompiled (row tails in instance keys) and a
  phantom-effect-parameter regression; both fixed test-first.

- **Recursion through a handle body** (2026-10-15, branch `auto/elya`, a prerequisite of
  steps 1 and 2): spec `docs/superpowers/specs/2026-10-15-elya-handle-recursion-design.md`.
  A recursive call directly in a handle body includes the member's row (labels, with
  their arguments unified) instead of unifying with it, so the handled effect no longer
  leaks into the function's type. Natively a chain of 1000 nested handlers runs. The
  review found two soundness holes in the first version (effect arguments; an escaped
  `resume`), fixed test-first; a function annotated with the formerly leaking effect is
  now E0423 (the exact-row rule). Extended the same day to recursion through a LAMBDA
  (a lambda including a member closes with the group): `spawn(fn() { fib(n - 1) })`
  checks, and row_soundness's `go` under its handler is accepted (64).

- **Async step 2, part 1: structured concurrency** (2026-10-15, branch `auto/elya`):
  spec `docs/superpowers/specs/2026-10-15-elya-async-step2-structured-design.md`.
  `examples/06_structured.elya`: `spawn`/`await`/`yld` and a `scope` that joins every
  task started in it, as ordinary handlers over the generic `List`/`Pair`; a task that
  spawns itself and a scope per call (`pfib`). The same bytes natively.

- **A backtracking example, and multi-shot copies of deep handler chains** (2026-10-15,
  branch `auto/elya`): spec `docs/superpowers/specs/2026-10-15-elya-search-and-deep-chains-design.md`.
  `examples/07_search.elya`: N-queens over a `multi` `Choose`, three handlers (all
  solutions, a count, the first), the same bytes natively. A multi-shot copy of a chain
  of 100 000 handler frames runs natively in linear time.

- **An N7 part 2 miscompile, found by a differential sweep** (2026-10-15, d07afea, an
  interactive session).
  - **Symptom:** an `IO` closure inside a parametric value given to a generic function
    (`call(B(fn() { io.println("x")  42 }))`) stopped natively with "match failed".
  - **Cause:** the instance key kept row labels, but `IO` is a label that doesn't change
    the calling convention.
  - **Fix:** an instance key renders a row as its convention alone.

- **Native strings** (2026-10-15, cc044d4 and cf6b08b, an interactive session; old step
  2). Spec `docs/superpowers/specs/2026-10-15-elya-native-strings-design.md`.
  - **Natively:** `<>`, `==`/`!=` on strings, and string-literal patterns.
  - **New builtin:** `int.to_string`, which is pure (decision D1).
  - **Unlocked:** 17 programs that were refused before now compile and match the
    evaluator.
  - **The collector:** concatenation is an allocation site. The left operand is rooted
    while the right one runs, and in a resumption function `root_live` keeps the
    continuation alive (the resumed frame is dead).
  - **E0208:** a builtin used as a value is now an error (D6). It used to pass `check`
    and fail at run time.
  - **Review:** no correctness bug. It showed that two of my controls were blind; both
    are pinned by tests now.
  - **Order:** this step was taken before the poll loop (now step 1) because that
    example has to print times and task ids.

- **Async step 2, part 2: waiting for time, and a poll loop** (2026-10-15/16, e84c105,
  7c951d2, 1d9f565, 61a84f9; old step 1). Spec
  `docs/superpowers/specs/2026-10-15-elya-async-step2-timers-design.md`.
  - **Builtins:** `time.now_ms()` and `time.sleep_ms(ms)`, both `IO`, in both evaluator
    paths and natively.
  - **`examples/08_timers.elya`:** `sleep` parks a task on a timer list; every turn
    wakes an expired timer; when every task is stuck the scheduler performs
    `Clock.wait_until` on the earliest deadline, and with none left names a deadlock.
    `virtual_clock` (exact, instant) and `real_clock` run the same scheduler.
  - **Review:** no correctness bug; starvation of an expired timer, an early-returning
    Windows `Sleep` and a timing-dependent value were fixed test-first.
  - **Found:** an effectful function of five parameters is refused natively (new step 5).

- **Operation-generic handlers, `with any`: a recorder for any effect** (2026-10-16,
  fe8b10c..6798543; old step 1). Spec
  `docs/superpowers/specs/2026-10-16-elya-any-handlers-design.md`, which includes a
  survey: Koka, Effekt, Frank, Eff, Links and Unison name their effects, and OCaml 5's
  generic `effc` is untyped.
  - `handle e with any { op(o) -> ... }` handles everything `e` performs but `IO`.
    `o : Op(t / H)` has a rigid answer type `t` per clause (E0435 when it escapes).
  - Builtins `op.name/show/perform/encode/decode`; Elya literal syntax is the
    encoding.
  - E0434: the handled effects must be data. A forwarding handler's tail is checked
    at run time (D5 revised, measured).
  - `examples/09_record_any.elya`: one `record`/`replay` for a dice game and a shop.
  - Evaluator only. Natively it is refused by name.
  - Two reviews found 4 soundness holes and fixed them test-first. One of them (an
    escaped resume dropping a clause's relayed row) was pre-existing for plain
    handlers too.

- **A function's own row in Core** (2026-10-16, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-16-elya-own-row-in-core-design.md`. The typed table
  records each function's whole type at its declaration's span; `CoreFn::row` carries
  it; N7 part 2 matches it at references, so a type variable generic only through a
  function's own row (`fn dup() { put(get()) }` over `St(s)`) is instantiated natively
  instead of refused. The node-count tripwire moved 5 -> 6 and the `typed_inference`
  snapshots gained one line per declaration (decided, recorded).

- **Failure as an effect, and scope cancellation** (2026-10-16, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-16-elya-failure-and-cancellation-design.md`. An empty
  `match e { }` is exhaustive exactly on a type with no constructors, and has any type;
  so `type Never { }`, `fail : String -> Never` and a library `raise`. A name declared
  twice (function, type, constructor, effect) is now E0209. `examples/10_failure.elya`:
  `catch`, and scopes that drop their queued tasks when one fails; the same bytes natively.

- **Computed callees and functions as values, natively** (2026-10-16, branch
  `auto/elya`): spec `docs/superpowers/specs/2026-10-16-elya-callees-and-function-values-design.md`.
  A back-end pass (`anf.rs`) names computed callees (evaluated first, as in the
  evaluator) and wraps top-level functions used as values in capture-free closures,
  which N7 then specializes; the two most common native refusals of the 2026-10-15 sweep
  are gone.

- **Argument records past three parameters** (2026-10-16, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-16-elya-argument-records-design.md`. Every function,
  lambda, type, call, operation and clause with more than three parameters keeps two
  and passes the rest in one synthesized record (`spill.rs`), so no kind can exceed its
  win64 tail-call cap; every arity refusal is gone. Needs the Windows gate.

- **Record and replay from the command line** (2026-10-16, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-16-elya-cli-record-replay-design.md`. `elya run
  --record T prog` writes the host's answers (`time.*`) as a text trace, also for a
  failing run; `--replay T` reproduces the run from it, stopping by name at a divergence
  or on unread entries. Evaluator only.

- **Deep non-tail recursion natively** (2026-10-16, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-16-elya-native-stack-design.md`. The program runs on
  its own stack (`elya_start` in runtime.c: 1 GiB reserved, a guard below it, a thread
  that owns it; `ELYA_STACK_MIB` overrides), so `len` over 1 000 000 and a non-tail
  resume over 1 000 000 run natively; overflowing it is the named `elya: native stack
  overflow: ...`, exit 1. Elya functions are now mangled `elya.<name>` (a function named
  `alloc`, `println` or `start` linked against the runtime's own symbol). Needs the
  Windows gate.

- **Structural equality** (2026-10-16, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-16-elya-structural-equality-design.md`. `==`/`!=` are
  defined on data only; comparing a function is the new E0436 at check time, through
  equality type variables (a compared variable is marked and checked where it is bound or
  instantiated). Equality is structural and iterative in the evaluator (long lists no
  longer overflow the host stack) and natively (`elya_eq`, a worklist over the
  descriptor table); `Unit` and every declared type compare natively.

- **A repeatable differential sweep** (2026-10-16, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-16-elya-differential-sweep-design.md`.
  `scripts/sweep.py` runs every program in the repository through `elya run --value`
  (new: main's value as the native shim prints it) and natively, and classifies the
  results; `elya run` now keeps output printed before a runtime error. At 9bb6f5d: 409
  checked programs, 382 byte-identical, no disagreement or crash; `with any` is the top
  native gap (9 programs). Found and fixed: `elya build` panicked on a warning-only
  program.

- **`with any` natively** (2026-10-17, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-17-elya-native-any-handlers-design.md`. After row
  specialization a `with any` handle is expanded (`any.rs`) into a plain one with a
  clause per handled operation, the clause copied at each answer type (generic helpers
  called at it cloned too); `op.*` and `o == o` are rewritten over the clause's
  parameters, so no operation value exists at run time (any other use of `o` is
  refused by name). Literals are written and read by the runtime through a reflection
  table, iteratively, with the evaluator's words for every failure.
  `examples/09_record_any.elya` runs natively. Needs the Windows gate.

- **Unary operators natively; wrapping and bit arithmetic** (2026-10-17, branch
  `auto/elya`): spec `docs/superpowers/specs/2026-10-17-elya-unary-and-wrapping-design.md`.
  `-e` and `!e` lower in Core as `0 - e` and `e == False`. New pure builtins
  `int.wrapping_add/sub/mul`, `int.and/or/xor`, `int.shl/shr/ushr` (a shift amount
  outside `0..=63` fails by name), in both evaluators and natively. Needs the Windows
  gate.

- **Record and replay natively** (2026-10-17, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-17-elya-native-record-replay-design.md`. A native
  binary records under `ELYA_RECORD=<file>` and replays under `ELYA_REPLAY=<file>`
  (runtime.c), in the evaluator's trace format with its failures' words, so a trace
  moves between `elya run --record/--replay` and native runs in both directions.
  Needs the Windows gate.

- **Multi-shot resumes share the captured chain** (2026-10-22, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-22-elya-multi-shot-sharing-design.md`. A handle's body
  now continues to one shared end frame (`elya_rt_handle_end` continues in the current
  handler), so no site frame points at a handler frame. A multi-shot resume shares the
  whole chain and copies only the handler frames on the perform's path, and the current
  handler is a collector root. PARKED's `nest(20000)` (out of memory) runs in 0.01 s;
  100 000 deep in 0.07 s (the evaluator 1.2 s). `copies_survive_a_collection_mid_copy`
  moved from m11 to a new row m13 (m11 no longer collects). Needs the Windows gate.

- **One wording for the one-shot trap; constructors and aliases as values natively**
  (2026-10-22, branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-22-elya-one-shot-words-and-sweep-gaps-design.md`.
  - A second resume of a one-shot continuation is "a one-shot continuation was
    resumed twice" on both sides. The evaluator's label is corrected: a `with multi`
    handler of an `effect multi`.
  - `anf` wraps a constructor used as a value in a closure. Natively a monomorphic
    one SEGFAULTED (`ap(Wrap, 5)`), and a parametric one was refused.
  - `anf` also substitutes an alias (`let mk2 = mk`) of a row-polymorphic local, or of
    a `with any` operation. So those aliases now run.
    - Not substituted: an alias of a top-level function, of a name a top-level function
      shares, or of a closed-row type, and any alias whose body rebinds the target.
    - The review found each of those exclusions by a wrong value (spec §3).
  - A user effect named `IO` is E0209 at resolve.
  - Changed expectations:
    - the native trap's words in three tests;
    - the first clause of `an_operation_value_used_as_a_value_is_refused`, a bare
      alias, which now runs;
    - the program of `an_alias_of_a_generic_local_used_at_a_user_effect_is_refused_by_name`,
      which now rebinds the alias's target so the guard stays pinned.
  - Needs the Windows gate.

- **Lambdas capture by scope; a wrapped function's result is coerced** (2026-10-23,
  branch `auto/elya`): spec
  `docs/superpowers/specs/2026-10-23-elya-scope-captures-and-wrapper-results-design.md`.
  Two native wrong values the previous slice's review found, both older than it:
  - Closure conversion decides a lambda's captures by the scope around it. Before,
    a local that shadows a top-level function the same function also calls was
    dropped: the top-level function ran (2004 for 1007), or the compiler panicked.
  - `anf`'s wrapper of a top-level function calls it at its own result and lets
    `specialize` coerce an upcast result: 5 for 15 before, and a pointer for two
    layers.
  - The review's fuzzers on the same seeds went from 7 build failures and 97 refusals
    to none and 18.
  - Needs the Windows gate.

- **Operation values as values natively** (2026-10-23, branch `auto/elya`, a
  scheduled run): spec
  `docs/superpowers/specs/2026-10-23-elya-operation-values-natively-design.md`.
  - A `with any` clause's `o` is a value of a synthesized type per operation instance
    (`$Op.E.op.N`, its arguments as fields): it can be given to helpers, stored,
    captured, returned and compared natively.
  - A function taking the operation is cloned per operation; one seeing only the
    answer type, per answer type. `op.*` and `==` are rewritten by type everywhere.
  - The review: `==` on data around an operation reached a closure in `elya_eq` (a
    wrong `eq`); `elya_eq` now stops at one with the evaluator's words, in the
    evaluator's order. Two regressions of the first version (clone budget, a nested
    recorder at an outer answer type) fixed.
  - Changed expectations: the `any.rs` refusal unit test and half of
    `what_is_not_compiled_is_refused_by_name` (they run now).
  - Needs the Windows gate (`runtime.c`).

- **Literals at the edges, and a layout rule** (2026-10-23, branch `auto/elya`, a
  scheduled run, 5058cda): spec
  `docs/superpowers/specs/2026-10-23-elya-literals-and-layout-design.md`.
  - `-` directly before a numeric literal is part of it: `-9223372036854775808` is
    MIN; an out-of-range integer literal is E0002; negative literals are patterns.
  - In braces a newline before an infix operator or a call's `(` ends the
    expression: a glued `-x` or `(` starts a statement, any other operator starting
    a line is E0101; in parentheses newlines are whitespace.
  - The review: a doubled diagnostic after a bad `let`, a `match` scrutinee inside
    parentheses, `-` in handler clauses, the float fold's stated reason; all fixed.
  - Parser only: the Windows gate needs nothing special.

- **A failing traced run keeps its output on stdout** (2026-10-23, branch
  `auto/elya`, a scheduled run, 8a72c45): spec
  `docs/superpowers/specs/2026-10-23-elya-traced-run-output-design.md`.
  `elya run --record|--replay` print the program's output on stdout and the error on
  stderr, as `elya run` and native binaries do (`record_source`, `replay_source`);
  a differential test against the native binary in `build_cli.rs`.

- **Modules, part 1: a program in more than one file** (2026-10-23, branch
  `auto/elya`, a scheduled run, c7bb82b): spec
  `docs/superpowers/specs/2026-10-23-elya-modules-part-1-design.md`.
  - `import a/b [as c]` is `a/b.elya` next to the program; functions cross files only
    qualified (`b.f`, `pub` exported); types, constructors, effects and operations
    stay unqualified, program-unique, visible where imported. E0600-E0606.
  - One merged module (an imported `f` is `a.b.f`); spans per file; the CLI reads the
    files; `*_with` library entry points take a reader.
  - `import a/b / {E}` is reserved syntax, E0606 until part 2.
  - Needs the Windows gate (symbols with `.` from module paths).

- **`Float` natively, and `float.to_string`** (2026-10-23, branch `auto/elya`, a
  scheduled run, 6baf595): spec
  `docs/superpowers/specs/2026-10-23-elya-float-natively-design.md`.
  - A float is its bits in an untraced word; IEEE arithmetic and equality, also
    inside data (a float mask per tag, `elya_eq_init`) and in `o == o`.
  - `elya_float_text` writes the evaluator's text (shortest, closest, ties half up,
    no exponent), proven against Rust on 200,000 random values; decode by `strtod`.
  - `float.to_string` on both sides.
  - Needs the Windows gate (`runtime.c`; `%.780e` exactness and `strtod` on UCRT).

- **Decoding at an effect's own parameter: decided, the refusal stays** (2026-10-23,
  a scheduled run): spec
  `docs/superpowers/specs/2026-10-23-elya-decode-at-a-parameter-design.md`. The
  evaluator has no instance inside effect-polymorphic code and native clauses are
  per instance, not per site, so decoding at the instance cannot agree on both
  sides; a static check would be partial. Runtime type witnesses are the recorded
  way forward, with modules part 2.

- **Upcasts through data, natively** (2026-10-23, branch `auto/elya`, scheduled
  runs): spec `docs/superpowers/specs/2026-10-23-elya-row-tails-in-data-design.md`.
  - A pure closure inside data used where data of an effectful one is expected:
    natively a pointer or "match failed" before (five measured programs).
  - D1: `anf`'s wrapper binds the own result's free row variables from the use.
  - D2: `specialize` rebuilds the data (`$coerce.T.k`, one pure function per
    pair of instances, a match per constructor, fields coerced).
  - D3: `conv_eq` compares data names, so the guard names what D2 misses.
  - D4 (the review's F1): a non-regular type is refused by name (the first
    version overflowed the compiler's stack).
  - Needs nothing special on Windows (Core passes only).

- **A wrapped function's row shared with a parameter** (2026-10-23, branch
  `auto/elya`, a scheduled run, 4a0aeb1): spec
  `docs/superpowers/specs/2026-10-23-elya-wrapper-parameter-rows-design.md`.
  `anf`'s wrapper binds a parameter's row variables from the use's parameters,
  and the result's own parameter positions before its results; six measured
  programs and two from the review printed a pointer natively. Core pass only.

- **Parser error recovery inside a body** (2026-10-23, branch `auto/elya`, a
  scheduled run, f4777d0): spec
  `docs/superpowers/specs/2026-10-23-elya-parser-recovery-design.md`. A failed
  statement is `Expr::Error` and parsing resumes at the next one (a bad `let`
  keeps its name); a function whose header fails stays a known name; the end of
  the file inside a declaration is an error (a truncated declaration used to
  pass `check`); `fn` starts a declaration only before a name. One mistake is
  one diagnostic: no E0200 cascade, later real errors still reported.

- **Modules, part 2: the effect allow-list at the import** (2026-10-24, branch
  `auto/elya`, a scheduled run, 48d5c8f): spec
  `docs/superpowers/specs/2026-10-24-elya-modules-part-2-design.md`.
  `import a/b / {IO, Net}` grants exactly that; no list grants nothing. A
  module's capability is the effect labels at positive positions of its `pub`
  functions' inferred types, through data fields, label arguments and operation
  signatures (the review found three holes there). E0606 (not granted), E0607
  (unknown or repeated name), E0608 (a list on a standard module).

- **`Float`, part 2: comparisons, negation and conversions** (2026-10-24, branch
  `auto/elya`, a scheduled run, 21a622e): spec
  `docs/superpowers/specs/2026-10-24-elya-float-part-2-design.md`. `<. <=. >. >=.`
  (IEEE ordered), `-.e`, `float.of_int`, `float.truncate`, `float.round` (failing
  by name on NaN and outside `Int`), both sides. Needs the Windows gate
  (`runtime.c`'s `elya_float_to_int`).

- **Record and replay as an Elya library** (2026-10-24, branch `auto/elya`, a
  scheduled run, c5d618a): spec
  `docs/superpowers/specs/2026-10-24-elya-std-replay-design.md`. `import
  elya/replay` is a standard module written in Elya (embedded `src/std/`):
  `record`, `replay`, `text`, `size`; it needs no grant and runs natively.

- **Aliases that rebind their target are renamed** (2026-10-24, branch
  `auto/elya`, a scheduled run, b7a176b): spec
  `docs/superpowers/specs/2026-10-24-elya-alias-renaming-design.md`. `anf`
  substitutes an alias of a row-polymorphic local with capture-avoiding
  renaming of the body's own binders of the target, also when a top-level
  function shares its name; the sweep's "effect-polymorphic function used at a
  user effect" refusals are gone. Core pass only.

- **Diagnostics at the edges** (2026-10-24, branch `auto/elya`, a scheduled
  run, 0f8da88): spec
  `docs/superpowers/specs/2026-10-24-elya-diagnostics-at-the-edges-design.md`.
  Byte offsets in the renderer; E0209/E0202 label both declarations; the end
  of the file is reported once; expressions nest at most 1024 levels (E0102,
  chains counted by height); the CLI compiles on its own 1 GiB stack (a
  700-line `main` overflowed a debug `elya build`).

- **Capabilities, part 3: attenuation in the standard library** (2026-10-24,
  branch `auto/elya`, a scheduled run, b70707f): spec
  `docs/superpowers/specs/2026-10-24-elya-capabilities-part-3-design.md`.
  `elya/console` (`Console`, `console_line`; `prefixed`, `limited`, `counted`,
  `captured`, `silenced`) and `elya/clock` (`WallClock`, `wall_clock_ms`;
  `fixed`, `stepping`, `shifted`), written in Elya: a dependency is granted
  these instead of `IO`. `elya/host` (`console`, `clock`) turns them into `IO`
  and needs `/ {IO}`. Builtins still need no import in a module (D5, for the
  maintainer's review).

- **Block tails and float exponents** (2026-10-24, branch `auto/elya`, a
  scheduled run, 3ccab88): spec
  `docs/superpowers/specs/2026-10-24-elya-block-tails-and-exponents-design.md`.
  A block ending in `let`, or empty, is E0103 (it was `Unit` in the evaluator
  and refused natively); float literals take an exponent (`1.0e19`), `1e19` is
  E0003, out of range is E0002. `float.to_string` unchanged.

- **Strings by byte, one trace line format, and `replay.parse`** (2026-10-30, branch
  `auto/elya`): spec `docs/superpowers/specs/2026-10-24-elya-trace-text-design.md`.
  - `string.length`, `string.byte_at` and `string.slice` index a string by byte. They
    are pure, each failure is named, and a slice may not split a UTF-8 character.
    They work on both evaluators and both native emitters.
  - Both host readers split a trace line at the first ` = ` outside a string
    literal; `read_trace` is the host's reader as a function.
  - `replay.parse(text)` is written in Elya and reads exactly as the host does
    (checked on 1 041 short traces).
  - The evaluator's strings are shared (`Rc<str>`), so a byte loop is linear.
  - Needs the Windows gate.

- **What `main` is worth, for every data type** (2026-10-30, branch `auto/elya`,
  fa24c18): spec `docs/superpowers/specs/2026-10-30-elya-main-values-design.md`.
  The sweep's last native refusals.
  - `elya run --value` and a native binary end with main's value as its literal
    (`True`, `1.5`, `Some(3)`); `Int` and `Unit` (`0`) are unchanged. A value that
    holds a function fails by name on both sides, after the output.
  - A generic `main` is instantiated at its defaults (natively it was "no `main`").
  - The two instance-budget refusals stay: both decode at a non-regular type.
  - From the review: a constructor used as a function is "a function" in the
    evaluator too, also in `op.encode` and `op.show`; natively the value line comes
    after a replay's unread-entry check.
  - Needs the Windows gate.

- **A block's length is not a stack bound** (2026-10-30, branch `auto/elya`,
  17869d4): spec `docs/superpowers/specs/2026-10-30-elya-long-blocks-design.md`.
  - Every codegen pass walks a block's `let`s with a loop (`core::let_spine`), in
    the recursion's order of effects; Core and the evaluator's environment drop
    through a heap stack.
  - `elya build` of 100 000 statements: 18.6 s (was a stack overflow). Long blocks
    in every place a block can be build on a 512 KiB thread.
  - From the review: `spill` had kept a `let`'s type unspilled (a function of four
    from a computed callee was refused); the CPS sites' saved sets were quadratic
    on a long block (names looked up from the oldest binding).
  - Nesting depth stays bounded; the parser refuses past 1024 levels by name.
  - Needs the Windows gate.

- **A name clash is reported once, where it was made** (2026-10-30, branch
  `auto/elya`, d868904): spec `docs/superpowers/specs/2026-10-30-elya-name-clashes-design.md`.
  - A module's reference to an operation means the operation, whatever the root
    declares: a root `fn console_line` is its one E0205 (it also made three E0200s
    inside `elya/console.elya`).
  - An effect declared twice is E0209 alone (its handlers made five E0207s).
  - E0605 is never about a name the file, or a file it imports, declares.
  - Types, constructors, effects and operations stay one name space (their names
    are data: traces, `op.show`).

- **The occurs checks look inside effect rows** (2026-10-30, branch `auto/elya`,
  2736024): spec `docs/superpowers/specs/2026-10-30-elya-occurs-in-rows-design.md`.
  Found measuring step 1: three-line programs crashed `elya check` with a stack
  overflow (a type or row variable bound inside its own effect label's argument).
  Now E0401 or E0424.

- **Row variables you can write in a signature** (2026-10-30, branch `auto/elya`,
  394a0b8; step 1's part 1): spec `docs/superpowers/specs/2026-10-30-elya-row-variables-design.md`.
  - `fn run(f: fn() / {e} -> Int) / {e} -> Int` and `/ {Log | e}`, written as the
    checker prints a row (a new `|` token); one name is one row for its whole
    function, flexible like a lowercase type name.
  - From the review: an older row-unification bug dropped a label when two rows
    ended in one variable (a clean-checking program failed at run time), fixed;
    named declared rows are unified per member before the ambient closes.

- **Row parameters on declarations** (2026-10-30, on `main`, 499c048; step 1's
  part 2): spec `docs/superpowers/specs/2026-10-30-elya-row-variables-design.md`
  section 3.
  - A declaration's parameter used where a row goes is a row parameter (found by
    a fixpoint over the declarations): `type Task(e) { Paused(fn() / {e} -> Task(e)) }`,
    `effect Async(e) { fn spawn(f: fn() / {Async | e} -> Int) -> Int }` (its own
    bare name is `Async(e)` there). A use writes a row, `Task({Log})`, or a row
    variable, `Task(e)`; a label's written arguments (`{State(Int)}`) are now
    checked (they were dropped).
  - Encoding: a row argument is a phantom `fn() / R -> $Row`; natively, instances
    key by convention and the instance builder substitutes rows first.
  - A scheduler is written once: a task that spawns has a finite type (it was
    E0401), on both back ends; `examples/04_async.elya`'s round robin, written
    generically, prints the same (`tests/row_parameters/round_robin.elya`).
  - Found on the way: a call's row whose labels mention its own tail is unified
    whole (pouring it label by label made `e` contain `Async(e)`, E0424), and the
    specializer's pattern binder types now substitute rows too.

- **Async as a standard module, `elya/async`** (2026-10-31, on `main`, faeec2a and
  a6f782a; step 1): spec `docs/superpowers/specs/2026-10-31-elya-async-module-design.md`.
  - `effect Async(a, e)` (`async_spawn`, `async_await`, `async_yield`,
    `async_sleep`); `async.scope` (a virtual clock: deterministic),
    `async.schedule` (a clock as two functions), `elya/async_host`'s
    `async_host.timed` (the machine's); a scope answers `AsyncFinished(v)` or
    `AsyncDeadlocked(ids)`, never a hang. Spawning is a capability a dependency
    is granted (`import lib/dep / {Async}`).
  - `examples/06_structured.elya` and `08_timers.elya` run on it, outputs as
    before (08's deadlock ids now ascending); 04 stays the scheduler written out.
  - The review's: a quadratic scheduler (fib(14) over 60 s in the evaluator, now
    1.7 s: waiting tasks parked, a two-list queue, a Braun tree by id); a clock
    effect that leaked into the tasks' row (now two functions). Nested scopes
    written inline are refused natively by name: older, step 4's.

- **Compiler scalability: no step quadratic in a program's size** (2026-10-31 to
  2026-11-06, on `main`; step 1): spec
  `docs/superpowers/specs/2026-10-31-elya-scalability-design.md`.
  - Parts 1 and 2 (033ee61): inference in proportion to the program plus its
    types, the evaluators to their steps (`tests/infer_complexity.rs`,
    `tests/eval_complexity.rs`).
  - Part 3a (dfe9c10): the native passes find a name without searching (`Scope`,
    `Env`); a parameter down 16 000 statements emits in 1.5 s (9.2 s).
  - Part 3b (c604eac): a block's binding is a root until its last reader; 2000
    string statements emit 18K IR lines (8M). `ELYA_GC_STRESS=N` collects at
    every Nth allocation and stops a program in which anything reachable names
    a freed block (`crates/codegen/tests/native_gc_stress.rs`).
  - Part 4 (60c108b): a `let` generalizes by level, not by reading the
    environment (D14-D16). Closures over a function parameter were cubic in the
    front end (4000: killed at 78 s; now 0.07 s). The environment's answer is
    kept as a check (`elya::types::check_levels`, `ELYA_CHECK_LEVELS=1`);
    `tests/levels_agree.rs`. Found with it, older: an annotation variable's
    rows were quantifiable (three programs that checked clean are now refused,
    E0420, as "one type for the function" says).
  - **Decisions (mine; please review):** levels after all (D2's open entries
    first, then levels when the review found the cubic); liveness, not rooting
    once at the binding (spec D13); a stress mode in the runtime, outside
    `gc_mark`. What it left is step 5.

- **The affine guarantee across functions** (2026-11-12, cf3f028): spec
  `docs/superpowers/specs/2026-11-06-elya-affine-across-functions-design.md`, plan
  beside it. Measured first: 17 programs reading a `linear` value twice checked clean
  and ran, 3 sound ones were refused. Now: duplicable types required by marks on type
  variables (`bind` checks, `instantiate` copies: a generic function's duplication is
  in its type); counts by arm and by how often a closure or clause runs (a syntactic
  pass, `src/affine.rs`, before inference); multi-shot points holding by row, the
  requirement riding on row variables; `with any` operations take duplicable
  arguments; new E0437. The review found two holes (a call's waiting operands; a local
  generic held on an outer row) and a false refusal, fixed test-first. Gate 1497.

## The next five steps

### 1. A collector for bare metal and for large programs alike

The maintainer's priority (2026-11-12: "make a gc that is able to use it in bare metal
aswell effienctly"; "this language covers from bare metal to python so make
everything more efficient and high quality"). Governing constraint: ONE collector for
both ends, not a bare-metal variant. Today (`crates/codegen/src/runtime.c`): mark-sweep,
a `calloc` per object, a two-word header (`meta`, `next`) on every block, all live
blocks on one list, free lists for sizes under 16 words; the runtime calls libc about
170 times (`exit`, `fprintf`, `getenv`, `pthread`, `mmap`, `sigaction`). Direction
(to decide in a spec, measured first, benchmarks in the scratchpad's `gcb/`):
- the heap as pages of one size class each, mark bits in a bitmap per page, no
  per-object header word for the list; pages from ONE page source: a fixed region
  given at start (`elya_heap_init(base, size)` or a static array) on bare metal,
  the OS (`mmap`/`VirtualAlloc`) growing on demand above it;
- lazy sweeping and a fixed-size mark stack with an overflow rescan: the collector
  allocates nothing and its pauses are bounded;
- a platform layer: the runtime's few needs from outside (pages, bytes out, stop)
  behind hooks, libc's on a host, the board's on bare metal; the collector compiles
  `-ffreestanding -nostdlib` with only those undefined;
- later layers the pages leave room for: a nursery for the high end; freeing at a
  `linear` value's last use (the affine guarantee) for the low end.
**Part 1 done** (3ad459a, 2026-11-12): `crates/codegen/src/gc.c`, pages and spans of
31 size classes, side tables, lazy sweeping, a bounded worklist with page rescans, one
page source (OS chunks hosted, `elya_heap_init` on a board); builds freestanding for
x86-64, Cortex-M4, RISC-V 32; faster on every program measured, half the memory on
large live sets (one recorded exception, `big_live`, spec). The runtime compiled -O2
once per process. Gate 1504.
**Part 2 done** (8d08c11, its review's fixes 6caabff, 2026-11-13): the whole runtime freestanding behind
`elya_platform_*`; Elya programs linked with a board and no C library write what
the hosted build writes (x86-64, the test machine's processor); working memory from
the one heap, free again as soon as freed; a board's roots grow in its region;
Float text 57 times faster. Gate 1520.
**ChatGPT's review of the collector** (467a22e): chunk growth, values over 64
words, size classes, scratch counting, small pages; fixed test-first.
**Part 3 done** (52bf757, 2026-11-14; spec 2026-11-13-elya-boards): code for a
Cortex-M3 (`Machine`, `compile_module_for`), nine programs and every call shape
writing on QEMU's `mps2-an385` what they write hosted, at O2 and O0; LLVM 18's
ARM `tailcc` pop mismatch found and padded around (`arm_tail.rs`); the stack
above an MPU guard, a stack overflow by name. Gate configuration C.
**The RISC-V board done** (6620efb, 2026-11-14; spec 2026-11-14-elya-riscv-board):
LLVM 18 has no `tailcc` on RISC-V, so every Elya function there is one
`fastcc` type of twelve 32-bit words (`rv_tail.rs`), every `musttail` valid
and a tail call; QEMU's `virt` with a locked PMP guard; pointers one word in the
runtime (`ELYA_WORD`); every board test and the ten examples on both boards;
ChatGPT's review of 52bf757 fixed (a gate that passed without the boards, a
test reader that could stop on large output, the vector table).
**Next:**
- the LLVM bug upstream (the boards spec's eleven-line reproducer; filing it is
  the maintainer's call; ChatGPT drafted the report);
- `elya build --target cortex-m3|riscv32`: the CLI does not reach
  `compile_module_for` yet; a board's link (its files, `libgcc`) in the
  compiler, not only in the tests;
- the nursery (D10) measured on churn-heavy programs, and `big_live`'s
  size-mix memory. ChatGPT's audit (2026-11-14, to verify first): Elya data is
  written only at initialisation except three stores, each able to make an old
  object point to a young one: an answering handler's `HF_NEXT` and
  `HF_PARENT` at a resume (`cps_emit.rs`), and a copied frame's `CONT_PARENT`
  in `elya_cont_copy` (`runtime.c`); `gc_collect` finishes sweeping before
  every mark (a minor must not); its proposal: sticky age bits per slot beside
  the mark bitmap (Demers et al. 1990), barriers at those three stores, and
  measuring survival by kind before building anything.

### 2. Native compilation without its implementation limits

**First, done** (fe2639d, spec 2026-11-13-elya-optimised-code): `default<O2>`
on every program, functions over 10,000 instructions built as written. The
arithmetic benchmarks reach C; what it left, in the spec's order: a closure's
call (13x C) and a perform (42x), allocation and the shadow stack's pushes at
every call (`queens` 5x), one trap block per checked operation, and the
benchmark suite and its runner in the repository (ChatGPT wrote the suite; it is
in the scratchpad's `bench/` and on the PC at `E:\Elya\chatgpt-jobs\bench`).

The maintainer's priority (2026-10-30: "native compilation has restrictive
implementation limits"). Inventory every refusal natively against the
evaluator: the budgets (256 type instances, 256 type and convention clones),
non-regular types (refused by name on purpose, spec 2026-10-17 D5), and the
effect-polymorphism corners ("upcast in a parameter position", "type variable
only in an effect row", "effect-polymorphic function used at a user effect",
"direct function used where an effectful one is expected", and since row
variables, "calling convention disagrees with the callee" for a function whose
named row says more than its body performs, and, found by the `elya/async`
review (R1), for a scope written inline in a task that also performs the outer
effect: `crates/codegen/tests/native_std_async.rs`'s
`a_nested_scope_inline_is_refused_by_name_natively` pins it, and a 14-line
program without the module fails the same way). For each: a
program the evaluator runs, then lift it or say in a spec why it stays. The
differential sweep is the measure: no refusal left that the evaluator runs.

### 3. Reading text from outside

Back from "Also open" (it was step 3 until the maintainer's priorities moved it,
2026-10-30). `replay.parse` reads a trace the program already holds as a `String`. No Elya
program can read a file or standard input (spec 2026-10-24-elya-trace-text D4).
Measure what reading needs:
- whole files or lines;
- what a missing file or invalid UTF-8 is (a failure by name, or an answer);
- how a replay answers a read (the trace holds the text it returned).
Then decide in a spec: the builtins (`io.read_file`, `io.read_line`?) and their
effect (`IO`, which the host records like `time.*`, or a capability of their own
under `elya/host`); and both back ends, recording and replaying a read the same way.

### 4. Abstract types: a module's types its importers cannot open

Found by `elya/async`'s review (R4, 2026-10-31): a task id is an `Int`, so an id
from one scope names a task of another (with the same result type, an inner scope
answers for its own task of that number; the spec writes it down). A typed handle
`AsyncTask(a)` that only the module can build or read would close it, and would
let `elya/async` keep `AsyncStep`, `AsyncSched` and the rest to itself (today an
importer can build and match every one). Measure what an importer can do with a
module's type now (construct, match, name), then decide in a spec: a type whose
constructors are the module's alone (`opaque type`? constructors without `pub`?),
what an importer may still do with its values (pass, store, compare?), the
diagnostics, and both back ends (natively nothing should change: types are
instances by then). Prove it on `elya/async`: `async_spawn` answers an
`AsyncTask(a)`.

### 5. Native IR in proportion to the program: what scalability left

From step 1 (spec 2026-10-31-elya-scalability, section 3's "Next" list and
D13's "What stays"), each measured, none designed:
- a site in an `if` arm copies the rest of its block into its continuation as
  well as after the branch: sites times length (84K / 280K / 1009K IR lines at
  2000 / 4000 / 8000 statements, one such site per 1000). A join: the
  conditional a site whose frame the arms continue to;
- a site's path holds every `let` above it (its resumption reads that path
  quadratically, and reads its block's deaths again), and its saved set reads
  the whole rest of its block;
- anf's rule 4 renames the rest of a block at each alias of a row-polymorphic
  local (2.4 s to emit 800, debug): carry the renaming down the walk;
- values all live to a block's end are rooted at every allocation (n^2);
  rooting once at the binding would not be (a GC-discipline change, every exit
  pops what its function holds).
Each: a test that fails at n and 2n first (`native_scalability.rs`), then the
design in the spec.

## Also open, unscheduled

`examples/08_timers.elya`'s real-clock part can reorder `a` and `c` under heavy
load (its snapshot then fails): the three deadlines are 10 ms apart and counted
from each task's own spawn (seen 2026-10-30 with a debugger running beside the
gate). Wider gaps, or deadlines from one start, would make it hold.

`else if` does not exist: an `else` branch must be a block, so a chain of
conditions nests (`else { if .. }`), as `std/replay.elya`'s `parse` does
(2026-10-30). Parsing `else if c { .. }` as `else { if c { .. } }` is a small
language decision with a spec of its own.

E0434 rejects a direct inner `with any` over `Hold(Opt(Op))` but accepts the same
handler through a generic function (the 2026-10-23 review's F4). A helper taking an
operation costs a clone per operation and helper (refused past the budget, 256).
Also I/O sources beyond timers (pipes, sockets: a readiness builtin
per platform, deferred by the timers spec D1); Elya functions are mangled
`elya.<name>`; the roadmap's
other priorities (record/replay handlers, then capabilities once modules exist); the rest
of the soundness sweep (recursion through a handle body and through a lambda: done
2026-10-15). A perform walks the handler chain, so a program whose handler stack really grows
(a resume inside a fresh handle every iteration) pays O(depth) per perform; the evaluator
is no faster there. Also: the Elya CEK machine's next versions (a parser for its terms, a step
counter as an Elya effect); PIC/PIE linking (needs a Windows run).
