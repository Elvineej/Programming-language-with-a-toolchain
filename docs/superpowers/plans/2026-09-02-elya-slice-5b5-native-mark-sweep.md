# Elya Slice 5b-5 — Native mark-sweep collection — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Teach the native runtime to reclaim the heap it has been allocating since 5b-4. `elya_alloc` grows from an allocate-don't-collect `calloc` wrapper into a stop-the-world, non-moving mark-sweep collector: construction tags become globally unique ids driving a static descriptor table; the one allocation site (`Ctor`) pushes its live roots onto a shadow stack so the collector can reach them; and a threshold-triggered mark/sweep recycles dead blocks through a free list. Proven by execution — an unbounded-allocation loop that terminates in bounded memory, and a counter asserting `freed > 0`.

**Architecture:** The collector is a codegen + runtime change with **no Core reach** — the pointer mask a mark phase needs is already in `CoreCtor.fields: Vec<Ty>`, and the evaluator (which refcounts) is untouched. Four tasks: globalize the tag (a no-new-capability refactor reviewed alone, mirroring 5b-4 Task 1); grow the runtime and declare its `ccc` symbols (inert, symbols declared but collection not yet triggered); wire the shadow-stack roots and the threshold plus both proof teeth (the headline); close out.

**Tech Stack:** Rust (three-crate workspace), inkwell 0.5, LLVM 18.1.6, clang 22.1.8 as link + C driver, the existing `crates/codegen/src/runtime.c`.

**Spec:** `docs/superpowers/specs/2026-09-02-elya-slice-5b5-native-mark-sweep-design.md`

## Global Constraints

- **Crate boundary.** LLVM lives *only* in `crates/codegen`; never add `inkwell`/`llvm-sys` to the root `elya` crate or `crates/cli`.
- **`CARGO_INCREMENTAL=0` on every cargo invocation.**
- **`cargo fmt --all` (write mode) before every gate.** The gate fmt-*checks* and fails hard.
- **Gate command:** `powershell -NoProfile -File scripts/check.ps1` — five stages, both configurations. From Cline, run it detached via `scripts/check-bg.ps1` and check `-Status`.
- **Proof is execution.** No `insta` snapshot of LLVM IR anywhere. No test may skip (`#[ignore]`).
- **Semantic fidelity** (5b-1 §3.4). Native must be neither more- nor less-undefined than the evaluator. This is why root discovery is a precise shadow stack, *not* a conservative stack scan — the latter can read an `Int` as a pointer and make native more-undefined.
- **Non-moving mark-sweep only.** No forwarding pointers, no write barrier, no relocation; the `Ctor`/`Match` field loads and stores the back end already emits must not change in *shape*.
- **The mark phase is iterative, never recursive** — a million-element `Cons` chain is a million worklist entries, not host-stack frames (the `CtorArgs` lesson applied to the collector as a hard constraint).
- **`elya_gc_report` is gated off by default.** Ordinary runs must produce byte-identical (empty) stderr; every existing execution test asserts empty stderr, and an unconditional dump would break the whole corpus.
- **One-allocator discipline.** `elya_alloc` has exactly one call site (the `Ctor` arm), so shadow-stack instrumentation is localized to that one arm. Preserve that invariant; if a second allocation site ever appears, its roots must be wired the same way.
---

## Task 1: Globalize the constructor tag (no new capability)

The mechanical center, reviewed alone. `build_ctor_table` today assigns each constructor its *index within its own type* (`lib.rs:722`), so `Zero` of `Nat` and `None` of `Opt` both carry tag `0`. A collector staring at a bare pointer cannot tell them apart. Globalize the id — a running offset across all types — so §2.2 becomes true rather than aspirational. Both `Ctor` (store) and `Match` (compare) already read the single `lc.ctors` table, so they change together and behaviour is **provably unchanged**: the 5b-4 ADT corpus is the regression suite.

**Files:** `crates/codegen/src/lib.rs`.

- [x] **Step 1: Globalize the tag.** In `build_ctor_table`, replace the per-type `enumerate()` index with a running global id. The map's value stays `(usize, Vec<Ty>)`; only the `usize` semantics change:

```rust
/// Fold `core.types` into a flat constructor table: name -> (tag, field types).
/// The tag is now a GLOBALLY unique id (a running offset across all types), not
/// the index within its own type — a collector must be able to recover a
/// constructor's type from the tag alone (spec §4).
fn build_ctor_table(core: &CoreModule) -> HashMap<String, (usize, Vec<Ty>)> {
    let mut out = HashMap::new();
    let mut next = 0usize;
    for t in &core.types {
        for c in t.ctors.iter() {
            out.insert(c.name.clone(), (next, c.fields.clone()));
            next += 1;
        }
    }
    out
}
```

- [x] **Step 2: Gate.** `cargo fmt --all`, then the full gate (detached). Expected: green, **no new tests** — pure renumbering, the 5b-4 ADT corpus (`the_adt_corpus_compiles_runs_and_prints_the_expected_answer`, `native_output_matches_the_evaluator_across_the_adt_corpus`, `a_nested_match_joins_through_an_n_armed_phi`, `a_failed_match_traps_with_a_named_error`) plus every 5b-1/2/3 test unchanged.

- [x] **Step 3: Commit.**

```text
refactor(codegen): globalize constructor tags (5b-5 Task 1)
```
---

## Task 2: The mark-sweep runtime + its `ccc` symbols (inert)

Grow `runtime.c` into a real collector — private prefix, per-size free lists, an all-blocks list, an iterative mark, a sweep — and declare the new `ccc` symbols in the module with the descriptor table emitted and initialized. Collection is **not yet triggered** (no roots are pushed, so a triggered collection would free live objects); `elya_alloc` must remain behaviourally a `calloc` wrapper for this task, proven by the unchanged corpus.

**Files:** `crates/codegen/src/runtime.c`, `crates/codegen/src/lib.rs`.

**Object layout (pinned):** `elya_alloc(words)` reserves `words + 2` words and returns a pointer to word 2. Words 0–1 are the collector's private prefix — *in front of* the tag, invisible to codegen, so the visible `[tag][fields]` layout the `Ctor` arm emits is frozen:

```
[ word0: (size<<1)|mark ] [ word1: all-blocks next-link ] [ tag ] [ field_0 ] ...
   ^ internal                 ^ internal                    ^ the pointer elya_alloc returns
```

Two prefix words, not one, because the all-blocks-list link needs a full pointer word. (The spec said "a private prefix word"; the plan uses two — size/mark plus a link — which is the same idea stated precisely. Deliberate spec→plan refinement.)

- [x] **Step 1: Write the runtime.** Replace `crates/codegen/src/runtime.c` with the collector. Key pieces and invariants:

```c
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>

/* GC state (single-threaded: Elya has no native concurrency yet). */
static int64_t   *gc_descriptors = NULL;   /* [2*i]=arity, [2*i+1]=ptr_mask */
static int64_t    gc_n_ctors = 0;
static void     **gc_shadow = NULL;        /* growable root stack */
static int64_t    gc_shadow_top = 0, gc_shadow_cap = 0;
static int64_t    gc_allocated = 0, gc_collections = 0, gc_freed = 0;

typedef struct Block { intptr_t meta; struct Block *next; } Block;
static Block *gc_all_blocks = NULL;        /* singly-linked list of ALL blocks */

enum { GC_MAX_WORDS = 16 };
static Block *gc_free_lists[GC_MAX_WORDS] = {0};  /* one per object word-count */

#define GC_MARK_BIT 1
```

The load-bearing invariants: (a) mark is an **explicit worklist** — push/pop on a `Block**` stack, never function recursion; (b) sweep walks `gc_all_blocks` once, reading each block's `meta` for size + mark, moving unmarked blocks to `gc_free_lists[size]` (their `next` link overwriting the dead tag slot, which is safe because the object is dead); (c) `elya_alloc` tries the per-size free list first (zeroing — the contract), else `calloc`, then *if enabled* checks the threshold; (d) `elya_gc_report` writes stats to stderr **only** when `getenv("ELY_GC_STATS")` is set; (e) a `gc_enabled` flag exists but is off, and the threshold constant is declared but not wired, until Task 3.

```c
void elya_gc_init(const int64_t *descriptors, int64_t n_ctors);  /* record table + n */
void elya_gc_push(void *root);   /* grow the shadow stack, store root  */
void elya_gc_pop(void);          /* drop the top root                */
void *elya_alloc(int64_t words); /* free-list first, then calloc; collect only if enabled */
void elya_match_fail(void);      /* unchanged: the deterministic trap  */
void elya_gc_report(void);       /* getenv-gated stderr stats         */
```

- [x] **Step 2: Emit the descriptor table.** In `build_module`, build a `Vec<i64>` of `2 * total_ctors` entries in **global-tag order** (the same order Task 1's running offset produced): `[arity, ptr_mask, …]`, where `ptr_mask` bit *i* is set iff `CoreCtor.fields[i]` is `Ty::Con`. Emit as a constant `i64` global:

```rust
let desc: Vec<u64> = core.types.iter()
    .flat_map(|t| t.ctors.iter())
    .flat_map(|c| {
        let arity = c.fields.len() as u64;
        let mut mask = 0u64;
        for (i, f) in c.fields.iter().enumerate() {
            if matches!(f, Ty::Con(..)) { mask |= 1 << i; }
        }
        [arity, mask].into_iter()
    })
    .collect();
let desc_const = i64t.const_array(
    &desc.iter().map(|&d| i64t.const_int(d, false)).collect::<Vec<_>>(),
);
let desc_global = module.add_global(desc_const.get_type(), Some(AddressSpace::default()), ".gc_desc");
desc_global.set_initializer(&desc_const);
desc_global.set_constant(true);
```

- [x] **Step 3: Declare + init the symbols.** In `build_module`, next to the existing `elya_alloc`/`elya_match_fail` declarations, add `elya_gc_init`, `elya_gc_push`, `elya_gc_pop`, `elya_gc_report` (all `ccc`, `None` convention). Extend `LowerCtx` with the new `FunctionValue` fields. In the `@main` shim, **before** the `@elya_main` call, call `elya_gc_init(&.gc_desc, n_ctors)`.

- [x] **Step 4: Gate.** The corpus must pass unchanged — `elya_alloc` is still `calloc`-equivalent here (free list always empty; collection disabled), and `elya_gc_report` is off by default so stderr stays empty. Full gate detached; the ADT corpus, the trap test, and the overflow test are the regression.

- [x] **Step 5: Commit.**

```text
feat(codegen): the mark-sweep runtime, descriptor table, and gc symbols (5b-5 Task 2)
```
---

## Task 3: Shadow-stack roots, the threshold, and both proof teeth

The headline. Push live ADT roots before the one allocation, enable collection, and prove it works two ways. This is where the collector stops being inert and becomes observable — and it lands as one task because roots-without-collection is inert and collection-without-roots is a use-after-free: they must be enabled together, never separately.

**Files:** `crates/codegen/src/lib.rs`, `crates/codegen/src/runtime.c`, `crates/codegen/tests/native_codegen.rs`.

### The threshold: measure-and-pin (K_MAX doctrine), before anything triggers

- [x] **Step 1: Instrument the count.** The runtime already tracks `gc_allocated` (words since last collection). Before enabling collection, measure what the *build phase* of the existing corpus actually allocates: run the full ADT + differential corpus with `ELY_GC_STATS` emitted (a temporary `fprintf` in `elya_alloc`'s exit path, or `elya_gc_report` called from the shim), observe the **maximum** running allocation across all programs, and record it. Do **not** guess.

- [x] **Step 2: Pin with headroom.** Set `GC_THRESHOLD_WORDS` to the measured max **plus** a comfortable factor (≥ 4×), so no corpus program trips collection during its build phase. The two-sided teeth: (a) an assertion that a representative corpus program runs with `collections == 0`; (b) an assertion that the unbounded loop runs with `collections > 0`. If a measurement disagrees with expectations, investigate the allocator — **never** nudge the constant to make a test pass.

### The shadow-stack root discipline

- [x] **Step 3: Wire the roots into the `Ctor` arm.** In `lower_expr`'s `Ctor` arm, the live ADT roots at the single alloc site are exactly two sets: every **`Ty::Con`-typed `env` binding** (later code will read them) and every **`Ty::Con`-typed field value just lowered** (`vals` — the outer object holds pointers to them). Over-approximating is safe (spurious roots are a benign one-cycle delay); missing one is a wrong answer. Before the `elya_alloc` call, push each ADT-typed root; after it, pop the same count:

```rust
// The collector's roots at the ONE allocation site: every live ADT-typed
// env binding, plus the ADT-typed field values about to be embedded in the
// new object. `field_tys` is in hand (from lc.ctors), so classify each val.
let mut pushed = 0usize;
for v in env.values() {
    if matches!(v.get_type(), BasicTypeEnum::PointerType(_)) {
        b.build_call(lc.gc_push, &[(*v).into()], "").map_err(internal)?;
        pushed += 1;
    }
}
// field vals may be ADT (PointerType) or Int/Bool; push only the pointers.
for (i, fv) in vals.iter().enumerate() {
    if matches!(field_tys[i], Ty::Con(..)) {
        b.build_call(lc.gc_push, &[(*fv).into()], "").map_err(internal)?;
        pushed += 1;
    }
}
let p = /* ... existing elya_alloc call ... */;
for _ in 0..pushed {
    b.build_call(lc.gc_pop, &[], "").map_err(internal)?;
}
```

> The pointer-ness of `vals[i]` is recovered from `field_tys[i]` (`Ty::Con` ⇔ pointer), matching exactly how the store below it already classifies the word.

- [x] **Step 4: Enable collection.** In `runtime.c`, replace the inert `gc_enabled` with the threshold check: in `elya_alloc`, before handing out a fresh block, if `gc_allocated >= GC_THRESHOLD_WORDS`, run `gc_collect()` (mark from `gc_shadow`, sweep into the free lists, reset `gc_allocated`, increment `gc_collections`). The `elya_gc_report` stderr dump stays getenv-gated.

### The teeth

- [x] **Step 5: The bounded-memory loop.** In `native_codegen.rs`, add a tail-recursive program that allocates and discards a block each iteration, at a count high enough to cross the threshold many times over:

```rust
const UNBOUNDED: &str = "\
type Node { End, Link(Node) }
fn loop(n) { if n == 0 { 0 } else { let _ = Link(End)  loop(n - 1) } }
pub fn main() { loop(1000000) }
";
```

Assert it exits 0 with the right answer *and* that `elya_gc_report` (gated on `ELY_GC_STATS`) shows `collections > 0` **and** `freed > 0`. That second assertion is the non-negotiable tooth: without it, a green test could be a program that never triggered a collection.

Be precise about what this proves, because the obvious stronger claim is false. A million iterations retains about 56 MB if nothing is ever reclaimed — 56 bytes an iteration, an `End` and a `Link` with two prefix words each — and no machine this runs on refuses that. So the test is NOT "it would die without collection"; the counters are the whole proof, and exiting 0 only rules out a crash on the way. Making the loop big enough to genuinely exhaust memory would trade a fast, deterministic test for a slow one whose failure mode is the machine swapping.

- [x] **Step 6: The no-trip-during-build assertion.** A separate test runs the whole ADT corpus with `ELY_GC_STATS` set and asserts `collections == 0` for every program — the threshold is above what a normal program's build phase allocates, so ordinary programs never pay for a collection they don't need.

- [x] **Step 7: Gate + commit.** Full gate green; every prior corpus test still passes with empty stderr.

```text
feat(codegen): shadow-stacked roots and threshold-triggered collection (5b-5 Task 3)
```
---

## Task 4: Close-out — docs, checklist, ledger

The slice is not done when the tests pass; it is done when the documentation stops lying about the runtime's memory behaviour and the spec/plan checklists reflect what was built.

**Files:** `README.md`, the spec, this plan; ledger in memory (uncommitted).

- [x] **Step 1: Patch the README.** The "As of Slice 5b-4 …" paragraph currently says ADTs are "heap-allocated (tag + fields)". Amend it to say the runtime now *reclaims* that heap — a threshold-triggered mark-sweep collector backed by a shadow stack — so a compiled binary that allocates unboundedly in a loop runs in bounded memory rather than exhausting it. Keep the fidelity framing: the evaluator refcounts, native traces, and reclaim timing is unobservable in this acyclic subset.

- [x] **Step 2: `graphify update .` + `cargo fmt --all` + the full gate** (detached), expected green, exit 0, no `.snap.new`.

- [x] **Step 3: Tick the spec's §9 checklist** (all boxes now hold by test) and **tick every `- [ ]` in this plan**.

- [x] **Step 4: Update the slice ledger** in memory: mark Slice 5b-5 (the collector) closed, note the arc (N1/N3/N2/N4 + the collector all landed), record obligations — **T4 discharged** (the header promise to GC is repaid: this slice *is* the collector), **T7** (cycles unobservable until N5), **T8** (global tag is load-bearing), **T9** (allocate-don't-collect retired, not deleted), and the carried **T5/T6** plus **T1/T2/T3**. Name N5 (closures) and N7 (runtime polymorphism) as still-open.

- [x] **Step 5: Commit.**

```text
docs(codegen): close out Slice 5b-5 — native mark-sweep collection
```

---

## Execution approach

1. **Subagent-driven (recommended)** — a fresh subagent per task, review between tasks.
2. **Inline** — execute tasks here with `executing-plans`, pausing at each gate for review.

Which approach?