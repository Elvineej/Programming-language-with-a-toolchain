//! Function types in type and effect declarations (async step 1, 2026-10-09;
//! spec `docs/superpowers/specs/2026-10-09-elya-async-step1-design.md`).
//!
//! A constructor field or an operation parameter or result may be a function
//! type `fn(A) / {E} -> R`. A declaration has no row variables, so its rows are
//! exactly what is written: no `/ {..}` is the EMPTY row, not "any effects" as
//! in a function's own annotations. A row may name `IO` and any declared
//! effect without type parameters, including the effect being declared.
//!
//! Negative controls, each reverted (`cmp` against a saved copy):
//! - C1, the old refusal back (E0432 "not supported yet"): every acceptance
//!   test fails;
//! - C2, an unwritten declaration row open (a fresh row variable) instead of
//!   empty: `an_unwritten_row_in_a_declaration_is_pure` checks clean;
//! - C3, the declared-effect set built AFTER the type declarations: the
//!   rows naming an effect fail with E0432 "unknown effect".

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

#[test]
fn a_function_typed_field_stores_and_calls_a_closure() {
    checks_and_prints(
        "type B { B(fn(Int) -> Int) }\n\
         pub fn main() {\n\
           let f = fn(x) { x + 1 }\n\
           let r = match B(f) { B(g) -> g(41) }\n\
           io.println(if r == 42 { \"42\" } else { \"wrong\" })\n\
         }\n",
        "42\n",
    );
}

#[test]
fn an_unwritten_row_in_a_declaration_is_pure() {
    let err = check_err(
        "effect L { fn lg(x: Int) -> Int }\n\
         type B { B(fn(Int) -> Int) }\n\
         fn mk() { B(fn(x) { lg(x) }) }\n\
         pub fn main() -> Int { 0 }\n",
    );
    assert!(err.contains("E0423"), "{err}");
}

#[test]
fn a_written_row_admits_that_effect_and_a_pure_closure_fits_it() {
    checks_and_prints(
        "effect L { fn lg(x: Int) -> Int }\n\
         type B { B(fn(Int) / {L} -> Int) }\n\
         fn call(b) { match b { B(g) -> g(5) } }\n\
         pub fn main() {\n\
           let r = handle { call(B(fn(x) { lg(x) + 1 })) + call(B(fn(x) { x })) } with { L.lg(x) -> resume(x * 100)  return(r) -> r }\n\
           io.println(if r == 506 { \"506\" } else { \"wrong\" })\n\
         }\n",
        "506\n",
    );
}

#[test]
fn an_unknown_effect_in_a_declaration_row_is_e0432() {
    let err = check_err("type B { B(fn(Int) / {Nope} -> Int) }\npub fn main() -> Int { 0 }\n");
    assert!(
        err.contains("E0432") && err.contains("unknown effect `Nope`"),
        "{err}"
    );
}

#[test]
fn a_parameterized_effect_in_a_declaration_row_is_named() {
    let err = check_err(
        "effect State(s) { fn get() -> s }\n\
         type B { B(fn(Int) / {State} -> Int) }\n\
         pub fn main() -> Int { 0 }\n",
    );
    assert!(
        err.contains("E0432") && err.contains("takes type arguments"),
        "{err}"
    );
}

#[test]
fn an_operation_takes_a_function() {
    checks_and_prints(
        "effect Ap { fn ap(f: fn(Int) -> Int, x: Int) -> Int }\n\
         pub fn main() {\n\
           let r = handle { ap(fn(x) { x * 2 }, 20) + 2 } with { Ap.ap(f, x) -> resume(f(x))  return(r) -> r }\n\
           io.println(if r == 42 { \"42\" } else { \"wrong\" })\n\
         }\n",
        "42\n",
    );
}

#[test]
fn an_effect_names_itself_and_a_later_effect_in_an_operation() {
    // `fork`'s parameter performs `Async` (the effect being declared) and
    // `Log` (declared after it). A clause runs outside its own handler, so
    // the forked body is run under a handler of its own.
    checks_and_prints(
        "effect Async { fn fork(f: fn() / {Async, Log} -> Unit) -> Unit }\n\
         effect Log { fn say(s: String) -> Unit }\n\
         pub fn main() {\n\
           handle {\n\
             handle { fork(fn() { say(\"child\") }) } with {\n\
               Async.fork(f) -> { handle { f() } with { Async.fork(g) -> resume(Unit)  return(u) -> u }  resume(Unit) }\n\
               return(u) -> u\n\
             }\n\
           } with { Log.say(s) -> { io.println(s)  resume(Unit) }  return(u) -> u }\n\
         }\n",
        "child\n",
    );
}

#[test]
fn a_type_parameter_inside_a_function_field() {
    checks_and_prints(
        "type Box(a) { Box(fn(a) -> a) }\n\
         fn ap(b, x) { match b { Box(f) -> f(x) } }\n\
         pub fn main() {\n\
           let r = ap(Box(fn(x) { x * 3 }), 14)\n\
           io.println(if r == 42 { \"42\" } else { \"wrong\" })\n\
         }\n",
        "42\n",
    );
}
