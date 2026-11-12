# One collector from bare metal to large programs

**Status:** designed (2026-11-12, on `main`, an interactive session). HANDOFF step 1,
the maintainer's request: "make a gc that is able to use it in bare metal aswell
effienctly"; "this language covers from bare metal to python so make everything
more efficient and high quality". Design choices are Claude's under the
maintainer's delegation (rule 2).

**Governing constraint.** One collector serves both ends. A microcontroller with a
few tens of KiB and no operating system, and a program holding millions of objects
on a PC, run the same allocation, marking and sweeping code; they differ only in
where memory comes from. Every decision below is checked against both.

## 0. Measured first

`crates/codegen/src/runtime.c` at cf3f028: stop-the-world, non-moving mark-sweep.
Each object is `calloc`ed with a two-word prefix (`(size << 1) | mark`, and a link
on the list of every live block); free lists exist for sizes under 16 words, larger
blocks go back to `free`; the shadow stack and the mark worklist grow by `realloc`.
The runtime calls the C library about 170 times (`exit` 48, `fprintf` 32, `free`
24, `fputs` 23, `getenv` 5, `pthread_*`, `mmap`, `sigaction`, `clock_gettime`).
None of it exists on bare metal.

Release builds, best of three (programs in the scratchpad's `gcb/`):

| program | time | peak RSS | collections |
|---|---|---|---|
| `live_list`: a 2M-cell list built, then summed | 0.174 s | 95.4 MB | 7 |
| `churn`: 100K lists of 100 cells, each dropped | 0.227 s | 8.8 MB | 459 |
| `trees`: binary trees, one of depth 18 kept | 0.173 s | 42.6 MB | 8 |
| `strings`: short strings made and dropped | 0.183 s | 8.9 MB | 364 |
| `closures`: a closure made per step | 0.067 s | 8.9 MB | 137 |

A `Cons(Int, List)` is 3 visible words; with the prefix and `calloc`'s own header
it costs about 48 bytes, twice its payload: the `live_list` peak.

## 1. Decisions

**D1. Memory comes from one page source.** The collector asks for memory in
*chunks* (aligned to their size) and nothing else asks: no `malloc` in it. Two
sources, chosen at build time:
- hosted (the default): chunks from the operating system (`mmap`, `VirtualAlloc`),
  as many as the program needs: no ceiling but the machine's;
- freestanding (`ELYA_FREESTANDING`): one region, a static array of
  `ELYA_HEAP_BYTES` or the region a board's start-up code gives
  `elya_heap_init(base, bytes)`; when it is full and a collection frees nothing
  that fits, the platform's out-of-memory hook stops the program.

**D2. Pages of one size class.** A chunk is cut into pages (`ELYA_PAGE_BYTES`, 4 KiB
hosted; a small board builds with less). A page holds slots of one size: 31 classes
in words, 1 to 4096, each step at most a quarter over the last. The small ones (to
128 words) fill one page with at least four slots; the medium ones fill a *span* of
several pages with at least two (the review: a 5 KB string took two whole pages). An
object above 4096 words, or in a span larger than an eighth of the heap (a board),
takes a run of whole pages of its own. A page's (a span's) header holds its class,
its free list and its mark bitmap; the object has no header at all: its size is its
page's. A `Cons` costs its 3 words. A page's KIND (free, small, large, a later page)
and, for a span's or run's later page, the distance to its head live in the chunk's
side table, never in the page: a large object's later pages are its own memory (the
review's B1).

**D3. Finding a page.** An object's page is its address rounded down to the page
size; whether an address is the heap's at all is read from the chunk table (a small
sorted array of chunk bases). A pointer the heap does not hold (the static
`elya_handle_end` block) is never marked or freed, as its set mark bit made it
before.

**D4. Allocation.** Each class has a current page; allocating pops its free list.
An empty list takes the next page of the class with free slots, sweeping it first
if it has not been since the last collection (D6); else free pages, found next-fit
from each chunk's cursor in the side table (a freed page pulls the cursor back, so
freed memory is reused before untouched memory); else, after the other classes'
pages are swept (their empty ones freed), a new chunk (hosted) or a collection
(freestanding), else the out-of-memory hook. Slots are zeroed as handed out, the
contract the constructor code relies on.

**D5. Marking allocates nothing.** The worklist is a fixed array
(`ELYA_MARK_STACK` entries: 16384 hosted, 256 freestanding). An object's fields
are pushed last first, so a list's leaf is taken before its tail and the list
stays short (the review: a list of strings left one entry per cell). When it is
full, the object stays gray and its page goes on a rescan list; after the list
drains, the marked objects of those pages only are scanned again, until none is
left (the review: rescanning the whole heap at every overflow was quadratic). A deep
list never touches the host stack (the explicit worklist stays) and the collector
never needs memory to collect. The shadow stack keeps its doubling growth hosted;
freestanding, it is a fixed array whose overflow is the out-of-memory hook.

**D6. Lazy sweeping.** A collection marks, then flags every page unswept; a page is
swept (its free list rebuilt from its bitmap, its live words counted) when
allocation next needs a page of its class. A page found empty returns to the pool,
for any class. The pause is the mark; sweeping is spread over the allocations that
follow, which is what a real-time board needs and costs a large program nothing.

**D7. When to collect.** As now: when the words allocated since the last
collection reach the larger of the floor (`1 << 16`) and the last live count, so a
growing live set is marked O(log n) times. Freestanding, also when no page is free.
`ELYA_GC_STRESS=N` keeps its meaning: a collection every Nth allocation; under it
what a collection frees is poisoned and never handed out again, and graying one
stops the program ("a reachable value names a freed block").

**D8. The counters keep their names.** `ELY_GC_STATS` prints `collections`,
`freed`, `live`, `words_since_gc`, `mid_copy` as now; `live` is counted while
marking. `elya_cont_copy` reads an object's size from its page (`elya_gc_words`).

**D9. A platform layer, in two parts.** Part 1 (this spec's code): the collector
and its page source are freestanding: they compile with `-ffreestanding -nostdlib`
and reference only `elya_platform_*` hooks (`pages`, `oom`). Part 2 (a plan of its
own): the rest of the runtime's needs (bytes out, stop, the start thread, the
clock, signals) behind the same layer, and an Elya program run on an emulated
bare-metal board (QEMU) if it can be installed.

**D10. What the pages leave room for, not built here.** A nursery for young objects
(the high end: most short-lived values die before a collection), and freeing a
value at its last use where the affine guarantee proves it (the low end: data the
checker proves linear needs no tracing). Both reuse the page and class structure.

`gc_mark` changes by design: it no longer reads a prefix word.

**D11. The runtime, compiled once.** It was compiled unoptimised at every link. Now
`-O2`, once per process, into a directory the process creates (no other process or
user shares or plants the object; threads linking at once wait for one compile;
the review found a shared temporary name raced).

**Recorded limit.** A live set of many medium sizes at once rounds each to its class
(up to a quarter over) and strands part-used spans: the review's `big_live` (20K
live 5 KB strings, made by doubling) peaks at 226 MB where `malloc` took 146 MB, in
the same time. Every other program measured uses less memory than before.

## 2. Measured after (release, best of three; the review's three programs added)

| program | before | after |
|---|---|---|
| `live_list` | 0.174 s, 95.4 MB | 0.098 s, 50.7 MB |
| `churn` | 0.227 s, 8.8 MB | 0.166 s, 9.1 MB |
| `trees` | 0.173 s, 42.6 MB | 0.103 s, 25.6 MB |
| `strings` | 0.183 s, 8.9 MB | 0.144 s, 9.1 MB |
| `closures` | 0.067 s, 8.9 MB | 0.045 s, 9.1 MB |
| `str_list` (3M strings) | 0.764 s, 305 MB | 0.408 s, 149 MB |
| `mid_churn` | 0.178 s, 9.0 MB | 0.140 s, 9.1 MB |
| `big_live` | 0.171 s, 146 MB | 0.204 s, 226 MB |

## 3. Predictions (made before the code)

- `live_list` peak memory about half (3-word slots, no prefix, no `calloc` header);
  every program's time within its noise or better; the same collection counts where
  the trigger is unchanged.
- The whole codegen suite passes unchanged, and under `ELYA_GC_STRESS=1` as before.
- The collector's object file built freestanding has no undefined symbol but the
  `elya_platform_*` hooks.
