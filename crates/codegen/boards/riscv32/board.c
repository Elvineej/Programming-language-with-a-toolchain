/* A 32-bit RISC-V board's platform (spec 2026-11-14-elya-riscv-board D2): bytes
 * out and stop by RISC-V semihosting (QEMU passes them to its own stdout,
 * stderr and exit status), a clock, a locked PMP guard below the stack, traps
 * by name, the collector's region after the static data, then the program's
 * `main`.
 * Written by ChatGPT (OpenAI) to Claude's brief, released by it to the public
 * domain (CC0-1.0); reviewed and run here. */
#include <stddef.h>
#include <stdint.h>

uintptr_t elya_semihost(uintptr_t op, const uintptr_t *args);
void elya_heap_init(void *base, size_t bytes);
/* The generated `main` takes the arguments; a board has none (spec
 * 2026-11-14-elya-files-and-arguments D5). */
int main(int argc, char **argv);
void elya_platform_exit(int code) __attribute__((noreturn));
void elya_board_trap(uint32_t mcause, uint32_t mepc, uint32_t mtval) __attribute__((noreturn));

extern unsigned char __heap_start[], __heap_end[];
extern unsigned char __stack_guard[], __stack_bottom[], __stack_top[];

enum { SYS_OPEN = 1, SYS_WRITE = 5, SYS_EXIT_EXTENDED = 0x20 };
enum { ADP_Stopped_ApplicationExit = 0x20026, GUARD_BYTES = 4096 };

void elya_platform_exit(int code) {
    const uintptr_t args[] = {ADP_Stopped_ApplicationExit, (uintptr_t)(unsigned)code};
    (void)elya_semihost(SYS_EXIT_EXTENDED, args);
    for (;;) {
        __asm__ volatile("wfi");
    }
}

/* `:tt` opened for writing is the host's stdout; for appending, its stderr. */
static uintptr_t elya_board_stream(int stream) {
    static uintptr_t handles[3] = {0, (uintptr_t)-1, (uintptr_t)-1};
    if (handles[stream] == (uintptr_t)-1) {
        const uintptr_t args[] = {(uintptr_t)":tt", stream == 2 ? 8u : 4u, 3};
        handles[stream] = elya_semihost(SYS_OPEN, args);
        if (handles[stream] == (uintptr_t)-1) {
            elya_platform_exit(70);
        }
    }
    return handles[stream];
}

void elya_platform_write(int stream, const char *p, size_t n) {
    uintptr_t h = elya_board_stream(stream == 2 ? 2 : 1);
    while (n) {
        const uintptr_t args[] = {h, (uintptr_t)p, n};
        uintptr_t left = elya_semihost(SYS_WRITE, args);
        if (left >= n) {
            elya_platform_exit(70);
        }
        p += n - left;
        n = left;
    }
}

/* The guard. Machine mode obeys a PMP entry only when it is locked, and the
 * lowest-numbered entry that matches decides:
 *   entry 0: the 4 KiB at `__stack_guard`, NAPOT, locked, no access
 *            (L = 1, A = 3, RWX = 0: 0x98); pmpaddr0 = base >> 2 | 0x1ff
 *            (nine ones: 2^(9 + 3) bytes);
 *   entry 1: every address, NAPOT, unlocked, all access (A = 3, RWX = 7:
 *            0x1f); pmpaddr1 = all ones.
 * pmpcfg0 holds entries 0 to 3 a byte each: 0x00001f98. */
static void elya_board_guard(void) {
    const uint32_t guard = ((uint32_t)(uintptr_t)__stack_guard >> 2) | ((GUARD_BYTES / 8u) - 1u);
    const uint32_t all = UINT32_MAX;
    const uint32_t cfg = 0x00001f98u;
    __asm__ volatile("csrw pmpaddr0, %0\n\t"
                     "csrw pmpaddr1, %1\n\t"
                     "csrw pmpcfg0, %2"
                     :
                     : "r"(guard), "r"(all), "r"(cfg)
                     : "memory");
}

static char *elya_board_hex(char *out, uint32_t v) {
    for (int i = 7; i >= 0; i--) {
        *out++ = "0123456789abcdef"[(v >> (4 * i)) & 15];
    }
    return out;
}

static char *elya_board_text(char *out, const char *s) {
    while (*s) {
        *out++ = *s++;
    }
    return out;
}

static char *elya_board_decimal(char *out, uint32_t v) {
    char digits[10];
    int n = 0;
    do {
        digits[n++] = (char)('0' + v % 10);
        v /= 10;
    } while (v);
    while (n) {
        *out++ = digits[--n];
    }
    return out;
}

/* A trap, by name. A load or store fault on the guard (mcause 5, 7) is a
 * stack overflow: the hosted build's line and status, with this stack's size.
 * Any other trap says its cause, where it was and the faulting address,
 * status 70. */
void elya_board_trap(uint32_t mcause, uint32_t mepc, uint32_t mtval) {
    uint32_t guard = (uint32_t)(uintptr_t)__stack_guard;
    char line[128];
    char *at = line;
    if ((mcause == 5u || mcause == 7u) && mtval - guard < GUARD_BYTES) {
        at = elya_board_text(at, "elya: native stack overflow: recursion deeper than the ");
        at = elya_board_decimal(at, (uint32_t)(__stack_top - __stack_bottom) / 1024u);
        at = elya_board_text(at, " KiB stack\n");
        elya_platform_write(2, line, (size_t)(at - line));
        elya_platform_exit(1);
    }
    at = elya_board_text(at, "elya: trap mcause ");
    at = elya_board_hex(at, mcause);
    at = elya_board_text(at, " at pc ");
    at = elya_board_hex(at, mepc);
    at = elya_board_text(at, " (mtval ");
    at = elya_board_hex(at, mtval);
    at = elya_board_text(at, ")\n");
    elya_platform_write(2, line, (size_t)(at - line));
    elya_platform_exit(70);
}

/* The board's clock: the CLINT's `mtime`, 10 MHz on QEMU's `virt`, read high,
 * low, high so a carry between the halves is never half seen. */
#define CLINT_MTIME_LO (*(volatile uint32_t *)0x0200BFF8)
#define CLINT_MTIME_HI (*(volatile uint32_t *)0x0200BFFC)

int64_t elya_platform_now_ms(void) {
    uint32_t hi, lo;
    do {
        hi = CLINT_MTIME_HI;
        lo = CLINT_MTIME_LO;
    } while (hi != CLINT_MTIME_HI);
    return (int64_t)((((uint64_t)hi << 32) | lo) / 10000u);
}

void elya_platform_sleep_ms(int64_t ms) {
    int64_t until = elya_platform_now_ms() + ms;
    while (elya_platform_now_ms() < until) {
    }
}

void elya_board_main(void) {
    elya_board_guard();
    elya_heap_init(__heap_start, (size_t)(__heap_end - __heap_start));
    elya_platform_exit(main(0, 0));
}
