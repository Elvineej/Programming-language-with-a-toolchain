/* A Cortex-M3 board's memory (spec 2026-11-13-elya-boards D3), for QEMU's
 * `mps2-an385`: 4 MiB of code memory at 0, 4 MiB of data memory at
 * 0x20000000. Written by ChatGPT (OpenAI) to Claude's brief (CC0), for a
 * smaller board; laid out again here.
 *
 * RAM, from its start: a 4 KiB guard the processor's memory protection unit
 * forbids, the program's stack (1 MiB, or `--defsym ELYA_STACK_BYTES=n`),
 * the static data, a 1 KiB stack for the fault handler, then the collector's
 * region to the end of RAM (or `--defsym ELYA_HEAP_BYTES=n` of it). The stack
 * grows down towards the guard, so a recursion too deep for it stops at the
 * guard, by name, and never writes over the static data or the heap. */
ENTRY(Reset_Handler)
MEMORY {
    FLASH (rx) : ORIGIN = 0, LENGTH = 4M
    RAM (rwx) : ORIGIN = 0x20000000, LENGTH = 4M
}
__stack_bytes = DEFINED(ELYA_STACK_BYTES) ? ELYA_STACK_BYTES : 1M;
SECTIONS {
    .vectors : { KEEP(*(.vectors)) } > FLASH
    .text : {
        *(.text .text.*)
        *(.rodata .rodata.*)
    } > FLASH
    .ARM.exidx : { *(.ARM.exidx*) } > FLASH
    .stack (NOLOAD) : {
        __stack_guard = .;
        . += 4K;
        __stack_bottom = .;
        . += __stack_bytes;
        __stack_top = .;
    } > RAM
    .data : ALIGN(8) {
        __data_start = .;
        *(.data .data.*)
        . = ALIGN(4);
        __data_end = .;
    } > RAM AT> FLASH
    __data_load = LOADADDR(.data);
    .bss (NOLOAD) : ALIGN(8) {
        __bss_start = .;
        *(.bss .bss.* COMMON)
        . = ALIGN(4);
        __bss_end = .;
    } > RAM
    .handler_stack (NOLOAD) : ALIGN(8) {
        . += 1K;
        __handler_stack_top = .;
    } > RAM
    __heap_start = ALIGN(., 4096);
    __heap_end = DEFINED(ELYA_HEAP_BYTES) ? __heap_start + ELYA_HEAP_BYTES : ORIGIN(RAM) + LENGTH(RAM);
    ASSERT(__stack_guard == ORIGIN(RAM), "the guard is not where the memory protection unit expects it")
    ASSERT(__stack_bytes % 8 == 0 && __stack_bytes >= 8K, "ELYA_STACK_BYTES: a multiple of 8, at least 8 KiB")
    ASSERT(__heap_start < __heap_end && __heap_end <= ORIGIN(RAM) + LENGTH(RAM), "the static data leaves no heap")
}
