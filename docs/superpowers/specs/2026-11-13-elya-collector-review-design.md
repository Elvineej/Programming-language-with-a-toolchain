# The collector after ChatGPT's review

**Status:** done (2026-11-13, on `main`, an interactive session). The maintainer
asked: "check if there was a better way to do the things we did until now", and
that ChatGPT be given work and its work be checked. ChatGPT (on the web, reading
the public repository at 12f91bd) reviewed the collector, the runtime and the back
end: 15 findings. Each was checked here before anything was changed; this is what
held, what was done, and what is recorded for later. Design choices are Claude's
(rule 2).

## 1. Checked and fixed here

**F1. An object just under a chunk's size failed "out of memory".** (Its finding 1;
confirmed: `elya_alloc(8386026)` on an empty hosted heap printed `elya: out of
memory`, as did 16,772,000 and 33,550,000 words.) `gc_grow` asked for `at_least`
plus two pages; a chunk loses two table bytes a page and the first page's
alignment. Now the chunk is grown until it holds the pages asked.

**F2. Values of more than 64 words.** (Its finding 14, "suspected"; confirmed
here.) A descriptor row's pointer mask was one word. A constructor's 65th field, a
closure's 64th capture or the 63rd value live across a call shifted past it: in a
debug build the compiler panicked; in a release build the shift wrapped, and the
collector's `mask >> field` (undefined in C) wrapped the same way on x86-64, so
the field was traced and an `Int` 64 places before it was read as a pointer. A
row of more than 64 words now carries, in the mask's place, where its mask words
begin after the table (`push_row`, lib.rs; `gc_field_is_heap`, gc.c); `==`'s
float masks likewise. No limit remains.

**F3. Medium size classes.** (Its finding 5; it measured the first table at 29%
over an object's size on average and 99% at worst: a 5 KB string took 8 KiB.) The
classes above 128 words are now made for the page size in use
(`gc_make_classes`): each fills a span of at most eight pages exactly, an eighth
or more above the last. At 4 KiB pages: 23 classes to 3828 words, 9% over on
average, 32% at worst. `big_live`: 226 MB, 0.204 s before; 197 MB, 0.160 s after
(the earlier allocator's 146 MB is not reached: see section 3).

**F4. A working block was counted toward the next collection** (its finding 7:
1 MiB asked and freed made the next one-word allocation collect). Not counted.

**F5. A page smaller than its header** (its finding 11: `ELYA_PAGE_BYTES=64`
wrapped the span arithmetic and wrote past the page). Refused at compile time, as
is a page size that is no power of two.

Its finding 2 (freed working blocks were garbage) had been found by the
independent review and fixed in 6caabff, pushed while it read.

## 2. Tests

`native_wide.rs` (new, 4): constructors of 64, 65, 70, 130 and 200 fields with
lists on both sides of each mask word; closures of 63 to 150 captures; 62 to 150
values live across a call and a perform; `==` on a 70-field constructor with a
list and a Float past the 64th field. Each plainly and under `ELYA_GC_STRESS=1`.
`native_gc_pages.rs` +3: objects just under chunks of 1 to 64 MiB; a working block
brings no collection nearer; the medium classes fill their spans (worst and mean
bounded at 4 KiB, 1 KiB and 512-byte pages).

Controls (each reverted, compared byte for byte), each failing its tests: W1 a
wide row read as one word; W2 the second mask word never read; W3 a wide float
mask read as one word; W4 bits past a word set in the first; W5 chunk growth by
the asked bytes alone; W6 working blocks counted.

## 3. Checked and recorded, not done here

- **No LLVM pass is run** (its finding 4; HANDOFF step 2 already starts there).
  Measured here with `default<O2>`: `fib(35)` and tight loops 10% to 35% faster,
  the allocation benchmarks unchanged, one stack test's expectation moving.
- **Lazy sweeping does not bound a pause** (3): a second collection sweeps what
  the first left (`gc_finish_sweeping` at the start of `gc_collect`). True; the
  spec's "the pause is the mark" holds only between collections. A sweep cursor
  with marks kept by epoch would bound it.
- **Freed pages are not returned to the host** (8): `gc_release_pages` frees
  them for reuse only. A `discard` hook (`madvise`, `MEM_RESET`) would cut the
  resident set after a burst; `big_live`'s remaining gap is partly this.
- **The shadow stack roots a block's values again at every call** (6): root
  frames per function first, stack maps much later.
- **An overflowing mark stack rescans a page's marked objects, not only its gray
  ones** (9): a pending bitmap would bound it.
- **A run's later pages past 255 find their head only from the object's start**
  (15): no Elya value points inside an object; the invariant is now this line.
- **A nursery; freeing at the last use the affine guarantee proves** (12, 13):
  after the above, measured first (survival rates).
- **Cross-target code** (10): part 3, next.

## 4. Predictions

Gate: 1520 + 4 (`native_wide`) + 3 (`native_gc_pages`) = **1527 passed, 162
suites**. No existing test's expectation changes.
