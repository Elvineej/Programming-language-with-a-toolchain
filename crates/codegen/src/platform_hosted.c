/* The platform layer, hosted (spec 2026-11-13-elya-platform-layer D2): the
 * hooks runtime.c and gc.c declare, over the C library and the operating
 * system (Linux, macOS, Windows), and the program's own stack. Included at the
 * end of runtime.c unless ELYA_FREESTANDING is defined; never compiled alone. */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <sys/mman.h>
#include <time.h>
#endif

/* Memory from the operating system (the collector's page source). */
void *elya_platform_pages(size_t bytes, size_t align) {
    size_t want = bytes + (align > 4096 ? align : 0);
#ifdef _WIN32
    char *m = (char *)VirtualAlloc(NULL, want, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
#else
    char *m = (char *)mmap(NULL, want, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (m == (char *)MAP_FAILED) {
        m = NULL;
    }
#endif
    if (!m) {
        return NULL;
    }
    return (void *)(((uintptr_t)m + align - 1) & ~(uintptr_t)(align - 1));
}

void elya_platform_release(void *p, size_t bytes) {
#ifdef _WIN32
    (void)bytes;
    VirtualFree(p, 0, MEM_RELEASE);
#else
    munmap(p, bytes);
#endif
}

void elya_platform_init(void) {
#ifdef _WIN32
    /* The shim calls this first, so stdout is byte-exact for the WHOLE run --
     * including main's value line, which a program that never prints would
     * otherwise write in text mode as "\r\n" (found by the Windows gate on the
     * non-printing CPS-rooting corpus, 2026-10-04). elya_println keeps its own
     * call; it is now redundant but harmless. stderr too (2026-10-30): a
     * failure is one line, "elya: ...\n", which text mode wrote as "\r\n"
     * (the Windows gate on main: three native_any.rs tests read a stray "\r"). */
    _setmode(_fileno(stdout), _O_BINARY);
    _setmode(_fileno(stderr), _O_BINARY);
#endif
}

/* stdout buffered as the C library buffers it (flushed at exit and before a
 * failure's line); stderr as it comes. */
void elya_platform_write(int stream, const char *bytes, size_t n) {
    fwrite(bytes, 1, n, stream == 1 ? stdout : stderr);
}

void elya_platform_flush(void) { fflush(stdout); }

void elya_platform_exit(int code) { exit(code); }

const char *elya_platform_getenv(const char *name) { return getenv(name); }

void *elya_platform_scratch(void *p, size_t bytes) {
    if (bytes == 0) {
        free(p);
        return NULL;
    }
    return realloc(p, bytes);
}

int64_t elya_platform_now_ms(void) {
#ifdef _WIN32
    return (int64_t)GetTickCount64();
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000 + (int64_t)(ts.tv_nsec / 1000000);
#endif
}

void elya_platform_sleep_ms(int64_t ms) {
#ifdef _WIN32
    /* `Sleep` may return before its interval, and `GetTickCount64` moves in
     * 10-16 ms steps, so wait on the clock `time.now_ms` reads until it has
     * moved by `ms` (the 2026-10-16 review). */
    int64_t end = elya_platform_now_ms() + ms;
    for (;;) {
        int64_t left = end - elya_platform_now_ms();
        if (left <= 0) {
            break;
        }
        Sleep(left > 0x7fffffff ? 0x7fffffff : (DWORD)left);
    }
#else
    /* An interrupted nanosleep goes on with what remains. */
    struct timespec req = {(time_t)(ms / 1000), (long)((ms % 1000) * 1000000L)};
    while (nanosleep(&req, &req) == -1 && errno == EINTR) {
    }
#endif
}

static FILE *elya_trace_file;

int elya_platform_trace_create(const char *path) {
    elya_trace_file = fopen(path, "wb");
    return elya_trace_file != NULL;
}

int elya_platform_trace_append(const char *bytes, size_t n) {
    fwrite(bytes, 1, n, elya_trace_file);
    return fflush(elya_trace_file) == 0 && !ferror(elya_trace_file);
}

void *elya_platform_file_open(const char *path) { return fopen(path, "rb"); }

int64_t elya_platform_file_read(void *file, char *into, int64_t n) {
    size_t got = fread(into, 1, (size_t)n, (FILE *)file);
    return ferror((FILE *)file) ? -1 : (int64_t)got;
}

void elya_platform_file_close(void *file) { fclose((FILE *)file); }

const char *elya_platform_error(void) { return strerror(errno); }

/* The program's own stack (spec 2026-10-16-elya-native-stack). `@main` calls
 * `elya_start` with `@elya_entry`, which calls `main`'s function; it runs on a
 * thread whose stack is a fixed, documented size (D1: 1 GiB, or ELYA_STACK_MIB,
 * D2), reserved, not committed, so a shallow program pays nothing. Reaching its
 * end is a NAMED failure, never a bare signal (D5). The collector never looks at
 * the machine stack (its roots are the shadow stack), so it is unaffected. */
#define ELYA_STACK_DEFAULT_MIB 1024
#define ELYA_STACK_MAX_MIB 1048576

typedef int64_t (*elya_entry_fn)(void);
static elya_entry_fn elya_entry_ptr;
static int64_t elya_entry_result;
static int64_t elya_stack_mib = ELYA_STACK_DEFAULT_MIB;

/* Output already written stays written (flushed first), then the one line,
 * exit status 1 like every runtime trap. */
static void elya_stack_overflowed(void) {
    fflush(stdout);
    fprintf(stderr, "elya: native stack overflow: recursion deeper than the %lld MiB stack\n",
            (long long)elya_stack_mib);
    fflush(stderr);
    exit(1);
}

__attribute__((noreturn)) static void elya_stack_unavailable(void) {
    fprintf(stderr, "elya: cannot reserve a stack of %lld MiB (ELYA_STACK_MIB sets a smaller one)\n",
            (long long)elya_stack_mib);
    exit(1);
}

/* D2: a whole number of MiB, 1 to ELYA_STACK_MAX_MIB, or the default when the
 * variable is unset. Anything else stops here, before the program runs. */
static int64_t elya_stack_size_mib(void) {
    const char *s = getenv("ELYA_STACK_MIB");
    if (!s) {
        return ELYA_STACK_DEFAULT_MIB;
    }
    int64_t v = 0;
    int ok = *s != '\0';
    for (const char *p = s; ok && *p; p++) {
        if (*p < '0' || *p > '9') {
            ok = 0;
        } else {
            v = v * 10 + (*p - '0');
            if (v > ELYA_STACK_MAX_MIB) {
                ok = 0;
            }
        }
    }
    if (!ok || v < 1) {
        fputs("elya: ELYA_STACK_MIB must be a whole number of MiB from 1 to 1048576\n", stderr);
        exit(1);
    }
    return v;
}

#ifdef _WIN32
/* Room for the handler to run on the overflowed thread. */
static DWORD WINAPI elya_program_thread(LPVOID arg) {
    (void)arg;
    ULONG guarantee = 64 * 1024;
    SetThreadStackGuarantee(&guarantee);
    elya_entry_result = elya_entry_ptr();
    return 0;
}

static LONG CALLBACK elya_overflow_handler(PEXCEPTION_POINTERS e) {
    if (e->ExceptionRecord->ExceptionCode == EXCEPTION_STACK_OVERFLOW) {
        elya_stack_overflowed();
    }
    return EXCEPTION_CONTINUE_SEARCH;
}

int64_t elya_start(elya_entry_fn entry) {
    elya_stack_mib = elya_stack_size_mib();
    elya_trace_init();
    elya_entry_ptr = entry;
    AddVectoredExceptionHandler(1, elya_overflow_handler);
    HANDLE t = CreateThread(NULL, (SIZE_T)elya_stack_mib << 20, elya_program_thread, NULL,
                            STACK_SIZE_PARAM_IS_A_RESERVATION, NULL);
    if (!t) {
        elya_stack_unavailable();
    }
    WaitForSingleObject(t, INFINITE);
    CloseHandle(t);
    elya_trace_finish();
    return elya_entry_result;
}
#else
#ifndef MAP_ANONYMOUS
#define MAP_ANONYMOUS MAP_ANON
#endif
/* D5: 1 MiB of PROT_NONE below the stack, so no frame can step over it. */
#define ELYA_GUARD_BYTES ((size_t)1 << 20)

static char *elya_guard_lo;
static char *elya_guard_hi;
static sigjmp_buf elya_overflow_jump;
static char elya_signal_stack[64 * 1024];

/* On the alternate stack. A fault in the guard is the overflow: jump back to
 * the thread's entry, whose stack is intact, and report from there. Any other
 * fault is not ours to name: restore the default action and return, so the
 * faulting instruction runs again and the process dies by the signal, as it
 * always did. */
static void elya_fault(int sig, siginfo_t *info, void *uc) {
    (void)uc;
    char *a = (char *)info->si_addr;
    if (a >= elya_guard_lo && a < elya_guard_hi) {
        siglongjmp(elya_overflow_jump, 1);
    }
    struct sigaction dfl;
    memset(&dfl, 0, sizeof dfl);
    dfl.sa_handler = SIG_DFL;
    sigemptyset(&dfl.sa_mask);
    sigaction(sig, &dfl, NULL);
    /* A signal SENT (kill, raise: si_code <= 0) has no instruction to run
     * again; send it once more, now to the default action (the 2026-10-16
     * review, L1). It is blocked while this handler runs, so it is delivered,
     * and kills, as the handler returns. */
    if (info->si_code <= 0) {
        raise(sig);
    }
}

static void *elya_program_thread(void *arg) {
    (void)arg;
    stack_t ss;
    ss.ss_sp = elya_signal_stack;
    ss.ss_size = sizeof elya_signal_stack;
    ss.ss_flags = 0;
    if (sigaltstack(&ss, NULL) != 0) {
        elya_stack_unavailable();
    }
    if (sigsetjmp(elya_overflow_jump, 1)) {
        elya_stack_overflowed();
    }
    elya_entry_result = elya_entry_ptr();
    return NULL;
}

int64_t elya_start(elya_entry_fn entry) {
    elya_stack_mib = elya_stack_size_mib();
    elya_trace_init();
    elya_entry_ptr = entry;
    size_t size = (size_t)elya_stack_mib << 20;
    int flags = MAP_PRIVATE | MAP_ANONYMOUS;
#ifdef MAP_NORESERVE
    flags |= MAP_NORESERVE;
#endif
    char *base = (char *)mmap(NULL, size + ELYA_GUARD_BYTES, PROT_READ | PROT_WRITE, flags, -1, 0);
    if (base == (char *)MAP_FAILED) {
        elya_stack_unavailable();
    }
    if (mprotect(base, ELYA_GUARD_BYTES, PROT_NONE) != 0) {
        elya_stack_unavailable();
    }
    elya_guard_lo = base;
    elya_guard_hi = base + ELYA_GUARD_BYTES;
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = elya_fault;
    sa.sa_flags = SA_SIGINFO | SA_ONSTACK;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGSEGV, &sa, NULL);
    sigaction(SIGBUS, &sa, NULL); /* macOS reports a guard page as SIGBUS */
    pthread_attr_t attr;
    pthread_t t;
    if (pthread_attr_init(&attr) != 0 || pthread_attr_setstack(&attr, base + ELYA_GUARD_BYTES, size) != 0 ||
        pthread_create(&t, &attr, elya_program_thread, NULL) != 0) {
        elya_stack_unavailable();
    }
    pthread_attr_destroy(&attr);
    pthread_join(t, NULL);
    elya_trace_finish();
    return elya_entry_result;
}
#endif
