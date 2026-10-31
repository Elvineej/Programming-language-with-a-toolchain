# Auto-run log

Runs working on `main` (on branch `auto/elya` until 2026-10-30) record their state here,
newest entry first.
A run whose top entry says IN PROGRESS and started less than 3 hours ago is still active.

## 2026-10-31 07:30 UTC — IN PROGRESS (interactive session, at the maintainer's "Continue"; started 01:22, re-dated at each checkpoint)

- The 2026-10-24 09:48 entry never marked DONE; its last commit is 2778bc1 (09:59, the
  spec and plan for HANDOFF step 1, strings to read a trace). Taken over from there.
- At the maintainer's "Commit in the main tho" (16:21), `auto/elya` was merged into
  `main` (731ce70, its tree identical to the gated 68fdc9f; PR #25 merged by it), and
  work continues on `main`.
- At the maintainer's "continue when the limit resets, no stops", this session keeps
  two resumes armed into itself, pushed forward at every checkpoint: "Elya: auto-resume
  after the usage limit (primary)" about 5 h ahead (trig_012WFJHxdZgSbHq5NwMX3uFD) and
  "(backup)" about 9 h ahead (trig_01KNPbPDMoaxidd8dxj5qTVQ).
- **Done: strings by byte, one trace line format, `replay.parse`** (HANDOFF step 1;
  spec 2026-10-24-elya-trace-text, the scheduled run's spec and plan; implementation
  20a4804).
  - `string.length/byte_at/slice` on both back ends.
  - Both host readers split at the first unquoted ` = `.
  - `replay.parse` is in Elya.
  - Gate 1310 / 135 (predicted 1308; +2 for the review's test).
  - **Decisions (mine; please review):**
    - bytes, not characters;
    - failures by name, not an `Option`;
    - the host keeps `str::trim` (the spec's first draft moved it to ASCII, which
      would have broken a native review test);
    - the evaluator's strings are `Rc<str>`.
  - The review found two issues, both fixed first:
    - `parse` split the trimmed line (21 of 1 041 short traces read differently);
    - the evaluator was quadratic on byte loops (89 s, now 27 s, linear).
  - Found: no `else if` in the language (HANDOFF's open list).
- **Done: what `main` is worth, for every data type** (HANDOFF step 1, the sweep's
  last native refusals; spec 2026-10-30-elya-main-values; fa24c18).
  - `--value` and native binaries write main's value as its literal; a function in
    it fails by name; a generic `main` builds at its defaults.
  - Gate 1315 / 136 (predicted 1315; 1312 before the review's fixes, as predicted).
  - **Decisions (mine; please review):** `Unit` stays `0`; the instance budget's two
    refusals stay (non-regular types).
  - The review found five issues, all fixed first. The worst: the evaluator wrote a
    constructor used as a function (`Some`) as data, in `op.encode` too.
  - Seen under load: `08_timers`'s real-clock snapshot can reorder (HANDOFF's open
    list); the re-run, alone, passed.
- **Done: a block's length is not a stack bound** (HANDOFF step 1; spec
  2026-10-30-elya-long-blocks; 17869d4).
  - Every codegen pass loops over a block's `let`s; Core and the evaluator's
    environment drop iteratively. `elya build` of 100 000 statements: 18.6 s (was a
    stack overflow).
  - Gate 1319 / 139 (predicted 1319).
  - **Decisions (mine; please review):** loops over the spine, not a flatter Core
    block node; no `stacker` (its `psm` needs Rust 1.88); nesting depth stays
    bounded (the parser's E0102 at 1024 levels).
  - The review found a miscompile, fixed first: `spill` had kept a `let`'s type
    unspilled. Its tests now run on 512 KiB threads.
  - Measured on the way: the evaluator's name lookup is quadratic on a long block
    (77 s for 20 000 statements): HANDOFF's new step 5.
- **Done: a name clash is reported once, where it was made** (HANDOFF step 1; spec
  2026-10-30-elya-name-clashes; d868904).
  - No diagnostics inside a module because of a root's clash; E0605 never for a
    name the file or an import declares. Gate 1331 / 141 (predicted 1331).
  - **Decision (mine; please review):** types, constructors, effects and operations
    stay one name space (their names are data in traces and `op.show`).
- PR #24 had been merged (2026-10-24), and nothing carried `auto/elya` after it, so
  the maintainer saw no new commits: opened draft PR #25.
- **Done: the occurs checks look inside effect rows** (2736024; spec
  2026-10-30-elya-occurs-in-rows). Found measuring step 1: three-line programs
  crashed `elya check` (a stack overflow). The review found the first fix turned some
  into a hang, fixed first. Gate 1337 / 143.
- At the maintainer's request (2026-10-30), HANDOFF's steps 3-5 are now their three
  priorities: compiler scalability, the affine guarantee across functions, native
  compilation without its limits.
- **Done: row variables you can write in a signature** (HANDOFF step 1, part 1; spec
  2026-10-30-elya-row-variables; 394a0b8). `/ {e}` and `/ {Log | e}`, one row per name
  per function. The review found three serious bugs, all fixed first, the worst an
  older row-unification bug that names made writable (a clean-checking program
  failed at run time). Gate 1355 / 145.
  - **Decisions (mine; please review):** the printer's syntax (`{Log | e}`);
    flexible names, not rigid; a lowercase name spelled like an effect is a typo.
- **Done: row parameters on declarations** (HANDOFF step 1, part 2; spec
  2026-10-30-elya-row-variables section 3; 499c048, on `main`). `type Task(e)`,
  `effect Async(e)`, row arguments written as rows (`Task({Log})`); a label's
  written arguments are now checked. A task that spawns has a finite type, on
  both back ends; 04's round robin written once for any other effects. Gate
  1385 / 148 (predicted 1385; 1372 and 1374 at the earlier checkpoints, each as
  predicted).
  - The review found one high (a lost label: checked clean, crashed natively),
    two medium (correct programs refused) and one low; all fixed first, and
    checking F3's fix found a fifth (a duplicate label dropped), fixed with a
    control. One existing expectation changed: `occurs_in_rows.rs`'s
    `fn g(k) { op(k)  k() }` checks now (a finite type).
  - **Decisions (mine; please review):** kinds by use, not marked; a row
    argument is a phantom function type; a label's arguments, once written, are
    checked.
- **Windows:** the gate on main (2ab5833) ran 1165 tests, 3 failed (native
  failure lines read "\r" on Windows: stderr was text mode); ba8b629 puts
  stderr in binary mode like stdout. The run was then cancelled from the
  maintainer's account.
- Windows: the gate on 5e3bb5c passed (1381, 0 failed): 20a4804, fa24c18, 499c048
  and ba8b629 are clear.
- **Done: `elya/async`, part 1, the module** (HANDOFF step 1; spec
  2026-10-31-elya-async-module; faeec2a, on `main`). `Async(a, e)`, `async.scope`
  (a virtual clock), `async.schedule` (a clock as two functions),
  `elya/async_host` (the machine's clock). Gate 1407 / 151 (predicted 1407; 1402
  before the review's fixes, as predicted).
  - The review found one high (nested scopes refused natively: older, recorded in
    HANDOFF's native-limits step), three medium (the host module reserving
    `Async*` names; a quadratic scheduler: fib(14) over 60 s, now 1.7 s; ids
    across scopes, written down) and two low. All fixed first or written down.
  - **Decisions (mine; please review):** a scope answers `AsyncEnded` (a deadlock
    by name); time is virtual by default; the clock is two functions, not an
    effect (measured: an effect leaked into the tasks' row and needed a grant).
- Next: HANDOFF step 1, part 2: the examples on `elya/async`.
- Windows-gate debt: 20a4804, fa24c18, and since then 499c048 and ba8b629
  (the next Windows run on `main` covers them).

## 2026-10-24 09:48 UTC — IN PROGRESS (scheduled run, FULL mode)

- The 05:53 entry is DONE (07:58). Next: HANDOFF step 1, strings to read a trace.

## 2026-10-24 05:53 UTC — DONE at 07:58 (scheduled run, FULL mode: three tasks)

- The 21:55 entry started eight hours ago (over the three-hour lock); its last commit
  is 8757408 (00:33). Taken over.
- LLVM 18 headers were missing again; installed (`llvm-18-dev`, `libpolly-18-dev`,
  `libzstd-dev`). Baseline 1241 / 125.
- **Done: diagnostics at the edges** (HANDOFF step 1). Spec+plan 863e7c5,
  implementation 0f8da88.
  - Byte offsets in the renderer (labels after non-ASCII text were shifted);
    E0209/E0202 label the first declaration too (in its own file); the end of
    the file is reported once; expressions nest at most 1024 levels, E0102
    (chains counted by height), recovery at the next declaration; the CLI
    compiles on a 1 GiB thread (measured: a 700-line `main` overflowed a debug
    `elya build`; 1600 `+` terms or ~900 nested blocks overflowed `check`).
  - Gate 1264 / 127 (predicted 1260; the review added two tests). K1-K4 distinct.
    Review: 686 programs + examples on both builds, no valid program changed;
    F1 (a real second missing token dropped), F2 (recovery inside an `effect`),
    F3 (the CLI refused under `ulimit -v`) fixed test-first; F4 recorded.
  - **Decisions (mine; please review):** a language-level nesting limit of
    1024 (not an explicit stack); the CLI's 1 GiB thread (block length stays
    unbounded, so iterative Core passes are new HANDOFF step 5); D3 narrowed to
    the end of the file.
- **Done: capabilities part 3, attenuation in the standard library** (HANDOFF
  step 1). Spec+plan 931b898, implementation b70707f.
  - `elya/console` (`Console`) and `elya/clock` (`WallClock`), written in Elya,
    with attenuating handlers; `elya/host` turns them into `IO` and needs
    `/ {IO}`. A dependency can now be given lines or a fixed clock, not the host.
  - Gate 1275 / 130 as predicted; K1-K2 distinct. Review: no soundness defect
    (no route from a `Console`-only grant to `IO`), evaluator = native for every
    handler; the generic names (`Out`, `Clock`, `Counted`) renamed on its advice.
  - **Decisions (mine; please review):** the host handlers in a module of their
    own; grants apply to the standard Elya modules unchanged (so
    `import elya/console / {Console}`); builtins stay importless in modules
    (D5: a file's own import adds no authority; requiring it would change eight
    tests' expectations).
- New HANDOFF step 5: name clashes across files (diagnostics in the user's file).
- **Done: block tails and float exponents** (HANDOFF step 1). Spec, plan and
  implementation 3ccab88.
  - A block ending in `let`, or empty, is E0103 (was `Unit` in the evaluator,
    refused natively). `1.0e19`-style literals; `1e19` E0003; out of range E0002.
  - Gate 1284 / 132 as predicted; K1-K2 distinct. Review: nothing changed
    meaning; F1-F2 (two help/diagnostic gaps) fixed test-first.
  - **Decisions (mine; please review):** a front-end error rather than lowering
    a missing tail as `Unit`; the `.` stays required (`1e19` refused, not a
    float); `float.to_string` keeps positional text (the trace format).
- New HANDOFF step 5: async as a standard module, `elya/async`.
- Stopped near the three-hour lock so a later run can take over cleanly. Next:
  HANDOFF step 1 (strings to read a trace). Gate at the tip: 1284 / 132.
- Windows gate (`scripts/check.ps1`) still owed for this run's commits (listed
  below) and the earlier ones.
- Windows-gate debt: 3ccab88 (lexer only; low risk), b70707f (new std
  sources; no runtime change), 0f8da88 (the CLI's thread; a
  `#[cfg(target_os = "linux")]` test), with the earlier list (dce886a, 670c842, dfb8a5c, e78ace6, ...).
- `graphify` is not installed here, so `graphify update .` was not run.

## 2026-10-23 21:55 UTC — ended without a DONE mark (last commit 8757408, 00:33), taken over 2026-10-24 05:53 (scheduled run, FULL mode)

- The 17:53 entry started four hours ago (over the three-hour lock); its last commit
  is daed520 (20:48). Taken over; if that run is still pushing, I merge, never force.
- LLVM 18 headers were missing again; installed (`llvm-18-dev`, `libpolly-18-dev`,
  `libzstd-dev`).
- Baseline 1193 / 116.
- **Done: upcasts through data, natively** (HANDOFF step 1). Spec 27696a2, plan
  4d6b1e9 (the 17:53 run), implementation 09d8681, HANDOFF 1b0c7f8. A pure closure
  inside data used at an effectful type: natively a pointer or "match failed"
  before. The wrapper binds its own result's free row variables from the use (D1);
  the data is rebuilt by a generated `$coerce.T.k` (D2); the guard compares data
  names (D3); a non-regular type is refused by name (D4, the review's F1: the first
  version overflowed the compiler's stack). Gate 1198 / 117 (predicted 1197; the
  review added one test). K1-K3 distinct. Review sweep: 886 programs, no change.
  - **Decisions (mine; please review):** rebuild data (a copy per upcast) rather
    than refuse or make data invariant in the checker; D4's size rule.
  - Found (older): the wrapper still prints a pointer when the own result's row
    variable is also a parameter's (the review's F2): new HANDOFF step 1.
- **Done: a wrapped function's row shared with a parameter** (HANDOFF step 1).
  Spec+plan ddbfbe8, fix 4a0aeb1. The wrapper binds a parameter's row variables
  from the use's parameters (D1), and walks the result's own parameters first
  (D2, the review's older finding). Gate 1199 / 117 as predicted; K4 distinct;
  review sweep 916 programs, 0 differ, no class changed.
  - **Decision (mine; please review):** bind rather than refuse.
- New HANDOFF step 5: aliases that rebind their target (rename, not keep).
- **Done: parser error recovery inside a body** (HANDOFF step 1). Spec+plan 9a08e3a,
  implementation f4777d0. A failed statement is `Expr::Error` (new AST node) and
  parsing resumes at the next statement; a header failure keeps a stub; the end of
  the file is an error (p6 used to pass `check`). Gate 1209 / 119 (predicted 1207;
  the review added one test). Review: 1083 valid programs unchanged, ~10 000
  mutants, no hang or panic, nothing newly accepted.
  - **Decisions (mine; please review):** statement-level recovery with an error
    node; `fn` is a declaration only before a name (column 0 was tried first and
    broke a test whose source loses its indentation); a stricter `}` check on
    handlers and effect rows (they were unchecked).
- New HANDOFF step 5: diagnostics at the edges (non-ASCII labels, E0209 across
  files, deep nesting overflow, a double report).
- **Done: modules part 2, the effect allow-list at the import** (HANDOFF step 1).
  Spec+plan a291b15, implementation 48d5c8f. An import grants exactly its
  list; none grants nothing; E0606/E0607/E0608. Gate 1221 / 119 (predicted 1217;
  the review added two tests). K5-K6 distinct. The review found three soundness
  holes (closures leaving through `resume`, an operation's argument, `St(s)`);
  fixed test-first by walking label arguments and operation signatures.
  - **Decisions (mine; please review):** the strict default (no list = nothing);
    capability = positive positions of `pub` types (a callback's effects are the
    caller's); `IO` and user effects alike; builtins still need no import, and a
    list on `elya/io` is refused.
  - **Changed tests:** `an_allow_list_is_e0606` (expectation, by D2); two programs
    gained `/ {Log}` (expectations unchanged).
- New HANDOFF step 5: capabilities part 3 (attenuation; builtins behind an import).
- **Done: `Float` part 2** (HANDOFF step 1). Spec+plan a4354ce, implementation
  21a622e. `<. <=. >. >=.`, `-.e`, `float.of_int/truncate/round` on both sides.
  Gate 1229 / 121 as predicted; K4-K6 distinct; the review found no defect.
  - **Decisions (mine; please review):** separate comparison operators (not
    overloading `<`); `-.` prefix; rounding `of_int`; failing (not saturating)
    `truncate`/`round`.
- New HANDOFF step 5: a block ending in `let` natively; exponent float literals.
- **Done: record and replay as an Elya library** (HANDOFF step 1). Spec+plan e04c0ee,
  implementation c5d618a. `import elya/replay` (Elya source embedded in the
  compiler): `record`, `replay`, `text`, `size`. Gate 1238 / 124 as predicted; K1
  distinct; the review found no defect, and four limitations of `with any`
  (recorded in the spec and HANDOFF step 5).
  - **Decisions (mine; please review):** the standard library's first Elya-source
    module (the `elya/` prefix reserved, the embedded file wins); prefixed type
    names (`Trace`, `Recorded`, `Replayed`); reading trace text deferred until
    string functions exist.
- New HANDOFF step 5: strings to read a trace.
- **Done: aliases that rebind their target** (HANDOFF step 1). Implementation, spec
  and plan b7a176b. Capture-avoiding renaming in `anf`'s alias rule; the
  top-level-name guard dropped. Gate 1241 / 125 as predicted; K1-K2 distinct; the
  review found no defect (sweep 982 programs, 0 failing, six refusals gone).
  - **Changed tests (by D1):** native_codegen's alias-guard test now runs its
    program; anf's rebound-alias unit test asserts the renaming; a new unit test
    pins the guard.
- New HANDOFF step 5: the sweep's last native refusals.
- Working on: HANDOFF step 1 (diagnostics at the edges).

## 2026-10-23 17:53 UTC — IN PROGRESS (scheduled run, FULL mode)

- Took the lock; the 10:50 entry is DONE.
- LLVM 18 headers were missing again; installed (`llvm-18-dev`, `libpolly-18-dev`,
  `libzstd-dev`). Baseline 1115 / 107.
- **Done: literals at the edges, and a layout rule** (HANDOFF step 1). Spec 7d7606f,
  plan c190e49, implementation 5058cda, HANDOFF 2714d82.
  - MIN is a literal (`-` folds into a numeric literal), E0002 for out-of-range
    literals, negative literal patterns, and in braces a newline ends an expression
    before an infix operator or a call's `(` (E0101 for an operator starting a line).
  - Gate 1139 / 110 (predicted 1131; the review added four tests). K1-K4 distinct.
    Sweep 836 programs, no disagreement.
  - **Decisions (mine; please review):** the fold also for floats (`-1.5` type-checks
    though `-x` on a Float stays E0400); the whitespace after a line-start `-` decides
    prefix (new statement) vs binary (E0101); newlines are whitespace inside parentheses.
  - **Behaviour change:** a line starting with `+`, `*`, ... that used to continue the
    line before is E0101. No corpus program does this; no test expectation changed.
- **Done: a failing traced run keeps its output on stdout** (HANDOFF step 1). Spec
  5037e19, plan 51f2937, fix 8a72c45, HANDOFF d33f241. `elya run --record|--replay`
  now print the program's text on stdout and the error on stderr, as native does.
  Gate 1144 / 110 as predicted; the review found no defect.
- New HANDOFF step 5: parser error recovery inside a body.
- **Done: modules, part 1** (HANDOFF step 1). Spec fac887f, plan 93b6c74,
  implementation c7bb82b, HANDOFF db49fd1. `import a/b [as c]` reads `a/b.elya` next to
  the program; qualified function calls (`b.f`, `pub` exported); type-level names stay
  unqualified, program-unique, visible where imported; E0600-E0606; one merged module;
  the CLI reads the files. Gate 1184 / 113 (predicted 1174; the review added five
  tests: separate name spaces in the visibility check, E0205 inside modules, duplicate
  E0201s, an end-of-file span, two messages).
  - **Decisions (mine; please review):** paths relative to the root program's
    directory (not the importing file); qualified functions only; unqualified,
    program-unique types/effects (so `op.show` and traces need no format change),
    visible only through a direct import; no `pub type`; `import a/b / {E}` reserved
    and refused (E0606) until part 2; builtins still need no import.
  - Found (older, now in HANDOFF step 4): a truncated declaration at the end of a file
    (`pub fn g(`) passes `check` and is silently dropped.
- **Done: `Float` natively, and `float.to_string`** (HANDOFF step 1). Spec c77d2a2,
  plan 97b8244, implementation 6baf595, HANDOFF 2e01a0f. Floats are their bits in an
  untraced word; IEEE equality also inside data (float masks per tag) and in
  `o == o`; a C printer proven against Rust on 200,000 random values (it found that
  `%.*e` rounds ties to even where Rust rounds half up); decode by `strtod`. Gate
  1193 / 116 as predicted; review found no defect; sweep: the 3 Float build errors
  are now `same`.
  - **Decisions (mine; please review):** unboxed bits (not boxed floats); a separate
    float-mask table for `elya_eq` (gc rows unchanged); `float.to_string` added;
    comparisons/conversions deferred (new HANDOFF step 5).
- Working on: HANDOFF step 1 (decoding at an effect's own parameter).

## 2026-10-23 10:50 UTC — DONE at 11:50 (scheduled run, LIGHT mode: four tasks)

- The 03:38 entry's last commit is 1f1e677 (05:00 UTC), over three hours old: taken
  over (lock 95e1e08). LLVM 18's headers and `libzstd-dev` were missing; installed
  (`llvm-18-dev`, `libpolly-18-dev`, `libzstd-dev`). Baseline 1106 / 106.
- **Done: operation values as values natively** (HANDOFF step 1).
  - Spec b4772c1, plan ff5d055, implementation dce886a:
    `docs/superpowers/specs/2026-10-23-elya-operation-values-natively-design.md`.
  - A `with any` clause's `o` is a value of a synthesized type per operation
    (its arguments as fields). It can be given to helpers, stored, captured,
    returned and compared natively; each of those was refused before.
  - Gate 1115 / 107 (plan predicted 1112; the review added three tests). Controls
    K1-K3 each failed distinctly. Sweep: 810 programs, no disagreement.
  - The review found one new wrong value: `==` on data around an operation
    answered `eq` where the evaluator fails. It also found two regressions: the
    clone budget, and a nested recorder at an outer answer type. All three were
    fixed test-first. The fix for the first changes `runtime.c`'s `elya_eq` (a
    fourth parameter; fields walked in the evaluator's order).
  - **Decisions (mine; please review):**
    - a per-operation type and per-operation clones, rather than a run-time
      operation value;
    - S2 decoding at an effect's parameter stays refused on both sides (now
      HANDOFF step 5).
  - **Changed tests, by these decisions:**
    - `any.rs`'s `an_operation_value_used_as_a_value_is_refused`, which now expands;
    - the `describe(o)` half of `what_is_not_compiled_is_refused_by_name`, which
      runs now.
- HANDOFF: step 1 moved to Done; new step 5, decoding at an effect's own parameter.
  Next: HANDOFF step 1, literals at the edges and a layout rule.
- `graphify` is not installed here, so `graphify update .` was not run.
- Windows-gate debt: dce886a (runtime C: `elya_eq`), with the earlier list
  (670c842, dfb8a5c, e78ace6, ...).

## 2026-10-23 03:38 UTC — ended without a DONE mark at 05:00, taken over 10:50 (interactive session, at the maintainer's "Continue until u reach the limit"; re-dated)

- Took the lock at ebc2635 on 2026-10-22 22:35 UTC. Baseline 1090 / 106. Working on
  HANDOFF step 1 (one wording for the one-shot trap, and the sweep's smaller gaps).
- Paused at the usage limit at about 00:00 UTC, as the maintainer asked (a four-hour
  pause, then one more round). Resumed 03:36 UTC.
- **Done: one wording for the one-shot trap; constructors and aliases as values
  natively** (670c842, HANDOFF step 1). Spec
  `docs/superpowers/specs/2026-10-22-elya-one-shot-words-and-sweep-gaps-design.md`.
  - **Decisions (mine; please review):**
    - "a one-shot continuation was resumed twice" on both sides, with a corrected
      evaluator label;
    - `anf` wraps a constructor used as a value (it SEGFAULTED natively), and
      substitutes an alias of a row-polymorphic local;
    - a user `effect IO` is E0209;
    - `Float` natively is deferred (HANDOFF's open list).
  - **Changed tests, by these decisions:** the native trap's words (three tests);
    `an_operation_value_used_as_a_value_is_refused`'s first clause (a bare alias, which
    now runs); `an_alias_of_a_generic_local_used_at_a_user_effect_is_refused_by_name`'s
    program (it rebinds the target, so its guard stays pinned).
  - Gate 1100 / 106 (predicted 1099; one new test for a consequence the gate found).
    Sweep: 771 programs, no `both-fail-differently`, no disagreement.
  - The review found three regressions in the first alias rule: wrong values, compiler
    panics and new refusals. All were fixed before the commit by narrowing the rule,
    and each is a corpus row now.
  - It also found two native wrong values that were already there before this slice,
    now HANDOFF step 1: a local shadowing a top-level function, used in a lambda (2004
    for 1007); and a wrapped function's result upcast (5 for 15).
- **Done: lambdas capture by scope; a wrapped function's result is coerced** (dfb8a5c, HANDOFF
  step 1, the review's native wrong values). Spec
  `docs/superpowers/specs/2026-10-23-elya-scope-captures-and-wrapper-results-design.md`.
  - Closure conversion decides captures by scope: 2004 for 1007 before, and compiler
    panics.
  - `anf`'s wrapper binds a function's own result so `specialize` coerces an upcast:
    5 for 15 before, and a pointer.
  - Gate 1106 / 106 as predicted.
  - The review's fuzzers on the earlier seeds: 7 build failures and 97 refusals went
    to 0 and 18.
  - The second review found one more older wrong value (a wrapper in a
    row-polymorphic function), fixed. One is still open, in HANDOFF: a row
    variable inside data in a function's own result.
- Windows-gate debt: 670c842 and dfb8a5c (with the earlier list).

## 2026-10-22 15:22 UTC — DONE at 22:12 (interactive session, at the maintainer's "Start next task")

- The 2026-10-17 01:53 entry never marked DONE; its last commit (7ac69a6) is 10-17 04:19.
  Every scheduled run since stopped on the usage limit at start. This session took over
  at 7ac69a6 (lock b2ec3cf, heartbeat cfe3021). LLVM 18 was already installed.
  Baseline 1086 / 106.
- **Done: multi-shot resumes share the captured chain** (e78ace6, HANDOFF step 1). Spec
  `docs/superpowers/specs/2026-10-22-elya-multi-shot-sharing-design.md`, plan
  `docs/superpowers/plans/2026-10-22-elya-multi-shot-sharing.md`.
  - A handle's body continues to one shared end frame, not to its handler frame. A
    multi-shot resume copies only the handler frames on the perform's path, and the
    current handler is a collector root.
  - PARKED's `nest(20000)` went from out of memory to 0.01 s; 100 000 deep runs in
    0.07 s (the evaluator 1.2 s).
  - Gate 1090 / 106 (predicted 1090). Controls K1-K4 each failed distinctly.
  - **Decisions (mine; please review):** the end frame and a handler-only copy (rather
    than copying the chain and un-rooting the clause's continuation, or copying lazily);
    the current handler as a GC root.
  - **Changed test, by this decision:** `copies_survive_a_collection_mid_copy` ran m11,
    which no longer collects. It runs a new corpus row, m13, and since the review
    asserts a collection INSIDE a copy (`mid_copy=`, a new ELY_GC_STATS field).
  - Review: no high or medium finding (2 525 random programs, the sweep and the
    corpora under a collect-every-few-allocations runtime). Three low notes fixed.
  - Sweeps: `scripts/sweep.py` (735) and the session's 763 found no disagreement. The
    4 both-fail-differently are the one-shot wording (now step 1).
- Windows-gate debt added: e78ace6 (runtime C: the end block, `elya_handle_end_init`,
  `elya_cont_copy`, `gc_collect`'s root and `mid_copy`; the generated end frame).
- HANDOFF: step 1 moved to Done; new step 5, "Modules, part 1". Next: HANDOFF step 1,
  one wording for the one-shot trap.

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
