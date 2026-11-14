/* The Elya collector (spec 2026-11-12-elya-paged-collector): one collector from
 * bare metal to large programs. Stop-the-world, non-moving, precise mark-sweep
 * over PAGES of one size class each, swept lazily, marking with a fixed
 * worklist. It asks for memory from one page source and for nothing else: this
 * file includes only <stdint.h> and <stddef.h> and calls only the
 * `elya_platform_*` hooks below (and memset/memcpy, which a freestanding C
 * compiler requires anyway), so it builds `-ffreestanding -nostdlib`.
 *
 * runtime.c includes it, so a program is still one translation unit.
 *
 * Object layout (5b-5, unchanged where the program sees it): a pointer names
 * word 0 of `[tag][fields...]`. There is no header in front of it any more:
 * an object's size is its page's size class, its mark bit is in its page's
 * bitmap (D2). A pointer the heap does not hold (the static `elya_handle_end`
 * block) is never marked or freed (D3).
 *
 * Single-threaded throughout, as before: one thread runs Elya code. */

#include <stddef.h>
#include <stdint.h>

void *memset(void *, int, size_t);
void *memcpy(void *, const void *, size_t);

/* The platform layer (D1, D9). Hosted, runtime.c implements them over the
 * operating system; on a board, its start-up code does. */
/* `bytes` of memory aligned to `align` (a power of two), or NULL. */
void *elya_platform_pages(size_t bytes, size_t align);
/* Give back what `elya_platform_pages` gave (may do nothing). */
void elya_platform_release(void *p, size_t bytes);
/* Stop the program: out of memory, or a collector invariant broken. */
void elya_platform_fail(const char *message);

/* A pointer as the word a heap field holds, as LLVM's `ptrtoint` to i64 makes
 * it: zero-extended (spec 2026-11-14-elya-riscv-board D3). `(int64_t)(intptr_t)`
 * sign-extends on a 32-bit machine, and RISC-V's RAM starts at 0x80000000: the
 * same pointer would be two different words, one written here, one by the
 * program. */
#define ELYA_WORD(p) ((int64_t)(uintptr_t)(p))

#ifndef ELYA_PAGE_BYTES
#define ELYA_PAGE_BYTES 4096
#endif
/* A board's collector is small: a short worklist (an overflow is rescanned,
 * D5) and a few chunks (its region is one). */
#ifdef ELYA_FREESTANDING
#ifndef ELYA_MARK_STACK
#define ELYA_MARK_STACK 256
#endif
#ifndef ELYA_MAX_CHUNKS
#define ELYA_MAX_CHUNKS 4
#endif
#endif
#ifndef ELYA_MARK_STACK
#define ELYA_MARK_STACK 16384
#endif
#ifndef ELYA_MAX_CHUNKS
#define ELYA_MAX_CHUNKS 4096
#endif
/* The first chunk asked of the operating system; each next one doubles, up to
 * the cap, so a large heap is few chunks. */
#ifndef ELYA_CHUNK_BYTES
#define ELYA_CHUNK_BYTES ((size_t)1 << 20)
#endif
#define ELYA_CHUNK_MAX ((size_t)1 << 26)

/* ---- counters (read by elya_gc_report, tests read them by name, D8) ---- */

/* Words handed out since the last collection (a flow); collections and freed
 * objects only ever climb; live is a LEVEL: the slot words the last mark
 * reached. */
static int64_t gc_allocated = 0, gc_collections = 0, gc_freed = 0, gc_live = 0;
/* Collections that ran inside a multi-shot copy (spec
 * 2026-10-22-elya-multi-shot-sharing, the review's note 1). */
static int64_t gc_copy_collections = 0;
static int gc_in_cont_copy = 0;
/* ELYA_GC_STRESS=N (spec 2026-10-31-elya-scalability D13): a collection at
 * every Nth allocation; what it frees is poisoned and never handed out again,
 * and graying it stops the program. Set by runtime.c's elya_gc_init. */
static int64_t gc_stress = 0, gc_stress_count = 0;

/* Collect when the words allocated since the last collection reach the larger
 * of this floor and the last live level (D7, as before: a growing live set is
 * marked O(log n) times). Pinned by measurement in 5b-5: ordinary programs
 * never collect incidentally. */
enum { GC_THRESHOLD_WORDS = 1 << 16 };

/* [2*i] = arity, [2*i+1] = pointer mask: bit f set iff field f holds a heap
 * pointer, so marking never guesses (precise; 5b-1 §3.4). `arity` counts the
 * traced-candidate words after the tag, not the object's size (a string's row
 * is [0, 0]). */
static int64_t *gc_descriptors = NULL;
static int64_t gc_n_ctors = 0;

/* ---- size classes (D2) ---- */

/* Small classes (to 128 words) in one page, four slots at least. Medium
 * ones are made by `gc_make_classes` for the page size in use: each fills a
 * span of at most eight pages exactly (its slots times its size is the span,
 * less the header), an eighth or more above the last. At 4 KiB pages that is
 * 23 classes to 3828 words; an object's slot is 9% over its size on average
 * and 32% at worst (the first table, fixed sizes in spans of two slots, was
 * 29% and 99%: found by ChatGPT's review, measured by it, the rule made here).
 * Above the last class an object is a run of its own, rounded to a page. */
enum { GC_SMALL_CLASSES = 16, GC_N_CLASSES = 56 };
static uint32_t gc_class_words[GC_N_CLASSES] = {1,  2,  3,  4,  5,  6,  8,  10,
                                                12, 16, 24, 32, 48, 64, 96, 128};
static int gc_n_classes = GC_SMALL_CLASSES;

/* ---- pages ---- */

/* A page's kind is kept in its chunk's side table, never in the page: a large
 * object's later pages are the object's own memory (the review's B1). */
enum {
    PAGE_FREE = 0,
    PAGE_SMALL = 1, /* slots of one class */
    PAGE_LARGE = 2, /* the head of a run holding one large object */
    PAGE_TAIL = 3,  /* a later page of a run (or one being taken) */
};

#define GC_MAX_SLOTS (ELYA_PAGE_BYTES / 8)
#define GC_BITMAP_WORDS ((GC_MAX_SLOTS + 63) / 64)

typedef struct Page {
    struct Page *next;   /* its class's list, the large list, or the rescan list */
    uint32_t large;      /* the head of a large run */
    uint32_t cls;        /* small: its class index */
    uint32_t slot_words; /* small: words per slot; large: the object's words */
    uint32_t nslots;     /* small: slots; large: 1 */
    uint32_t npages;     /* large: pages in the run */
    uint32_t unswept;    /* marked by the last collection, not swept since */
    uint32_t rescan;     /* on the rescan list (D5) */
    void *free;          /* small: its free slots, linked through word 0 */
    struct Page *rescan_next;
    uint64_t marks[GC_BITMAP_WORDS];
    uint64_t dead[GC_BITMAP_WORDS]; /* stress mode: freed, kept, never reused */
} Page;

/* Slots start here, 8-aligned. */
#define GC_PAGE_HEADER ((sizeof(Page) + 7) & ~(size_t)7)

/* A page holds its header and four one-word slots at least; smaller, the
 * span arithmetic below would wrap (ChatGPT's review). */
typedef char gc_page_holds_its_header[ELYA_PAGE_BYTES >= GC_PAGE_HEADER + 32 ? 1 : -1];
typedef char gc_page_is_a_power_of_two[(ELYA_PAGE_BYTES & (ELYA_PAGE_BYTES - 1)) == 0 ? 1 : -1];

static inline int64_t *page_slots(Page *p) { return (int64_t *)((char *)p + GC_PAGE_HEADER); }

/* ---- chunks: what the page source gave (D1, D3) ---- */

typedef struct {
    char *base;     /* its first page, page-aligned */
    size_t npages;  /* pages it holds */
    uint8_t *kinds; /* each page's kind: the side table, at the chunk's start */
    uint8_t *back;  /* a span's or run's later page: pages back to its head (to 255) */
    size_t nfree;   /* its free pages */
    size_t cursor;  /* where the next search for free pages starts (next fit) */
} Chunk;

static Chunk gc_chunks[ELYA_MAX_CHUNKS]; /* sorted by base */
static size_t gc_n_chunks = 0;
static size_t gc_next_chunk = ELYA_CHUNK_BYTES;
static size_t gc_total_pages = 0;

/* The chunk holding `p`, or NULL. */
static Chunk *gc_chunk_of(const void *ptr) {
    const char *p = (const char *)ptr;
    size_t lo = 0, hi = gc_n_chunks;
    while (lo < hi) {
        size_t mid = (lo + hi) / 2;
        Chunk *c = &gc_chunks[mid];
        if (p < c->base) {
            hi = mid;
        } else if (p >= c->base + c->npages * ELYA_PAGE_BYTES) {
            lo = mid + 1;
        } else {
            return c;
        }
    }
    return NULL;
}

static inline size_t chunk_index(Chunk *c, const void *p) {
    return (size_t)((const char *)p - c->base) / ELYA_PAGE_BYTES;
}

static inline Page *chunk_page(Chunk *c, size_t i) { return (Page *)(c->base + i * ELYA_PAGE_BYTES); }

/* Does the heap hold `p` in an object? Then its page (D3). */
static Page *gc_page_of(void *ptr) {
    Chunk *c = gc_chunk_of(ptr);
    if (!c) {
        return NULL;
    }
    size_t i = chunk_index(c, ptr);
    if (c->kinds[i] == PAGE_TAIL) {
        i -= c->back[i];
    }
    uint8_t k = c->kinds[i];
    return k == PAGE_SMALL || k == PAGE_LARGE ? chunk_page(c, i) : NULL;
}

/* Take a region as a chunk: its side table first, then its whole pages. */
static int gc_add_chunk(void *mem, size_t bytes) {
    if (!mem || gc_n_chunks == ELYA_MAX_CHUNKS) {
        return 0;
    }
    uintptr_t lo = (uintptr_t)mem, hi = (uintptr_t)mem + bytes;
    /* n pages need 2n table bytes before them: n * (PAGE + 2) <= the room. */
    size_t n = bytes / (ELYA_PAGE_BYTES + 2);
    uintptr_t start = 0;
    while (n > 0) {
        start = (lo + 2 * n + ELYA_PAGE_BYTES - 1) & ~(uintptr_t)(ELYA_PAGE_BYTES - 1);
        if (start + n * ELYA_PAGE_BYTES <= hi) {
            break;
        }
        n--;
    }
    if (n == 0) {
        return 0;
    }
    size_t i = gc_n_chunks;
    while (i > 0 && gc_chunks[i - 1].base > (char *)start) {
        gc_chunks[i] = gc_chunks[i - 1];
        i--;
    }
    gc_chunks[i].base = (char *)start;
    gc_chunks[i].npages = n;
    gc_chunks[i].kinds = (uint8_t *)lo;
    gc_chunks[i].back = (uint8_t *)lo + n;
    gc_chunks[i].nfree = n;
    gc_chunks[i].cursor = 0;
    memset((void *)lo, 0, 2 * n);
    gc_n_chunks++;
    gc_total_pages += n;
    return 1;
}

/* A board gives its region (D1); hosted, chunks come from the platform. */
void elya_heap_init(void *base, size_t bytes) { gc_add_chunk(base, bytes); }

/* `n` free pages in a row, first fit from each chunk's cursor (next fit):
 * marked PAGE_TAIL, the caller names the head. */
static Page *gc_take_run(size_t n) {
    for (size_t k = 0; k < gc_n_chunks; k++) {
        Chunk *c = &gc_chunks[k];
        if (c->nfree < n || c->npages < n) {
            continue;
        }
        /* Two passes: from the cursor to the end, then from the start. */
        for (int pass = 0; pass < 2; pass++) {
            size_t from = pass == 0 ? c->cursor : 0;
            size_t to = pass == 0 ? c->npages : c->cursor + n - 1;
            if (to > c->npages) {
                to = c->npages;
            }
            size_t run = 0;
            for (size_t i = from; i < to; i++) {
                run = c->kinds[i] == PAGE_FREE ? run + 1 : 0;
                if (run == n) {
                    size_t first = i + 1 - n;
                    memset(c->kinds + first, PAGE_TAIL, n);
                    for (size_t j = 0; j < n; j++) {
                        c->back[first + j] = (uint8_t)(j < 255 ? j : 255);
                    }
                    c->nfree -= n;
                    c->cursor = i + 1 < c->npages ? i + 1 : 0;
                    return chunk_page(c, first);
                }
            }
        }
    }
    return NULL;
}


static void gc_set_kind(Page *pg, uint8_t kind) {
    Chunk *c = gc_chunk_of(pg);
    c->kinds[chunk_index(c, pg)] = kind;
}

/* Give back `n` pages from `pg` on. */
static void gc_release_pages(Page *pg, size_t n) {
    Chunk *c = gc_chunk_of(pg);
    size_t i = chunk_index(c, pg);
    memset(c->kinds + i, PAGE_FREE, n);
    c->nfree += n;
    /* Freed memory first: the next search starts no later than here (the
     * review's peak memory: next fit walked on into untouched pages). */
    if (i < c->cursor) {
        c->cursor = i;
    }
    pg->unswept = 0;
}


/* Hosted: ask the platform for another chunk, larger each time (D1). */
static int gc_grow(size_t at_least) {
    /* A chunk of `bytes` holds at least bytes / (PAGE + 2) - 1 pages (two
     * table bytes a page, and the first page aligned): enough of them for
     * what is asked (found by ChatGPT's review: `at_least` plus two pages was
     * not, and an object just under a chunk's size failed "out of memory"
     * with memory to be had). */
    size_t bytes = gc_next_chunk;
    while (bytes / (ELYA_PAGE_BYTES + 2) < at_least / ELYA_PAGE_BYTES + 2) {
        if (bytes > ((size_t)-1) / 4) {
            return 0;
        }
        bytes *= 2;
    }
    void *mem = elya_platform_pages(bytes, 8);
    if (!gc_add_chunk(mem, bytes)) {
        if (mem) {
            elya_platform_release(mem, bytes);
        }
        return 0;
    }
    if (gc_next_chunk < ELYA_CHUNK_MAX) {
        gc_next_chunk *= 2;
    }
    return 1;
}

/* ---- per-class allocation state (D4, D6) ---- */

typedef struct {
    Page *cur;     /* allocating from its free list */
    Page *unswept; /* its pages marked by the last collection, to sweep */
    Page *swept;   /* swept, or exhausted */
} Class;

static Class gc_classes[GC_N_CLASSES];
static Page *gc_large = NULL; /* every large run's head */
/* Pages in each class's span. */
static uint32_t gc_class_pages[GC_N_CLASSES];

/* Words -> class index for the small classes, filled by gc_setup. */
static int8_t gc_class_for[129];

/* Its class, or -1 for a run of its own: above the last class, or a span
 * larger than an eighth of the heap (a board's small region). */
static inline int gc_class_of(int64_t words) {
    int c;
    if (words <= 128) {
        c = gc_class_for[words];
    } else {
        c = -1;
        for (int i = GC_SMALL_CLASSES; i < gc_n_classes; i++) {
            if ((int64_t)gc_class_words[i] >= words) {
                c = i;
                break;
            }
        }
    }
    if (c >= 0 && gc_class_pages[c] > 1 && gc_total_pages > 0 &&
        (size_t)gc_class_pages[c] * 8 > gc_total_pages) {
        return -1;
    }
    return c;
}

static inline int bit_get(const uint64_t *m, uint32_t i) { return (int)((m[i >> 6] >> (i & 63)) & 1); }
static inline void bit_set(uint64_t *m, uint32_t i) { m[i >> 6] |= (uint64_t)1 << (i & 63); }

static inline uint32_t gc_slot_index(Page *pg, void *p) {
    return (uint32_t)(((char *)p - (char *)page_slots(pg)) / ((size_t)pg->slot_words * 8));
}

static inline void bit_clear(uint64_t *m, uint32_t i) { m[i >> 6] &= ~((uint64_t)1 << (i & 63)); }

static void gc_poison(int64_t *s, uint32_t words) {
    for (uint32_t w = 0; w < words; w++) {
        s[w] = (int64_t)0xDEADBEEFDEADBEEFULL;
    }
}

/* Sweep one page of a class (D6): what the last mark did not reach is freed
 * (or, under stress, poisoned and kept), the free list rebuilt, the marks
 * cleared. Returns the slots still in use. */
static uint32_t gc_sweep_page(Page *pg) {
    pg->unswept = 0;
    int64_t *slots = page_slots(pg);
    /* What was free when the page was marked: its free list, read here
     * rather than a bitmap kept up at every allocation. */
    uint64_t was_free[GC_BITMAP_WORDS] = {0};
    for (void *f = pg->free; f; f = *(void **)f) {
        bit_set(was_free, gc_slot_index(pg, f));
    }
    void *free = NULL;
    uint32_t live = 0;
    for (uint32_t i = pg->nslots; i-- > 0;) {
        int64_t *s = slots + (size_t)i * pg->slot_words;
        if (bit_get(pg->marks, i)) {
            live++;
            continue;
        }
        if (!bit_get(was_free, i) && !bit_get(pg->dead, i)) {
            gc_freed++;
            if (gc_stress > 0) {
                gc_poison(s, pg->slot_words);
                bit_set(pg->dead, i);
                continue;
            }
        }
        if (!bit_get(pg->dead, i)) {
            *(void **)s = free;
            free = s;
        }
    }
    for (uint32_t w = 0; w < GC_BITMAP_WORDS; w++) {
        pg->marks[w] = 0;
    }
    pg->free = free;
    return live;
}

static int gc_page_has_dead(Page *pg) {
    for (uint32_t w = 0; w < GC_BITMAP_WORDS; w++) {
        if (pg->dead[w]) {
            return 1;
        }
    }
    return 0;
}

/* Sweep a class's pages until one has a free slot: it becomes current. An
 * empty page (and no stress-kept slot in it) goes back to the free pages. */
static Page *gc_next_page_of(Class *c) {
    while (c->unswept) {
        Page *pg = c->unswept;
        c->unswept = pg->next;
        uint32_t live = gc_sweep_page(pg);
        if (live == 0 && !gc_page_has_dead(pg)) {
            gc_release_pages(pg, pg->npages);
            continue;
        }
        pg->next = c->swept;
        c->swept = pg;
        if (pg->free) {
            return pg;
        }
    }
    return NULL;
}

static Page *gc_new_small_page(int cls) {
    Page *pg = gc_take_run(gc_class_pages[cls]);
    if (!pg) {
        return NULL;
    }
    uint32_t words = gc_class_words[cls];
    gc_set_kind(pg, PAGE_SMALL);
    pg->large = 0;
    pg->rescan = 0;
    pg->cls = (uint32_t)cls;
    pg->slot_words = words;
    pg->npages = gc_class_pages[cls];
    pg->nslots =
        (uint32_t)(((size_t)pg->npages * ELYA_PAGE_BYTES - GC_PAGE_HEADER) / ((size_t)words * 8));
    if (pg->nslots > GC_MAX_SLOTS) {
        pg->nslots = GC_MAX_SLOTS;
    }
    pg->unswept = 0;
    for (uint32_t w = 0; w < GC_BITMAP_WORDS; w++) {
        pg->marks[w] = 0;
        pg->dead[w] = 0;
    }
    int64_t *slots = page_slots(pg);
    void *free = NULL;
    for (uint32_t i = pg->nslots; i-- > 0;) {
        int64_t *s = slots + (size_t)i * words;
        *(void **)s = free;
        free = s;
    }
    pg->free = free;
    pg->next = gc_classes[cls].swept;
    gc_classes[cls].swept = pg;
    return pg;
}

/* ---- roots ---- */

/* Working memory held from the heap (`elya_gc_scratch`): a list of blocks,
 * each `[-1][prev][next][bytes...]`, roots while held. The tag is no
 * constructor's, so the bytes are never scanned. */
static int64_t *gc_scratch_held = NULL;
/* Set while a working block is allocated: its caller's values may be on no
 * root, so nothing is collected under it. */
static int gc_no_collect = 0;
/* What an allocation that finds no memory names (the shadow stack's growth
 * names itself). */
static const char *gc_oom_what = "elya: out of memory\n";

void *elya_gc_scratch(void *p, size_t bytes);


/* The shadow stack (codegen pushes every live heap binding around allocations
 * and calls). Hosted it doubles in pages of its own; on a board it doubles in
 * the heap's region, a held working block (its entries are marked from here,
 * the block never scanned). */
#ifdef ELYA_FREESTANDING
/* A board's first roots are a fixed array (D5): the heap is asked only when it
 * is full (the review's R2: a page source, which a board need not give, was
 * asked, and 1024 roots were all a board had). */
#ifndef ELYA_SHADOW_ROOTS
#define ELYA_SHADOW_ROOTS 1024
#endif
static void *gc_shadow_fixed[ELYA_SHADOW_ROOTS];
static void **gc_shadow = gc_shadow_fixed;
static int64_t gc_shadow_top = 0, gc_shadow_cap = ELYA_SHADOW_ROOTS;
#else
static void **gc_shadow = NULL;
static int64_t gc_shadow_top = 0, gc_shadow_cap = 0;
#endif

void elya_gc_push(void *root) {
    if (gc_shadow_top == gc_shadow_cap) {
        int64_t cap = gc_shadow_cap ? gc_shadow_cap * 2 : 256;
#ifdef ELYA_FREESTANDING
        gc_oom_what = "elya: out of memory growing the shadow stack\n";
        int fixed = gc_shadow == gc_shadow_fixed;
        void **grown = (void **)elya_gc_scratch(fixed ? NULL : (void *)gc_shadow,
                                                (size_t)cap * sizeof(void *));
        gc_oom_what = "elya: out of memory\n";
        if (fixed) {
            memcpy(grown, gc_shadow_fixed, (size_t)gc_shadow_top * sizeof(void *));
        }
#else
        void **grown = (void **)elya_platform_pages((size_t)cap * sizeof(void *), 8);
        if (!grown) {
            elya_platform_fail("elya: out of memory growing the shadow stack\n");
        }
        if (gc_shadow) {
            memcpy(grown, gc_shadow, (size_t)gc_shadow_top * sizeof(void *));
            elya_platform_release(gc_shadow, (size_t)gc_shadow_cap * sizeof(void *));
        }
#endif
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

/* ---- marking (D5) ---- */

/* A fixed worklist: marking allocates nothing. Full, it leaves an object
 * marked but unscanned and sets the overflow flag; a rescan then pushes the
 * children of every marked object again until none is left out. */
static void *gc_gray[ELYA_MARK_STACK];
static int64_t gc_gray_top = 0;
static int gc_overflow = 0;
/* The pages holding an object the full worklist left gray. */
static Page *gc_rescan_pages = NULL;
/* Objects the full worklist left to a rescan, since start (for tests). */
static int64_t gc_overflows = 0;

/* Whether word `f` (after the tag) of a value tagged `tag` is a heap value.
 * A row is `[arity, mask]`; a value of more than 64 words has, in the mask's
 * place, where its mask words begin in the table (lib.rs `push_row`). */
static inline int gc_field_is_heap(int64_t tag, int64_t f) {
    const int64_t *row = gc_descriptors + 2 * tag;
    if (row[0] <= 64) {
        return (int)(((uint64_t)row[1] >> f) & 1);
    }
    return (int)(((uint64_t)gc_descriptors[row[1] + (f >> 6)] >> (f & 63)) & 1);
}

/* An object's words (a large object's own; a small one's slot), for
 * elya_cont_copy (D8). */
int64_t elya_gc_words(void *p) {
    Page *pg = gc_page_of(p);
    return pg ? (int64_t)pg->slot_words : 0;
}


static void gc_gray_push(void *p) {
    if (!p) {
        return;
    }
    Page *pg = gc_page_of(p);
    if (!pg) {
        return; /* not the heap's: a static block (D3) */
    }
    uint32_t i = pg->large ? 0 : gc_slot_index(pg, p);
    if (gc_stress > 0 && bit_get(pg->dead, i)) {
        /* Stress mode (spec 2026-10-31-elya-scalability D13): nothing
         * reachable is an object a collection freed. */
        elya_platform_fail("elya: gc stress: a reachable value names a freed block\n");
    }
    if (bit_get(pg->marks, i)) {
        return; /* shared structure and cycles terminate */
    }
    bit_set(pg->marks, i);
    gc_live += pg->slot_words;
    if (gc_gray_top == ELYA_MARK_STACK) {
        /* Left gray: its page is scanned again (D5), only its page. */
        gc_overflow = 1;
        gc_overflows++;
        if (!pg->rescan) {
            pg->rescan = 1;
            pg->rescan_next = gc_rescan_pages;
            gc_rescan_pages = pg;
        }
        return;
    }
    gc_gray[gc_gray_top++] = p;
}

static void gc_scan(int64_t *obj) {
    int64_t tag = obj[0];
    if (tag < 0 || tag >= gc_n_ctors) {
        return; /* not a constructor we have a descriptor for */
    }
    int64_t arity = gc_descriptors[2 * tag];
    /* Last field first: popped last, so a list's tail (its last field) is
     * taken after its head's leaf, and the worklist stays short. */
    for (int64_t f = arity; f-- > 0;) {
        if (gc_field_is_heap(tag, f)) {
            gc_gray_push((void *)(intptr_t)obj[1 + f]);
        }
    }
}

static void gc_drain(void) {
    while (gc_gray_top > 0) {
        gc_scan((int64_t *)gc_gray[--gc_gray_top]);
    }
}

/* After an overflow: scan the marked objects of each page that held one left
 * gray (scanning is idempotent: marked children are skipped), until no page
 * is left (D5). Only those pages, never the whole heap. */
static void gc_rescan(void) {
    while (gc_rescan_pages) {
        Page *pg = gc_rescan_pages;
        gc_rescan_pages = pg->rescan_next;
        pg->rescan = 0;
        if (pg->large) {
            gc_scan(page_slots(pg));
            gc_drain();
            continue;
        }
        for (uint32_t s = 0; s < pg->nslots; s++) {
            if (bit_get(pg->marks, s)) {
                gc_scan(page_slots(pg) + (size_t)s * pg->slot_words);
                gc_drain();
            }
        }
    }
    gc_overflow = 0;
}

static void gc_mark(void) {
    for (int64_t i = 0; i < gc_shadow_top; i++) {
        gc_gray_push(gc_shadow[i]);
    }
    for (int64_t *b = gc_scratch_held; b; b = (int64_t *)(intptr_t)b[2]) {
        gc_gray_push(b);
    }
    gc_drain();
    gc_rescan();
}

/* Defined with the handler protocol in runtime.c. */
extern void *elya_current_handler;

/* Finish the last collection's sweeping, so every mark bit is clear. */
static void gc_finish_sweeping(void) {
    for (int k = 0; k < gc_n_classes; k++) {
        Class *c = &gc_classes[k];
        while (c->unswept) {
            Page *pg = c->unswept;
            c->unswept = pg->next;
            uint32_t live = gc_sweep_page(pg);
            if (live == 0 && !gc_page_has_dead(pg) && pg != c->cur) {
                gc_release_pages(pg, pg->npages);
            } else {
                pg->next = c->swept;
                c->swept = pg;
            }
        }
    }
}

/* Large runs are swept at once: one object each. */
static void gc_sweep_large(void) {
    Page **link = &gc_large;
    while (*link) {
        Page *pg = *link;
        if (bit_get(pg->marks, 0)) {
            bit_clear(pg->marks, 0);
            link = &pg->next;
            continue;
        }
        if (gc_stress > 0 && !bit_get(pg->dead, 0)) {
            gc_freed++;
            bit_set(pg->dead, 0);
            gc_poison(page_slots(pg), pg->slot_words);
            link = &pg->next;
            continue;
        }
        if (gc_stress > 0) {
            link = &pg->next;
            continue;
        }
        gc_freed++;
        *link = pg->next;
        gc_release_pages(pg, pg->npages);
    }
}

static void gc_collect(void) {
    if (gc_no_collect) {
        return;
    }
    gc_collections++;
    gc_finish_sweeping();
    gc_live = 0;
    /* The current handler is a root (spec 2026-10-22-elya-multi-shot-sharing
     * D3), grayed before the shadow stack. */
    gc_gray_push(elya_current_handler);
    if (gc_in_cont_copy) {
        gc_copy_collections++;
    }
    gc_mark();
    gc_sweep_large();
    /* Every class's pages wait to be swept, lazily (D6); its current page's
     * free list is dropped: its unmarked slots are found again by its sweep. */
    for (int k = 0; k < gc_n_classes; k++) {
        Class *c = &gc_classes[k];
        Page *all = c->swept;
        c->swept = NULL;
        c->cur = NULL;
        while (all) {
            Page *next = all->next;
            all->unswept = 1;
            all->next = c->unswept;
            c->unswept = all;
            all = next;
        }
    }
    gc_allocated = 0;
}

/* The classes' spans, and the medium classes themselves (see above). */
static void gc_make_classes(void) {
    for (int i = 0; i < GC_SMALL_CLASSES; i++) {
        size_t m = 1;
        while ((m * ELYA_PAGE_BYTES - GC_PAGE_HEADER) / ((size_t)gc_class_words[i] * 8) < 4) {
            m++;
        }
        gc_class_pages[i] = (uint32_t)m;
    }
    gc_n_classes = GC_SMALL_CLASSES;
    size_t w = 128;
    while (gc_n_classes < GC_N_CLASSES) {
        size_t target = w + w / 8 + 1, best = 0, best_m = 0;
        if (target > 4096) {
            break;
        }
        /* The span, of eight pages at most (more only if fewer hold no two
         * slots), whose exact fill is nearest above the target. */
        for (size_t m = 1; m <= 64; m++) {
            size_t room = (m * ELYA_PAGE_BYTES - GC_PAGE_HEADER) / 8, k = room / target;
            if (k >= 2 && k <= GC_MAX_SLOTS && (best == 0 || room / k < best)) {
                best = room / k;
                best_m = m;
            }
            if (m >= 8 && best) {
                break;
            }
        }
        if (!best) {
            break;
        }
        gc_class_words[gc_n_classes] = (uint32_t)best;
        gc_class_pages[gc_n_classes] = (uint32_t)best_m;
        gc_n_classes++;
        w = best;
    }
}

void elya_gc_setup(const int64_t *descriptors, int64_t n_ctors, int64_t stress) {
    gc_descriptors = (int64_t *)descriptors;
    gc_n_ctors = n_ctors;
    gc_stress = stress;
    gc_make_classes();
    for (int64_t w = 0; w <= 128; w++) {
        int c = 0;
        while ((int64_t)gc_class_words[c] < w) {
            c++;
        }
        gc_class_for[w] = (int8_t)c;
    }
}

/* ---- allocation (D4) ---- */

static void *gc_alloc_large(int64_t words) {
    if ((uint64_t)words > ((size_t)-1 - GC_PAGE_HEADER) / 16 || (uint64_t)words > 0xFFFFFFFFu) {
        elya_platform_fail("elya: out of memory\n");
    }
    size_t bytes = GC_PAGE_HEADER + (size_t)words * 8;
    size_t n = (bytes + ELYA_PAGE_BYTES - 1) / ELYA_PAGE_BYTES;
    Page *pg = gc_take_run(n);
    if (!pg) {
        /* Empty pages may wait in other classes' unswept lists. */
        gc_finish_sweeping();
        pg = gc_take_run(n);
    }
    if (!pg && gc_grow(n * ELYA_PAGE_BYTES)) {
        pg = gc_take_run(n);
    }
    if (!pg) {
        gc_collect();
        gc_finish_sweeping();
        pg = gc_take_run(n);
    }
    if (!pg) {
        elya_platform_fail(gc_oom_what);
    }
    gc_set_kind(pg, PAGE_LARGE);
    pg->large = 1;
    pg->rescan = 0;
    pg->slot_words = (uint32_t)words;
    pg->nslots = 1;
    pg->npages = (uint32_t)n;
    pg->unswept = 0;
    for (uint32_t w = 0; w < GC_BITMAP_WORDS; w++) {
        pg->marks[w] = 0;
        pg->dead[w] = 0;
    }
    pg->next = gc_large;
    gc_large = pg;
    int64_t *p = page_slots(pg);
    memset(p, 0, (size_t)words * 8);
    return p;
}

/* `words` 8-byte words, zeroed: the contract the constructor code relies on. */
void *elya_alloc(int64_t words) {
    /* Roots for a collection triggered here are already on the shadow stack;
     * the new object is not yet a mark target. */
    if (gc_no_collect) {
        /* a working block: no collection, and none counted toward the next */
    } else if (gc_allocated >= (gc_live > GC_THRESHOLD_WORDS ? gc_live : GC_THRESHOLD_WORDS)) {
        gc_collect();
    } else if (gc_stress > 0 && ++gc_stress_count >= gc_stress) {
        gc_stress_count = 0;
        gc_collect();
    }
    if (words < 1) {
        words = 1;
    }
    if (!gc_no_collect) {
        gc_allocated += words;
    }
    int cls = gc_class_of(words);
    if (cls < 0) {
        return gc_alloc_large(words);
    }
    Class *c = &gc_classes[cls];
    if (!c->cur || !c->cur->free) {
        c->cur = gc_next_page_of(c);
        if (!c->cur) {
            c->cur = gc_new_small_page(cls);
        }
        if (!c->cur) {
            /* Empty pages may wait in other classes' unswept lists. */
            gc_finish_sweeping();
            c->cur = gc_new_small_page(cls);
        }
        if (!c->cur && gc_grow(ELYA_PAGE_BYTES)) {
            /* No page left: more memory where the platform has it (hosted;
             * the budget, D7, already decides when to collect)... */
            c->cur = gc_new_small_page(cls);
        }
        if (!c->cur) {
            /* ...else a collection may free some (a board's fixed region). */
            gc_collect();
            c->cur = gc_next_page_of(c);
            if (!c->cur) {
                gc_finish_sweeping();
                c->cur = gc_new_small_page(cls);
            }
            if (!c->cur) {
                elya_platform_fail(gc_oom_what);
            }
        }
    }
    Page *pg = c->cur;
    int64_t *s = (int64_t *)pg->free;
    pg->free = *(void **)s;
    for (int64_t i = 0; i < words; i++) {
        s[i] = 0;
    }
    return s;
}

/* Working memory outside the program's values, from the same heap (spec
 * 2026-11-13-elya-platform-layer D5): `realloc`'s contract ((NULL, n) a new
 * block, (p, n) p grown, (p, 0) p freed, NULL returned). A board has one
 * memory; this is how the runtime's own lists and texts live in it. A block is
 * kept while held, and free memory again as soon as it is freed. Allocating
 * one never collects (the caller's values may be on no root), so it needs
 * pages free now; none is the out-of-memory failure. */
void *elya_gc_scratch(void *p, size_t bytes) {
    int64_t *old = p ? (int64_t *)p - 3 : NULL;
    int64_t *fresh = NULL;
    if (old && bytes && ((size_t)elya_gc_words(old) - 3) * 8 >= bytes) {
        return p; /* its slot already holds `bytes` (a class rounds up) */
    }
    if (bytes) {
        if (bytes > ((size_t)-1) / 2) {
            elya_platform_fail("elya: out of memory\n");
        }
        gc_no_collect = 1;
        fresh = (int64_t *)elya_alloc(3 + (int64_t)((bytes + 7) / 8));
        gc_no_collect = 0;
        fresh[0] = -1;
        fresh[1] = 0;
        fresh[2] = ELYA_WORD(gc_scratch_held);
        if (gc_scratch_held) {
            gc_scratch_held[1] = ELYA_WORD(fresh);
        }
        gc_scratch_held = fresh;
        if (old) {
            size_t had = ((size_t)elya_gc_words(old) - 3) * 8;
            memcpy(fresh + 3, old + 3, had < bytes ? had : bytes);
        }
    }
    if (old) {
        int64_t *prev = (int64_t *)(intptr_t)old[1], *next = (int64_t *)(intptr_t)old[2];
        if (prev) {
            prev[2] = ELYA_WORD(next);
        } else {
            gc_scratch_held = next;
        }
        if (next) {
            next[1] = ELYA_WORD(prev);
        }
        /* A freed block is free memory at once: the next working block may
         * be allocated before any collection (none runs under one), and a
         * board's region holds little garbage of its own making (the review's
         * B1: 250 `==` on two-field values filled 256 KiB). A run's pages go
         * back; a slot goes on its page's free list, its mark (it was a root)
         * cleared, and its page allocates next if the class's current one is
         * full and the page was swept since the last collection (an unswept
         * page is never allocated from: its sweep would free what was). Under
         * stress mode the collector frees it, poisoned. */
        Page *pg = gc_page_of(old);
        if (pg && pg->large && gc_stress == 0) {
            for (Page **link = &gc_large; *link; link = &(*link)->next) {
                if (*link == pg) {
                    *link = pg->next;
                    gc_release_pages(pg, pg->npages);
                    break;
                }
            }
        } else if (pg && gc_stress == 0) {
            bit_clear(pg->marks, gc_slot_index(pg, old));
            *(void **)old = pg->free;
            pg->free = old;
            Class *c = &gc_classes[pg->cls];
            if ((!c->cur || !c->cur->free) && !pg->unswept) {
                c->cur = pg;
            }
        }
    }
    return fresh ? (void *)(fresh + 3) : NULL;
}
