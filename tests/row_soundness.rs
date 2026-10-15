//! HANDOFF step "the soundness sweep the resume-row review found".
//!
//! An effect added to an ambient whose row was ALREADY closed used to vanish:
//! `add_effect`/`add_row` returned the conflict and every caller discarded it,
//! so a function could be typed pure while performing, `check` passed, and the
//! evaluator stopped on "unhandled effect ... reached the machine". Rows unify
//! by EQUALITY (no sub-effecting, spec 3.6), so a row closes early whenever it
//! is joined to a closed one -- a pure lambda's, through recursion, an `if`, or
//! a call. The fix reports the conflict (E0423, "this effect reaches a row that
//! was already closed") instead of dropping the effect.
//!
//! Tried and reverted (the independent review): also leaving a lambda's tail
//! open when it is free in the environment. It accepted the recursive program
//! below under a handler, but merged rows that are only related by inclusion --
//! a handled effect leaked into a recursive function's row, and a forwarding
//! lambda forced its function's own effect onto the forwarded parameter -- two
//! regressions on programs base accepted soundly (guarded below). Accepting
//! both kinds needs row subsumption: a language question (PARKED).
//!
//! Negative controls, each reverted (`cmp` against a saved copy):
//! - C1, conflicts discarded again: the three rejection tests fail (each
//!   checks clean);
//! - C2, the environment rule restored: the three regression guards fail
//!   (E0420 for the handled effect, E0423 for the two forwarding programs),
//!   and both `go` tests change diagnosis (E0420 / accepted: `go` keeps L).
//!   (Since recursion through a lambda, 2026-10-15, both `go` tests expect
//!   exactly that: E0420 and accepted.)

use elya::{check_source, run_source};

fn check_err(src: &str) -> String {
    check_source("t.elya", src).expect_err("should fail to type-check")
}

fn checks_and_prints(src: &str, want: &str) {
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
    assert_eq!(run_source("t.elya", src).expect("runs"), want);
}

/// `go`'s lambda calls `go`, so its row is `go`'s; the lambda's closing step
/// closed it before `lg(n)` was inferred.
const GO: &str = "effect L { fn lg(x: Int) -> Int }\n\
fn go(n) { let f = fn(s) { s + (if n == 0 { 0 } else { go(n - 1) }) }  f(1) + lg(n) }\n";

#[test]
fn an_effect_after_a_lambda_that_calls_its_recursive_function_is_not_dropped() {
    // Before: `go : fn(Int) -> Int`, `check` clean, the evaluator stopped on an
    // unhandled `lg`. Then E0423 ("already closed"): the lambda closed `go`'s
    // row early. Since recursion through a lambda (2026-10-15, spec
    // `2026-10-15-elya-handle-recursion-design.md` §5) the lambda's row waits
    // for the group, `go` gets its real row {L}, and L reaches `main`
    // unhandled: E0420. Not dropped either way (expectation changed by that
    // decision).
    let err = check_err(&format!("{GO}pub fn main() -> Int {{ go(3) }}\n"));
    assert!(err.contains("E0420") && err.contains('L'), "{err}");
}

#[test]
fn an_effect_after_an_if_that_joins_a_pure_lambda_is_not_dropped() {
    // The review's witness with no recursion: the `if` joined `k`'s row to the
    // pure lambda's closed one, and `lg(1)` then reached a closed row (E0423).
    // Since sub-effecting the pure lambda's row is open, so nothing closes
    // early and L simply reaches `main` unhandled: E0420. Not dropped either way.
    let err = check_err(
        "effect L { fn lg(x: Int) -> Int }\n\
         fn w(k) { let a = k(0)  let z = if a == 0 { k } else { fn(x) { x } }  a + z(1) + lg(1) }\n\
         pub fn main() -> Int { w(fn(x) { x }) }\n",
    );
    assert!(err.contains("E0420") && err.contains('L'), "{err}");
}

#[test]
fn the_recursive_program_under_a_handler_is_accepted() {
    // Was `the_recursive_program_under_a_handler_is_rejected_too`: "the cost,
    // recorded: the evaluator runs this (64), but `go`'s row was closed before
    // `lg` reached it". Recursion through a lambda (2026-10-15) defers the
    // lambda's close to its group, so `go : fn(Int) / {L} -> Int` and the
    // program checks and runs (expectation changed by that decision).
    let src = format!(
        "{GO}pub fn main() {{ io.println(if handle {{ go(3) }} with {{ L.lg(x) -> resume(x * 10)  return(r) -> r }} == 64 {{ \"64\" }} else {{ \"wrong\" }}) }}\n"
    );
    checks_and_prints(&src, "64\n");
}

// ---- regression guards: sound programs base accepted, kept accepted ----

#[test]
fn a_recursive_function_may_handle_its_own_effect_through_a_lambda() {
    checks_and_prints(
        "effect L { fn lg(x: Int) -> Int }\n\
         fn go(n) {\n\
           let f = fn(s) { if s == 0 { 0 } else { go(s - 1) } }\n\
           let r = handle { f(n) + lg(n) } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
           r\n\
         }\n\
         pub fn main() { io.println(if go(2) == 3 { \"3\" } else { \"wrong\" }) }\n",
        "3\n",
    );
}

#[test]
fn a_forwarding_lambda_does_not_push_its_functions_effect_onto_the_parameter() {
    checks_and_prints(
        "effect L { fn lg(x: Int) -> Int }\n\
         fn w(k) { let f = fn(s) { k(s) }  f(1) + lg(1) }\n\
         pub fn main() {\n\
           let r = handle { w(fn(x) { x }) } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
           io.println(if r == 2 { \"2\" } else { \"wrong\" })\n\
         }\n",
        "2\n",
    );
}

#[test]
fn a_recursive_map_with_logging_through_a_forwarding_lambda_checks() {
    checks_and_prints(
        "effect L { fn lg(x: Int) -> Int }\n\
         type List(a) { Nil, Cons(a, List(a)) }\n\
         fn each(xs, f) { match xs { Nil -> 0  Cons(h, t) -> { let k = fn(y) { f(y) }  k(h) + lg(h) + each(t, f) } } }\n\
         pub fn main() {\n\
           let r = handle { each(Cons(1, Cons(2, Nil)), fn(x) { x }) } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
           io.println(if r == 6 { \"6\" } else { \"wrong\" })\n\
         }\n",
        "6\n",
    );
}
