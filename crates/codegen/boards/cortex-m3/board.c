/* A Cortex-M3 board's platform (spec 2026-11-13-elya-boards D3): bytes out and
 * stop by Arm semihosting (QEMU passes them to its own stdout, stderr and exit
 * status), a clock, a guard below the stack, the collector's region after the
 * static data, then the program's `main`. Semihosting written by ChatGPT (OpenAI) to Claude's
 * brief (CC0); reviewed here: the handles are opened once, not per write. */
#include <stddef.h>
#include <stdint.h>

uintptr_t elya_semihost(uintptr_t op, const uintptr_t *args);
void elya_heap_init(void *base, size_t bytes);
/* The generated `main` takes the arguments; a board has none (spec
 * 2026-11-14-elya-files-and-arguments D5). */
int main(int argc, char **argv);

extern unsigned char __heap_start[], __heap_end[];

enum { SYS_OPEN = 1, SYS_WRITE = 5, SYS_EXIT_EXTENDED = 0x20 };
enum { ADP_Stopped_ApplicationExit = 0x20026 };

void elya_platform_exit(int code) {
    const uintptr_t args[] = {ADP_Stopped_ApplicationExit, (uintptr_t)(unsigned)code};
    (void)elya_semihost(SYS_EXIT_EXTENDED, args);
    for (;;) {
    }
}

/* `:tt` opened for writing is the host's stdout; for appending, its stderr. */
static uintptr_t elya_board_stream(int stream) {
    static uintptr_t handles[3] = {0, (uintptr_t)-1, (uintptr_t)-1};
    if (handles[stream] == (uintptr_t)-1) {
        const uintptr_t args[] = {(uintptr_t)":tt", stream == 2 ? 8u : 4u, 3};
        handles[stream] = elya_semihost(SYS_OPEN, args);
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

/* The memory protection unit's guard below the program's stack (link.ld):
 * region 0, the 4 KiB at `__stack_guard`, no access and no execution, and the
 * processor's default map everywhere else (PRIVDEFENA). The fault handler runs
 * with the unit off (HFNMIENA clear). A processor with no unit has no guard;
 * its stack still grows away from the data and the heap. */
#define SCB_CFSR (*(volatile uint32_t *)0xE000ED28)
#define SCB_HFSR (*(volatile uint32_t *)0xE000ED2C)
#define SCB_MMFAR (*(volatile uint32_t *)0xE000ED34)
#define SCB_BFAR (*(volatile uint32_t *)0xE000ED38)
#define MPU_TYPE (*(volatile uint32_t *)0xE000ED90)
#define MPU_CTRL (*(volatile uint32_t *)0xE000ED94)
#define MPU_RBAR (*(volatile uint32_t *)0xE000ED9C)
#define MPU_RASR (*(volatile uint32_t *)0xE000EDA0)

extern unsigned char __stack_guard[], __stack_bottom[], __stack_top[];
enum { GUARD_BYTES = 4096 };

static void elya_board_guard(void) {
    if (((MPU_TYPE >> 8) & 0xff) == 0) {
        return;
    }
    MPU_RBAR = (uint32_t)(uintptr_t)__stack_guard | (1u << 4) | 0; /* VALID, region 0 */
    /* XN, AP = no access, SIZE = 2^(11 + 1) = 4 KiB, ENABLE. */
    MPU_RASR = (1u << 28) | (0u << 24) | (11u << 1) | 1u;
    MPU_CTRL = (1u << 2) | 1u; /* PRIVDEFENA, ENABLE */
    __asm__ volatile("dsb\n\tisb" ::: "memory");
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

/* A fault, by name. A stack overflow (the frame could not be pushed, or the
 * program touched the guard) is the hosted build's line and status, with this
 * stack's size; any other fault says where it was and the fault registers,
 * status 70. */
void elya_board_fault(const uint32_t *frame) {
    uint32_t cfsr = SCB_CFSR;
    uint32_t guard = (uint32_t)(uintptr_t)__stack_guard;
    int stacking = (cfsr & ((1u << 4) | (1u << 12))) != 0; /* MSTKERR, STKERR */
    int touched = (cfsr & (1u << 1)) && (cfsr & (1u << 7)) /* DACCVIOL, MMARVALID */
                  && SCB_MMFAR - guard < GUARD_BYTES;
    char line[128];
    char *at = line;
    if (stacking || touched) {
        at = elya_board_text(at, "elya: native stack overflow: recursion deeper than the ");
        at = elya_board_decimal(at, (uint32_t)(__stack_top - __stack_bottom) / 1024);
        at = elya_board_text(at, " KiB stack\n");
        elya_platform_write(2, line, (size_t)(at - line));
        elya_platform_exit(1);
    }
    int readable = (const unsigned char *)frame >= __stack_bottom
                   && (const unsigned char *)(frame + 8) <= __stack_top;
    at = elya_board_text(at, "elya: hard fault at pc ");
    at = elya_board_hex(at, readable ? frame[6] : 0xffffffffu);
    at = elya_board_text(at, " (cfsr ");
    at = elya_board_hex(at, cfsr);
    at = elya_board_text(at, ", hfsr ");
    at = elya_board_hex(at, SCB_HFSR);
    at = elya_board_text(at, ", bfar ");
    at = elya_board_hex(at, SCB_BFAR);
    at = elya_board_text(at, ")\n");
    elya_platform_write(2, line, (size_t)(at - line));
    elya_platform_exit(70);
}

/* The board's clock: the MPS2 FPGA's 100 Hz counter (FPGAIO CLK100HZ, counting
 * from reset), so `time.now_ms` moves in steps of 10 ms and `time.sleep_ms`
 * waits on it. A board without one keeps the runtime's clock that stands
 * still. */
#define FPGAIO_CLK100HZ (*(volatile uint32_t *)0x40028014)

int64_t elya_platform_now_ms(void) { return (int64_t)FPGAIO_CLK100HZ * 10; }

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

/* The ARM EABI's memory helpers, which a compiler for ARM calls where another
 * would call memset or memcpy (run-time ABI for the Arm architecture, 4.3.4):
 * note `__aeabi_memset`'s order, (dest, n, c). */
void *memcpy(void *, const void *, size_t);
void *memmove(void *, const void *, size_t);
void *memset(void *, int, size_t);
void __aeabi_memcpy(void *d, const void *s, size_t n) { memcpy(d, s, n); }
void __aeabi_memcpy4(void *d, const void *s, size_t n) { memcpy(d, s, n); }
void __aeabi_memcpy8(void *d, const void *s, size_t n) { memcpy(d, s, n); }
void __aeabi_memmove(void *d, const void *s, size_t n) { memmove(d, s, n); }
void __aeabi_memmove4(void *d, const void *s, size_t n) { memmove(d, s, n); }
void __aeabi_memmove8(void *d, const void *s, size_t n) { memmove(d, s, n); }
void __aeabi_memset(void *d, size_t n, int c) { memset(d, c, n); }
void __aeabi_memset4(void *d, size_t n, int c) { memset(d, c, n); }
void __aeabi_memset8(void *d, size_t n, int c) { memset(d, c, n); }
void __aeabi_memclr(void *d, size_t n) { memset(d, 0, n); }
void __aeabi_memclr4(void *d, size_t n) { memset(d, 0, n); }
void __aeabi_memclr8(void *d, size_t n) { memset(d, 0, n); }
