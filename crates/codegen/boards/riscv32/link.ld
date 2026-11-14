/* A 32-bit RISC-V board's memory (spec 2026-11-14-elya-riscv-board D2), for
 * QEMU's `virt`: 128 MiB of RAM at 0x80000000. Written by ChatGPT (OpenAI) to
 * Claude's brief, released by it to the public domain (CC0-1.0); reviewed and
 * run here.
 *
 * RAM, from its start: the code and constants, a 4 KiB guard the PMP forbids,
 * the program's stack (1 MiB, or `--defsym ELYA_STACK_BYTES=n`), the static
 * data (loaded in place), a 1 KiB stack for traps, then the collector's region
 * to the end of RAM (or `--defsym ELYA_HEAP_BYTES=n` of it). The stack grows
 * down towards the guard, so a recursion too deep for it stops at the guard,
 * by name, and never writes over the static data or the heap. */
ENTRY(_start)
MEMORY {
    RAM (rwx) : ORIGIN = 0x80000000, LENGTH = 128M
}
__stack_bytes = DEFINED(ELYA_STACK_BYTES) ? ELYA_STACK_BYTES : 1M;
SECTIONS {
    .text : {
        KEEP(*(.text.init))
        *(.text .text.*)
        *(.rodata .rodata.* .srodata .srodata.*)
        *(.eh_frame .eh_frame.*)
    } > RAM
    .stack (NOLOAD) : ALIGN(4096) {
        __stack_guard = .;
        . += 4K;
        __stack_bottom = .;
        . += __stack_bytes;
        __stack_top = .;
    } > RAM
    .data : ALIGN(16) {
        __data_start = .;
        *(.data .data.*)
        __global_pointer$ = . + 0x800;
        *(.sdata .sdata.*)
        . = ALIGN(16);
        __data_end = .;
    } > RAM
    .bss (NOLOAD) : ALIGN(16) {
        __bss_start = .;
        *(.bss .bss.* .sbss .sbss.* COMMON)
        . = ALIGN(16);
        __bss_end = .;
    } > RAM
    .handler_stack (NOLOAD) : ALIGN(16) {
        . += 1K;
        __handler_stack_top = .;
    } > RAM
    __heap_start = ALIGN(., 4096);
    __heap_end = DEFINED(ELYA_HEAP_BYTES) ? __heap_start + ELYA_HEAP_BYTES : ORIGIN(RAM) + LENGTH(RAM);
    ASSERT((__stack_guard & 4095) == 0, "the PMP guard is not 4 KiB aligned")
    ASSERT(__stack_bytes % 16 == 0 && __stack_bytes >= 8K, "ELYA_STACK_BYTES: a multiple of 16, at least 8 KiB")
    ASSERT(__heap_start < __heap_end && __heap_end <= ORIGIN(RAM) + LENGTH(RAM), "the static data leaves no heap")
}
