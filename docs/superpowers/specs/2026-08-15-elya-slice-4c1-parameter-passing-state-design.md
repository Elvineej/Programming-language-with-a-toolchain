# Elya — Slice 4c-1 Design Specification: Parameter-Passing State

- **Codename:** Elya
- **Slice:** 4c-1 — the first sub-slice of the generic-effects/State arc (Slice 4c), which follows the closed closure arc (Slice 4b)
- **Status:** Design draft — awaiting review before an implementation plan is written
- **Date:** 2026-08-15
- **Depends on:** Slice 4b (complete, `48a08cf`) — lambdas/closures, effect-carrying closures & relay, the value restriction with row-dimension teeth; Slice 3 — algebraic effects with row-polymorphic inference, deep handlers, one-shot/multi-shot `resume`, and measured effect-TCE (`K_MAX_EFF = 4`).
- **Companion documents:** the language design spec (`.../2026-08-05-elya-language-design.md`, "design spec"); the Slice 3 effects spec (`.../2026-08-06-elya-slice-3-effects.md`, "effects spec"); the 4b closure specs.

---

## 0. How to read this document

This is a **design spec** for Slice 4c-1 — **parameter-passing `State`**: a real, user-written `State { get / set }` effect whose deep handler threads state *through* `resume` in the classic state-passing style (each clause returns a function of the state), now expressible because lambdas exist (Slice 4b). It is the first of two sub-slices in the generic-effects/State arc; **4c-2** (generic/parametric effects, `effect State(s)`) follows and is out of scope here (§8).

**This is a coverage-and-obligation-discharge slice, verified to need no production code.** During brainstorming both load-bearing facts were checked against the *current* compiler:

- **It works.** A parameter-passing `State` handler — clauses `State.get() -> fn(s){ (resume(s))(s) }`, `State.set(v) -> fn(s){ (resume(Unit))(v) }`, `return(x) -> fn(s){ x }`, applied to an initial state — type-checks and runs: `set("a"); let x = get(); set(x <> "b"); get()` under it yields `ab`.
- **It is bounded.** A tail-recursive driver performing N `State` operations under this handler has a **constant** peak continuation depth of **5** across N = 10, 100, 1000 — even though the state-passing clause calls `resume` in non-tail position `(resume(s))(s)`. The tail-resume splice (effects spec §6) plus the tail-recursive driver keep it bounded; the extra `+1` over `K_MAX_EFF = 4` is the single state-passing function-application frame.

So 4c-1 **discharges the last open Slice-3 §11 obligation** — the "full parameter-passing `State` tail-loop test (with lambdas)" — and demonstrates the manifesto's claim (design spec §1: "mutable state is the `State` effect") *concretely*, with **no new syntax, no new diagnostic, no new runtime machinery, and (expected) no production change.**

**Honesty note.** `State` here is **monomorphic** — a concrete state type per handler (the demonstration uses `String`). Making one `State(s)` work at *any* state type is the genuine type-system extension, deliberately held for 4c-2 (§8). Monomorphic-first mirrors how 4a shipped nullary ADTs before parametric ones.

Section refs: "effects spec §X" → the Slice 3 spec; "design spec §X" → the language design spec; bare "§X" → this document.

---

## 1. Scope

### 1.1 What Slice 4c-1 delivers

1. **A real parameter-passing `State` effect (§2)** — `effect State { fn get() -> String  fn set(v: String) -> Unit }` with a state-passing deep handler, run on the CEK, output-verified (`ab`). Demonstrates that `State` is an ordinary user handler over the one effect mechanism, not a language primitive.
2. **The §11 State tail-loop TCE obligation, discharged (§3)** — a tail-recursive driver performing many `State` operations is **bounded** (constant peak depth `K_MAX_STATE = 5`, measured), with a **non-tail grow control** proving the bound has teeth. This is the parameter-passing counterpart to 3e's tail-resumptive TCE tests, owed "when lambdas land."
3. **The generic-effects/State arc opened** — 4c-1 establishes the stateful surface that 4c-2 (generic `State(s)`) generalizes and that the frontier linear/affine arc will later make resource-safe.

### 1.2 Surface additions

**None.** No new syntax — `effect`/`handle`/`with`/`resume`, lambdas, and `<>` are all already in the language. The whole slice is output-verified coverage over the existing surface.

### 1.3 What Slice 4c-1 does NOT do (deferred — §8)

- **Generic/parametric effects** (`effect State(s)`, operations polymorphic over the state type) — the real type-system extension → 4c-2.
- **`State` composed with other effects** in one handler stack (e.g. `State` + `Log`) — beyond the single-effect demonstration; a natural later addition.
- **A `State` standard-library module / sugar** — 4c-1 writes the handler inline to *prove* the mechanism; packaging is a stdlib concern (design-spec roadmap).
- **Mutable-cell / reference semantics or linear resource safety** — the frontier linear/affine arc; 4c-1's row teeth (from 4b-3) guard that future.

---

## 2. Parameter-passing `State` — the encoding

`State` is declared like any effect and interpreted by a **state-passing deep handler**: every clause and the `return` clause evaluate to a *function of the current state*, and the whole `handle` expression is therefore a function `fn(State) -> Result` applied to the initial state.

```elya
effect State {
  fn get() -> String
  fn set(v: String) -> Unit
}

fn run() {
  let _ = set("a")
  let x = get()
  let _ = set(x <> "b")
  get()
}

pub fn main() {
  let program = handle { run() } with {
    State.get()  -> fn(s) { (resume(s))(s) }       // pass current state to resume AND thread it on
    State.set(v) -> fn(s) { (resume(Unit))(v) }     // thread the new state v
    return(x)    -> fn(s) { x }                      // discard final state, yield the result
  }
  io.println(program("init"))                         // apply to the initial state
}
```

**How it threads (verified output `ab`):** each `perform` captures the continuation up to the handler; the clause returns `fn(s){ … }`, so `resume(v)` yields "the rest of the computation as a state function", which the clause applies to the (possibly updated) state. `set("a")` threads `"a"`; `get()` reads it back into `x`; `set(x <> "b")` threads `"ab"`; the final `get()` yields `"ab"`. This is the standard deep-handler state monad, made concrete — and it is exactly the manifesto's "state is the `State` effect" delivered as a library handler over `resume`, not a language feature.

**No new machinery.** This composes existing pieces: effect declarations (Slice 3), `resume` returning an arbitrary result type (Slice 3 — here a function type), and lambdas (Slice 4b). Verified to type-check and run on the current compiler.

## 3. TCE — the parameter-passing State tail-loop (the §11 obligation)

Effects spec §11 recorded a **tracked obligation**: a real state-threading `State` handler needs lambdas, so its tail-loop TCE test was deferred "when lambdas land"; 3e's tail-resumptive tests used the *expressible* constant-resuming form instead. Lambdas have landed, and 4c-1 discharges it — measured, two-sided, in the discipline of every prior TCE claim.

- **Bounded side.** A tail-recursive driver `fn loop(n) { if n == 0 { get() } else { let _ = set("x")  loop(n - 1) } }` run under the state-passing handler at a large N peaks at a **constant** continuation depth `K_MAX_STATE = 5`, invariant across N (measured 5 at N = 10, 100, 1000). Output-verified (the program completes and yields the final state). The bound is pinned from measurement; it is one more than `K_MAX_EFF = 4` because the state-passing clause applies the resumed function `(resume(…))(…)`, a single extra frame — *not* a per-operation leak.
- **Grow-control side.** A **non-tail** variant (the recursive `loop` call placed under an operation, e.g. `set` then `x <> loop(...)`) makes the peak **grow with N**, proving the bounded side is a real property of the tail-recursive driver, not an accident of the machine.

"Fix the machine, don't raise the constant" holds: if the bounded loop's peak is not constant in N, a state-passing splice is leaking a frame per operation — that is the bug, not the ceiling.

## 4. Diagnostics

**No new diagnostic code.** The demonstration and the tail-loop assert exact program output and a measured depth; nothing about `State` introduces a new failure mode (an unhandled `State` is the existing `E0420`, a misuse is the existing type/row diagnostics). The no-`%row`-token invariant continues to hold.

## 5. Pipeline & module changes

**Expected: none.** Everything is verified to work on the current compiler, so 4c-1 is expected to add only tests. If — contrary to the probes — a test surfaces a gap (e.g. the state-passing handler fails to type-check in some shape, or the tail-loop is not bounded), *that* is the real, high-value work; it is not expected. Layering (tests/arch/layering.rs) unchanged; no new module.

## 6. Testing strategy

New file `tests/state_effect.rs` (CEK-only, via `run_source` / the `run_effect` depth harness), fully output-verified per the effects-corpus discipline (no `cek == tree` oracle behind effect evaluation):

- **Demonstration (§2):** the `set/get` threading program yields exactly `ab`.
- **TCE bounded (§3):** the `State` tail-loop at a large N completes with the right output **and** peaks at `K_MAX_STATE = 5`, with peak **constant across two large N** (the real bound).
- **TCE grow control (§3):** a non-tail `State` loop's peak **grows with N**.
- **Regression:** the full prior suite stays green; the effect-free `cek == tree` corpus and `K_MAX_EFF` tests are untouched.

## 7. Build order (tasks — a plan per this sub-slice)

Two tasks; each ends output-verified.

1. **The `State` handler demonstration.** `tests/state_effect.rs`: the parameter-passing `State` program yields `ab`. Full suite green.
2. **The State tail-loop TCE + exit gate.** Add the bounded loop (constant `K_MAX_STATE = 5`, measured-then-pinned, plus the constant-across-N assertion) and the non-tail grow control; run the full Slice-4c-1 exit gate; commit + push. This discharges the effects-spec §11 obligation.

### Exit criterion

A parameter-passing `State` handler runs on the CEK with verified output (`ab`); the `State` tail-loop is bounded (`K_MAX_STATE = 5`, constant in N) with a non-tail grow control; the full suite is green; `cargo fmt --all` + `sh scripts/check.sh` clean. The last open Slice-3 §11 `State`-TCE obligation is discharged.

## 8. Risks & mitigations

- **The measured constant (5) drifts under a different loop shape.** *Mitigation:* the bounded test asserts **constant-across-N** (the real property) alongside the pinned `5`; a shape that peaks higher-but-still-constant is pinned to its measured value with the same invariant, and a shape that *grows* is the grow control catching a leak.
- **"Coverage with no production code" hides a gap.** *Mitigation:* the probes ran green; if a test nonetheless fails, the output-verified assertions make the gap concrete and it becomes the slice's real work, not a silent pass.
- **Over-claiming generality.** *Mitigation:* §0's honesty note states `State` is monomorphic here; the generic `State(s)` claim is explicitly 4c-2.

## 9. Deferred / Honestly-Flagged

- **Generic/parametric effects** (`effect State(s)`, polymorphic operation signatures) → 4c-2, the type-system extension of this arc.
- **`State` + other effects composed**, and a `State` stdlib module → later.
- **Mutable-cell / reference semantics and linear resource safety** → the frontier linear/affine arc, where 4b-3's row teeth and the deferred relay-plus-own-effect row-leak fix also live.

## 10. Milestone Checklist (Slice 4c-1)

- [ ] Parameter-passing `State` handler runs on the CEK → `ab`.
- [ ] `State` tail-loop bounded: constant peak `K_MAX_STATE = 5` across two large N, output-verified complete.
- [ ] Non-tail `State` loop grow control: peak grows with N.
- [ ] Full suite green; `cargo fmt --all` + `sh scripts/check.sh` clean; committed + pushed. Slice-3 §11 State-TCE obligation discharged.
