# The evaluator's continuation: linear cost at any depth

**Status:** done (2026-10-10). HANDOFF step 1. Design choices are Claude's (rule 2).

## 0. Measured first

At `a583181` (tip of `auto/elya`), debug `elya run`, wall clock:

| shape | n = 1000 | 2000 | 4000 | 8000 |
|---|---|---|---|---|
| d0: `len(deep(n))`, no handler | 0.07 s | 0.20 s | 1.16 s | -- |
| d1: the same under a `handle` (PARKED's shape) | 0.11 s | 0.34 s | 1.16 s | -- |
| d0 with the depth probe disabled | 0.03 s | 0.06 s | 0.06 s | -- |
| r: `ask() + lp(n - 1)` under a handle, probe disabled | 0.12 s | 0.33 s | 1.38 s | 4.75 s |

PARKED's entry blamed the handler; d0 shows it is not the handler. Two independent causes,
both read off `src/eval.rs`:

1. **The depth probe.** `run_loop` calls `kont_len` -- a walk of the whole continuation --
   on every step, to keep `peak_kont_depth` (which the TCE tests pin). A program n frames
   deep pays O(n) per step: O(n^2) for any non-tail recursion, handled or not.
2. **Capture and re-installation copy frames.** `perform` walks the continuation to the
   handler and clones every frame on the way into a `Vec`; `resume` pushes them all back.
   A perform d frames above its handler costs O(d) twice: shape r is O(n^2) even with the
   probe gone.

A timing-free measure, `Interp::cost()` = steps + continuation nodes visited or copied by
whole-continuation operations (§3), shows both: every test shape in
`tests/eval_complexity.rs` has cost(2n)/cost(n) = 3.98-3.99 at n = 1000.

## 1. Design

### D1. The continuation is segmented at handlers (a meta-continuation)

`Kont` becomes `{ top: Seg, meta: Meta }`: `top` is the frames above the innermost handler
(a persistent list, as before), and `meta` is a list of handler boundaries, each holding
the handle's handler and environment, the segment of frames beneath it (`below`), and the
next boundary. `Frame::HandleK` disappears: installing a handle starts a fresh segment.

- A perform walks **boundaries**, not frames. Its captured continuation is `top` (shared,
  not copied) plus the boundaries it skipped; `k_rest` is the answering boundary's `below`
  and `rest`. Cost O(handlers skipped + 1).
- A resume re-installs the answering handler over `k_now` and rebuilds the skipped
  boundaries over it; `top` is reused as is. Cost O(handlers skipped + 1).
- A return into an empty `top` runs the boundary's return clause with `below`/`rest`.

Options considered:
(a) keep the flat list and only fix the probe -- leaves cause 2 (shape r stays quadratic),
and every effect-heavy differential test keeps its N cap;
(b) a flat list with "splice" nodes so resume is O(1) but perform still walks frames to
find its handler -- fixes resume, not perform;
(c) segment at handlers (this) -- the textbook meta-continuation of deep-handler machines
(e.g. the multicore-OCaml fiber stack, Koka's evidence-free CEK), and the same cost model
as native, where a perform walks handler `parent` links (5b-10) and never frames.
**Taken: (c)**, the one where the evaluator's cost model is the native one.

Multi-shot stays sound by construction: segments are immutable persistent lists, so a
second resume of the same `top` pops shared nodes by cloning their frames, exactly what the
old per-resume `f.clone()` did. The one-shot rule (E0425) is unchanged and keyed on
`Handler::multi`, as before.

### D2. Depth is cached in the nodes

Every segment node records its segment length, every boundary the total depth beneath and
including it (`1 + len(below) + depth(rest)`). `kont_depth` is O(1), and a boundary counts
as one frame -- exactly what `HandleK` counted -- so every pinned `peak_kont_depth` (TCE,
state, closures, match) keeps its value. Option rejected: drop the peak measurement. The
TCE guarantees are pinned by it and are worth more than the counter.

### D3. Dropping a deep continuation is iterative

A captured `top` is now a shared linked list rather than a `Vec`, so dropping an abandoned
100 000-frame continuation (a clause that does not resume) would recurse through nested
destructors on the host stack. `KontNode` and `MetaNode` get an iterative `Drop`, as
`CtorArgs` has. A guard test (abort at depth 100 000) pins it; its negative control is
removing the `Drop`.

## 2. What does not change

The evaluator is the reference semantics: every differential corpus (native vs evaluator,
CEK vs tree-walker) stays green with unchanged expectations, and no existing test's value
changes. Output, errors (E0300, E0425) and `peak_kont_depth` are identical.

## 3. The cost counter

`Interp::cost()` = machine steps + nodes visited or built by the depth probe, a perform's
search and capture, and a resume's re-installation. It is a measure, not a limit; tests
assert `cost(2n) <= 2 * cost(n) + 64` (linear doubles, quadratic quadruples).

## 4. Negative controls

Each was applied, run, and reverted (`cmp` against the saved file).

| control | predicted | observed |
|---|---|---|
| K1: restore the per-step walk (count the depth every step) | 5 ratio tests fail | 5 fail |
| K2: count a frame-by-frame copy of `top` at capture | 2 perform tests fail | 2 fail |
| K3: count a frame-by-frame re-push of `top` at resume | perform tests and multi-shot fail | the 2 perform tests fail; multi-shot passes |
| K4: remove the iterative `Drop`s | the abandon guard overflows | "has overflowed its stack" |

K3's miss is a wrong prediction, not a gap: the multi-shot shape re-enters ONE n-frame
segment twice, 2n work, linear with or without the copy. The perform tests are the ones
that re-enter at every level, and they catch it.

## 5. Results

| test shape | cost(1000) before | ratio before | cost(1000) after | ratio after |
|---|---|---|---|---|
| deep | 16 096 054 | 3.99 | 32 022 | 2.00 |
| deep under a handle | 16 116 081 | 3.99 | 32 028 | 2.00 |
| perform at every level | 11 079 047 | 3.99 | 25 018 | 2.00 |
| perform past two handlers | 11 123 097 | 3.98 | 29 026 | 2.00 |
| multi-shot at depth | 10 585 615 | 3.98 | 21 041 | 2.00 |

The PARKED shape at n = 100 000 (deep list under a handle, plus a perform at every level)
now runs in the test suite; before, it did not finish (the guard test timed out at 2
minutes on the old machine too, through the probe).
