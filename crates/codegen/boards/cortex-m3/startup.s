/* A Cortex-M3 board's start-up (spec 2026-11-13-elya-boards D3). Written by
 * ChatGPT (OpenAI) to Claude's brief, released by it to the public domain
 * (CC0-1.0); reviewed and run here, and given its own fault stack.
 *
 * The vector table, then the reset handler: `.data` copied from flash, `.bss`
 * zeroed, then the program runs on its own stack (the process stack, `psp`)
 * while faults run on theirs (the main stack, `msp`): a program stopped by its
 * stack overflowing still has a stack to say so on. */
.syntax unified
.cpu cortex-m3
.thumb
.section .vectors,"a",%progbits
.word __handler_stack_top, Reset_Handler, Default_Handler, HardFault_Handler
.rept 12 + 44
.word Default_Handler
.endr

.text
.global Reset_Handler
.thumb_func
Reset_Handler:
    ldr r0, =__handler_stack_top
    msr msp, r0
    ldr r0, =__data_load
    ldr r1, =__data_start
    ldr r2, =__data_end
1:  cmp r1, r2
    bhs 2f
    ldr r3, [r0], #4
    str r3, [r1], #4
    b 1b
2:  ldr r1, =__bss_start
    ldr r2, =__bss_end
    movs r3, #0
3:  cmp r1, r2
    bhs 4f
    str r3, [r1], #4
    b 3b
4:  ldr r0, =__stack_top
    msr psp, r0
    movs r0, #2
    msr control, r0 /* SPSEL: thread mode on the process stack */
    isb
    bl elya_board_main
.thumb_func
Default_Handler:
    b Default_Handler

/* Every fault the board does not enable separately arrives here, escalated,
 * on the main stack. The frame it is given is the program's, on the process
 * stack (and is not read unless it lies wholly inside that stack). */
.global HardFault_Handler
.thumb_func
HardFault_Handler:
    mrs r0, psp
    b elya_board_fault

.global elya_semihost
.thumb_func
elya_semihost:
    bkpt 0xab
    bx lr
