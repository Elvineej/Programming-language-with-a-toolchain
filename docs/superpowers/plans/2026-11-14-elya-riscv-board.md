# Plan: Elya on a RISC-V board

Spec: `docs/superpowers/specs/2026-11-14-elya-riscv-board-design.md` (D1-D5).
Base: `main` at the Windows fix after 0609983.

## Steps

1. The source: which conventions LLVM 18's RISC-V back end has, when it makes a
   `musttail` call a tail call, `fastcc`'s argument registers (spec section 0).
2. `rv_tail.rs` (D1): one type; first six 64-bit words (the nine programs ran
   on ChatGPT's earlier board, at O2 and O0), then thirteen 32-bit ones, so a
   clause of five Ints fits, then twelve (LLVM main's `fastcc` has twelve
   registers: ChatGPT's verification, checked against its source).
3. `Machine::rv32imac`, `compile_module_for`'s RISC-V path, `boards` with
   LLVM's RISC-V back end.
4. In parallel, ChatGPT: the board's files (D2), the review of 52bf757 (D4),
   research on the ARM bug. Each checked here.
5. `ELYA_WORD` in the runtime (D3).
6. `native_boards.rs` over both boards; the examples; the review's tests and
   fixes; `check.sh` strict.
7. Controls R1-R4.

## Predictions

- Gate: A and B **1533 passed, 164 suites**; C **5 passed, 1 suite**.
