#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <math.h>
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
/* Collections that ran inside a multi-shot copy (elya_cont_copy sets the
 * flag), reported as `mid_copy=` (a name without `collections=` in it, which
 * a test reads as a substring) with the other counters: a test asserts the copy really is
 * interrupted (spec 2026-10-22-elya-multi-shot-sharing, the review's note 1). */
static int64_t gc_copy_collections = 0;
static int gc_in_cont_copy = 0;

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

/* One float mask per declared constructor tag: bit i set iff field i is a
 * `Float` (spec 2026-10-23-elya-float-natively D2). Read by `elya_eq` only; the
 * collector's rows and `gc_mark` do not change. */
static const int64_t *eq_float_masks = NULL;
static int64_t eq_n_masks = 0;

void elya_eq_init(const int64_t *masks, int64_t n) {
    eq_float_masks = masks;
    eq_n_masks = n;
}

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

/* Defined with the handler protocol below. */
extern void *elya_current_handler;

static void gc_collect(void) {
    gc_collections++;
    /* The current handler is a root (spec 2026-10-22-elya-multi-shot-sharing
     * D3): a handle's body ends at the shared end frame, not at its handler
     * frame, so the running chain no longer reaches the frame that is current.
     * Grayed here, before the shadow stack, so gc_mark itself is unchanged. */
    gc_gray_push(elya_current_handler);
    if (gc_in_cont_copy) {
        gc_copy_collections++;
    }
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

/* A string by byte (spec 2026-10-24-elya-trace-text D1), with the
 * evaluator's words for every failure. `[tag, length, bytes..., NUL]`: the
 * length and any byte are one load each. */
int64_t elya_str_length(void *s) { return ((int64_t *)s)[1]; }

int64_t elya_str_byte_at(void *s, int64_t i) {
    int64_t len = ((int64_t *)s)[1];
    if (i < 0 || i >= len) {
        fprintf(stderr, "elya: string.byte_at: index %lld is outside a string of %lld bytes\n",
                (long long)i, (long long)len);
        exit(1);
    }
    return ((const unsigned char *)&((int64_t *)s)[2])[i];
}

/* Rust's `str::is_char_boundary` for `i` in [0, len]: not a continuation byte. */
static int elya_str_boundary(const unsigned char *b, int64_t len, int64_t i) {
    return i == 0 || i == len || (b[i] & 0xC0) != 0x80;
}

/* `string.slice(s, from, to)`: a fresh string of the bytes [from, to). The
 * range is checked first, then both ends against UTF-8 boundaries (the
 * evaluator's order). `s` is rooted while the slice allocates. */
void *elya_str_slice(int64_t tag, void *s, int64_t from, int64_t to) {
    int64_t len = ((int64_t *)s)[1];
    if (!(0 <= from && from <= to && to <= len)) {
        fprintf(stderr, "elya: string.slice: %lld..%lld is not a range in a string of %lld bytes\n",
                (long long)from, (long long)to, (long long)len);
        exit(1);
    }
    const unsigned char *b = (const unsigned char *)&((int64_t *)s)[2];
    if (!elya_str_boundary(b, len, from) || !elya_str_boundary(b, len, to)) {
        fprintf(stderr, "elya: string.slice: %lld..%lld splits a UTF-8 character\n",
                (long long)from, (long long)to);
        exit(1);
    }
    int64_t n = to - from;
    elya_gc_push(s);
    int64_t *p = (int64_t *)elya_alloc(2 + ((n + 1) + 7) / 8);
    elya_gc_pop();
    p[0] = tag;
    p[1] = n;
    memcpy(&p[2], (const char *)&((int64_t *)s)[2] + from, (size_t)n);
    return p;
}

/* `==` on data (spec 2026-10-16-elya-structural-equality, D3): 1 when `a` and
 * `b` are structurally equal. The descriptor table says which fields are heap
 * values (compared as values) and which are words (compared as words); a
 * string compares its length and bytes. An explicit worklist of pairs, never
 * recursion, so two long lists compare in flat host stack. Allocates nothing
 * from the collector, so nothing needs rooting across it. The type checker
 * (E0436) keeps functions and continuations out of declared data, but not
 * out of an operation's arguments (an operation value is data to the checker,
 * and its arguments are checked at run time): a closure, a tag in
 * [first_closure_tag, string_tag), stops the walk with the evaluator's words
 * where the evaluator's walk would reach it (spec
 * 2026-10-23-elya-operation-values-natively, the review's F1). */
int64_t elya_eq(void *a, void *b, int64_t string_tag, int64_t first_closure_tag) {
    /* Triples: the two words and whether they are heap values (1), plain
     * words (0) or floats (2, compared as doubles: a NaN is unequal to itself
     * and -0.0 equals 0.0, spec 2026-10-23-elya-float-natively D2). Fields are pushed in order and popped last first, the
     * evaluator's order, so the first difference or function reached is the
     * evaluator's too (a word field no longer decides before its later heap
     * siblings are walked: the review's F1). */
    int64_t *work = NULL;
    int64_t top = 0, cap = 0;
    int64_t equal = 1;
#define EQ_PUSH(x, y, heap)                                                            \
    do {                                                                               \
        if (top + 3 > cap) {                                                           \
            cap = cap ? cap * 2 : 96;                                                  \
            int64_t *grown = (int64_t *)realloc(work, (size_t)cap * sizeof *work);     \
            if (!grown) {                                                              \
                fputs("elya: out of memory comparing values\n", stderr);               \
                exit(1);                                                               \
            }                                                                          \
            work = grown;                                                              \
        }                                                                              \
        work[top++] = (int64_t)(x);                                                    \
        work[top++] = (int64_t)(y);                                                    \
        work[top++] = (heap);                                                          \
    } while (0)
    EQ_PUSH((intptr_t)a, (intptr_t)b, 1);
    while (top > 0) {
        int64_t heap = work[--top];
        int64_t yw = work[--top];
        int64_t xw = work[--top];
        if (heap == 2) {
            double xd, yd;
            memcpy(&xd, &xw, sizeof xd);
            memcpy(&yd, &yw, sizeof yd);
            if (!(xd == yd)) {
                equal = 0;
                break;
            }
            continue;
        }
        if (!heap) {
            if (xw != yw) {
                equal = 0;
                break;
            }
            continue;
        }
        int64_t *x = (int64_t *)(intptr_t)xw;
        int64_t *y = (int64_t *)(intptr_t)yw;
        /* No shortcut for one shared object: whether two values are shared
         * must never decide `==` (a NaN field, once floats compile natively,
         * is not equal to itself), and the evaluator takes none. */
        if (!x || !y) {
            equal = 0;
            break;
        }
        int64_t tag = x[0];
        if (tag >= first_closure_tag && tag < string_tag) {
            fputs("elya: `==` reached a value that is not data (E0436 should have rejected it)\n",
                  stderr);
            exit(1);
        }
        if (x[0] != y[0]) {
            equal = 0;
            break;
        }
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
        int64_t fmask = tag < eq_n_masks ? eq_float_masks[tag] : 0;
        for (int64_t f = 0; f < arity; f++) {
            EQ_PUSH(x[1 + f], y[1 + f], ((mask >> f) & 1) ? 1 : (((fmask >> f) & 1) ? 2 : 0));
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

/* ---- Floats (spec 2026-10-23-elya-float-natively D3): the literal text. -----
 *
 * The shortest digit string that reads back to the same double, the closest
 * such string when several are equally short, written as a plain decimal (no
 * exponent), `.0` when integral; `inf`, `-inf`, `nan` (a NaN's sign is dropped).
 * That is the evaluator's text, Rust's `Display` plus `.0`. For each length p
 * from 1 to 17 the correctly rounded p-digit string (`%.*e`) is tried first:
 * if any p-digit string reads back, it does unless x sits next to a power of
 * two, where the gap below is half the gap above and the closest string can
 * fall outside; then the one candidate on the other side, the closest string's
 * neighbour in the last digit, is tried. */

/* Does D (n digits, a decimal point after the first) times 10^e read back as x? */
static int float_reads_back(const char *d, int n, int e, double x) {
    char buf[64];
    int k = 0;
    buf[k++] = d[0];
    buf[k++] = '.';
    for (int i = 1; i < n; i++) {
        buf[k++] = d[i];
    }
    k += snprintf(buf + k, sizeof buf - (size_t)k, "e%d", e);
    return strtod(buf, NULL) == x;
}

/* Writes `bits`' literal text to `out` (room for 1100 bytes); returns its length. */
int64_t elya_float_text(int64_t bits, char *out) {
    double x;
    memcpy(&x, &bits, sizeof x);
    if (isnan(x)) {
        memcpy(out, "nan", 3);
        return 3;
    }
    int64_t k = 0;
    if (signbit(x)) {
        out[k++] = '-';
        x = -x;
    }
    if (isinf(x)) {
        memcpy(out + k, "inf", 3);
        return k + 3;
    }
    if (x == 0.0) {
        memcpy(out + k, "0.0", 3);
        return k + 3;
    }
    /* The exact decimal expansion (a double has at most 767 significant
     * digits), then each length's candidate rounded from it half UP, as Rust
     * rounds a tie (2^-25 is ...953125 and writes ...95313, where `%.*e`'s
     * half-even would give ...95312). */
    char ex[800];
    snprintf(ex, sizeof ex, "%.780e", x);
    char all[790];
    int na = 0;
    const char *c = ex;
    for (; *c && *c != 'e'; c++) {
        if (*c >= '0' && *c <= '9') {
            all[na++] = *c;
        }
    }
    int e0 = atoi(c + 1);
    char d[24];
    int n = 0, e = 0;
    for (int p = 1; p <= 17 && n == 0; p++) {
        uint64_t v = 0;
        for (int i = 0; i < p; i++) {
            v = v * 10 + (uint64_t)(all[i] - '0');
        }
        /* Below, at, or above x: the digits after the first p. */
        int up = all[p] >= '5';
        int exact = 1;
        for (int i = p; i < na; i++) {
            if (all[i] != '0') {
                exact = 0;
                break;
            }
        }
        if (up) {
            v += 1;
        }
        for (int attempt = 0; attempt < 2 && n == 0; attempt++) {
            if (attempt == 1) {
                /* The neighbour on the far side of x from the closest string. */
                if (exact) {
                    break;
                }
                if (up) {
                    v -= 1; /* the truncated string: below x */
                } else {
                    v += 1; /* one above the truncated string: above x */
                }
            }
            char nd[24];
            int nm = snprintf(nd, sizeof nd, "%llu", (unsigned long long)v);
            /* A carry (999 -> 1000) moves the exponent. */
            int ne = e0 + (nm - p);
            if (v != 0 && float_reads_back(nd, nm, ne, x)) {
                memcpy(d, nd, (size_t)nm);
                n = nm;
                e = ne;
            }
        }
    }
    if (n == 0) {
        /* 17 digits always read back; unreachable. */
        n = 1;
        d[0] = '0';
    }
    while (n > 1 && d[n - 1] == '0') {
        n--;
    }
    /* Plain decimal: the value is d[0].d[1..n] * 10^e. */
    if (e >= n - 1) {
        memcpy(out + k, d, (size_t)n);
        k += n;
        for (int i = 0; i < e - (n - 1); i++) {
            out[k++] = '0';
        }
        memcpy(out + k, ".0", 2);
        k += 2;
    } else if (e >= 0) {
        memcpy(out + k, d, (size_t)(e + 1));
        k += e + 1;
        out[k++] = '.';
        memcpy(out + k, d + e + 1, (size_t)(n - e - 1));
        k += n - e - 1;
    } else {
        out[k++] = '0';
        out[k++] = '.';
        for (int i = 0; i < -e - 1; i++) {
            out[k++] = '0';
        }
        memcpy(out + k, d, (size_t)n);
        k += n;
    }
    return k;
}

/* `float.to_string` (D5): the literal text, as a fresh string. */
void *elya_float_to_string(int64_t tag, int64_t bits) {
    char buf[1100];
    int64_t len = elya_float_text(bits, buf);
    return elya_str_lit(tag, buf, len);
}

/* ---- `with any` natively: literals (spec 2026-10-17-elya-native-any-handlers,
 * D3/D4). The back end hands over a reflection table (`reflect.rs`): a node is
 * an offset into `t`; `[0]` Int, `[1]` Bool, `[2]` Unit, `[3]` String, `[4]` a
 * function, `[5, n, (tag, name_off, name_len, arity, field_node..) * n]` a
 * declared type (source constructor names in `names`), `[6]` an instance no
 * value of exists. Every walk keeps its own stack: a million-cell list neither
 * recurses nor overflows. Failures carry the evaluator's words. */

enum { ANY_INT, ANY_BOOL, ANY_UNIT, ANY_STRING, ANY_FN, ANY_ADT, ANY_UNBUILT, ANY_FLOAT };

static void any_oom(void) {
    fputs("elya: out of memory\n", stderr);
    exit(1);
}

static void *any_grow(void *p, int64_t *cap, int64_t need, size_t elem) {
    if (need <= *cap) {
        return p;
    }
    int64_t c = *cap ? *cap : 16;
    while (c < need) {
        c *= 2;
    }
    void *q = realloc(p, (size_t)c * elem);
    if (!q) {
        any_oom();
    }
    *cap = c;
    return q;
}

typedef struct {
    char *p;
    int64_t len, cap;
} AnyBuf;

static void any_put(AnyBuf *b, const char *s, int64_t n) {
    b->p = (char *)any_grow(b->p, &b->cap, b->len + n + 1, 1);
    memcpy(b->p + b->len, s, (size_t)n);
    b->len += n;
}

static void any_puts(AnyBuf *b, const char *s) { any_put(b, s, (int64_t)strlen(s)); }

/* The constructor entry of `tag` in the declared-type node `node`, or -1. */
static int64_t any_ctor(const int64_t *t, int64_t node, int64_t tag) {
    int64_t n = t[node + 1];
    int64_t at = node + 2;
    for (int64_t i = 0; i < n; i++) {
        if (t[at] == tag) {
            return at;
        }
        at += 4 + t[at + 3];
    }
    return -1;
}

static void any_internal(const char *what) {
    fprintf(stderr, "elya: internal: %s\n", what);
    exit(1);
}

/* `value` (a word of the type `node`) in literal syntax, as a fresh string.
 * Reaching a function fails with `before` "a function" `after`. */
void *elya_any_show(int64_t string_tag, const int64_t *t, const char *names, int64_t node,
                    int64_t value, const char *before, const char *after) {
    typedef struct {
        int64_t node, value;
        const char *text;
    } Item;
    Item *stack = NULL;
    int64_t top = 0, cap = 0;
    AnyBuf out = {0};
    stack = (Item *)any_grow(stack, &cap, 1, sizeof(Item));
    stack[top++] = (Item){node, value, NULL};
    while (top > 0) {
        Item it = stack[--top];
        if (it.text) {
            any_puts(&out, it.text);
            continue;
        }
        char num[32];
        switch (t[it.node]) {
        case ANY_INT:
            any_put(&out, num, snprintf(num, sizeof num, "%lld", (long long)it.value));
            break;
        case ANY_FLOAT: {
            char fbuf[1100];
            any_put(&out, fbuf, elya_float_text(it.value, fbuf));
            break;
        }
        case ANY_BOOL:
            any_puts(&out, it.value ? "True" : "False");
            break;
        case ANY_UNIT:
            any_puts(&out, "Unit");
            break;
        case ANY_STRING: {
            int64_t *s = (int64_t *)(intptr_t)it.value;
            int64_t len = s[1];
            const unsigned char *bytes = (const unsigned char *)&s[2];
            any_puts(&out, "\"");
            for (int64_t i = 0; i < len; i++) {
                switch (bytes[i]) {
                case '"': any_puts(&out, "\\\""); break;
                case '\\': any_puts(&out, "\\\\"); break;
                case '\n': any_puts(&out, "\\n"); break;
                case '\t': any_puts(&out, "\\t"); break;
                case '\r': any_puts(&out, "\\r"); break;
                default: any_put(&out, (const char *)&bytes[i], 1);
                }
            }
            any_puts(&out, "\"");
            break;
        }
        case ANY_FN:
            /* The program's output first, as from the evaluator (the
             * main-values review's L1). */
            fflush(stdout);
            fprintf(stderr, "elya: %sa function%s\n", before, after);
            exit(1);
        case ANY_ADT: {
            int64_t *p = (int64_t *)(intptr_t)it.value;
            int64_t c = any_ctor(t, it.node, p[0]);
            if (c < 0) {
                any_internal("op: a value without a descriptor");
            }
            any_put(&out, names + t[c + 1], t[c + 2]);
            int64_t arity = t[c + 3];
            if (arity > 0) {
                any_puts(&out, "(");
                stack = (Item *)any_grow(stack, &cap, top + 2 * arity + 1, sizeof(Item));
                stack[top++] = (Item){0, 0, ")"};
                for (int64_t i = arity - 1; i >= 0; i--) {
                    stack[top++] = (Item){t[c + 4 + i], p[1 + i], NULL};
                    if (i > 0) {
                        stack[top++] = (Item){0, 0, ", "};
                    }
                }
            }
            break;
        }
        default:
            any_internal("op: a value of a type with no instance");
        }
    }
    free(stack);
    void *s = elya_str_lit(string_tag, out.p ? out.p : "", out.len);
    free(out.p);
    return s;
}

/* `o == o` (the review's R2): an operation equals itself unless the comparison
 * reaches a function in an argument, which stops the evaluator's `==` with
 * these words. Walks `value` (a word of type `node`); allocates nothing. */
int64_t elya_any_check(const int64_t *t, int64_t node, int64_t value) {
    typedef struct {
        int64_t node, value;
    } Item;
    Item *stack = NULL;
    int64_t top = 0, cap = 0;
    stack = (Item *)any_grow(stack, &cap, 1, sizeof(Item));
    stack[top++] = (Item){node, value};
    while (top > 0) {
        Item it = stack[--top];
        switch (t[it.node]) {
        case ANY_FN:
            fputs("elya: `==` reached a value that is not data (E0436 should have rejected it)\n",
                  stderr);
            exit(1);
        case ANY_ADT: {
            int64_t *p = (int64_t *)(intptr_t)it.value;
            int64_t c = any_ctor(t, it.node, p[0]);
            if (c < 0) {
                any_internal("op: a value without a descriptor");
            }
            int64_t arity = t[c + 3];
            stack = (Item *)any_grow(stack, &cap, top + arity, sizeof(Item));
            for (int64_t i = 0; i < arity; i++) {
                stack[top++] = (Item){t[c + 4 + i], p[1 + i]};
            }
            break;
        }
        case ANY_UNBUILT:
            any_internal("op: a value of a type with no instance");
            break;
        case ANY_FLOAT: {
            /* A NaN is unequal to itself: the evaluator's `==` stops there,
             * false (spec 2026-10-23-elya-float-natively D2). */
            double v;
            memcpy(&v, &it.value, sizeof v);
            if (isnan(v)) {
                free(stack);
                return 0;
            }
            break;
        }
        default:
            break;
        }
    }
    free(stack);
    return 1;
}

/* `op.decode`'s refusal of an answer at an effect's own parameter (D4). */
void elya_any_fail(const char *msg) {
    fprintf(stderr, "elya: %s\n", msg);
    exit(1);
}

/* A parsed literal: the evaluator's `Value`s a literal can denote. */
enum { LIT_INT, LIT_FLOAT, LIT_BOOL, LIT_UNIT, LIT_STR, LIT_CTOR };

typedef struct {
    int kind;
    int64_t i;          /* Int, Bool; a Float's bits */
    char *s;            /* String bytes (malloc'd); a constructor's name (into the text) */
    int64_t slen;
    int64_t *kids;      /* a constructor's arguments, as indices */
    int64_t nkids;
} AnyLit;

typedef struct {
    const char *s;
    int64_t i, n;
    AnyLit *lits;
    int64_t nlits, cap;
} AnyParser;

static int64_t any_new(AnyParser *p, AnyLit l) {
    p->lits = (AnyLit *)any_grow(p->lits, &p->cap, p->nlits + 1, sizeof(AnyLit));
    p->lits[p->nlits] = l;
    return p->nlits++;
}

static int any_space(char c) {
    /* Rust's `is_ascii_whitespace`: space, \t, \n, \x0C, \r. */
    return c == ' ' || c == '\t' || c == '\n' || c == '\x0C' || c == '\r';
}

static void any_ws(AnyParser *p) {
    while (p->i < p->n && any_space(p->s[p->i])) {
        p->i++;
    }
}

static int any_starts(AnyParser *p, const char *w) {
    int64_t k = (int64_t)strlen(w);
    return p->n - p->i >= k && memcmp(p->s + p->i, w, (size_t)k) == 0;
}

static int any_digit(char c) { return c >= '0' && c <= '9'; }

static int any_alnum(char c) {
    return any_digit(c) || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z');
}

/* A decimal Int in range, as Rust's `i64::from_str` reads `[-]digits`. */
static int any_int(const char *s, int64_t n, int64_t *out) {
    int64_t k = 0;
    int neg = 0;
    if (k < n && s[k] == '-') {
        neg = 1;
        k++;
    }
    if (k == n) {
        return 0;
    }
    uint64_t limit = neg ? (uint64_t)INT64_MAX + 1 : (uint64_t)INT64_MAX;
    uint64_t v = 0;
    for (; k < n; k++) {
        uint64_t d = (uint64_t)(s[k] - '0');
        if (v > (limit - d) / 10) {
            return 0;
        }
        v = v * 10 + d;
    }
    *out = neg ? (int64_t)(0 - v) : (int64_t)v;
    return 1;
}

/* The evaluator's literal parser (`literal.rs`), the same errors in the same
 * order. Returns the literal's index, or -1 with `*err` set. */
static int64_t any_parse(AnyParser *p, const char **err) {
    typedef struct {
        const char *name;
        int64_t len;
        int64_t *args;
        int64_t nargs, cap;
    } Open;
    Open *open = NULL;
    int64_t nopen = 0, ocap = 0;
    int64_t v = -1;
    for (;;) {
        any_ws(p);
        if (p->i >= p->n) {
            *err = "nothing to read";
            goto fail;
        }
        char c = p->s[p->i];
        if (c == '"') {
            p->i++;
            char *bytes = NULL;
            int64_t len = 0, bcap = 0;
            for (;;) {
                if (p->i >= p->n) {
                    free(bytes);
                    *err = "an unterminated string";
                    goto fail;
                }
                char d = p->s[p->i];
                if (d == '"') {
                    p->i++;
                    break;
                }
                if (d == '\\') {
                    p->i++;
                    if (p->i >= p->n) {
                        free(bytes);
                        *err = "an unterminated string";
                        goto fail;
                    }
                    char e = p->s[p->i];
                    char out;
                    switch (e) {
                    case '"': out = '"'; break;
                    case '\\': out = '\\'; break;
                    case 'n': out = '\n'; break;
                    case 't': out = '\t'; break;
                    case 'r': out = '\r'; break;
                    default:
                        free(bytes);
                        *err = "an unknown escape";
                        goto fail;
                    }
                    d = out;
                }
                bytes = (char *)any_grow(bytes, &bcap, len + 1, 1);
                bytes[len++] = d;
                p->i++;
            }
            /* The bytes between two quotes of UTF-8 text, with ASCII escapes
             * replaced, are UTF-8: the evaluator's "not UTF-8" cannot occur. */
            v = any_new(p, (AnyLit){LIT_STR, 0, bytes, len, NULL, 0});
        } else if (c == '-' || any_digit(c)) {
            if (any_starts(p, "-inf")) {
                p->i += 4;
                double ninf = -INFINITY;
                int64_t nb;
                memcpy(&nb, &ninf, sizeof nb);
                v = any_new(p, (AnyLit){LIT_FLOAT, nb, NULL, 0, NULL, 0});
            } else {
                int64_t start = p->i;
                int dots = 0, digits = 0;
                if (p->s[p->i] == '-') {
                    p->i++;
                }
                while (p->i < p->n && (any_digit(p->s[p->i]) || p->s[p->i] == '.')) {
                    if (p->s[p->i] == '.') {
                        dots++;
                    } else {
                        digits++;
                    }
                    p->i++;
                }
                if (dots > 0) {
                    /* Rust's `f64::from_str` on `[-][digits][.][digits]`: one point
                     * and at least one digit. */
                    if (dots != 1 || digits == 0) {
                        *err = "a malformed number";
                        goto fail;
                    }
                    /* `strtod` rounds correctly, as Rust's `from_str` (D4). */
                    char nbuf[512];
                    char *big = NULL;
                    int64_t len = p->i - start;
                    char *at = nbuf;
                    if (len + 1 > (int64_t)sizeof nbuf) {
                        big = (char *)malloc((size_t)len + 1);
                        if (!big) {
                            any_oom();
                        }
                        at = big;
                    }
                    memcpy(at, p->s + start, (size_t)len);
                    at[len] = 0;
                    double fv = strtod(at, NULL);
                    free(big);
                    int64_t fb;
                    memcpy(&fb, &fv, sizeof fb);
                    v = any_new(p, (AnyLit){LIT_FLOAT, fb, NULL, 0, NULL, 0});
                } else {
                    int64_t n;
                    if (!any_int(p->s + start, p->i - start, &n)) {
                        *err = "a malformed or out-of-range integer";
                        goto fail;
                    }
                    v = any_new(p, (AnyLit){LIT_INT, n, NULL, 0, NULL, 0});
                }
            }
        } else if (c == 'i' && any_starts(p, "inf")) {
            p->i += 3;
            double pinf = INFINITY;
            int64_t pb;
            memcpy(&pb, &pinf, sizeof pb);
            v = any_new(p, (AnyLit){LIT_FLOAT, pb, NULL, 0, NULL, 0});
        } else if (c == 'n' && any_starts(p, "nan")) {
            p->i += 3;
            double qnan = NAN;
            int64_t qb;
            memcpy(&qb, &qnan, sizeof qb);
            v = any_new(p, (AnyLit){LIT_FLOAT, qb, NULL, 0, NULL, 0});
        } else if (c >= 'A' && c <= 'Z') {
            int64_t start = p->i;
            while (p->i < p->n && (any_alnum(p->s[p->i]) || p->s[p->i] == '_')) {
                p->i++;
            }
            const char *name = p->s + start;
            int64_t len = p->i - start;
            int64_t save = p->i;
            any_ws(p);
            if (p->i < p->n && p->s[p->i] == '(') {
                p->i++;
                open = (Open *)any_grow(open, &ocap, nopen + 1, sizeof(Open));
                open[nopen++] = (Open){name, len, NULL, 0, 0};
                continue;
            }
            p->i = save;
            if (len == 4 && memcmp(name, "True", 4) == 0) {
                v = any_new(p, (AnyLit){LIT_BOOL, 1, NULL, 0, NULL, 0});
            } else if (len == 5 && memcmp(name, "False", 5) == 0) {
                v = any_new(p, (AnyLit){LIT_BOOL, 0, NULL, 0, NULL, 0});
            } else if (len == 4 && memcmp(name, "Unit", 4) == 0) {
                v = any_new(p, (AnyLit){LIT_UNIT, 0, NULL, 0, NULL, 0});
            } else {
                v = any_new(p, (AnyLit){LIT_CTOR, 0, (char *)name, len, NULL, 0});
            }
        } else {
            *err = "an unexpected character";
            goto fail;
        }
        /* Give the value to the constructor it is an argument of, closing every
         * constructor whose `)` follows. */
        for (;;) {
            if (nopen == 0) {
                free(open);
                return v;
            }
            Open *o = &open[nopen - 1];
            o->args = (int64_t *)any_grow(o->args, &o->cap, o->nargs + 1, sizeof(int64_t));
            o->args[o->nargs++] = v;
            any_ws(p);
            if (p->i < p->n && p->s[p->i] == ',') {
                p->i++;
                break;
            }
            if (p->i < p->n && p->s[p->i] == ')') {
                p->i++;
                nopen--;
                v = any_new(p, (AnyLit){LIT_CTOR, 0, (char *)o->name, o->len, o->args, o->nargs});
                continue;
            }
            *err = "a constructor's arguments not closed by `)`";
            goto fail;
        }
    }
fail:
    for (int64_t k = 0; k < nopen; k++) {
        free(open[k].args);
    }
    free(open);
    return -1;
}

static void any_free(AnyParser *p) {
    for (int64_t k = 0; k < p->nlits; k++) {
        if (p->lits[k].kind == LIT_STR) {
            free(p->lits[k].s);
        }
        free(p->lits[k].kids);
    }
    free(p->lits);
}

/* Does literal `lit` denote a value of the type `node`? Iterative. */
static int any_conforms(AnyParser *p, const int64_t *t, const char *names, int64_t lit,
                        int64_t node, int64_t **centry) {
    typedef struct {
        int64_t lit, node;
    } Pair;
    Pair *work = NULL;
    int64_t top = 0, cap = 0;
    work = (Pair *)any_grow(work, &cap, 1, sizeof(Pair));
    work[top++] = (Pair){lit, node};
    int ok = 1;
    while (ok && top > 0) {
        Pair w = work[--top];
        AnyLit *l = &p->lits[w.lit];
        switch (t[w.node]) {
        case ANY_INT: ok = l->kind == LIT_INT; break;
        case ANY_FLOAT: ok = l->kind == LIT_FLOAT; break;
        case ANY_BOOL: ok = l->kind == LIT_BOOL; break;
        case ANY_UNIT: ok = l->kind == LIT_UNIT; break;
        case ANY_STRING: ok = l->kind == LIT_STR; break;
        case ANY_FN: ok = 0; break;
        case ANY_ADT: {
            ok = 0;
            if (l->kind != LIT_CTOR) {
                break;
            }
            int64_t n = t[w.node + 1];
            int64_t at = w.node + 2;
            for (int64_t k = 0; k < n; k++) {
                if (t[at + 2] == l->slen && memcmp(names + t[at + 1], l->s, (size_t)l->slen) == 0) {
                    if (t[at + 3] == l->nkids) {
                        ok = 1;
                        (*centry)[w.lit] = at;
                        work = (Pair *)any_grow(work, &cap, top + l->nkids, sizeof(Pair));
                        for (int64_t f = 0; f < l->nkids; f++) {
                            work[top++] = (Pair){l->kids[f], t[at + 4 + f]};
                        }
                    }
                    break;
                }
                at += 4 + t[at + 3];
            }
            break;
        }
        default:
            any_internal("op.decode: a type with no instance");
        }
    }
    free(work);
    return ok;
}

static void any_decode_fail(const char *why1, const char *text, int64_t tlen, const char *why2,
                            const char *opname) {
    fprintf(stderr, "elya: op.decode: %s", why1);
    fwrite(text, 1, (size_t)tlen, stderr);
    fprintf(stderr, "%s, for an answer of `%s`\n", why2, opname);
    exit(1);
}

/* `op.decode`: parse the whole text, then refuse input after the value, then
 * check the literal against the type, then build it -- the evaluator's order,
 * so the first failure named is the same. Every value built is rooted until
 * its parent holds it. */
int64_t elya_any_decode(int64_t string_tag, const int64_t *t, const char *names, int64_t node,
                        void *text, const char *display, const char *opname) {
    int64_t *ts = (int64_t *)text;
    int64_t tlen = ts[1];
    const char *src = (const char *)&ts[2];
    AnyParser p = {src, 0, tlen, NULL, 0, 0};
    const char *err = NULL;
    int64_t root = any_parse(&p, &err);
    if (root < 0) {
        fprintf(stderr, "elya: op.decode: `");
        fwrite(src, 1, (size_t)tlen, stderr);
        fprintf(stderr, "` is not a literal (%s), for an answer of `%s`\n", err, opname);
        exit(1);
    }
    any_ws(&p);
    if (p.i != p.n) {
        any_decode_fail("`", src, tlen, "` has input after its value", opname);
    }
    int64_t *centry = (int64_t *)calloc((size_t)(p.nlits ? p.nlits : 1), sizeof(int64_t));
    if (!centry) {
        any_oom();
    }
    if (!any_conforms(&p, t, names, root, node, &centry)) {
        fprintf(stderr, "elya: op.decode: `");
        fwrite(src, 1, (size_t)tlen, stderr);
        fprintf(stderr, "` is not a value of type `%s`, for an answer of `%s`\n", display, opname);
        exit(1);
    }
    /* Build, post-order. `done` holds built words; a heap one is also on the
     * shadow stack, in the same order, until its parent is built. */
    typedef struct {
        int64_t lit, node;
        int expanded;
    } Step;
    typedef struct {
        int64_t word;
        int heap;
    } Done;
    Step *steps = NULL;
    Done *done = NULL;
    int64_t ns = 0, scap = 0, nd = 0, dcap = 0;
    steps = (Step *)any_grow(steps, &scap, 1, sizeof(Step));
    steps[ns++] = (Step){root, node, 0};
    while (ns > 0) {
        Step *s = &steps[ns - 1];
        AnyLit *l = &p.lits[s->lit];
        int64_t kind = t[s->node];
        if (kind == ANY_ADT && !s->expanded) {
            s->expanded = 1;
            int64_t at = centry[s->lit];
            int64_t lit = s->lit;
            steps = (Step *)any_grow(steps, &scap, ns + l->nkids, sizeof(Step));
            for (int64_t f = l->nkids - 1; f >= 0; f--) {
                steps[ns++] = (Step){p.lits[lit].kids[f], t[at + 4 + f], 0};
            }
            continue;
        }
        Done d = {0, 0};
        switch (kind) {
        case ANY_INT: d.word = l->i; break;
        case ANY_FLOAT: d.word = l->i; break;
        case ANY_BOOL: d.word = l->i ? 1 : 0; break;
        case ANY_UNIT: d.word = 0; break;
        case ANY_STRING:
            d.word = (int64_t)(intptr_t)elya_str_lit(string_tag, l->s ? l->s : "", l->slen);
            d.heap = 1;
            break;
        case ANY_ADT: {
            int64_t at = centry[s->lit];
            int64_t k = l->nkids;
            int64_t *obj = (int64_t *)elya_alloc(1 + k);
            obj[0] = t[at];
            int pops = 0;
            for (int64_t f = 0; f < k; f++) {
                Done c = done[nd - k + f];
                obj[1 + f] = c.word;
                pops += c.heap;
            }
            nd -= k;
            for (int q = 0; q < pops; q++) {
                elya_gc_pop();
            }
            d.word = (int64_t)(intptr_t)obj;
            d.heap = 1;
            break;
        }
        default:
            any_internal("op.decode: a type with no instance");
        }
        if (d.heap) {
            elya_gc_push((void *)(intptr_t)d.word);
        }
        done = (Done *)any_grow(done, &dcap, nd + 1, sizeof(Done));
        done[nd++] = d;
        ns--;
    }
    int64_t result = done[0].word;
    if (done[0].heap) {
        elya_gc_pop();
    }
    free(steps);
    free(done);
    free(centry);
    any_free(&p);
    return result;
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

/* ---- Record and replay natively (spec 2026-10-17-elya-native-record-replay).
 * `ELYA_RECORD=<file>` writes the host's answers as the evaluator's trace,
 * entry by entry (D4); `ELYA_REPLAY=<file>` answers from one, without waiting
 * (D5). The format and every failure's words are the evaluator's
 * (`src/lib.rs`, `src/eval.rs` `host_builtin`). */
#define ELYA_TRACE_HEADER "elya trace 1"

enum { TRACE_LIVE, TRACE_RECORD, TRACE_REPLAY };
static int elya_trace_mode = TRACE_LIVE;
static FILE *elya_trace_out;
static const char *elya_trace_path;

/* A piece of the trace text: bytes and a length (a NUL byte is a character, as
 * in the evaluator's strings; the review's L1). */
typedef struct {
    const char *p;
    int64_t n;
} TraceStr;

typedef struct {
    TraceStr q, a;
} TraceEntry;
static TraceEntry *elya_trace_entries;
static int64_t elya_trace_n, elya_trace_cap, elya_trace_next;

/* The length of the whitespace character starting at `p` (before `e`), or 0:
 * Rust's `char::is_whitespace`, which `str::trim` uses -- ASCII \t..\r and
 * space, and U+0085, U+00A0, U+1680, U+2000..U+200A, U+2028, U+2029, U+202F,
 * U+205F, U+3000 in UTF-8 (the review's M2). */
static int trace_ws_at(const unsigned char *p, const unsigned char *e) {
    if (p >= e) {
        return 0;
    }
    if (*p == ' ' || (*p >= '\t' && *p <= '\r')) {
        return 1;
    }
    if (e - p >= 2 && p[0] == 0xC2 && (p[1] == 0x85 || p[1] == 0xA0)) {
        return 2;
    }
    if (e - p >= 3) {
        if (p[0] == 0xE1 && p[1] == 0x9A && p[2] == 0x80) {
            return 3;
        }
        if (p[0] == 0xE2 && p[1] == 0x80 &&
            ((p[2] >= 0x80 && p[2] <= 0x8A) || p[2] == 0xA8 || p[2] == 0xA9 || p[2] == 0xAF)) {
            return 3;
        }
        if (p[0] == 0xE2 && p[1] == 0x81 && p[2] == 0x9F) {
            return 3;
        }
        if (p[0] == 0xE3 && p[1] == 0x80 && p[2] == 0x80) {
            return 3;
        }
    }
    return 0;
}

static TraceStr trace_trim(const char *b, const char *e) {
    const unsigned char *s = (const unsigned char *)b, *t = (const unsigned char *)e;
    int k;
    while ((k = trace_ws_at(s, t)) > 0) {
        s += k;
    }
    for (;;) {
        int cut = 0;
        for (int len = 1; len <= 3 && t - s >= len; len++) {
            if (trace_ws_at(t - len, t) == len) {
                cut = len;
                break;
            }
        }
        if (!cut) {
            break;
        }
        t -= cut;
    }
    return (TraceStr){(const char *)s, (int64_t)(t - s)};
}

static int trace_is(TraceStr s, const char *lit) {
    int64_t n = (int64_t)strlen(lit);
    return s.n == n && memcmp(s.p, lit, (size_t)n) == 0;
}

static void trace_put(TraceStr s) { fwrite(s.p, 1, (size_t)s.n, stderr); }

static void trace_bad(const char *msg, int64_t line) {
    if (line > 0) {
        fprintf(stderr, "elya: trace line %lld: %s\n", (long long)line, msg);
    } else {
        fprintf(stderr, "elya: %s\n", msg);
    }
    exit(1);
}

/* Is [p, p+n) UTF-8, as Rust's `read_to_string` requires (the review's M3)? */
static int trace_utf8(const unsigned char *p, int64_t n) {
    int64_t i = 0;
    while (i < n) {
        unsigned char c = p[i];
        int64_t len;
        uint32_t cp;
        if (c < 0x80) {
            i++;
            continue;
        } else if (c >= 0xC2 && c <= 0xDF) {
            len = 2;
            cp = c & 0x1F;
        } else if (c >= 0xE0 && c <= 0xEF) {
            len = 3;
            cp = c & 0x0F;
        } else if (c >= 0xF0 && c <= 0xF4) {
            len = 4;
            cp = c & 0x07;
        } else {
            return 0;
        }
        if (n - i < len) {
            return 0;
        }
        for (int64_t k = 1; k < len; k++) {
            if ((p[i + k] & 0xC0) != 0x80) {
                return 0;
            }
            cp = (cp << 6) | (p[i + k] & 0x3F);
        }
        if ((len == 3 && cp < 0x800) || (len == 4 && (cp < 0x10000 || cp > 0x10FFFF)) ||
            (cp >= 0xD800 && cp <= 0xDFFF)) {
            return 0;
        }
        i += len;
    }
    return 1;
}

/* Reads the trace as `run_source_replaying` does: a byte-order mark skipped,
 * `lines()` (a trailing `\r` dropped), the header trimmed, blank lines
 * skipped, the first ` = ` outside quotes splitting a line, both sides trimmed. The entries
 * point into `t`, which lives as long as the process. */
static void trace_parse(const char *t, int64_t n) {
    if (n >= 3 && (unsigned char)t[0] == 0xEF && (unsigned char)t[1] == 0xBB &&
        (unsigned char)t[2] == 0xBF) {
        t += 3;
        n -= 3;
    }
    const char *end = t + n;
    const char *line = t;
    int64_t index = -1; /* -1: the header */
    while (line < end) {
        const char *nl = memchr(line, '\n', (size_t)(end - line));
        const char *le = nl ? nl : end;
        const char *next = nl ? nl + 1 : end;
        if (le > line && le[-1] == '\r') {
            le--;
        }
        if (index < 0) {
            if (!trace_is(trace_trim(line, le), ELYA_TRACE_HEADER)) {
                trace_bad("not a trace: the first line must be `" ELYA_TRACE_HEADER "`", 0);
            }
        } else if (trace_trim(line, le).n > 0) {
            /* The first ` = ` outside a string literal (spec
             * 2026-10-24-elya-trace-text D2), as `split_entry` splits. */
            const char *split = NULL;
            int in_str = 0;
            for (const char *p = line; p < le; p++) {
                if (in_str && *p == '\\') {
                    p++;
                    if (p >= le) {
                        break;
                    }
                } else if (*p == '"') {
                    in_str = !in_str;
                } else if (!in_str && p + 3 <= le && p[0] == ' ' && p[1] == '=' && p[2] == ' ') {
                    split = p;
                    break;
                }
            }
            if (!split) {
                trace_bad(in_str ? "a string is not closed" : "expected `question = answer`",
                          index + 2);
            }
            elya_trace_entries = (TraceEntry *)any_grow(elya_trace_entries, &elya_trace_cap,
                                                        elya_trace_n + 1, sizeof(TraceEntry));
            elya_trace_entries[elya_trace_n++] =
                (TraceEntry){trace_trim(line, split), trace_trim(split + 3, le)};
        }
        index++;
        line = next;
    }
    if (index < 0) {
        trace_bad("not a trace: the first line must be `" ELYA_TRACE_HEADER "`", 0);
    }
}

/* A failed write of the trace (a full disk): the CLI's exit 2 (the review's M1). */
static void trace_wrote(void) {
    if (fflush(elya_trace_out) != 0 || ferror(elya_trace_out)) {
        fprintf(stderr, "elya: cannot write the trace `%s`: %s\n", elya_trace_path, strerror(errno));
        exit(2);
    }
}

/* Before the program runs: the environment, the trace file (D1, D3). */
static void elya_trace_init(void) {
    const char *rec = getenv("ELYA_RECORD");
    const char *rep = getenv("ELYA_REPLAY");
    if (rec && rep) {
        fputs("elya: ELYA_RECORD and ELYA_REPLAY cannot both be set\n", stderr);
        exit(2);
    }
    if (rec) {
        elya_trace_path = rec;
        elya_trace_out = fopen(rec, "wb");
        if (!elya_trace_out) {
            fprintf(stderr, "elya: cannot write the trace `%s`: %s\n", rec, strerror(errno));
            exit(2);
        }
        fputs(ELYA_TRACE_HEADER "\n", elya_trace_out);
        trace_wrote();
        elya_trace_mode = TRACE_RECORD;
    } else if (rep) {
        FILE *f = fopen(rep, "rb");
        if (!f) {
            fprintf(stderr, "elya: cannot read the trace `%s`: %s\n", rep, strerror(errno));
            exit(2);
        }
        char *buf = NULL;
        int64_t len = 0, cap = 0;
        for (;;) {
            buf = (char *)any_grow(buf, &cap, len + 4096, 1);
            size_t got = fread(buf + len, 1, 4096, f);
            len += (int64_t)got;
            if (got < 4096) {
                break;
            }
        }
        if (ferror(f)) {
            fprintf(stderr, "elya: cannot read the trace `%s`: %s\n", rep, strerror(errno));
            exit(2);
        }
        fclose(f);
        if (!trace_utf8((const unsigned char *)buf, len)) {
            fprintf(stderr, "elya: cannot read the trace `%s`: stream did not contain valid UTF-8\n",
                    rep);
            exit(2);
        }
        trace_parse(buf ? buf : "", len);
        elya_trace_mode = TRACE_REPLAY;
    }
}

/* After `main` returns, before its value is printed: a replay must have asked
 * everything (the evaluator's unread-entries error). `elya_start` calls it, and
 * so does `@elya_entry` before it writes a non-`Int` value line (the main-values
 * review's F2: the line, or the failure to write it, comes after the check, as
 * from the evaluator). Writing the line reads no entry, so a second call finds
 * what the first did. */
void elya_trace_finish(void) {
    if (elya_trace_mode == TRACE_REPLAY && elya_trace_next < elya_trace_n) {
        int64_t left = elya_trace_n - elya_trace_next;
        TraceEntry *e = &elya_trace_entries[elya_trace_next];
        fflush(stdout);
        fprintf(stderr, "elya: replay: `main` returned with %lld unread trace %s, from entry %lld (`",
                (long long)left, left == 1 ? "entry" : "entries", (long long)elya_trace_next + 1);
        trace_put(e->q);
        fputs(" = ", stderr);
        trace_put(e->a);
        fputs("`): the run asked less than the recording\n", stderr);
        exit(1);
    }
}

/* The next replayed answer to `q`, or a named divergence. */
static TraceStr trace_answer(const char *q) {
    int64_t n = elya_trace_next + 1;
    if (elya_trace_next >= elya_trace_n) {
        fflush(stdout);
        fprintf(stderr, "elya: replay diverged at entry %lld: the program asked `%s`, the trace has ended\n",
                (long long)n, q);
        exit(1);
    }
    TraceEntry *e = &elya_trace_entries[elya_trace_next];
    if (!trace_is(e->q, q)) {
        fflush(stdout);
        fprintf(stderr, "elya: replay diverged at entry %lld: the program asked `%s`, the trace holds `",
                (long long)n, q);
        trace_put(e->q);
        fputs("`\n", stderr);
        exit(1);
    }
    elya_trace_next++;
    return e->a;
}

static void trace_wrong_type(TraceStr a, const char *ty) {
    fflush(stdout);
    fprintf(stderr, "elya: replay entry %lld: `", (long long)elya_trace_next);
    trace_put(a);
    fprintf(stderr, "` is not an answer of type %s\n", ty);
    exit(1);
}

/* An answer read as Rust's `i64::from_str` reads it: `[+-]digits`, in range. */
static int trace_int(TraceStr a, int64_t *out) {
    int64_t sign = (a.n > 0 && (a.p[0] == '+' || a.p[0] == '-')) ? 1 : 0;
    if (a.n - sign == 0) {
        return 0;
    }
    for (int64_t k = sign; k < a.n; k++) {
        if (!any_digit(a.p[k])) {
            return 0;
        }
    }
    /* `any_int` reads `[-]digits`; a `+` is dropped first. */
    return a.p[0] == '+' ? any_int(a.p + 1, a.n - 1, out) : any_int(a.p, a.n, out);
}

static int64_t elya_time_live_now_ms(void) {
    static int64_t start = -1;
    int64_t now = elya_time_raw_ms();
    if (start < 0) {
        start = now;
    }
    return now - start;
}

int64_t elya_time_now_ms(void) {
    if (elya_trace_mode == TRACE_REPLAY) {
        TraceStr a = trace_answer("time.now_ms()");
        int64_t v;
        if (!trace_int(a, &v)) {
            trace_wrong_type(a, "Int");
        }
        return v;
    }
    int64_t v = elya_time_live_now_ms();
    if (elya_trace_mode == TRACE_RECORD) {
        fprintf(elya_trace_out, "time.now_ms() = %lld\n", (long long)v);
        trace_wrote();
    }
    return v;
}

static void elya_time_live_sleep_ms(int64_t ms);

void elya_time_sleep_ms(int64_t ms) {
    if (elya_trace_mode == TRACE_REPLAY) {
        char q[64];
        snprintf(q, sizeof q, "time.sleep_ms(%lld)", (long long)ms);
        TraceStr a = trace_answer(q);
        if (!trace_is(a, "Unit")) {
            trace_wrong_type(a, "Unit");
        }
        return;
    }
    elya_time_live_sleep_ms(ms);
    if (elya_trace_mode == TRACE_RECORD) {
        fprintf(elya_trace_out, "time.sleep_ms(%lld) = Unit\n", (long long)ms);
        trace_wrote();
    }
}

static void elya_time_live_sleep_ms(int64_t ms) {
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
 * return of a direct handle) as its continuation. A GC root since
 * 2026-10-22 (multi-shot sharing D3, grayed by gc_collect): a handle's body
 * ends at the end frame below, so the running chain does not reach the frame
 * that is current. Single-threaded, like the runtime. */
void *elya_current_handler = 0;

/* The end frame (spec 2026-10-22-elya-multi-shot-sharing D1): every handle's
 * body is a region whose continuation is THIS frame, not its own handler
 * frame. Its code (`elya_rt_handle_end`, generated: C cannot `musttail` a
 * `tailcc` function) continues in the CURRENT handler's code -- at the end of
 * a handle's body that handler is always the handle's own frame. So no site
 * frame points at a handler frame, and a multi-shot resume can share every
 * site frame of its chain.
 *
 * A static block shaped like a heap one (header, then `[tag][code]`): its mark
 * bit is set and it is never on gc_all_blocks, so marking it stops at once and
 * the sweep never frees it. Its tag is no constructor's, so nothing reads past
 * the code word. The code word is set at start-up by elya_handle_end_init,
 * which the generated `main` calls after elya_gc_init: the runtime refers to
 * no generated symbol, so a C harness can link it alone. */
static struct {
    Block header;
    int64_t words[2];
} elya_handle_end_block = {{(2 << 1) | GC_MARK_BIT, NULL}, {-1, 0}};

void *elya_handle_end = &elya_handle_end_block.words[0];

void elya_handle_end_init(void *code) {
    elya_handle_end_block.words[1] = (int64_t)(intptr_t)code;
}

/* 5b-8 D13: one-shot is enforced natively. A second resume stops here -- a
 * named message and a non-zero exit, never a re-run. The evaluator's E0425
 * words (spec 2026-10-22 D1). */
void elya_resume_twice(void) {
    fputs("elya: a one-shot continuation was resumed twice\n", stderr);
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

/* A shift amount outside 0..=63 (spec 2026-10-17-elya-unary-and-wrapping D2):
 * the evaluator's words. */
void elya_shift_range(const char *name, int64_t n) {
    fprintf(stderr, "elya: int.%s: a shift by %lld is outside 0..=63\n", name, (long long)n);
    exit(1);
}

/* `float.truncate` (round == 0) and `float.round` (round == 1) (spec
 * 2026-10-24-elya-float-part-2 D4): the evaluator's words on NaN and outside
 * `Int`'s range [-2^63, 2^63), both bounds exact doubles. */
int64_t elya_float_to_int(const char *name, int64_t bits, int64_t round_half_away) {
    double x;
    memcpy(&x, &bits, sizeof x);
    if (x != x) {
        fprintf(stderr, "elya: float.%s: NaN has no Int value\n", name);
        exit(1);
    }
    /* Every double at or past 2^52 is an integer, and the doubles next to
     * +-2^63 are 1024 or 2048 apart, so truncating or rounding x lands in
     * range exactly when x does. No libm: `trunc` and `round` would need `-lm`
     * on Linux. */
    if (!(x >= -9223372036854775808.0 && x < 9223372036854775808.0)) {
        char buf[1100];
        int64_t len = elya_float_text(bits, buf);
        fprintf(stderr, "elya: float.%s: %.*s is outside Int's range\n", name, (int)len, buf);
        exit(1);
    }
    int64_t t = (int64_t)x; /* toward zero */
    if (round_half_away) {
        double frac = x - (double)t; /* exact: |x| < 2^52 or frac == 0 */
        if (frac >= 0.5) {
            t += 1;
        } else if (frac <= -0.5) {
            t -= 1;
        }
    }
    return t;
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
                "elya-gc: collections=%lld freed=%lld live=%lld words_since_gc=%lld "
                "mid_copy=%lld\n",
                (long long)gc_collections, (long long)gc_freed, (long long)gc_live,
                (long long)gc_allocated, (long long)gc_copy_collections);
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

/* Native multi-shot (spec 2026-10-10, D1/D4; copy rule revised by spec
 * 2026-10-22-elya-multi-shot-sharing D2). A resume of a `with multi` handle
 * re-enters the captured chain as a fresh run. Site frames are never written
 * after allocation, so every run SHARES them: the chain `k` itself is the new
 * continuation's chain. Handler frames are written (every resume rewrites its
 * frame's `next` and `parent`, 5b-10), and the ones a run re-enters are exactly
 * the perform's path: from `innermost` (word 3) through `parent` (word 4) to
 * the answering frame `h` (word 2). Those are copied, each copy's `parent`
 * relinked to the next copy (the last one is re-installed by the resume, as
 * before). No site frame points at a handler frame (D1: a handle's body ends
 * at the end frame), so nothing else needs relinking. Copies are shallow
 * (2026-10-10 D4): saved values are shared, as the evaluator shares them.
 * Returns a fresh continuation object `[tag][k][h'][innermost']`. Cost: the
 * handlers on the path, not the frames on the chain. The caller has rooted
 * what it still needs; the continuation and every copy are rooted here while
 * the copy allocates. */
static void elya_chain_broken(void) {
    fputs("elya: resume: a continuation's handler chain does not reach its handler\n", stderr);
    exit(1);
}

#define CONT_PARENT 4

void *elya_cont_copy(void *cont, int64_t hlo, int64_t hhi) {
    int64_t *c = (int64_t *)cont;
    int64_t *h = (int64_t *)(intptr_t)c[2];
    int64_t *inner = (int64_t *)(intptr_t)c[3];
    int64_t n = 0;
    /* Defence in depth: the path holds handler frames only (recognised by tag,
     * `[hlo, hhi)`) and ends at `h`; anything else is a broken protocol, named. */
    for (int64_t *f = inner;; f = (int64_t *)(intptr_t)f[CONT_PARENT]) {
        if (!f || f[0] < hlo || f[0] >= hhi) {
            elya_chain_broken();
        }
        n++;
        if (f == h) {
            break;
        }
    }
    int64_t **copy = (int64_t **)malloc((size_t)n * sizeof(int64_t *));
    if (!copy) {
        fputs("elya: out of memory copying a continuation\n", stderr);
        exit(1);
    }
    elya_gc_push(cont);
    gc_in_cont_copy = 1;
    int64_t i = 0;
    for (int64_t *f = inner; i < n; f = (int64_t *)(intptr_t)f[CONT_PARENT]) {
        int64_t words = (int64_t)(gc_block_of(f)->meta >> 1);
        int64_t *p = (int64_t *)elya_alloc(words);
        memcpy(p, f, (size_t)words * 8);
        copy[i] = p;
        elya_gc_push(p);
        i++;
    }
    for (i = 0; i + 1 < n; i++) {
        copy[i][CONT_PARENT] = (int64_t)(intptr_t)copy[i + 1];
    }
    int64_t *fresh = (int64_t *)elya_alloc(4);
    fresh[0] = c[0];
    fresh[1] = c[1];
    fresh[2] = (int64_t)(intptr_t)copy[n - 1];
    fresh[3] = (int64_t)(intptr_t)copy[0];
    gc_in_cont_copy = 0;
    for (i = 0; i <= n; i++) {
        elya_gc_pop();
    }
    free(copy);
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
