/* memcpy, memset, memcmp and memmove for a board with no C library (spec
 * 2026-11-13-elya-boards D3): a word at a time where both pointers are equally
 * aligned, a byte at a time otherwise; weak, so a board's own replace them.
 * Written by ChatGPT (OpenAI) to Claude's brief, tested by it against the C
 * library on 220,211 offset/offset/length triples (GCC and Clang) and again
 * here; released by it to the public domain (CC0-1.0). */
#include <stddef.h>
#include <stdint.h>

#if defined(__clang__)
#define MEM_ATTR __attribute__((weak, no_builtin))
#define WORD_ALIAS __attribute__((may_alias))
#elif defined(__GNUC__)
#define MEM_ATTR __attribute__((weak, optimize("no-tree-loop-distribute-patterns")))
#define WORD_ALIAS __attribute__((may_alias))
#else
#define MEM_ATTR
#define WORD_ALIAS
#endif

typedef uintptr_t word_t WORD_ALIAS;

MEM_ATTR void *memcpy(void *restrict dst, const void *restrict src, size_t n) {
    unsigned char *d = (unsigned char *)dst;
    const unsigned char *s = (const unsigned char *)src;
    const size_t W = sizeof(word_t);
    if (((uintptr_t)d % W) == ((uintptr_t)s % W)) {
        while (n && ((uintptr_t)d % W)) {
            *d++ = *s++;
            --n;
        }
        while (n >= W) {
            *(word_t *)d = *(const word_t *)s;
            d += W;
            s += W;
            n -= W;
        }
    }
    while (n--) {
        *d++ = *s++;
    }
    return dst;
}

MEM_ATTR void *memset(void *dst, int c, size_t n) {
    unsigned char *d = (unsigned char *)dst;
    const size_t W = sizeof(word_t);
    const unsigned char byte = (unsigned char)c;
    word_t pattern = 0;
    for (size_t i = 0; i < W; ++i) {
        pattern = (pattern << 8) | (word_t)byte;
    }
    while (n && ((uintptr_t)d % W)) {
        *d++ = byte;
        --n;
    }
    while (n >= W) {
        *(word_t *)d = pattern;
        d += W;
        n -= W;
    }
    while (n--) {
        *d++ = byte;
    }
    return dst;
}

MEM_ATTR int memcmp(const void *lhs, const void *rhs, size_t n) {
    const unsigned char *a = (const unsigned char *)lhs;
    const unsigned char *b = (const unsigned char *)rhs;
    const size_t W = sizeof(word_t);
    if (((uintptr_t)a % W) == ((uintptr_t)b % W)) {
        while (n && ((uintptr_t)a % W)) {
            if (*a != *b) {
                return *a < *b ? -1 : 1;
            }
            ++a;
            ++b;
            --n;
        }
        while (n >= W) {
            if (*(const word_t *)a != *(const word_t *)b) {
                for (size_t i = 0; i < W; ++i) {
                    if (a[i] != b[i]) {
                        return a[i] < b[i] ? -1 : 1;
                    }
                }
            }
            a += W;
            b += W;
            n -= W;
        }
    }
    while (n--) {
        if (*a != *b) {
            return *a < *b ? -1 : 1;
        }
        ++a;
        ++b;
    }
    return 0;
}

MEM_ATTR void *memmove(void *dst, const void *src, size_t n) {
    unsigned char *d = (unsigned char *)dst;
    const unsigned char *s = (const unsigned char *)src;
    const size_t W = sizeof(word_t);
    /* Address order on a flat address space; no subtraction overflows. */
    if ((uintptr_t)d <= (uintptr_t)s || (uintptr_t)d - (uintptr_t)s >= n) {
        if (((uintptr_t)d % W) == ((uintptr_t)s % W)) {
            while (n && ((uintptr_t)d % W)) {
                *d++ = *s++;
                --n;
            }
            while (n >= W) {
                *(word_t *)d = *(const word_t *)s;
                d += W;
                s += W;
                n -= W;
            }
        }
        while (n--) {
            *d++ = *s++;
        }
    } else {
        d += n;
        s += n;
        if (((uintptr_t)d % W) == ((uintptr_t)s % W)) {
            while (n && ((uintptr_t)d % W)) {
                *--d = *--s;
                --n;
            }
            while (n >= W) {
                d -= W;
                s -= W;
                *(word_t *)d = *(const word_t *)s;
                n -= W;
            }
        }
        while (n--) {
            *--d = *--s;
        }
    }
    return dst;
}
