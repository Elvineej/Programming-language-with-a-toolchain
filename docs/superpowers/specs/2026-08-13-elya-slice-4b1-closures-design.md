# Elya — Slice 4b-1 Design Specification: Basic Closures & the Value Restriction

- **Codename:** Elya
- **Slice:** 4b-1 — the first of three closure sub-slices (Slice 4b, "lambdas/closures"), itself the second half of Slice 4 ("ADTs + generics + lambdas")
- **Status:** Design draft — awaiting review before an implementation plan is written
- **Date:** 2026-08-13
- **Depends on:** Slice 4a (complete, `52cefce`) — parametric ADTs, pattern matching, Maranget exhaustiveness (E043x), CLI warning surfacing; and Slice 3 — HM types with row-polymorphic effect inference, algebraic effects on the CEK machine, measured TCE.
- **Companion documents:** the language design spec (`.../2026-08-05-elya-language-design.md`, "design spec"); the Slice 3 effects spec (`.../2026-08-06-elya-slice-3-effects.md`, "effects spec"); the Slice 4a ADT spec (`.../2026-08-08-elya-slice-4a-adts-design.md`, "4a spec").

---

## 0. How to read this document

This is a **design spec** for Slice 4b-1 — **first-class anonymous functions (lambdas), closures that capture their defining environment, and the value restriction** on let-generalization. It is the first of **three** sub-slices decomposing Slice 4b:

- **4b-1 (this doc)** — basic closures + the value-restriction gate. Lambda syntax, `Value::Closure`, the CEK call path, sound let-polymorphism for lambda-bound names, TCE through a closure call, and retirement of E0433 (bare constructors become function values).
- **4b-2** — effect-carrying closures & higher-order relay. HOFs that *take* a closure and relay its latent effects; the multi-shot lint (E0426) and discharge interacting with closures; dynamic handler scoping.
- **4b-3** — row-polymorphic combinators and the effect-row stress test of the value restriction (the soundness capstone that sets up the frontier linear/affine work).

Section refs: "design spec §X" → the language design spec; "effects spec §X" → the Slice 3 spec; "4a spec §X" → the Slice 4a spec; bare "§X" → this document. Diagnostic codes: **E044x are reserved for closures/lambdas**; 4b-1 introduces **none** (§5) — it *retires* a code (E0433) rather than adding one.

**The load-bearing realizations (from brainstorming):**

1. **The effect row is not on the value.** A runtime closure carries *code + captured environment only*. Effects are threaded **dynamically** by the CEK machine to the nearest enclosing `HandleK` at the **call site** — not the definition site. The latent effect row lives entirely in the *type* (`Ty::Fn`'s `EffectRow`), and `infer_call`'s general path already pours a called value's row into the caller's ambient ([`types.rs:928-936`](../../../src/types.rs#L928)). So the *type* side of calling closures is essentially already built; 4b-1 is mostly the **value** side plus surface syntax.

2. **The value restriction is a required correctness change, not a nicety.** Elya currently let-generalizes **every** local binding *unconditionally* ([`types.rs:685-688`](../../../src/types.rs#L685)). That is latent-unsound but currently unreachable. **Lambdas make it reachable** (Elya has had first-class continuations since 3c; generalizing a *non-value* binding is the classic soundness hole, and a lambda-bound cell makes it trivial). Introducing the value restriction is therefore part of *doing lambdas safely* — pulled forward into 4b-1, applied at that one generalization site, gating both type- and row-variable generalization with a single `is_syntactic_value` predicate.

---

## 1. Scope

### 1.1 What Slice 4b-1 delivers

1. **Anonymous functions (lambdas)** — `fn(x, y) { body }`: an expression-position function literal with a **block body**, **uncurried** (multi-parameter), consistent with `fn` declarations. Parameters are untyped (HM infers them).
2. **Closures** — a lambda captures its defining environment by value (O(1) `Rc` clone of the persistent env). Free variables resolve to the captured bindings.
3. **First-class function values** — lambdas may be `let`-bound, passed as arguments, returned, and stored in ADTs. Calling them reuses ordinary call syntax.
4. **The value restriction** — `let`-generalization is gated on `is_syntactic_value(rhs)`: lambdas, variables, literals, and bare constructors generalize; applications, `match`, `if`, blocks, and operators do not. This makes lambda-bound names *soundly* polymorphic and closes the latent hole.
5. **Constructors as function values (retires E0433's bare case)** — a bare, unapplied n-ary constructor (`Some`, `Cons`) is now a first-class function value of the constructor's type, fulfilling the 4a promise ("unapplied constructors become values in 4b"). Partial application (`Cons(1)` — some but not all args) **remains an error** (§5, §6): Elya stays uncurried.
6. **TCE through a closure call** — a tail call whose callee is a closure reuses the current continuation (no frame), so it is bounded exactly like a named-function tail call. Held to the two-sided teeth of every prior TCE claim (§3.3).
7. **cek == tree extends to closures** — `Value::Closure` and its call path exist on **both** evaluators; the differential oracle gains a higher-order program (§3.4).

### 1.2 Surface additions

Over Slice 4a's surface, 4b-1 adds one production — a lambda in expression position:

```elya
type Option(a) { None, Some(a) }
type List(a)   { Nil, Cons(a, List(a)) }

fn map(xs, f) {
  match xs {
    Nil        -> Nil
    Cons(h, t) -> Cons(f(h), map(t, f))
  }
}

pub fn main() {
  let inc  = fn(n) { n + 1 }          // a closure over nothing
  let by   = 10
  let bump = fn(n) { n + by }         // captures `by`
  let some = Some                     // a bare constructor as a value
  let _    = map(Cons(1, Cons(2, Nil)), bump)
  let _    = map(Cons(1, Nil), some)  // Cons(Some(1), Nil)
  io.println("ok")
}
```

New syntax: `fn( PARAMS ) { BLOCK }` as an **expression**. Everything else (calls, `let`, `match`, constructors) is unchanged.

### 1.3 What Slice 4b-1 does NOT do (deferred — §10)

- **Effect-carrying closures & higher-order effect relay** — closures whose bodies *perform* effects, HOFs that relay a callback's effects, E0426/discharge interaction, dynamic-scoping tests → **4b-2**.
- **Row-polymorphic combinators & the row value-restriction stress test** → **4b-3**.
- **Recursive local closures** — a `let`-bound lambda does **not** see its own name; self/mutual recursion remains a top-level-`fn` concern. (This is what guarantees no `Rc` cycles in 4b — §3.1.)
- **Currying / partial application** — `Cons(1)` stays an error; no auto-curry, no `_` sections.
- **Typed lambda parameters / return annotations, closures over mutable state, tuples** — out of scope.
- **The "relaxed" (constructor) value restriction** — `Some(v)` where `v` is a value is treated as a non-value here (§2.4); the refinement is unnecessary because such bindings are monomorphic on the current corpus, and is flagged (§11).

---

## 2. Type system

### 2.1 Lambda typing

`Expr::Lambda { params, body }` types to a `Ty::Fn`:

- Allocate a fresh type variable per parameter; bind them (monomorphically) in a pushed scope.
- Allocate a **fresh ambient row** `ρ_λ` for the body — a lambda has its *own* latent effects, independent of the enclosing ambient. Infer the body block under `ρ_λ`.
- The lambda's type is `Ty::Fn(param_tys, resolve_row(ρ_λ), body_ty)`.
- **Creating a lambda performs nothing.** The lambda expression adds *no* effect to the *enclosing* ambient; only *calling* it does, via `infer_call` pouring the arrow's row into the caller's ambient ([`types.rs:934-935`](../../../src/types.rs#L934)). This mirrors how each top-level `fn` body gets its own ambient.

For 4b-1, lambda bodies are pure or perform only already-ambient effects, so `ρ_λ` typically resolves to a concrete/pure row; the *interesting* residual-relay behavior (and the `close_unrelayed_residual` nuance, [`types.rs:1098-1118`](../../../src/types.rs#L1098)) is exercised in **4b-2**. The typing rule is written generally here so 4b-2 needs no re-opening.

### 2.2 Calling a closure — already built

No new inference code for the call: a lambda-valued callee is neither `Expr::Qualified` (builtin) nor a constructor/operation `Expr::Var`, so it flows to `infer_call`'s **general path** ([`types.rs:926-936`](../../../src/types.rs#L926)), which infers the callee, unifies it against a fresh `Ty::Fn` of the argument arity, and pours the latent row into the ambient. Arity/type mismatches surface through the **existing** arrow-unification diagnostics (E0400 family) — a closure called with the wrong number of arguments is a `Ty::Fn`-arity unification failure, not a new code.

### 2.3 Bare constructors as function values

In `Expr::Var`, the 4a branch that emitted E0433 for a bare n-ary constructor ([`types.rs:711-718`](../../../src/types.rs#L711)) is replaced: a bare constructor now **instantiates its scheme** like any other name. The scheme is *already* an arrow (`Some : ∀a. (a) / {} -> Option(a)`, registered in 4a), so a bare constructor is simply a polymorphic function value — no special typing. `emit_unapplied_ctor` / E0433 is retained **only** for the *partial-application* case (§6).

### 2.4 The value restriction

Introduce a pure syntactic predicate:

```
is_syntactic_value(e) =
  | Lambda            -> true      // a function literal is a value
  | Var | Qualified   -> true      // a name (incl. bare ctor, top-level fn) is a value
  | Int|Float|Str|Bool|Unit -> true
  | _                 -> false     // Call, Match, If, Block, Binary, Unary, Handle, Resume
```

At the one generalization site ([`types.rs:687`](../../../src/types.rs#L687)), generalize the let-bound type **iff** `is_syntactic_value(rhs)`; otherwise insert the (resolved, ungeneralized) monotype. The predicate gates **both** `vars` and `row_vars` in one place. Consequences:

- `let id = fn(x) { x }` → value → `id : ∀a. (a) -> a` — `id(1)` and `id("a")` both type-check.
- `let e = Nil` → value (a `Var`) → `∀a. List(a)`, as today.
- `let xs = Cons(1, Nil)` → **non-value** (an application) → monotype `List(Int)`. No behavioral change (already monomorphic).
- `let x = f(y)` → non-value → monotype. This is the case that *was* unsound to generalize once lambdas/continuations exist.

**Expected to be behavior-preserving on the current corpus:** every non-value `let` RHS in today's tests infers a *monomorphic* type (no free vars), so declining to generalize it changes nothing. This is a claim to *verify* at implementation (§8), not merely assert.

---

## 3. Evaluation

### 3.1 Value representation & the no-cycle guarantee

```rust
Value::Closure { params: Rc<[String]>, body: Rc<Spanned<Block>>, env: Env }
Value::CtorFn  { name: String, arity: usize }   // an unapplied n-ary constructor
```

(The body is a `Block` — consistent with `Expr::If`'s block operands and with how `FnDecl` bodies feed `eval_block_state`; `params` narrows to just the names the runtime needs.)

- `Env` is a persistent parent-pointer chain (`Env(Option<Rc<Scope>>)`, [`eval.rs:133`](../../../src/eval.rs#L133)); capture is an **O(1) `Rc` clone**, and cloning a `Value::Closure` is O(1).
- **No `Rc` cycles are possible in 4b-1.** A cycle would require a closure to capture an environment transitively containing itself — which needs local recursion or mutation, both deferred (§1.3). So plain `Rc` suffices (no `Weak`, no `RefCell`), and the iterative-`Drop` discipline from 4a is unaffected. (When 4b-later adds recursive local closures, the cycle question is confronted *there*, with a knot-tying/`Weak` self-slot — flagged §11.)
- `PartialEq for Value`: closures and ctor-functions compare **`false`** (functions are not comparable; the type system never lets a base-typed `==` observe one), consistent with the existing `Value::Resume` treatment ([`eval.rs:56-69`](../../../src/eval.rs#L56)).

### 3.2 Construction & call on the CEK machine

- **Creating a closure** (`Expr::Lambda`) returns `Value::Closure` capturing the *current* env — a pure return, no frame.
- **A bare n-ary constructor** (`Expr::Var` naming a constructor of arity > 0) returns `Value::CtorFn { name, arity }`. (Nullary constructors keep returning `Value::Ctor(name, [])` as in 4a.)
- **Calling** extends `apply_callee` ([`eval.rs:1069`](../../../src/eval.rs#L1069)) with two arms:
  - `Value::Closure { params, body, env }` — check `params.len() == args.len()`, bind params onto the **captured** `env`, then `eval_block_state(body, call_env, k)` **reusing `k`** (no frame — the TCE-preserving structure, identical to the `Value::Fn` arm).
  - `Value::CtorFn { name, arity }` — check `arity == args.len()`, then `State::Return(Value::Ctor(name, CtorArgs(Rc::new(args))), k)`.
- The tree-walker gains the mirror-image arms in its `apply`/call path. **Both** evaluators implement both arms — the cek == tree gate (§3.4) covers them.

Arity mismatches at runtime remain the existing `E0300` runtime error (defensive; the type system rejects them statically first).

### 3.3 TCE through a closure call — two-sided

The closure-call arm reuses `k`, so a tail call to a closure is bounded. Teeth, in the discipline of `K_MAX`/`K_MAX_EFF`/`K_MAX_MATCH`:

- **Bounded side** — a top-level recursive driver whose *base case tail-calls a closure argument* runs at depth ~1,000,000 in O(1) host stack. (This threads a closure into tail position without needing recursive *local* closures.) Measure the residual continuation depth `K_MAX_CLOSURE` and pin it as a constant (expected small, e.g. ≤ 3); assert it does not grow with N.
- **Grow-control side** — a *non-tail* variant (closure call under a pending `+`) grows with N, proving the bound is a real property of tail position, not an accident.

"Fix the machine, don't raise the constant" holds: if the measured depth is not constant, the closure-call arm is pushing a frame it should reuse.

### 3.4 Cross-check net *extends*

`cek == tree` is a hard, per-golden gate (via `run_both`, live since 4a Task 2). 4b-1 registers a higher-order effect-free program (e.g. `map`/`compose` over a small `List`, plus a bare-constructor-as-value use) in both the `tests/adt.rs` `run_both` goldens **and** the `tests/crosscheck.rs` hand-written corpus, so the differential oracle provably extends to closures and ctor-functions.

---

## 4. Resolution & scoping

- **Lambda parameters** introduce a new local scope for the body; free variables in the body resolve to enclosing bindings (the existing block/scope machinery — a lambda body is scoped like a nested block plus its parameters).
- A `let`-bound lambda does **not** bind its own name in its body (no local recursion — §1.3); a self-reference resolves to an outer binding or is the existing unresolved-name error (E0200).
- Bare constructors are already validated by the resolver (4a); a bare constructor in value position is simply a resolved name — no new resolve logic beyond *not* rejecting it.

## 5. Diagnostics

**4b-1 introduces no new diagnostic code.** E044x is reserved for future closure diagnostics (arity/callability messages, should we choose to specialize them beyond the arrow-unification defaults). 4b-1's diagnostic surface is:

- **E0433 narrows.** Retired for the *bare/unapplied* constructor case (now a value, §2.3). Retained for **partial application** — `Cons(1)` given 1 of 2 arguments — with its message updated to drop the now-fulfilled "become values in 4b" clause. Suggested wording: *"constructor `Cons` takes 2 arguments but 1 was given; Elya constructors are not curried — apply all arguments, or wrap in a lambda (`fn(t) { Cons(1, t) }`)."* Held to the E042x no-gibberish, named-label standard.
- **Wrong-arity / non-callable closure calls** surface via existing arrow-unification (E0400 family) — a closure called with the wrong argument count is a `Ty::Fn`-arity mismatch; a non-function called as one is a `unify(non-arrow, Ty::Fn)` failure.

## 6. Saturation & the partial-application boundary

Elya remains **uncurried** in 4b-1. A constructor or function is either fully applied (a call) or referenced bare (a value). There is no `Cons(1)` partial value. This keeps the value/computation boundary crisp for the value restriction and defers currying as a genuine, separable language decision (§10). The bare-constructor-to-value rule (§2.3) is *not* currying — it is nullary reference of the whole constructor, which then requires a saturated call.

## 7. Pipeline & module changes

| Layer | Change |
|---|---|
| `lex` | none — reuses `KwFn` and `->`/block tokens. |
| `ast` | add `Expr::Lambda { params, body: Rc<Spanned<Block>> }` reusing `FnDecl`'s parameter representation; pretty-printer arm. |
| `parse` | `atom()` dispatches `KwFn` in expression position → lambda parser (params list + block body). |
| `resolve` | scope lambda params for the body; accept bare constructors in value position. |
| `types` | lambda typing (§2.1); bare-ctor scheme instantiation replacing E0433's bare case (§2.3); `is_syntactic_value` gate at the `let` generalization site (§2.4). |
| `eval` | `Value::Closure`, `Value::CtorFn`; `PartialEq` arms (compare false); construction + call on **both** tree and CEK (§3). |
| `main`/`lib` | none (warning surfacing already shipped in 4a). |

Layering (tests/arch/layering.rs) is unchanged — no new module.

## 8. Testing strategy

- **End-to-end, both evaluators** (`tests/adt.rs` `run_both`): a closure capturing a local; a HOF `map`/`compose`; a bare-constructor-as-value (`map(xs, Some)`); `let id = fn(x){x}` used at two types. Each asserts `cek == tree` by construction.
- **Value restriction** (`tests` in `types.rs` and/or a UI fixture): `let id = fn(x){x}; id(1); id("a")` **type-checks** (polymorphism); a control that a *non-value* binding is **not** over-generalized (e.g. a binding that would only type-check if wrongly generalized fails with E0400). Plus the **regression assertion** that the whole existing corpus still passes (the tightening is behavior-preserving — §2.4).
- **E0433 narrowing** (`tests/ui`): bare `Some` as a value **compiles**; `Cons(1)` (partial) still **errors** with the updated E0433 message.
- **TCE** (`tests/tce_closure.rs`, new): the two-sided teeth of §3.3 — bounded ~1M driver tail-calling a closure (pinned `K_MAX_CLOSURE`), and a non-tail grow control.
- **Cross-check** (`tests/crosscheck.rs`): one effect-free higher-order program added to the hand-written corpus.

## 9. Build order (tasks — a plan per this sub-slice)

Sequenced so a **minimal end-to-end closure program** (`let inc = fn(n){n+1}; inc(41)`) runs lex→parse→resolve→type→eval as early as possible (vertical-slice discipline inside 4b-1), with the cek == tree gate held green from the first task that evaluates a closure.

1. **AST + parser** — `Expr::Lambda`, pretty-printer, `atom()` dispatch; parser round-trip tests.
2. **Eval (both) — construction + call, TCE-preserving** — `Value::Closure`, both evaluators' call arms reusing `k`; the minimal program runs; `run_both` closure golden green (**cek == tree hard gate from here**).
3. **Types — lambda typing** — `Ty::Fn` synthesis with a fresh body ambient; general call path exercised.
4. **Value restriction** — `is_syntactic_value` gate at the generalization site; polymorphism test + non-value control + full-corpus regression.
5. **Bare constructors as values (retire E0433's bare case)** — `Value::CtorFn`; scheme instantiation on the type side; E0433 narrowed to partial application with updated message; UI fixtures.
6. **TCE two-sided** — `tests/tce_closure.rs`, pin `K_MAX_CLOSURE`, grow control.
7. **Integration & exit gate** — broaden the cross-check corpus; run the full 4b-1 exit gate; commit + push.

### Exit criterion

Closures capture and run on **both** evaluators with `cek == tree` green; lambda-bound names are **soundly** polymorphic (value restriction gates generalization, verified behavior-preserving on the existing corpus); bare n-ary constructors are first-class function values (E0433 retired for the bare case, retained for partial application); a closure tail call is bounded (pinned `K_MAX_CLOSURE`, two-sided); the full suite (4a's 133 + new) is green; `cargo fmt --all` + `sh scripts/check.sh` clean.

## 10. Risks & mitigations

- **Value restriction breaks the corpus.** *Mitigation:* §2.4's behavior-preserving analysis + the full-corpus regression assertion (Task 4). If a non-value binding *did* rely on generalization, that binding was relying on a currently-unsound path — surfacing it is correct, and the fix (bind the lambda to a name / annotate) is local.
- **TCE regression through the closure arm.** *Mitigation:* the closure-call arm is a structural copy of the `Value::Fn` arm (reuse `k`, no frame); the two-sided teeth catch any accidental frame.
- **Bare-ctor value representation drift.** A `Value::CtorFn` that is *not* consumed by a call must still behave (e.g. stored in a list, compared). *Mitigation:* it is a first-class `Value` (compares false, prints as a function); the cross-check and `run_both` goldens exercise `map(xs, Some)` — a stored, later-called ctor-function.
- **Dynamic vs. lexical effect scoping confusion (looking ahead to 4b-2).** *Mitigation:* 4b-1 states the invariant explicitly (§0, §2.1) — the closure captures *no* handler stack; effects resolve at the call site. 4b-2's tests enforce it.

## 11. Deferred / Honestly-Flagged

- **Recursive local closures** (self/mutual) — deferred; re-opens the `Rc`-cycle question (Weak self-slot / knot-tying) when added.
- **Effect-carrying closures, higher-order relay, E0426/discharge interaction, dynamic-scoping tests** → 4b-2.
- **Row-polymorphic combinators + the effect-row value-restriction stress test** → 4b-3 (the soundness capstone toward linear/affine).
- **Currying / partial application / `_` sections** — a separate language decision, not scheduled.
- **The relaxed (constructor) value restriction** — `Some(v)` treated as a non-value here; harmless on the current corpus (§2.4); revisit only if a real program needs a polymorphic constructor-application binding.
- **Typed lambda parameters & return annotations** — HM infers them for now.

## 12. Milestone Checklist (Slice 4b-1)

- [ ] `Expr::Lambda` parses (`fn(x, y) { … }`), pretty-prints, round-trips.
- [ ] `Value::Closure` captures env; constructs + calls on **both** evaluators; `run_both` closure golden green.
- [ ] Lambda typing yields `Ty::Fn` with a fresh body ambient; general call path types closure calls.
- [ ] `is_syntactic_value` gates let-generalization; `id = fn(x){x}` is polymorphic; non-value control fails; **full existing corpus still green**.
- [ ] Bare n-ary constructor is a `Value::CtorFn`; E0433 retired for the bare case, retained (updated message) for partial application.
- [ ] `tests/tce_closure.rs`: bounded ~1M closure tail call (pinned `K_MAX_CLOSURE`) + non-tail grow control.
- [ ] Cross-check corpus gains an effect-free higher-order program.
- [ ] `cargo fmt --all` + `sh scripts/check.sh` clean; committed + pushed.
