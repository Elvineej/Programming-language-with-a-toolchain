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
freed, it is free memory at once (section 6, B1): a run's pages go back, a slot
goes on its page's free list and its page allocates next if it was swept since
the last collection. Grown within its slot, it stays. Hosted keeps `realloc`
(`==` on data must not bring collections nearer). `==` keeps its first 32
pairs on the host stack, so most comparisons ask for none. A board's first 1024
roots are a fixed array (`ELYA_SHADOW_ROOTS`); past it they grow in the heap, a
held working block (section 6, R2).

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

## 6. The independent review (after 8d08c11), fixed test-first

The review compared 51 programs native against the old runtime and the
evaluator (stdout, stderr, status, merged order, plain and under stress), record
and replay on nine traces, the working-memory allocator under a random model
(ASan/UBSan, a 32-bit build), the freestanding build for six targets, and 3.6M
floats: no difference there. It found:

- **B1 (bug):** on a board, the runtime's freed working blocks were garbage only
  a collection could free, and no collection runs under one: 250 `==` on
  two-field values filled 256 KiB ("out of memory"), as did 300 `with any`
  performs; a 40,000-cell search by `==` needed 64 MiB. Fixed (D5): a freed
  block is free memory at once; `==`'s first 32 pairs on the host stack.
- **R2 (risk):** a board giving no page source had 1024 roots in all (depth 600
  failed in an empty 4 MiB region). Fixed (D5): the roots grow in the heap.
- **N3:** `rt_atoll` did not skip leading white space as `atoll` did
  (`ELYA_GC_STRESS=' 1'` turned stress off). Fixed.
- **N4:** a failure's line was several writes, which another process's line on
  a shared stderr could split. Fixed: one write when it fits 512 bytes.
- Notes taken: `trace_line`'s length clamped to its buffer; two stale comments;
  `elya_stack_unavailable` marked `noreturn` (a warning at `-Wall`). The order
  of the hosted headers (after the runtime's own names) was suspected for
  Windows and macOS; the runtime builds for Windows with MinGW's GCC and
  Clang's `x86_64-w64-windows-gnu` here, and the Windows gate builds it with
  MSVC's headers.

Tests: `native_platform.rs` +4 (working memory on a board is not garbage: 10,000
`==`, 2,000 on 300-cell lists equal and unequal, a 40,000-cell search in 4 MiB,
3,000 `with any` performs, each against the hosted build; the roots grow in the
region; what grown roots hold survives collections; settings read as the C
library reads them); `a_board_out_of_roots_fails_by_name` now asks for more
roots than a 64 KiB region holds (its expectation changed: the fixed array
alone no longer runs out). `native_gc_pages.rs`: the working block is grown
where it was regrown (a block grown within its slot stays).

Controls (each reverted, compared byte for byte), each failing its tests: Q1 a
freed slot not back on its page; Q2 an unswept page made current; Q3 the fixed
roots not copied (survived until the grown roots were made to hold across
collections); Q4 the roots' growth not named; Q5 a large block not given back;
Q6 `==`'s first array not copied out (survived until lists unequal in their
first element); Q7 a block kept in place when its slot is too small; Q8 white
space not skipped (survived until the settings test).

Gate predicted: 1516 + 4 = **1520 passed, 161 suites**; measured 1520, 161.

The Windows gate, green again in the front end (12f91bd), then reached the
collector's tests for the first time: `a_live_set_larger_than_the_region_fails_by_name`
compared a line the test's own C harness writes in text mode (`\r\n`). The test
now compares the line, not its ending.
