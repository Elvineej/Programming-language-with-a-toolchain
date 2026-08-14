//! TCE through a *closure* call is a measured guarantee (spec §3.3). The
//! closure-call path reuses the continuation, so a tail call to a closure adds no
//! net depth. We drive a million alternating tail calls between a top-level `drive`
//! and a closure `k` (the closure references top-level fns by name — no recursive
//! local closures, and no self-application, which HM's occurs-check would reject).
//! A per-closure-call frame leak would drive the peak toward the iteration count;
//! the non-tail grow control proves the bound has teeth. Do NOT raise the constant
//! to hide a regression — fix the machine.

use elya::parse::parse_module;
use elya::Session;

// Pinned from the first measurement: peak = 3, *identical* to pure tail recursion
// (`tce.rs` K_MAX = 3) and tail-position `match` (K_MAX_MATCH = 3) — a closure tail
// call adds zero net depth. The constant-in-N test below is the real bound; this
// tight ceiling additionally catches a constant +1 frame leak. Never raise it to
// mask a regression — fix the machine.
const K_MAX_CLOSURE: usize = 3;

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

fn drive_prog(n: i64) -> String {
    // `drive`'s else-branch tail-calls the closure `k`; `k`'s body tail-calls
    // `drive` with a fresh closure from `mk`. Both tail positions fire ~n times.
    format!(
        "fn drive(n, k) {{ if n == 0 {{ 0 }} else {{ k(n) }} }}\n\
         fn mk() {{ fn(m) {{ drive(m - 1, mk()) }} }}\n\
         pub fn main() {{ let _ = drive({n}, mk())\n io.println(\"done\") }}\n"
    )
}

#[test]
fn tail_closure_call_is_bounded() {
    let (out, peak) = run_peak(&drive_prog(1_000_000));
    assert_eq!(
        out, "done\n",
        "closure tail-call loop must run to completion"
    );
    assert!(
        peak <= K_MAX_CLOSURE,
        "closure tail-call peak={peak} exceeds K_MAX_CLOSURE={K_MAX_CLOSURE}"
    );
}

#[test]
fn tail_closure_call_peak_is_constant_in_n() {
    // The real property: peak does not grow with the iteration count.
    let (_a, small) = run_peak(&drive_prog(100_000));
    let (_b, large) = run_peak(&drive_prog(1_000_000));
    assert_eq!(
        small, large,
        "closure tail-call peak must be constant in N: {small} vs {large}"
    );
}

#[test]
fn non_tail_closure_call_grows_with_length() {
    // `f(h) + fold(t, f)` — the recursive call is under `+`, so each element leaves
    // a frame; peak grows with the list length. Proves the bound distinguishes
    // tail (flat) from non-tail (grows).
    let prog = |n: i64| {
        format!(
            "type List(a) {{ Nil, Cons(a, List(a)) }}\n\
             fn range(n, acc) {{ if n == 0 {{ acc }} else {{ range(n - 1, Cons(n, acc)) }} }}\n\
             fn fold(xs, f) {{ match xs {{ Nil -> 0  Cons(h, t) -> f(h) + fold(t, f) }} }}\n\
             pub fn main() {{ let _ = fold(range({n}, Nil), fn(x) {{ x }})\n io.println(\"done\") }}\n"
        )
    };
    let (_s, shallow) = run_peak(&prog(5));
    let (_d, deep) = run_peak(&prog(50));
    assert!(
        deep > shallow,
        "non-tail closure fold must grow: {shallow} vs {deep}"
    );
    assert!(
        deep >= 45,
        "expected depth ~proportional to n=50, got {deep}"
    );
}
