# Elya on a board

**Status:** done (2026-11-14, on `main`, an interactive session). HANDOFF step 1,
part 3 (the paged collector's D9): code for a processor other than the host, and
an Elya program run on a board. Design choices are Claude's (rule 2). The
maintainer asked that work be divided with ChatGPT: the board's start-up, linker
script, semihosting and memory functions began as ChatGPT's (section 4).

**Governing constraint**, unchanged: one runtime and one compiler from a
microcontroller to a PC.

## 0. Measured first

Nine programs (a counting loop, `fib`, trial division, n-queens on lists, binary
trees, a merge sort of 2,000, a list of closures, a state handler, a failure)
compiled for a Cortex-M3 and run on QEMU, after the obvious steps (the module
given the machine's triple and data layout, static code, the runtime built
freestanding for the same processor):

- **Every object failed to link.** The `perform` clause table was an array of
  `ptrtoint` constants widened to `i64`, which a 32-bit ELF cannot relocate.
  `libgcc`'s unwinder was pulled in (and with it `abort`) because no Elya
  function said it does not unwind. A compiler for ARM calls `__aeabi_memclr8`
  and its kin where another calls `memset`.
- **Then three of nine stopped with a hard fault at pc 0** (`queens`, `closures`,
  and `sort` once it was given room), at O2 and at O0 alike.

The hard fault is LLVM's. Under `tailcc` a callee pops its own stacked
arguments. `ARMISelLowering.cpp` (LLVM 18, and still on its main branch) has the
callee pop them rounded up to the data layout's stack alignment, 8 bytes under
AAPCS (`LowerFormalArguments`: `StackArgSize = alignTo(StackArgSize,
DL.getStackAlignment())`), and a caller making an ordinary call expect them
popped rounded up to 16 (`LowerCall`: `CalleePopBytes = canGuaranteeTCO(..) ?
alignTo(NumBytes, 16)`). `many(fs: Fs, i: Int, acc: Int)` stacks 8 bytes; its
caller does `bl elya.many; sub sp, #16; add sp, #8; pop {r4, r5, r7, pc}`, and
`elya.many` ends `add sp, #8; bx lr`: the `pop` loads 0 into pc.

LLVM's own reproducer, eleven lines (`llc-18 -mcpu=cortex-m3 -O2`):

```llvm
target triple = "thumbv7m-none-eabi"
define tailcc i64 @callee(ptr %p, i64 %a, i64 %b) {
  %s = add i64 %a, %b
  ret i64 %s
}
define i64 @caller(ptr %p) {
  %r = call tailcc i64 @callee(ptr %p, i64 1, i64 2)
  ret i64 %r
}
```

`callee` ends `add sp, #8; bx lr`; `caller` follows `bl callee` with
`sub sp, #16; add sp, #8; pop {r7, pc}`.

## 1. Decisions

**D1. A machine to compile for.** `Machine { triple, cpu, features }`:
`Machine::host()` (what `compile_module` uses, unchanged) and
`Machine::cortex_m3()` (`thumbv7m-none-eabi`, `cortex-m3`, AAPCS soft float).
`compile_module_for(core, obj, optimise, &machine)` gives the module the
machine's triple and data layout before any pass reads them, builds static code
for a board, and marks every Elya function `nounwind` on a board (none unwinds: a
failure stops the program; a host keeps the unwind tables its debuggers and
Windows' stack walks read). A machine other than the host or a 32-bit ARM is
refused by name. The ARM back end is linked only with the `boards` feature of
`elya-codegen`: the maintainer's vcpkg LLVM has the x86 back end only, and a
reference to another is an unresolved symbol at link time.

**D2. The clause table is an array of pointers.** It was an array of `i64`
constants (`ptrtoint`), with 0 where a handle has no clause; it is an array of
`ptr`, with null. Every target, the host's included, reads it the same way.

**D3. A board is four files** (`crates/codegen/boards/`): `cortex-m3/startup.s`
(the vector table; `.data` copied, `.bss` zeroed; the program on the process
stack, faults on the main stack), `cortex-m3/link.ld` (QEMU's `mps2-an385`:
4 MiB of code memory, 4 MiB of RAM), `cortex-m3/board.c` (bytes out and stop by
Arm semihosting, the stack's guard, faults by name, the collector's region, the
ARM EABI's memory helpers) and `common/mem.c` (`memcpy`, `memset`, `memcmp`,
`memmove`, a word at a time, weak). RAM from its start: a 4 KiB guard the memory
protection unit forbids, the program's stack (1 MiB, or
`--defsym ELYA_STACK_BYTES=n`), the static data, a 1 KiB fault stack, then the
collector's region to the end of RAM (or `--defsym ELYA_HEAP_BYTES=n` of it). A
recursion deeper than the stack reaches the guard and stops with the hosted
build's line and status: `elya: native stack overflow: recursion deeper than the
1024 KiB stack`, status 1, after the output already written. With the stack above
the heap, as the first layout had it, a recursion past the stack's end went on
into the heap: one test passed by luck (3,000 frames at O0 need more than the
256 KiB that layout gave), and a deeper one ended in a fault that could not even
be reported. Any other fault is `elya: hard fault at pc ..` with the
fault registers, status 70. The program links with `ld.lld` and `libgcc` for
64-bit division and soft floats; no C library.

**D4. No call LLVM's ARM back end disagrees about.** After the optimiser, every
`tailcc` function and every `tailcc` call whose stacked arguments end 1 to 8
bytes past a multiple of 16 (AAPCS placement: a word in the next of `r0`-`r3` or
4 bytes of stack; an `i64` in `r0:r1` or `r2:r3` or 8 bytes at an 8-byte offset;
once one argument is stacked, all later ones are) gets one more `i64` parameter,
passed as `poison` (`arm_tail.rs`). Being last and stacked, it takes the area to
the next multiple of 16, where both roundings agree. Running last, it is covered
for every function and call however made, and no pass sees the unused parameter
to remove it. A host never runs it.

**D5. RISC-V is recorded, not built.** LLVM 18's RISC-V back end has no `tailcc`
at all: `LowerFormalArguments` ends `report_fatal_error("Unsupported calling
convention")` for any convention but C, Fast and GHC, and so does main. Without
it, `musttail` needs the caller's and callee's types to match and no stacked
argument, which Elya's calls do not promise. The runtime already builds
freestanding for RV32 (part 2); Elya code for it needs its own calling
convention (a fixed signature for every transfer, or a dispatch loop), a design
of its own. Until then the machine is refused by name rather than by LLVM ending
the process.

## 2. Tests

`native_boards.rs` (new, 4, run by the gate's configuration C where
`qemu-system-arm` and `arm-none-eabi-gcc` are installed; the runtime, start-up,
board and memory functions built once with `clang`, linked with `ld.lld`):

- the nine programs write on the Cortex-M3 what they write hosted, status
  included, at O2 and at O0;
- every call shape: functions whose arguments stack 4, 8, 12, 16 and 24 bytes,
  words, pointers and Bools in each position, each calling itself in tail
  position and called by an ordinary call; a ring of tail calls through five
  shapes (stacked bytes 16, 32, 32, 16 and 16 after padding, so the area grows
  and shrinks between transfers); ordinary recursion 3,000 deep through stacked
  arguments; a four-parameter closure called both ways;
- a recursion a million deep stops with the stack overflow line, status 1, after
  `before` was written;
- a RISC-V machine is refused by name.

`arm_tail.rs` +1 (unit): the placement, case by case.

**Controls** (each reverted, compared byte for byte), each failing its tests:
C1 no padding (`queens` and the call shapes: hard fault at pc 0); C2 the
placement not skipping an odd register (the call shapes fault; the unit test
fails); C3 RISC-V let through (refused by LLVM's "no available targets", not by
name); C4 no guard (the overflow is a hard fault with no readable pc).

## 3. ChatGPT's research, checked

Asked, before the faults were found, whether LLVM 18's ARM and RISC-V back ends
support `musttail` with `tailcc`, it answered (17 minutes, High) that ARM does,
stack arguments included, and that RISC-V does when no argument is stacked.
Checked against the source and QEMU: ARM's support has the rounding bug above,
which it did not see; and RISC-V rejects `tailcc` itself, before any tail call is
considered. It found one thing worth keeping: the runtime turns pointers into
words with `(int64_t)(intptr_t)p`, which sign-extends on a 32-bit machine, where
LLVM's `ptrtoint` zero-extends; harmless on this board (RAM below 2 GiB), it is
fixed on its own (HANDOFF).

## 4. What ChatGPT wrote

The start-up, the linker script and the semihosting (CC0, written to Claude's
brief for QEMU's `lm3s6965evb`) and the memory functions (CC0, tested by it on
220,211 cases against the C library, and here on 337,881 under the address and
undefined-behaviour sanitisers, then built for Thumb and RV32 with nothing
undefined). Changed here: the board moved to `mps2-an385` (64 KiB of RAM did not
hold `sort`); the semihosting handles opened once; the guard, the two stacks and
the fault lines; the ARM EABI helpers.

## 5. Predictions

Gate: configurations A and B, 1530 + 1 = **1531 passed, 164 suites** (the
`native_boards` binary is empty without the feature); configuration C,
**4 passed, 1 suite**. No existing expectation changes.
