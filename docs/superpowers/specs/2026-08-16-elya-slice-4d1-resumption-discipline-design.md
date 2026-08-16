# Elya — Slice 4d-1 Design Specification: Effect Resumption Discipline

- **Codename:** Elya
- **Slice:** 4d-1 — the first piece of the frontier linear/affine-types arc (Slice 4d, "affine resource safety"); the **enabling infrastructure**, not the guarantee itself.
- **Status:** Design draft — awaiting review before an implementation plan is written
- **Date:** 2026-08-16
- **Depends on:** Slice 3 (effects, handlers, `with multi`, `resume`, the E042x diagnostics); Slice 4c-2 (generic effects — the effect-declaration surface this extends).
- **Companion documents:** the language design spec (`.../2026-08-05-elya-language-design.md`, "design spec"; esp. §8.3 "one-shot by default; `multi` opt-in", §13); the Slice 3 effects spec ("effects spec"; esp. §11 deferrals).

---

## 0. How to read this document

This is a **design spec** for Slice 4d-1 — **effect resumption discipline**: an effect declares whether its continuation may be resumed more than once, via a `multi` modifier on the *declaration* (`effect multi Flip { … }`), with one-shot the default. A **conformance rule** requires a `with multi` handler to be handling a `multi`-declared effect. That is the whole slice.

**Why this exists — and what it deliberately does NOT deliver.** The frontier arc's goal (Slice 4d overall) is to make the E0426 hazard *unrepresentable*: an affine resource captured into a continuation that resumes more than once (→ double-free). Making that a type error requires the checker to know, at each perform, whether the continuation may be duplicated. Slice 4d-1 supplies exactly that knowledge as a **static, looked-up property of the effect declaration** — which is what lets the affine check (Slice 4d-2) avoid any reach into `unify_row` (the multiplicity is a declaration field, not a row attribute).

**Scope honesty (load-bearing).** 4d-1 is **enabling infrastructure**. It does **not** make E0426 precise, and it delivers **no affine/linearity guarantee** — those are Slice 4d-2, where the affine resource marker and the "live across a multi-shot perform" rule land, and where E0426's best-effort lint generalizes into a guarantee for resources. **4d-1's exit criteria claim none of that.** What 4d-1 *does* deliver standalone: resumption intent is now **encoded and statically checked** — today *any* effect may be handled `with multi`; after 4d-1 an effect must *declare* itself `multi` to be multi-resumed. That is a real, if modest, soundness/intent tightening, and it is §8.3's existing "one-shot by default, `multi` opt-in" policy relocated from the handler to the declaration, where linearity can look it up.

Section refs: "design spec §X" → the language spec; "effects spec §X" → the Slice 3 spec; "4c-2 spec §X" → the generic-effects spec; bare "§X" → this document.

---

## 1. Scope

### 1.1 What Slice 4d-1 delivers

1. **A `multi` declaration modifier** — `effect multi Name(params) { … }` declares that the effect's continuation may be resumed more than once. An unmarked effect is **one-shot by default** (its continuation is resumed at most once — an *affine* continuation).
2. **The conformance rule** — a `handle … with multi { … }` handler is legal **only** when the handled effect is declared `multi`. Multi-resuming a (default) one-shot effect is a compile error, **E0427**.
3. **The corpus migration** — the few existing `with multi` handlers (all over `Flip`) get `effect multi Flip` declarations. Small and mechanical (§7).

### 1.2 Surface additions

One optional keyword position on the effect declaration:

```elya
effect multi Flip { fn flip() -> Bool }     // may be multi-resumed
effect Exn        { fn fail() -> Unit }      // one-shot (default) — cannot be `with multi`
```

`handle`/`with`/`with multi`/`resume` are otherwise unchanged. The `multi` keyword already exists (used by `with multi`); 4d-1 accepts it in one new position (after `effect`).

### 1.3 What Slice 4d-1 does NOT do (deferred — §8)

- **The affine resource marker** (`linear type File { … }`) and the **"live across a multi-shot perform" liveness rule** — Slice **4d-2** (the guarantee). Settled in brainstorming, recorded (§8), *not* built here.
- **Making E0426 precise / generalizing it into a guarantee** — 4d-2. E0426 is **unchanged** by 4d-1 (still the best-effort observable-effect lint).
- **Sound static double-`resume` detection (E0425)** — still deferred (effects spec §11; needs control-flow analysis). 4d-1's conformance rule governs the *`with multi` keyword*, not runtime double-resume within a one-shot clause, which stays a runtime check.
- **Any change to the runtime** — resumption discipline is a compile-time concern; `eval.rs` is untouched (§6).

---

## 2. The `multi` declaration modifier

`EffectDecl` gains `is_multi: bool` (default `false` = one-shot). The parser accepts an optional `multi` keyword immediately after `effect`:

```
effect  [multi]  Name  [ ( param, … ) ]  { op … }
```

`multi` reuses the existing `KwMulti` token. A monomorphic or generic effect may be `multi` or not, orthogonally (the modifier composes with 4c-2's type parameters). The default (`false`) means every existing effect declaration keeps its current meaning — one-shot — so the *declaration* surface is backward-compatible; only the *conformance rule* (§3) is a behavior change, and only for `with multi` handlers.

**Semantics.** `multi` on the declaration means "a handler for this effect *may* resume its continuation more than once." It does not force multi-resumption — a `multi` effect may still be handled one-shot (a non-`multi` handler). It is the *permission*, and it is the property Slice 4d-2's affine check will look up (an effect that *may* be multi-resumed is where an affine resource must not be live).

## 3. The conformance rule (E0427)

At a `handle body with multi { … }`, the handled effect must be declared `multi`. Concretely, in `infer_handle`, when `handler.multi` is true, look up the handled effect's declared discipline; if it is not `multi`, emit **E0427**:

> **E0427** — *"a one-shot effect cannot be handled with `multi`"*, labelling the `with multi` handler, with help: *"effect `Exn` is one-shot (its continuation is resumed at most once); declare it `effect multi Exn { … }` to allow multi-shot resumption, or remove `multi`."*

The four cases:

| Effect declared | Handler | Result |
|---|---|---|
| one-shot (default) | plain | OK (the common case, unchanged) |
| one-shot (default) | `with multi` | **E0427** |
| `multi` | plain | OK (a `multi` effect handled once) |
| `multi` | `with multi` | OK |

Held to the E042x named-label, no-`%row`-token diagnostic discipline. **E0427** is the next free effect-diagnostic code (E0420–E0426 taken; verified).

**Where the discipline is stored for lookup.** Populate a `HashMap<String, bool>` (effect name → `is_multi`) from the effect declarations during elaboration (alongside the `OpInfo` table), so the conformance check — and, later, Slice 4d-2's affine check — is a direct lookup, not a row operation. This is the concrete mechanism behind "no `unify_row` reach": resumption multiplicity is a declaration fact, read by name.

## 4. Interaction with the existing effect system

- **`resume` / one-shot runtime enforcement (E0425)** — unchanged. A one-shot effect (or a plain handler of a `multi` effect) still enforces at runtime that its continuation is resumed at most once. 4d-1 adds a *static* gate on the `with multi` *keyword*; it does not replace the runtime double-resume check (whose sound static form still needs CFA, effects spec §11).
- **E0426 (multi-shot cleanup lint)** — **unchanged.** It still fires at a `with multi` handler whose body performs an observable effect. (After 4d-1, such a handler is guaranteed to be over a `multi`-declared effect — a small consistency gain, but E0426's *logic and best-effort nature are not changed*, and 4d-1 claims no precision improvement.)
- **Generic effects (4c-2)** — orthogonal; `effect multi State(s) { … }` is well-formed (a multi-shot generic effect), though `State` is naturally one-shot.
- **Discharge / rows** — untouched. The row carries effect names (+ 4c-2 args) exactly as before; `is_multi` is *not* in the row.

## 5. Diagnostics

**One new code, E0427** (§3): a `with multi` handler over a one-shot effect. No other diagnostic changes; E0420–E0426, E0430–E0433 unchanged.

## 6. Pipeline & module changes

| Layer | Change |
|---|---|
| `lex` | none (`KwMulti` exists). |
| `ast` | `EffectDecl.is_multi: bool`. |
| `parse` | accept optional `multi` after `effect` (one line, mirroring the handler-side `KwMulti` eat). |
| `resolve` | none. |
| `types` | build the `effect_name -> is_multi` table during effect elaboration; the conformance check in `infer_handle` (emit **E0427** when `handler.multi` and the effect is one-shot). |
| `eval` | **none** — runtime is untouched (§1.3). |
| tests | new UI/type fixtures for E0427 and for a legal `multi`-effect handler; the `with multi` corpus migration (§7). |

Layering (tests/arch/layering.rs) unchanged; no new module.

## 7. The corpus migration

Grounding measured, not assumed: the corpus has **8 `with multi` usages, all over the `Flip` effect**. Under the conformance rule each such handler's `effect Flip { … }` declaration must become `effect multi Flip { … }`. This is a **mechanical source migration of a handful of test programs** — the `with multi` handlers and their assertions (outputs like `TrueFalse`) are otherwise unchanged. This is expected surface-change adaptation (a new required declaration), **not** a weakening of any assertion; every migrated test asserts the same behavior it did before.

## 8. Testing strategy

- **Declaration parses:** `effect multi Flip { … }` parses with `is_multi == true`; an unmarked effect has `is_multi == false`.
- **Conformance — the four cases (§3):** `with multi` over a `multi` effect type-checks; `with multi` over a one-shot (default) effect is **E0427** (a UI fixture, named-label, no `%row`); plain handlers over either are fine.
- **Migration green:** the migrated `with multi` corpus (Flip → `multi` Flip) type-checks and runs with **identical output** to before (the multi-shot golden behavior is preserved — e.g. the `resume(True) <> resume(False)` collect still yields its pinned string).
- **Runtime unchanged:** an output-verified multi-shot program still produces its golden output (no `eval.rs` change).
- **Regression:** the full prior suite is green (only the `with multi` sources migrated; no assertion changed).

## 9. Build order (tasks — a plan per this sub-slice)

1. **`multi` on the effect declaration + parse.** `EffectDecl.is_multi`; parser accepts `effect multi …`; a parse/round-trip test. (No conformance yet — existing `with multi` still allowed, suite green.)
2. **The conformance rule + E0427 + migration.** The `effect_name -> is_multi` table; the `infer_handle` check emitting E0427; migrate the `with multi` corpus (Flip → `multi` Flip); the four-case tests + the E0427 fixture; exit gate.

### Exit criterion

`effect multi Flip { … }` parses (`is_multi`); a `with multi` handler over a one-shot effect is **E0427** while over a `multi` effect it type-checks; the migrated multi-shot corpus runs with unchanged output; the full suite is green; `cargo fmt --all` + `sh scripts/check.sh` clean. **No claim of precise E0426 and no affine guarantee — those are 4d-2.**

## 10. Risks & mitigations

- **Over-reach into rows.** *Mitigation:* `is_multi` is a declaration field looked up by name — explicitly *not* a row attribute; the row and `unify_row` are untouched (§4). This is the whole point of the targeted/declaration design.
- **Migration mistaken for masking a regression.** *Mitigation:* §7 draws the line — the migration adds a now-required `multi` keyword to declarations and changes **no assertion**; migrated tests assert identical behavior. (Distinct from the 4c-2 arity-0 gate, which forbade edits precisely because it claimed behavior-preservation; here the surface *intentionally* changes for `with multi`.)
- **Scope creep into the affine guarantee.** *Mitigation:* §1.3 and the exit criterion draw a hard line; 4d-1 ships the modifier + conformance only.

## 11. Deferred / Honestly-Flagged (recorded for Slice 4d-2)

- **The affine guarantee (Slice 4d-2):** a **`linear type File { … }`** modifier makes a type's values **affine** (use *at most* once — the double-free hazard is ≥2 uses, and affine forbids exactly that; full linear/exactly-once "no-leak" is a stronger later refinement). The rule: **an affine resource may not be *live across* a perform of a `multi` effect** (used after such a perform in its delimited scope), computed **sound-but-strict** over the AST (the value-restriction / 4b-3-row-teeth discipline). This **consumes 4d-1's `is_multi` lookup** and **generalizes E0426 into a guarantee** for resources.
- **GC interaction (settled, unchanged):** affine is a **static use-discipline layered on the tracing GC** — not a memory scheme. The GC still allocates and reclaims all values (design-spec §2); "consuming" a resource (e.g. closing a file) is an ordinary operation, not a deallocation. **No borrow checker, no ownership, memory model untouched** — the resource-safety-in-Elya's-idiom reconciliation.
- **Sound static E0425** (double-`resume` in a one-shot clause) — still deferred; needs CFA (effects spec §11).
- **The relay-plus-own-effect row-leak fix** — its own row-inference slice; not here.

## 12. Milestone Checklist (Slice 4d-1)

- [ ] `EffectDecl.is_multi`; `effect multi Flip { … }` parses (`is_multi == true`); unmarked = `false`.
- [ ] `effect_name -> is_multi` table built during elaboration.
- [ ] Conformance in `infer_handle`: `with multi` over a one-shot effect is **E0427** (named-label fixture, no `%row`); over a `multi` effect type-checks; plain handlers fine.
- [ ] `with multi` corpus migrated (Flip → `multi` Flip); multi-shot goldens run with **unchanged output**.
- [ ] `eval.rs` untouched; full suite green; `cargo fmt --all` + `sh scripts/check.sh` clean.
- [ ] Exit criteria assert **no** precise-E0426 / affine-guarantee win (those are 4d-2).
