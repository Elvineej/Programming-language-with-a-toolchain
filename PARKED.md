# Parked

- Consider RelocMode::PIC + PIE linking on all platforms. Needs a Windows gate run before landing.
- Before fast-forwarding main to the 5b-8 branch: run scripts/check.ps1 on Windows and get exit 0. Commits from 65b08e4 onward have only been gated on Linux.
  - The ≤4 source-param cap for CPS functions (5 + continuation) is untested on win64, where >5 tailcc params caused a fatal abort. Verify it there.
  - *2026-10-04: met in CI, not on the maintainer's machine.* `.github/workflows/windows-gate.yml` runs the unmodified check.ps1 on windows-latest (MSVC) with LLVM from the official 18.1.8 archive (+ libxml2s.lib from vcpkg, which the archive names but omits) -- not the local vcpkg LLVM. First run: 592/593; the failure was a real bug (stdout in text mode for programs that never print, "\r\n"), fixed in d8ede9c; then 593/593, exit 0. The cap is now exercised by `effectful_functions_at_the_parameter_cap_run_natively`. A local check.ps1 run is still worth doing once, since the vcpkg LLVM is what the maintainer builds with.

## RESOLVED (slice 5c-1, 2026-10-04): unqualified handler clauses (front-end/evaluator disagreement)

Op names are now unique per module (E0202), a clause must name a declared op of the effect
it names (E0203), and a bare clause means its op's effect in the evaluator and in Core
lowering (native inherits it). The direction below changed on measurement: a perform cannot
be qualified and every op index is last-declaration-wins, so "reject a shared op name only
inside a handler" would have left performs of it silently mis-resolved. See the 5c-1 spec.
Original report:


The front end accepts a clause written `op(...) -> …` with no effect name
(`OpClause.effect == None`), and inference resolves its effect through the
op-name registry. But the evaluator's `handler_handles` (`src/eval.rs`)
matches a clause only when `effect == Some(name)`, so such a clause is never
dispatched and `elya run` ends in E0300 "unhandled effect". Core lowering
refuses it by name, `Unsupported("unqualified handler clause")`, since
8374687.

Measured repro (2026-10-02, at e2efa00):

```
effect Ask { fn ask() -> Int }
fn one() { ask() }
pub fn main() -> Int {
  handle { one() } with {
    ask() -> resume(2)
    return(x) -> x
  }
}
```

- `elya check` → `ok`, exit 0
- `elya run` → `[E0300] Error: internal: unhandled effect `ask` reached the machine`, exit 1
- `elya build` → `error: core lowering failed: Unsupported("unqualified handler clause")`, exit 1
- Control: the same program with `Ask.ask() -> resume(2)` runs, exit 0.

Decided direction, to be done in its own slice after 5b-8: keep the
unqualified syntax, have inference record the resolved effect, and make the
evaluator and lowering both read it. An op name shared by two handled effects
gets rejected as ambiguous by the checker.

## RESOLVED (slice 5c-2, 2026-10-05): front end: should a fn sharing a name with an effect op be an error?

Yes: E0205. And a LOCAL now shadows an op lexically, in inference, the evaluator and Core
lowering (the resolver's scope walk supplies lowering's call sites). See the 5c-2 spec.
Original:


Measured: accepted today, evaluator performs the op. Both inference
(`infer_call` checks the op registry before the environment) and the evaluator
(`CalleeSlot::Operation` from `op_table`) resolve an op name before any
variable, so the function is unreachable by that name. Native lowering matches
them through `CoreKind::Perform` (plan D11).

Measured repro (2026-10-03, at adbd854):

```
effect E { fn ping() -> Int }
fn ping() -> Int { 5 }
fn user() -> Int { ping() }
pub fn main() {
  let v = handle { user() } with {
    E.ping() -> resume(1)
    return(x) -> x
  }
  if v == 1 { io.println("evaluator PERFORMED the op") } else { io.println("evaluator CALLED fn ping") }
}
```

- `elya check` → `ok`, exit 0
- `elya run` → `evaluator PERFORMED the op`, exit 0

Ops-first also beats a local binding: `let ping = …; ping()` performs the op.
Matches inference + evaluator; probably should be local-first in the language.
Decide with the collision question. Measured (2026-10-03, at 40c09a0 + 7b-1):

```
effect E { fn ping() -> Int }
fn user() -> Int {
  let ping = fn() { 5 }
  ping()
}
pub fn main() {
  let v = handle { user() } with {
    E.ping() -> resume(1)
    return(x) -> x
  }
  if v == 1 { io.println("evaluator PERFORMED the op") } else { io.println("evaluator CALLED the local") }
}
```

- `elya check` → `ok`, exit 0
- `elya run` → `evaluator PERFORMED the op`, exit 0

## RESOLVED (5b-8 close-out, 2026-10-04): slice close-out doc fixes

Spec §6.3 now carries an as-built note (guards unchanged, a separate frame guard added). Original:


- Spec §6.3 says both existing guards "must be extended" to cover the frame
  tag; b370191 left both unchanged and added a separate frame guard of the same
  shape. (A8's own row was rewritten by plan D10 on 2026-10-03; §6.3's sentence
  still says "extended".)

## RESOLVED (parked-cleanup-1, 2026-10-04): op → effect index: one helper, one copy left

The evaluator's `op_table` is now `ast::op_effects`; 5c-1's E0202 makes op names unique, so
`Infer.ops` (kept: it carries signatures) agrees with it by rule. Original:


Plan D11 adds `ast::op_effects` for lowering: the third construction of the op →
effect index, after `Infer.ops` (richer: it carries signatures, like
`effect_multi` it stays separate) and the evaluator's `op_table`. Redirecting
`op_table` to the helper touches the reference semantics, so it was not done in
5b-8; do it, gated, in a later slice.

## RESOLVED (next commit after b24b96d): shadowed heap bindings were not rooted by the direct emitter (pre-existing, found 2026-10-03)

Fixed by `bind_local`/`unbind_local` in `lib.rs`; pinned by `a_shadowed_heap_binding_stays_rooted_and_in_scope`. A match-arm binder that shadowed an outer name also REMOVED the outer binding after the arm ("unbound var" on a valid program); fixed and pinned by the same test. The CPS emitter had the same hole (its name view kept only the innermost binding); `St::bind`/`rebuild_env` now park shadowed bindings the same way, pinned by the `shadowed-binding-in-an-effectful-region` case. Original report:

`lower_expr`'s `Let` saves a shadowed binding in a Rust local (`prev`) and roots only the
`env` view, so an outer heap binding hidden by an inner `let` of the same name is NOT on the
shadow stack while the inner scope allocates. A collection frees it; after the inner scope
the outer name reads a reused block. Measured at 831b023+ (the last commit before 7b-3, and
unchanged by 7b-3): prints `33792`, expected `30049` (`7 + 30000 + 42`).

```
type L { Nil, Cons(Int, L) }
fn churn(n, acc) { if n == 0 { acc } else { let g = Cons(n, Cons(n, Cons(n, Nil)))  churn(n - 1, acc + 1) } }
fn head(l) { match l { Nil -> 0  Cons(h, _) -> h } }
pub fn main() -> Int {
  let s = Cons(42, Nil)
  let r = { let s = Cons(7, Nil)  let z = churn(30000, 0)  head(s) + z }
  r + head(s)
}
```

The CPS emitter (7b-3) roots by binding index (`St::binds`), not by the name view, so its own
allocations do not have this hole -- but the direct code it calls into does. Fix in its own
slice: root every live binding (shadowed ones included), with this program as the red test.

## 7b-3 review follow-ups (Task 8 and later)

- Resumption functions are declared and emitted for EVERY call site, including sites inside
  clause bodies, which 7b-3 never emits. A site whose path runs through `resume(..)` would
  stop the build with "resume (Task 8)". Every such program the reviewer could build is
  refused earlier today; Task 8 must emit clause bodies and revisit this.
- D16's refusal is conservative: `fn ap(f) { f(1) + get() }` is refused at every use that
  names a user effect, even with a pure lambda (its generic signature has an open row). Lift
  with N7.

## RESOLVED (2026-10-10): Evaluator: non-tail recursion under a handler is quadratic (found 2026-10-03)

Not the handler: a per-step depth walk and frame-copying capture/resume. Fixed by a
segmented continuation; see `docs/superpowers/specs/2026-10-10-elya-eval-linear-continuations-design.md`.
Original report:


`deep(n)` (non-tail recursion that keeps a `Cons` per level) under a `handle` takes 0.12 s at
n = 1000, 0.43 s at 2000, 1.70 s at 4000 (debug `elya run`): ~4x per doubling, and n = 100 000
did not finish in 10 minutes. The 7b-3 deep-frames test uses n = 4000 because of this.

## Task 8 review follow-ups (2026-10-03)

- **RESOLVED (commit after 9ac2074): finding the handler is O(depth) per perform.** Replaced by `elya_current_handler`, set by the handle site, re-installed by every resume (the continuation object now records its handler: `[tag][k][consumed][handler]`). 400k deep: >30 s before, 0.7 s after; pinned by `a_deep_non_tail_effectful_recursion_finds_its_handler_in_constant_time`. Original report: `elya_handler_of` walks every frame's `next`
  to the handler frame, so non-tail effectful recursion is quadratic. Measured (review,
  `fn loop(n) { if n == 0 { 0 } else { get() + loop(n - 1) } }`, `resume(1)`): 10k 0.12 s, 20k
  0.5 s, 40k 2.9 s, 80k 15.2 s; 1M did not finish. One perform at depth 80k: 0.009 s. Answers
  are correct. The evaluator is also super-linear here (see the evaluator entry). Fix: give each
  frame O(1) access to its handler (e.g. a handler word in every site frame, or the handler
  passed alongside `k`) -- a frame-layout change, so its own step with its own controls. Must be
  settled before any N = 1 000 000 obligation that recurses non-tail through a perform.
- **Native stack overflow is an unnamed SIGSEGV.** A clause with a NON-tail `resume` over a
  1M-perform tail loop segfaults (rc 139, no message); 100k works. Each nested resume is a
  native frame (D14's nesting call). Plain non-tail recursion at 1M segfaults the same way, so it
  is the existing backend limit; A9/Task 11 must state it and the Linux overflow diagnosis (the
  Task 11 note) must name it.
  *Update (Task 11, 636f98c):* `diagnose_crash` names SIGSEGV as the likely overflow, but only
  A9 uses it; the other deep tests still report a bare exit status on Linux. The 1M
  non-tail-resume limit itself is unchanged.
- Refusals that Task 8 did not lift were renamed from "(Task 8)" to "(not yet compiled
  natively)": effectful lambdas, effectful closure calls, effectful calls inside a `match`.
  *Update (slice 5b-9a, 2026-10-05):* effectful code inside a `match` now compiles natively
  (`MATCH_EFFECTS` corpus + a GC row). *Update (slice 5b-9b):* effectful lambdas and
  closure calls compile natively too. Only s4 -- an effect-polymorphic function used at a
  user effect -- stays refused (D16, N7).

## RESOLVED (commit after e2c112a): a fixed 64K-word threshold made deep live structures quadratic (found 2026-10-03)

`elya_alloc` now collects when allocation since the last collection reaches max(64K, live). 1M list: 45 -> 6 collections; 1M-deep effectful loop: 6.8 s -> 0.4 s. Pinned by `a_growing_live_set_is_collected_a_logarithmic_number_of_times`. Original report:

`elya_alloc` collects every `GC_THRESHOLD_WORDS` (1 << 16) words regardless of the live set,
and every collection marks everything live. With a growing live set the total marking is
O(live^2 / threshold). Measured on the 400k/1M deep effectful loop (1M live frames): 73
collections at 400k (0.7 s), 183 at 1M (6.8 s, live 4M words). Fix in the allocator, NOT in
`gc_mark` (A7): grow the threshold with the live set (e.g. collect when allocation since the
last collection exceeds max(64K, live)). Needs its own step: it changes when collections
happen, which several GC tests calibrate against.

## RESOLVED (slice 5b-10, 2026-10-08): `elya_current_handler` depends on today's handle refusals (review of the lookup fix, 2026-10-03)

D17 is lifted. Every clause and return clause runs with its frame's new `parent` word
current; a perform walks `parent`s to the frame with a clause; a resume re-installs its
frame (`parent`, and `next` for a CPS handle) at the resume. The tail-resume install in
`clause_tail` is now load-bearing and isolated: control K5 fails five `NESTED_HANDLES`
rows. Original:


The global equals "the handler at the end of k's chain" because clauses never perform (D17:
no handle inside a handle or inside an effectful function). When those refusals are lifted, a
clause must run with the global set to the handler OUTSIDE its handle -- a perform must switch
it before jumping to the clause -- and the tail-resume install in `clause_tail` (redundant today)
becomes load-bearing. Revisit both together with D17.

Measured by Task 12 control 1a (6bede80): with re-installation removed from BOTH resume paths,
only programs that resume after their handle returned failed (4 tests); A6 still printed 6.
So no test today isolates the `clause_tail` install, and A6 is not a re-installation witness
in this design. Lifting D17 needs a test with two live handlers where the wrong one is
observable, red first.

## RESOLVED (parked-cleanup-1, 2026-10-04): corpus loops stop at their first failing row (Task 12)

Both Task 9 corpus loops now collect failures (control: two wrong expectations, both
reported). Original:


`the_handler_corpus_compiles_and_runs` and its differential twin assert inside the loop, so
under a control the rows after the first failure are never run (control 1b never reached
`two-handles`). The Task 8 corpus collects failures and reports them all. Make the Task 9 pair
collect too, so a control's full footprint is visible; test-only, no expected value changes.

## RESOLVED (slice 5c-2, 2026-10-05): two clauses for one op in a handler

An error: E0204. Native first-wins (5c-1) stays as a defensive rule, now unreachable from
source. Original:


`handle { .. } with { Ask.ask() -> resume(2)  Ask.ask() -> resume(5) }` checks and runs; the
evaluator takes the first clause (`find`). Natively it printed **5**: each clause overwrote
its op's slot in the clause table, so the LAST won -- an accepted program answering
differently under `run` and `build`. Fixed in 5c-1 to match the reference (first wins),
pinned by the `duplicate-clause-first-wins` native corpus row (red first, 5 vs 2).
Still open: whether a duplicate clause should be an error (likely E0204) instead.

## RESOLVED (slice 5c-3, 2026-10-05): an op used as a value checks clean, then fails at run time

An error now: E0206 (an operation may only be called), with a wrapper fix-it. First-class
ops were declined. Original:


`let f = ping  f()` (op `ping`, no local of that name) passes `elya check` -- inference types
the bare `ping` as `Ty::Error` silently -- and `elya run` ends in E0300 "unbound variable
`ping`". Pre-existing; 5c-2's non-goal. Decide: reject a bare op reference (an error), or
make an op a first-class value (an eta-expanded perform).

## RESOLVED (auto/elya, 2026-10-09: relays, the return clause and re-entered clauses -- spec 2026-10-09-elya-resume-row-complete-design.md): `resume` is typed effect-free -- an escaped resume can perform an unhandled effect (found by slice 5b-10)

`resume` now adds the handled body's labels (minus the handled effect) to the ambient at
the resume site (`tests/resume_row.rs`); the program below is E0420 and m10's
T-carrying variant compiles natively. Still open, same family (found by the review,
`check` clean, evaluator "unhandled effect `t`"): the RETURN clause's effects and those of
clauses the resumed body re-enters are not in resume's row --
`return(x) -> { let y = t()  fn(s) { x + y } }`, and a clause `{ let y = t()  fn(s) {
(resume(s + y))(s) } }` over `get() + get()`; nor are effects the body only relays through
an open row. Including them needs the clause rows before the clauses are typed (a second
pass, or a row variable shared without merging). Original:


**Soundness.** `check` accepts this, and the evaluator stops with "internal: unhandled
effect `t` reached the machine":

```
effect S { fn get() -> Int }
effect T { fn t() -> Int }
pub fn main() -> Int {
  let f = handle {
    handle { get() + t() } with { S.get() -> fn(s) { (resume(s))(s) }  return(x) -> fn(s) { x } }
  } with { T.t() -> resume(10)  return(r) -> r }
  f(5)
}
```

The lambda `fn(s) { (resume(s))(s) }` is typed `fn(Int) -> Int` with an EMPTY row
(measured on its Core node), but calling it runs the rest of the handled body, which
performs T -- here after T's handler has returned. Under deep handlers `resume` should
carry the handle's outer row (the effects the resumed computation may still perform).
Natively the same shape is refused by name ("resume of an effectful handler in direct
code", pinned by `an_escaped_resume_of_a_leaking_handle_is_refused_by_name`); once the
row is carried, that program (m10 in the 5b-10 spec) should compile.

## Front end: recursion through a handle body is rejected (found by slice 5b-10, 2026-10-08)

`fn nest(n: Int) / {T} -> Int { if n == 0 { t() } else { handle { nest(n - 1) + get() }
with { S.get() -> resume(1)  return(r) -> r } } }` is E0423 ("the rows differ by exactly:
{S}") and E0420; unannotated, or split into `nest`/`wrap`, it is E0420. The evaluator runs
it (1000). The handle discharges S, so S should not reach `nest`'s row; recursion inside
the SCC seems to unify the row before the handle subtracts S. It also blocks the native
test of deep dynamic handler nesting (the perform walk over many `parent`s).

## RESOLVED (branch `claude/partial-handlers`, 2026-10-09): a handle with clauses for only some of its effect's ops checks clean (found by the 5b-10 review, 2026-10-08)

The maintainer chose the error: `E0207`, "this handler does not cover every operation of
`S`", with the clauses to add (`tests/handler_coverage.rs`). `cps::Fx` keeps treating a
partial handle as leaking, as defence in depth (unit test
`a_partial_handle_leaks_the_ops_it_does_not_cover`). Original:


The front end types such a handle as discharging the whole effect, so `fn f() -> Int {
handle { get() + put(5) } with { S.get() -> resume(1)  return(r) -> r * 10 } }` is typed
pure; called from `main` with no other handler, `check` is clean and the evaluator stops
with "internal: unhandled effect `put` reached the machine". With an outer handler for
`put` the evaluator forwards it there (1226). Natively such a handle now LEAKS the effect
(`cps::effect_facts` counts an effect handled only when every op the module performs has
a clause), so it compiles as a CPS handle where its region is effectful and is refused by
name where the types call it pure. Language question for the maintainer: require a clause
for every op (an error), or keep forwarding and put the unhandled ops in the handle's row.

## Front end: further soundness gaps found by the resume-row review (2026-10-08)

- **RESOLVED (branch `claude/row-conflicts`, 2026-10-08): effects silently dropped on a
  closed row** -- now E0423 "this effect reaches a row that was already closed"
  (`tests/row_soundness.rs`). The cost: programs that checked clean ONLY by dropping the
  effect are rejected, including ones the evaluator runs under a handler (the recursive
  `go` below, 64). Accepting them soundly needs row subsumption (see the next entry).
  Original: `add_effect`/`add_row` return a
  `RowConflict` when the ambient is already closed, and the perform and call paths discard
  it (`let _ = ...`). `fn go(n) { let f = fn(s) { s + (if n == 0 { 0 } else { go(n - 1) })
  }  f(1) + lg(n) }` is typed `fn(Int) -> Int`: the lambda calls the enclosing recursive
  `go`, its closing step closes `go`'s row early, and the later `lg` is dropped; `main`
  calls it unhandled, `check` is clean, the evaluator stops on unhandled `lg`. Moving `lg`
  before the lambda gives E0420. Surfacing the conflict as a diagnostic is the first step.
- **Type annotations are ignored -- all of them.** The parser's `skip_type_annotation`
  ("Slice 1 has no type checker") discards parameter, return and `let` types:
  `let s: String = 1` and `fn f(x: Int) -> String { x }` check clean. Only effect rows
  (`/ {..}`) are checked. Checking them is a language decision (the annotation syntax for
  function types and type variables, and their scoping): ask the maintainer.
- **An open-row function value called inside a handle body** gets the handled effect forced
  into its row and is rejected (the evaluator prints 12) -- likely the same root as
  "recursion through a handle body".

## Language question: rows unify by EQUALITY, so relays are shared both ways (2026-10-09)

Since a lambda relaying an enclosing parameter keeps its row open (resume-row completion),
a local `let k = fn(x) { f(x) }` used under two different handlers puts both handlers'
effects into `f`'s row (pinned: `a_local_lambda_forwarding_a_parameter_shares_its_row_known_limitation`),
and a stored lambda that relays `body` and performs `z` puts `z` into `task`'s row (r5 in
that spec). Calling `f` directly has always behaved so. Inclusion constraints for relays
(`ambient ⊇ ρ` without equality) would accept both; a real inference change.

## RESOLVED in part (sub-effecting, 2026-10-09): effect rows unify by equality -- sub-effecting? (row-soundness sweep, 2026-10-08)

Rows of function values, `if` branches and clause values unify by EQUALITY (spec 3.6: no
sub-effecting), and a call pours the callee's row into the caller by unifying tails. So a
pure lambda joined to an effectful one closes a row early, and since the sweep the later
effect is a named E0423 rather than silently dropped. Programs the evaluator runs that
are now rejected for this reason: the recursive `go` under a handler
(`tests/row_soundness.rs`), m10 of the 5b-10 spec (clause values `{T}` vs pure), a
handler's `return(x) -> fn(s) { x }` beside an effectful clause value, `if c { fn(s) { s }
} else { fn(s) { s + go(n - 1) } }`. A tried fix (leave a lambda's tail open when it is free
in the environment) accepted some and rejected others base accepted soundly -- a handled
effect leaking into a recursive function's row, and a forwarding lambda forcing its
function's own effect onto the forwarded parameter (`w(k) { let f = fn(s) { k(s) }  f(1)
+ lg(1) }`) -- so it was reverted. Options: (a) keep equality (sound, strict); (b)
sub-effecting at function-value joins (a pure function usable where `{T}` is expected);
(c) full row-constraint inference. Ask the maintainer before any of (b)/(c).

## Annotations: what is not checked yet (the annotations review, 2026-10-09)

- Effect ARGUMENTS in a written row are discarded by `effect_row()`: `k: fn(Int) /
  {State(Bool)} -> Int` is accepted while the state is Int (not unsound: the argument is
  inferred consistently), and `/ {State(Int), State(Bool)}` merges silently. Same for
  top-level `/ {..}` rows, as always.
- A parameter or lambda-parameter mismatch is reported at the uses, not at the annotation
  (the annotation is unified with a fresh variable before the body is inferred).
- Type variables are flexible, not rigid: `fn f(x: Int) -> a` is accepted. Rigid
  variables behind an explicit `forall` would need skolems.
- Function-typed ADT fields and effect-operation parameters are a named error ("not
  supported yet"): they need a row story for type declarations.

