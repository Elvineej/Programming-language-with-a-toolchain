# Plan: the runtime behind a platform layer (the collector, part 2)

Spec: `docs/superpowers/specs/2026-11-13-elya-platform-layer-design.md` (D1-D6).
Base: `main` at 8940baa.

## Steps

1. Measure the C library's uses in `runtime.c` (spec section 0).
2. `runtime.c`: the hooks declared, the formatter, every call through them;
   `platform_hosted.c` (new): the hosted hooks and the start machinery, moved.
3. `lib.rs`: `@main` calls `elya_main_int`; no `printf`, no format global.
4. Tests first found two gaps: a board had no working memory (`with any`'s writer
   failed "out of memory" printing a String) and no roots. `gc.c`:
   `elya_gc_scratch`, the fixed first roots.
5. In parallel, ChatGPT: the float printer, the float parser, two QEMU boards.
   Each checked here (spec section 3), then `float_fmt.c`, `float_parse.c` in.
6. Tests: `crates/codegen/tests/native_platform.rs` (new, 5): the runtime built
   freestanding for x86-64, Cortex-M4 and RISC-V 32 with only a board's symbols
   undefined; eight programs (Ints, strings, lists under collection in 256 KiB,
   handlers, Floats, data) linked with a board and no C library writing what the
   hosted build writes; a failure's line and status; out of memory by name; out
   of roots by name. `native_gc_pages.rs` +1: working memory held across
   collections, grown, never collecting, given back.
7. Controls (each reverted, compared byte for byte), each failing its tests: P1
   no minus sign; P2 working blocks not roots; P3 a grown block not copied; P4
   large blocks not given back (survived until six regrows with no collection
   between were added); P5 a working block collects; P6 no `.0` appended; P7 a
   trailing point kept; P8 a failure's status 0; P9 a board with no working
   memory.

## Predictions

- Gate: **1510 passed, 159 suites**. No existing expectation changes.

## Part 3 (its own plan)

Code generation for a target triple; an Elya program on the Cortex-M3 and RISC-V
boards under QEMU, in the gate where QEMU is installed.
