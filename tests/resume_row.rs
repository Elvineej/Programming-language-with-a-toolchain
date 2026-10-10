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

// ---- relayed and return-clause effects (2026-10-09, auto-run) -------------
//
// Re-found by the async-step-1 review: `resume`'s row took the handled
// body's LABELS only, so an effect the body RELAYS through a parameter's row
// (`task(body)` calling `body()`) and the RETURN clause's effects were not in
// the row of a lambda that resumes. Such a lambda, stored and called after
// the outer handler returned, performed an effect nothing handled: `check`
// clean, the evaluator stopped on "unhandled effect", natively the runtime's
// "no clause" guard.

const RELAY_HEAD: &str = "effect Y { fn yld() -> Unit }\n\
effect L { fn lg() -> Int }\n\
effect Z { fn z() -> Int }\n";

#[test]
fn a_resume_carries_effects_the_body_relays_through_a_parameter() {
    for (tag, field_row, run) in [
        ("pure field", "", "match t { D(x) -> x  K(k) -> match k() { D(x) -> x  K(j) -> 0 } }"),
        (
            "field admits Z",
            " / {Z}",
            "handle { match t { D(x) -> x  K(k) -> match k() { D(x) -> x  K(j) -> 0 } } } with { Z.z() -> resume(1)  return(r) -> r }",
        ),
    ] {
        let src = format!(
            "{RELAY_HEAD}type T {{ D(Int), K(fn(){field_row} -> T) }}\n\
             fn task(body) {{ handle {{ body() }} with {{ Y.yld() -> K(fn() {{ resume(Unit) }})  return(x) -> D(x) }} }}\n\
             pub fn main() -> Int {{\n\
             \x20 let t = handle {{ task(fn() {{ let _ = yld()  lg() }}) }} with {{ L.lg() -> resume(1)  return(r) -> r }}\n\
             \x20 {run}\n\
             }}\n"
        );
        let err = check_err(&src);
        assert!(err.contains("E0423") || err.contains("E0420"), "{tag}: {err}");
    }
}

#[test]
fn a_resume_carries_the_return_clauses_effects() {
    let err = check_err(&format!(
        "{RELAY_HEAD}type T {{ D(Int), K(fn() / {{Z}} -> T) }}\n\
         fn sp() -> T {{ handle {{ let _ = yld()  5 }} with {{ Y.yld() -> K(fn() {{ resume(Unit) }})  return(x) -> D(x + lg()) }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let t = handle {{ sp() }} with {{ L.lg() -> resume(1)  return(r) -> r }}\n\
         \x20 handle {{ match t {{ D(x) -> x  K(k) -> match k() {{ D(x) -> x  K(j) -> 0 }} }} }} with {{ Z.z() -> resume(1)  return(r) -> r }}\n\
         }}\n"
    ));
    assert!(err.contains("E0423") || err.contains("E0420"), "{err}");
}

/// The same relay, the resuming lambda returned as the handle's value: its
/// row names the body's relay, so calling it where L is not handled is E0420.
const RELAY_RETURNED: &str = "fn task(body) { handle { body() } with { Y.yld() -> fn() { let a = z()  let r = resume(Unit)  r() + a }  return(x) -> fn() { x } } }\n";

#[test]
fn a_returned_resuming_lambda_carries_the_relay() {
    let err = check_err(&format!(
        "{RELAY_HEAD}{RELAY_RETURNED}\
         pub fn main() -> Int {{\n\
         \x20 let k = handle {{ task(fn() {{ let _ = yld()  lg() }}) }} with {{ L.lg() -> resume(1)  return(r) -> r }}\n\
         \x20 handle {{ k() }} with {{ Z.z() -> resume(1)  return(r) -> r }}\n\
         }}\n"
    ));
    assert!(err.contains("E0420"), "{err}");
}

#[test]
fn a_relayed_resume_called_under_its_handlers_checks_and_runs() {
    // The control: the same lambda called while L is still handled. Rows
    // unify by equality, so the lambda's own `z` reaches the relayed row of
    // `body` and with it `task`'s row: Z is handled around the whole run.
    checks_and_prints(
        &format!(
            "{RELAY_HEAD}{RELAY_RETURNED}\
             pub fn main() {{\n\
             \x20 let v = handle {{ handle {{\n\
             \x20   let k = task(fn() {{ let _ = yld()  lg() }})\n\
             \x20   k()\n\
             \x20 }} with {{ Z.z() -> resume(1)  return(r) -> r }} }} with {{ L.lg() -> resume(1)  return(r) -> r }}\n\
             \x20 io.println(if v == 2 {{ \"2\" }} else {{ \"wrong\" }})\n\
             }}\n"
        ),
        "2\n",
    );
}

/// The resumed body re-enters the SAME clause at its second `get()`, and the
/// clause performs T before building the lambda: the lambda's row must hold
/// every clause's effects, which are known only after all clauses are typed.
const REENTERS: &str = "effect S { fn get() -> Int }\n\
effect T { fn t() -> Int }\n\
fn inner() { handle { get() + get() } with { S.get() -> { let y = t()  fn(s) { (resume(s + y))(s) } }  return(x) -> fn(s) { x } } }\n";

#[test]
fn a_resume_carries_the_effects_of_clauses_it_re_enters() {
    let err = check_err(&format!(
        "{REENTERS}pub fn main() -> Int {{\n\
         \x20 let f = handle {{ inner() }} with {{ T.t() -> resume(10)  return(r) -> r }}\n\
         \x20 f(5)\n\
         }}\n"
    ));
    assert!(err.contains("E0420"), "{err}");
}

#[test]
fn a_re_entering_resume_called_under_its_handler_checks_and_runs() {
    // (5 + 10) + (5 + 10) = 30: both gets, each clause run adds y = 10.
    checks_and_prints(
        &format!(
            "{REENTERS}pub fn main() {{\n\
             \x20 let v = handle {{ let f = inner()  f(5) }} with {{ T.t() -> resume(10)  return(r) -> r }}\n\
             \x20 io.println(if v == 30 {{ \"30\" }} else {{ \"wrong\" }})\n\
             }}\n"
        ),
        "30\n",
    );
}

// ---- the independent review of the re-entered-clause half -----------------
//
// The first version kept a resuming lambda's row open until its handle was
// done, keyed on the lambda: a `let` generalized that open row first (its
// later labels never reached the uses), and a resume inside a nested handle
// inside the lambda was recorded on the nested handle's ambient, so the lambda
// closed early. Both checked clean and stopped on "unhandled effect `lg`".
// Now an escaped resume's ambient takes the handle's CLAUSES ROW as a tail,
// which is neither closed nor generalized until the handle is done.

const RC_HEAD: &str = "effect Y { fn yld() -> Unit  fn get() -> Int }\n\
effect L { fn lg() -> Int }\n\
effect Z { fn z() -> Int }\n\
type T { D(Int), K(fn() -> T) }\n";

fn rc_program(yld_clause: &str) -> String {
    format!(
        "{RC_HEAD}fn sp() -> T {{ handle {{ let _ = yld()  get() }} with {{ {yld_clause}  Y.get() -> resume(lg())  return(x) -> D(x) }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let t = handle {{ sp() }} with {{ L.lg() -> resume(4)  return(r) -> r }}\n\
         \x20 match t {{ D(x) -> x  K(k) -> match k() {{ D(x) -> x  K(j) -> 0 }} }}\n\
         }}\n"
    )
}

#[test]
fn a_let_bound_resuming_lambda_carries_the_clauses_it_re_enters() {
    let err = check_err(&rc_program(
        "Y.yld() -> { let k = fn() { resume(Unit) }  K(k) }",
    ));
    assert!(err.contains("E0423"), "{err}");
}

#[test]
fn a_resume_in_a_nested_handle_inside_a_lambda_carries_the_clauses_it_re_enters() {
    for clause in [
        "Y.yld() -> K(fn() { handle { resume(Unit) } with { Z.z() -> resume(0)  return(r) -> r } })",
        "Y.yld() -> K(fn() { handle { 0 } with { Z.z() -> resume(0)  return(r) -> resume(Unit) } })",
    ] {
        let err = check_err(&rc_program(clause));
        assert!(err.contains("E0423"), "{clause}: {err}");
    }
}

#[test]
fn a_local_lambda_forwarding_a_parameter_shares_its_row_known_limitation() {
    // Rows unify by equality: `k` relays `f`'s row, so `k`'s uses under two
    // handlers put L and Z into `f`'s row, and L reaches `g` (E0420). The same
    // program calling `f` directly instead of `k` was already rejected this way;
    // before 2026-10-09 `k` was closed at the lambda (forcing `f` pure), which
    // accepted this and rejected `wrap(f)` used at an effect.
    let err = check_err(
        "effect L { fn lg(x: Int) -> Int }\n\
         effect Z { fn z() -> Int }\n\
         fn g(f) { let k = fn(x) { f(x) }  let a = handle { k(1) + lg(1) } with { L.lg(x) -> resume(100)  return(r) -> r }  let b = handle { k(2) + z() } with { Z.z() -> resume(10)  return(r) -> r }  a + b }\n\
         pub fn main() -> Int { g(fn(x) { x }) }\n",
    );
    assert!(err.contains("E0420"), "{err}");
}
