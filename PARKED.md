# Parked

- Consider RelocMode::PIC + PIE linking on all platforms. Needs a Windows gate run before landing.
- Before fast-forwarding main to the 5b-8 branch: run scripts/check.ps1 on Windows and get exit 0. Commits from 65b08e4 onward have only been gated on Linux.
  - The ≤4 source-param cap for CPS functions (5 + continuation) is untested on win64, where >5 tailcc params caused a fatal abort. Verify it there.

## Unqualified handler clauses (front-end/evaluator disagreement)

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

## Front end: should a fn sharing a name with an effect op be an error?

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

## Slice close-out doc fixes

- Spec §6.3 says both existing guards "must be extended" to cover the frame
  tag; b370191 left both unchanged and added a separate frame guard of the same
  shape. (A8's own row was rewritten by plan D10 on 2026-10-03; §6.3's sentence
  still says "extended".)

## Op → effect index: one helper, one copy left

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

## Evaluator: non-tail recursion under a handler is quadratic (found 2026-10-03)

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
- Refusals that Task 8 did not lift were renamed from "(Task 8)" to "(not yet compiled
  natively)": effectful lambdas, effectful closure calls, effectful calls inside a `match`.

## RESOLVED (commit after e2c112a): a fixed 64K-word threshold made deep live structures quadratic (found 2026-10-03)

`elya_alloc` now collects when allocation since the last collection reaches max(64K, live). 1M list: 45 -> 6 collections; 1M-deep effectful loop: 6.8 s -> 0.4 s. Pinned by `a_growing_live_set_is_collected_a_logarithmic_number_of_times`. Original report:

`elya_alloc` collects every `GC_THRESHOLD_WORDS` (1 << 16) words regardless of the live set,
and every collection marks everything live. With a growing live set the total marking is
O(live^2 / threshold). Measured on the 400k/1M deep effectful loop (1M live frames): 73
collections at 400k (0.7 s), 183 at 1M (6.8 s, live 4M words). Fix in the allocator, NOT in
`gc_mark` (A7): grow the threshold with the live set (e.g. collect when allocation since the
last collection exceeds max(64K, live)). Needs its own step: it changes when collections
happen, which several GC tests calibrate against.

## `elya_current_handler` depends on today's handle refusals (review of the lookup fix, 2026-10-03)

The global equals "the handler at the end of k's chain" because clauses never perform (D17:
no handle inside a handle or inside an effectful function). When those refusals are lifted, a
clause must run with the global set to the handler OUTSIDE its handle -- a perform must switch
it before jumping to the clause -- and the tail-resume install in `clause_tail` (redundant today)
becomes load-bearing. Revisit both together with D17.
