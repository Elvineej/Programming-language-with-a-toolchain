//! TCE through `match` is a measured guarantee (spec §3.3): a tail-position
//! match preserves tail-call elimination (the `MatchK` frame is transient, so it
//! adds no net depth). `K_MAX_MATCH` is pinned from the first measurement — a
//! tail fold over a million-element list peaks at 3, the same as pure tail
//! recursion (`tce.rs` `K_MAX = 3`). A per-arm frame leak would drive the peak
//! toward the list length; the non-tail grow control below proves the bound has
//! teeth. Do NOT raise `K_MAX_MATCH` to hide a regression — fix the machine.

use elya::parse::parse_module;
use elya::Session;

const K_MAX_MATCH: usize = 3;

fn run_peak(src: &str) -> (String, usize) {
    assert!(
        elya::check_source("t.elya", src).is_ok(),
        "{:?}",
        elya::check_source("t.elya", src)
    );
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let interp = elya::eval::run_module(&m).unwrap();
    (interp.output().to_string(), interp.peak_kont_depth())
}

#[test]
fn tail_fold_over_match_is_bounded() {
    // Build a million-element list tail-recursively, then fold it through a
    // tail-position `match`. Output-verified (runs to completion) AND bounded.
    let src = "type List(a) { Nil, Cons(a, List(a)) }\n\
               fn range(n, acc) { if n == 0 { acc } else { range(n - 1, Cons(n, acc)) } }\n\
               fn sum(acc, xs) { match xs { Nil -> acc  Cons(h, t) -> sum(acc + h, t) } }\n\
               pub fn main() { let xs = range(1000000, Nil)\n let _ = sum(0, xs)\n io.println(\"done\") }\n";
    let (out, peak) = run_peak(src);
    assert_eq!(out, "done\n", "tail fold over match must run to completion");
    assert!(
        peak <= K_MAX_MATCH,
        "tail match fold peak={peak} exceeds K_MAX_MATCH={K_MAX_MATCH}"
    );
}

#[test]
fn non_tail_fold_grows_with_length() {
    // `h + sum(t)` — the recursive call is under `+`, so each `Cons` leaves a
    // frame in the continuation. Peak MUST grow with the list length — proving
    // the machine distinguishes tail-match (flat) from non-tail (grows).
    let prog = |n: i64| {
        format!(
            "type List(a) {{ Nil, Cons(a, List(a)) }}\n\
             fn range(n, acc) {{ if n == 0 {{ acc }} else {{ range(n - 1, Cons(n, acc)) }} }}\n\
             fn sum(xs) {{ match xs {{ Nil -> 0  Cons(h, t) -> h + sum(t) }} }}\n\
             pub fn main() {{ let _ = sum(range({n}, Nil))\n io.println(\"done\") }}\n"
        )
    };
    let (_s5, shallow) = run_peak(&prog(5));
    let (_s50, deep) = run_peak(&prog(50));
    assert!(
        deep > shallow,
        "non-tail match must grow: shallow={shallow}, deep={deep}"
    );
    assert!(
        deep >= 45,
        "expected depth ~proportional to n=50, got {deep}"
    );
}
