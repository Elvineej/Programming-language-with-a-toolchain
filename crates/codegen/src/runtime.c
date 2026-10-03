#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

/* The Elya native runtime: a stop-the-world, non-moving mark-sweep collector.
 * Single-threaded throughout — Elya has no native concurrency yet, so none of
 * this state needs synchronisation.
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
    if (gc_allocated >= GC_THRESHOLD_WORDS) {
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

/* The deterministic failed-match trap. Spec 5b-4 §5: never `unreachable`. */
void elya_match_fail(void) {
    fputs("elya: match failed (no arm matched)\n", stderr);
    exit(1);
}

/* 5b-8 Task 8: the handler the running computation performs to. A handle
 * site sets it around its body and restores it after; a resume installs the
 * continuation's own handler around the resumed computation. A perform reads
 * it in O(1) -- walking the frame chain to its end on every perform made deep
 * non-tail effectful recursion quadratic (measured: 80k deep, 15 s). Not a GC
 * root: whenever it is read, the handler frame is reachable from the live
 * continuation, whose chain ends at it. Single-threaded, like the runtime. */
void *elya_current_handler = 0;

/* 5b-8 D13: one-shot is enforced natively. A second resume stops here -- a
 * named message and a non-zero exit, never a re-run. */
void elya_resume_twice(void) {
    fputs("elya: resume: a one-shot continuation was resumed twice\n", stderr);
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
