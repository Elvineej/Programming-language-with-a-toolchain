# Native multi-shot handlers (`with multi`)

**Status:** in progress (2026-10-10). HANDOFF step 1. Design choices are Claude's (rule 2).

## 0. Measured first

At `f65937a` (tip of `auto/elya`). Every program below checks clean; the evaluator's value
(`run_module_value`) and what `elya build` does today:

| # | Shape | evaluator | native today |
|---|---|---|---|
| m1 | `if flip() {1} else {2}`, `resume(True) * 10 + resume(False)` | 12 | refused: "multi-shot handler (`with multi`)" |
| m2 | two flips in sequence, `resume(True) * 1000 + resume(False)` | 12012000 | refused |
| m3 | the flip under an inner CPS `Ask` handle whose clause is `resume(5) * 2` (non-tail) | 40100 | refused |
| m4 | backtracking: subsets of 1..12 summing to 7, `resume(True) + resume(False)` | 5 | refused |
| m5 | a PLAIN `with` over a `multi` effect, resuming once | 1 | refused (keyed on the declaration, 5b-8 §5.2's over-refusal) |
| m6 | a plain `with` over a `multi` effect, resuming twice | E0425 | refused |
| m7 | `let k = fn(b) { resume(b) }  k(True) * 10 + k(False)` | 12 | refused |
| m8 | multi `Pick` nested inside multi `Flip` | 10199898 | refused |
| m9 | a ONE-SHOT inner clause that performs the multi `flip` before resuming | E0425 | refused |
| m10 | flip under `Tell` under `Ask`; the `ask` after the flip escapes `Tell` to `Ask` | 10012 | refused |
| m11 | 40 x a 300-deep non-tail recursion with a flip at the bottom | 3612120 | refused |
| m12 | 1000 x an 8-flip `count` (256 runs each) | 1024000 | refused |

What a native frame is today (read off `cps_emit.rs`): a captured continuation is the chain
of heap frames from `k` (the perform's continuation) through `next` (word 2) to the
answering handler frame `h`. Site frames `[tag][code][next][saved..]` are written once, at
allocation, and never again. Handler frames `[tag][code][next][table][parent][saved..]` are
MUTATED: every resume sets `h.next` to the resume's own continuation and `h.parent` to the
handler current at the resume (5b-10), and an inner CPS handle's clause that resumes does
the same to ITS frame. The continuation object `[tag][k][h][innermost]` has its `h` word
zeroed by the first resume (D13's one-shot flag).

So running one chain twice is wrong whenever the chain contains a handler frame that the
first run mutated. m3 is the minimal case: the second run's `ask` reads the inner frame's
`next` that the first run left behind and applies the `* 2` twice (predicted natively with
no copy: 40200 for 40100). m10 adds a `parent` link inside the captured segment.

## 1. Design

### D1. The copy rule keys on the HANDLER's `multi`, not the effect's declaration

Core gains `CoreHandle::multi` (the `with multi` bit, `Handler::multi`). A resume belongs to
exactly one handle (`cps::resume_owners`); a resume whose handle is `multi` re-enters by
copying (D2); every other resume keeps D13's one-shot flag and trap.

Options: (a) key on `is_multi_declared` (the effect is declared `multi`), as 5b-8's refusal
did; (b) key on the handler's `multi`. (a) would let a plain handler over a `multi` effect
re-enter natively where the evaluator stops with E0425 (m6), so the two semantics would
disagree. **Taken: (b)**, the stricter one: native agrees with the reference evaluator on
every program, including the failing ones. m5 and m6 stop being refused: m5 compiles
one-shot (1), m6 traps by name like E0425. `is_multi_declared` stays on Core (lowering and
its tests read it); codegen no longer reads it.

### D2. Copy on every multi-shot resume; the captured chain itself never runs

A multi-shot resume first copies the chain `k .. h` (inclusive) and runs the COPY; the
original stays exactly as captured, so any number of later resumes see it unchanged. This is
the evaluator's rule ("each resumption is an independent run of the same immutable captured
frames", `eval.rs` `resume_apply`) made literal.

Options: (a) copy at capture (every perform) -- pays for continuations resumed zero or one
times and still needs a copy per extra resume; (b) copy every resume but the last -- "last"
is not known statically, and a dynamic count would need the clause to declare it; (c) make
handler frames immutable (move `next`/`parent` into the continuation object or a separate
cell) -- a redesign of 5b-10's whole handler protocol for a feature that is opt-in; (d) copy
only the handler frames -- every frame whose `next` points at a copied one must be copied
too, and the chain ENDS at `h`, so (d) is the whole chain. **Taken: copy every resume.**

Cost, stated: one allocation and `size + 2` words written per frame in `k .. h`, so
O(|chain|) per resume, plus one 3-word continuation object. The evaluator clones every
captured frame per resume (`f.clone()` over `rd.captured`), the same order. One-shot
resumes are untouched (no new code on that path but a compile-time branch).

### D3. The copy lives in the runtime, generic over frame size

`void *elya_cont_copy(void *cont, int64_t hlo, int64_t hhi)` walks `next` from `k` to `h`,
allocates each copy with `elya_alloc` (the block header already holds the size, so no
per-site code is generated), relinks each copy's `next` to the next copy, rewrites the
`parent` word of every copied HANDLER frame that points into the segment to the
corresponding copy, and returns a fresh continuation object whose `k`, `h` and `innermost`
name the copies. Handler frames are recognised by tag: handler rows are assigned
consecutively just below the continuation row (`descriptor_rows`), so `[hlo, hhi)` with
`hhi = cont_tag`, `hlo = cont_tag - #handlers` is exactly the handler tags; a guard in
`descriptor_rows` makes that a compile-time property. `gc_mark` is untouched (A7).

GC: the copy allocates, so the caller roots everything it still needs (the resume's
argument and every live binding) across the call, and the runtime roots the continuation
object and each copy as it goes. Copies start as word-for-word duplicates of live frames,
so a collection in the middle of a copy traces only valid pointers.

A chain that reaches null before `h` stops by name (`elya: resume: a continuation's frame
chain does not reach its handler`): unreachable from a clean front end, a guard, never a
crash.

### D4. Copies are shallow at the frame level

Saved values (closures, strings, ADTs, and continuation objects of OTHER handles) are
shared between runs, as the evaluator shares them (its captured frames hold `Value`s, and a
`Value::Resume` holds its consumed flag behind an `Rc`). So a one-shot continuation saved in
a multi-shot segment is consumed by the first run and traps in the second (m9): E0425 in the
evaluator, the named trap natively -- both fail, as the differential corpus expects.

### D5. Invariant N8-1 still holds

Copies are heap frames, so continuation depth stays heap residency; a multi-shot loop's live
set must settle across N (M4 below), and a copy that leaked would grow it.

## 2. Acceptance

- **M1** the multi corpus (m1, m2, m3, m4, m7, m8, m10, m11, m12) runs natively to the
  evaluator's values, and differentially.
- **M2** m5 runs (1); m6 stops with the named one-shot trap (and E0425 in the evaluator).
- **M3** m9: both sides fail (E0425; the trap).
- **M4** a multi-shot loop's live set is equal over four N across an 8x spread, with
  collections > 0.
- **M5** m11 collects (ELY_GC_STATS) and still prints 3612120 natively: copies survive a
  collection that happens mid-copy.
- **M6** Core stamps `multi` from `with multi` only (a plain `with` over a `multi` effect is
  `multi = false`, `is_multi_declared = true`).
- Negative controls (each reverted): K1 no copy (share the chain) -> m3 wrong (predicted
  40200); K2 no `parent` remap -> m10 wrong (predicted 10024) while m3 stays right; K3 the
  one-shot flag kept on multi resumes -> m1 traps; K4 `innermost` not remapped -> m10 wrong.

## 3. Evidence

(Filled at close-out.)
