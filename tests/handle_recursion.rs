//! Recursion through a handle body (spec
//! `docs/superpowers/specs/2026-10-15-elya-handle-recursion-design.md`, PARKED
//! since slice 5b-10). A recursive call made directly in a handle's body used
//! to UNIFY the function's own row with the body's ambient, which holds the
//! handled effect, so the effect leaked into the function's type and every
//! caller was asked to handle it (E0420, or E0423 against an annotation).
//!
//! Negative control (reverted, `cmp` against a saved copy):
//! `member_in_handle_body` always `None` -- the first two tests fail (E0420,
//! and E0423 for the annotated `nest`), the controls still pass.

use elya::check_source;
use elya::parse::parse_module;
use elya::Session;

const S: &str = "effect S { fn get() -> Int }\neffect T { fn t() -> Int }\n";

fn value(src: &str) -> i64 {
    if let Err(d) = check_source("t.elya", src) {
        panic!("should check clean: {d}\n{src}");
    }
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "{pd:?}");
    match elya::eval::run_module_value(&m).expect("runs").1 {
        elya::eval::Value::Int(n) => n,
        v => panic!("not an Int: {v:?}"),
    }
}

#[test]
fn a_function_recursing_inside_its_own_handle_checks_and_runs() {
    for (body, want) in [
        // n1: annotated with its real row.
        (
            "fn nest(n: Int) / {T} -> Int { if n == 0 { t() } else { handle { nest(n - 1) + get() } with { S.get() -> resume(1)  return(r) -> r } } }\n\
             pub fn main() -> Int { handle { nest(1000) } with { T.t() -> resume(0)  return(r) -> r } }\n",
            1000,
        ),
        // n2: the same, unannotated.
        (
            "fn nest(n) { if n == 0 { t() } else { handle { nest(n - 1) + get() } with { S.get() -> resume(1)  return(r) -> r } } }\n\
             pub fn main() -> Int { handle { nest(1000) } with { T.t() -> resume(0)  return(r) -> r } }\n",
            1000,
        ),
        // n3: pure apart from the handled effect.
        (
            "fn nest(n) { if n == 0 { 0 } else { handle { nest(n - 1) + get() } with { S.get() -> resume(1)  return(r) -> r } } }\n\
             pub fn main() -> Int { nest(1000) }\n",
            1000,
        ),
        // m1: a callback relayed beside the recursion.
        (
            "fn h(f, n) { if n == 0 { 0 } else { handle { f() + h(f, n - 1) + get() } with { S.get() -> resume(1)  return(r) -> r } } }\n\
             pub fn main() -> Int { h(fn() { 2 }, 10) }\n",
            30,
        ),
    ] {
        assert_eq!(value(&format!("{S}{body}")), want, "{body}");
    }
}

#[test]
fn mutual_recursion_through_a_handle_checks_and_runs() {
    // n5: PARKED's split form.
    assert_eq!(
        value(&format!(
            "{S}fn nest(n) {{ if n == 0 {{ 0 }} else {{ wrap(n) }} }}\n\
             fn wrap(n) {{ handle {{ nest(n - 1) + get() }} with {{ S.get() -> resume(2)  return(r) -> r }} }}\n\
             pub fn main() -> Int {{ nest(500) }}\n"
        )),
        1000
    );
}

#[test]
fn an_effect_the_recursion_really_performs_is_still_reported() {
    for body in [
        // n4: `t()` inside the handle is not what the handle handles.
        "fn h(n) { handle { if n == 0 { t() } else { h(n - 1) + get() } } with { S.get() -> resume(1)  return(r) -> r } }\n\
         pub fn main() -> Int { h(5) }\n",
        // n7: `get()` OUTSIDE every handle, at n == 0.
        "fn h(n) { if n == 0 { get() } else { handle { h(n - 1) } with { S.get() -> resume(n)  return(r) -> r + 1 } } }\n\
         pub fn main() -> Int { h(3) }\n",
    ] {
        let err = check_source("t.elya", &format!("{S}{body}"))
            .expect_err("must stay rejected")
            .to_string();
        assert!(err.contains("E0420"), "{err}");
    }
}

#[test]
fn the_reviews_soundness_holes_are_rejected() {
    // F1: the member's `St` arguments were not unified with the handled
    // `St(Bool)` when the label was already present: accepted, the evaluator
    // stopped on a type error, natively a silent 2.
    let f1 = "effect St(s) { fn get() -> s }\n\
              fn f(n) { if n > 0 { handle { let b = get()  if b { f(n - 1) } else { 0 } } with { St.get() -> resume(True)  return(r) -> r } } else { get() + 1 } }\n\
              pub fn main() -> Int { handle { f(2) } with { St.get() -> resume(10)  return(r) -> r } }\n";
    // F2: a resume escaping in a lambda copied the body's labels before the
    // member's `T` was known: accepted, the evaluator met an unhandled `t`.
    let f2 = "effect S { fn get() -> Int }\neffect T { fn t() -> Int }\n\
              type Task { Done(Int), Paused(fn() -> Task) }\n\
              fn f(n) -> Task {\n\
              \x20 if n > 0 { handle { let _ = get()  f(n - 1) } with { S.get() -> Paused(fn() { resume(1) })  return(r) -> r } } else { let _ = t()  Done(0) }\n\
              }\n\
              pub fn main() -> Int {\n\
              \x20 let tk = handle { f(1) } with { T.t() -> resume(0)  return(r) -> r }\n\
              \x20 match tk { Paused(k) -> match k() { Done(x) -> x  Paused(k2) -> 7 }  Done(x) -> x }\n\
              }\n";
    for src in [f1, f2] {
        let err = check_source("t.elya", src)
            .expect_err("must be rejected")
            .to_string();
        assert!(err.contains("E0423"), "{err}");
    }
}

// ---- Recursion through a lambda (spec §5) ----

const SPAWN: &str =
    "effect Async { fn spawn(f: fn() / {Async} -> Int) -> Int  fn await(id: Int) -> Int }\n\
fn fib(n: Int) / {Async} -> Int {\n\
\x20 if n < 2 { n } else {\n\
\x20   let a = spawn(fn() { fib(n - 1) })\n\
\x20   let b = spawn(fn() { fib(n - 2) })\n\
\x20   await(a) + await(b)\n\
\x20 }\n\
}\n";

#[test]
fn a_function_recursing_inside_a_lambda_checks_and_runs() {
    for (src, want) in [
        // The row-soundness `go` under its handler (was E0423).
        (
            "effect L { fn lg(x: Int) -> Int }\n\
             fn go(n) { let f = fn(s) { s + (if n == 0 { 0 } else { go(n - 1) }) }  f(1) + lg(n) }\n\
             pub fn main() -> Int { handle { go(3) } with { L.lg(x) -> resume(x * 10)  return(r) -> r } }\n"
                .to_string(),
            64,
        ),
        // A task that spawns itself: the lambda closed `fib`'s row before
        // `spawn` reached it (E0423). Answered by a handler that runs each
        // child to completion on the spot (ids are the answers).
        (
            format!(
                "{SPAWN}fn eager(f: fn() / {{Async}} -> Int) -> Int {{\n\
                 \x20 handle {{ f() }} with {{ Async.spawn(g) -> resume(eager(g))  Async.await(v) -> resume(v)  return(r) -> r }}\n\
                 }}\n\
                 pub fn main() -> Int {{ eager(fn() {{ fib(15) }}) }}\n"
            ),
            610,
        ),
        // A handle around a lambda that recurses: the handled S stays inside.
        (
            "effect S { fn get() -> Int }\n\
             fn h(n) { handle { let f = fn() { h(n - 1) }  if n == 0 { 0 } else { f() + get() } } with { S.get() -> resume(2)  return(r) -> r } }\n\
             pub fn main() -> Int { h(50) }\n"
                .to_string(),
            100,
        ),
    ] {
        assert_eq!(value(&src), want, "{src}");
    }
}

#[test]
fn a_member_that_becomes_a_relay_later_reaches_the_including_row() {
    // The lambda review's F1: `f`'s row is still a fresh variable when `g`
    // includes it, and becomes a relay of `k` only at `k(n)`; the deferred `g`
    // closed pure, escaped, and `lg` met no handler (check was clean). F1b: the
    // same through an escaped `resume` (accepted at 9eb8d5f too).
    let f1 = "effect L { fn lg(x: Int) -> Int }\n\
              fn f(k, n) {\n\
              \x20 let g = fn() { let h = f(k, n - 1)  0 }\n\
              \x20 let z = k(n)\n\
              \x20 g\n\
              }\n\
              pub fn main() -> Int {\n\
              \x20 let g = handle { f(fn(x) { lg(x) }, 3) } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
              \x20 g()\n\
              }\n";
    let f1b = "effect S { fn get() -> Int }\neffect L { fn lg(x: Int) -> Int }\n\
               fn f(k, n) {\n\
               \x20 let r = handle { let x = get()  x + (if n == 0 { 0 } else { let q = f(k, n - 1)  0 }) } with { S.get() -> fn() { let r2 = resume(1)  r2() }  return(r) -> fn() { r } }\n\
               \x20 let z = k(n)\n\
               \x20 r\n\
               }\n\
               pub fn main() -> Int { let r = handle { f(fn(x) { lg(x) }, 2) } with { L.lg(x) -> resume(x)  return(r) -> r }  r() }\n";
    for (src, code) in [(f1, "E0420"), (f1b, "E0423")] {
        let err = check_source("t.elya", src)
            .expect_err("must be rejected")
            .to_string();
        assert!(err.contains(code), "{err}");
    }
}
