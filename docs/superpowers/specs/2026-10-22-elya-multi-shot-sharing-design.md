# Multi-shot resumes share the captured chain

**Status:** done (2026-10-22, branch `auto/elya`). HANDOFF step 1. The design
choices are Claude's under the maintainer's delegation (rule 2), recorded here.
Revises `2026-10-10-elya-native-multi-shot-design.md` D2/D3 (copy the whole chain on
every multi-shot resume) and the frame linkage of `2026-10-08-elya-slice-5b10-nested-handles-design.md`.

## 0. Measured first (at 7ac69a6)

PARKED.md's program (2026-10-16): a `with multi` clause that resumes NON-tail, over a
non-tail recursion that performs at every level.

```
effect multi Flip { fn flip() -> Bool }
fn nest(n) { if n == 0 { 0 } else { let b = flip()  if b { 1 + nest(n - 1) } else { nest(n - 1) } } }
pub fn main() -> Int { handle nest(N) with multi { Flip.flip() -> 0 + resume(True) } }
```

| N | native time | native max RSS | native `live` words (last collection) | evaluator |
|---|---|---|---|---|
| 1 000 | 0.06 s | 25 MB | 1 021 671 | 0.02 s, 27 MB |
| 2 000 | 0.57 s | 94 MB | 4 077 373 | 0.04 s, 27 MB |
| 4 000 | 3.22 s | 370 MB | 16 290 856 | 0.08 s, 27 MB |
| 8 000 | 10.5 s | 1.47 GB | 65 126 164 | 0.12 s, 29 MB |
| 20 000 | `elya: out of memory` after 32 s under `ulimit -v` 4 GiB | | | 0.29 s, 32 MB |
| 100 000 | | | | 1.20 s, 57 MB |

- **The live set is quadratic** (`live` x4 per doubling; `freed` is 13 168 blocks at
  N = 8 000), so the copies are not garbage: each is retained.
- **Why.** A continuation of depth `i` is copied in full (`elya_cont_copy`, 2026-10-10
  D2). The next level's original chain is built on top of that copy (its frames link
  into it through `next`), and every original stays reachable because a direct
  resume roots the clause's whole environment, its continuation included, across the
  resumed computation (`resume_nested`). So level `i` keeps a copy of `i` frames
  alive: `N^2/2` frames in all.
- **The evaluator is linear.** Its captured frames are immutable values shared by every
  resumption (the meta-continuation of `2026-10-10-elya-eval-linear-continuations`).
- **Which native frames are written after allocation** (every `store_word` in
  `cps_emit.rs` on a frame it did not just allocate): only HANDLER frames (`next` and
  `parent`, at every resume: `resume_cps`, `reparent`) and continuation objects (word 2,
  the one-shot mark). Site frames are written once, when they are built.
- **Why the whole chain is copied today** (2026-10-10 D2 option (d)): a handle's body is
  a CPS region whose continuation IS its handler frame, so the body's last site frame
  points at `h` through `next`, and every frame between `k` and `h` reaches `h` that
  way. Copying `h` forces copying every frame that leads to it.

## 1. Decisions

- **D1. A handle's body ends at one shared end frame, not at its handler frame.**
  - The body function still receives its handler frame (it loads the frame's saved
    bindings from it), but its region's continuation is the end frame `E`.
  - `E`'s code, `elya_rt_handle_end` (generated, `tailcc`, the frame-code signature
    `(i64, ptr) -> i64`), loads `elya_current_handler` and `musttail`s into that frame's
    code (its return clause) with the frame. That is the handler the old link named.
  - **The invariant that makes it exact:** wherever control reaches the end of a
    handle's body, that handle's own frame is current. Install makes it current. Every
    inner handle's clause and return clause run with that inner frame's `parent`
    current, which is this frame. A resume makes the perform's `innermost` current
    again (5b-10).
  - A handler frame's `next` keeps its meaning (where the handle's answer goes) and
    keeps being rewritten by a resume. But no site frame points at a handler frame any
    more, so a chain from `k` reaches `E`, never `h`.
  - *Cost:* one indirect `musttail` per handle-body return. One-shot code is otherwise
    unchanged.
- **D2. A multi-shot resume copies only the handler frames from `innermost` up to `h`,
  through `parent`, and shares every site frame.**
  - Site frames are immutable after allocation (§0), so sharing them is the evaluator's
    rule ("each resumption is an independent run of the same immutable captured
    frames") with no copying at all.
  - The frames a run can WRITE are the handler frames it re-enters. Those are exactly
    the ones the perform walked from `innermost` to the answering `h`. A handle inside
    the segment whose clause is running is not on that path, and it was not on the old
    `next` chain either; its continuation object is shared, as D4 of 2026-10-10 already
    says.
  - `elya_cont_copy` walks `parent` from word 3 (`innermost`) to word 2 (`h`). It copies
    each frame (its size is in the block header, as before), links each copy's `parent`
    to the next copy, and returns `[tag][k][h'][innermost']` with `k` itself shared.
    The resume then re-installs `h'` exactly as before. A path that reaches null, or a
    frame whose tag is not a handler's, before `h` stops by name ("a continuation's
    handler chain does not reach its handler"). That is unreachable from a clean front
    end: a guard, never a crash.
  - *Cost:* O(handlers on the path) per resume, instead of O(frames in the chain). The
    PARKED program copies one frame per resume; `m-deep-handler-chain` still copies
    1 001.
- **D3. `elya_current_handler` becomes a collector root.**
  - Before, it was "reachable from the live continuation through `next` and `parent`".
    D1 removes the `next` route: a running body's chain reaches `E`, not its handler. A
    frame that is current and saved nowhere else (a handle with no performs pending)
    would otherwise be freed under its own body.
  - `gc_collect` pushes it as a gray root before `gc_mark` runs. `gc_mark` itself stays
    byte-identical.
  - `E` is a static block in the runtime with its mark bit set. It is never on the
    collector's block list, so it is never swept, and marking it stops at once. Its
    code word is set by `elya_handle_end_init(code)`, which the generated `main` calls
    right after `elya_gc_init`. The runtime references no generated symbol, so the C
    harness tests that link `runtime.c` alone keep linking.
- **D4. The continuation object keeps its layout** (`[tag][k][h][innermost]`, words 1
  and 3 traced). `h` stays untraced: it is on `innermost`'s `parent` path, which is
  traced.

*Options considered:*
- (a) Keep copying the chain, and stop rooting the clause's continuation across a
  direct resume. That fixes the live set but not the time: still `N^2/2` frame copies.
- (b) Copy lazily, as frames are re-entered. Every frame re-entered would need a proxy
  that knows which run it belongs to, because the chain still ends at a per-run `h'`.
  That is the same remapping, moved to run time.
- (c) The end frame plus a handler-only copy. **Taken:** it removes the one link that
  forced (a)'s copying, keeps the frame layout, and the resume protocol stays as 5b-10
  wrote it.

## 2. Evidence

Measured after the change, the same programs as §0:

| N | native time | native max RSS | native `live` words | evaluator |
|---|---|---|---|---|
| 8 000 | 0.01 s | 11 MB | 104 866 (13 a level) | 0.12 s |
| 20 000 | 0.01 s | 11 MB | 188 757 | 0.29 s |
| 100 000 | 0.07 s | 36 MB | 1 100 834 | 1.20 s |

The CPS variant (the multi handle leaks `S`) frees 23 830 blocks at N = 4 000 (6 a
level). Before the change it freed 8 017 989.

`crates/codegen/tests/native_codegen.rs`, each red first as the plan predicted:
- `a_deep_multi_shot_chain_is_shared_not_copied` (N = 8 000 and 16 000, under 64 live
  words a level). Red: 65 126 164 at N = 8 000;
- `a_deep_cps_multi_shot_chain_is_shared_not_copied` (N = 2 000 and 4 000, under 64
  blocks freed a level). Red: 2 008 989 at N = 2 000;
- `the_parked_deep_multi_shot_program_runs_natively` (N = 20 000 on the default stack).
  Red, under `ulimit -v` 4 GiB: `elya: out of memory copying a continuation`;
- `a_handler_frame_reachable_only_as_current_survives_collections` (a direct and a CPS
  handle). A guard: green before the change, red under K4 (below).

**Changed test (a consequence of D2):** `copies_survive_a_collection_mid_copy` ran
m11, whose 300-frame chain used to be copied whole. Its copies are now one frame, and
it never collects (`collections: 0`, the answer still right), so the test no longer
tested what it is named for. It now runs a new corpus row, m13 (`go(100)` over 301
nested handlers: 301-frame handler paths, copied 200 times, 16 collections). m11 stays
in the corpus for its value and the differential.

Controls (each reverted, `cmp` against the kept files):
- **K1**, the copy returns the continuation unchanged: m3 prints 40200 (the 2026-10-10
  K1 value), m8 10199998, m10 10024.
- **K2**, copies' `parent`s not relinked: m10 prints 10024, and nothing else fails. In
  every other row a re-entered handler is re-installed with the values it already had.
- **K3**, the body's continuation is its own frame again (old linkage, new copy): m3
  prints 20050, m8 99999, m10 5006. Runs return through the ORIGINAL handler frame.
- **K4**, no root for the current handler: the D3 guard's direct program dies with
  SIGSEGV. Every deep and corpus test fails too; the PARKED program stops with the new
  guard's words ("a continuation's handler chain does not reach its handler"), on a
  freed frame.

## 3. Review (2026-10-22)

An independent review found no high or medium finding. It ran three things against a
stress runtime that collects every N allocations, with freed blocks either poisoned
or reused at once:
- 2 525 random programs, mixing one-shot and multi, CPS and direct handles, tail and
  non-tail resumes, resumes in lambdas, aborting clauses, return clauses that perform,
  and schedulers;
- the 735-program sweep;
- the effect corpora.

None disagreed with the evaluator or crashed. Its three low notes, all fixed:
- **N1.** `copies_survive_a_collection_mid_copy` asserted only that some collection
  ran. The runtime now counts collections inside `elya_cont_copy`
  (`mid_copy=` in the `ELY_GC_STATS` line), and the test asserts at least one.
  It was red first ("the runtime reports no `mid_copy=`"); 6 of m13's 16
  collections fall inside a copy.
  The field is named `mid_copy=`, not `copy_collections=`: the first name broke
  `heap_values_live_across_cps_operands_survive_collections`, which reads
  `collections=0` as a substring of the line.
- **N2.** PARKED.md gave the whole test's time (0.47 s) as the program's (0.01 s).
- **N3.** rustfmt hung a comment off a trailing `// ccc`; the declaration was reshaped.
