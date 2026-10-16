#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <errno.h>
#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <sys/mman.h>
#include <time.h>
#endif

/* The Elya native runtime: a stop-the-world, non-moving mark-sweep collector.
 * Single-threaded throughout — Elya has no native concurrency yet, so none of
 * this state needs synchronisation. (The program runs on a thread of its own,
 * `elya_start`, but only one thread ever runs Elya code, and the main thread
 * reads the collector's state only after joining it.)
 *
 * Object layout (5b-5 plan, pinned). `elya_alloc(words)` reserves `words + 2`
 * and returns a pointer to word 2, so the collector's prefix sits IN FRONT of
 * the tag and the visible `[tag][fields]` layout the Ctor arm emits is frozen:
 *
 *   [ word0: (size<<1)|mark ] [ word1: in-use next-link ] [ tag ] [ field_0 ] ...
 *      ^ internal                ^ internal                 ^ what elya_alloc returns
 *
 * Two prefix words rather than one because the list link needs a full pointer.
 */

/* [2*i] = arity, [2*i+1] = ptr_mask. Bit i of the mask is set iff field i
 * holds a heap pointer, so the mark phase knows which words to trace without
 * ever guessing. This precision is the point: a conservative stack scan could
 * read an Int as a pointer and make native MORE undefined than the evaluator,
 * which 5b-1 §3.4 forbids.
 *
 * `arity` is the count of traced-candidate words FOLLOWING the tag, NOT the
 * block's size: gc_sweep reads the size out of Block::meta and never consults a
 * descriptor, so a row may legitimately describe fewer words than the block
 * holds. N6's string row is exactly that case — [0, 0] for a block whose bytes
 * follow the tag but are not words to trace. */
static int64_t *gc_descriptors = NULL;
static int64_t gc_n_ctors = 0;

/* The shadow stack of live roots. Written by elya_gc_push/pop; read by mark.
 * The codegen brackets every allocation and every non-tail call with pushes and
 * pops, so when gc_mark runs this array holds exactly the heap values the
 * program can still reach a name for. */
static void **gc_shadow = NULL;
static int64_t gc_shadow_top = 0, gc_shadow_cap = 0;

/* gc_allocated counts WORDS handed out since the last collection, not objects.
 * Words are the memory-pressure proxy the threshold is denominated in: a
 * one-word `Nil` should not push toward a collection as hard as a sixteen-word
 * record does.
 *
 * gc_live is the odd one out: a LEVEL, not a flow. gc_collections and gc_freed
 * only ever climb; gc_live is recomputed from scratch by every sweep and
 * answers "how much is still reachable", which is the one place a
 * refcounting/tracing divergence could ever show up (5b-6 §11, obligation T7). */
static int64_t gc_allocated = 0, gc_collections = 0, gc_freed = 0, gc_live = 0;

typedef struct Block {
    intptr_t meta; /* (size << 1) | mark, where size counts VISIBLE words */
    struct Block *next;
} Block;

/* Every block that is currently handed out. A block lives on exactly one of
 * this list or its size's free list, never both — that is what makes handing
 * the same block out twice impossible by construction rather than by care.
 * (The plan calls this the all-blocks list; sweep "moves" dead blocks off it,
 * so at rest it holds precisely the in-use ones.) */
static Block *gc_all_blocks = NULL;

enum { GC_MAX_WORDS = 16 };
static Block *gc_free_lists[GC_MAX_WORDS] = {0};

#define GC_MARK_BIT 1

/* Pinned by measurement, not by taste. Built with collection disabled, the ADT
 * corpus reports its whole lifetime allocation at exit: option_extract 2 words,
 * recursive_nat 7, three_way 2, nested_match 5. The arithmetic, control-flow and
 * tail-call corpora allocate nothing at all. Seven words is therefore the FLOOR
 * this constant has to clear -- below it, ordinary programs would start
 * collecting incidentally and "no collection during an ordinary build" would
 * stop proving anything.
 *
 * The pin is 1<<16 words, half a mebibyte of payload and some nine thousand
 * times that floor, so the no-trip question is not a close call. The upper end
 * is purely an amortisation choice: at three words an iteration a
 * million-iteration allocating loop still collects about forty-five times. */
enum { GC_THRESHOLD_WORDS = 1 << 16 };

static int64_t *gc_payload(Block *b) { return (int64_t *)(b + 1); }

static Block *gc_block_of(void *payload) { return (Block *)((int64_t *)payload - 2); }

void elya_gc_init(const int64_t *descriptors, int64_t n_ctors) {
    gc_descriptors = (int64_t *)descriptors;
    gc_n_ctors = n_ctors;
#ifdef _WIN32
    /* The shim calls this first, so stdout is byte-exact for the WHOLE run --
     * including main's value line, which a program that never prints would
     * otherwise write in text mode as "\r\n" (found by the Windows gate on the
     * non-printing CPS-rooting corpus, 2026-10-04). elya_println keeps its own
     * call; it is now redundant but harmless. */
    _setmode(_fileno(stdout), _O_BINARY);
#endif
}

void elya_gc_push(void *root) {
    if (gc_shadow_top == gc_shadow_cap) {
        int64_t cap = gc_shadow_cap ? gc_shadow_cap * 2 : 256;
        void **grown = (void **)realloc(gc_shadow, (size_t)cap * sizeof(void *));
        if (!grown) {
            fputs("elya: out of memory growing the shadow stack\n", stderr);
            exit(1);
        }
        gc_shadow = grown;
        gc_shadow_cap = cap;
    }
    gc_shadow[gc_shadow_top++] = root;
}

void elya_gc_pop(void) {
    if (gc_shadow_top > 0) {
        gc_shadow_top--;
    }
}

/* The mark phase: an EXPLICIT worklist, never function recursion.
 *
 * This is the CtorArgs lesson applied to the collector as a hard constraint.
 * The evaluator's Drop already paid for it once on million-element Cons
 * chains: a recursive walk of a deep chain overflows the HOST stack, which
 * turns a routine collection into a process crash. A million-element chain is
 * a million worklist entries in heap memory here, and the host stack stays
 * flat. Schorr-Waite pointer reversal would remove the worklist's own space
 * cost; it is deferred as a later optimisation, not needed for correctness. */
static void **gc_gray = NULL;
static int64_t gc_gray_top = 0, gc_gray_cap = 0;

static void gc_gray_push(void *p) {
    if (!p) {
        return;
    }
    Block *b = gc_block_of(p);
    if (b->meta & GC_MARK_BIT) {
        return; /* already marked: shared structure and cycles both terminate */
    }
    b->meta |= GC_MARK_BIT;
    if (gc_gray_top == gc_gray_cap) {
        int64_t cap = gc_gray_cap ? gc_gray_cap * 2 : 256;
        void **grown = (void **)realloc(gc_gray, (size_t)cap * sizeof(void *));
        if (!grown) {
            fputs("elya: out of memory growing the mark worklist\n", stderr);
            exit(1);
        }
        gc_gray = grown;
        gc_gray_cap = cap;
    }
    gc_gray[gc_gray_top++] = p;
}

static void gc_mark(void) {
    for (int64_t i = 0; i < gc_shadow_top; i++) {
        gc_gray_push(gc_shadow[i]);
    }
    while (gc_gray_top > 0) {
        int64_t *obj = (int64_t *)gc_gray[--gc_gray_top];
        int64_t tag = obj[0];
        if (tag < 0 || tag >= gc_n_ctors) {
            continue; /* not a constructor we have a descriptor for */
        }
        int64_t arity = gc_descriptors[2 * tag];
        int64_t mask = gc_descriptors[2 * tag + 1];
        for (int64_t f = 0; f < arity; f++) {
            if ((mask >> f) & 1) {
                gc_gray_push((void *)(intptr_t)obj[1 + f]);
            }
        }
    }
}

static void gc_sweep(void) {
    Block **prev = &gc_all_blocks;
    Block *b = gc_all_blocks;
    /* Reset per cycle: gc_live is what survived THIS collection, not a running
     * total the way gc_freed is. */
    gc_live = 0;
    while (b) {
        Block *next = b->next;
        if (b->meta & GC_MARK_BIT) {
            /* The shift drops the mark bit, so this reads the same size whether
             * it runs before or after the clear below. */
            gc_live += b->meta >> 1;
            b->meta &= ~(intptr_t)GC_MARK_BIT; /* clear for the next cycle */
            prev = &b->next;
        } else {
            *prev = next; /* move it off the in-use list */
            intptr_t size = b->meta >> 1;
            if (size > 0 && size < GC_MAX_WORDS) {
                /* Thread the free link through the dead tag slot. Safe
                 * precisely because the object is dead: nothing reads that
                 * word again until elya_alloc re-zeroes it. */
                *(Block **)gc_payload(b) = gc_free_lists[size];
                gc_free_lists[size] = b;
            } else {
                free(b); /* no free list this wide: hand it back to malloc */
            }
            gc_freed++;
        }
        b = next;
    }
}

static void gc_collect(void) {
    gc_collections++;
    gc_mark();
    gc_sweep();
    gc_allocated = 0;
}

/* Allocate `words` 8-byte words, zeroed. The zeroing is the contract the Ctor
 * arm relies on, so a recycled block is re-zeroed on the way out. */
void *elya_alloc(int64_t words) {
    /* Roots for a collection triggered HERE are already on the shadow stack:
     * allocation sites push live heap bindings, and constructors also push
     * just-lowered fields. Marking runs before the block is handed out, so
     * the new object is never itself a mark target -- nothing yet points at
     * it. */
    /* Collect when allocation since the last collection reaches the larger of
     * the fixed floor and the last measured live set: the heap doubles
     * between collections, so a growing live set is marked O(log n) times,
     * not every 64K words (which made total marking quadratic -- measured:
     * 45 collections to build a 1M list, 183 for a 1M-deep effectful loop).
     * Small programs see the old 64K floor unchanged. */
    if (gc_allocated >= (gc_live > GC_THRESHOLD_WORDS ? gc_live : GC_THRESHOLD_WORDS)) {
        gc_collect();
    }
    gc_allocated += words;

    if (words > 0 && words < GC_MAX_WORDS && gc_free_lists[words]) {
        Block *b = gc_free_lists[words];
        gc_free_lists[words] = *(Block **)gc_payload(b);
        int64_t *payload = gc_payload(b);
        for (int64_t i = 0; i < words; i++) {
            payload[i] = 0;
        }
        b->meta = (intptr_t)words << 1;
        b->next = gc_all_blocks;
        gc_all_blocks = b;
        return payload;
    }

    Block *b = (Block *)calloc((size_t)words + 2, 8);
    if (!b) {
        fputs("elya: out of memory\n", stderr);
        exit(1);
    }
    b->meta = (intptr_t)words << 1;
    b->next = gc_all_blocks;
    gc_all_blocks = b;
    return gc_payload(b);
}

/* Copy static literal bytes into a tagged heap block. The extra byte reserves
 * the NUL terminator; elya_alloc zeroes it even when len is a multiple of 8. */
void *elya_str_lit(int64_t tag, const char *bytes, int64_t len) {
    int64_t words = 2 + ((len + 1) + 7) / 8;
    int64_t *p = (int64_t *)elya_alloc(words);
    p[0] = tag;
    p[1] = len;
    memcpy(&p[2], bytes, (size_t)len);
    return p;
}

/* Native strings (spec 2026-10-15-elya-native-strings, D2/D4). `a <> b` is a
 * fresh immutable string. Both operands are rooted while it allocates: callers
 * root them too (an operand is a temporary the emitter pushed), so this is
 * defence in depth, as in elya_cont_copy. */
void *elya_str_concat(int64_t tag, void *a, void *b) {
    int64_t la = ((int64_t *)a)[1];
    int64_t lb = ((int64_t *)b)[1];
    elya_gc_push(a);
    elya_gc_push(b);
    int64_t *p = (int64_t *)elya_alloc(2 + ((la + lb + 1) + 7) / 8);
    elya_gc_pop();
    elya_gc_pop();
    p[0] = tag;
    p[1] = la + lb;
    memcpy(&p[2], &((int64_t *)a)[2], (size_t)la);
    memcpy((char *)&p[2] + la, &((int64_t *)b)[2], (size_t)lb);
    return p;
}

/* `==` on strings (D3): byte equality, as the evaluator's. Allocates nothing. */
int64_t elya_str_eq(void *a, void *b) {
    int64_t la = ((int64_t *)a)[1];
    int64_t lb = ((int64_t *)b)[1];
    if (la != lb) {
        return 0;
    }
    return memcmp(&((int64_t *)a)[2], &((int64_t *)b)[2], (size_t)la) == 0;
}

/* `==` on data (spec 2026-10-16-elya-structural-equality, D3): 1 when `a` and
 * `b` are structurally equal. The descriptor table says which fields are heap
 * values (compared as values) and which are words (compared as words); a
 * string compares its length and bytes. An explicit worklist of pairs, never
 * recursion, so two long lists compare in flat host stack. Allocates nothing
 * from the collector, so nothing needs rooting across it. The type checker
 * (E0436) keeps functions and continuations out. */
int64_t elya_eq(void *a, void *b, int64_t string_tag) {
    int64_t **work = NULL;
    int64_t top = 0, cap = 0;
    int64_t equal = 1;
#define EQ_PUSH(x, y)                                                                  \
    do {                                                                               \
        if (top + 2 > cap) {                                                           \
            cap = cap ? cap * 2 : 64;                                                  \
            int64_t **grown = (int64_t **)realloc(work, (size_t)cap * sizeof *work);   \
            if (!grown) {                                                              \
                fputs("elya: out of memory comparing values\n", stderr);               \
                exit(1);                                                               \
            }                                                                          \
            work = grown;                                                              \
        }                                                                              \
        work[top++] = (int64_t *)(x);                                                  \
        work[top++] = (int64_t *)(y);                                                  \
    } while (0)
    EQ_PUSH(a, b);
    while (top > 0) {
        int64_t *y = work[--top];
        int64_t *x = work[--top];
        /* No shortcut for one shared object: whether two values are shared
         * must never decide `==` (a NaN field, once floats compile natively,
         * is not equal to itself), and the evaluator takes none. */
        if (!x || !y || x[0] != y[0]) {
            equal = 0;
            break;
        }
        int64_t tag = x[0];
        if (tag == string_tag) {
            if (x[1] != y[1] || memcmp(&x[2], &y[2], (size_t)x[1]) != 0) {
                equal = 0;
                break;
            }
            continue;
        }
        if (tag < 0 || tag >= gc_n_ctors) {
            fputs("elya: ==: a value without a descriptor\n", stderr);
            exit(1);
        }
        int64_t arity = gc_descriptors[2 * tag];
        int64_t mask = gc_descriptors[2 * tag + 1];
        for (int64_t f = 0; f < arity; f++) {
            if ((mask >> f) & 1) {
                EQ_PUSH((intptr_t)x[1 + f], (intptr_t)y[1 + f]);
            } else if (x[1 + f] != y[1 + f]) {
                equal = 0;
                break;
            }
        }
        if (!equal) {
            break;
        }
    }
#undef EQ_PUSH
    free(work);
    return equal;
}

/* `int.to_string` (D1): decimal, a leading '-' for a negative, INT64_MIN
 * included (20 characters at most). */
void *elya_int_to_string(int64_t tag, int64_t n) {
    char buf[32];
    int len = snprintf(buf, sizeof buf, "%lld", (long long)n);
    return elya_str_lit(tag, buf, (int64_t)len);
}

/* Time, the first I/O source (spec 2026-10-15-elya-async-step2-timers, D3).
 * `time.now_ms`: monotonic milliseconds since the first reading. `time.sleep_ms`:
 * a wait of at least `ms` (none when `ms <= 0`); an interrupted nanosleep goes on
 * with what remains. Neither allocates. */
static int64_t elya_time_raw_ms(void) {
#ifdef _WIN32
    return (int64_t)GetTickCount64();
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000 + (int64_t)(ts.tv_nsec / 1000000);
#endif
}

int64_t elya_time_now_ms(void) {
    static int64_t start = -1;
    int64_t now = elya_time_raw_ms();
    if (start < 0) {
        start = now;
    }
    return now - start;
}

void elya_time_sleep_ms(int64_t ms) {
    if (ms <= 0) {
        return;
    }
#ifdef _WIN32
    /* `Sleep` may return before its interval, and `GetTickCount64` moves in
     * 10-16 ms steps, so wait on the clock `time.now_ms` reads until it has
     * moved by `ms` (the 2026-10-16 review). */
    int64_t end = elya_time_raw_ms() + ms;
    for (;;) {
        int64_t left = end - elya_time_raw_ms();
        if (left <= 0) {
            break;
        }
        Sleep(left > 0x7fffffff ? 0x7fffffff : (DWORD)left);
    }
#else
    struct timespec req = {(time_t)(ms / 1000), (long)((ms % 1000) * 1000000L)};
    while (nanosleep(&req, &req) == -1 && errno == EINTR) {
    }
#endif
}

/* The deterministic failed-match trap. Spec 5b-4 §5: never `unreachable`. */
void elya_match_fail(void) {
    fputs("elya: match failed (no arm matched)\n", stderr);
    exit(1);
}

/* 5b-8 Task 8: the handler the running computation performs to. A handle
 * site sets it around its body and restores it after; a resume installs the
 * continuation's own handler around the resumed computation. A perform reads
 * it in O(1) -- walking the frame chain to its end on every perform made deep
 * non-tail effectful recursion quadratic (measured: 80k deep, 15 s). Since
 * slice 5b-10 the innermost handler: a perform walks handler frames' traced
 * `parent` words from here, and a clause or return clause runs with its
 * frame's `parent` current, paired with that frame's `next` (or the native
 * return of a direct handle) as its continuation. Not a GC root: whenever it
 * is read, it is reachable from the live continuation through `next` and
 * `parent`. Single-threaded, like the runtime. */
void *elya_current_handler = 0;

/* 5b-8 D13: one-shot is enforced natively. A second resume stops here -- a
 * named message and a non-zero exit, never a re-run. */
void elya_resume_twice(void) {
    fputs("elya: resume: a one-shot continuation was resumed twice\n", stderr);
    exit(1);
}

/* `Int` arithmetic is exact or fails by name (spec
 * 2026-10-10-elya-checked-integer-arithmetic, D3): the same words, and the
 * same exit status, as the evaluator's E0300. */
void elya_int_overflow(void) {
    fputs("elya: integer overflow\n", stderr);
    exit(1);
}

void elya_div_zero(void) {
    fputs("elya: division by zero\n", stderr);
    exit(1);
}

void elya_rem_zero(void) {
    fputs("elya: remainder by zero\n", stderr);
    exit(1);
}

/* A perform whose handler has no clause for it. The type checker makes this
 * unreachable; a guard, never `unreachable`. */
void elya_unhandled_effect(void) {
    fputs("elya: perform: the handler has no clause for this operation\n", stderr);
    exit(1);
}

/* Gated OFF by default, and that is load-bearing: every existing execution
 * test asserts stderr is empty, so an unconditional dump would break the whole
 * corpus at once. */
void elya_gc_report(void) {
    if (getenv("ELY_GC_STATS")) {
        fprintf(stderr,
                "elya-gc: collections=%lld freed=%lld live=%lld words_since_gc=%lld\n",
                (long long)gc_collections, (long long)gc_freed, (long long)gc_live,
                (long long)gc_allocated);
    }
}

/* N6 §6.2: stdout is the program-output channel; stderr remains reserved for
 * collector diagnostics and runtime failures. */
void elya_println(void *s) {
#ifdef _WIN32
    _setmode(_fileno(stdout), _O_BINARY);
#endif
    int64_t *p = (int64_t *)s;
    int64_t len = p[1];
    const char *bytes = (const char *)&p[2];
    fwrite(bytes, 1, (size_t)len, stdout);
    fputc('\n', stdout);
}

/* Native multi-shot (spec 2026-10-10, D2-D4). A resume of a `with multi`
 * handle re-enters a COPY of its captured frames, so the original chain is
 * never run and any number of later resumes see it exactly as captured. Site
 * frames are never written after allocation, but handler frames are: every
 * resume rewrites its frame's `next` and `parent` (5b-10), including the
 * frames of handles INSIDE the captured segment. So the whole chain from `k`
 * through `next` (word 2) to the answering frame `h` is copied, each copy's
 * `next` relinked to the next copy, and the `parent` (word 4) of every copied
 * handler frame -- recognised by tag, `[hlo, hhi)` -- that points into the
 * segment redirected to the corresponding copy, as is the continuation's
 * `innermost`. Copies are shallow (D4): saved values are shared, as the
 * evaluator shares them. Returns a fresh continuation object
 * `[tag][k'][h'][innermost']`. The caller has rooted what it still needs;
 * the continuation and every copy are rooted here while the copy allocates. */
static void elya_chain_broken(void) {
    fputs("elya: resume: a continuation's frame chain does not reach its handler\n", stderr);
    exit(1);
}

#define CONT_NEXT 2
#define CONT_PARENT 4

void *elya_cont_copy(void *cont, int64_t hlo, int64_t hhi) {
    int64_t *c = (int64_t *)cont;
    int64_t *k = (int64_t *)(intptr_t)c[1];
    int64_t *h = (int64_t *)(intptr_t)c[2];
    int64_t n = 0;
    for (int64_t *f = k;; f = (int64_t *)(intptr_t)f[CONT_NEXT]) {
        if (!f) {
            elya_chain_broken();
        }
        n++;
        if (f == h) {
            break;
        }
    }
    int64_t **orig = (int64_t **)malloc((size_t)n * sizeof(int64_t *));
    int64_t **copy = (int64_t **)malloc((size_t)n * sizeof(int64_t *));
    /* Open addressing, original frame address -> its index: a `parent` or
     * `innermost` is mapped in O(1), so a chain with many handler frames does
     * not copy in quadratic time. */
    int64_t cap = 16;
    while (cap < 2 * n) {
        cap *= 2;
    }
    int64_t **keys = (int64_t **)calloc((size_t)cap, sizeof(int64_t *));
    int64_t *vals = (int64_t *)malloc((size_t)cap * sizeof(int64_t));
    if (!orig || !copy || !keys || !vals) {
        fputs("elya: out of memory copying a continuation\n", stderr);
        exit(1);
    }
    int64_t i = 0;
    for (int64_t *f = k; i < n; f = (int64_t *)(intptr_t)f[CONT_NEXT]) {
        orig[i] = f;
        uint64_t s = ((uint64_t)(uintptr_t)f >> 3) & (uint64_t)(cap - 1);
        while (keys[s]) {
            s = (s + 1) & (uint64_t)(cap - 1);
        }
        keys[s] = f;
        vals[s] = i;
        i++;
    }
    elya_gc_push(cont);
    for (i = 0; i < n; i++) {
        int64_t words = (int64_t)(gc_block_of(orig[i])->meta >> 1);
        int64_t *p = (int64_t *)elya_alloc(words);
        memcpy(p, orig[i], (size_t)words * 8);
        copy[i] = p;
        elya_gc_push(p);
    }
#define LOOKUP(ptr, out)                                                        \
    do {                                                                        \
        int64_t *q_ = (ptr);                                                    \
        (out) = -1;                                                             \
        if (q_) {                                                               \
            uint64_t s_ = ((uint64_t)(uintptr_t)q_ >> 3) & (uint64_t)(cap - 1); \
            while (keys[s_]) {                                                  \
                if (keys[s_] == q_) {                                           \
                    (out) = vals[s_];                                           \
                    break;                                                      \
                }                                                               \
                s_ = (s_ + 1) & (uint64_t)(cap - 1);                            \
            }                                                                   \
        }                                                                       \
    } while (0)
    for (i = 0; i < n; i++) {
        if (i + 1 < n) {
            copy[i][CONT_NEXT] = (int64_t)(intptr_t)copy[i + 1];
        }
        int64_t tag = copy[i][0];
        if (tag >= hlo && tag < hhi) {
            int64_t j;
            LOOKUP((int64_t *)(intptr_t)copy[i][CONT_PARENT], j);
            if (j >= 0) {
                copy[i][CONT_PARENT] = (int64_t)(intptr_t)copy[j];
            }
        }
    }
    int64_t *fresh = (int64_t *)elya_alloc(4);
    fresh[0] = c[0];
    fresh[1] = (int64_t)(intptr_t)copy[0];
    fresh[2] = (int64_t)(intptr_t)copy[n - 1];
    int64_t j;
    LOOKUP((int64_t *)(intptr_t)c[3], j);
    fresh[3] = j >= 0 ? (int64_t)(intptr_t)copy[j] : c[3];
#undef LOOKUP
    for (i = 0; i <= n; i++) {
        elya_gc_pop();
    }
    free(orig);
    free(copy);
    free(keys);
    free(vals);
    return fresh;
}

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

static void elya_stack_unavailable(void) {
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
    elya_entry_ptr = entry;
    AddVectoredExceptionHandler(1, elya_overflow_handler);
    HANDLE t = CreateThread(NULL, (SIZE_T)elya_stack_mib << 20, elya_program_thread, NULL,
                            STACK_SIZE_PARAM_IS_A_RESERVATION, NULL);
    if (!t) {
        elya_stack_unavailable();
    }
    WaitForSingleObject(t, INFINITE);
    CloseHandle(t);
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
    return elya_entry_result;
}
#endif
