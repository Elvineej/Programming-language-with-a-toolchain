# Slice 5b-8 — native effect handlers (arc node N8)

**Design spec. Status: written 2026-09-18, awaiting review.**

Every factual claim about the codebase below is marked. **✓ VERIFIED** means the
file was read and the code traced; **? INFERRED** means it rests on a search or
on a reading of intent; **✗ UNCERTAIN** means it has not been checked and must be
checked during planning. Nothing in the acceptance criteria (§11) rests on an
unverified claim.

---

## 0. The cut

**One-shot handlers, monomorphic effects, general resume position.**

In scope:

- `handle { … } with { E.op(x) -> …, return v -> … }` compiled to native code,
  for a **monomorphic, user-declared** effect.
- `resume(e)` in **general position** — not restricted to the tail of a clause.
- `with multi` **refused by name at codegen**, with an execution test.
- `Ty::Var` stays refused, unchanged, so N7 (runtime polymorphism) remains an
  independent slice rather than a prerequisite.

Deferred, named so they are not re-derived:

- Copyable multi-shot continuations (`with multi` executing natively).
- Polymorphic effects (`effect E[a] { … }`) — N7's territory.
- Effect rows as a runtime artifact. Rows stay a compile-time object.

**Why general resume position is the expensive half, and why it is the point.**
A tail-resumptive handler — one whose clause bodies end in `resume(e)` — needs no
reified continuation at all: the clause can compile to an ordinary call and the
machine stack does the splicing. Taking that cut would produce a slice that
compiles the *syntax* of handlers while proving nothing about the *mechanism*,
and would leave the whole of N8's difficulty for a later slice under a name that
sounds already-done. General position forces the continuation to become a value.
That is the thing N8 is for.

**Scope, honestly: this is bounded, not small.** Every prior arc node reached for
machinery that already existed — 5b-6 reused the descriptor mechanism for
closures, 5b-7 added one leaf heap shape and one builtin. N8 does not. It invents,
for the first time:

1. **A Core effect vocabulary** — three or four new `CoreKind` variants, each
   owing the type-carrying discipline the frozen span→`Ty` table imposes (§3.2's
   `$k` is the first case where a synthesised node's type must be *cloned from a
   neighbour* rather than looked up, and it will not be the last).
2. **A calling convention for handler clauses** — clauses are not functions in the
   existing sense; they receive an operation's arguments *and* a continuation, and
   they may return to a trampoline rather than to their caller (§8.3). *(2026-10-03:
   no runtime trampoline — every transfer is a `musttail` jump; §13 D14.)*
3. **The first heterogeneous pointer-bearing heap object** — every heap shape so
   far has been homogeneous in what it points at. A frame cell points at a next
   frame *and* at saved values of unrelated types (§6.1).

The cut is drawn to bound each of these, not to make any of them disappear. What
keeps it bounded is that the fixed-arity choice (§6.2) buys the third item at the
price of *zero* new mechanism in the collector — which is why §6.2 is a load-bearing
decision and not a micro-optimisation.

---

## 1. T7 — closed, and the premise it was closed on

T7 has been carried since 5b-5 and was re-filed in the 5b-6 spec (§7) pointing at
this slice: *"the likeliest future arrival point for cycles is **N8**: a
`Value::Resume` holding captured frames could close a loop through a handler,"*
explicitly marked `? INFERRED` — *"a reading of the intended N8 shape, not of
code that exists."*

**The reading has now been done, and the inference is false.** ✓ VERIFIED:

| Fact | Site |
|---|---|
| `ResumeData { captured: Vec<Frame>, handler: Rc<Handler>, ret_env: Env, consumed: Cell<bool> }` | `src/eval.rs:495-500` |
| `ret_env` is the **handler's** environment, captured before `$resume` is bound | `src/eval.rs:495-500` |
| `struct Scope { vars: HashMap<String, Value>, parent: Option<Rc<Scope>> }` — no interior mutability | `src/eval.rs:133-137` |
| `Env::extend` allocates a fresh `Rc<Scope>` whose `parent` is the *existing* chain — an edge can only point at a scope older than itself | `src/eval.rs:148-157` |
| The sole `Cell` in `src/eval.rs` holds a `bool` (`consumed`) | `src/eval.rs:499`, only `Cell<` match in the file |

A continuation is a `Vec<Frame>` plus an `Env` that was already complete when the
continuation was built. There is no write-back, no fixup pass, and no mutable
cell holding a value. **A captured continuation cannot close a cycle.**

**T7 is closed, not carried into this slice.** It is closed on the *binding
forms* — the same premise on which the ADT case and the closure case were closed,
which makes this the third time the argument has held: ADTs, closures,
continuations, each verified from the forms rather than from the values. The 5b-6
spec's §7 has been corrected in place to say so rather than quietly re-scoped,
following the precedent 5b-7 set.

**What survives T7:** the live-words instrument built in 5b-6 §7.1 for an
obligation that turned out not to exist. It is not discarded; §7 below reuses it
against a different and real obligation. An instrument built for a hazard that
evaporated is still an instrument.

**Close-out obligation:** the out-of-repo ledger's line *"**T7** still points at
N8"* becomes false when this spec lands, and must be updated at close-out.

---

## 2. Two premise corrections, recorded

These are recorded because both cut against things previously written down.

### 2.1 `resume` is a syntactic form, not a callable value

✓ VERIFIED at `src/parse.rs:186-197`: the parser bumps `resume`, then **requires**
`LParen` (`"expected ( after resume"`, returning `None` otherwise), parses exactly
one expression, requires `RParen`, and builds a dedicated AST node
`Expr::Resume { arg }`. ✓ VERIFIED at `src/types.rs:155, 947, 1290, 1293`: it is
typed from `resume_stack: Vec<(Ty, Ty)>`, pushed around a clause body and popped
after, read via `resume_stack.last()`. **`resume` never enters `TyEnv`.** It has
no binding, no name, and no type-environment entry.

`Value::Resume`, the `$resume` binder, and `Frame::ResumeApply` are
**evaluator-internal**. Reasoning from `Value::Resume` to "resume is a
first-class function value, therefore handlers need first-class functions,
therefore N7 comes first" reads the evaluator's implementation as if it were the
language's surface.

**Consequence:** N7 is **not** a prerequisite for N8. It is a prerequisite only
for *polymorphic* effects, which this cut excludes. The collision between the two
slices is avoidable, not inherent, and this cut avoids it.

### 2.2 Acyclicity is a property of the binding forms

Recorded in §1. The point worth preserving separately is the *method*: three
times now, a cycle hazard has been argued away not by inspecting values but by
inspecting the forms that can introduce a name into its own initialiser. There is
no expression-level `letrec`; `let`'s right-hand side is resolved before the name
enters scope (`src/resolve.rs:102-104`, ✓ VERIFIED in 5b-6); environments are
append-only. **The next cycle hazard should be tested the same way, and should be
expected to fail the same way.**

---

## 3. Two questions settled by reading rather than assumption

Both were flagged as load-bearing for the fidelity corpus and required
confirmation rather than assumption.

### 3.1 Clause bodies are free; handle bodies are not. The question splits.

**The question was asked as one and has two different answers.** Both ✓ VERIFIED.
The half that was expected to be expensive is free; the half nobody asked about
is the one that costs something.

**(a) Clause bodies: free.** `OpClause.body` and `ReturnClause.body` are
`Rc<Spanned<Expr>>` (`src/ast.rs:96, 102`) — **not** `Rc<Spanned<Block>>`, which
is what `Expr::Lambda` carries. ✓ VERIFIED at `src/parse.rs:753` (return clause)
and `src/parse.rs:840` (op clause): both are parsed by `self.expr(0)`. So a clause
body is lowered by `lower_expr`, and **no fifth Core node is needed for it.** *(2026-10-03:
performs DO get a node of their own, `CoreKind::Perform` — a third new Core node, §13
D11. Clause bodies still need none.)* The
corpus's actual handler shape —

```
State.get() -> fn(s) { … }
```

— is an `Expr::Lambda` whose `Block` body routes through the **existing** Lambda
arm at `src/core.rs:327`. The `Unsupported("Block")` refusal is not on this path.

**(b) Handle bodies: they do hit the refusal.** `Expr::Handle` carries
`body: Rc<Spanned<Expr>>` (`src/ast.rs:192`) — an `Expr`, like the clauses, which
is why this half hides. But the *surface* decides what that `Expr` is:

| Step | Site |
|---|---|
| `handle_expr` parses its body with `self.expr(0)` | `src/parse.rs:713` |
| a leading `{` in expression position dispatches to `block_expr()` | `src/parse.rs:125` |
| `block_expr()` returns `Expr::Block(b.node)` | `src/parse.rs:1092-1095` |
| the corpus writes `handle { run() } with {` and `handle { loop(n) } with {` | `tests/state_effect.rs:40, 64, 103` |
| `Expr::Block(_) => Unsupported("Block")` | `src/core.rs:347` |

**So every handle in the fidelity corpus lowers to a refusal today.** Being an
`Expr` was necessary but not sufficient; the brace makes it the one `Expr` variant
that is still refused.

**The cost is still not a fifth Core node.** `lower_block` returns a whole
`CoreExpr` (`src/core.rs:207-234`) — it folds the statement list into a `Let`
chain around the tail — and is already called from four sites: function bodies
(`:175`), both `if` branches (`:308`, `:309`), and lambda bodies (`:327`). The fix
is **one arm delegating `Expr::Block(b)` to `lower_block(b, …)`**, an early return
in the same shape as the existing four callers.

Two consequences, stated so they do not arrive silently:

- **This un-refuses `Expr::Block` in *every* expression position, not just under
  `handle`.** `let x = { … }`, a block as a call argument, a block in a match arm
  — all become lowerable in the same edit. That is a **deliberate surface
  widening beyond this slice's stated scope**, and it is cheaper to accept and say
  so than to gate it to handle bodies only (which would mean threading a
  positional flag through `lower_expr` for no semantic gain). ✓ VERIFIED that it
  breaks nothing: `src/core.rs:347` is the sole production site and **no test
  anywhere asserts `Unsupported("Block")`** — corroborated independently at
  `docs/superpowers/plans/2026-08-29-elya-slice-5b2-native-control-flow.md:72`,
  which verified the same absence by grep for the 5b-2 slice. Zero test updates.
- **No new span enters the frozen table.** `lower_block` spans each `Let` it
  builds at `stmt.span` and leaves the tail at its own span, and it takes every
  `ty` by *cloning the accumulator's* (`let ty = acc.ty.clone()`), never by
  looking up the brace span. The brace span is therefore never queried, which
  matters because it may have no entry: the table is frozen before lowering and
  is keyed on what the solver visited.

**Net:** one arm, no new node, no table risk, one widening to declare. The fork
map's worry — that this could cost a whole new Core node — does not materialise,
but the answer is "one cheap arm," not "free."

### 3.2 `(resume(s))(s)` trips the computed-callee refusal — unconditionally

**Answer: yes, and the fidelity corpus's own handler therefore cannot compile as
written.** ✓ VERIFIED.

`build_elya_call` guards at `crates/codegen/src/lib.rs:509-512` with

```rust
let CoreKind::Var(name) = &callee.kind else {
    return Err(CodegenError::Unsupported("computed callee"))
};
```

The guard is keyed on **Var-ness**, not on lambda-ness. A `resume` node in callee
position is not a `Var`, so it is refused — this is the same seam that refuses
immediately-applied lambdas, reached from a different direction.

**The resolution: A-normalize in Core lowering, not in codegen.** Lower an applied
resume as

```
Let("$k", <resume node>, App(Var("$k"), [s]))
```

giving a `Var` callee that is present in `env`, which routes to
`build_closure_call`, whose `Ty::Fn` guard passes because `resume(s) : R` and
`R = (S) -> A` at this site.

Two existing precedents make the synthetic binder house style rather than a new
device: `lower_block`'s synthetic `"_"` binder (`src/core.rs:225`), and the
evaluator's own `$`-prefixed internal names. ✓ VERIFIED that both `CoreKind::Let`
codegen arms save and restore a shadowed binding, so `$k` is shadowing-safe.

**One consequence that must not be missed:** a synthesized Core node has no span
in the frozen type table, so `$k`'s `CoreExpr.ty` must be **cloned from the
`resume` node's own type**, never looked up by span. A span lookup here would
either panic or silently pick up a neighbouring type.

**What was rejected, and why:** relaxing `build_elya_call`'s dispatcher to accept
computed callees. That guard's comment exists to hold 5b-3 §5.3's cut in place.
Moving a cut to make an unrelated slice easier is how cuts stop meaning anything.

---

## 4. What the native back end must reproduce

The evaluator is the specification; the differential test enforces it. The
semantics N8 must match, ✓ VERIFIED at `src/eval.rs:1034-1068`:

1. **`perform` walks the continuation outward**, cloning every frame it passes
   into `cap: Vec<Frame>`, until it finds a `Frame::HandleK` whose handler
   matches.
2. **The match is strict on `(effect, op)`** (`handler_handles`) — a handler
   catches `E.op` only if a clause names both. Two effects may share an operation
   name without ambiguity.
3. **The handler runs with `cap` as the reified continuation**, the handler's own
   `env` as `ret_env`, and `k_rest` (the frames *beyond* the handler) as the
   surrounding continuation.
4. **Deep handlers:** on every resume, the handler is re-installed *beneath* the
   captured frames, so `Kont' = k_cap ++ [HandleK] ++ k_now`, deepest-first. A
   resumed computation that performs again finds the same handler.
5. **One-shot enforcement is dynamic in the evaluator**: `consumed: Cell<bool>`
   plus E0425, skipped entirely when `rd.handler.multi`.

Point 4 is the one most easily got wrong natively, because the obvious
implementation — jump back into the captured frames — drops the handler and makes
the *second* perform in a loop escape. The fidelity test in §7 is a loop
precisely so that it performs more than once.

Point 5 is where the native back end **diverges deliberately**: see §5. *(2026-10-03:
for a one-shot effect, native enforces point 5 with a consumed flag that traps with a
named error, never by re-executing — §13 D13.)*

---

## 5. Fork 1(a) — `with multi` refused, keyed on the declaration

**Decision: refuse by name at codegen, keyed on the effect's declaration, using
the same rule `affine.rs` already uses, and refusing a deliberate superset.**

### 5.1 The enabling fact, and its limit

✓ VERIFIED at `src/types.rs:1211-1225`: E0427 fires when `handler.multi` is true
and the handled effect's `effect_multi` lookup is false — *"a one-shot effect
cannot be handled with `multi`"*.

Its contrapositive is what makes a *static* decision possible: a
non-`multi`-declared effect can never be caught by a multi-shot handler, anywhere
in the module. At any perform of such an effect, the continuation is statically
one-shot — **from the declaration alone**, with no site analysis.

**The converse is not constrained, and this is the limit.** ✓ VERIFIED: there is
no `else` branch. A *plain* handler over a `multi`-declared effect is accepted
without comment. So `multi` on a declaration is a **permission, not an
obligation**.

### 5.2 The over-refusal, stated

Keying the refusal on the declaration therefore **refuses a superset**: a plain
`handle` over a `multi`-declared effect is refused at codegen even though the
continuation at that site is statically one-shot and would compile correctly.

This is deliberate. The alternative — keying on `Handler.multi` at the handle site
— is more precise, and is also a second partition of the same concept maintained
in a third place, whose failure mode is silent mis-compilation rather than a
refusal. **Refusing too much is a diagnostic; refusing too little is a wrong
answer.** The over-refusal is recorded here so the slice that lifts it knows it is
lifting a choice, not fixing a bug.

### 5.3 The rule, and where it is computed

✓ VERIFIED at `src/affine.rs:14-51`, the existing construction:

```rust
let mut multi_ops: HashSet<String> = HashSet::new();
for d in &module.decls {
    if let Decl::Effect(e) = &d.node {
        if e.is_multi {
            for op in &e.ops { multi_ops.insert(op.node.name.clone()); }
        }
    }
}
```

with the comment *"rebuilt locally, exactly as Slice 4d-1 built
`Infer.effect_multi` — keeps the pass self-contained."* **There are already two
constructions of this one rule.**

**The obstacle:** ✓ VERIFIED at `src/core.rs:111` that
`CoreModule { fns: Vec<CoreFn>, types: Vec<CoreType> }` carries **no effect
declarations**, and at `src/core.rs:125-128` that this is explicit — *"`Decl::Effect`
is not re-homed (spec §2 — exhaustiveness-on-Core is out of scope)."* Codegen
therefore has no path to the declaration today.

**Two readings, and the one taken:**

- **(i) `CoreModule` gains a `multi_effects: HashSet<String>` field**, built in
  `lower_module` pass 1 (which already walks `module.decls`). Straightforward, but
  it re-homes a piece of `Decl::Effect` into Core against a comment that says Core
  does not do that, and it makes the rule's *third* stored copy.
- **(ii) The Core handle node carries the resolved bit.** `lower_module` knows both
  the handler's effect name and the module's declarations, and stamps
  `is_multi_declared: bool` onto the handle node. Codegen refuses on a bool it was
  handed.

**Taken: (ii).** It applies the rule exactly once, at the only place that has both
inputs; it adds nothing to `CoreModule`; and it parallels `CoreKind::Builtin`,
which carries the owned name that codegen refuses on
(`crates/codegen/src/lib.rs:794-808`). The `CoreFn` "second source of truth"
objection does not apply: Core holds no other copy of `is_multi`, so this is the
only copy, not a duplicate.

**Surfaced for decision (§9.4):** this makes a *third* local construction of one
rule. The alternative is to extract
`pub fn multi_declared_ops(&Module) -> HashSet<String>` once and have all three
call it — which touches two existing, working call sites and is therefore a
separable change, not something to fold in silently.

### 5.4 The refusal itself

At codegen, on the handle node, **before lowering any clause body**: if
`is_multi_declared`, return `CodegenError::Unsupported`. The message names `multi`
so the diagnostic points at the feature rather than at the machinery. Refusing
before touching the arguments is the pattern `io.println`'s arity refusal already
sets (`crates/codegen/src/lib.rs:794-808`, negative controls at `:2048` and
`:2071`).

This is a **reachable** refusal — a user can write `effect multi Flip` and handle
it — so under the standing rule it owes an **execution test**, not a structural
argument.

---

## 6. Fork 3(a) — the continuation representation

**Decision: a linked list of fixed-arity frame blocks. The existing descriptor
mechanism is unchanged, and `gc_mark` is not touched.**

### 6.1 Shape

A captured continuation is a cons list. Each cell is one heap block of the shape
the collector already understands — ✓ VERIFIED that closures are
`[tag][code_ptr][cap_0..]` and have been traced since N5:

```
frame cell:  [tag][code_ptr][next]
```

`code_ptr` resumes one frame's worth of work; `next` is the rest of the
continuation, or null at the base. **A continuation frame is a closure.** That is
not an analogy used to explain the design — it is the design, and it is why the
design is cheap.

*(2026-10-03, §13 D10: the cell above has no room for the values a suspended
computation still needs. A frame is shaped per continuation site,
`[tag][code_ptr][next][saved…]` — a closure in exactly the lambda sense — and the
three-word shape above is its no-saved-values case.)*

### 6.2 Why 3(a) and not a variable-length block

A continuation is variable-length, pointer-bearing, and **heterogeneous** — the
first object of that kind in the heap. Strings (5b-7) were variable-length but
*uniform*: `[tag][len][bytes..]`, arity 0, mask 0, a leaf. Being a leaf is what
made strings free.

A variable-length heterogeneous block cannot be described by one descriptor row;
it needs a per-object trace, which means **a new case in `gc_mark`**. ✓ VERIFIED
that `gc_mark` came out byte-identical across two consecutive slices (5b-6, 5b-7;
638 bytes both sides, measured, not eyeballed).

**The governing rule, stated as a rule:** *do not add a case to the one function
whose failures are silent.* A wrong trace does not crash at the trace — it drops a
live object and crashes arbitrarily later, in code that is correct. Fixed-arity
cells keep `gc_mark` out of the slice entirely, and put it on track to be
byte-identical for a **third** consecutive slice.

### 6.3 What this costs

**Exactly one new descriptor row:** arity 2, mask `0b10`. Bit `f` of a row's mask
governs word `1 + f` (`gc_mark`), so for `[tag][code_ptr][next]` bit 0 is `code_ptr`
and bit 1 is `next`. Only `next` is a heap pointer, so only bit 1 is set: `code_ptr`
is a text-segment address stored as an integer word — exactly as a closure's word 1
is — and is never traced. *(Corrected in place 2026-10-02. This sentence first read
"mask `0b11` (both `code_ptr` and `next` are pointers)": true of the machine words,
wrong for the mask, which marks HEAP pointers. Tracing the code pointer makes
`gc_gray_push` write a mark bit 16 bytes before a text-segment address; measured with
the lambda rows' bit 0 forced on, a closure program that collects dies with signal
11. See plan D5; implemented in b370191 and pinned by
`the_frame_row_follows_the_string_row_with_mask_0b10`.)* ✓ VERIFIED that tag allocation is linear and computed
(`crates/codegen/src/lib.rs:1253-1255`):

```rust
let n_real_ctors: usize = core.types.iter().map(|t| t.ctors.len()).sum();
let lambdas = closure::collect_lambdas(core, n_real_ctors);
let string_tag = n_real_ctors + lambdas.len();
```

The frame tag appends after `string_tag`, and **both** existing guards must be
extended with it: the per-lambda guard at `:1391` and the row-count guard
`string_tag != desc.len() / 2` at `:1412`. Those guards are structural invariants
— no Elya program can make them disagree — so per the standing rule they are
discharged by the check that enforces them and owe no execution test.

### 6.4 The cost this design does carry

Every captured frame is a separate allocation. A deep continuation is a long list,
and capturing it is O(depth) allocations. That is worse than one flat block would
be, and it is accepted. §7 explains why it is also the thing that makes the slice
honest.

---

## 7. Fork 4 — the instrument, its coupling, and the explicit choice

**This is the part of the spec most likely to be wrong in a way that looks right,
so it is stated at length.**

### 7.1 The coupling

The evaluator's fidelity corpus (`tests/state_effect.rs`) pins `K_MAX_STATE = 5`
— *"It MUST be constant across N; a peak that grows is a per-operation splice leak
(fix the machine, never raise the constant)."* That constant counts **continuation
depth**, via `peak_kont_depth()`, an evaluator-only observable.

The native instrument available is `live` — surviving words after a collection. It
measures **heap residency**.

**These are not the same axis.** A native implementation that keeps continuation
frames on the machine stack would make `live` settle perfectly across every N
while leaking continuation depth on every iteration. The test would be green and
would prove nothing. **That is the vacuous-control shape**, and it is the specific
failure this section exists to prevent.

The choice is between two resolutions: pair `live` with a second instrument that
can see the stack, or make the representation such that there is no stack to see.

### 7.2 The choice, stated explicitly

**Taken: primarily the representation, and — because the representation covers
only one of the two ways depth can leak — one required test that observes the
machine stack directly. Both, and they are not interchangeable.**

The first half is the design decision. Fork 3(a) heap-allocates every captured
frame, so for anything held in the *frame list*, **continuation depth is heap
residency** — the two axes are made into one by construction, not by correlation.
A splice leak in the list (failing to drop the consumed prefix, or re-installing
the handler above rather than beneath the captured frames) leaves frame cells
reachable, and `live` grows. This is why §6.4's per-frame allocation cost is
accepted rather than optimised away: **the cost is the observability.**

**Why that is not sufficient on its own.** Heap-flatness and machine-stack
flatness are *independent* properties of the generated code. A `perform` that
transfers control by calling the handler clause from the perform site nests a
native frame on the performer's frame — and does so whether or not the heap frame
list splices perfectly. In that implementation `live` settles, every heap
assertion passes, and the machine stack still grows once per perform. The
representation makes the *list* observable; it says nothing about the *transfer*.
The transfer is what the second half watches, and it is criterion **A9** (§11) —
required, not incidental.

**Consequence for the design, not just for the tests:** the transfer must return
to a trampoline rather than nest, and §8.3's dispatch is where that is discharged.
*(2026-10-03, §13 D14: "must not nest" stands; it is met by `musttail` transfers, not
by a trampoline loop.)*
General resume position (§0) forces the issue anyway — a clause that applies
`resume`'s result, as the corpus's does, cannot be a nested call — but the
instrument must be able to catch it if the implementation drifts back.

### 7.3 The invariant this creates, and how it is guarded

Choosing this resolution buys a guarantee and incurs an obligation:

> **Invariant N8-1. The one-shot path has no stack-resident fast path.** Every
> captured continuation frame is heap-allocated, including in the tail-resumptive
> case, for as long as the fidelity instrument is `live`.

A later slice that adds "tail-resumptive handlers compile to a direct call, frames
stay on the machine stack" would be a correct and desirable optimisation that
**silently re-blinds the instrument.** Writing the invariant down is not enough; it
needs teeth.

**An earlier draft of this section claimed the teeth already existed in the 5b-6
instrument and had merely been misread as decoration. That claim was checked and
is false, on two independent grounds.** It is recorded here because it is exactly
the kind of error §7 exists to catch, and because both grounds constrain what the
N8 instrument has to be.

**Ground 1 — the existing instrument's program contains no handler.** ✓ VERIFIED
at `crates/codegen/tests/native_codegen.rs:1163-1169`, the program under
`the_live_set_settles_independent_of_iteration_count` is an ADT churn:

```
fn churn(n) { if n == 0 { 0 } else { let _ = Cons(1, Nil)  churn(n - 1) } }
```

No `handle`, no `resume`, no effect declaration. Its `collections > 0` is
discharged by `Cons` garbage and would be discharged identically if handlers did
not exist. **The existing test cannot be the guard for an invariant about frames,
because it never allocates one.** What transfers is the *shape* and nothing else,
exactly as §7.4 says — the earlier claim contradicted §7.4 and §7.4 was right.

**Ground 2 — on the new handler program, `collections > 0` still cannot
distinguish heap frames from stack frames.** The corpus handler is ✓ VERIFIED at
`tests/state_effect.rs:65-67`:

```
State.get() -> fn(s) { (resume(s))(s) }
State.set(v) -> fn(s) { (resume(Unit))(v) }
```

Each clause body **evaluates a lambda**, so each perform allocates a closure —
100 000 closures at n = 100 000, independent of where frames live. Closure garbage
alone satisfies `collections > 0`. The assertion remains valid for what it
actually says (*a collection happened, so `live` was computed at all*), and must
be kept for that; it is **not** an anti-vacuity guard for Invariant N8-1, and the
spec must not claim it is.

**The actual teeth, therefore, are two tests and they watch different things:**

1. **Heap half — `live` equality over the spread, on a handler program.** Written
   fresh (§9.1), not the 5b-6 test reused. This catches a splice leak *in the
   frame list*. Its anti-vacuity control is a growing one (§9.1, A5), on the
   `a_growing_live_set_moves_the_instrument` pattern.
2. **Machine-stack half — completion at large N.** A per-perform native frame
   overflows the stack and kills the process; an execution test reads that as a
   failed run. This is the *only* observation in the slice that can see a nesting
   transfer, so it is **a required acceptance criterion (A9), not a free
   backstop.** An earlier draft filed it as incidental; downgrading the one
   instrument that watches the stack to a lucky side-effect is how the coupling
   hazard would have survived the spec that was written to name it.

**A9's anti-vacuity problem is real but already discharged, and not by a control
test.** "The program completed" passes vacuously if N is too small for a nesting
transfer to overflow. A first draft of this section demanded a non-tail control
growing through a supported operation. That is the wrong instrument: a non-tail
control would vary *where `resume` sits*, whereas Invariant N8-1's stack half is
about the **perform → clause transfer**, which nests or does not nest regardless of
resume's position. A1 already exercises non-tail resume. What A9 needs is not a
contrast — it is a **calibrated N**, and the repository already contains the
calibration.

✓ VERIFIED — four existing native tests run to 1 000 000 frames and are *only*
survivable because a call was eliminated: `native_codegen.rs:648` (self tail
recursion), `:660` (mutual, the case a loop rewrite cannot fake), `:941` (an
allocating loop), `:1065` (an indirect tail jump through a loaded code pointer). At
that N a frame-per-iteration is not a slow program, it is a dead one.

✓ VERIFIED — the failure signal is already discriminated **by name**.
`STACK_OVERFLOW = 0xC00000FD` at `native_codegen.rs:207`;
`diagnose_stack_overflow` at `:209-221` panics with *"A tail call that `musttail`
was supposed to eliminate grew the machine stack instead. This is the tail-call
guarantee failing, not a generic crash."*; `assert_runs` calls it at `:226` before
checking the exit status. Its own doc comment (`:204-206`) states the reading:
*"Observing it from a corpus binary means one thing: a call that must have been
eliminated was not."*

So A9 is discharged by running the corpus's `state_tail_loop` source at
N = 1 000 000 through `assert_runs`, and the three properties that make it
non-vacuous are already in the tree: the N is calibrated by four tests, the
failure mode is named rather than numeric, and the expected bytes at exactly that
N are independently pinned by the evaluator (`tests/state_effect.rs:77-82`
asserts `"x\n"` at 1 000 000). That makes A9 a **differential** observation of the
machine stack, not a bare liveness check — and it needs no new control, only the
existing harness. **The one thing planning must not do is lower the N.**

### 7.4 What transfers from the evaluator corpus, and what does not

- **Transfers:** the *shape* — equality across four N over an 8× spread, with the
  growing control proving the instrument can move. This also satisfies the standing
  constraint against nudging constants, because **no constant is pinned**: the
  assertion is an equality between measurements.
- **Does not transfer:** `K_MAX_STATE = 5` itself. It counts evaluator `Kont`
  frames (pure TCE's K_MAX 3, plus the effect frame, plus the state-passing
  application frame). Porting the number would be pinning a constant that means
  nothing on the native side. It must not be copied across.

---

## 8. The transform

### 8.1 Selective CPS, and the key it selects on

A function that may perform an effect must be able to have its continuation
captured, so it cannot use the machine's return address. A function that cannot
perform needs no such thing and should stay a direct call.

**The key already exists in the frozen type table.** ✓ VERIFIED at
`src/types.rs:12-32`: `Ty::Fn(Vec<Ty>, EffectRow, Box<Ty>)` — the middle field
**is** the latent effect row, carried by every function type. It is reachable at
each call site through the callee's `CoreExpr.ty`. **No new `CoreFn` field is
needed**, which matters because ✓ VERIFIED at `src/core.rs` that `CoreFn` has
deliberately no `ret` field, on the grounds that a second source of truth is worse
than a lookup.

### 8.2 The key cannot be "the row is non-pure"

**The key is: the row mentions a *user-declared* effect — one with a `Decl::Effect`.**
Not "the row is non-pure", which degenerates to "every function that prints``.

The full argument is in §9.2 rather than here, because it is the largest piece of
unfinished reach in this spec rather than an aside, and because the premise it
rests on is marked ? INFERRED and must be settled before the plan is written.

### 8.3 Handler dispatch

A handle node compiles to: install a handler record; run the body; on a perform of
a matching `(effect, op)`, capture frames into the cons list of §6, invoke the
clause with the reified continuation and the handler's own environment, and on
`resume` splice `k_cap ++ [handler] ++ k_now` — handler **beneath** the captured
frames, per §4 point 4.

The strict `(effect, op)` match of §4 point 2 is reproduced as-is. Nothing here
needs a row at runtime; the match is on two names.

*(2026-10-03: the perform site is a `CoreKind::Perform` carrying both names, resolved
ops-first as inference and the evaluator resolve it — §13 D11; transfers are `musttail`
jumps — §13 D14.)*

---

## 9. Further reach surfaced while writing this spec

Four items. The first three change what the slice must do; the fourth is a
decision to be taken rather than a problem to be solved.

### 9.1 The native corpus is a deliberate subset, and the subset is not the obvious one

**The phrase to avoid is "run the same tests."** ✓ VERIFIED that
`tests/state_effect.rs` holds exactly three tests, and they do not fall on the
same side of the line. The split is between a test's **program** (is the surface
native-compilable after 5b-7?) and its **assertion** (is the observable one the
native side has?), and those come apart:

| Test | Program | Assertion | Verdict |
|---|---|---|---|
| `parameter_passing_state_threads_through_resume` (`:36`) | uses `set(x <> "b")` — **`<>`** | output equality | **out.** Surface refused. |
| `state_tail_loop_is_bounded` (`:60-71`) | `set("x")`, `get()`, literals only — **no `<>`** | `peak <= K_MAX_STATE`, `peak` constant in N, via `peak_kont_depth()` | **program transfers, assertion does not.** |
| `non_tail_state_loop_grows_with_length` (`:95-101`) | `loop(n - 1) <> "y"` — **`<>`** | `deep > shallow` | **out.** Surface refused — **and it is the control.** |

`<>` has been refused by name since N6 (5b-7 shipped four string refusals; a
string is constructible and printable and nothing else).

Two consequences, and the second is the one that bites:

- **The middle row is why A9 exists and what it runs.** `state_tail_loop`'s source
  compiles natively as written — it is the one program in the corpus that does —
  so the native side does not have to invent a handler program to measure. What it
  must replace is the *assertion*: `peak_kont_depth()` is an evaluator-only
  observable, so the native form asserts **completion and exact output bytes at
  N = 1 000 000** — the corpus's own large case, and the N four existing native
  tests already calibrate as fatal to a non-eliminated call (§7.3). The
  substitution is assertion-for-assertion at the *same* N, which is what makes A9
  differential rather than merely alive.
- **The half with teeth is the half that is refused.** The bounded assertion
  transfers and the growing control does not. A naive port therefore keeps exactly
  the assertion that can pass vacuously and drops exactly the test that proves it
  can fail. **That is the §7.1 shape again, arriving by a third route** — and it is
  now the third time in this spec that the portable thing was the weak thing.

**Resolution:** the native fidelity test is **written fresh in native-compilable
Elya**, not ported, and it ships with its own growing control that does not use
string concatenation. ✓ VERIFIED that a native growing control of the right shape
already exists and works — `a_growing_live_set_moves_the_instrument` at
`crates/codegen/tests/native_codegen.rs:1197-1220`, using a strict inequality — so
the pattern to follow is in the repo, and the substitute for `<>` is to grow an ADT
structure rather than a string.

### 9.2 `io.println` performs `{IO}`, so "non-pure" is not the CPS key

✓ VERIFIED at `src/types.rs:1100`:

```rust
let _ = self.add_effect(amb, "IO", Vec::new(), span); // io.println performs {IO}
```

**Every printing program is "effectful."** A selective-CPS key of "this function's
row is non-pure" therefore degenerates to "every function that prints" — which, in
a corpus whose whole point is that programs say things, is close to *every
function*. Selective CPS that selects everything is CPS, and CPS for the whole
program is a much larger slice than this one.

**The partition must be narrower: rows that mention a *user-declared* effect — one
with a `Decl::Effect`.** `IO` is discharged at the runtime boundary and can never
be caught by a user handler, so it cannot cause a capture.

**? INFERRED**, and load-bearing, so it is flagged rather than assumed: that `IO`
has no `Decl::Effect` and cannot acquire one. Related sites that must be read
during planning — `src/parse.rs:1243` (asserts labels `["IO"]`) and
`src/types.rs:1500` (`if label != "IO"`). **If a user can write `effect IO { … }`,
this partition needs a different discriminator**, and that must be settled before
the plan is written, not during it.

### 9.3 The slice is large, and the obvious decomposition does not work

Stated plainly: at this cut, N8 is **selective CPS + a heap frame list + handler
dispatch + a rewritten fidelity corpus**. That is bigger than 5b-7, which was one
leaf heap shape and one builtin.

**The tempting split is N8a (tail-resumptive only) then N8b (general position).**
It should be rejected, and for a reason worth recording: a tail-resumptive-only
slice needs no reified continuation, so it would allocate no frames, so — by §7.2 —
its fidelity instrument would be **vacuous by construction**. N8a would ship green
with no real proof and N8b would carry the entire evidentiary burden. The fidelity
instrument cannot be separated from general resume position, because general
resume position is what makes it non-vacuous. **The slice resists this
decomposition, and forcing it would manufacture exactly the vacuity §7 exists to
prevent.**

**Proposed decomposition along the machinery axis instead, within one slice:**

1. **Refusals first.** `with multi` (§5.4) and the polymorphic-effect boundary
   (§11 A3). Negative space is cheap, and it fixes the slice's edges before any of
   it is built.
2. **Core vocabulary.** The handle node with its `is_multi_declared` bit, the
   resume node, and the `$k` A-normalization of §3.2.
3. **Runtime shape.** Frame-cell allocation, the one new descriptor row, the tag
   extension, and both guards (§6.3).
4. **The transform.** Selective CPS over the §9.2 partition.
5. **Dispatch and splice.** Including deep-handler re-installation (§4 point 4).
6. **Fidelity.** Both halves of Invariant N8-1, which are separate work: the heap
   instrument written fresh on a handler program with its growing control (A4/A5),
   and the machine-stack observation, which reuses the one corpus program that
   transfers (A9, §9.1). Sizing this as "port the corpus" would under-size it —
   one of the three corpus programs transfers, and only its source, not its
   assertion.
7. **Negative controls.** Built, observed failing *differently*, reverted in the
   commit that adds the proofs they control — the 5b-7 pattern.

**Step 4 is the one that may not fit in a single task**, and the plan phase should
size it honestly rather than discover it mid-slice.

### 9.4 One rule, three constructions — a decision, not a defect

Per §5.3, this slice would add a third local construction of "ops of a
`multi`-declared effect" (after `Infer.effect_multi` and `affine.rs`'s
`multi_ops`). The house style is explicitly local rebuild, and the comment in
`affine.rs` says so. The alternative is one extracted helper called from three
places, which touches two working call sites.

**Recommended: extract the helper, as its own task, with its own gate** — three
copies is the point at which "self-contained pass" stops being a style and starts
being a maintenance hazard. **Flagged for decision** rather than folded in, because
it modifies code outside this slice's scope.

---

## 10. Non-goals

- Multi-shot continuations executing natively. Refused, with a test.
- Polymorphic effects. N7.
- Effect rows at runtime.
- Optimising away the per-frame allocation of §6.4 — explicitly *not* wanted in
  this slice, per Invariant N8-1.
- A flat variable-length continuation block, and with it any new `gc_mark` case.
- Lifting the §5.2 over-refusal.

---

## 11. Acceptance

Every criterion is discharged by **execution**. No IR snapshots, no `#[ignore]`.
Reachable refusals get execution tests; structural invariants are discharged by the
checks that enforce them.

| # | Criterion | Kind |
|---|---|---|
| A1 | A `State`-effect program with a **non-tail** `resume` compiles natively, and its output equals the evaluator's — both the value and the exact bytes written. | differential execution |
| A2 | `with multi` is refused at codegen by a message naming `multi`, before any clause body is lowered. | execution (reachable refusal) |
| A3 | ~~A polymorphic effect is refused.~~ **✓ VERIFIED by execution (2026-10-02): a polymorphic effect has no refusal of its own, at any stage.** Its declaration passes the front end, lowers, and compiles and runs natively (`a3_a_polymorphic_effect_declaration_compiles_and_runs_natively`). An op performed at one concrete type reaches codegen as a monomorphic op does — the identical refusal, until this slice's handlers land. Only a type parameter left unconstrained survives, as `Ty::Var`, and it falls into the existing `Ty::Var` refusal, `unrepresentable type` (`a3_an_unconstrained_polymorphic_effect_meets_the_ty_var_refusal`). So it is the existing refusal, not a distinct one. Plan D2 stands; Task 3's prediction of a refusal was wrong. | execution (measured) |
| A4 | On a **handler** program, the live set settles across four N over an 8× spread, **no constant pinned**. `collections > 0` and `live > 0` are retained as guards that `live` was computed at all — **not** as Invariant N8-1 guards, which they cannot be (§7.3, ground 2). | execution |
| A5 | A growing control moves the heap instrument (strict inequality), written without `<>` (§9.1). | execution |
| A6 | A deep handler resumed inside a loop performs more than once and finds the same handler each time (§4 point 4). | differential execution |
| A7 | `gc_mark` is byte-identical to its form at the slice base, measured by byte count, not eyeballed — third consecutive slice. If it is not, the deviation is **reported, not patched**. | measurement |
| A8 | One descriptor row per continuation site, each guarded by "tag == row index"; Task 6's `[2, 0b10]` row is the no-saved-values case (§13 D10). *(Was: "The tag/row guards at `crates/codegen/src/lib.rs:1391` and `:1412` are extended to cover the frame tag.")* | structural (discharged by the check) |
| A9 | **The other half of Invariant N8-1 — the machine stack.** `state_tail_loop`'s source (§9.1, the one row that transfers) compiles natively as written and runs to completion at **N = 1 000 000** through `assert_runs`, printing exactly `x`. Non-vacuous by the calibration in §7.3, not by a control test. **The N must not be lowered.** | execution (differential bytes) |

**A4 and A9 are not interchangeable and neither subsumes the other** — A4 watches
the heap frame list, A9 watches the machine stack, and §7.1's coupling hazard is
precisely that each is blind where the other sees (§7.2).

**A3 is the one criterion this spec cannot fully specify**, and it is marked rather
than guessed. *(2026-10-02: no longer — A3 was measured and its row above now states
the result.)*

---

## 12. Obligations carried out of this slice

- **Closed here:** T7 (§1). The out-of-repo ledger line naming N8 as its arrival
  point must be updated at close-out.
- **Created here:** Invariant N8-1 (§7.3) — no stack-resident fast path on the
  one-shot path while `live` is the fidelity instrument. A future tail-resumptive
  optimisation must either keep the invariant or replace the instrument first.
  It is guarded by **two** criteria that watch different machines, A4 (heap frame
  list) and A9 (machine stack), because neither can see the other's leak (§7.1).
  `collections > 0` is **not** one of its guards — that reading was checked and
  disproved on two independent grounds (§7.3), and the disproof is recorded
  because the same mistake is available to every future slice that reuses the
  5b-6 instrument by shape.
- **Created here:** A9's dependence on N = 1 000 000. The figure is not a
  performance target; it is the calibration that makes a completion assertion
  mean something, inherited from four existing tests and from
  `diagnose_stack_overflow`'s named failure (§7.3). Lowering it silently converts
  A9 into a tautology.
- **Recorded here:** the §5.2 over-refusal, as a choice to be lifted rather than a
  bug to be fixed.
- **Carried unchanged:** T2 (whole-module representability), T5 (sequential match
  chain is an unoptimised floor), T6 (literal-pattern match unfocused), uniform
  function representation, the immediately-applied-lambda seam, and the ADT-field
  `Ty::Fn` execution test — which still comes due at N7, not here.
- **Open for decision before planning:** §9.2 (whether `IO` can be user-declared)
  and §9.4 (extract the `multi_declared_ops` helper, or accept a third local
  rebuild).

---

## 13. Amendments at the Task 7b checkpoint (2026-10-03)

Taken by the reviewer on evidence measured at the plan's Task 7b checkpoint (at
`adbd854`). The full rationale and evidence are the plan's D10–D15, with the same
numbers; each section amended above carries a dated pointer here.

- **D10 (§0 item 3, §6.1, §6.3, §11 A8).** Frames are shaped per continuation site,
  `[tag][code_ptr][next][saved…]`, each site with one descriptor row guarded "tag == row
  index". Bit 0 is clear, bit 1 is set, and bit `j + 2` is set iff saved value `j` is a heap
  value. The row Task 6 laid down, `[2, 0b10]`, is the no-saved-values case. `gc_mark` is
  still untouched (A7): rows are generic, and lambdas already have one row per site.
- **D11 (§3.1, §8.3).** `CoreKind::Perform` carries the effect, the op and the arguments,
  resolved ops-first exactly as inference and the evaluator resolve a call. Measured: with a
  function and an op both named `ping`, the front end accepts the program and the evaluator
  performs the op; codegen's local-then-function order would have called the function. It is
  a third new Core node, against §3.1's "no fifth Core node".
- **D12 (§2.1, §3.2).** A lambda that calls `resume` captures the clause's continuation
  implicitly, as a synthetic binder the ordinary free-variable rule carries inward. It is a
  heap pointer to the frame chain, so its mask bit MUST be set.
- **D13 (§4 point 5, §5).** Natively, one-shot is enforced by a consumed flag that traps
  with a named runtime error, never by re-executing; the differential test expects both
  sides to fail (E0425, and the trap).
- **D14 (§0 item 2, §7.2, §8.3).** No runtime trampoline: every transfer is a `musttail`
  jump, and the only nesting calls into effectful code are the handle and resume sites. A CPS
  function therefore has at most 4 source parameters. A9's N = 1 000 000 is unchanged.
- **D15.** Plan-only: the plan's 7b test moves to Task 8, its predicted failure corrected
  to the measured one.
