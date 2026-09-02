#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>

/* Allocate `words` 8-byte words, zeroed. Spec 5b-4 §2.1: allocate-don't-collect. */
void *elya_alloc(int64_t words) {
    return calloc((size_t)words, 8);
}

/* The deterministic failed-match trap. Spec 5b-4 §5: never `unreachable`. */
void elya_match_fail(void) {
    fputs("elya: match failed (no arm matched)\n", stderr);
    exit(1);
}
