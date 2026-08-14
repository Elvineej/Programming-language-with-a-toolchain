# Elya — Slice 4b-3 Design Specification: Row-Polymorphic Combinators & the Value-Restriction Row Teeth

- **Codename:** Elya
- **Slice:** 4b-3 — the third and final closure sub-slice (Slice 4b, "lambdas/closures"); the soundness capstone of the closure arc
- **Status:** Design draft — awaiting review before an implementation plan is written
- **Date:** 2026-08-15
- **Depends on:** Slice 4b-2 (complete, `811f460`) — effect-carrying closures, higher-order relay, `close_unrelayed_residual` on lambdas; Slice 4b-1 — the value restriction (`is_syntactic_value` gating let-generalization); Slice 3 — algebraic effects with row-polymorphic inference and the discharge pass.
- **Companion documents:** the language design spec (`.../2026-08-05-elya-language-design.md`, "design spec"); the Slice 3 effects spec (`.../2026-08-06-elya-slice-3-effects.md`, "effects spec"); the 4b-1 closures spec (`.../2026-08-13-elya-slice-4b1-closures-design.md`); the 4b-2 effect-closures spec (`.../2026-08-14-elya-slice-4b2-effect-closures-design.md`).

---

## 0. How to read this document

This is a **design spec** for Slice 4b-3 — the **row-dimension capstone** of the closure arc. It proves two things with an output-verified corpus and closes Slice 4b:

1. **The value restriction has teeth on the effect-row dimension** — a value-restricted *non-value* does not over-generalize its effect row (two-sided teeth, §2).
2. **Row polymorphism works for realistic recursive combinators** — `map`/`fold` relaying an effectful callback (§3).

**Like 4b-2, this is a coverage-and-capstone slice — verified to need no production code.** During brainstorming every case was checked against the *current* compiler:

- **Positive teeth:** `let g = fn(thunk){ thunk() }` (a value) is row-polymorphic — used at a pure row *and* `{Log}` in one program, it type-checks and runs `hi!`.
- **Negative teeth:** the *matched pair* `let g = make_relay()` (identical program, RHS a **call** not a lambda) fails with **E0423** at the second use — the row does not generalize.
- **Combinators:** `map` relaying `fn(s){log(s)}` yields `a!b!c!`; `fold` relaying `fn(acc,s){acc <> log(s)}` yields `a.b.c.`.

So 4b-3 introduces **no new syntax, no new diagnostic, no new runtime machinery, and (expected) no production change** — it is deliberate, output-verified coverage that locks the row-dimension soundness discipline before the frontier work that will *depend* on it.

**The honesty flag (front and center).** The negative teeth are a **regression-guard**, not an *exhibited* row unsoundness. The easiest non-value-with-a-free-row-var to construct — the relay — is itself **sound to generalize** (a relay performs exactly its argument's effects; every row instantiation is fine), so the value restriction is deliberately *conservative* on the row dimension today. Its teeth matter **forward**: when effect-based mutable state / first-class resumable references arrive (the frontier linear/affine + `State` work), generalizing a non-value's row *would* be unsound, and this corpus ensures the gate cannot silently regress before then. This is the same "introduce the discipline before it is strictly needed" logic that justified the value restriction in 4b-1 (the type dimension) — 4b-3 does it for the row dimension.

Section refs: "4b-1 spec §X" / "4b-2 spec §X" → those specs; bare "§X" → this document.

---

## 1. Scope

### 1.1 What Slice 4b-3 delivers

1. **The two-sided value-restriction row teeth (§2)** — a matched pair isolating the effect-row dimension:
   - **Positive:** a value binding (`let g = fn(thunk){ thunk() }`) is row-polymorphic — usable at a pure row and at `{Log}` — output-verified (`hi!`).
   - **Negative (the guard):** the structurally identical non-value binding (`let g = make_relay()`) keeps its row **monomorphic**; a second use at a different row is **E0423**. Because both uses agree in *type* and differ only in *effect row*, the E0423 (a row error, not E0400) proves it is the row dimension being held.
2. **Row-polymorphic combinators (§3)** — `map` and `fold` relaying an effectful callback through recursion, output-verified (`a!b!c!`, `a.b.c.`).
3. **The closure arc closed** — with 4b-3 green, Slice 4b (lambdas/closures) is complete: basic closures (4b-1), effect-carrying closures & relay (4b-2), and the row capstone (4b-3).

### 1.2 Surface additions

**None.** No new syntax. The whole slice is output-verified coverage over the existing surface (lambdas, effects, `handle`/`with`, `match`, ADTs).

### 1.3 What Slice 4b-3 does NOT do (deferred — §9)

- **The relay-plus-own-effect row-leak fix** (the 4b-2 tracked obligation) — its own focused effect-inference slice, not 4b-3.
- **Constructing a genuine row unsoundness** (ref-via-effect / stateful handler) — deliberately out of scope (§0 honesty flag); the negative teeth are a forward-looking regression-guard, and manufacturing an unsoundness is a rabbit hole for a capstone.
- **Generic effects / parameter-passing `State`** — the next arc after the closure arc closes.
- **The frontier linear/affine work** — where the row-dimension soundness this slice guards actually becomes load-bearing.

---

## 2. The value-restriction row teeth (two-sided)

The value restriction ([types.rs](../../../src/types.rs), the `Stmt::Let` arm) already gates *both* type- and row-variable generalization on `is_syntactic_value`: a non-value binding gets `Scheme { vars: [], row_vars: [], ty }`. 4b-1 proved the **type** dimension (`value_restriction_blocks_nonvalue_generalization`). 4b-3 proves the **row** dimension with a matched pair that differs in exactly one thing — whether the RHS bound to `g` is a syntactic value.

**Positive (value → row generalized → row-polymorphic).** A `let`-bound lambda is a value, so its free row variable is quantified:

```elya
let g = fn(thunk) { thunk() }              // value: g : ∀a e. fn(fn()/{e}->a)/{e}->a
let a = g(fn() { "pure" })                  // e ↦ pure
let r = handle { g(fn() { log("hi") }) }    // e ↦ {Log}
        with { Log.log(m) -> resume(m <> "!")  return(x) -> x }
```

Both uses type-check (each instantiates `e` fresh); the program runs and prints `hi!`.

**Negative (non-value → row NOT generalized → monomorphic).** Bind the *same* relay to `g`, but through a call so the RHS is a non-value:

```elya
fn make_relay() { fn(thunk) { thunk() } }
…
let g = make_relay()                        // non-value: g : fn(fn()/{e0}->a0)/{e0}->a0, e0 monomorphic
let a = g(fn() { "pure" })                  // fixes e0 = pure
let r = handle { g(fn() { log("hi") }) } …  // needs e0 = {Log}  →  E0423 (row mismatch)
```

The only difference from the positive program is `let g = make_relay()` vs `let g = fn(thunk){ thunk() }`. **Row-dimension isolation:** both callback thunks return `String`, so `g`'s *result* type unifies cleanly (no E0400); the sole conflict is the effect row, so the diagnostic is **E0423 "effect row mismatch"** — the proof that the gate is holding the row, not merely the type.

This is the row-dimension analogue of 4b-1's type-dimension control, and it is the "bounded assertion + must-not-generalize negative control" pair: the value generalizes (positive, runs), the non-value must not (negative, E0423).

## 3. Row-polymorphic combinators

Two canonical recursive combinators, each relaying an effectful callback (verified end-to-end under a handler):

- **`map` (transform-relay):** `fn map(xs, f) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }`. With `f = fn(s){ log(s) }` under a `Log` handler resuming `m <> "!"`, `map(["a","b","c"], f)` yields the list `["a!","b!","c!"]`, concatenated to `a!b!c!`.
- **`fold` (accumulate-relay):** `fn fold(xs, acc, f) { match xs { Nil -> acc  Cons(h, t) -> fold(t, f(acc, h), f) } }`. With `f = fn(acc,s){ acc <> log(s) }` under a `Log` handler resuming `m <> "."`, `fold(["a","b","c"], "", f)` yields `a.b.c.`.

These prove row polymorphism composes through recursion: the combinator's own row stays polymorphic (relayed from `f`), the effect threads to the call-site handler, and the discharge is correct. They are the realistic payoff of the relay path that 4b-2 activated.

## 4. Diagnostics

**No new diagnostic code.** The negative teeth assert the *existing* `E0423` (effect-row mismatch, effects spec §9); the positive teeth and combinators assert exact program output. The no-`%row`-token invariant continues to hold (existing UI harness).

## 5. Pipeline & module changes

**Expected: none.** Every case is verified to work on the current compiler, so 4b-3 is expected to add only tests. If — contrary to the brainstorm probes — a test surfaces a gap (e.g. the negative control fails to error, meaning the gate leaks on the row dimension), *that* is the real work and would be a genuine, high-value finding; it is not expected. Layering (tests/arch/layering.rs) is unchanged; no new module.

## 6. Testing strategy

New file `tests/row_polymorphism.rs` (CEK-only for the effectful runs, via `run_source`; `check_source` for the negative control), fully output-verified per the 4b-2 discipline (no `cek == tree` oracle behind effect evaluation, so every case pins a concrete value):

- **Positive teeth (§2):** the value-bound relay used at a pure row and `{Log}` runs `hi!`.
- **Negative teeth (§2):** the matched non-value-bound relay is rejected with **E0423** at the second (differing-row) use; the assertion checks the code is `E0423` (a row error), documenting the row-dimension isolation in a comment.
- **`map` (§3):** effectful `map` yields `a!b!c!`.
- **`fold` (§3):** effectful `fold` yields `a.b.c.`.
- **Regression:** the full prior suite stays green; the effect-free `cek == tree` corpus is untouched.

## 7. Build order (tasks — a plan per this sub-slice)

Two tasks; each ends output-verified.

1. **The value-restriction row teeth.** `tests/row_polymorphism.rs`: the positive (value → `hi!`) and negative (non-value → `E0423`) matched pair, with the row-dimension-isolation comment. Full suite green.
2. **Row-poly combinators + exit gate.** Add `map` (`a!b!c!`) and `fold` (`a.b.c.`); run the full Slice-4b-3 exit gate; commit + push. With this green, Slice 4b is complete.

### Exit criterion

The value restriction's row teeth are pinned two-sided (value row-poly runs `hi!`; matched non-value is `E0423`, row-isolated); `map` and `fold` relay an effectful callback with verified output (`a!b!c!`, `a.b.c.`); the full suite is green; `cargo fmt --all` + `sh scripts/check.sh` clean. Slice 4b (the closure arc) is closed.

## 8. Risks & mitigations

- **The "coverage slice with no production code" turns out to hide a gap.** *Mitigation:* the probes already ran green end-to-end; if a test nonetheless fails, the output-verified assertions make the gap concrete and it becomes the slice's real (valuable) work rather than a silent pass.
- **The negative control passes for the wrong reason** (e.g. E0400 on a type var instead of E0423 on the row). *Mitigation:* the matched pair returns `String` from both thunks so the type unifies; the assertion specifically requires **E0423**, so a type-dimension error would fail the test and be caught.
- **Over-claiming soundness.** *Mitigation:* §0's honesty flag states plainly that the teeth are a forward-looking regression-guard, not an exhibited unsoundness — the relay is sound to generalize, and the guard protects future soundness when effect-state/refs arrive.

## 9. Deferred / Honestly-Flagged

- **Relay-plus-own-effect row-leak fix** — the 4b-2 tracked obligation; its own effect-inference slice.
- **Exhibiting a genuine row unsoundness** (ref-via-effect) — out of scope by design; the negative teeth are a regression-guard.
- **Generic effects / parameter-passing `State`** — the next arc.
- **Linear/affine types on the effect system** — the frontier, where the row-dimension soundness this slice guards becomes load-bearing.

## 10. Milestone Checklist (Slice 4b-3)

- [ ] Positive row teeth: value-bound relay used at a pure row and `{Log}` runs `hi!`.
- [ ] Negative row teeth: matched non-value-bound relay is `E0423` at the differing-row use (row-isolated: both thunks return `String`).
- [ ] `map` relays an effectful callback → `a!b!c!`.
- [ ] `fold` relays an effectful callback → `a.b.c.`.
- [ ] Full suite green; `cargo fmt --all` + `sh scripts/check.sh` clean; committed + pushed. **Slice 4b closed.**
