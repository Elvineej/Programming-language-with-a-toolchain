//! The collector on its own, as a board runs it (spec
//! 2026-11-12-elya-paged-collector): `gc.c` with a platform that gives no
//! memory but one fixed region (`elya_heap_init`), and `gc.c` built
//! freestanding for bare-metal targets.

use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-gc-pages-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn gc_c() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/gc.c")
}

/// A board's platform: no memory but the region it gives at start; failing
/// prints the message and exits 3. Constructor 0 is `Cons(Int, List)` (arity
/// 2, field 1 a pointer); constructor 1 is `Nil`.
const HARNESS: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include "GC_C"
void *elya_current_handler = 0;
/* A static block outside the heap, the current handler throughout (as
 * `elya_handle_end` may be), in zeroed memory that must stay zero. */
static struct { int64_t canary[1024]; int64_t blk[2]; int64_t after[1024]; } statics
    __attribute__((aligned(4096))) = {{0}, {-1, 0}, {0}};
static int canary_ok(void) {
    for (int i = 0; i < 1024; i++) if (statics.canary[i] || statics.after[i]) return 0;
    return 1;
}
#ifdef HOSTED_PAGES
/* A host's page source, for the tests of growth. */
void *elya_platform_pages(size_t bytes, size_t align) {
    char *m = (char *)malloc(bytes + align);
    return m ? (void *)(((uintptr_t)m + align - 1) & ~(uintptr_t)(align - 1)) : 0;
}
#else
void *elya_platform_pages(size_t bytes, size_t align) { (void)bytes; (void)align; return 0; }
#endif
void elya_platform_release(void *p, size_t bytes) { (void)p; (void)bytes; }
void elya_platform_fail(const char *m) { fputs(m, stdout); fflush(stdout); exit(3); }
static unsigned char region[REGION] __attribute__((aligned(4096)));
static int64_t shadow_store[1024];
static const int64_t descriptors[] = {2, 2, 0, 0, 2, 3, 2, 3};
static int64_t *nil(void) { int64_t *n = (int64_t *)elya_alloc(1); n[0] = 1; return n; }
static int64_t *node(int64_t *l, int64_t *r) {
    elya_gc_push(l); elya_gc_push(r);
    int64_t *n = (int64_t *)elya_alloc(3);
    elya_gc_pop(); elya_gc_pop();
    n[0] = 2; n[1] = (int64_t)(intptr_t)l; n[2] = (int64_t)(intptr_t)r;
    return n;
}
static int64_t *tree(int d) {
    if (d == 0) return nil();
    int64_t *l = tree(d - 1);
    elya_gc_push(l);
    int64_t *r = tree(d - 1);
    elya_gc_pop();
    return node(l, r);
}
static int64_t leaves(int64_t *t) { return t[0] == 2 ? leaves((int64_t *)(intptr_t)t[1]) + leaves((int64_t *)(intptr_t)t[2]) : 1; }
static int64_t *cons(int64_t h, int64_t *t) {
    elya_gc_push(t);
    int64_t *c = (int64_t *)elya_alloc(3);
    elya_gc_pop();
    c[0] = 0; c[1] = h; c[2] = (int64_t)(intptr_t)t;
    return c;
}
static int64_t sum(int64_t *xs) { int64_t s = 0; while (xs[0] == 0) { s += xs[1]; xs = (int64_t *)(intptr_t)xs[2]; } return s; }
int main(int argc, char **argv) {
    (void)argc;
    /* The shadow stack is the region's too on a board: give it one up front. */
    gc_shadow = (void **)shadow_store; gc_shadow_cap = 1024;
#ifndef HOSTED_PAGES
    elya_heap_init(region, sizeof region);
#endif
    elya_current_handler = &statics.blk[0];
#ifndef STRESS
#define STRESS 0
#endif
    elya_gc_setup(descriptors, 4, STRESS);
    long keep = atol(argv[1]), rounds = atol(argv[2]);
    if (argc > 3 && argv[3][0] == 'P') {
        /* Stress mode: a cell no root names is freed by the next allocation's
         * collection. Read through the stale pointer, it holds the poison;
         * written through it (`keep`), the next sweep stops the program. */
        volatile int64_t *stale = (int64_t *)elya_alloc(3);
        stale[0] = 1;
        (void)elya_alloc(3);
        int poisoned = stale[1] == (int64_t)0xDEADBEEFDEADBEEFULL;
        if (keep) stale[rounds] = 7;
        (void)elya_alloc(3);
        printf("poisoned=%d no stale write seen\n", poisoned);
        return 0;
    }
    int64_t total = 0;
    int64_t *kept = argc > 3 && argv[3][0] >= '0' && argv[3][0] <= '9' ? tree(atoi(argv[3])) : 0;
    elya_gc_push(kept);
    if (argc > 3 && argv[3][0] == 'L') {
        /* Large objects: `keep` words each, kept two at a time and checked
         * after each allocation, `rounds` times (the review's B1, B2). */
        int64_t bad = 0;
        int64_t *a = 0, *b = 0;
        elya_gc_push(0); elya_gc_push(0);
        for (long r = 0; r < rounds; r++) {
            int64_t *n = (int64_t *)elya_alloc(keep);
            n[0] = 1; /* no constructor's tag past this: the bytes are data */
            for (long i = 1; i < keep; i++) n[i] = r * 7 + i;
            a = b; b = n;
            gc_shadow[gc_shadow_top - 2] = a; gc_shadow[gc_shadow_top - 1] = b;
            for (long i = 1; a && i < keep; i++) bad += a[i] != (r - 1) * 7 + i;
        }
        printf("total=%lld collections=%lld\n", (long long)bad, (long long)gc_collections);
        return 0;
    }
    if (argc > 3 && argv[3][0] == 'S') {
        /* A list whose cells each hold a leaf before the tail: marked, the
         * worklist must stay bounded (the review's quadratic rescans). */
        int64_t *xs = nil();
        elya_gc_push(xs);
        for (long i = 0; i < keep; i++) {
            int64_t *leaf = nil();
            elya_gc_push(leaf);
            int64_t *c = (int64_t *)elya_alloc(3);
            elya_gc_pop();
            c[0] = 3; c[1] = (int64_t)(intptr_t)leaf; c[2] = (int64_t)(intptr_t)xs;
            xs = c;
            gc_shadow[gc_shadow_top - 1] = xs;
        }
        gc_collect();
        printf("total=0 collections=%lld overflows=%lld\n", (long long)gc_collections, (long long)gc_overflows);
        return 0;
    }
    if (argc > 3 && argv[3][0] == 'K') {
        /* The classes: each medium one fills its span; what an object of r
         * words takes of a span, over its size, at worst and on average (in
         * thousandths), for r from 129 words to the last class. */
        int64_t bad = 0, worst = 0, sum = 0, n = 0;
        int last = gc_n_classes - 1;
        for (int c = GC_SMALL_CLASSES; c <= last; c++) {
            size_t room = ((size_t)gc_class_pages[c] * ELYA_PAGE_BYTES - GC_PAGE_HEADER) / 8;
            size_t k = room / gc_class_words[c];
            bad += k < 2 || room / k != gc_class_words[c] || gc_class_words[c] <= gc_class_words[c - 1];
        }
        for (int64_t r = 129; r <= (int64_t)gc_class_words[last]; r++) {
            int c = GC_SMALL_CLASSES;
            while ((int64_t)gc_class_words[c] < r) c++;
            size_t room = ((size_t)gc_class_pages[c] * ELYA_PAGE_BYTES - GC_PAGE_HEADER) / 8;
            int64_t per = (int64_t)((size_t)gc_class_pages[c] * ELYA_PAGE_BYTES / (room / gc_class_words[c]));
            int64_t ratio = per * 1000 / (8 * r);
            if (ratio > worst) worst = ratio;
            sum += ratio; n++;
        }
        printf("total=%lld classes=%d last=%u worst=%lld mean=%lld\n", (long long)bad, gc_n_classes,
               gc_class_words[last], (long long)worst, (long long)(sum / n));
        return 0;
    }
    if (argc > 3 && argv[3][0] == 'G') {
        /* One object of `keep` words from a growing heap: both ends written. */
        int64_t *big = (int64_t *)elya_alloc(keep);
        big[0] = 1; big[keep - 1] = 2;
        printf("total=0 words=%lld\n", (long long)elya_gc_words(big));
        return 0;
    }
    if (argc > 3 && argv[3][0] == 'A') {
        /* A working block freed is not counted toward the next collection. */
        void *w = elya_gc_scratch(0, 1 << 20);
        elya_gc_scratch(w, 0);
        int64_t before = gc_collections;
        (void)cons(1, 0);
        printf("total=%lld\n", (long long)(gc_collections - before));
        return 0;
    }
    if (argc > 3 && argv[3][0] == 'W') {
        /* Working memory from the heap: two blocks held across collections
         * (one grown while held, its bytes kept), then freed; `rounds` times
         * in a region far smaller than what was asked in all. */
        int64_t bad = 0;
        /* A block grown is as large as asked, whatever its slot's class. */
        for (size_t n = 1; n < 3000; n += 7) {
            void *p = elya_gc_scratch(0, n);
            void *q = elya_gc_scratch(p, n + 40);
            bad += ((size_t)elya_gc_words((int64_t *)q - 3) - 3) * 8 < n + 40;
            elya_gc_scratch(q, 0);
        }
        /* A block freed on a page not swept since the last collection: that
         * page is not allocated from before its sweep (the sweep would free
         * what was), so an object made next keeps its words. */
        for (int t = 0; t < 50; t++) {
            void *held = elya_gc_scratch(0, 8);
            gc_collect();
            elya_gc_scratch(held, 0);
            int64_t *x = (int64_t *)elya_alloc(4);
            elya_gc_push(x);
            x[0] = 0; x[1] = 2 * 0x5a5a5a5a + 1; x[2] = 2 * t + 1; x[3] = 1;
            for (long i = 0; i < 4000; i++) {
                int64_t *g = (int64_t *)elya_alloc(4);
                g[0] = 0; g[1] = 1; g[2] = 1; g[3] = 1;
            }
            bad += x[1] != 2 * 0x5a5a5a5a + 1 || x[2] != 2 * t + 1;
            elya_gc_pop();
        }
        for (long r = 0; r < rounds; r++) {
            /* A collection is due, and a working block is allocated: it
             * does not collect (its caller's values may be on no root). */
            gc_allocated = GC_THRESHOLD_WORDS + gc_live;
            int64_t before = gc_collections;
            unsigned char *a = (unsigned char *)elya_gc_scratch(0, (size_t)keep);
            bad += gc_collections != before;
            unsigned char *b = (unsigned char *)elya_gc_scratch(0, (size_t)keep * 3);
            for (long i = 0; i < keep; i++) a[i] = (unsigned char)(r + i);
            for (long i = 0; i < keep * 3; i++) b[i] = (unsigned char)(r ^ i);
            for (long i = 0; i < 3000; i++) (void)cons(i, 0);
            a = (unsigned char *)elya_gc_scratch(a, (size_t)keep * 2);
            /* Grown six times with no collection between: a large block's
             * pages come back at once, or the six would not fit. */
            for (int k = 0; k < 6; k++) a = (unsigned char *)elya_gc_scratch(a, (size_t)keep * 2 + (size_t)(k + 1) * 4096);
            for (long i = 0; i < 3000; i++) (void)cons(i, 0);
            gc_collect();
            for (long i = 0; i < keep; i++) bad += a[i] != (unsigned char)(r + i);
            for (long i = 0; i < keep * 3; i++) bad += b[i] != (unsigned char)(r ^ i);
            if (elya_gc_scratch(a, 0) || elya_gc_scratch(b, 0)) bad++;
        }
        bad += gc_scratch_held != 0;
        printf("total=%lld collections=%lld\n", (long long)bad, (long long)gc_collections);
        return 0;
    }
    if (argc > 3 && argv[3][0] == 'i') {
        /* Interleaved: each round keeps one cell, made between garbage
         * cells on the same pages, so no page ever empties. */
        int64_t *alive = nil();
        elya_gc_push(alive);
        for (long r = 0; r < rounds; r++) {
            for (long i = 0; i < keep; i++) (void)cons(i, 0);
            alive = cons(1, alive);
            gc_shadow[gc_shadow_top - 1] = alive;
        }
        printf("total=%lld collections=%lld\n", (long long)sum(alive), (long long)gc_collections);
        return 0;
    }
    for (long r = 0; r < rounds; r++) {
        int64_t *xs = nil();
        for (long i = 1; i <= keep; i++) xs = cons(i, xs);
        total += sum(xs);
    }
    if (kept) total += leaves(kept);
    printf("total=%lld collections=%lld%s\n", (long long)total, (long long)gc_collections,
           canary_ok() ? "" : " canary=written");
    return 0;
}
"#;

fn run_board(tag: &str, region: usize, keep: u64, rounds: u64) -> (i32, String) {
    run_board_with(tag, region, keep, rounds, &[], None)
}

fn run_board_with(
    tag: &str,
    region: usize,
    keep: u64,
    rounds: u64,
    defines: &[&str],
    tree: Option<&str>,
) -> (i32, String) {
    let dir = temp_dir(tag);
    let c = dir.join("board.c");
    let gc = gc_c().to_string_lossy().replace('\\', "/");
    std::fs::write(&c, HARNESS.replace("GC_C", &gc)).unwrap();
    let exe = dir.join(format!("board{}", std::env::consts::EXE_SUFFIX));
    let built = Command::new("clang")
        .arg("-O2")
        .arg(format!("-DREGION={region}"))
        .args(defines)
        .arg(&c)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("spawn clang");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let mut run = Command::new(&exe);
    run.arg(keep.to_string()).arg(rounds.to_string());
    if let Some(d) = tree {
        run.arg(d);
    }
    let out = run.output().expect("run board");
    std::fs::remove_dir_all(&dir).ok();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Stress mode (a collection at every allocation): a freed block holds its
/// poison, and a write through a stale pointer, to any of its words, stops
/// the program at the next sweep (2026-11-14; it was silent before, and in an
/// ordinary run corrupts whatever reuses the slot).
#[test]
fn a_write_to_a_freed_block_is_caught_under_stress() {
    let stress = ["-DSTRESS=1"];
    let (code, out) = run_board_with("stale-read", 64 << 10, 0, 0, &stress, Some("P"));
    assert_eq!(
        (code, out.trim_end()),
        (0, "poisoned=1 no stale write seen")
    );
    for word in 0..3 {
        let (code, out) = run_board_with("stale-write", 64 << 10, 1, word, &stress, Some("P"));
        assert_eq!(
            (code, out.trim_end()),
            (3, "elya: gc stress: a freed block was written"),
            "word {word}"
        );
    }
    // Without stress the same writes pass unseen: the mode is what catches them.
    let (code, out) = run_board_with("stale-quiet", 64 << 10, 1, 1, &[], Some("P"));
    assert_eq!(
        (code, out.trim_end()),
        (0, "poisoned=0 no stale write seen")
    );
}

/// 64 KiB, no other memory: 2000 lists of 1000 cells (about 6 MB in all) are
/// made and dropped, the region reused by collections, and every sum right.
#[test]
fn a_fixed_region_is_reused_by_collections() {
    let (code, out) = run_board("reuse", 64 << 10, 1000, 2000);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("total=1001000000 "), "{out}");
    // The static block was grayed at every collection and nothing near it
    // written (D3).
    assert!(!out.contains("canary"), "{out}");
    let collections: i64 = out
        .trim()
        .rsplit("collections=")
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(collections > 10, "{out}");
}

/// Marking with a worklist of two entries: a kept tree of 2^10 leaves (a node
/// pushes two children) overflows it at every level, and the rescan (D5)
/// marks all of it while lists churn the region around it.
#[test]
fn an_overflowing_mark_stack_is_rescanned() {
    let (code, out) = run_board_with(
        "overflow",
        256 << 10,
        1000,
        500,
        &["-DELYA_MARK_STACK=2"],
        Some("10"),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("total=250251024 "), "{out}");
}

/// Survivors among garbage on the same pages: a cell kept per round between
/// 100 dropped ones, 2000 rounds (about 4.8 MB made, 48 KB kept) in 128 KiB.
/// No page ever empties, so this needs the dead slots of live pages reused.
#[test]
fn dead_slots_among_live_ones_are_reused() {
    let (code, out) = run_board_with("interleave", 128 << 10, 100, 2000, &[], Some("i"));
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("total=2000 "), "{out}");
}

/// Large objects two at a time, each checked after the next is made, in a
/// fixed region: a run's later pages are never taken while it lives (the
/// review's B1), and freed runs and one-page objects are reused (B2). Sizes:
/// one page, a page and a bit, three pages.
#[test]
fn large_objects_keep_their_pages_and_reuse_freed_ones() {
    for words in [200, 600, 1300] {
        let (code, out) = run_board_with(
            &format!("large-{words}"),
            256 << 10,
            words,
            3000,
            &[],
            Some("L"),
        );
        assert_eq!(code, 0, "{words}: {out}");
        assert!(out.starts_with("total=0 "), "{words}: {out}");
    }
}

/// The runtime's working memory from the same heap (spec
/// 2026-11-13-elya-platform-layer D5): blocks of 1 KiB, 5 KiB and 40 KiB held
/// across collections keep their bytes, a block grown while held keeps them,
/// allocating one never collects, and freed blocks are the collector's again (400 rounds in a region one
/// round nearly fills).
#[test]
fn working_memory_is_held_from_the_heap_and_given_back() {
    for bytes in [1 << 10, 5 << 10, 40 << 10] {
        let (code, out) = run_board_with(
            &format!("scratch-{bytes}"),
            768 << 10,
            bytes,
            400,
            &[],
            Some("W"),
        );
        assert_eq!(code, 0, "{bytes}: {out}");
        assert!(out.starts_with("total=0 "), "{bytes}: {out}");
    }
}

/// The medium classes are made for the page size: each fills its span exactly,
/// and an object's share of a span is at most a third over its size (a tenth
/// on average) at 4 KiB, 1 KiB and 512-byte pages. The first table's fixed
/// sizes in two-slot spans were 99% over at worst, 29% on average (ChatGPT's
/// review measured it: a 5 KB string took 8 KiB).
#[test]
fn medium_classes_fill_their_spans() {
    for page in ["4096", "1024", "512"] {
        let define = format!("-DELYA_PAGE_BYTES={page}");
        let (code, out) = run_board_with(
            &format!("classes-{page}"),
            4 << 20,
            1,
            1,
            &[&define],
            Some("K"),
        );
        assert_eq!(code, 0, "{page}: {out}");
        assert!(out.starts_with("total=0 "), "{page}: {out}");
        let field = |name: &str| -> i64 {
            out.split_whitespace()
                .find_map(|w| w.strip_prefix(name))
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| panic!("{name} in {out}"))
        };
        assert!(field("worst=") <= 1340, "{page}: {out}");
        assert!(field("mean=") <= 1110, "{page}: {out}");
        assert!(field("last=") >= 500, "{page}: {out}");
    }
}

/// An object just under a chunk's size grows the heap by a chunk that holds it
/// (ChatGPT's review: 8,386,026 words needed 16,380 pages, the 64 MiB chunk
/// chosen held 16,376, and the allocation failed "out of memory"). The sizes
/// around each chunk size from 1 MiB to 64 MiB.
#[test]
fn an_object_just_under_a_chunk_grows_a_chunk_that_holds_it() {
    for chunk in [1usize << 20, 4 << 20, 16 << 20, 64 << 20] {
        for under in [0usize, 1, 2, 3, 5, 9, 17] {
            let words = (chunk - under * 4096) / 8 - 40;
            let (code, out) = run_board_with(
                &format!("grow-{chunk}-{under}"),
                4096,
                words as u64,
                1,
                &["-DHOSTED_PAGES"],
                Some("G"),
            );
            assert_eq!(code, 0, "{words}: {out}");
            assert!(out.starts_with("total=0 "), "{words}: {out}");
        }
    }
}

/// A working block is not counted toward the next collection (ChatGPT's
/// review: 1 MiB asked and freed made the next one-word allocation collect).
#[test]
fn a_working_block_brings_no_collection_nearer() {
    let (code, out) = run_board_with("scratch-count", 4 << 20, 1, 1, &[], Some("A"));
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("total=0"), "{out}");
}

/// A list of 100K cells, each holding a leaf before its tail, marked with a
/// worklist of 64: it never overflows (the leaf is taken first), where it
/// overflowed every 64 cells and rescanned the heap each time (the review's
/// quadratic rescans).
#[test]
fn a_list_with_leaves_marks_in_a_bounded_worklist() {
    let (code, out) = run_board_with(
        "leaves",
        16 << 20,
        100_000,
        0,
        &["-DELYA_MARK_STACK=64"],
        Some("S"),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.trim().ends_with("overflows=0"), "{out}");
}

/// A live set larger than the region stops by name, not by a crash.
#[test]
fn a_live_set_larger_than_the_region_fails_by_name() {
    let (code, out) = run_board("oom", 64 << 10, 100_000, 1);
    assert_eq!(code, 3, "{out}");
    // The harness's hook writes through the C library, in text mode on Windows.
    assert_eq!(out.replace("\r\n", "\n"), "elya: out of memory\n");
}

/// The collector builds freestanding for bare-metal targets, needing only the
/// platform hooks, the handler root, and what a freestanding C compiler
/// requires (`memcpy`, `memset`, and ARM's EABI helpers).
#[test]
fn the_collector_builds_freestanding() {
    let dir = temp_dir("freestanding");
    for target in [
        "x86_64-unknown-none-elf",
        "thumbv7em-none-eabi",
        "riscv32-unknown-none-elf",
    ] {
        let obj = dir.join(format!("gc-{target}.o"));
        let built = Command::new("clang")
            .args([
                "-ffreestanding",
                "-nostdlib",
                "-O2",
                "-Wall",
                "-Werror",
                "-c",
            ])
            .arg(format!("--target={target}"))
            .arg(gc_c())
            .arg("-o")
            .arg(&obj)
            .output()
            .expect("spawn clang");
        assert!(
            built.status.success(),
            "{target}: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        let nm = Command::new(llvm_nm())
            .arg("-u")
            .arg(&obj)
            .output()
            .expect("spawn llvm-nm");
        let undefined: Vec<String> = String::from_utf8_lossy(&nm.stdout)
            .lines()
            .filter_map(|l| l.split_whitespace().last().map(str::to_string))
            .collect();
        for u in &undefined {
            assert!(
                u.starts_with("elya_platform_")
                    || u == "elya_current_handler"
                    || u == "memcpy"
                    || u == "memset"
                    || u.starts_with("__aeabi_"),
                "{target}: the collector needs `{u}`: {undefined:?}"
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

fn llvm_nm() -> String {
    for name in ["llvm-nm", "llvm-nm-18"] {
        if Command::new(name).arg("--version").output().is_ok() {
            return name.to_string();
        }
    }
    "llvm-nm".to_string()
}
