# Handoff — where Elya is, and the next five steps

Written 2026-10-05 for whichever agent picks this up next. Read this, then `CLAUDE.md`,
then `PARKED.md`. The newest spec in `docs/superpowers/specs/` shows the house style.

## State at handoff (updated 2026-10-22, an interactive session on `auto/elya`)

- `main` has everything through #23 (merged 2026-10-16 at ab791b6: N7 part 2 through
  argument records and `elya run --record/--replay`). Draft PR #24 carries `auto/elya`
  since (the native stack onward).
- Scheduled auto-runs work on branch `auto/elya` (one draft PR to `main`) and log in
  `AUTO_RUN_LOG.md`; the maintainer reviews on GitHub.
- History was rewritten on 2026-10-05 (the maintainer's request) so no commit carries a
  former e-mail address; every branch was force-pushed. Do not push old local branches.
- The repo is private until the maintainer flips it public (only they can).
- Parked, unmerged: `claude/outside-edit-tail-in-match` -- edits from another writer (not
  this session); superseded by `claude/tail-in-match`, kept until the maintainer says.
- Linux gate at the tip of `auto/elya`: 1100 passed, 106 suites. Native programs run on
  their own 1 GiB stack (`elya_start`). Since `main`, the runtime
  has gained `elya_cont_copy` (multi-shot), the string functions and the two time
  functions, and a handle's body ends at the shared end frame (`gc_collect` grays the
  current handler as a root); `gc_mark` is unchanged (637 bytes of source).
- Interactive sessions may also work on `auto/elya`. They use the same
  `AUTO_RUN_LOG.md` lock: an IN PROGRESS entry under 3 hours old makes a scheduled run
  exit.
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

## The next five steps

### 1. Two native wrong values: a shadowed top-level name, and a wrapped function's result

The 2026-10-23 review of the one-shot-words slice found both on the build BEFORE it.
The slice narrowed its alias rule around them; they need fixing properly.
- **A local that shadows a top-level function, used free in a lambda or a handle
  body, runs the top-level function.**
  - `fn inc(x) { x + 1000 }` then `let inc = fn(z) { z * 2 }  let f = fn(q) { inc(q) }`:
    native 2004, the evaluator 1007.
  - Some shapes panic the compiler in inkwell instead ("Found PointerValue ...
    expected the IntValue variant"), and some fuzzed programs fail LLVM
    verification.
  - The suspected cause, from the review: closure conversion (`closure.rs:216,235`)
    and the lifted handle bodies treat a name as module-level whenever a top-level
    function has it, instead of resolving it by scope.
  - The reproducing programs are in the 2026-10-22 spec §3 (re-derive them from it:
    the scratchpad does not travel).
- **Rule 2's wrapper does not coerce a result-layer upcast.**
  - `fn mk() -> fn() -> Int { fn() { 5 } }` passed where
    `fn() -> fn() / {E} -> Int` is expected (`ap2(mk)`): native 5, the evaluator 15.
  - `inner_type` (`anf.rs`) corrects only the outer row, and the upcast adapter
    (`specialize.rs`) tracks local binders only.

Measure each with a corpus that writes every binder kind (let, parameter, pattern,
clause parameter, return binder) and every place an upcast can happen. Fix by scope,
not by name, then re-run a fuzzer like the review's: aliases, shadowing,
constructors, multi-shot, every native result against the evaluator.

### 2. Operation values as values natively

`with any` natively (Done) refuses an operation value used other than by `op.*` or
`==` (passed to a function, stored, returned: spec D2; a bare alias `let p = o` runs
since 2026-10-22, because `anf` substitutes it), and two evaluator behaviours
stay native gaps: decoding a non-regular type (`too many type instances`) and
decoding at an effect's own parameter (refused on both sides, the evaluator's S2).
Measure what a recorder library would write (a `describe(o)` helper, a log of
operations), then decide in a spec: clone functions over `Op(t)` per operation as the
expansion clones clauses, or a run-time operation value; and whether both sides should
decode at the instantiation (the evaluator would need it carried).

### 3. Literals at the edges, and a layout rule

Two lexical gaps the 2026-10-17 review found: `MIN` cannot be written
(`-9223372036854775808` is E0001 "not a valid Elya token"; an out-of-range literal
deserves its own diagnostic, and `-` applied to the literal `9223372036854775808`
could be accepted as `MIN`, as Rust does), and a line starting with `-` or `(`
continues the previous expression (`let y = 4` then `-x * y` parses as one
expression). Measure the corpus for both, then decide in a spec: a literal rule for
negation, and a layout rule (an operator at the start of a line ends the statement
before it, or is an error).

### 4. A failing traced run keeps its output on stdout

`elya run --replay` drops what the program printed before a failure, and `--record`
prints it to stderr with the diagnostic; plain `elya run` (since 2026-10-16) and a
native binary (record or replay) keep it on stdout. Make the two trace modes do the
same (`run_source_recording`/`run_source_replaying` return the output apart from the
error), then a differential test of a failing replay's stdout across both sides.

### 5. Modules, part 1: a program in more than one file

ROADMAP priority 2 (capabilities per dependency) needs modules first. Today every
program is one file and `elya check|run|build` take one path. `import a/b` (with an
optional alias) parses -- `examples/01_hello.elya` opens with `import elya/io` -- and
so does `pub` on any function, but nothing after the parser reads either
(`ast::Module::imports`, `ast::FnDecl::is_pub`). Measure first how the resolver, the
checker and Core name things today, and what a two-file version of an example would
need. Then decide in a spec:
- what `import` means and how its path resolves (a file next to the importing one?),
  and what `import elya/io` becomes (ignored today; `io.println` is a builtin);
- what a module exports (`pub` on functions, types and effects?);
- qualified (`json.parse`) or unqualified names;
- how Core and codegen see the program (one merged `CoreModule` is the smallest step).

Leave the effect allow-list at the import boundary to part 2, but choose the syntax so
it has a place to go.

Also open, unscheduled: `Float` natively (the evaluator has literals,
`+. -. *. /.`, the comparisons, `-x`, NaN-aware `==` and `with any` encoding; Core
refuses the literal, 3 sweep programs). Measure exactly what the evaluator writes for a
float first (`10000000000000000000000000000000000000000.0` for `1e40`, `-inf`), then
decide the word representation, a C printer that matches Rust's `f64` output on edge
values, and decoding. Also row variables in annotations (`fn(fn() / {e} -> a)`); I/O sources beyond timers (pipes, sockets: a readiness builtin
per platform, deferred by the timers spec D1); Elya functions are mangled
`elya.<name>`; the parser continues a call across a
newline (`f()` then a line starting `(` is `f()(...)`) -- a layout rule is a language
decision; the roadmap's
other priorities (record/replay handlers, then capabilities once modules exist); the rest
of the soundness sweep (recursion through a handle body and through a lambda: done
2026-10-15). A perform walks the handler chain, so a program whose handler stack really grows
(a resume inside a fresh handle every iteration) pays O(depth) per perform; the evaluator
is no faster there. Also: the Elya CEK machine's next versions (a parser for its terms, a step
counter as an Elya effect); PIC/PIE linking (needs a Windows run).
