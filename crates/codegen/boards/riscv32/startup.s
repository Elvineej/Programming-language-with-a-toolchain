/* A 32-bit RISC-V board's start-up (QEMU `virt`, rv32imac, machine mode only;
 * spec 2026-11-14-elya-riscv-board D2). Written by ChatGPT (OpenAI) to
 * Claude's brief, released by it to the public domain (CC0-1.0); reviewed and
 * run here.
 *
 * QEMU loads the program into RAM, `.data` included. Hart 0 zeroes `.bss` and
 * runs the program on its own stack; a trap switches to a stack of its own, so
 * a program stopped by its stack overflowing still has one to say so on. */
.section .text.init, "ax", @progbits
.globl _start
.type _start, @function
.balign 4
_start:
    csrr t0, mhartid
    bnez t0, .Lother_hart
    /* The linker may relax other addresses relative to gp, but not this one. */
    .option push
    .option norelax
    la gp, __global_pointer$
    .option pop
    la t0, __bss_start
    la t1, __bss_end
.Lzero_bss:
    bgeu t0, t1, .Lbss_done
    sw zero, 0(t0)
    addi t0, t0, 4
    j .Lzero_bss
.Lbss_done:
    la t0, .Ltrap_entry
    /* Direct mode: the handler's low two bits are zero. */
    csrw mtvec, t0
    la sp, __stack_top
    call elya_board_main
.Lstopped:
    wfi
    j .Lstopped
.Lother_hart:
    wfi
    j .Lother_hart

/* Never reads, writes or relies on the interrupted program's sp. */
.balign 4
.Ltrap_entry:
    la sp, __handler_stack_top
    csrr a0, mcause
    csrr a1, mepc
    csrr a2, mtval
    call elya_board_trap
.Ltrap_stopped:
    wfi
    j .Ltrap_stopped

/* RISC-V semihosting: the Arm operations behind three 32-bit instructions,
 * contiguous (hence no compressed ones) and 16-byte aligned. */
.section .text.semihost, "ax", @progbits
.option push
.option norvc
.balign 16
.globl elya_semihost
.type elya_semihost, @function
elya_semihost:
    slli x0, x0, 0x1f
    ebreak
    srai x0, x0, 7
    ret
.option pop
