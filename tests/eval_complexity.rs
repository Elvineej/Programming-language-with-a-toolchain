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
//!
//! The lookups (spec `docs/superpowers/specs/2026-10-31-elya-scalability-design.md`
//! D7, D8), each reverted with `cmp` after:
//! - K8: `Env::bind` in place whoever else holds the scope -> the in-place
//!   guard fails (1275 for 375; and 6008 for 6007, checked alone);
//! - K9: `Env::bind` never in place -> both lookup tests fail (ratios 3.96 and
//!   3.99).

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

/// A parameter used down a block of `n` `let`s (spec
/// `docs/superpowers/specs/2026-10-31-elya-scalability-design.md` D7, D8):
/// each `let` made a scope, and every use of the parameter below walked them.
fn a_name_down_a_block(n: u64) -> String {
    let mut src = String::from("fn f(x: Int) -> Int {\n  let a0 = x\n");
    for i in 1..n {
        src.push_str(&format!("  let a{i} = (a{} + x) % 1000003\n", i - 1));
    }
    src.push_str(&format!(
        "  a{}\n}}\npub fn main() -> Int {{ f(7) }}\n",
        n - 1
    ));
    src
}

#[test]
fn a_name_used_down_a_long_block_costs_linearly() {
    let (a, b) = assert_linear("a name down a block", a_name_down_a_block, 1000);
    assert_eq!((a.as_str(), b.as_str()), ("Int(7000)", "Int(14000)"));
}

#[test]
fn the_tree_oracle_looks_a_name_up_down_a_long_block_linearly() {
    let cost = |n| {
        let src = a_name_down_a_block(n);
        let (m, d) = parse_module(&Session::new(), &src);
        assert!(d.is_empty(), "parse: {d:?}");
        elya::eval::run_module_tree(&m).unwrap().cost()
    };
    let (c1, c2) = (cost(1000), cost(2000));
    assert!(
        c2 <= 2 * c1 + SLACK,
        "tree: cost(1000) = {c1}, cost(2000) = {c2}, ratio {:.2} (linear is 2.00)",
        c2 as f64 / c1 as f64
    );
}

/// A binding made in place (D7) is seen by no one who could tell: a closure
/// that captured the scope before a later `let` keeps what it saw, in both
/// evaluators; a resumption entered twice binds its own names each time.
#[test]
fn a_binding_made_in_place_is_seen_by_no_one_else() {
    let captured = "pub fn main() / {IO} {\n  let x = 1\n  let y = 2\n  \
                    let f = fn() { x + y }\n  let y = 5\n  let x = 7\n  \
                    io.println(int.to_string(f() * 100 + x * 10 + y))\n}\n";
    let (m, d) = parse_module(&Session::new(), captured);
    assert!(d.is_empty(), "parse: {d:?}");
    let (interp, _) = elya::eval::cek::run_module_value(&m).unwrap();
    assert_eq!(interp.output(), "375\n", "the machine");
    let tree = elya::eval::run_module_tree(&m).unwrap();
    assert_eq!(tree.output(), "375\n", "the tree oracle");
    // Each resumption reads the outer `a`, never the one the other bound.
    let resumed = "effect multi Flip { fn flip() -> Bool }\n\
                   pub fn main() -> Int {\n  let a = 5\n  handle {\n    \
                   let t = if flip() { 1 } else { 2 }\n    let a = a + t\n    a\n  \
                   } with multi {\n    \
                   Flip.flip() -> resume(True) * 1000 + resume(False)\n    \
                   return(x) -> x\n  }\n}\n";
    let (_, v) = cost_and_value(resumed);
    assert_eq!(v, "Int(6007)");
}
