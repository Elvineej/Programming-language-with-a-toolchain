# The runtime behind a platform layer

**Status:** designed (2026-11-13, on `main`, an interactive session). HANDOFF step 1,
part 2 (the paged collector's D9). Design choices are Claude's under the
maintainer's delegation (rule 2). The maintainer asked that work be divided with
ChatGPT: three pieces here were written by ChatGPT (Codex, on the maintainer's PC)
to Claude's briefs and checked by Claude before they landed (section 3).

**Governing constraint**, unchanged: one runtime from a microcontroller to a PC.
After part 1 the collector needed no C library; the rest of `runtime.c` still
called it about 170 times.

## 0. Measured first

`runtime.c` at 8940baa: `exit` 44, `fprintf` 32, `free` 20, `fputs` 20, `fflush` 8,
`snprintf` 6, `fwrite` 5, `getenv` 5, `malloc`/`realloc`/`calloc`, `strtod`,
`strlen`, `atoi`/`atoll`, `memchr`, `isnan`/`isinf`/`signbit`, `fopen`/`fread`
(the trace), `clock_gettime`/`nanosleep`, `pthread_*`, `sigaction`, `mmap`. The
`@main` shim called `printf`. A Float's text was found by writing the double as
`%.780e` and reading candidates back with `strtod`, up to 34 times: 20 microseconds
a number.

## 1. Decisions

**D1. One list of hooks.** `runtime.c` and `gc.c` include `<stdint.h>`,
`<stddef.h>` and `<stdarg.h>` and reach the machine through `elya_platform_*`:
`write(stream, bytes, n)` (1 the program's output, 2 its failures), `flush`,
`exit(code)`, `init`, `getenv`, `scratch(p, bytes)` (working memory, `realloc`'s
contract), `now_ms`, `sleep_ms`, the trace file (`trace_create`, `trace_append`,
`file_open`, `file_read`, `file_close`, `error`), and part 1's `pages` and
`release`. `elya_platform_fail` is the runtime's own (flush, the line, status 1).

**D2. Hosted is one file.** `platform_hosted.c` (included at the end of
`runtime.c` unless `ELYA_FREESTANDING`): the hooks over the C library, and the
program's own stack, guard page and signal handlers, moved whole. What a hosted
program writes, and when, is unchanged.

**D3. A board gives two functions.** With `ELYA_FREESTANDING`, `write` and `exit`
are the board's; every other hook has a weak default it may replace: no settings,
a clock that stands still, no files, working memory from the heap (D5).
`elya_start` runs the program on the stack the board's start-up code set. What is
left undefined is those two, `memcpy`/`memset`/`memcmp`, and the compiler's
arithmetic helpers on 32-bit machines (`__aeabi_*`, `__*di3`, soft floats:
compiler-rt's builtins).

**D4. Text without a C library.** The runtime's own formatter (`%s`, `%.*s`,
`%d`, `%lld`, `%llu`) writes to a stream or a buffer; `@main` calls
`elya_main_int` where it called `printf` (the IR declares no C library function).

**D5. One memory.** A board has a region and nothing else, so the runtime's
working lists and texts (`==`'s worklist, `with any`'s writer and parser, a
continuation's copy table) come from the collector's heap: `elya_gc_scratch`, a
block `[-1][prev][next][bytes]` on a list of roots while held, its bytes never
scanned. Allocating one never collects (the caller's values may be on no root);
freed, a block with pages of its own returns them at once, a small one is the
collector's. Hosted keeps `realloc` (`==` on data must not bring collections
nearer). A board's first 1024 roots are a fixed array (`ELYA_SHADOW_ROOTS`).

**D6. Float text in the runtime.** `float_fmt.c`: the shortest digits that read
back (Ryu's method, 128-bit products from 64-bit halves), written as Rust's `{}`
writes them; `elya_float_text` adds `.0` where no point is. `float_parse.c`: the
nearest double of a decimal text (Eisel-Lemire, then an exact comparison on at
most 800 digits). Both freestanding, no state, 8 KiB and 13 KiB of code. The
`%.780e` and `strtod` round trips are gone on every platform: one algorithm, the
same bytes.

## 2. Recorded limits

- A working block needs pages free when it is asked for (D5): a board whose
  region is full of garbage fails `with any` on a large value where a collection
  would have made room. Rooting the callers' values would lift it.
- Growing a board's roots past the fixed array from a page source is written and
  not tested (no board here has one).
- Record and replay on a board need the board's file hooks; the defaults refuse.
- No program has yet run on a board's own processor: the board tests run on the
  test machine's (x86-64, no C library, its own `_start`). Cross-target code
  generation is part 3; the start-up code and linker scripts for a Cortex-M3 and
  a RISC-V board (QEMU) are written and run a C `entry` (section 3).

## 3. Written by ChatGPT, checked here

| piece | its own checks | checked here before landing |
|---|---|---|
| `float_fmt.c` | 10M round trips, Rust oracle | 20.2M doubles (every exponent's edges, integers, random bits) against Rust's `{}`; 3.6M against the text the runtime wrote before, byte for byte; ASan/UBSan |
| `float_parse.c` | 10M against `strtod`; parse-number-fxx, 5.3M rows | 3.1M texts (random, exact midpoints of adjacent doubles and their neighbours, 800-digit numbers, six corpus files) against Rust's `from_str`; the grammar's rejections; ASan/UBSan |
| QEMU boards (Cortex-M3 `lm3s6965evb`, RISC-V `virt`) | built, not run (no QEMU on the PC) | run under QEMU 8.2.2: `hello`, status 3, both boards. Not in the repository yet (part 3 uses them) |

One change to its code: `powers` in `float_parse.c` renamed `parse_powers` (both
files are one translation unit here).

## 4. Predictions (made before the gate)

- No existing test's expectation changes.
- Gate: 1504 + 5 (`native_platform`) + 1 (`native_gc_pages`) = **1510 passed, 159
  suites**.
- Float text faster by an order of magnitude; the benchmarks of part 1 within
  noise.

## 5. Measured after

- Gate (Linux): 1510 passed, 159 suites, as predicted. No existing expectation
  changed.
- 300,000 `float.to_string` calls: 2.36 s before, 0.041 s after (57 times).
- Part 1's benchmarks against 8940baa on the same machine, interleaved: within
  noise (`churn` 0.21-0.25 s both, `mid_churn` 0.13-0.17 s against 0.145, `trees`
  0.127 against 0.127, `live_list` 0.099 against 0.111).
- The whole runtime freestanding at `-O2`: 52 KiB of code for x86-64, 60 KiB for
  a Cortex-M4, 52 KiB for RISC-V 32 (before the linker drops what a program does
  not call).
- **Found, not done here:** `compile_module` emits at `OptimizationLevel::None`
  and runs no LLVM pass at all. HANDOFF step 2 now starts there.
