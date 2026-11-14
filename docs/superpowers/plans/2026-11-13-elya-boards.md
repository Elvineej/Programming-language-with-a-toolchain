# Plan: Elya on a board (the collector, part 3)

Spec: `docs/superpowers/specs/2026-11-13-elya-boards-design.md` (D1-D5).
Base: `main` at fe2639d.

## Steps

1. `lib.rs`: `Machine`, `compile_module_for`; the module given the machine's
   triple and data layout; static code and `nounwind` on a board.
2. Nine programs compiled for the Cortex-M3 and linked by hand with the runtime
   and ChatGPT's board files: what failed to link (spec section 0), fixed in
   turn: the clause table as pointers (`cps_emit.rs`, D2), `nounwind`, the ARM
   EABI helpers in `board.c`.
3. In parallel, ChatGPT researched LLVM's bare-metal support (spec section 3).
4. The hard faults at pc 0: QEMU's instruction trace to the faulting `pop`, the
   disassembly of caller and callee, the two lines in `ARMISelLowering.cpp`
   (release/18.x and main). `arm_tail.rs` (D4): the padding, after the optimiser.
5. The board moved to `mps2-an385`; a recursion past the stack found writing over
   the heap: the guard, the stacks, the fault lines (D3).
6. The `boards` feature (the maintainer's LLVM has x86 only); `check.sh`'s
   configuration C.
7. Tests: `native_boards.rs` (new, 4), `arm_tail.rs` +1. Controls C1-C4.

## Predictions

- Gate: A and B **1531 passed, 164 suites**; C **4 passed, 1 suite**. No existing
  expectation changes.
