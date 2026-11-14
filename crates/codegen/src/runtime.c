#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

/* The Elya native runtime. The collector is gc.c (spec
 * 2026-11-12-elya-paged-collector), included here so a program is one
 * translation unit. Single-threaded throughout: one thread runs Elya code
 * (`elya_start`), and the main thread reads the collector's state only after
 * joining it.
 *
 * This file is freestanding (spec 2026-11-13-elya-platform-layer): it includes
 * <stdint.h>, <stddef.h> and <stdarg.h> and reaches the machine only through
 * the `elya_platform_*` hooks below (and memcpy/memset/memcmp, which a
 * freestanding C compiler may call by itself). Hosted (the default), the hooks
 * are platform_hosted.c, included at the end; with ELYA_FREESTANDING a board
 * gives `write` and `exit` and may give the rest (weak defaults at the end). */

int memcmp(const void *, const void *, size_t);

/* ---- the platform layer (D1) ---- */

/* Bytes out: stream 1 is the program's output, 2 its failures. */
void elya_platform_write(int stream, const char *bytes, size_t n);
/* What stream 1 holds is out before a failure's line. */
void elya_platform_flush(void);
/* Stop the program with this status; does not return. */
void elya_platform_exit(int code) __attribute__((noreturn));
/* Once, before anything is written. */
void elya_platform_init(void);
/* A setting by name, or NULL (a board has none). */
const char *elya_platform_getenv(const char *name);
/* Working memory outside the heap, `realloc`'s contract: (NULL, n) a new
 * block, (p, n) p grown, (p, 0) p freed; NULL when there is none. */
void *elya_platform_scratch(void *p, size_t bytes);
/* A clock that never goes back, in milliseconds; a wait of at least `ms`. */
int64_t elya_platform_now_ms(void);
void elya_platform_sleep_ms(int64_t ms);
/* The trace file (record and replay): each returns 0 (NULL, -1) on a failure
 * `elya_platform_error` then names. */
int elya_platform_trace_create(const char *path);
int elya_platform_trace_append(const char *bytes, size_t n);
void *elya_platform_file_open(const char *path);
int64_t elya_platform_file_read(void *file, char *into, int64_t n);
void elya_platform_file_close(void *file);
const char *elya_platform_error(void);
/* A program's files, input and arguments (spec 2026-11-14-elya-files-and-arguments
 * D5): its paths are UTF-8. `path_open` is read with `file_read`/`file_close`;
 * `path_write` writes the whole file, 1 on success; `path_error` is D2's reason
 * for the failure just met on `path`; `stdin_read` answers 0 at the end, -1 on
 * a failure; `args` gives the arguments as UTF-8 (and their count). */
void *elya_platform_path_open(const char *path);
int elya_platform_path_exists(const char *path);
int elya_platform_path_write(const char *path, const char *bytes, int64_t n);
const char *elya_platform_path_error(const char *path);
int64_t elya_platform_stdin_read(char *into, int64_t n);
char **elya_platform_args(int argc, char **argv, int *count);

#include "gc.c"

/* A failure the collector names: the line, status 1. */
void elya_platform_fail(const char *message) {
    size_t n = 0;
    while (message[n]) {
        n++;
    }
    elya_platform_flush();
    elya_platform_write(2, message, n);
    elya_platform_exit(1);
}

/* ---- text without a C library: `%s`, `%.*s`, `%d`, `%lld`, `%llu`, `%%` ---- */

static int64_t rt_strlen(const char *s) {
    int64_t n = 0;
    while (s[n]) {
        n++;
    }
    return n;
}

/* `[-]digits` as atoll reads it (no overflow check: settings and exponents). */
static int64_t rt_atoll(const char *s) {
    int neg = 0;
    while (*s == ' ' || (*s >= '\t' && *s <= '\r')) {
        s++; /* as atoll (the review's N3: `ELYA_GC_STRESS=' 1'`) */
    }
    if (*s == '-' || *s == '+') {
        neg = *s == '-';
        s++;
    }
    uint64_t v = 0;
    while (*s >= '0' && *s <= '9') {
        v = v * 10 + (uint64_t)(*s - '0');
        s++;
    }
    return (int64_t)(neg ? 0 - v : v);
}

/* A sink: a buffer of `cap` bytes (stream 0; `n` counts what was asked, as
 * snprintf's result does) or a stream, written in pieces. */
typedef struct {
    int stream;
    char *buf;
    size_t cap, n;
} RtSink;

static void rt_emit(RtSink *k, const char *p, size_t n) {
    if (k->stream) {
        elya_platform_write(k->stream, p, n);
    } else {
        for (size_t i = 0; i < n; i++) {
            if (k->n + i + 1 < k->cap) {
                k->buf[k->n + i] = p[i];
            }
        }
    }
    k->n += n;
}

static void rt_emit_u64(RtSink *k, uint64_t v, int neg) {
    char d[24];
    int at = (int)sizeof d;
    do {
        d[--at] = (char)('0' + v % 10);
        v /= 10;
    } while (v);
    if (neg) {
        d[--at] = '-';
    }
    rt_emit(k, d + at, sizeof d - (size_t)at);
}

static void rt_vformat(RtSink *k, const char *fmt, va_list ap) {
    const char *run = fmt;
    for (const char *c = fmt;; c++) {
        if (*c != '%' && *c != 0) {
            continue;
        }
        rt_emit(k, run, (size_t)(c - run));
        if (*c == 0) {
            break;
        }
        c++;
        if (c[0] == 's') {
            const char *s = va_arg(ap, const char *);
            rt_emit(k, s, (size_t)rt_strlen(s));
        } else if (c[0] == '.' && c[1] == '*' && c[2] == 's') {
            int n = va_arg(ap, int);
            const char *s = va_arg(ap, const char *);
            rt_emit(k, s, (size_t)n);
            c += 2;
        } else if (c[0] == 'd') {
            int v = va_arg(ap, int);
            rt_emit_u64(k, v < 0 ? 0 - (uint64_t)(int64_t)v : (uint64_t)v, v < 0);
        } else if (c[0] == 'l' && c[1] == 'l' && c[2] == 'd') {
            long long v = va_arg(ap, long long);
            rt_emit_u64(k, v < 0 ? 0 - (uint64_t)v : (uint64_t)v, v < 0);
            c += 2;
        } else if (c[0] == 'l' && c[1] == 'l' && c[2] == 'u') {
            rt_emit_u64(k, (uint64_t)va_arg(ap, unsigned long long), 0);
            c += 2;
        } else {
            rt_emit(k, c, 1); /* `%%` */
        }
        run = c + 1;
    }
}

static int rt_vsnprintf(char *buf, size_t cap, const char *fmt, va_list ap) {
    RtSink k = {0, buf, cap, 0};
    rt_vformat(&k, fmt, ap);
    if (cap) {
        buf[k.n < cap ? k.n : cap - 1] = 0;
    }
    return (int)k.n;
}

static int rt_snprintf(char *buf, size_t cap, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = rt_vsnprintf(buf, cap, fmt, ap);
    va_end(ap);
    return n;
}

/* A line to a stream in one write when it fits 512 bytes (a failure's line
 * is not interleaved with another process's on a shared stderr: the review's
 * N4), else in pieces. */
static void rt_printf(int stream, const char *fmt, ...) {
    char line[512];
    va_list ap, again;
    va_start(ap, fmt);
    va_copy(again, ap);
    int n = rt_vsnprintf(line, sizeof line, fmt, ap);
    if ((size_t)n < sizeof line) {
        elya_platform_write(stream, line, (size_t)n);
    } else {
        RtSink k = {stream, NULL, 0, 0};
        rt_vformat(&k, fmt, again);
    }
    va_end(again);
    va_end(ap);
}

/* The first `c` in [p, p+n), or NULL. */
static const char *rt_memchr(const char *p, int c, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (p[i] == (char)c) {
            return p + i;
        }
    }
    return NULL;
}

static void rt_puts(int stream, const char *s) { elya_platform_write(stream, s, (size_t)rt_strlen(s)); }


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
    {
        const char *stress = elya_platform_getenv("ELYA_GC_STRESS");
        elya_gc_setup(descriptors, n_ctors, stress ? rt_atoll(stress) : 0);
    }
    elya_platform_init();
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

/* A linear text buffer (spec 2026-11-14-elya-buffer D3): one object,
 * `[tag][length][capacity][bytes]`, holding no pointers. It carries the string
 * tag, whose descriptor row traces nothing, so the collector needs no row of
 * its own; nothing reads a buffer as a string (`Buffer` is its own type, and
 * neither `any` nor `==` takes it). The affine guarantee means the program
 * holds the only reference: `push` writes in place when the text fits and
 * answers the same object, else a new one of twice the capacity (the old one
 * is garbage). Amortised O(|s|) a push, O(length) the finish. */
#define ELYA_BUFFER_FIRST 64

static int64_t *buffer_alloc(int64_t tag, int64_t cap) {
    int64_t *p = (int64_t *)elya_alloc(3 + (cap + 7) / 8);
    p[0] = tag;
    p[1] = 0;
    p[2] = cap;
    return p;
}

void *elya_buffer_new(int64_t tag) { return buffer_alloc(tag, ELYA_BUFFER_FIRST); }

void *elya_buffer_push(int64_t tag, void *buffer, void *text) {
    int64_t *b = (int64_t *)buffer;
    const int64_t *t = (const int64_t *)text;
    int64_t len = b[1], n = t[1];
    if (n <= b[2] - len) {
        memcpy((char *)&b[3] + len, &t[2], (size_t)n);
        b[1] = len + n;
        return b;
    }
    int64_t cap = b[2] > INT64_MAX / 2 ? INT64_MAX : 2 * b[2];
    if (cap - len < n) {
        cap = len + n;
    }
    elya_gc_push(buffer);
    elya_gc_push(text);
    int64_t *grown = buffer_alloc(tag, cap);
    elya_gc_pop();
    elya_gc_pop();
    memcpy(&grown[3], &b[3], (size_t)len);
    memcpy((char *)&grown[3] + len, &t[2], (size_t)n);
    grown[1] = len + n;
    return grown;
}

void *elya_buffer_finish(int64_t string_tag, void *buffer) {
    int64_t *b = (int64_t *)buffer;
    elya_gc_push(buffer);
    void *s = elya_str_lit(string_tag, (const char *)&b[3], b[1]);
    elya_gc_pop();
    return s;
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
        rt_printf(2, "elya: string.byte_at: index %lld is outside a string of %lld bytes\n",
                (long long)i, (long long)len);
        elya_platform_exit(1);
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
        rt_printf(2, "elya: string.slice: %lld..%lld is not a range in a string of %lld bytes\n",
                (long long)from, (long long)to, (long long)len);
        elya_platform_exit(1);
    }
    const unsigned char *b = (const unsigned char *)&((int64_t *)s)[2];
    if (!elya_str_boundary(b, len, from) || !elya_str_boundary(b, len, to)) {
        rt_printf(2, "elya: string.slice: %lld..%lld splits a UTF-8 character\n",
                (long long)from, (long long)to);
        elya_platform_exit(1);
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
    /* The first 32 triples on the host stack: most comparisons ask for no
     * working memory at all (the review's B1, and a `realloc` per `==` hosted). */
    int64_t first[96];
    int64_t *work = first;
    int64_t top = 0, cap = 96;
    int64_t equal = 1;
#define EQ_PUSH(x, y, heap)                                                            \
    do {                                                                               \
        if (top + 3 > cap) {                                                           \
            cap = cap * 2;                                                             \
            int64_t *grown = (int64_t *)elya_platform_scratch(                         \
                work == first ? NULL : work, (size_t)cap * sizeof *work);              \
            if (!grown) {                                                              \
                rt_puts(2, "elya: out of memory comparing values\n");                 \
                elya_platform_exit(1);                                                 \
            }                                                                          \
            if (work == first) {                                                       \
                memcpy(grown, first, sizeof first);                                    \
            }                                                                          \
            work = grown;                                                              \
        }                                                                              \
        work[top++] = (int64_t)(x);                                                    \
        work[top++] = (int64_t)(y);                                                    \
        work[top++] = (heap);                                                          \
    } while (0)
    EQ_PUSH(ELYA_WORD(a), ELYA_WORD(b), 1);
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
            rt_puts(2, "elya: `==` reached a value that is not data (E0436 should have rejected it)\n");
            elya_platform_exit(1);
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
            rt_puts(2, "elya: ==: a value without a descriptor\n");
            elya_platform_exit(1);
        }
        int64_t arity = gc_descriptors[2 * tag];
        /* A constructor's float mask, or (more than 64 fields) where its mask
         * words begin, as its descriptor row. */
        const int64_t *fmask = tag < eq_n_masks ? eq_float_masks + tag : NULL;
        if (fmask && arity > 64) {
            fmask = eq_float_masks + *fmask;
        }
        for (int64_t f = 0; f < arity; f++) {
            EQ_PUSH(x[1 + f], y[1 + f],
                    gc_field_is_heap(tag, f)
                        ? 1
                        : ((fmask && (((uint64_t)fmask[f >> 6] >> (f & 63)) & 1)) ? 2 : 0));
        }
    }
#undef EQ_PUSH
    if (work != first) {
        elya_platform_scratch(work, 0);
    }
    return equal;
}

/* `int.to_string` (D1): decimal, a leading '-' for a negative, INT64_MIN
 * included (20 characters at most). */
void *elya_int_to_string(int64_t tag, int64_t n) {
    char buf[32];
    int len = rt_snprintf(buf, sizeof buf, "%lld", (long long)n);
    return elya_str_lit(tag, buf, (int64_t)len);
}

/* ---- Floats (spec 2026-10-23-elya-float-natively D3): the literal text. -----
 *
 * The shortest digit string that reads back to the same double, the closest
 * such string when several are equally short, written as a plain decimal (no
 * exponent), `.0` when integral; `inf`, `-inf`, `nan` (a NaN's sign is dropped).
 * That is the evaluator's text, Rust's `Display` plus `.0`. Without a C library
 * (spec 2026-11-13-elya-platform-layer D6): the shortest digits that read back
 * (float_fmt.c, Ryu's method) and the nearest double of a text (float_parse.c,
 * Eisel-Lemire, then an exact comparison). */
#include "float_fmt.c"
#include "float_parse.c"

/* Writes `bits`' literal text to `out` (room for 1100 bytes); returns its
 * length: `nan`, `inf`, `-inf`, else the shortest decimal that reads back, in
 * plain notation with a point (`1.0`, `-0.0`, `0.000001`, `1e21` in full), as
 * the evaluator writes it. */
int64_t elya_float_text(int64_t bits, char *out) {
    double x;
    memcpy(&x, &bits, sizeof x);
    if (__builtin_isnan(x)) {
        memcpy(out, "nan", 3);
        return 3;
    }
    int64_t n = (int64_t)elya_fmt_f64(x, out);
    if (__builtin_isinf(x)) {
        return n;
    }
    for (int64_t i = 0; i < n; i++) {
        if (out[i] == '.') {
            return n;
        }
    }
    out[n] = '.';
    out[n + 1] = '0';
    return n + 2;
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

enum { ANY_INT, ANY_BOOL, ANY_UNIT, ANY_STRING, ANY_FN, ANY_ADT, ANY_UNBUILT, ANY_FLOAT, ANY_BUFFER };

static void any_oom(void) {
    rt_puts(2, "elya: out of memory\n");
    elya_platform_exit(1);
}

static void *any_grow(void *p, int64_t *cap, int64_t need, size_t elem) {
    if (need <= *cap) {
        return p;
    }
    int64_t c = *cap ? *cap : 16;
    while (c < need) {
        c *= 2;
    }
    void *q = elya_platform_scratch(p, (size_t)c * elem);
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

static void any_puts(AnyBuf *b, const char *s) { any_put(b, s, (int64_t)rt_strlen(s)); }

/* `n` bytes as a string literal, `literal.rs`'s `show`: five escapes, every
 * other byte as it is. A run of plain bytes is copied at once. */
static void any_put_lit(AnyBuf *b, const char *bytes, int64_t n) {
    any_puts(b, "\"");
    int64_t from = 0;
    for (int64_t i = 0; i < n; i++) {
        const char *esc = NULL;
        switch (bytes[i]) {
        case '"': esc = "\\\""; break;
        case '\\': esc = "\\\\"; break;
        case '\n': esc = "\\n"; break;
        case '\t': esc = "\\t"; break;
        case '\r': esc = "\\r"; break;
        default: break;
        }
        if (esc) {
            any_put(b, bytes + from, i - from);
            any_puts(b, esc);
            from = i + 1;
        }
    }
    any_put(b, bytes + from, n - from);
    any_puts(b, "\"");
}

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
    rt_printf(2, "elya: internal: %s\n", what);
    elya_platform_exit(1);
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
            any_put(&out, num, rt_snprintf(num, sizeof num, "%lld", (long long)it.value));
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
            any_put_lit(&out, (const char *)&s[2], s[1]);
            break;
        }
        case ANY_FN:
            /* The program's output first, as from the evaluator (the
             * main-values review's L1). */
            elya_platform_flush();
            rt_printf(2, "elya: %sa function%s\n", before, after);
            elya_platform_exit(1);
        case ANY_BUFFER:
            elya_platform_flush();
            rt_printf(2, "elya: %sa buffer%s\n", before, after);
            elya_platform_exit(1);
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
    elya_platform_scratch(stack, 0);
    void *s = elya_str_lit(string_tag, out.p ? out.p : "", out.len);
    elya_platform_scratch(out.p, 0);
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
        case ANY_BUFFER:
            rt_puts(2, "elya: `==` reached a value that is not data (E0436 should have rejected it)\n");
            elya_platform_exit(1);
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
            if (__builtin_isnan(v)) {
                elya_platform_scratch(stack, 0);
                return 0;
            }
            break;
        }
        default:
            break;
        }
    }
    elya_platform_scratch(stack, 0);
    return 1;
}

/* `op.decode`'s refusal of an answer at an effect's own parameter (D4). */
void elya_any_fail(const char *msg) {
    rt_printf(2, "elya: %s\n", msg);
    elya_platform_exit(1);
}

/* A parsed literal: the evaluator's `Value`s a literal can denote. */
enum { LIT_INT, LIT_FLOAT, LIT_BOOL, LIT_UNIT, LIT_STR, LIT_CTOR };

typedef struct {
    int kind;
    int64_t i;          /* Int, Bool; a Float's bits */
    char *s;            /* String bytes (working memory); a constructor's name (into the text) */
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
    int64_t k = (int64_t)rt_strlen(w);
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
                    elya_platform_scratch(bytes, 0);
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
                        elya_platform_scratch(bytes, 0);
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
                        elya_platform_scratch(bytes, 0);
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
                double ninf = -__builtin_inf();
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
                    /* Correctly rounded, as Rust's `from_str` (D4), which also
                     * takes `5.` and `-.5`: the parser wants a digit on both
                     * sides of the point. */
                    const char *at = p->s + start;
                    int64_t len = p->i - start;
                    int neg = at[0] == '-';
                    double fv = 0.0;
                    if (at[len - 1] == '.') {
                        len--; /* `5.` is `5` */
                    }
                    if (at[neg] == '.') {
                        at += neg; /* `-.5`: `.5` read as `0.5`, then the sign */
                        len -= neg;
                        char *zero = (char *)elya_platform_scratch(NULL, (size_t)len + 1);
                        if (!zero) {
                            any_oom();
                        }
                        zero[0] = '0';
                        memcpy(zero + 1, at, (size_t)len);
                        elya_parse_f64(zero, (size_t)len + 1, &fv);
                        elya_platform_scratch(zero, 0);
                        if (neg) {
                            fv = -fv;
                        }
                    } else {
                        elya_parse_f64(at, (size_t)len, &fv);
                    }
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
            double pinf = __builtin_inf();
            int64_t pb;
            memcpy(&pb, &pinf, sizeof pb);
            v = any_new(p, (AnyLit){LIT_FLOAT, pb, NULL, 0, NULL, 0});
        } else if (c == 'n' && any_starts(p, "nan")) {
            p->i += 3;
            double qnan = __builtin_nan("");
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
                elya_platform_scratch(open, 0);
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
        elya_platform_scratch(open[k].args, 0);
    }
    elya_platform_scratch(open, 0);
    return -1;
}

static void any_free(AnyParser *p) {
    for (int64_t k = 0; k < p->nlits; k++) {
        if (p->lits[k].kind == LIT_STR) {
            elya_platform_scratch(p->lits[k].s, 0);
        }
        elya_platform_scratch(p->lits[k].kids, 0);
    }
    elya_platform_scratch(p->lits, 0);
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
        case ANY_FN:
        case ANY_BUFFER: ok = 0; break;
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
    elya_platform_scratch(work, 0);
    return ok;
}

static void any_decode_fail(const char *why1, const char *text, int64_t tlen, const char *why2,
                            const char *opname) {
    rt_printf(2, "elya: op.decode: %s", why1);
    elya_platform_write(2, text, (size_t)tlen);
    rt_printf(2, "%s, for an answer of `%s`\n", why2, opname);
    elya_platform_exit(1);
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
        rt_printf(2, "elya: op.decode: `");
        elya_platform_write(2, src, (size_t)tlen);
        rt_printf(2, "` is not a literal (%s), for an answer of `%s`\n", err, opname);
        elya_platform_exit(1);
    }
    any_ws(&p);
    if (p.i != p.n) {
        any_decode_fail("`", src, tlen, "` has input after its value", opname);
    }
    size_t centry_bytes = (size_t)(p.nlits ? p.nlits : 1) * sizeof(int64_t);
    int64_t *centry = (int64_t *)elya_platform_scratch(NULL, centry_bytes);
    if (!centry) {
        any_oom();
    }
    memset(centry, 0, centry_bytes);
    if (!any_conforms(&p, t, names, root, node, &centry)) {
        rt_printf(2, "elya: op.decode: `");
        elya_platform_write(2, src, (size_t)tlen);
        rt_printf(2, "` is not a value of type `%s`, for an answer of `%s`\n", display, opname);
        elya_platform_exit(1);
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
            d.word = ELYA_WORD(elya_str_lit(string_tag, l->s ? l->s : "", l->slen));
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
            d.word = ELYA_WORD(obj);
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
    elya_platform_scratch(steps, 0);
    elya_platform_scratch(done, 0);
    elya_platform_scratch(centry, 0);
    any_free(&p);
    return result;
}

/* Time, the first I/O source (spec 2026-10-15-elya-async-step2-timers, D3).
 * `time.now_ms`: monotonic milliseconds since the first reading. `time.sleep_ms`:
 * a wait of at least `ms` (none when `ms <= 0`). Neither allocates. */

/* ---- Record and replay natively (spec 2026-10-17-elya-native-record-replay).
 * `ELYA_RECORD=<file>` writes the host's answers as the evaluator's trace,
 * entry by entry (D4); `ELYA_REPLAY=<file>` answers from one, without waiting
 * (D5). The format and every failure's words are the evaluator's
 * (`src/lib.rs`, `src/eval.rs` `host_builtin`). */
#define ELYA_TRACE_HEADER "elya trace 1"

enum { TRACE_LIVE, TRACE_RECORD, TRACE_REPLAY };
static int elya_trace_mode = TRACE_LIVE;
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
    int64_t n = (int64_t)rt_strlen(lit);
    return s.n == n && memcmp(s.p, lit, (size_t)n) == 0;
}

static void trace_put(TraceStr s) { elya_platform_write(2, s.p, (size_t)s.n); }

static void trace_bad(const char *msg, int64_t line) {
    if (line > 0) {
        rt_printf(2, "elya: trace line %lld: %s\n", (long long)line, msg);
    } else {
        rt_printf(2, "elya: %s\n", msg);
    }
    elya_platform_exit(1);
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
        const char *nl = rt_memchr(line, '\n', (size_t)(end - line));
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
static void trace_wrote(int ok) {
    if (!ok) {
        rt_printf(2, "elya: cannot write the trace `%s`: %s\n", elya_trace_path, elya_platform_error());
        elya_platform_exit(2);
    }
}

/* One line of the trace, written through (an entry is on disk before the
 * program goes on). */
static void trace_line(const char *fmt, ...) {
    char line[160];
    va_list ap;
    va_start(ap, fmt);
    int n = rt_vsnprintf(line, sizeof line, fmt, ap);
    va_end(ap);
    if ((size_t)n >= sizeof line) {
        n = (int)sizeof line - 1; /* no line is near it (44 bytes at most) */
    }
    trace_wrote(elya_platform_trace_append(line, (size_t)n));
}

/* Before the program runs: the environment, the trace file (D1, D3). */
static void elya_trace_init(void) {
    const char *rec = elya_platform_getenv("ELYA_RECORD");
    const char *rep = elya_platform_getenv("ELYA_REPLAY");
    if (rec && rep) {
        rt_puts(2, "elya: ELYA_RECORD and ELYA_REPLAY cannot both be set\n");
        elya_platform_exit(2);
    }
    if (rec) {
        elya_trace_path = rec;
        if (!elya_platform_trace_create(rec)) {
            rt_printf(2, "elya: cannot write the trace `%s`: %s\n", rec, elya_platform_error());
            elya_platform_exit(2);
        }
        trace_line("%s\n", ELYA_TRACE_HEADER);
        elya_trace_mode = TRACE_RECORD;
    } else if (rep) {
        void *f = elya_platform_file_open(rep);
        if (!f) {
            rt_printf(2, "elya: cannot read the trace `%s`: %s\n", rep, elya_platform_error());
            elya_platform_exit(2);
        }
        char *buf = NULL;
        int64_t len = 0, cap = 0;
        for (;;) {
            buf = (char *)any_grow(buf, &cap, len + 4096, 1);
            int64_t got = elya_platform_file_read(f, buf + len, 4096);
            if (got < 0) {
                rt_printf(2, "elya: cannot read the trace `%s`: %s\n", rep, elya_platform_error());
                elya_platform_exit(2);
            }
            len += got;
            if (got < 4096) {
                break;
            }
        }
        elya_platform_file_close(f);
        if (!trace_utf8((const unsigned char *)buf, len)) {
            rt_printf(2, "elya: cannot read the trace `%s`: stream did not contain valid UTF-8\n",
                    rep);
            elya_platform_exit(2);
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
        elya_platform_flush();
        rt_printf(2, "elya: replay: `main` returned with %lld unread trace %s, from entry %lld (`",
                (long long)left, left == 1 ? "entry" : "entries", (long long)elya_trace_next + 1);
        trace_put(e->q);
        rt_puts(2, " = ");
        trace_put(e->a);
        rt_puts(2, "`): the run asked less than the recording\n");
        elya_platform_exit(1);
    }
}

/* The next replayed answer to `q`, or a named divergence. */
/* The answer to the question `q` (`qn` bytes: a string argument may hold a NUL
 * byte), the next entry's, or the divergence by name. */
static TraceStr trace_answer_n(const char *q, int64_t qn) {
    int64_t n = elya_trace_next + 1;
    TraceStr asked = {q, qn};
    if (elya_trace_next >= elya_trace_n) {
        elya_platform_flush();
        rt_printf(2, "elya: replay diverged at entry %lld: the program asked `", (long long)n);
        trace_put(asked);
        rt_puts(2, "`, the trace has ended\n");
        elya_platform_exit(1);
    }
    TraceEntry *e = &elya_trace_entries[elya_trace_next];
    if (!(e->q.n == qn && memcmp(e->q.p, q, (size_t)qn) == 0)) {
        elya_platform_flush();
        rt_printf(2, "elya: replay diverged at entry %lld: the program asked `", (long long)n);
        trace_put(asked);
        rt_puts(2, "`, the trace holds `");
        trace_put(e->q);
        rt_puts(2, "`\n");
        elya_platform_exit(1);
    }
    elya_trace_next++;
    return e->a;
}

static TraceStr trace_answer(const char *q) { return trace_answer_n(q, (int64_t)rt_strlen(q)); }

static void trace_wrong_type(TraceStr a, const char *ty) {
    elya_platform_flush();
    rt_printf(2, "elya: replay entry %lld: `", (long long)elya_trace_next);
    trace_put(a);
    rt_printf(2, "` is not an answer of type %s\n", ty);
    elya_platform_exit(1);
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
    int64_t now = elya_platform_now_ms();
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
        trace_line("time.now_ms() = %lld\n", (long long)v);
    }
    return v;
}

static void elya_time_live_sleep_ms(int64_t ms);

void elya_time_sleep_ms(int64_t ms) {
    if (elya_trace_mode == TRACE_REPLAY) {
        char q[64];
        rt_snprintf(q, sizeof q, "time.sleep_ms(%lld)", (long long)ms);
        TraceStr a = trace_answer(q);
        if (!trace_is(a, "Unit")) {
            trace_wrong_type(a, "Unit");
        }
        return;
    }
    elya_time_live_sleep_ms(ms);
    if (elya_trace_mode == TRACE_RECORD) {
        trace_line("time.sleep_ms(%lld) = Unit\n", (long long)ms);
    }
}

static void elya_time_live_sleep_ms(int64_t ms) {
    if (ms > 0) {
        elya_platform_sleep_ms(ms);
    }
}

/* ---- Files, standard input and arguments (spec 2026-11-14-elya-files-and-arguments).
 * Each builtin is the evaluator's (`eval.rs`, `io_builtin` and `host_builtin`):
 * live through the platform's hooks (D5), failing by name in the same words
 * (D2), recorded and replayed in the same trace (D3). A question holds its
 * string arguments as literals, so it and its line are built in working memory,
 * any length; a replay touches no file. Every string argument is rooted first:
 * on a board working memory is the heap's. */

static char **elya_args;
static int elya_args_n;

/* The generated `main`'s first call: the process's arguments (a board's
 * start-up passes none). */
void elya_args_init(int argc, char **argv) {
    elya_args = argv ? elya_platform_args(argc, argv, &elya_args_n) : NULL;
    if (!elya_args || elya_args_n < 0) {
        elya_args_n = 0;
    }
}

/* The program's arguments, not counting the program. */
static int64_t io_arg_count(void) { return elya_args_n > 1 ? (int64_t)elya_args_n - 1 : 0; }

static void io_fail(AnyBuf *m) {
    any_puts(m, "\n");
    elya_platform_flush();
    elya_platform_write(2, m->p, (size_t)m->len);
    elya_platform_exit(1);
}

/* `elya: cannot <verb> `<path>`: <reason>`, status 1. */
static void io_fail_path(const char *verb, const int64_t *path, const char *reason) {
    AnyBuf m = {0};
    any_puts(&m, "elya: cannot ");
    any_puts(&m, verb);
    any_puts(&m, " `");
    any_put(&m, (const char *)&path[2], path[1]);
    any_puts(&m, "`: ");
    any_puts(&m, reason);
    io_fail(&m);
}

/* A path the C library can take: one with a NUL byte would be cut there. */
static const char *io_path(const char *verb, const int64_t *path) {
    const char *c = (const char *)&path[2];
    if (rt_memchr(c, 0, (size_t)path[1])) {
        io_fail_path(verb, path, "a path cannot hold a NUL byte");
    }
    return c;
}

/* `name(<path literal>)`. */
static void io_question(AnyBuf *q, const char *name, const int64_t *path) {
    any_puts(q, name);
    any_puts(q, "(");
    any_put_lit(q, (const char *)&path[2], path[1]);
}

/* A recorded entry: the question, then the answer `a` (`an` bytes, or as a
 * string literal when `lit`), on one line written at once. */
static void io_record(AnyBuf *q, const char *a, int64_t an, int lit) {
    any_puts(q, " = ");
    if (lit) {
        any_put_lit(q, a, an);
    } else {
        any_put(q, a, an);
    }
    any_puts(q, "\n");
    trace_wrote(elya_platform_trace_append(q->p, (size_t)q->len));
}

/* A replay's `String` answer: exactly one literal, as `literal::read_string`
 * reads it, its bytes a fresh string. The trace is UTF-8 and the escapes are
 * ASCII, so the bytes are. */
static void *trace_string(int64_t tag, TraceStr a) {
    AnyBuf out = {0};
    int64_t i = 1;
    if (a.n < 2 || a.p[0] != '"') {
        trace_wrong_type(a, "String");
    }
    for (;;) {
        if (i >= a.n) {
            trace_wrong_type(a, "String");
        }
        char c = a.p[i];
        if (c == '"') {
            i++;
            break;
        }
        if (c == '\\') {
            char e = i + 1 < a.n ? a.p[i + 1] : 0;
            c = e == '"' ? '"' : e == '\\' ? '\\' : e == 'n' ? '\n' : e == 't' ? '\t' : e == 'r' ? '\r' : 0;
            if (!c) {
                trace_wrong_type(a, "String");
            }
            i++;
        }
        any_put(&out, &c, 1);
        i++;
    }
    if (i != a.n) {
        trace_wrong_type(a, "String");
    }
    void *r = elya_str_lit(tag, out.p ? out.p : "", out.len);
    elya_platform_scratch(out.p, 0);
    return r;
}

/* Everything a reader gives until its end, in working memory. */
#define IO_CHUNK 65536

void *elya_io_read_file(int64_t tag, void *path) {
    int64_t *p = (int64_t *)path;
    elya_gc_push(path);
    AnyBuf q = {0};
    io_question(&q, "io.read_file", p);
    any_puts(&q, ")");
    void *r;
    if (elya_trace_mode == TRACE_REPLAY) {
        r = trace_string(tag, trace_answer_n(q.p, q.len));
    } else {
        const char *c = io_path("read", p);
        void *f = elya_platform_path_open(c);
        if (!f) {
            io_fail_path("read", p, elya_platform_path_error(c));
        }
        AnyBuf data = {0};
        for (;;) {
            data.p = (char *)any_grow(data.p, &data.cap, data.len + IO_CHUNK + 1, 1);
            int64_t got = elya_platform_file_read(f, data.p + data.len, IO_CHUNK);
            if (got < 0) {
                /* The reason before the close, which may change it. */
                const char *why = elya_platform_path_error(c);
                elya_platform_file_close(f);
                io_fail_path("read", p, why);
            }
            if (got == 0) {
                break;
            }
            data.len += got;
        }
        elya_platform_file_close(f);
        if (!trace_utf8((const unsigned char *)data.p, data.len)) {
            AnyBuf m = {0};
            any_puts(&m, "elya: `");
            any_put(&m, c, p[1]);
            any_puts(&m, "` is not UTF-8");
            io_fail(&m);
        }
        r = elya_str_lit(tag, data.p, data.len);
        if (elya_trace_mode == TRACE_RECORD) {
            elya_gc_push(r);
            io_record(&q, data.p, data.len, 1);
            elya_gc_pop();
        }
        elya_platform_scratch(data.p, 0);
    }
    elya_platform_scratch(q.p, 0);
    elya_gc_pop();
    return r;
}

int64_t elya_io_file_exists(void *path) {
    int64_t *p = (int64_t *)path;
    elya_gc_push(path);
    AnyBuf q = {0};
    io_question(&q, "io.file_exists", p);
    any_puts(&q, ")");
    int64_t r;
    if (elya_trace_mode == TRACE_REPLAY) {
        TraceStr a = trace_answer_n(q.p, q.len);
        r = trace_is(a, "True");
        if (!r && !trace_is(a, "False")) {
            trace_wrong_type(a, "Bool");
        }
    } else {
        const char *c = (const char *)&p[2];
        r = !rt_memchr(c, 0, (size_t)p[1]) && elya_platform_path_exists(c);
        if (elya_trace_mode == TRACE_RECORD) {
            io_record(&q, r ? "True" : "False", r ? 4 : 5, 0);
        }
    }
    elya_platform_scratch(q.p, 0);
    elya_gc_pop();
    return r;
}

void elya_io_write_file(void *path, void *text) {
    int64_t *p = (int64_t *)path;
    int64_t *t = (int64_t *)text;
    elya_gc_push(path);
    elya_gc_push(text);
    AnyBuf q = {0};
    io_question(&q, "io.write_file", p);
    any_puts(&q, ", ");
    any_put_lit(&q, (const char *)&t[2], t[1]);
    any_puts(&q, ")");
    if (elya_trace_mode == TRACE_REPLAY) {
        TraceStr a = trace_answer_n(q.p, q.len);
        if (!trace_is(a, "Unit")) {
            trace_wrong_type(a, "Unit");
        }
    } else {
        const char *c = io_path("write", p);
        if (!elya_platform_path_write(c, (const char *)&t[2], t[1])) {
            io_fail_path("write", p, elya_platform_path_error(c));
        }
        if (elya_trace_mode == TRACE_RECORD) {
            io_record(&q, "Unit", 4, 0);
        }
    }
    elya_platform_scratch(q.p, 0);
    elya_gc_pop();
    elya_gc_pop();
}

void *elya_io_read_stdin(int64_t tag) {
    static const char q[] = "io.read_stdin()";
    if (elya_trace_mode == TRACE_REPLAY) {
        return trace_string(tag, trace_answer(q));
    }
    AnyBuf data = {0};
    for (;;) {
        data.p = (char *)any_grow(data.p, &data.cap, data.len + IO_CHUNK + 1, 1);
        int64_t got = elya_platform_stdin_read(data.p + data.len, IO_CHUNK);
        if (got < 0) {
            elya_platform_flush();
            rt_printf(2, "elya: cannot read standard input: %s\n", elya_platform_error());
            elya_platform_exit(1);
        }
        if (got == 0) {
            break;
        }
        data.len += got;
    }
    if (!trace_utf8((const unsigned char *)data.p, data.len)) {
        elya_platform_flush();
        rt_puts(2, "elya: standard input is not UTF-8\n");
        elya_platform_exit(1);
    }
    void *r = elya_str_lit(tag, data.p, data.len);
    if (elya_trace_mode == TRACE_RECORD) {
        AnyBuf line = {0};
        elya_gc_push(r);
        any_puts(&line, q);
        io_record(&line, data.p, data.len, 1);
        elya_gc_pop();
        elya_platform_scratch(line.p, 0);
    }
    elya_platform_scratch(data.p, 0);
    return r;
}

int64_t elya_io_arg_count(void) {
    if (elya_trace_mode == TRACE_REPLAY) {
        TraceStr a = trace_answer("io.arg_count()");
        int64_t v;
        if (!trace_int(a, &v)) {
            trace_wrong_type(a, "Int");
        }
        return v;
    }
    int64_t v = io_arg_count();
    if (elya_trace_mode == TRACE_RECORD) {
        trace_line("io.arg_count() = %lld\n", (long long)v);
    }
    return v;
}

void *elya_io_arg(int64_t tag, int64_t i) {
    char q[64];
    rt_snprintf(q, sizeof q, "io.arg(%lld)", (long long)i);
    if (elya_trace_mode == TRACE_REPLAY) {
        return trace_string(tag, trace_answer(q));
    }
    if (i < 0 || i >= io_arg_count()) {
        elya_platform_flush();
        rt_printf(2, "elya: no argument %lld (the program has %lld)\n", (long long)i,
                  (long long)io_arg_count());
        elya_platform_exit(1);
    }
    const char *a = elya_args[i + 1];
    int64_t n = (int64_t)rt_strlen(a);
    if (!trace_utf8((const unsigned char *)a, n)) {
        elya_platform_flush();
        rt_printf(2, "elya: argument %lld is not UTF-8\n", (long long)i);
        elya_platform_exit(1);
    }
    void *r = elya_str_lit(tag, a, n);
    if (elya_trace_mode == TRACE_RECORD) {
        AnyBuf line = {0};
        elya_gc_push(r);
        any_puts(&line, q);
        io_record(&line, a, n, 1);
        elya_gc_pop();
        elya_platform_scratch(line.p, 0);
    }
    return r;
}

/* The deterministic failed-match trap. Spec 5b-4 §5: never `unreachable`. */
void elya_match_fail(void) {
    rt_puts(2, "elya: match failed (no arm matched)\n");
    elya_platform_exit(1);
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
 * A static block `[tag][code]` outside the heap: the collector neither marks
 * nor frees what the heap does not hold (spec 2026-11-12-elya-paged-collector
 * D3). Its tag is no constructor's, so nothing reads past
 * the code word. The code word is set at start-up by elya_handle_end_init,
 * which the generated `main` calls after elya_gc_init: the runtime refers to
 * no generated symbol, so a C harness can link it alone. */
static int64_t elya_handle_end_block[2] = {-1, 0};

void *elya_handle_end = &elya_handle_end_block[0];

void elya_handle_end_init(void *code) {
    elya_handle_end_block[1] = ELYA_WORD(code);
}

/* 5b-8 D13: one-shot is enforced natively. A second resume stops here -- a
 * named message and a non-zero exit, never a re-run. The evaluator's E0425
 * words (spec 2026-10-22 D1). */
void elya_resume_twice(void) {
    rt_puts(2, "elya: a one-shot continuation was resumed twice\n");
    elya_platform_exit(1);
}

/* `Int` arithmetic is exact or fails by name (spec
 * 2026-10-10-elya-checked-integer-arithmetic, D3): the same words, and the
 * same exit status, as the evaluator's E0300. */
void elya_int_overflow(void) {
    rt_puts(2, "elya: integer overflow\n");
    elya_platform_exit(1);
}

void elya_div_zero(void) {
    rt_puts(2, "elya: division by zero\n");
    elya_platform_exit(1);
}

void elya_rem_zero(void) {
    rt_puts(2, "elya: remainder by zero\n");
    elya_platform_exit(1);
}

/* A shift amount outside 0..=63 (spec 2026-10-17-elya-unary-and-wrapping D2):
 * the evaluator's words. */
void elya_shift_range(const char *name, int64_t n) {
    rt_printf(2, "elya: int.%s: a shift by %lld is outside 0..=63\n", name, (long long)n);
    elya_platform_exit(1);
}

/* `float.truncate` (round == 0) and `float.round` (round == 1) (spec
 * 2026-10-24-elya-float-part-2 D4): the evaluator's words on NaN and outside
 * `Int`'s range [-2^63, 2^63), both bounds exact doubles. */
int64_t elya_float_to_int(const char *name, int64_t bits, int64_t round_half_away) {
    double x;
    memcpy(&x, &bits, sizeof x);
    if (x != x) {
        rt_printf(2, "elya: float.%s: NaN has no Int value\n", name);
        elya_platform_exit(1);
    }
    /* Every double at or past 2^52 is an integer, and the doubles next to
     * +-2^63 are 1024 or 2048 apart, so truncating or rounding x lands in
     * range exactly when x does. No libm: `trunc` and `round` would need `-lm`
     * on Linux. */
    if (!(x >= -9223372036854775808.0 && x < 9223372036854775808.0)) {
        char buf[1100];
        int64_t len = elya_float_text(bits, buf);
        rt_printf(2, "elya: float.%s: %.*s is outside Int's range\n", name, (int)len, buf);
        elya_platform_exit(1);
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
    rt_puts(2, "elya: perform: the handler has no clause for this operation\n");
    elya_platform_exit(1);
}

/* Gated OFF by default, and that is load-bearing: every existing execution
 * test asserts stderr is empty, so an unconditional dump would break the whole
 * corpus at once. */
void elya_gc_report(void) {
    if (elya_platform_getenv("ELY_GC_STATS")) {
        rt_printf(2,
                "elya-gc: collections=%lld freed=%lld live=%lld words_since_gc=%lld "
                "mid_copy=%lld\n",
                (long long)gc_collections, (long long)gc_freed, (long long)gc_live,
                (long long)gc_allocated, (long long)gc_copy_collections);
    }
}

/* N6 §6.2: stdout is the program-output channel; stderr remains reserved for
 * collector diagnostics and runtime failures. */
void elya_println(void *s) {
    int64_t *p = (int64_t *)s;
    int64_t len = p[1];
    const char *bytes = (const char *)&p[2];
    elya_platform_write(1, bytes, (size_t)len);
    elya_platform_write(1, "\n", 1);
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
    rt_puts(2, "elya: resume: a continuation's handler chain does not reach its handler\n");
    elya_platform_exit(1);
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
    int64_t **copy = (int64_t **)elya_platform_scratch(NULL, (size_t)n * sizeof(int64_t *));
    if (!copy) {
        rt_puts(2, "elya: out of memory copying a continuation\n");
        elya_platform_exit(1);
    }
    elya_gc_push(cont);
    gc_in_cont_copy = 1;
    int64_t i = 0;
    for (int64_t *f = inner; i < n; f = (int64_t *)(intptr_t)f[CONT_PARENT]) {
        int64_t words = elya_gc_words(f);
        int64_t *p = (int64_t *)elya_alloc(words);
        memcpy(p, f, (size_t)words * 8);
        copy[i] = p;
        elya_gc_push(p);
        i++;
    }
    for (i = 0; i + 1 < n; i++) {
        copy[i][CONT_PARENT] = ELYA_WORD(copy[i + 1]);
    }
    int64_t *fresh = (int64_t *)elya_alloc(4);
    fresh[0] = c[0];
    fresh[1] = c[1];
    fresh[2] = ELYA_WORD(copy[n - 1]);
    fresh[3] = ELYA_WORD(copy[0]);
    gc_in_cont_copy = 0;
    for (i = 0; i <= n; i++) {
        elya_gc_pop();
    }
    elya_platform_scratch(copy, 0);
    return fresh;
}

/* `main`'s `Int` (or `Unit`) value line, written by the `@main` shim. */
void elya_main_int(int64_t v) { rt_printf(1, "%lld\n", (long long)v); }

#ifndef ELYA_FREESTANDING
#include "platform_hosted.c"
#else
/* ---- freestanding (D3): a board gives `elya_platform_write` and
 * `elya_platform_exit` (and the collector's region, `elya_heap_init`); what
 * follows it may replace. No settings, working memory from the heap, a clock that
 * stands still, no files. ---- */
#define ELYA_WEAK __attribute__((weak))
ELYA_WEAK void elya_platform_flush(void) {}
ELYA_WEAK void elya_platform_init(void) {}
ELYA_WEAK const char *elya_platform_getenv(const char *name) {
    (void)name;
    return NULL;
}
/* Working memory from the one heap (gc.c). */
ELYA_WEAK void *elya_platform_scratch(void *p, size_t bytes) { return elya_gc_scratch(p, bytes); }
ELYA_WEAK void *elya_platform_pages(size_t bytes, size_t align) {
    (void)bytes;
    (void)align;
    return NULL;
}
ELYA_WEAK void elya_platform_release(void *p, size_t bytes) {
    (void)p;
    (void)bytes;
}
ELYA_WEAK int64_t elya_platform_now_ms(void) { return 0; }
ELYA_WEAK void elya_platform_sleep_ms(int64_t ms) { (void)ms; }
ELYA_WEAK int elya_platform_trace_create(const char *path) {
    (void)path;
    return 0;
}
ELYA_WEAK int elya_platform_trace_append(const char *bytes, size_t n) {
    (void)bytes;
    (void)n;
    return 0;
}
ELYA_WEAK void *elya_platform_file_open(const char *path) {
    (void)path;
    return NULL;
}
ELYA_WEAK int64_t elya_platform_file_read(void *file, char *into, int64_t n) {
    (void)file;
    (void)into;
    (void)n;
    return -1;
}
ELYA_WEAK void elya_platform_file_close(void *file) { (void)file; }
ELYA_WEAK const char *elya_platform_error(void) { return "this platform has no files"; }
ELYA_WEAK void *elya_platform_path_open(const char *path) {
    (void)path;
    return NULL;
}
ELYA_WEAK int elya_platform_path_exists(const char *path) {
    (void)path;
    return 0;
}
ELYA_WEAK int elya_platform_path_write(const char *path, const char *bytes, int64_t n) {
    (void)path;
    (void)bytes;
    (void)n;
    return 0;
}
ELYA_WEAK const char *elya_platform_path_error(const char *path) {
    (void)path;
    return "this platform has no files";
}
/* No input: at its end at once. */
ELYA_WEAK int64_t elya_platform_stdin_read(char *into, int64_t n) {
    (void)into;
    (void)n;
    return 0;
}
ELYA_WEAK char **elya_platform_args(int argc, char **argv, int *count) {
    *count = argc;
    return argv;
}

/* The program runs on the stack the board's start-up code set. */
typedef int64_t (*elya_entry_fn)(void);
int64_t elya_start(elya_entry_fn entry) {
    elya_trace_init();
    int64_t v = entry();
    elya_trace_finish();
    return v;
}
#endif
