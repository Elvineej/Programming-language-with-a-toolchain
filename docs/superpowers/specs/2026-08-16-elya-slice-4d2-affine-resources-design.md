# Elya — Slice 4d-2 Design Specification: Affine Resources & the Multi-Shot Capture Guarantee

- **Codename:** Elya
- **Slice:** 4d-2 — the second piece of the frontier linear/affine arc (Slice 4d); **the payoff** — the guarantee that makes the E0426 hazard unrepresentable, scoped honestly.
- **Status:** Design draft — awaiting review before an implementation plan is written
- **Date:** 2026-08-16
- **Depends on:** Slice 4d-1 (complete, `8a90cfc`) — effect resumption discipline (`effect multi`, `Infer.effect_multi` lookup, E0427); Slice 4a (ADT declarations — the surface `linear` extends); Slice 3 (effects, `handle`/`resume`).
- **Companion documents:** the language design spec (`.../2026-08-05-elya-language-design.md`, "design spec"; esp. §2 memory axis, §8.6, §13); the 4d-1 resumption-discipline spec.

---

## 0. How to read this document

This is a **design spec** for Slice 4d-2 — **affine resources**: a `linear type File { … }` declares a type whose values must be used **at most once** and may **not be captured into a multi-shot continuation** (used across a perform of a `multi`-declared effect). This is the frontier's payoff: the E0426 double-free hazard, promoted from a best-effort lint to a **type-checked guarantee** — for the cases the first cut covers.

**The scoped-guarantee honesty line (load-bearing, the 4d-1 discipline).** The first cut is **intra-function local** with a **restricted create-and-consume** usage model, chosen (brainstormed 2026-08-16) to be sound-but-strict while dodging the two hard parts of substructural typing rather than solving them. So the guarantee is **scoped, not universal**, and the spec says so everywhere it matters:

> An affine resource **created and consumed within a function** is guaranteed use-at-most-once and never captured across a `multi`-perform. **Two things are explicitly deferred as tracked obligations, not silent gaps:** (a) an affine value passed to a callee that internally *duplicates* its parameter is a **soundness** gap the local check misses (the linearity-polymorphism edge — obligation `affine-callee-duplication-obligation`); (b) the whole-body, branch-insensitive, intra-function liveness rule rejects some **valid** programs (a completeness/false-positive gap — obligation `affine-intraprocedural-overapprox`). Both tighten when we go inter-procedural. **The exit criteria scope the guarantee and name both edges; they never claim a blanket no-double-use guarantee.**

Same shape as 4b-1's value restriction and 4b-3's row teeth: *reject the ambiguous case first, relax/tighten later.*

Section refs: "design spec §X" → the language spec; "4d-1 spec §X" → the resumption-discipline spec; bare "§X" → this document.

---

## 1. Scope

### 1.1 What Slice 4d-2 delivers

1. **A `linear` type modifier** — `linear type File { … }`: the type's values are **affine** (use at most once). A binding is affine iff its inferred type's head is a linear-declared type (§2).
2. **The inference exposure** — `infer` reports which let-bindings and parameters have a linear type (a side-table), the one bounded reach into inference (§3).
3. **The `affine::check` pass** — a new self-contained analysis pass (shaped like `exhaust.rs`) that, per function body, enforces (a) **≤ 1 use** of each affine binding (**E0428**) and (b) **no use of an affine binding across a `multi`-perform** (**E0429**), consuming the §3 side-table + 4d-1's `effect_multi` (§4).
4. **Two new diagnostics** — **E0428** (affine value used more than once), **E0429** (affine value captured into a multi-shot continuation) (§5).

### 1.2 Surface additions

One modifier on a type declaration:

```elya
linear type File { Handle(Int) }     // values are affine (use at most once)
```

`linear` is a new keyword (`KwLinear`). Everything else is unchanged.

### 1.3 What Slice 4d-2 does NOT do (deferred — §11)

- **Inter-procedural affine tracking** and **linearity/multiplicity polymorphism** — the two tracked obligations (§6); the first cut is intra-function local with create-and-consume.
- **Full linear / "must use exactly once" (no-leak)** — 4d-2 is *affine* (at most once); requiring consumption (leak-freedom) is a stronger later refinement.
- **Handler-precise / branch-sensitive liveness** — the first cut over-approximates (§6b).
- **Generalizing E0426's *observable-effect* lint** — E0426 stays as-is for observable effects; 4d-2 adds the *resource* guarantee alongside it (they are different axes: observability vs affine-resource-safety).
- **Any change to the runtime or the GC** — affine is a static use-discipline; `eval.rs` and the memory model are untouched (§0, §7).

## 2. The `linear` type modifier and what "affine" means

`TypeDecl` gains `is_linear: bool`; the parser accepts `linear` before `type` (new `KwLinear` token). A **linear type's values are affine**: each *binding* whose inferred type's head is a linear-declared type may be **referenced at most once**, and may not be **live across a `multi`-perform**.

- **"Reference" = use.** Each occurrence of an affine binding's name (as an argument, in a `match` scrutinee, returned, etc.) is one use. Zero uses is allowed (affine permits dropping; leak-freedom is deferred). Two uses is **E0428**.
- **Affine is a property of the *binding*, determined by its *type*.** `let f = Handle(3)` and a parameter `f` whose type is `File` are both affine. This is why the check needs inferred types (§3).
- **Values flow, the type carries linearity.** A `match` that binds a field of linear type produces an affine binding; a function returning a linear type produces an affine value at its call site.

## 3. The inference exposure (the one bounded reach)

`infer` today is `infer(module) -> Vec<Diagnostic>` and persists no per-node types. The affine pass needs to know **which bindings are linear-typed**, which only inference can determine (`let f = open()` is affine only if `open`'s return type is linear). So:

- Collect the set of **linear type names** from the declarations (trivial: `Decl::Type` with `is_linear`).
- During inference, for each **let-binding and each parameter**, after its type is known, record whether its resolved type's head is a linear type — into a **`LinearBindings` side-table keyed by the binding's `Span`** (a `HashSet<Span>` of affine binding sites).
- Expose it: `infer` (or a sibling `infer_with_linearity(module) -> (Vec<Diagnostic>, HashSet<Span>)`) returns the side-table; `lib.rs` threads it to `affine::check`.

This is the whole reach into inference: a read-only side-table of affine binding sites. **No `unify_row`, no substructural context, no change to the HM algorithm** — inference already resolves these types; it now also records which are linear. The affine *logic* lives entirely in the new pass (§4).

## 4. The `affine::check` pass

A new module `src/affine.rs`, `pub fn check(module: &Module, affine_sites: &HashSet<Span>) -> Vec<Diagnostic>`, sequenced in `lib.rs` after `infer` (and gated the same way — run only on an error-free front end). It walks **each function/lambda body** independently (intra-function) with a small forward state per body:

- A map from each in-scope **affine binding** (name → its defining `Span`, present iff the span is in `affine_sites`) to a **use count** and a **crossed-a-multi-perform** flag.
- **On a `let`/param binding of an affine site:** register it (count 0, not crossed).
- **On a use** (an `Expr::Var` referencing an affine binding): increment its count; if the count reaches 2, emit **E0428** at the use. If the binding is *crossed*, emit **E0429** at the use.
- **On a perform of a `multi` effect** (an operation call whose effect `self`/`effect_multi` marks `multi` — reuse the 4d-1 lookup, exposed to the pass): mark **every currently in-scope affine binding** as *crossed*. (Over-approximation: any affine binding created before this point and used after it is treated as captured — §6b.)
- **Scope discipline:** affine bindings leave scope at block/match-arm/lambda-body end, matching the resolver's scoping.

The walk mirrors `exhaust.rs`'s `walk_expr`/`walk_block` structure (evaluation-order traversal of `Expr`/`Block`), with the affine-state threaded. Effect-multi-ness is read from a table the pass is given (built from the declarations exactly as 4d-1 built `Infer.effect_multi`).

**Worked example (the guarantee):**

```elya
effect multi Flip { fn flip() -> Bool }
linear type Tok { Tok }
fn use_tok(t) { match t { Tok -> "used" } }
pub fn main() {
  let t = Tok
  io.println(handle {
    let _ = flip()      // multi-perform -> t is now "crossed"
    use_tok(t)          // use of a crossed affine binding -> E0429
  } with multi { Flip.flip() -> resume(True) })
}
```

## 5. Diagnostics

Two new codes, held to the E042x named-label, no-`%row` discipline:

- **E0428** — *"affine value `t` used more than once"*, labelling the second use, help: *"`t` has an affine type (`Tok`); an affine value may be used at most once."*
- **E0429** — *"affine value `t` may be captured by a multi-shot handler"*, labelling the use, help: *"`t` is used after a perform of a `multi` effect; a multi-shot resume would use it more than once. Consume `t` before the perform, or don't hold it across it."*

E0420–E0427, E0430–E0433 unchanged. E0426 is untouched (§1.3).

## 6. The scoped guarantee and the two tracked obligations

**What is guaranteed (first cut):** within a function body, an affine binding is used at most once (E0428) and is never used across a `multi`-perform (E0429). For a resource **created and consumed in one function under its handler**, this is exactly the no-double-free / no-multi-capture guarantee.

**(a) Soundness obligation — callee duplication (`affine-callee-duplication-obligation`).** Passing an affine value to a function counts as one use; the checker does **not** recurse. A callee that duplicates its parameter (`fn use2(x){ f(x)  g(x) }`) therefore duplicates an affine argument at runtime — a real double-use the local check **misses**. Closing it needs a param multiplicity annotation or inter-procedural analysis (the substructural machinery the targeted scope avoided). **Recorded, not silent; the guarantee is scoped to exclude it.**

**(b) Completeness obligation — over-approximation (`affine-intraprocedural-overapprox`).** The liveness rule is whole-function-body, branch-insensitive, and intra-function, so it **rejects some valid programs** (a use after a `multi`-perform that an intervening handler actually delimits out of the continuation; uses in mutually-exclusive branches; cross-function threading). Sound (never wrongly accepts) but strict. Relax as the analysis sharpens (handler-precise, branch-sensitive, inter-procedural).

Both are the same *reject-ambiguous-first* discipline as the value restriction and the row teeth. **The exit criteria (§9) scope the guarantee and name both; they must not claim a universal guarantee.**

## 7. Pipeline & module changes

| Layer | Change |
|---|---|
| `lex` | add `KwLinear` (`#[token("linear")]`). |
| `ast` | `TypeDecl.is_linear: bool`. |
| `parse` | accept `linear` before `type`. |
| `types` | record affine binding sites (let-bindings + params whose type head is linear) into a `HashSet<Span>`; expose it from `infer` (§3). No `unify_row`/HM change. |
| `affine` (new) | `affine.rs`: `pub fn check(module, affine_sites) -> Vec<Diagnostic>` — the per-body use-count + multi-cross walk (§4). |
| `lib` | sequence `affine::check` after `infer` (error-free front end only). |
| `eval` | **none** — affine is static; runtime and GC untouched. |
| tests | positive/negative fixtures (§8). |

Layering (`tests/arch/layering.rs`): `affine` is a new pass module; add it at the analysis layer (same tier as `exhaust`, layer 5) and to the reference-scan list.

## 8. Testing strategy

- **Guarantee — positive:** a linear `Tok` created, used once, consumed → type-checks and runs.
- **E0428:** an affine binding used twice → E0428 (UI fixture + a direct test).
- **E0429 (the payoff):** an affine binding used after a `multi`-perform (the §4 example) → E0429 (UI fixture). And the *one-shot* counterpart — the same program over a **one-shot** effect — is **accepted** (a one-shot continuation resumes at most once, so no capture hazard): the two-sided teeth, proving E0429 keys on `multi`-ness, not on "any perform."
- **Monomorphic/regression:** the entire prior suite is green; non-linear types are entirely unaffected (a binding not in `affine_sites` is never checked).
- **Scoped-guarantee honesty (documented, not a bug):** a test or comment records that a callee-duplication program (`use2(tok)`) is **currently accepted** (the tracked soundness gap `affine-callee-duplication-obligation`) — pinned so a future inter-procedural tightening flips it visibly, exactly as the 4c-2 known-limitation tests pin the row leak.

## 9. Build order (tasks — a plan per this sub-slice)

1. **`linear` marker + affine-site inference exposure.** `KwLinear`, `TypeDecl.is_linear`, parse; `infer` records affine binding sites (`HashSet<Span>`) and exposes them. A test that `infer_with_linearity` flags a linear-typed `let` and not a plain one. (No checking yet — suite green.)
2. **The `affine::check` pass — E0428 (use-at-most-once).** The new module + the per-body use-count walk; sequenced in `lib.rs`; double-use → E0428 (fixture); single/zero use accepted.
3. **E0429 (multi-shot capture) + the two-sided teeth.** The multi-cross marking + E0429; the one-shot counterpart accepted; the §4 payoff example.
4. **Scoped-guarantee pins + exit gate.** Pin the callee-duplication gap as *currently accepted* (documented soundness obligation) and any over-approx false-positive worth recording; full-suite exit gate; commit + push.

### Exit criterion

`linear type` values are affine: a second use is **E0428**; a use across a `multi`-perform is **E0429** while the same use across a **one-shot** effect is accepted; non-linear code is unaffected; `eval.rs`/GC untouched; full suite green; `cargo fmt --all` + `sh scripts/check.sh` clean. **The guarantee is stated as SCOPED — the callee-duplication soundness gap and the intra-function over-approximation are named as tracked obligations, never claimed closed.**

## 10. Risks & mitigations

- **Over-reach into inference.** *Mitigation:* the only inference change is a read-only `HashSet<Span>` of affine sites; the affine logic is a separate pass — no `unify_row`, no HM change (§3).
- **Over-claiming soundness.** *Mitigation:* §0/§6/§9 scope the guarantee and name both obligations; a pinned test records the callee-duplication gap as currently accepted, so it can't be mistaken for covered.
- **False positives frustrating real code.** *Mitigation:* recorded as the completeness obligation; the first cut targets the common local pattern, and the sharpening path is known (handler-precise/branch-sensitive/inter-procedural).
- **Scope creep into full linear types.** *Mitigation:* §1.3 draws the line at affine (at-most-once), intra-function, create-and-consume.

## 11. Deferred / Honestly-Flagged

- **`affine-callee-duplication-obligation`** (soundness) and **`affine-intraprocedural-overapprox`** (completeness) — the two tracked obligations (§6), tighten inter-procedurally.
- **Linearity/multiplicity polymorphism**, **inter-procedural affine tracking**, **handler-precise / branch-sensitive liveness** — the machinery to close both obligations; the frontier-within-the-frontier.
- **Full linear (exactly-once / no-leak)** — a stronger discipline than affine; later.
- **`relay-own-effect-row-leak`** — separate row-inference slice, unrelated.

## 12. Milestone Checklist (Slice 4d-2)

- [ ] `KwLinear`; `TypeDecl.is_linear`; `linear type File { … }` parses.
- [ ] `infer` exposes affine binding sites (`HashSet<Span>`); a linear-typed `let`/param is flagged, a plain one is not.
- [ ] `src/affine.rs` `check(module, affine_sites)` sequenced after `infer`; layering updated.
- [ ] **E0428:** an affine binding used twice is rejected; ≤1 use accepted.
- [ ] **E0429:** a use across a `multi`-perform is rejected; the same use across a **one-shot** effect is accepted (two-sided teeth).
- [ ] Non-linear code unaffected; `eval.rs`/GC untouched; full suite green.
- [ ] The callee-duplication soundness gap is pinned as *currently accepted*; exit criteria state the guarantee is **scoped** and name both tracked obligations.

## 13. Far-Horizon: Full Systems-Programming Capability (the north-star-beyond-the-north-star)

*This section is not part of Slice 4d-2. It is a far-horizon note — its own large arc, sequenced after native compilation and after the linear-types frontier — recorded here because 4d-2 is the first rung of its ladder. It sets direction, not scope; nothing below is a v1 concern.*

**The goal.** One language that does **both** OS/kernel-level systems programming (bootloaders, kernels, drivers, allocators — code that runs with no OS and no runtime beneath it) **and** high-level application programming — fully, not partially. Not "Elya, plus an `unsafe` escape hatch for the hard parts"; the systems tier is a first-class target of the same language, type system, and effect discipline.

**The robust path (explicitly preferred).** Get there with **a single coherent memory model that scales app-to-kernel**, by growing the linear/affine-types frontier (this slice's arc) all the way into **full linear ownership of memory** — GC-optional, GC-free where systems code needs deterministic, manual-grade control over allocation and lifetime. Application code keeps the ergonomic managed default; systems code drops to statically-owned memory *within the same model*, because "owned exactly once, released deterministically" is the same linearity we are already building, pushed to its limit. Robustness here is reached **through Elya's own effect + linearity design**, not by importing a foreign borrow checker — the aim is Rust-class memory safety derived from Elya's semantics, not a bolted-on lifetime system with someone else's rules.

**The fragile alternative to avoid.** A **dual-mode** design — a tracing GC for app code *and* a separate manual/`unsafe` allocation mode for systems code, bridged by a boundary. This is split-brained: two memory models, two mental models, two sets of bugs, and an interior seam that every abstraction has to be aware of. We reject it in favor of the single-model path above. If the single model proves intractable, *that* is the decision to revisit — not a reason to quietly accept the split.

**Dependency chain (each a real subsystem, not a flag).**
- **Native compilation** — the Core IR → LLVM backend (itself deferred; the AST-direct interpreter is v1). Systems targets need real codegen.
- **The memory-model arc above** — linear/affine ownership grown into deterministic, GC-free memory management.
- **Bare-metal / no-runtime targets** — `#![no_std]`-class builds: no GC thread, no runtime services, freestanding binaries, control over the entry point and the machine.
- **A systems standard library** — a core library that assumes **neither a GC underneath nor an OS underneath**: allocation is explicit and owned, I/O is a capability/effect rather than an ambient syscall, and the managed-heap collections live in a separate, higher tier.

**The tension, flagged honestly.** This arc **extends** the v1 tracing-GC core, and for systems code it eventually **supersedes** it. That is a deliberate future *evolution* of the memory model — the GC remains the right default for application code — not a contradiction with v1 and not something the current slices must accommodate. The two coexist by tier; the systems tier is where the GC is dialed out.

**Revisit trigger.** Re-evaluate the tractability of this whole arc **after the linear-types frontier (4d) lands.** 4d is the first rung of the ladder: its outcome is the direct evidence for whether the linearity-to-full-ownership path is achievable *through Elya's own design*. If affine-at-most-once, then full linear ownership, land cleanly and compose with effects, the ladder is real and this section graduates into its own spec arc. If they fight the effect system or the ergonomics, that is the signal to reconsider the path — before committing to the backend and no-runtime work that assume it.
