# Slice 5b-5 (arc: the collector) — Native mark-sweep collection, the GC threshold

**Goal.** Native binaries reclaim heap memory. The allocate-don't-collect runtime from 5b-4 becomes allocate-then-collect: a stop-the-world, non-moving mark-sweep collector, threshold-triggered at the one allocation safepoint, with precise roots from a shadow stack. Proven the way every 5b claim is — by execution: a program that allocates unboundedly in a loop terminates in bounded memory, and a test-observable counter proves memory was actually freed rather than merely not crashing.

**Arc position.** This is the collector node, newly unblocked. N2 (recursion, 5b-3) plus N4 (heap, 5b-4) together made unbounded allocation possible for the first time — before them no program could allocate more than a constant number of blocks, so a collector could never be exercised. The 2-ii header was designed so collection is an *addition*, not a rewrite; that groundwork is now spent. It sits alongside N5 (closures) as an available next node; the sequencing call between them is the user's.

**Status.** Design drafted from the fork map recorded in `next-slice-decision.md` (2026-09-02) and checked against the code (`build_ctor_table` at `crates/codegen/src/lib.rs:722`); awaiting review before an implementation plan.

---

## §0. Inherited ground rules

From 5b-1/2/3/4, not re-argued here:

- **Proof is execution.** No IR snapshot; every claim is proven by compiling, linking, running, and reading exit status / stdout / stderr. No test may skip.
- **Semantic fidelity** (5b-1 §3.4). Native must be neither more- nor less-undefined than the evaluator. A clean `Unsupported` refusal is fidelity-preserving; silently mis-compiling is not.
- **Refused by name, never mis-compiled.** Every rejection is a `CodegenError` carrying a specific message.
- **Gate shape.** `cargo fmt --all` (write) before each gate; the five-stage `scripts/check.ps1` in both configurations; `CARGO_INCREMENTAL=0`.

---

## §1. What the collector must and must not be

The thin cut is deliberately small:

- **Mark-sweep, non-moving, stop-the-world.** Pointers never move, so no forwarding pointers, no write barrier, no change to how `Ctor`/`Match` read and write fields. The header's pointer-at-tag design is non-moving-friendly by construction.
- **One safepoint.** `elya_alloc` is the only allocation point, so it is the only place a collection can trigger. The collector runs *inside* `elya_alloc`, before a fresh block is handed out, when a threshold trips.
- **Iterative, never recursive.** Mark uses an explicit gray-stack; sweep walks a free list. This is a hard constraint carried from the evaluator's `CtorArgs` lesson (a naive recursive mark of a million-element `Cons` chain overflows the host stack exactly as recursive `Drop` once did).
- **A small `ccc` symbol family.** The existing `elya_alloc`/`elya_match_fail` remain the C-ABI boundary; the collector adds `elya_gc_init`, `elya_gc_root_push`/`elya_gc_root_pop`, and `elya_gc_report`. Everything Elya-internal stays `tailcc`.

---

## §2. The header: freeze the visible layout, reserve a private prefix word

The codegen-visible object is `[tag][field_0..][field_{arity-1}]`, pointer **at** the tag, unchanged. The collector needs bookkeeping the header does not carry, and the clean way to add it without disturbing anything the codegen emits is a **private prefix word the runtime allocates in front of the tag**:

```
[ runtime bookkeeping ] [ tag ] [ field_0 ] [ field_1 ] ...
  ^ elya_alloc's internal header          ^ the pointer elya_alloc returns (= the tag)
```

The prefix word holds, minimally, a **mark bit**, the **block size** (in words), and a **free-list link**. It is invisible to codegen: construction still stores the tag at `+0` and fields at `+8*(i+1)`; the codegen only ever sees the tag pointer. "Pointer-at-tag" is therefore preserved; the *only* thing that changes in emission is the tag's value (§4).

**Zeroing on reuse.** `elya_alloc`'s contract is "zeroed" (today `calloc`). Reuse must preserve that contract: a block popped from the free list and handed back out must be zeroed, or a reused object inherits a stale tag/garbage field. A correctness requirement, not a nicety.
---

## §3. Fork 1 (settled): shadow-stack roots, over-approximate liveness

The collector needs live roots, and the only viable thin cut is an explicit **shadow stack** — a runtime-maintained array of pointer roots the collector walks — forked out from two alternatives:

- **Shadow stack (this slice).** Codegen pushes every live ADT pointer before each `elya_alloc` and pops after. It is the safe choice because of an asymmetry this spec states as doctrine:

  > Under non-moving mark-sweep, a **missed root is a wrong-answer bug** (the live object is swept and reused; the differential harness catches the corrupted answer), while a **spurious root is a benign one-cycle delay** (the dead object survives one collection and is reclaimed on the next). The two failure modes are asymmetric in severity, so the safe direction is to **over-approximate** liveness. Erring toward keeping too much is harmless; erring toward losing a root is not.

- **LLVM-GC statepoints / stack maps** — deferred. The industrial answer (let LLVM record precise register/stack maps at each safepoint), but it is a rewrite of the lowering and inkwell support is thin. Not the thin cut; named as the eventual correctness ceiling.
- **Conservative stack scan (Boehm-style)** — rejected on §3.4 grounds. It is imprecise in the *unsafe* direction: an `Int` whose bits alias a heap address becomes a false root, or worse, a dereference of a non-object. That makes native *more*-undefined than the evaluator's precise refcount, which the fidelity rule forbids.

### The thin-cut discipline: "whole-live-env" push

The cheapest correct rule: at each `elya_alloc` call site, push **every ADT-typed binding currently in the lowering environment**, then allocate, then pop. "Live" is over-approximated to "in `env` with `Ty::Con`" — whether or not the binding is used again — so a soon-to-die binding may be kept one collection longer. That is the benign direction, and it defers precise per-temporary liveness to a later tightening. `Int`/`Bool`/function values are never roots.

**Carried interaction — tail calls.** The shadow stack must be correct across `musttail`, where LLVM has retired the caller's frame. Three facts close it: (a) roots are only needed *at* the single safepoint inside `elya_alloc`, never across a bare `musttail`; (b) a tail-call argument's ADT value is produced by a `Ctor` (which ran `elya_alloc`, hence already pushed its fields at the right moment), and once passed, the callee holds it as a parameter in *its* `env`, which its own allocs push; (c) no alloc happens *between* argument evaluation and the `musttail`, so there is no safepoint in that gap. The shadow stack is correct with no special tail-call machinery.

---

## §4. Fork 2 (settled): globalize the tag; emit a static descriptor table

The mark phase must learn, per object, **which stored words are pointers**. Today it cannot: `build_ctor_table` assigns each constructor the tag `i` = its *index within its own type* (`lib.rs:722-730`), so `Zero` of `Nat` and `None` of `Opt` both carry tag `0`. `match` is unaffected (it always knows the scrutinee's static type and compares tag against the right type's indices), but a collector staring at a bare pointer cannot tell a `Zero` from a `None`. §2.2's line — "the tag word uniquely identifies a (type, constructor) pair" — is **aspirational as-built, not yet true**. The collector is the first consumer that needs it true.

Fork 2 resolves **A** (globalize) over **B** (two-word header):

- **A — globalize the tag (chosen).** Assign each constructor a *globally unique* id (a per-type running offset: `nat Z`→0, `nat S`→1, `opt None`→2, `opt Some`→3, …). The object header stays **one word**; only the stored value changes. This makes §2.2 literally true. The `ptr_mask` is emitted as a **static descriptor table** — a runtime-side array indexed by global tag, recording arity and the pointer/non-pointer bit per field — built from `CoreCtor.fields: Vec<Ty>`, which already distinguishes `Int`/`Bool` (not a pointer) from `Ty::Con` (a pointer). No Core change; a codegen-side artifact.
- **B — two-word header** (`[type-id][ctor-index]`): genuinely self-describing and better for separate compilation, but grows every object to buy a capability this slice does not need. Rejected for now, named as the separate-compilation upgrade.

**The private prefix word (from §2) is explicit and orthogonal:** the runtime's mark/size/free-list bookkeeping sits *in front of* the tag, so the codegen still receives a pointer at the tag and the visible `[tag][fields]` layout stays frozen. Only the tag's *value* changes; `Ctor`/`Match` emission shape does not.
---

## §5. The mark phase (against the header) and the sweep

Given the shadow-stack roots (§3) and the global tag + descriptor table (§4):

1. **Mark — iterative gray-stack, never recursive.** Seed a worklist with the shadow-stack roots. Pop a pointer; if already marked, skip; else mark it (in the private prefix word), read its tag, index the descriptor table for arity + ptr_mask, and for each pointer field push that field's pointer onto the worklist. A million-element `Cons` chain is a million *stack entries*, not a million host-stack frames — the `CtorArgs` iterative-`Drop` lesson (5b-4 §4a) applied to the collector, as a hard constraint rather than a preference. **Schörr–Waite pointer-reversal is deferred** (it saves the worklist at the cost of transiently mutating pointers — a later space optimization, not the thin cut).

2. **Sweep — iterative free-list walk.** All live blocks sit on a runtime allocation list (threaded through the private prefix word's link). Sweep walks it once: unmarked blocks go back to the free list; marked blocks have their mark bit cleared for the next cycle. No recursion, no per-object descriptor needed here — only the block size.

3. **Allocation.** `elya_alloc` tries the free list first (first-fit), then `calloc` when empty; it counts words allocated and triggers a collection when the count crosses the threshold (§6). Freed-block reuse **zeros** (the §2 contract).

**Non-moving is the whole point.** No forwarding pointers, no write barrier, no relocation; the `Ctor`/`Match` field loads and stores the back end already emits are untouched. Copying/compacting collection is deferred indefinitely — for an immutable language there is no mutation-in-place to defragment against, so the payoff is unclear and the risk (every stored pointer becomes stale) is not.

---

## §6. The trigger and the proof of collection

**Trigger: allocation threshold.** A live counter of words allocated since the last collection; when it crosses a constant, the next `elya_alloc` collects first. Deterministic. The constant is **measured-and-pinned by the plan** (the `K_MAX` doctrine): run the corpus, observe what the build phase actually allocates, pin it with headroom, and assert *both* directions — collection trips in the unbounded loop, and does **not** trip during the corpus's build phase. If the measurement disagrees with expectations, the allocator is investigated, not the constant nudged.

**The proof is two-sided, and the second is non-negotiable.**

1. **A program that allocates unboundedly in a loop and terminates in bounded memory.** A tail-recursive function (recursion is available; 5b-3) that allocates and discards a block per iteration — with collection it runs in bounded live memory and exits 0 with the right answer, checked differentially against the evaluator. **Be precise about what this half proves, because the obvious stronger claim is false:** at any iteration count fast enough for a test, an uncollected run does *not* exhaust the heap (a million iterations retains ~56 MB, which no machine here refuses). So this tooth is not "it would die without collection" — exiting 0 rules out a crash on the way, and the counters below are the actual proof. Sizing the loop to genuinely exhaust memory would trade a fast, deterministic test for a slow one whose failure mode is the machine swapping.
2. **`elya_gc_report` asserting `freed > 0`.** Without this, a green test might be a program that never actually triggered a collection — allocate-don't-collect also "doesn't crash." The collector keeps counters (`allocations`, `collections`, `freed`); a new `elya_gc_report(void)` (`ccc`, like the rest) dumps them to **stderr** when an environment variable gates it in. **Hard requirement: gated off by default** — an ordinary run's stderr stays byte-identical to today's empty output, because every existing execution test asserts empty stderr and an unconditional dump would break the whole corpus. The one test that wants the counters sets the variable and asserts `collections > 0` **and** `freed > 0`.

Both teeth live in `native_codegen.rs` alongside the existing direct + differential runners. The first is "it runs to completion in bounded live memory"; the second is "and a collection demonstrably happened."
---

## §7. Semantic fidelity: refcount vs. trace

The evaluator **refcounts** (`Value::Ctor(String, CtorArgs)`, `CtorArgs(pub Rc<Vec<Value>>)`); native **traces**. They reclaim differently, and that divergence is named, not papered over.

- **Answer fidelity is preserved.** §3.4 compares *answers*; Elya's surface has no `sizeof`, address-of, or finalizer, so *when* memory reclaims is unobservable in output. Both back ends evaluate the same values to the same `Int`.
- **Sharpened for this subset:** with immutable, bottom-up construction there are **no cycles**, so the evaluator's eager refcount and native's lazy trace agree on *which* objects die — they differ only on *when*. The honest line is exactly: **reclamation timing is unobservable-by-construction in this subset.**
- **The cycle-collection divergence is re-filed as an N5 obligation.** When closures land, a closure can capture an environment that (transitively) captures the closure, and then the evaluator's refcount leaks that cycle while native's trace collects it. That is a real fidelity gap — but it is *N5's* gap, because cycles can be constructed there, not here.

---

## §8. Obligations

Still open, carried over: **T1** (≤5 arity), **T2** (whole-module representability), **T3** (lambda params carry no types in Core), **T4** (header promise to GC — *discharging this slice*; re-validate before any moving collector), **T5** (sequential chain is a floor), **T6** (literal-pattern match unfocused). New:

- **T7 — cycles unobservable until N5** (§7): the "unobservable-by-construction" line holds only for the acyclic ADT subset; do not silently extend it when closures land.
- **T8 — the global tag is load-bearing for collection**, but `match`'s per-type comparison still works because a scrutinee's type is known statically; keep both consumers correct under one numbering.
- **T9 — allocate-don't-collect is retired, not deleted**: `elya_alloc`'s signature and "zeroed" contract persist (§2); only the internals change.
---

## §9. Completion checklist

- [x] `build_ctor_table` assigns **globally unique** tags across all types; `Ctor` stores them; the descriptor table carries each constructor's arity + ptr_mask
- [x] Runtime reserves a **private prefix word** per block (mark/size/free-list link); codegen still gets a pointer at the tag; `[tag][fields]` visible layout frozen
- [x] `elya_alloc` reuses freed blocks (zeroed) via a free list, `calloc`-backed when empty
- [x] Mark is an **iterative gray-stack**; sweep is an **iterative free-list walk**; no recursion anywhere
- [x] Shadow-stack roots: **whole-live-env** push before each `elya_alloc`, pop after; no conservative scan
- [x] Threshold-triggered collection runs inside `elya_alloc`
- [x] The unbounded-allocation loop **terminates in bounded memory**, differentially checked
- [x] `elya_gc_report` (getenv-gated, off by default) asserts `freed > 0` (a collection demonstrably happened)
- [x] 5b-1/2/3/4 corpora pass unchanged; the evaluator is untouched
- [x] No IR snapshot; no test skips; full five-stage gate green, both configurations