# Plan: one collector from bare metal to large programs, part 1

Spec: `docs/superpowers/specs/2026-11-12-elya-paged-collector-design.md` (D1-D10).
Base: `main` at 9d76069.

## Steps

1. Measure (spec section 0): five allocation-heavy programs, release, best of three.
2. `crates/codegen/src/gc.c` (new): chunks from one page source; pages of one size
   class with mark bitmaps and no object header; lazy sweeping (an empty page back to
   the pool); a fixed mark worklist with an overflow rescan; the shadow stack through
   the page source; stress mode's poisoned, kept slots by a bitmap. Freestanding:
   `<stdint.h>`, `<stddef.h>` and the `elya_platform_*` hooks.
3. `runtime.c`: includes `gc.c`; the hosted hooks (`mmap` / `VirtualAlloc`); the
   static `elya_handle_end` block outside the heap; `elya_cont_copy` reads sizes by
   `elya_gc_words`.
4. Found measuring: the runtime was compiled unoptimised at every link. Now `-O2`,
   once per content of `runtime.c` and `gc.c`, the object kept in the temporary
   directory (`runtime_object`, `crates/codegen/src/lib.rs`).
5. Found measuring: a fresh page rescanned every page handed out (quadratic on a
   large live set); a single page now comes from the free list or the untouched end.
6. Tests: `crates/codegen/tests/native_gc_pages.rs` (new, 5; 7 with the review's): a board's platform (no
   memory but a fixed region) reusing it across collections, a static block grayed
   with the memory around it untouched; dead slots among live ones reused; a 2-entry
   mark worklist rescanned; a live set larger than the region failing by name; the
   collector built `-ffreestanding -nostdlib -Werror` for x86-64, Cortex-M4 and
   RISC-V 32, needing only the hooks.
7. Controls (each reverted, compared byte for byte): G1 no rescan; G2 a sweep reusing
   nothing (survived the first four tests: dead slots among live ones added); G3 marks
   never cleared; G4 collecting before growing (`ordinary_programs_finish_without_
   collecting`); G5 the page lookup without the chunk table (the first G5, a static
   pointer traced, was no control: its tag stops the scan; the canary added).
8. An independent review: a large run's later pages taken while it lived, one-page
   objects never reusing freed pages, quadratic rescans and run searches, sweeping
   only the allocating class, 230 KB of static tables, 32-bit size overflows, the
   runtime cache racing. Fixed test-first: kinds in a side table, medium classes in
   spans, rescans by page, fields pushed last first, next fit from freed pages,
   small tables freestanding, the runtime once per process (controls G6-G8; tests
   +2).

## Predictions

- Gate: 1497 + 5 (codegen, counted once) = **1502 passed, 158 suites** (gated);
  with the review's 2, **1504**.
- No existing test's expectation changes; `gc_mark` changes by design.
- Every benchmark faster or within noise; `live_list` and `trees` half the memory.

## Part 2 (its own plan)

The rest of the runtime behind the platform layer (bytes out, stop, the start
thread, the clock, signals); a freestanding runtime build; an Elya program on an
emulated board (QEMU), if it installs.
