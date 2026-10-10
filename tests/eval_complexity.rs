//! The evaluator's cost is linear in the work a program does, not in its depth.
//!
//! Spec: `docs/superpowers/specs/2026-10-10-elya-eval-linear-continuations-design.md`.
//! `Interp::cost()` is machine steps plus every continuation node a whole-continuation
//! operation visits or copies (the depth probe, a perform's search and capture, a
//! resume's re-installation). A linear program doubles its cost when its size doubles;
//! a quadratic one quadruples it. So each test runs one shape at n and 2n and asserts
//! `cost(2n) <= 2 * cost(n) + SLACK` -- no timing, no machine dependence.
//!
//! Negative controls (spec §4), each reverted with `git diff --quiet` afterwards:
//! - K1: compute the peak depth by walking the continuation every step (the old probe)
//!   -> all five ratio tests fail (cost ratio ~3.9).
//! - K2: capture a perform's continuation by copying its frames one by one instead of
//!   sharing the segment -> the two perform tests fail; the rest pass.
//! - K3: re-install a resumed continuation by re-pushing its frames -> the two perform
//!   tests fail. (Predicted: the multi-shot test too. It passes: one flip re-enters an
//!   n-frame segment twice, 2n work, which is linear either way.)
//! - K4: remove `KontNode`'s iterative `Drop` -> the abandoned-continuation guard
//!   overflows the test thread's stack.

use elya::parse::parse_module;
use elya::Session;

/// A constant slack for set-up (main's own frames, the handle, the result walk).
const SLACK: u64 = 64;

fn cost_and_value(src: &str) -> (u64, String) {
    // Every program here is one the language accepts.
    elya::check_source("cost.elya", src).unwrap_or_else(|e| panic!("check: {e}\n{src}"));
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let (interp, v) = elya::eval::cek::run_module_value(&m).unwrap();
    (interp.cost(), format!("{v:?}"))
}

/// `shape(n)` is a whole program; asserts it costs linearly and returns both values.
fn assert_linear(name: &str, shape: impl Fn(u64) -> String, n: u64) -> (String, String) {
    let (c1, v1) = cost_and_value(&shape(n));
    let (c2, v2) = cost_and_value(&shape(2 * n));
    assert!(
        c2 <= 2 * c1 + SLACK,
        "{name}: cost({n}) = {c1}, cost({}) = {c2}, ratio {:.2} (linear is 2.00)",
        2 * n,
        c2 as f64 / c1 as f64
    );
    (v1, v2)
}

const LIST: &str = "type L { Nil, Cons(Int, L) }\n\
    fn len(l: L) -> Int { match l {\n Nil -> 0\n Cons(_, t) -> 1 + len(t)\n } }\n\
    fn deep(n: Int) -> L { if n == 0 { Nil } else { Cons(n, deep(n - 1)) } }\n";

#[test]
fn non_tail_recursion_costs_linearly() {
    let shape = |n| format!("{LIST}pub fn main() -> Int {{ len(deep({n})) }}\n");
    let (a, b) = assert_linear("deep", shape, 1000);
    assert_eq!((a.as_str(), b.as_str()), ("Int(1000)", "Int(2000)"));
}

#[test]
fn non_tail_recursion_under_a_handler_costs_linearly() {
    let shape = |n| {
        format!(
            "effect Ask {{ fn ask() -> Int }}\n{LIST}\
             pub fn main() -> Int {{\n  let l = handle {{ deep({n}) }} with {{\n    \
             Ask.ask() -> resume(1)\n    return(x) -> x\n  }}\n  len(l)\n}}\n"
        )
    };
    let (a, b) = assert_linear("deep under a handle", shape, 1000);
    assert_eq!((a.as_str(), b.as_str()), ("Int(1000)", "Int(2000)"));
}

fn perform_at_every_level(n: u64) -> String {
    format!(
        "effect Ask {{ fn ask() -> Int }}\n\
         fn lp(n: Int) -> Int {{ if n == 0 {{ 0 }} else {{ ask() + lp(n - 1) }} }}\n\
         pub fn main() -> Int {{\n  handle {{ lp({n}) }} with {{\n    \
         Ask.ask() -> resume(1)\n    return(x) -> x\n  }}\n}}\n"
    )
}

#[test]
fn a_perform_costs_its_handler_distance_not_its_depth() {
    let (a, b) = assert_linear("perform at every level", perform_at_every_level, 1000);
    assert_eq!((a.as_str(), b.as_str()), ("Int(1000)", "Int(2000)"));
}

#[test]
fn a_perform_past_other_handlers_costs_linearly() {
    // Each `ask` skips two inner handlers of other effects, every level deep.
    let shape = |n| {
        format!(
            "effect Ask {{ fn ask() -> Int }}\n\
             effect T1 {{ fn t1() -> Int }}\n\
             effect T2 {{ fn t2() -> Int }}\n\
             fn lp(n: Int) -> Int {{ if n == 0 {{ 0 }} else {{ ask() + lp(n - 1) }} }}\n\
             pub fn main() -> Int {{\n  handle {{\n    handle {{\n      handle {{ lp({n}) }} with {{\n        \
             T2.t2() -> resume(2)\n        return(x) -> x\n      }}\n    }} with {{\n      \
             T1.t1() -> resume(1)\n      return(x) -> x\n    }}\n  }} with {{\n    \
             Ask.ask() -> resume(1)\n    return(x) -> x\n  }}\n}}\n"
        )
    };
    let (a, b) = assert_linear("perform past two handlers", shape, 1000);
    assert_eq!((a.as_str(), b.as_str()), ("Int(1000)", "Int(2000)"));
}

#[test]
fn a_multi_shot_resume_at_depth_costs_linearly() {
    // One `flip` at the bottom of an n-deep recursion, resumed twice: the
    // captured continuation is n frames, re-entered twice, so 2n frames of work
    // are inherent -- but never n per frame.
    let shape = |n| {
        format!(
            "effect multi Flip {{ fn flip() -> Bool }}\n\
             fn lp(n: Int) -> Int {{ if n == 0 {{ if flip() {{ 1 }} else {{ 2 }} }} else {{ 1 + lp(n - 1) }} }}\n\
             pub fn main() -> Int {{\n  handle {{ lp({n}) }} with multi {{\n    \
             Flip.flip() -> resume(True) * 100000 + resume(False)\n    return(x) -> x\n  }}\n}}\n"
        )
    };
    let (a, b) = assert_linear("multi-shot at depth", shape, 1000);
    assert_eq!(
        (a.as_str(), b.as_str()),
        ("Int(100101002)", "Int(200102002)")
    );
}

#[test]
fn the_parked_shape_runs_a_hundred_thousand_deep() {
    // PARKED (2026-10-03): n = 100 000 did not finish in 10 minutes.
    let src = format!(
        "effect Ask {{ fn ask() -> Int }}\n{LIST}\
         fn lp(n: Int) -> Int {{ if n == 0 {{ 0 }} else {{ ask() + lp(n - 1) }} }}\n\
         pub fn main() -> Int {{\n  let l = handle {{ deep(100000) }} with {{\n    \
         Ask.ask() -> resume(1)\n    return(x) -> x\n  }}\n  \
         let s = handle {{ lp(100000) }} with {{\n    Ask.ask() -> resume(1)\n    \
         return(x) -> x\n  }}\n  len(l) + s\n}}\n"
    );
    let (_, v) = cost_and_value(&src);
    assert_eq!(v, "Int(200000)");
}

#[test]
fn an_abandoned_continuation_a_hundred_thousand_deep_drops() {
    // The clause never resumes: the 100 000-frame captured continuation is
    // dropped. Guard (spec D3): dropping it must not recurse on the host stack.
    let src = "effect Ask { fn ask() -> Int }\n\
               fn lp(n: Int) -> Int { if n == 0 { ask() } else { 1 + lp(n - 1) } }\n\
               pub fn main() -> Int {\n  handle { lp(100000) } with {\n    \
               Ask.ask() -> 7\n    return(x) -> x\n  }\n}\n";
    let (_, v) = cost_and_value(src);
    assert_eq!(v, "Int(7)");
}
