# Slice 5b-6 — Native Closures (arc node N5) — Design

**Status:** design approved in brainstorming; awaiting spec review.
**Predecessor:** Slice 5b-5 (native mark-sweep GC), closed at `b6050e9` + close-out.
**Successor candidates:** N6 (strings/io), N7 (runtime polymorphism), N8 (effects).

## 0. What this slice ships

A lambda written in Elya compiles to native code: it is heap-allocated, it
captures exactly the variables it uses, it is called through a loaded code
pointer, the call is a guaranteed tail call when it is in tail position, and the
collector traces its captures precisely.

```elya
pub fn main() {
  let k = 10
  let add = fn(x) { x + k }
  add(32)          // 42, natively
}
```

Out of scope, deliberately, each with its reason in the section that owns it:

- **Recursive lambdas** (§7). Not a simplification — Elya's binding forms make a
  self-capturing value *unconstructible*, so there is nothing to ship.
- **First-class top-level functions** (§6). The `"function used as a value"`
  refusal stays; uniform representation is N7's problem, where it is actually
  forced.
- **Polymorphic lambdas.** `repr_ty` still refuses a type variable through its catch-all arm; that is N7
  knocking, unchanged from 5b-4.
- **Closures capturing affine values.** The front end's `E0428`/`E0429` work is
  independent of representation and unaffected by this slice.

---

## 1. Finding 0 — the C-ii probe: indirect `musttail` is safe, and the arity boundary does not move

The 5b-3 spec closes its arity discussion with "the cap counts *parameters*, not
fields — so N4's ADTs and **N5's closures** do not tighten it." That sentence is
an argument about *fields*. Closure conversion is a change to *parameters*: it
prepends an environment argument to every lambda's signature. The claim was
therefore re-derived by measurement, not inherited.

**Environment.** `llvm-as` / `llc` **18.1.6** (the version inkwell links, from
the vcpkg LLVM tree — not the 22.x clang on `PATH`, which serves only as link
driver), target `x86_64-pc-windows-msvc`, `llc -O0`.

### 1.1 `musttail` through a loaded code pointer is legal and real

The probe used the N5 shape exactly: the closure pointer *is* the environment
argument, the code pointer is loaded out of the block, and the call goes through
the loaded value. Verifier accepts; backend accepts; the emitted instruction is
`rex64 jmpq *%rax  # TAILCALL` — a jump, not a call.

Every case carries a built-and-run negative control, because a differential
test's teeth are proven by building the unfixed version and watching it fail
differently:

| probe | shape | exit | stdout |
|---|---|---|---|
| P1 | self-recursive closure, capture read through env, 1e6 deep | `0x00000000` | `7777` |
| P1N | identical, `musttail` → plain `call` | `0xC00000FD` | — |
| P2 | **mutual** — two cross-linked closure blocks, 1000001 deep | `0x00000000` | `0` |
| P2N | identical, plain `call` | `0xC00000FD` | — |
| P3 | mutual, **at the cap this slice adopts**: env + 4 params = arity 5 | `0x00000000` | `0` |
| P3N | identical, plain `call` | `0xC00000FD` | — |

`0xC00000FD` is `STATUS_STACK_OVERFLOW`. P2 and P3 are the load-bearing rows:
self-recursion can be faked by a loop-rewrite, mutual recursion cannot.

### 1.2 The arity matrix is byte-identical with the env parameter counted

Both 8×8 sweeps (direct callee, indirect callee) were re-run fresh and agree
cell for cell:

```
         callee=  1  2  3  4  5  6  7  8
caller=1 |  .  .  .  .  .  X  X  X
caller=2 |  .  .  .  .  .  X  X  X
caller=3 |  .  .  .  .  .  X  X  X
caller=4 |  .  .  .  .  .  X  X  X
caller=5 |  .  .  .  .  .  X  X  X
caller=6 |  .  .  .  .  .  .  .  X
caller=7 |  .  .  .  .  .  .  .  X
caller=8 |  .  .  .  .  .  .  .  .
```

Indirection does not move the boundary. Counting the environment pointer against
the *existing* cap is therefore sufficient, and **no new mechanism is needed**.

`X` is a hard abort — `LLVM ERROR: Can't handle guaranteed tail call under win64
yet`, raised in `Prologue/Epilogue Insertion & Frame Finalization`, with no
`Result` to catch — which is why §8's check runs before any emission begins.

Two integrity checks on the accepted cells, since a silent degrade would void
the guarantee exactly where the cap permits it:

- Disassembly of `i1_4`, `i1_5`, `i5_5`, `i6_7`, `i8_8` — all `jmpq *%rax`;
  `d1_5` — `jmp callee`; P3's two tail sites — `rex64 jmpq *%r10`. No accepted
  cell degrades to a call.
- **P4**, the first cell past the cap (a 5-parameter lambda, converted arity 6,
  tail-called from an arity-5 closure body): the verifier *accepts* it and `llc`
  aborts. The cap is load-bearing, and violating it can never ship a binary.

### 1.3 Correction to the archived 5b-3 record

The 5b-3 spec's prose rule — "Safe iff **callee arity ≤ 5**, or **caller arity ≥
callee arity**" — contradicts **its own table** at exactly one cell,
(caller = 6, callee = 7): the prose predicts failure, the table prints `.`, and
both the archived `m6_7.obj` and a fresh run produce an object file.

The divergence is in the conservative direction — the prose refuses something
that works — and it is the *only* divergence: the prose never accepts a cell
that fails. Nothing built on it is unsafe, and 5b-3's flat ≤5 cap is unaffected,
sitting well inside the always-safe band.

No win64 stack-byte model reproduces the measured `K = 5` row, so the mechanism
sentence in 5b-3 ("the callee's stack-argument bytes, rounded to 16, not
exceeding the caller's") is not a derivation this project can stand behind.
**This spec carries the measured predicate, not a derived rule:**

> `K ≤ 5` → always safe. `K ∈ {6, 7}` → safe iff `C ≥ 6`. `K = 8` → safe iff
> `C ≥ 8`. Outside the measured range 1–8, nothing is claimed.

Same discipline as `K_MAX`: pin what was measured.

---

## 2. Closure representation — one flat heap block

A closure is **one** heap object in the existing allocator, laid out in the
existing shape. Payload:

```
[0] tag        -- a synthetic tag, unique per lambda SITE
[1] code_ptr   -- the lifted function, a pointer into .text
[2] cap_0
[3] cap_1
[2+i] cap_i   -- one word per capture, in the deterministic order of §3.1
```

That is `elya_alloc(2 + n_captures)`, mirroring `CoreKind::Ctor`'s
`elya_alloc(1 + n_fields)` exactly one word wider.

**Rejected: a two-block closure** (a code/arity header pointing at a separate
environment record). It doubles the allocation count and the pointer chase for
no expressiveness this slice needs, and it adds a second object shape the
collector must recognise.

### 2.1 One synthetic descriptor row per lambda site — and `gc_mark` does not change

`gc_mark` today reads an object's tag, indexes `gc_descriptors[2*tag]` /
`[2*tag+1]` for `(arity, ptr_mask)`, and traces field `f` iff bit `f` of the
mask is set. A closure participates in that scheme *as written*:

- **arity** = `1 + n_captures` (the code pointer counts as field 0).
- **mask bit 0 = 0.** The code pointer is not a heap pointer; leaving its bit
  clear is what keeps it untraced. This is the same mechanism that already stops
  an `Int` field from being mistaken for a pointer — no new case.
- **mask bit `i+1`** set iff capture `i` is a heap pointer (see §5).

So `gc_mark` stays **byte-identical**. This was the deciding argument. The
alternative considered and **rejected** was a second dispatch path inside
`gc_mark` — a reserved tag range meaning "closure", handled separately. A second
branch in the one function whose failure mode is silent is exactly what not to
add: `gc_mark`'s existing guard is `if (tag < 0 || tag >= gc_n_ctors) continue;`,
which *skips* an object it cannot interpret. An object skipped there has its
captures untraced and its live children swept. There is no crash, no diagnostic,
and no test that fails on the day it breaks — only wrong answers later.

Synthetic tags are appended to the table `build_ctor_table` builds, continuing
the same running global counter, so a closure's tag indexes a real row by
construction and `gc_n_ctors` covers it. See §5, which is where getting this
wrong is caught.

---

## 3. Closure conversion — capture exactly the free variables

### 3.1 The free-variable walk is a standalone function

```rust
/// The free variables of `e`, in deterministic order.
/// Excludes top-level function names and constructor names — those are globals,
/// not captures.
fn free_vars(e: &CoreExpr, globals: &HashSet<String>) -> BTreeSet<String>
```

It is a plain function over `CoreExpr` with no builder, no context, and no LLVM
in its signature, so it is **unit-testable in isolation** — the binding forms
can be tested one at a time against hand-built Core, in the same style as the
existing inline codegen tests that construct Core by hand.

Binders it must respect:

- `CoreKind::Let(name, value, body)` — `name` is bound in `body`, **not** in
  `value`. This is the same asymmetry §7 turns on.
- `CoreKind::Lambda(params, body)` — the parameters are bound in `body`; a
  nested lambda's own free variables propagate outward minus its parameters.
- `CoreKind::Match` arm patterns — every variable a pattern binds is bound in
  that arm's body only.
- `CoreKind::Var(x)` where `x ∈ globals` — not a capture.

Every other variant recurses structurally, and the match must be **exhaustive
with no catch-all arm**, so a variant added by a later slice is a compile error
rather than a silently dropped capture. Under-capture is the mirror of §3.2's
over-capture and strictly worse: over-capture retains too much, under-capture
emits a lambda that reads an environment slot which was never stored.

`BTreeSet` and not `HashSet`: capture order must be deterministic, because the
descriptor row's mask bits and the emitted stores must agree, and a
nondeterministic order would make that agreement a coin flip that usually lands
right. Lexicographic order is pinned by the type.

### 3.2 Named and rejected: capture everything in scope

The obvious shortcut is to snapshot every binding live at the lambda site and
skip the analysis. **This is rejected, and named here so it is not reinvented as
an optimisation-free simplification.**

An over-broad environment is a leak the collector cannot see through. The tracer
traces precisely what the descriptor says is there; it has no way to know a
captured slot is unreachable from the lambda's body. So capturing everything
retains arbitrarily large object graphs for the closure's whole lifetime, with
no action the program can take to drop them — and the retention is invisible in
every observable except peak memory. Precision here is a **space guarantee**,
not a performance tweak, and it is the same guarantee 5b-5 was built to deliver.

---

## 4. Discharging T3 structurally — lambda parameters carry types

> **T3 (open):** lambda parameters carry no types in Core.

`CoreKind::Lambda(Rc<[String]>, Rc<CoreExpr>)` stores names only, because
`core.rs`'s lambda lowering does `params.iter().map(|p| p.node.name.clone())`
and throws the types away. The back end needs a parameter's type to pick its
LLVM representation.

**The assumption is not buried in the back end.** It is discharged where the
information exists:

1. **Record the types.** In `types.rs`'s `Expr::Lambda` arm, each parameter
   already gets a fresh type variable `pv`. Insert it at the parameter's own
   span — `self.node_types.insert(p.span, pv.clone())` — exactly as 5b-3 does for
   top-level function parameters. Recorded pre-zonk like every other entry; the
   single zonk pass at the end maps over the whole table, so these resolve for
   free.
2. **Reshape the Core node.** `CoreKind::Lambda(Rc<[CoreParam]>, Rc<CoreExpr>)`,
   reusing the existing `CoreParam { name, ty }` that `CoreFn` already uses. The
   lowering reads `table[p.span]` the same way it reads any expression's type.

The rejected alternative was to have the back end infer lambda parameter types
from the lambda's own `Ty::Fn(args, _, _)` node type. It would work today. It
would also mean the back end reconstructs a fact the front end deleted, and the
reconstruction would rest on an unstated invariant — that the positional order
of `Ty::Fn`'s argument list matches the parameter name list — with nothing
checking it. T3 closes properly or not at all.

This also feeds the recorder-totality class: `Expr::Lambda`'s parameter spans
become entries lowering depends on, so the existing `node_types` cross-check
covers them.

---

## 5. CR-3 — the silent failure, closed in both directions at once

This is the only failure in this slice that produces no crash, no diagnostic,
and no failing test — just freed-while-live memory and wrong answers later. It
is closed in **both** directions, and the two are inseparable:

**Direction (a) — the closure's own row must exist.** Synthetic tags are
appended to the descriptor table in the same global tag order
`build_ctor_table` assigns, and `gc_n_ctors` is computed from the full table
including them. If a closure tag ever lands at or beyond `gc_n_ctors`,
`gc_mark`'s `tag >= gc_n_ctors` guard skips the object and its captures are
never traced.

**Direction (b) — the mask predicate must widen.** The descriptor mask is built
with

```rust
if matches!(f, Ty::Con(..)) { mask |= 1 << i; }
```

Before this slice that was total: `Ty::Con` was the only heap representation.
It stops being total the moment a closure exists, because a value of **function
type is now a heap pointer too**. A closure captured inside another closure, or
stored in an ADT field of function type, would have its mask bit clear and go
untraced. So the predicate becomes

```rust
if matches!(f, Ty::Con(..) | Ty::Fn(..)) { mask |= 1 << i; }
```

and `repr_ty` gains the matching arm — `Ty::Fn(..) => ptr` — so the two agree.
The invariant to state once and hold: **`repr_ty` says pointer ⟺ the mask bit is
set.** Any future type that becomes heap-represented must touch both, and this
is the sentence that says so.

Shipping (a) without (b) is the dangerous half-fix: closures would be traced,
captured closures would not, and the corpus would pass.

### 5.1 How this is proven rather than argued

A silent failure needs a test that fails loudly *before* the fix exists. The
acceptance test builds a closure that captures another closure which captures a
heap ADT, forces at least one collection (the `GC_THRESHOLD_WORDS = 1 << 16`
threshold is reached by an allocating loop), then calls through both and reads
the ADT's payload. Run against the un-widened mask, it reads freed memory and
must fail differently — that failure is observed and recorded when the test is
written, in the 5b-5 discipline. Passing on the fixed build alone proves nothing.

---

## 6. First-class top-level functions — the thin cut

The `"function used as a value"` refusal in `lower_expr` **stays**. A bare
top-level function name in value position is still rejected by name.

This is a deliberate thin cut, not an oversight. Making `fn f(..)` usable as a
value means giving it the same runtime shape as a lambda — either wrapping it in
a zero-capture closure block at every use, or eta-expanding it — and then
`build_elya_call` must decide at every call site whether the callee is a direct
symbol or a block. That is a *uniform representation* problem, and N7 (runtime
polymorphism) is where it is genuinely forced, because a polymorphic call site
cannot know which it has.

**Intended end state, recorded so it is not relitigated:** top-level functions
and lambdas converge on the closure representation of §2, with a zero-capture
block for a top-level function, and the direct-call path in `build_elya_call`
kept as an optimisation for the statically-known-symbol case rather than as the
only path. This slice does not build it, and a future slice should not re-derive
the decision from scratch.

---

## 7. T7 — re-filed on a corrected premise, redirected at N8

T7 as written in the 5b-5 spec (L110, L118) rests on a false premise: that
self-capturing closures make cycles constructible, and therefore that the
refcounting evaluator and the tracing back end can diverge on reclaim.

**The premise is false, and the evidence is `resolve.rs:102-104`:**

```rust
Stmt::Let { name, value } => {
    self.check_expr(&value.node, value.span, scope);
    scope.last_mut().unwrap().insert(name.clone());
}
```

A `let`'s right-hand side is resolved **before** the name enters scope. A lambda
on the right-hand side therefore cannot refer to the name being bound; there is
no expression-level `letrec`; and no other binding form introduces a name into
its own initialiser. **Elya's value graph is acyclic by construction of the
binding forms** — a strictly stronger statement than T7's current wording, and
one that holds independently of whether closures exist.

T7 is re-filed with that corrected statement, and redirected:

> **T7 (re-filed).** Elya's value graph is acyclic by construction of the
> binding forms (`resolve.rs:102-104`; no expression-level `letrec`). The
> refcounting evaluator and the tracing back end therefore cannot diverge on
> reclaim in any program expressible today. The likeliest future arrival point
> for cycles is **N8**: a `Value::Resume` holding captured frames could close a
> loop through a handler. **`? INFERRED`** — this is a reading of the intended
> N8 shape, not of code that exists.

### 7.1 The measurement is built now, not the day it is needed

Because the arrival point is inferred rather than known, the instrument goes in
now, while the answer is known to be "acyclic" and a baseline can be pinned
honestly.

`elya_gc_report` currently reports `collections`, `freed`, and `words_since_gc`
under `ELY_GC_STATS`. All three are *flow* figures. None of them observes
**space**, so none of them can show a leak. The slice adds a live-words
accumulator computed during `gc_sweep` (the sum of surviving block sizes) and
reports it:

```
elya-gc: collections=N freed=N words_since_gc=N live=N
```

The acceptance test runs a compiled program that allocates unboundedly in a loop
while retaining a bounded working set, and asserts `live` settles at a pinned
bound across many collections — a measured constant, pinned in the `K_MAX`
discipline, never nudged to make a test pass.

Today that test passes and proves the current claim. The day N8 makes a cycle
constructible, the same instrument is the thing that shows whether the tracer
still bounds `live` while the evaluator's refcounts do not. The divergence gets
measured on arrival instead of argued about.

---

## 8. The arity cap — lambdas take at most 4 parameters

Converted arity is `1 + params.len()` (the environment pointer is parameter 0).
Capping lambdas at **4 parameters** puts every closure's converted signature at
arity ≤ 5, which §1.2 measured as safe **for every caller arity**, direct or
indirect. No call-site analysis, safe by construction, entirely inside measured
ground — the same reasoning that chose 5b-3's flat cap over its predicate.

Enforcement mirrors T1's exactly, and for the same reason: the failure is
LLVM's `report_fatal_error`, which kills the process with no span and no
`Result`, so it must be impossible for any emission to have begun when the check
fires. The existing whole-module scan in `build_module` — currently
`f.params.len() > MAX_PARAMS` over `core.fns` — is extended to walk every
function body for `CoreKind::Lambda` sites and refuse
`params.len() > MAX_PARAMS - 1`, with its own message:

```
"lambda takes more than four parameters"
```

Distinct wording from the top-level refusal, because the numbers differ and a
shared message would make the environment parameter invisible to whoever hits
it. T1 stays open and now covers both forms.

---

## 9. The call path

`build_elya_call` currently refuses a non-`Var` callee as `"computed callee"`
and a `Var` absent from `lc.decls` as `"callee is not a top-level function"`.
N5 adds one path and keeps both refusals for what they still cover.

**Dispatch is by binding, not by guesswork:** a callee that is a `Var` present
in the local `env` map, whose own `CoreExpr.ty` is `Ty::Fn(..)`, is a closure
call. Everything else keeps its existing behaviour.

Note the type is read **inline off the Core node**, not looked up by span. The
back end holds no `node_types` table — `CoreExpr` carries `ty` as a field, and
`core.rs:20` states the rule the other way round for spans: *"Provenance
(diagnostics + the §5 lookup cross-check); never a type key."* Span lookup is
the *lowerer's* mechanism (`lower_module`/`lower_expr` take
`table: &BTreeMap<Span, Ty>`), which is why §4 records at parameter spans and
this section does not.

The emitted sequence:

1. Lower the callee expression to the closure block pointer.
2. Load the code pointer from payload word 1 — descriptor field 0, the same
   slot §2.1 leaves untraced.
3. Call through it with the **closure pointer itself as argument 0**, followed by
   the source-level arguments.

### 9.1 Why the environment argument is the closure pointer

This is what makes the tail case require nothing new. `build_elya_call`'s
existing asymmetry — `let env_roots = if tail { 0 } else { gc_root_env(..) }` —
exists because a `musttail` call must be immediately followed by `ret`, leaving
nowhere to pop a shadow-stack entry, and because a million tail iterations would
otherwise grow the shadow stack without bound.

Passing the closure block as argument 0 means it is **live as an argument**. It
travels the same path every other pointer argument already travels: rooted as it
is lowered for a non-tail call, unrooted LIFO before the call, and handed over
unrooted because the callee roots its own parameters at its own first allocation
site and nothing allocates in between. There is nothing extra to root at the
tail boundary, and N2's constant-stack guarantee is untouched.

The rejected alternative was a separate environment pointer loaded out of the
block and passed alongside it, which would have made the block itself
unreachable from the argument list at the moment of the jump — a root the
collector cannot see, precisely where there is nowhere to put one.

### 9.2 Lifting

Each `CoreKind::Lambda` site is lifted to a module-level `tailcc` function, with
the environment pointer prepended to the parameter list. Captures are read out
of the environment inside the lifted body; the body's own `env` map binds each
captured name to the loaded value. Because captures are loaded rather than
threaded, a lifted body's rooting discipline is the same as any other
function's.

**The lifted name is pinned, not left to taste:**
`<enclosing_fn>.lambda.<n>`, where `n` is the 0-based index of the site in a
**pre-order traversal** of the enclosing function's body. Two properties make
this the right scheme:

- **It cannot collide with a user-written name.** `.` is Elya's access operator
  (`io.println`), so it cannot appear in a function identifier. This matters
  because the existing guard at `lib.rs:199-203` exists for exactly this hazard:
  *"LLVM silently uniquifies a duplicate symbol (`elya_f.1`) rather than
  complaining, which would give us two functions where Core has one."* Lifted
  functions go through that same duplicate check and the same `mangle`
  (`elya_{name}`, a pure prefix, so unmangled uniqueness implies mangled
  uniqueness).
- **It is the same traversal that assigns synthetic tags** (§2.1). One order, not
  two — a lambda's lifted symbol and its descriptor row are indexed by the same
  walk, so they cannot drift apart.

---

## 10. Testing

The proof-is-execution rule holds: no IR snapshots, no `#[ignore]`. Every claim
below is a program that is compiled, linked, run, and checked against the
evaluator by the existing differential harness.

1. **`free_vars` unit tests** — one per binding form, on hand-built Core:
   `Let`'s value/body asymmetry, nested lambda shadowing, match-arm pattern
   binders, globals excluded, deterministic order.
2. **Basic capture** — the `add` example of §0: correct value, differential
   agreement with `elya run`.
3. **Capture of a heap value across a collection** — a closure capturing an ADT,
   with an allocating loop between construction and call that forces at least
   one collection. This is the direction-**(a)** test, and the distinction is
   load-bearing: an ADT capture's mask bit is *already* set by the un-widened
   `Ty::Con` predicate, so this test says nothing about (b). What it catches is a
   **missing synthetic descriptor row** — with the closure's tag at or beyond
   `gc_n_ctors`, `gc_mark`'s guard skips the object, the ADT goes untraced and is
   swept while live. Built that way first, and the failure observed and recorded.
4. **Nested closure capture** (§5.1) — the direction-**(b)** test, and the only
   one that exercises the widened predicate: a closure capturing a closure
   capturing an ADT, built first against the un-widened mask and observed
   failing.
5. **Tail-call guarantee, mutual** — two closures calling each other in tail
   position at 1e6 depth, asserting exit 0 and the correct answer. This is the
   primary tail-call criterion; the self-recursive case is secondary because a
   loop-rewrite can fake it. The negative control (plain call → `0xC00000FD`) is
   built and observed, not argued.
6. **Arity refusal** — a 5-parameter lambda is refused by name, before emission,
   with `"lambda takes more than four parameters"`.
7. **Space observation** (§7.1) — `live` settles at a pinned bound across many
   collections.
8. **The refusals that stay** — `"function used as a value"` for a top-level
   function name in value position; a polymorphic lambda still refused by `repr_ty`.

---

## 11. Obligations ledger after this slice

| ID | State | Note |
|---|---|---|
| T1 | **open, widened** | Now covers lambda sites (≤ 4 params) as well as top-level fns (≤ 5). Measured predicate of §1.3 recorded; padding still deferred to real demand. |
| T2 | open | Whole-module `repr_ty` strictness — unchanged. |
| T3 | **closed** | Discharged structurally in §4: types recorded at parameter spans, `CoreKind::Lambda` reshaped to `Rc<[CoreParam]>`. |
| T7 | **re-filed, redirected** | §7: premise corrected to "acyclic by construction of the binding forms"; redirected at N8's `Value::Resume`, marked `? INFERRED`; measurement built now. |
| **new** | open | **Uniform function representation** (§6): top-level functions converge on the closure shape at N7. Intended end state recorded so it is not relitigated. |
| — | noted | `GC_MAX_WORDS = 16` bounds the size-segregated free lists; a closure with many captures exceeds it and falls to the general path. **Measured and noted, not nudged** — the constant is not moved to accommodate this slice. |

Unchanged and carried forward: the relay+own-effect row leak, the affine
callee-duplication soundness gap, the affine intra-procedural
over-approximation, and the perform-callee recorder gap.
