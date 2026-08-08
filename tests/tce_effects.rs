//! Effect-TCE is a measured guarantee (spec §6): tail-resumptive handlers and
//! tail calls in handler clauses run in bounded continuation depth. `K_MAX_EFF`
//! is pinned from the first measurement (both tail-resumptive loops peak at 4 —
//! the same footing as pure TCE's pinned `K_MAX = 3`). A per-operation splice
//! leak would drive the peak toward the iteration count; the non-tail grow
//! control below proves the bound has teeth. Do NOT raise `K_MAX_EFF` to hide a
//! regression — fix the machine.
//!
//! (The bound is *emergent*: the persistent `Kont` re-push in `resume_apply`
//! splices `k_cap` back into the same slot in tail position — `k_now = k_rest` —
//! and the following recursive call is a tail call that reuses `k`. No explicit
//! tail-detection code was needed; measurement confirms the construction.)

use elya::parse::parse_module;
use elya::Session;

const K_MAX_EFF: usize = 4;

/// Type-check (effects are checked since 3b), then evaluate — returning the
/// program's output and the peak continuation depth.
fn run_effect(src: &str) -> (String, usize) {
    assert!(
        elya::check_source("t.elya", src).is_ok(),
        "must type-check: {:?}",
        elya::check_source("t.elya", src)
    );
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let interp = elya::eval::run_module(&m).unwrap();
    (interp.output().to_string(), interp.peak_kont_depth())
}

#[test]
fn self_tail_resumptive_is_bounded() {
    // A `Gen.yield_()` handler resuming in tail position drives a million-deep
    // countdown. Output-verified AND depth-bounded.
    let src = "effect Gen { fn yield_() -> Bool }\n\
               fn run(n) { if n == 0 { \"end\" } else { if yield_() { run(n - 1) } else { \"stop\" } } }\n\
               pub fn main() { io.println(handle run(1000000) with { Gen.yield_() -> resume(True) }) }\n";
    let (out, peak) = run_effect(src);
    assert_eq!(
        out, "end\n",
        "tail-resumptive loop must produce the right result"
    );
    assert!(
        peak <= K_MAX_EFF,
        "self tail-resumptive peak={peak} exceeds K_MAX_EFF={K_MAX_EFF}"
    );
}

#[test]
fn non_tail_resume_grows_with_depth() {
    // Grow control (the bound's teeth): the clause sequences work AFTER `resume`
    // (`resume(Unit) <> "."` — resume is NOT in tail position), so each operation
    // leaves a frame in the continuation. Peak MUST grow with depth — proving the
    // machine distinguishes tail-resume (flat) from non-tail (grows), so the
    // bounded assertion can't be quietly loosened to hide a splice leak.
    let prog = |n: i64| {
        format!(
            "effect Tick {{ fn tick() -> Unit }}\n\
             fn count(n) {{ if n == 0 {{ \"x\" }} else {{ let _ = tick()  count(n - 1) }} }}\n\
             pub fn main() {{ io.println(handle count({n}) with {{ Tick.tick() -> resume(Unit) <> \".\" }}) }}\n"
        )
    };
    let (_os, shallow) = run_effect(&prog(5));
    let (_od, deep) = run_effect(&prog(50));
    assert!(
        deep > shallow,
        "non-tail resume must grow: shallow={shallow}, deep={deep}"
    );
    assert!(
        deep >= 45,
        "expected depth ~proportional to n=50, got {deep}"
    );
}

#[test]
fn mutual_tail_resumptive_is_bounded() {
    // Two mutually-recursive functions, each performing an op resumed in tail
    // position. Output-verified AND depth-bounded.
    let src = "effect Tick { fn tick() -> Bool }\n\
               fn ev(n) { if n == 0 { \"done\" } else { if tick() { od(n - 1) } else { \"stop\" } } }\n\
               fn od(n) { if n == 0 { \"done\" } else { if tick() { ev(n - 1) } else { \"stop\" } } }\n\
               pub fn main() { io.println(handle ev(1000000) with { Tick.tick() -> resume(True) }) }\n";
    let (out, peak) = run_effect(src);
    assert_eq!(
        out, "done\n",
        "mutual tail-resumptive loop must produce the right result"
    );
    assert!(
        peak <= K_MAX_EFF,
        "mutual tail-resumptive peak={peak} exceeds K_MAX_EFF={K_MAX_EFF}"
    );
}
