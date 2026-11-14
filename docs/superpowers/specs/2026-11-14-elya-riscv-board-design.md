# Elya on a RISC-V board

**Status:** done (2026-11-14, on `main`, an interactive session). HANDOFF step 1,
part 3's second board (spec 2026-11-13-elya-boards D5 had refused RISC-V by
name). Design choices are Claude's (rule 2). The board's files began as
ChatGPT's (section 4).

## 0. Measured first

LLVM 18's RISC-V back end has no `tailcc`: `LowerFormalArguments` accepts the C,
Fast and GHC conventions and ends anything else with `report_fatal_error(
"Unsupported calling convention")` (`RISCVISelLowering.cpp`, release/18.x; the
same on main). GHC needs the F and D extensions (an `rv32imac` has neither).
Every Elya function and call is `tailcc`, and every tail call `musttail`. For
the conventions RISC-V has, the IR verifier lets a `musttail` call through only
when the caller's and callee's types are the same (it exempts `tailcc` and
`swifttailcc` alone), and the back end makes it a tail call only when no
argument is stacked (`isEligibleForTailCallOptimization`: "Do not tail call opt
if the stack is used to pass parameters"), or ends the process ("failed to
perform tail call elimination on a call site marked musttail").

Elya's signatures on a 32-bit machine: at most five parameters and an effectful
function's continuation (a lambda: its closure, four parameters and the
continuation), and a clause of an operation of `n` parameters takes `n` and two
pointers. An `Int` is two 32-bit words, a pointer or a Bool one: eleven words
at most for a function, `n` Ints and two for a clause.

## 1. Decisions

**D1. One function type on RISC-V.** After the optimiser (`rv_tail.rs`), every
`tailcc` function becomes a `fastcc` function of one type, twelve 32-bit words
in and one 64-bit word out, and every `tailcc` call a `fastcc` call of that
type. A parameter takes one word (a pointer by `ptrtoint`, a Bool zero-extended)
or two (an Int, low half first), in order; the words left over are `poison`. The
result is a pointer by `ptrtoint`, a Bool zero-extended, an Int as it is. A
`musttail` call's `ret` returns the word itself, nothing between them. One type
makes every `musttail` valid IR; `fastcc` passes integers in thirteen registers
in LLVM 18 (`a0`-`a7`, `t2`-`t6`: `getFastCCArgGPRs`) and in twelve on LLVM's
main branch (it gives `t2` up for Zicfilp, ChatGPT found), so twelve words are
never stacked by either, and the back end makes every `musttail` a tail call.
An indirect tail call's target needs one register more of the class a tail call
may use (`GPRTC`: `t1`, `t2`, `a0`-`a7`, `t3`-`t6`, fourteen): twelve arguments
leave two. Twelve words hold every Elya signature: five Ints and a continuation
(eleven), a lambda's closure, four Ints and a continuation (ten), a clause of an
operation of five Ints (twelve). A function or call needing more (a clause of
an operation of six Ints) is refused by name. Running last, like the ARM
padding, it covers every function and call however made, and the optimiser saw
the real types. A first version took six 64-bit words (twelve registers, a
clause of four Ints at most); a second thirteen 32-bit ones, until LLVM main's
twelve were found.

Considered: a dispatch loop (every transfer returns to a trampoline: a call and a
return where a jump was, and the frames move to the heap), arguments past the
registers in a global area (memory traffic on every call with many arguments,
and a contract every function's entry must keep), and waiting for LLVM. The one
type costs moves between registers at calls that pass fewer words (none for the
`poison` ones) and a shift to join an Int's halves, and nothing else.

**D2. The board** (`crates/codegen/boards/riscv32/`): QEMU's `virt`, machine
mode, 128 MiB of RAM at 0x80000000, the ELF loaded into RAM (so `.data` is not
copied). From the start of RAM: the code, a 4 KiB guard, the program's stack
(1 MiB, or `--defsym ELYA_STACK_BYTES=n`), the static data, a 1 KiB trap stack,
then the collector's region. The guard is a locked PMP entry with no access
(machine mode obeys only locked entries), with a lowest-priority entry allowing
the rest. A trap switches to its own stack and says what it was: an access to
the guard is `elya: native stack overflow: recursion deeper than the 1024 KiB
stack`, status 1, the Cortex-M3's line; anything else `elya: trap mcause ..` with
the pc and the faulting address, status 70. Bytes out and stop by RISC-V
semihosting (the Arm operations behind `slli x0, x0, 0x1f; ebreak; srai x0, x0,
7`).

**D3. A pointer is one word everywhere.** The runtime wrote pointers into words
as `(int64_t)(intptr_t)p`, which sign-extends on a 32-bit machine, where LLVM's
`ptrtoint` zero-extends: below 2 GiB the same, above it (all of `virt`'s RAM) one
pointer was two different words, one written by the runtime, one by the
program. No word is compared with another today (checked: the multi-shot
search, whose continuation copies the runtime links, writes the same on the board
with either), so nothing failed; `ELYA_WORD(p)` (`(int64_t)(uintptr_t)p`) makes
it one word before something does. ChatGPT's finding (boards spec section 3).

**D4. The review of 52bf757** (ChatGPT, asked to review the Cortex-M3 commit;
checked here, each finding real):
- `check.sh` passed with configuration C skipped where the tools were missing:
  now a missing tool fails the gate, and `ELYA_BOARDS=skip` skips it on purpose,
  saying so;
- the tests read a program's output only after it ended, so one writing more
  than a pipe holds would have stopped (a false timeout): the output is read
  while it runs, and a program writing 130 KiB is among the tests;
- the Cortex-M3's vector table had room for 44 interrupts, the AN385 has 45:
  48 now (none is enabled; QEMU models 32);
- the tests it asked for: clauses of two and three Ints (their calls are padded
  on the Cortex-M3, and their entries live in the clause table, a constant),
  and the padding at the IR level (a function, a declaration, direct and
  indirect `musttail` calls, an ordinary call, a constant holding the function,
  a parameter's attribute).

**D5. A fix the Windows gate needed** (found looking at it after 52bf757, red
since fe2639d): `native_optimise`'s proof that the passes ran was that the
optimised object is the smaller. On Windows `sum`'s is the larger (2,038 bytes
against 1,986: the loop is inlined into `main` and `elya_entry` as well, and
COFF's unwind tables grow with it). The proof is now read from the IR the object
is made from (`emit_ir_for`, new): `remainder` is inlined and the tail call to
itself is a loop, optimised; both are calls as written. Changed expectation
(named): `the_optimised_object_is_the_smaller` is
`the_passes_inline_and_loop`. Committed on its own before this work.

**D6. A clock on each board.** The examples run on both boards, and the
timers example's second half sleeps on the real clock: with the runtime's
default, a clock that stands still, it said "too soon". The Cortex-M3's clock is
the MPS2 FPGA's 100 Hz counter (`time.now_ms` in steps of 10 ms), RISC-V's the
CLINT's `mtime` (10 MHz on `virt`, read high, low, high); `time.sleep_ms`
waits on each. And QEMU runs without `-nographic`, which makes its stdout
non-blocking: the 130 KiB program's writes failed on a full pipe (status 70)
on RISC-V.

## 2. Tests

`native_boards.rs` runs every test on both boards (the Cortex-M3 and RISC-V), at
O2 and O0, against the hosted build: the nine programs and two the review asked
for (D4), every call shape, the stack overflow by name, and (new) the ten
examples (async, structured concurrency, timers on the virtual and the real
clock, the multi-shot search, replay, `with any` recording, failure as an
effect, the CEK machine). A machine other than these is refused by name.
`rv_tail.rs` +1 (unit): twelve words fit and thirteen are refused by name.

**Controls** (each reverted, compared byte for byte), each failing its tests:
R1 an Int's halves in the other order (the call shapes on RISC-V never end:
the 300 s timeout); R2 `ccc` for `fastcc` (LLVM ends the process: "failed to
perform tail call elimination on a call site marked musttail": eight argument
registers are too few); R3 no PMP guard (the overflow runs on into the code
below the guard: an illegal-instruction trap, its own line garbled); R4 a
`musttail` call's result converted before its `ret` (the verifier: "musttail
call must precede a ret with an optional bitcast", at `prime`, a Bool); R5
QEMU with `-nographic` (D4: the 130 KiB program stops at status 70 on RISC-V);
R6 no clock on the boards (D6: the timers example's real half says "too soon").

## 3. What it leaves

The Cortex-M3's padding (boards D4) and this pass are two answers to one
problem; a 32-bit ARM could take the one type too, if `fastcc` on ARM passed
enough in registers (it passes four words, so it cannot). Floats on a board are
soft; a board with an FPU (Cortex-M4F, `rv32imafc`) is untried. A frame
larger than the 4 KiB guard could step over it (into nothing on the Cortex-M3,
into the code on RISC-V: R3 shows what that does); a locked PMP entry making
the code read-only would close RISC-V's half.

## 4. What ChatGPT wrote

The board's three files (CC0, to Claude's brief, with the Cortex-M3's as the
model). It built them with clang 17 and checked the layout and the semihosting
opcodes; it could not run QEMU. Checked here: built with clang 18 (`-Wall
-Wextra -Werror`), every test run on QEMU 8.2's `virt`, the guard proved by
control R3, the PMP encodings by the RISC-V privileged specification (a locked
NAPOT entry of nine trailing ones is 4 KiB; machine mode obeys locked entries
only). Changed here: a comment after an instruction (LLVM's RISC-V assembler
refuses `/* */` there), the comments. Also ChatGPT's: the review (D4), and
research confirming the ARM bug (its reproducer, run with clang 17, the same
eleven lines) and drafting the upstream report.

## 5. Predictions

Gate: configurations A and B, 1531 + 2 = **1533 passed, 164 suites** (`rv_tail`
+1, `arm_tail` +1); configuration C, **5 passed, 1 suite** (the examples and
`another_machine` new, the RISC-V refusal gone). Changed expectation: D5.
