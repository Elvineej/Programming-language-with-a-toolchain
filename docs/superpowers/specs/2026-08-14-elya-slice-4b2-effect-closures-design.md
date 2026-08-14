# Elya — Slice 4b-2 Design Specification: Effect-Carrying Closures & Higher-Order Relay

- **Codename:** Elya
- **Slice:** 4b-2 — the second of three closure sub-slices (Slice 4b, "lambdas/closures")
- **Status:** Design draft — awaiting review before an implementation plan is written
- **Date:** 2026-08-14
- **Depends on:** Slice 4b-1 (complete, `92dd6db`) — lambdas, closures on both evaluators, the value restriction, bare constructors as function values; and Slice 3 — algebraic effects & handlers with row-polymorphic inference, the discharge pass, one-shot/multi-shot resume, and the E0420–E0426 diagnostics.
- **Companion documents:** the language design spec (`.../2026-08-05-elya-language-design.md`, "design spec"); the Slice 3 effects spec (`.../2026-08-06-elya-slice-3-effects.md`, "effects spec"); the Slice 4b-1 closures spec (`.../2026-08-13-elya-slice-4b1-closures-design.md`, "4b-1 spec").

---

## 0. How to read this document

This is a **design spec** for Slice 4b-2 — **effect-carrying closures** (lambdas whose bodies *perform* effects) and **higher-order relay** (functions that take a closure and relay its latent effects). It is the second of three sub-slices; **4b-1** (basic closures) is complete, **4b-3** (row-polymorphic combinators + the effect-row value-restriction stress test) follows and is out of scope here (§11).

**This is a precision-and-coverage slice, stated honestly.** During brainstorming I verified against the *current* (post-4b-1) compiler that the relay machinery **already works end-to-end**:

- `fn apply(f, x) { f(x) }` applied to `fn(n) { log(n) }`, handled at the call site, resumes through the closure and yields the right value (probe output `hi!`).
- A **let-bound** HOF lambda `let apply = fn(f, x) { f(x) }` is **row-polymorphic** via the value restriction: the same `apply` used at a *pure* row and at `{Log}` in one program type-checks and runs (probe output `hi!`). This is exactly "3b's row-polymorphism earning out on a **non-top-level** function" — the composition of 4b-1's value restriction with 3b's relay path.

So 4b-2 introduces **one** small type-system change (a precision fix, §2) and otherwise **discharges a dormant obligation with coverage**: the effects-spec §11 test obligation for `close_unrelayed_residual`'s relay path, which had no lambdas to exercise it in 3b. It adds **no new diagnostic code and no new runtime machinery.**

Section refs: "design spec §X" → language design spec; "effects spec §X" → Slice 3 spec; "4b-1 spec §X" → Slice 4b-1 spec; bare "§X" → this document.

---

## 1. Scope

### 1.1 What Slice 4b-2 delivers

1. **The precision fix (Fork A)** — a lambda's latent effect row gets `close_unrelayed_residual` applied, exactly as top-level fns do. A **concrete-effect** lambda (`fn(n){ log(n) }`) gets a **minimal, closed** row `{Log}` instead of a spurious `{Log | ρ}`; a **relay** lambda keeps its open, row-polymorphic tail. This corrects both the printed type and its generalization (§2).
2. **Relay coverage — the §11 obligation, discharged** — a test corpus proving effect-carrying closures relayed through higher-order functions work across **inference** (row-poly schemes), **execution** (run + verified output on the CEK), and **discharge** (§3).
3. **Row-polymorphism on non-top-level functions** — pinned tests for a **let-bound** HOF lambda used at two distinct rows (pure and effectful) in one program — the moment 3b's row-poly earns out beyond top-level fns (§3).
4. **Dynamic-scoping lock** — a test proving a closure performs against the handler dynamically enclosing its **call site**, not its definition site (§4).
5. **E0426 × closures** — a test proving the multi-shot cleanup lint fires when a closure performs an *observable* effect under a multi-shot handler (§5).
6. **The known-limitation coverage (Fork C)** — the relay-plus-own-effect case (`fn(f,x){ log(x)  f(x) }`) exercised, its inferred row pinned by a test and documented honestly; fixed only if the row is genuinely wrong (§6).

### 1.2 Surface additions

**None.** 4b-2 adds no syntax — effect declarations, `handle`/`with`, `resume`, and lambdas are all already in the language. The whole slice is inference precision plus test coverage over the existing surface.

### 1.3 What Slice 4b-2 does NOT do (deferred — §11)

- **Lambda effect-row annotations** (`fn(x) / {Log} { … }`) with exact-row checking — deferred (Fork B). Lambdas stay inference-only; annotation is a clean, separable parser + `check_exact_row` addition for later.
- **Row-polymorphic combinator library & the effect-row value-restriction stress test** → 4b-3.
- **Deep multi-shot × closure interactions** (a closure captured into a continuation that resumes many times, resource-safety of captured closures) — beyond the E0426 lint test; the full guarantee is the frontier linear/affine arc.
- **`cek == tree` on effectful programs** — *not in scope and not possible by design*: the tree-walker does not evaluate effects (`Expr::Handle | Expr::Resume => Err("effects are not evaluated yet")`, since 3c). Effect-carrying programs run **CEK-only** (`run_source`), exactly like the effects-spec corpus. `cek == tree` remains the effect-free oracle (4b-1).

---

## 2. The precision fix — closing lambda residuals (Fork A)

**The problem, verified.** 4b-1's lambda-typing arm resolves the body's ambient row but never closes its residual tail. A concrete-effect lambda therefore keeps a dangling open row var, and because a lambda is a syntactic value, generalization then **quantifies** it. Measured against the current compiler:

```
fn make_logger() { fn(n) { log(n) } }
  now:   forall a. fn() -> fn(String) / {Log | a} -> Unit     ← spurious row-poly
  4b-2:            fn() -> fn(String) / {Log} -> Unit          ← minimal, closed
```

The `{Log | a}` and its `forall a` are wrong: the lambda performs *exactly* `Log`. The open tail is harmless to *execution* (an unconstrained tail contributes no labels) but it pollutes inferred types and makes concrete-effect lambdas spuriously polymorphic in their effect row.

**The fix.** In the `Expr::Lambda` inference arm ([types.rs](../../../src/types.rs)), after inferring the body under the lambda's fresh ambient `lam_amb` and before building the arrow, call the **existing** `close_unrelayed_residual(lam_amb, &param_tys)`. Its logic is exactly right for lambdas:

- **Concrete-effect lambda** (`fn(n){ log(n) }`): the residual tail is not present in any parameter's row → closed to pure → row `{Log}`.
- **Relay lambda** (`fn(f, x){ f(x) }`): the tail *is* present in parameter `f`'s row → left open → stays row-polymorphic. Verified: `relay` already infers `forall a b c. fn(fn(a)/{c}->b, a)/{c}->b`, and this must be preserved.

This reuses the top-level fn discipline verbatim; no new function, no algorithm change. It is the single load-bearing code change of 4b-2.

**Regression guard.** Because `close_unrelayed_residual` only ever *closes an otherwise-free tail*, it must not change any relay row or any already-closed row. The full existing suite (145) staying green is the guard; the relay scheme `forall a b c. …` is pinned so a regression that over-closes a relayed tail is caught.

## 3. The relay, activated (the §11 obligation)

The effects-spec §11 flagged `close_unrelayed_residual`'s relay path as a test obligation: in 3b it could only be exercised by passing a top-level fn *by name*, and only the **pure** relay (`run_it`) was in the corpus. Lambdas make the effectful relay writable, and 4b-2 discharges the obligation with a corpus that covers all three layers:

- **Inference** — pin the row-poly schemes: `relay` (top-level) is `forall a b c. fn(fn(a) / {c} -> b, a) / {c} -> b`; a let-bound `apply` generalizes the same way (value restriction quantifies the row var).
- **Execution** — run effect-carrying closures relayed through HOFs on the CEK and assert concrete output. Verified probe: `apply(fn(n){ log(n) }, "hi")` under a `Log` handler that `resume(m <> "!")`s yields `hi!`.
- **Discharge** — the relayed effect is discharged by the handler at the call site; an *un*handled relayed effect is `E0420` (the discharge pass already covers this).

**Row-poly on non-top-level functions (the earn-out).** The corpus includes the verified killer case: one program binds `let apply = fn(f, x) { f(x) }` and uses it at a **pure** row and at `{Log}`, both type-checking and running. If `apply` were not row-polymorphic the two uses would conflict; that they compose is the proof that row-polymorphism now reaches locally-bound higher-order functions.

## 4. Execution & dynamic scoping

No new runtime machinery: the CEK already dispatches a `perform` to the nearest enclosing `HandleK` on the continuation, regardless of whether the performing code sits in a top-level fn or a closure (verified — the relay probe runs). 4b-2 **locks the scoping semantics with a test**: a closure created in one dynamic context and *called* under a different handler performs against the handler enclosing its **call**, not its definition. This is the correct (dynamic, exception-like) semantics for row-typed effects and is the only choice consistent with how the row is threaded; the test prevents a future refactor from silently switching to lexical capture.

## 5. E0426 × closures

E0426 (the multi-shot cleanup lint for *observable* effects, effects spec §11) operates on the ambient effect row at a multi-shot handler. A closure that performs an observable effect pours that effect into the ambient exactly as inline code does, so the lint already applies. 4b-2 adds a test: a closure performing an observable effect (e.g. `IO`) under a `multi` handler triggers `E0426`; a one-shot handler over the same closure does not. If the test reveals the lint fails to see a closure-performed effect, that is a real gap to close within 4b-2 (not expected, given the effect flows through the same ambient).

## 6. The known limitation — relay plus own effect (Fork C)

The `close_unrelayed_residual` comment flags a function that *both* relays a parameter *and* performs its own concrete effect as possibly "more general than minimal." Lambdas make it trivially writable: `fn(f, x) { log(x)  f(x) }`. With Fork A applied, its tail *is* relayed through `f`, so it is left open and the row is `{Log | ρ}` — which is in fact the **minimal** description ("performs `Log`, plus whatever `f` does"). 4b-2's approach (approved): **activate, test, and document.** Write the relay-plus-own-effect program, pin its inferred row and its runtime behavior with a test, and record the result honestly in this section during implementation. Fix the primitive only if the pinned row proves genuinely wrong (unsound or misleading) rather than merely verbose. This converts a dormant caveat into a covered, documented property.

## 7. Diagnostics

**No new diagnostic code.** 4b-2 exercises the existing effect diagnostics unchanged: `E0420` (unhandled relayed effect), `E0421`/`E0423` (annotated-row mismatch, still only on top-level fns since lambda annotations are deferred), `E0426` (multi-shot observable-effect lint). The no-`%row`-token invariant (effects spec §9) continues to hold and is checked by the existing UI harness.

## 8. Pipeline & module changes

| Layer | Change |
|---|---|
| `lex` / `parse` / `resolve` / `ast` | none (no new surface). |
| `exhaust` | none. |
| `types` | **one change:** call `close_unrelayed_residual(lam_amb, &param_tys)` in the `Expr::Lambda` inference arm (§2). |
| `eval` | none (dynamic effect dispatch already handles closures). |
| `main` / `lib` | none. |
| tests | the effect-carrying-closure corpus (`tests/effect_closures.rs`, new), scheme pins in `tests/effect_types.rs`, an E0426 case, and the Fork-C pin. |

Layering (tests/arch/layering.rs) is unchanged — no new module.

## 9. Testing strategy

New file `tests/effect_closures.rs` (CEK-only, via `run_source`, mirroring `tests/effects_run.rs`), plus scheme pins in `tests/effect_types.rs`:

- **Precision (§2):** `make_logger` infers `fn() -> fn(String) / {Log} -> Unit` (closed, no `forall`); the relay scheme `forall a b c. fn(fn(a) / {c} -> b, a) / {c} -> b` is unchanged.
- **Relay execution (§3):** `apply(fn(n){ log(n) }, "hi")` under a resuming `Log` handler yields `hi!`; an unhandled relayed effect is `E0420`.
- **Row-poly earn-out (§3):** the let-bound `apply` used at a pure row and at `{Log}` in one program type-checks and runs.
- **Dynamic scoping (§4):** a closure performs against the handler enclosing its call, not its definition (output-verified).
- **E0426 × closures (§5):** a closure performing an observable effect under `multi` fires `E0426`; under one-shot it does not.
- **Fork C (§6):** relay-plus-own-effect program — inferred row pinned, runtime output verified, behavior documented.
- **Regression:** full suite (145) stays green; the effect-free `cek == tree` corpus is untouched.

## 10. Build order (tasks — a plan per this sub-slice)

Sequenced so the precision fix and its guard land first, then coverage widens outward:

1. **Fork A — close lambda residuals.** The one-line-ish `types` change; pin `make_logger` (closed) and the unchanged `relay` scheme; full suite green.
2. **Relay execution + the row-poly earn-out.** `tests/effect_closures.rs`: effectful relay yields `hi!`; unhandled → `E0420`; the let-bound `apply` used at two rows.
3. **Dynamic scoping + E0426 × closures.** The call-site-scoping test; the multi-shot observable-effect test.
4. **Fork C + exit gate.** Relay-plus-own-effect pinned + documented; full-suite exit gate; commit + push.

### Exit criterion

Concrete-effect lambdas infer minimal closed rows (`make_logger` pinned) while relay lambdas stay row-polymorphic (`relay` pinned); effect-carrying closures relayed through HOFs run on the CEK with verified output and discharge correctly (`E0420` when unhandled); a let-bound HOF lambda is row-polymorphic across two distinct rows; dynamic call-site scoping and E0426 × closures are locked by tests; the relay-plus-own-effect row is pinned and documented; the full suite is green; `cargo fmt --all` + `sh scripts/check.sh` clean.

## 11. Deferred / Honestly-Flagged

- **Lambda effect-row annotations** (`fn(x) / {E} { … }`) + exact-row checking (Fork B) — deferred; parser + `check_exact_row` wiring, separable.
- **Row-polymorphic combinator library + the effect-row value-restriction stress test** → 4b-3 (the soundness capstone toward linear/affine).
- **Deep multi-shot × closure resource safety** — a closure captured into a multiply-resuming continuation; the real guarantee is the linear/affine arc, not 4b-2.
- **The `close_unrelayed_residual` "known limitation"** — if the Fork-C pin shows the row is acceptable (minimal), the limitation is downgraded from "caveat" to "documented behavior"; a genuine fix stays deferred unless the pin proves it wrong.

## 12. Milestone Checklist (Slice 4b-2)

- [ ] `Expr::Lambda` inference calls `close_unrelayed_residual`; `make_logger` = `fn() -> fn(String) / {Log} -> Unit`; `relay` row-poly scheme unchanged.
- [ ] Effect-carrying closure relayed through a HOF runs on the CEK with verified output; unhandled relay is `E0420`.
- [ ] Let-bound HOF lambda is row-polymorphic across a pure and an effectful use in one program.
- [ ] Closure performs against the call-site handler (dynamic scoping) — output-verified.
- [ ] E0426 fires for a closure performing an observable effect under `multi`; not under one-shot.
- [ ] Relay-plus-own-effect row pinned and documented (Fork C).
- [ ] Full suite green; `cargo fmt --all` + `sh scripts/check.sh` clean; committed + pushed.
