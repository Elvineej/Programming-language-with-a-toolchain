//! Sub-effecting for function values (the maintainer's decision, 2026-10-08;
//! spec `docs/superpowers/specs/2026-10-09-elya-sub-effecting-design.md`).
//!
//! A function that performs ε may be used where a function performing more is
//! expected. Rows still unify by equality; sub-effecting is row-polymorphic
//! upcasting, Koka's open/close discipline:
//! - a lambda literal's TYPE gets a fresh "phantom" tail when its body's row is
//!   closed, and so does a closed row in a covariant position at every VALUE
//!   use of a name (`open_covariant`); a callee is not a value use;
//! - a call whose callee row ends in a phantom tail not shared with a parameter
//!   type records an INCLUSION (ambient ⊇ what the tail becomes), flushed --
//!   labels only -- before a lambda's ambient closes and at the end of the
//!   group, instead of unifying the two rows (which made them equal);
//! - phantom tails nothing bound are closed when the group is done, so printed
//!   schemes and Core types are unchanged.
//!
//! Negative controls, each reverted (`cmp` against a saved copy):
//! - C1, no opening (lambda literals and value uses keep closed rows): the five
//!   acceptance tests fail (E0423);
//! - C2, a phantom callee tail unified with the ambient instead of recorded:
//!   the handle-result and `if`-join tests fail (E0423), the rejection tests
//!   still pass;
//! - C3, the inclusions never flushed: `a_function_picked_at_run_time_still_
//!   carries_its_effects` checks clean -- the soundness test.

use elya::{check_source, run_source};

fn checks_and_prints(src: &str, want: &str) {
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
    assert_eq!(run_source("t.elya", src).expect("runs"), want);
}

#[test]
fn a_pure_lambda_joins_an_effectful_one() {
    // Rejected before (E0423): the two branches' rows differed by {L}.
    checks_and_prints(
        "effect L { fn lg(x: Int) -> Int }\n\
         fn go(n) {\n\
           let a = lg(n)\n\
           let f = if n == 0 { fn(s) { s } } else { fn(s) { s + go(n - 1) } }\n\
           f(1) + a\n\
         }\n\
         pub fn main() {\n\
           let r = handle { go(2) } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
           io.println(if r == 6 { \"6\" } else { \"wrong\" })\n\
         }\n",
        "6\n",
    );
}

#[test]
fn a_pure_callback_fits_a_parameter_that_admits_more() {
    // `w`'s own `lg` shares its row with `k`'s (a relay); a pure `k` used to be
    // E0423.
    checks_and_prints(
        "effect L { fn lg(x: Int) -> Int }\n\
         fn w(k) { k(0) + lg(1) }\n\
         pub fn main() {\n\
           let r = handle { w(fn(x) { x }) } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
           io.println(if r == 1 { \"1\" } else { \"wrong\" })\n\
         }\n",
        "1\n",
    );
}

#[test]
fn a_parameter_joined_with_a_pure_lambda_keeps_its_effects() {
    checks_and_prints(
        "effect L { fn lg(x: Int) -> Int }\n\
         fn w(k) { let a = k(0)  let z = if a == 0 { k } else { fn(x) { x } }  a + z(1) + lg(1) }\n\
         pub fn main() {\n\
           let r = handle { w(fn(x) { x }) } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
           io.println(if r == 2 { \"2\" } else { \"wrong\" })\n\
         }\n",
        "2\n",
    );
}

#[test]
fn clause_values_with_different_rows_join_in_the_handle_result() {
    // The resuming lambda performs T (the rest of the body); the return
    // clause's does not.
    checks_and_prints(
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         pub fn main() {\n\
           let r = handle {\n\
             let f = handle { get() + t() } with { S.get() -> fn(k) { (resume(k(1)))(k) }  return(x) -> fn(k) { k(x) } }\n\
             f(fn(z) { z * 2 })\n\
           } with { T.t() -> resume(10)  return(r) -> r }\n\
           io.println(if r == 24 { \"24\" } else { \"wrong\" })\n\
         }\n",
        "24\n",
    );
}

#[test]
fn a_resumed_body_run_inside_a_clause_handler() {
    // The review's c8: rejected before (the handled effect was forced into an
    // open-row function value's row); the evaluator gives 12.
    checks_and_prints(
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         pub fn main() {\n\
           let f = handle {\n\
             handle { get() + t() } with { S.get() -> fn(s) { handle { (resume(s))(s) } with { T.t() -> resume(7) } }  return(x) -> fn(s) { x } }\n\
           } with { T.t() -> resume(10)  return(r) -> r }\n\
           io.println(if f(5) == 12 { \"12\" } else { \"wrong\" })\n\
         }\n",
        "12\n",
    );
}

// ---- soundness: an upcast never hides an effect ----

#[test]
fn a_function_picked_at_run_time_still_carries_its_effects() {
    // `pick` returns either lambda; calling the result may perform L, so main
    // must handle it (the evaluator stops on an unhandled `lg`).
    let err = check_source(
        "t.elya",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn pick(c) { if c { fn(x) { x } } else { fn(x) { lg(x) } } }\n\
         pub fn main() -> Int { let f = pick(False)  f(1) }\n",
    )
    .expect_err("L is unhandled");
    assert!(err.contains("E0420"), "{err}");
}

#[test]
fn an_effectful_argument_does_not_fit_a_pure_parameter() {
    // Parameter rows are contravariant and are never opened: `apply1` calls
    // `k` under no handler, so `k` performing L reaches main.
    let err = check_source(
        "t.elya",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn apply1(k) { k(1) }\n\
         pub fn main() -> Int { apply1(fn(x) { lg(x) }) }\n",
    )
    .expect_err("L is unhandled");
    assert!(err.contains("E0420"), "{err}");
}

// ---- the independent review (2026-10-09) ----------------------------------

#[test]
fn a_call_result_whose_row_later_becomes_a_parameters_is_not_typed_pure() {
    // F1, a hole this slice opened: `g()` calls the result of a group member
    // whose body is not inferred yet, so the call recorded `ambient ⊇ tail`.
    // Later that tail aliased `k`'s row -- a meaningful variable -- and the
    // labels-only flush never relayed it: `f` was typed pure though it calls
    // `k`, and main's unhandled `lg` passed `check`.
    for src in [
        "effect L { fn lg(x: Int) -> Int }\n\
         fn f(n, k) {\n\
           let rl = fn(h) { let z = if True { h } else { k }  h() }\n\
           if n == 0 { k } else {\n\
             let g = f(n - 1, k)\n\
             let r = g()\n\
             fn() { r }\n\
           }\n\
         }\n\
         pub fn main() -> Int {\n\
           let h = f(1, fn() { lg(1) })\n\
           7\n\
         }\n",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn idk(n, k) { if n == 0 { k } else { let _ = f(n - 1, k)  k } }\n\
         fn f(n, k) {\n\
           let rl = fn(h) { let z = if True { h } else { k }  h() }\n\
           let g = idk(n, k)\n\
           g()\n\
         }\n\
         pub fn main() -> Int { f(1, fn() { lg(1) }) }\n",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn idk(n, k) { if n == 0 { k } else { let _ = f(n - 1, k)  k } }\n\
         fn f(n, k) {\n\
           let g = idk(n, k)\n\
           g()\n\
         }\n\
         pub fn main() -> Int { f(1, fn() { lg(1) }) }\n",
    ] {
        let err = check_source("t.elya", src).expect_err("L is unhandled");
        assert!(err.contains("E0420"), "{err}");
    }
}

#[test]
fn a_parameter_reached_through_a_group_members_result_keeps_its_row() {
    // F2, an over-rejection this slice introduced (base accepted it): the
    // recorded tail turned out to be `k`'s row, stayed phantom, and was closed
    // pure at the group's end -- forcing every `k` pure.
    checks_and_prints(
        "effect L { fn lg(x: Int) -> Int }\n\
         fn idk(n, k) { if n == 0 { k } else { let _ = f(n - 1, k)  k } }\n\
         fn f(n, k) {\n\
           let g = idk(n, k)\n\
           g()\n\
         }\n\
         pub fn main() {\n\
           let r = handle { f(1, fn() { lg(5) }) } with { L.lg(x) -> resume(x + 1)  return(r) -> r }\n\
           io.println(if r == 6 { \"6\" } else { \"wrong\" })\n\
         }\n",
        "6\n",
    );
}
