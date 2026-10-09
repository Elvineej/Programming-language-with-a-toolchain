//! HANDOFF step "`resume` carries its handle's row" (found by slice 5b-10, PARKED).
//!
//! Under deep handlers `resume(v)` runs the rest of the handled body, which may
//! still perform the effects the handle does NOT handle. Inference gave `resume`
//! no effects at all ("its latent effect is the clause's ambient, already
//! threaded") -- true for a resume directly in a clause, false for a resume
//! inside a lambda: the lambda was typed pure, and calling it after its handlers
//! returned performed an effect nothing handles. The evaluator then stopped with
//! "internal: unhandled effect `t` reached the machine" on a program `check`
//! called clean.
//!
//! Negative controls, each reverted (`cmp` against a saved copy), each
//! failing differently:
//! - C1, no row added at the resume (the pre-fix inference): the escape checks
//!   clean, m10 checks clean, and the T-carrying state-passing program is E0423
//!   the other way round;
//! - C2, the HANDLED effect kept in resume's row: six tests -- every program
//!   whose resumed handler handles its own effect again;
//! - C3, the first version (the row added WITH its open tail): the four review
//!   tests below -- a new hole and three over-rejections.
//!
//! Known gaps (PARKED): resume's row is the handled BODY's; effects of the
//! return clause and of clauses re-entered by the resumed body are not in it,
//! nor effects the body only relays through an open row.

use elya::{check_source, run_source, warnings};

fn check_err(src: &str) -> String {
    check_source("t.elya", src).expect_err("should fail to type-check")
}

/// The PARKED program: the lambda escapes T's handler and is called in `main`.
const ESCAPES_T: &str = "effect S { fn get() -> Int }\n\
effect T { fn t() -> Int }\n\
pub fn main() -> Int {\n\
  let f = handle {\n\
    handle { get() + t() } with { S.get() -> fn(s) { (resume(s))(s) }  return(x) -> fn(s) { x } }\n\
  } with { T.t() -> resume(10)  return(r) -> r }\n\
  f(5)\n\
}\n";

#[test]
fn an_escaped_resume_that_performs_an_unhandled_effect_is_e0420() {
    let err = check_err(ESCAPES_T);
    assert!(err.contains("E0420"), "expected E0420: {err}");
    assert!(err.contains('T'), "should name T: {err}");
}

#[test]
fn a_resuming_lambda_beside_a_pure_return_value_checks_by_sub_effecting() {
    // m10 of the 5b-10 spec, called where T IS handled. The resuming lambda is
    // `fn(Int) / {T} -> Int` and the return clause's `fn(s) { x }` is pure.
    // Without sub-effecting the two clause values did not unify (E0423, this
    // fix's strictness cost when it shipped); with it (2026-10-08) the pure
    // one stands where `{T}` is expected, and the program runs (15).
    // Approved expected-value change.
    let src = "effect S { fn get() -> Int }\n\
               effect T { fn t() -> Int }\n\
               fn g() -> Int {\n\
                 let f = handle { get() + t() } with {\n\
                   S.get() -> fn(s) { (resume(s))(s) }\n\
                   return(x) -> fn(s) { x }\n\
                 }\n\
                 f(5)\n\
               }\n\
               pub fn main() {\n\
                 let r = handle { g() } with { T.t() -> resume(10)  return(r) -> r }\n\
                 io.println(if r == 15 { \"15\" } else { \"wrong\" })\n\
               }\n";
    checks_and_prints(src, "15\n");
}

#[test]
fn a_resuming_lambda_called_inside_the_handler_checks_and_runs() {
    // The same state-passing shape with both clause values carrying T.
    let src = "effect S { fn get() -> Int }\n\
               effect T { fn t() -> Int }\n\
               fn g() -> Int {\n\
                 let f = handle { get() + t() } with {\n\
                   S.get() -> fn(s) { (resume(s))(s) }\n\
                   return(x) -> fn(s) { x + 0 * t() }\n\
                 }\n\
                 f(5)\n\
               }\n\
               pub fn main() {\n\
                 let r = handle { g() } with { T.t() -> resume(10)  return(r) -> r }\n\
                 io.println(if r == 15 { \"15\" } else { \"wrong\" })\n\
               }\n";
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
    assert_eq!(run_source("t.elya", src).expect("runs"), "15\n");
}

#[test]
fn the_handled_effect_itself_is_not_in_resumes_row() {
    // The state-passing idiom: the lambda resumes S's own continuation; S is
    // handled by the resumed handler (deep), so calling the lambda outside any
    // S handler is fine.
    let src = "effect S { fn get() -> Int }\n\
               pub fn main() {\n\
                 let f = handle { get() + get() } with {\n\
                   S.get() -> fn(s) { (resume(s))(s + 1) }\n\
                   return(x) -> fn(s) { x }\n\
                 }\n\
                 io.println(if f(1) == 3 { \"3\" } else { \"wrong\" })\n\
               }\n";
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
    assert_eq!(run_source("t.elya", src).expect("runs"), "3\n");
}

// ---- the review of the first version of this fix ----------------------------
//
// The first version added the body's row at each resume site WITH its open
// tail, and `add_row` unifies tails: the resume site's ambient became EQUAL to
// the handled body's tail, merging rows that are only related by inclusion.

fn checks_and_prints(src: &str, want: &str) {
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
    assert_eq!(run_source("t.elya", src).expect("runs"), want);
}

#[test]
fn a_direct_resume_does_not_tie_its_ambient_to_a_closing_lambda() {
    // A NEW hole the first version opened: the direct `resume(0)` merged main's
    // ambient with the body's tail, the lambda then closed that tail, and the
    // later `lg()` was dropped -- `check` clean, the evaluator stopped on an
    // unhandled `lg`. Before any of this it was E0420, as it must be.
    let src = "effect S { fn get() -> Int }\n\
               effect L { fn lg() -> Int }\n\
               pub fn main() {\n\
                 let f = handle { get() } with { S.get() -> if True { resume(0) } else { fn(s) { (resume(s))(s) } }  return(x) -> fn(s) { x } }\n\
                 let n = lg()\n\
                 io.println(\"x\")\n\
               }\n";
    let err = check_err(src);
    assert!(err.contains("E0420"), "expected E0420: {err}");
}

#[test]
fn a_function_whose_handle_resumes_directly_keeps_its_own_effects() {
    // The same merge, in a function: `g` was typed pure though it performs L,
    // and natively the build failed ("calling convention disagrees").
    checks_and_prints(
        "effect S { fn get() -> Int }\n\
         effect L { fn lg(x: Int) -> Int }\n\
         fn g(c) {\n\
           let f = handle { get() } with { S.get() -> if c { resume(0) } else { fn(s) { (resume(s))(s) } }  return(x) -> fn(s) { x } }\n\
           f(7) + lg(1)\n\
         }\n\
         pub fn main() {\n\
           let r = handle { g(False) } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
           io.println(if r == 8 { \"8\" } else { \"wrong\" })\n\
         }\n",
        "8\n",
    );
}

#[test]
fn a_resuming_lambdas_own_effects_do_not_leak_into_the_handle() {
    // Over-rejection by the first version: the lambda's `lg` reached the
    // handle's residual, so building `f` needed an L handler (E0420).
    checks_and_prints(
        "effect S { fn get() -> Int }\n\
         effect L { fn lg(x: Int) -> Int }\n\
         pub fn main() {\n\
           let f = handle { get() } with { S.get() -> fn(s) { lg(s) + (resume(s))(s) }  return(x) -> fn(s) { lg(x) } }\n\
           let r = handle { f(2) } with { L.lg(x) -> resume(x * 10)  return(r) -> r }\n\
           io.println(if r == 40 { \"40\" } else { \"wrong\" })\n\
         }\n",
        "40\n",
    );
}

#[test]
fn a_direct_resume_does_not_pour_the_enclosing_effects_into_a_sibling_clause() {
    // Over-rejection by the first version, dependent on clause order: the
    // direct `resume(Unit)` tied main's {IO} to the body's tail, so the
    // sibling clause's lambda was `fn(Int) / {IO} -> Int` (E0423).
    checks_and_prints(
        "effect S { fn get() -> Int  fn tick() -> Unit }\n\
         pub fn main() {\n\
           io.println(\"a\")\n\
           let f = handle { tick()  get() } with { S.tick() -> resume(Unit)  S.get() -> fn(s) { (resume(s))(s) }  return(x) -> fn(s) { x } }\n\
           io.println(if f(7) == 7 { \"7\" } else { \"wrong\" })\n\
         }\n",
        "a\n7\n",
    );
}

#[test]
fn a_multi_shot_direct_resume_does_not_borrow_the_enclosing_io() {
    // Spurious E0426 from the first version: the continuation performs no IO;
    // the warning came from the enclosing function's merged ambient.
    let src = "effect multi Flip { fn flip() -> Bool }\n\
               fn count() {\n\
                 io.println(\"before\")\n\
                 handle { if flip() { 1 } else { 2 } } with multi { Flip.flip() -> resume(True) + resume(False) }\n\
               }\n\
               pub fn main() {\n\
                 let n = count()\n\
                 io.println(if n == 3 { \"3\" } else { \"wrong\" })\n\
               }\n";
    checks_and_prints(src, "before\n3\n");
    assert!(
        warnings("t.elya", src).is_none(),
        "{:?}",
        warnings("t.elya", src)
    );
}
