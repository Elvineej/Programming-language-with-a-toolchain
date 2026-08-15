# Elya — Slice 4c-2 Design Specification: Generic (Parametric) Effects

- **Codename:** Elya
- **Slice:** 4c-2 — the second sub-slice of the generic-effects/State arc (Slice 4c); the real type-system half, following the coverage half (4c-1, parameter-passing State)
- **Status:** Design draft — awaiting review before an implementation plan is written
- **Date:** 2026-08-15
- **Depends on:** Slice 4c-1 (complete, `8857ba0`) — parameter-passing `State`; Slice 4a — parametric ADTs (the *template* for this work: `Ty::Con` as a nominal head + `Vec<Ty>` args, unified by head + pairwise args, with monomorphic as the arity-0 case); Slice 3 — the effect-row type system (`unify_row`, ambient threading, discharge, `E042x`).
- **Companion documents:** the language design spec (`.../2026-08-05-elya-language-design.md`, "design spec"); the Slice 3 effects spec (`.../2026-08-06-elya-slice-3-effects.md`, "effects spec"); the Slice 4a ADT spec (`.../2026-08-08-elya-slice-4a-adts-design.md`, "4a spec"); the 4c-1 State spec.

---

## 0. How to read this document

This is a **design spec** for Slice 4c-2 — **generic (parametric) effects**: effect declarations that carry type parameters, e.g. `effect State(s) { fn get() -> s  fn set(v: s) -> Unit }`, usable at any state type. It generalizes 4c-1's monomorphic `State` (fixed to `String`) to one `State(s)` reusable at `Int`, `String`, or any `s`.

**This is a real type-system extension, not a coverage slice — and it reaches into `unify_row`.** During brainstorming I established this with a concrete soundness argument (§2): sound, *compositional* generic effects **require the effect's type argument to travel in the effect row**, because a function can use the type argument concretely without it appearing anywhere else in its type, and only the row can carry it to the handler. So this slice changes the row representation (`EffectRow.labels` gains type arguments) and the core row-unification primitive (`unify_row` gains a pairwise-argument reconciliation arm). This corrects the earlier "small follow-on / no new IR" framing — flagged deliberately and chosen with eyes open.

**The mitigating structure: it is *additive* and *contained*, the exact shape of 4a.** The change to `unify_row` is not a rewrite — it is the same pattern 4a validated for `Ty::Con`: a nominal head plus a `Vec` of arguments, unified by matching the head then unifying the arguments **pairwise**. Effect labels with arguments are that pattern one level up (rows rather than types). And the **monomorphic path is the arity-0 case**: an effect with no type parameters is a label with empty args, and `unify_row` on empty args is exactly today's behavior — so 4c-1's `State`, `IO`, and every existing effect program are undisturbed, precisely as nullary ADTs were undisturbed by parametric ones (§7).

Section refs: "4a spec §X" → the ADT spec; "effects spec §X" → the Slice 3 spec; "design spec §X" → the language spec; bare "§X" → this document.

---

## 1. Scope

### 1.1 What Slice 4c-2 delivers

1. **Parametric effect declarations** — `effect Name(p, …) { fn op(args) -> ret … }` where the operation signatures may reference the effect's type parameters. Recursive/higher-kinded params are out of scope (§12).
2. **Type arguments in the effect row** — an effect appears in a row as `Name(τ, …)`; `State(Int)` and `State(String)` are distinguishable, and `unify_row` reconciles the arguments of a matching label pairwise (§4).
3. **Operations as schemes** — `OpInfo` carries the effect's type parameters; a *perform* instantiates them (fresh, shared across a computation via the row); a *handle* fixes them for its scope (§5).
4. **Generalized `State`** — one `effect State(s)` used at two distinct state types in one program, output-verified; the monomorphic path (4c-1, `IO`, `Log`) stays green (§7, §10).
5. **Sound composition** — two computations performing the same effect at *different* type arguments in one scope is an `E0423` effect-row mismatch (the fork's chosen diagnostic), naming the effect and the conflicting arguments (§8) — the soundness the name-only row could not provide (§2).

### 1.2 Surface additions

Over 4c-1's surface, one production: **type parameters on an effect declaration**, and the appearance of the effect's parameters in operation signatures:

```elya
effect State(s) {
  fn get() -> s
  fn set(v: s) -> Unit
}
```

`handle`/`with`/`resume` and the state-passing handler shape are unchanged from 4c-1 — only the declaration is now parametric, and the handler works at whatever `s` the body demands.

### 1.3 What Slice 4c-2 does NOT do (deferred — §12)

- **Higher-kinded / constrained effect parameters, multiple distinct instances of one effect in a scope** (`State(Int)` and `State(String)` *simultaneously* under separate handlers with row-level instance distinction) — effect aliases/masking/instances remain deferred (effects spec §11).
- **The relay-plus-own-effect row-leak fix** (the 4c-1/4b-2 tracked obligation) — a separate row-inference change; not folded in here.
- **A `State` / effect standard-library module** — 4c-2 writes handlers inline to prove the mechanism.
- **Linear/affine resource safety** — the frontier arc.

---

## 2. Why the type argument must live in the row (the soundness rationale)

Consider a completely natural function under `effect State(s)`:

```elya
fn f() { set(get() <> "x") }   // read state, append "x", write it back
```

`get() <> "x"` forces the state type `s = String`. But `f`'s type is `() / {State} -> Unit` — **`String` appears in neither the parameters, the result, nor (with a name-only row) the row.** If the row carried only the *name* `State`, then a `State(Int)` handler would type-check `f` (label `State` matches) while at runtime `f` performs `String` operations on `Int` state — **unsound.**

The only place the type argument can travel to reach the handler is the **row itself**. Therefore a sound, compositional design must carry the argument in the row: `f : () / {State(String)} -> Unit`. This is design **B** (chosen); the name-only design **A** is unsound for exactly this natural case and is rejected. This is why 4c-2 necessarily touches the row representation and `unify_row` — it is a soundness requirement, not a stylistic choice.

## 3. Representation — effect labels carry arguments

`EffectRow.labels: BTreeMap<String, Span>` becomes a map from effect name to a small **`EffectLabel { args: Vec<Ty>, span: Span }`** (keyed by name — an effect still appears at most once per row; conflicting instantiations reconcile via §4, they do not coexist). The fan-out — mechanical, each site handles the `args` the way 4a's `Ty::Con` fan-out handled its `Vec<Ty>`:

| Site | Change |
|---|---|
| `resolve_row` | resolve each label's `args` (they contain type vars). |
| type-var substitution / `write_ty` (zonk) | substitute into label `args`. |
| **`free_vars`** (type-var collection) | **must now descend into a `Ty::Fn`'s row label args** — a type variable appearing *only* in an effect argument must still be generalized. This is the one non-obvious fan-out. |
| `free_row_vars` | unchanged in spirit (still collects row vars), but traverses the new structure. |
| `add_effect` / `add_row` | carry and thread `args`. |
| printing (`display_ty` / row rendering) | a label prints `State(Int)`; empty args print bare `State` (so monomorphic output is unchanged). |
| `emit_row_mismatch` | may name a label with its args for the §8 arg-conflict message. |

## 4. `unify_row` — the pairwise-argument reconciliation (the core touch)

`unify_row` today splits each side's *extra* labels (`only1`/`only2`) and treats labels present on **both** sides as "compatible" — the code even notes *"monomorphic ops carry no payloads to reconcile"* ([types.rs:323](../../../src/types.rs#L323)), marking this exact extension point. 4c-2 adds one arm: **for every label present on both rows, reconcile its argument vectors pairwise.**

- Same effect, same arity ⇒ unify `args1[i]` with `args2[i]` for each `i` (ordinary type unification, which binds vars and threads the `s` across performs — §5).
- An **irreconcilable** argument pair (two different concrete types, e.g. `Int` vs `String`) ⇒ emit **`E0423`** "effect row mismatch", naming the effect and the conflicting arguments (help: *"effect `State` used at conflicting type arguments: `Int` vs `String`"*), and **poison the argument** so no cascading `E0400` is also reported (the fork's choice: one row-level diagnostic, no new code).

Everything else in `unify_row` — the `only1`/`only2` split, `absorb`, `unify_tails`, the `E0424` occurs-check — is unchanged. The addition is localized and additive, the 4a `Ty::Con` shape at the row level.

## 5. Operations as schemes — perform and handle instantiation

`OpInfo { effect, params, ret }` gains the effect's **type-parameter variables** (`effect_params: Vec<u32>`), and `params`/`ret` are elaborated *over* them (§6). The parameter is threaded thus:

- **Perform** (the operation-call branch of `infer_call`). Instantiate the effect's parameters with **fresh** type variables `τ̄`; substitute into `params`/`ret`. Add `Name(τ̄)` to the ambient via `add_effect` — which routes through `unify_row`, so a **second** perform of the same effect reconciles its `τ̄` against the first's (§4): the row does the cross-operation linking. The call's result is `ret[τ̄]`.
- **Handle** (`infer_handle`). Allocate one fresh instantiation `τ̄_h` for the handled effect, **seed the body's ambient with `Name(τ̄_h)`** (not the bare name), and type each clause's parameters and `resume` against `params[τ̄_h]`/`ret[τ̄_h]`. Because the body's performs and the clauses now share `τ̄_h` through the row, the handler and the body agree on the state type — and the discharge removes `Name(τ̄_h)` from the residual.

A function that performs an effect polymorphically (e.g. a relay) keeps the effect's argument as a **free type variable in its row**, generalized with the rest of its scheme; a function that uses it concretely (like §2's `f`) has a concrete argument in its row. Both are sound because the argument is *in the row*.

## 6. Effect declarations with type parameters (parser + elaboration)

Mirrors 4a's parametric ADTs exactly:

- **AST/parser:** `EffectDecl` gains `params: Vec<String>`; `effect State(s) { … }` parses the parenthesized parameter list (reusing the ADT type-parameter parsing shape). Operation signatures may name the parameters.
- **Elaboration:** operation signatures are elaborated under a **parameter environment** mapping each declared parameter to a fresh type variable (the 4a `elaborate_adt_ty` pattern) — so `get() -> s` elaborates `s` to the effect's parameter variable rather than an unknown type. `OpInfo` records those variables as `effect_params`.
- **Resolve:** an effect's parameters are in scope for its operation signatures; using an undeclared parameter is the existing unknown-type error.

## 7. The monomorphic path preserved (arity-0)

An effect declared without parameters (`effect Log { fn log(msg: String) -> Unit }`, or 4c-1's `effect State { fn get() -> String … }`, or the built-in `IO`) has `effect_params = []`. Its label carries **empty args**; a perform instantiates nothing; `add_effect` adds `Log()`; `unify_row` reconciles empty arg vectors (a no-op); printing shows bare `Log`. **Every existing effect program is byte-for-byte unchanged** — the arity-0 case *is* the current behavior, exactly as nullary ADTs were the arity-0 case of parametric ADTs. This is a hard regression gate (§10): the entire prior effect + TCE suite stays green, and inferred-scheme snapshots (`tests/effect_types.rs`) are unchanged for monomorphic effects.

## 8. Diagnostics

**No new diagnostic code** (the chosen fork). A conflict between two instantiations of the same effect reuses **`E0423`** ("effect row mismatch"), extended to name the effect and its conflicting arguments in the help line. The no-`%row`-token invariant holds — arguments render as named types (`Int`, `String`, `List(a)`), never internal tokens. `E0420` (unhandled), `E0421`/`E0423` (annotated-row), `E0426` (multi-shot) are otherwise unchanged.

## 9. Pipeline & module changes

| Layer | Change |
|---|---|
| `ast` / `parse` | `EffectDecl.params`; parse `effect Name(p, …)` (reuse ADT type-param parsing). |
| `resolve` | effect params in scope for op signatures (mechanical). |
| `types` | **the substance:** `EffectLabel { args, span }` + fan-out (§3); `unify_row` pairwise-arg arm (§4); `OpInfo.effect_params` + perform/handle instantiation (§5); op-signature elaboration under a param-env (§6); `free_vars` descends into row label args (§3). |
| `eval` | **none** — effects are erased at runtime; the machine already threads `State` dynamically (4c-1). Type arguments are a *static* artifact. |
| `main` / `lib` | none. |

Layering (tests/arch/layering.rs) unchanged; no new module. **Runtime is untouched** — this is purely a type-system slice, so the evaluators and `cek == tree` (effect-free) are unaffected.

## 10. Testing strategy

Output-verified for runs (CEK-only, no oracle behind effects) and scheme/diagnostic-pinned for types:

- **Parametric demonstration:** one `effect State(s)` used at **`Int`** (state a counter, result checked by `==`) **and** `String` (yields `ab`) in one program — output-verified. Proves one declaration serves two state types.
- **Inferred-row pin:** a function performing `State` at a concrete type infers a row naming the argument — e.g. §2's `f` infers `fn() / {State(String)} -> Unit` (scheme-pinned via `tests/effect_types.rs`'s `schemes`), and a polymorphic relay keeps the argument as a quantified variable.
- **Soundness negative (the point of design B):** two functions performing `State` at *different* concrete types composed in one computation (`fs` at `String`, `fi` at `Int`) is **`E0423`**, naming the effect and `Int` vs `String`. Under a name-only row this would be silently accepted — the test is the proof the argument rides in the row.
- **Monomorphic regression (hard gate, §7):** the entire prior effect suite, TCE suite, and monomorphic-effect scheme snapshots stay **green and unchanged**; 4c-1's monomorphic `State` still yields `ab`.
- **`main` discharge / unhandled:** an unhandled generic effect is still `E0420`.

## 11. Build order (sub-slices — a plan per task)

Sequenced so the representation + core-unification change lands first behind the arity-0 regression gate, then parametric surface, then the payoff:

1. **Representation + `unify_row` arg arm, arity-0 only.** Introduce `EffectLabel { args, span }` with the full fan-out (§3) and the `unify_row` pairwise-arg reconciliation (§4), *without* parser changes — all effects are still arity-0. **Hard gate: the entire existing suite stays green and monomorphic scheme snapshots are unchanged** (proves the representation change is behavior-preserving before any parametric surface exists — the same "prove the invariant before the feature consumes it" discipline as the 4b-1 value restriction).
2. **Parametric effect declarations.** `EffectDecl.params` + parser + param-env elaboration + `OpInfo.effect_params` (§6). An `effect State(s)` parses, resolves, and elaborates.
3. **Perform + handle instantiation** (§5). Generic operations perform and handle; the parametric `State` demonstration runs at `Int` and `String` → output-verified.
4. **Soundness + exit gate.** The cross-instantiation `E0423` negative; the inferred-row scheme pins; full-suite exit gate; commit + push.

### Exit criterion

One `effect State(s)` is usable at two distinct state types in one program (output-verified); an effect at conflicting type arguments in one scope is `E0423`; a concrete performer infers a row naming its argument (`{State(String)}`) while a relay keeps it polymorphic; the entire monomorphic path (effects, TCE, snapshots) is green and unchanged; `cargo fmt --all` + `sh scripts/check.sh` clean. The generic-effects/State arc closes.

## 12. Risks & mitigations

- **The `unify_row` change regresses monomorphic effect programs.** *Mitigation:* Task 1 lands the representation + unification change **arity-0 only**, behind the hard "entire suite green + snapshots unchanged" gate, before any parametric surface exists — so a regression is caught with zero parametric confounds.
- **`free_vars` missing the row-arg descent** (an effect-argument type var not generalized). *Mitigation:* §3 flags it explicitly as the one non-obvious fan-out; the parametric relay scheme-pin (a function generic in its state type) fails loudly if the variable is not generalized.
- **Arg conflict double-reporting (`E0423` *and* `E0400`).** *Mitigation:* §4 poisons the conflicting argument after the single `E0423`, suppressing the cascade (the established poison discipline).
- **Scope creep into effect instances / higher-kinded params.** *Mitigation:* §1.3 draws the line — one instantiation per effect per scope; multiple simultaneous instances stay deferred.

## 13. Deferred / Honestly-Flagged

- **Multiple simultaneous instances of one effect** (row-level instance labels), effect aliases/masking, higher-kinded effect parameters → later (effects spec §11).
- **The relay-plus-own-effect row-leak fix** → its own row-inference slice; not folded in.
- **Effect / `State` standard-library module** → post-self-hosting stdlib work.
- **Linear/affine resource safety** → the frontier arc, now the next thing after this arc closes.

## 14. Milestone Checklist (Slice 4c-2)

- [ ] Task 1: `EffectLabel { args, span }` + fan-out + `unify_row` pairwise-arg arm, arity-0 only; **entire suite green, monomorphic snapshots unchanged**.
- [ ] Task 2: `effect State(s)` parses, resolves, elaborates under a param-env; `OpInfo.effect_params`.
- [ ] Task 3: perform + handle instantiation; parametric `State` runs at `Int` and `String` → output-verified.
- [ ] Task 4: cross-instantiation `E0423` negative; inferred-row scheme pins (`{State(String)}` concrete, polymorphic relay); full suite green; committed + pushed. Arc closed.
