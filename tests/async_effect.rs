//! Async without function colouring (2026-10-09; ROADMAP priority 1). The
//! front-end half: a scheduler written as an Elya handler needs function types
//! in declarations (a queue of continuations, `fork(task: fn() ..)`) and a
//! lambda whose ambient waits for `resume`'s result type before closing.
//! `examples/04_async.elya` is the full program (snapshot in `examples.rs`).
//!
//! Negative controls, each reverted (`cmp` against a saved copy):
//! - D1, deferred closing disabled: the scheduler shapes are E0423 ("this
//!   effect reaches a row that was already closed");
//! - D2, a deferred lambda closed without the members' parameters: the relay
//!   guard is E0423 (its parameter forced pure);
//! - F1, the deferred value row is the ambient itself again (review6): the six
//!   over-rejection tests and the growing-row guard fail;
//! - F2, `instantiate` does not copy the inclusion to an instance: only the
//!   pure-field soundness guard fails (the program is accepted);
//! - F3, a grown value row's tail is made meaningful: only the growing-row
//!   guard fails (E0423).

use elya::{check_source, run_source};

fn checks_and_prints(src: &str, want: &str) {
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
    assert_eq!(run_source("t.elya", src).expect("runs"), want);
}

const QUEUE: &str =
    "effect Async { fn yield_() -> Unit  fn fork(t: fn() / {Async, IO} -> Unit) -> Unit }\n\
type Q { Empty, More(fn(Q) / {IO} -> Unit, Q) }\n\
fn append(q, k) { match q { Empty -> More(k, Empty)  More(h, t) -> More(h, append(t, k)) } }\n\
fn next(q) { match q { Empty -> Unit  More(k, rest) -> k(rest) } }\n";

#[test]
fn a_yield_only_scheduler_checks_and_interleaves() {
    // The lambda `fn(q2) { (resume(Unit))(q2) }` calls `resume`'s result,
    // whose type is only known once the clause's value is: closing its row at
    // its end made the clause's IO a closed-row conflict.
    checks_and_prints(
        &format!(
            "{QUEUE}fn run(t) {{\n\
               handle {{ t() }} with {{\n\
                 Async.yield_() -> fn(q) {{ next(append(q, fn(q2) {{ (resume(Unit))(q2) }})) }}\n\
                 Async.fork(c) -> fn(q) {{ (resume(Unit))(append(q, fn(q2) {{ (run(c))(q2) }})) }}\n\
                 return(x) -> fn(q) {{ next(q) }}\n\
               }}\n\
             }}\n\
             pub fn main() {{\n\
               let p = run(fn() {{\n\
                 fork(fn() {{ io.println(\"a1\")  yield_()  io.println(\"a2\") }})\n\
                 io.println(\"m1\")  yield_()  io.println(\"m2\")\n\
               }})\n\
               p(Empty)\n\
             }}\n"
        ),
        "m1\na1\nm2\na2\n",
    );
}

#[test]
fn a_function_typed_field_holds_effectful_and_pure_closures() {
    checks_and_prints(
        &format!(
            "{QUEUE}pub fn main() {{\n\
               let q = More(fn(q) {{ io.println(\"x\") }}, More(fn(q) {{ Unit }}, Empty))\n\
               next(q)\n\
             }}\n"
        ),
        "x\n",
    );
}

#[test]
fn a_deferred_lambda_keeps_a_relayed_parameter_open() {
    // Regression guard (review2's a1, accepted since sub-effecting): the
    // resuming lambda's ambient is deferred; closing it with only its own
    // parameters closed `k`'s row and forced every `k` pure.
    checks_and_prints(
        "effect S { fn get() -> Int }\n\
         effect L { fn lg(x: Int) -> Int }\n\
         fn h(k) {\n\
           let f = handle { k() + get() } with { S.get() -> fn(s) { (resume(s))(s) }  return(x) -> fn(s) { x } }\n\
           f(1)\n\
         }\n\
         pub fn main() {\n\
           let r = handle { h(fn() { lg(3) }) } with { L.lg(x) -> resume(x * 100)  return(r) -> r }\n\
           io.println(if r == 301 { \"301\" } else { \"wrong\" })\n\
         }\n",
        "301\n",
    );
}

/// A deferred lambda's VALUE row only includes its ambient (review6's third
/// finding). It was the ambient itself, so every later use joined it by
/// equality: a context's effects leaked into the lambda's own row and on to
/// its other uses, and a let-bound one lost its generalization. Each program
/// checked clean before deferral, and the evaluator's value is the one shown.
fn checks_with_value(src: &str, want: i64) {
    let wrapped = format!(
        "{src}pub fn main() {{\n  let v = go()\n  io.println(if v == {want} {{ \"ok\" }} else {{ \"wrong\" }})\n}}\n"
    );
    checks_and_prints(&wrapped, "ok\n");
}

#[test]
fn a_lambda_calling_a_let_bound_lambda_is_passed_to_a_handled_helper() {
    // q04: `with_t(q)` instantiated q's row at `{T}`; `q(1)` then performed
    // `T` in main (E0420).
    checks_with_value(
        "effect T { fn t() -> Int }\n\
         fn with_t(f) { handle { f(1) + t() } with { T.t() -> resume(1)  return(x) -> x } }\n\
         fn go() -> Int {\n\
           let p = fn(x) { x + 1 }\n\
           let q = fn(y) { p(y) }\n\
           with_t(q) + q(1)\n\
         }\n",
        5,
    );
}

#[test]
fn a_lambda_calling_a_let_bound_lambda_is_joined_with_an_effectful_one() {
    // q01: the `if` joined q's row with `{T}`.
    checks_with_value(
        "effect T { fn t() -> Int }\n\
         fn go() -> Int {\n\
           let p = fn(x) { x + 1 }\n\
           let q = fn(y) { p(y) }\n\
           let r = if True { q } else { fn(y) { t() } }\n\
           q(1)\n\
         }\n",
        2,
    );
}

#[test]
fn a_lambda_calling_a_pattern_bound_closure_is_joined_with_an_effectful_one() {
    // q03: the callee is a closure taken out of `B(p)`.
    checks_with_value(
        "effect T { fn t() -> Int }\n\
         type Box(a) { B(a) }\n\
         fn go() -> Int {\n\
           let b = B(fn(x) { x + 1 })\n\
           let q = fn(y) { match b { B(p) -> p(y) } }\n\
           let r = if True { q } else { fn(y) { t() } }\n\
           q(1)\n\
         }\n",
        2,
    );
}

#[test]
fn a_deferred_lambda_used_under_two_different_handlers_keeps_both_out_of_its_row() {
    // q07: S and T both leaked into q's row.
    checks_with_value(
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         fn app(f) { f(1) }\n\
         fn go() -> Int {\n\
           let p = fn(x) { x + 1 }\n\
           let q = fn(y) { p(y) }\n\
           let a = handle { app(fn(z) { q(z) + get() }) } with { S.get() -> resume(10)  return(r) -> r }\n\
           let b = handle { app(fn(z) { q(z) + t() }) } with { T.t() -> resume(100)  return(r) -> r }\n\
           a + b\n\
         }\n",
        114,
    );
}

#[test]
fn a_lambda_calling_a_recursive_members_result_is_upcast_without_changing_it() {
    // h01: `mk` became `fn(Int) -> fn() / {T} -> Int`.
    checks_with_value(
        "effect T { fn t() -> Int }\n\
         fn mk(n) {\n\
           if n == 0 { fn() { 0 } } else {\n\
             let r = mk(n - 1)\n\
             let f = fn() { r() + 1 }\n\
             let g = if n == 100 { f } else { fn() { t() } }\n\
             f\n\
           }\n\
         }\n\
         fn go() -> Int { let k = mk(3)  k() }\n",
        3,
    );
}

#[test]
fn a_state_handlers_result_is_called_inside_a_handled_lambda_and_outside() {
    // h03c: the clause lambda `fn(s) { (resume(s))(s + 1) }` is deferred;
    // calling the handle's result inside a T-handled lambda put `T` into it.
    checks_with_value(
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         fn go() -> Int {\n\
           let f = handle { get() + get() } with { S.get() -> fn(s) { (resume(s))(s + 1) }  return(x) -> fn(s) { x * 100 + s } }\n\
           let a = handle { let g = fn() { f(1) + t() }  if False { g() } else { 5 } } with { T.t() -> resume(1000)  return(r) -> r }\n\
           a + f(1)\n\
         }\n",
        308,
    );
}

#[test]
fn a_let_bound_deferred_lambda_in_a_pure_field_is_still_rejected() {
    // Soundness guard for the fix above: a let-bound deferred lambda is
    // generalized now, so each USE gets its own value row -- which must still
    // include the ambient, labels arriving later included. Here `k`'s use is a
    // pure field, and `k` turns out to perform `T` once the return clause is
    // typed: a conflict, never a pure closure that performs.
    let src = "effect S { fn get() -> Int }\n\
               effect T { fn t() -> Int }\n\
               type Box { B(fn() -> Int) }\n\
               fn h() {\n\
                 handle { get() + 1 } with {\n\
                   S.get() -> fn(q) { let k = fn() { (resume(1))(q) }  match B(k) { B(g) -> g() } }\n\
                   return(x) -> fn(q) { t() + x + q }\n\
                 }\n\
               }\n\
               pub fn main() -> Int { handle { let f = h()  f(10) } with { T.t() -> resume(5)  return(r) -> r } }\n";
    let err = check_source("t.elya", src).expect_err("a performing closure in a pure field");
    assert!(format!("{err:?}").contains("E0423"), "{err:?}");
}

#[test]
fn a_deferred_value_row_that_grows_later_stays_an_upcast_tail() {
    // `p` is monomorphic (an `if`, not a syntactic value), so joining it with
    // a `V` lambda AFTER the clause lambda called it gives that lambda's
    // ambient -- deferred, it calls `resume`'s result -- a label its value row
    // does not have yet: the flush adds it. What remains of the value row's
    // tail must stay phantom: made meaningful, the direct call `f(1)` tied
    // `go`'s ambient to it, `u()` put `U` into `f`, and `apply_v(f)` was a
    // conflict (E0423; base rejected this program the same way).
    checks_and_prints(
        "effect S { fn get() -> Int }\n\
         effect U { fn u() -> Int }\n\
         effect V { fn v() -> Int }\n\
         fn apply_v(k: fn(Int) / {V} -> Int) / {V} -> Int { k(1) }\n\
         fn go(c) -> Int {\n\
           let p = if c { fn(x) { x } } else { fn(x) { x + 0 } }\n\
           let f = handle { get() } with { S.get() -> fn(s) { (resume(s))(s) + p(s) }  return(x) -> fn(s) { x + s } }\n\
           let z = if c { p } else { fn(x) { v() } }\n\
           let w = fn() { 0 }\n\
           let d = if False { f(1) } else { 0 }\n\
           let b = u()\n\
           b + d + apply_v(f)\n\
         }\n\
         pub fn main() {\n\
           let r = handle { handle { go(True) } with { V.v() -> resume(7)  return(r) -> r } } with { U.u() -> resume(30)  return(r) -> r }\n\
           io.println(if r == 33 { \"ok\" } else { \"wrong\" })\n\
         }\n",
        "ok\n",
    );
}
