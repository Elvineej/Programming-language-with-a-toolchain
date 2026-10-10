//! Type annotations are checked (2026-10-09; Claude's decision under the
//! maintainer's delegation, HANDOFF rule 2). The parser used to DISCARD every
//! parameter, return and `let` type (`skip_type_annotation`, "Slice 1 has no
//! type checker"), so `let s: String = 1` checked clean.
//!
//! The rules (spec `2026-10-09-elya-annotations-design.md`):
//! - an annotation is unified with the inferred type and reported at the
//!   annotation;
//! - a lowercase name is ONE flexible type variable per top-level function
//!   (shared by its signature, `let`s and lambdas), not a rigid "for all";
//! - `fn(A) / {E} -> R` is a function type; with no `/ {..}` its row is open
//!   (any effects), with one it is exactly those.
//!
//! Negative controls, each reverted (`cmp` against a saved copy):
//! - C1, `check_ann` a no-op: all seven rejection tests fail (they check
//!   clean);
//! - C2, a fresh variable per lowercase occurrence: `a_type_variable_is_shared`
//!   fails (it checks clean);
//! - C3, the function body passed as the "expected" side again: the return
//!   message test fails (the message is reversed).

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
fn a_let_annotation_is_checked() {
    // Nothing else uses `s`: only the annotation can object.
    let err = check_err("pub fn main() -> Int { let s: String = 1\n 0 }\n");
    assert!(err.contains("E04"), "{err}");
}

#[test]
fn a_return_annotation_is_checked() {
    let err = check_err(
        "fn f(x) -> String { x + 1 }\n\
         pub fn main() -> Int { 0 }\n",
    );
    assert!(err.contains("E04"), "{err}");
}

#[test]
fn a_parameter_annotation_constrains_the_callers() {
    let err = check_err(
        "fn f(x: Int) -> Int { x }\n\
         pub fn main() -> Int { f(\"a\") }\n",
    );
    assert!(err.contains("E04"), "{err}");
}

#[test]
fn a_lambda_parameter_annotation_is_checked() {
    let err = check_err("pub fn main() -> Int { let f = fn(x: Int) { x }  f(True) }\n");
    assert!(err.contains("E04"), "{err}");
}

#[test]
fn an_unknown_type_in_an_annotation_is_e0432() {
    let err = check_err("fn f(x: Integer) -> Int { 1 }\npub fn main() -> Int { f(1) }\n");
    assert!(err.contains("E0432"), "{err}");
}

#[test]
fn a_function_type_with_no_row_admits_any_effects() {
    checks_and_prints(
        "effect L { fn lg(x: Int) -> Int }\n\
         fn apply(k: fn(Int) -> Int, x: Int) -> Int { k(x) }\n\
         pub fn main() {\n\
           let r = handle { apply(fn(x) { lg(x) }, 4) } with { L.lg(x) -> resume(x * 10)  return(r) -> r }\n\
           io.println(if r == 40 { \"40\" } else { \"wrong\" })\n\
         }\n",
        "40\n",
    );
}

#[test]
fn a_function_type_with_a_written_row_means_exactly_that_row() {
    // `/ {}`: a pure callback; an effectful one does not fit.
    let err = check_err(
        "effect L { fn lg(x: Int) -> Int }\n\
         fn apply(k: fn(Int) / {} -> Int, x: Int) -> Int { k(x) }\n\
         pub fn main() -> Int {\n\
           handle { apply(fn(x) { lg(x) }, 4) } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
         }\n",
    );
    assert!(err.contains("E0423"), "{err}");
}

#[test]
fn an_annotated_identity_is_still_polymorphic() {
    checks_and_prints(
        "fn id(x: a) -> a { x }\n\
         pub fn main() { io.println(if id(True) { if id(3) == 3 { \"ok\" } else { \"no\" } } else { \"no\" }) }\n",
        "ok\n",
    );
}

#[test]
fn a_type_variable_is_shared_across_the_signature() {
    let err = check_err(
        "fn first(x: a, y: a) -> a { x }\n\
         pub fn main() -> Int { first(1, True) }\n",
    );
    assert!(err.contains("E04"), "{err}");
}

#[test]
fn adt_and_effect_annotations_check() {
    checks_and_prints(
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         effect L { fn lg(x: Int) -> Int }\n\
         fn len(xs: List(a)) -> Int { match xs { Nil -> 0  Cons(_, t) -> 1 + len(t) } }\n\
         fn logged(xs: List(Int)) / {L} -> Int { lg(len(xs)) }\n\
         pub fn main() {\n\
           let n: Int = handle { logged(Cons(1, Cons(2, Nil))) } with { L.lg(x) -> resume(x + 1)  return(r) -> r }\n\
           io.println(if n == 3 { \"3\" } else { \"wrong\" })\n\
         }\n",
        "3\n",
    );
}

// ---- the independent review (2026-10-09) ----------------------------------

#[test]
fn a_let_does_not_generalize_the_functions_annotation_variable() {
    // F1 (high): `let id: fn(a) -> a = ..` generalized the function-wide `a`,
    // `let k: a = 1` then bound it to Int, and the final zonk rewrote every
    // node typed `a` -- natively a captured ADT was treated as an untraced
    // Int (3395 for the evaluator's 42). The two orders must agree: `a` is
    // one type for the whole function, so `id("s")` is a mismatch either way.
    for src in [
        "fn t(x: Int) -> Int {\n\
           let id: fn(a) -> a = fn(y) { y }\n\
           let k: a = 1\n\
           let s = id(\"s\")\n\
           id(x)\n\
         }\n\
         pub fn main() -> Int { t(4) }\n",
        "fn t(x: Int) -> Int {\n\
           let k: a = 1\n\
           let id: fn(a) -> a = fn(y) { y }\n\
           let s = id(\"s\")\n\
           id(x)\n\
         }\n\
         pub fn main() -> Int { t(4) }\n",
    ] {
        let err = check_err(src);
        assert!(err.contains("E0400"), "{err}");
    }
}

#[test]
fn an_unwritten_row_in_a_result_annotation_still_upcasts() {
    // F2: a function type with no row is OPEN, so annotating `mk`'s result
    // must not stop its pure closure from fitting a pure parameter (E0423
    // before the fix; unannotated, the program always checked: 8).
    checks_and_prints(
        "effect L { fn lg(x: Int) -> Int }\n\
         fn apply_p(k: fn(Int) / {} -> Int, x: Int) -> Int { k(x) }\n\
         fn mk() -> fn(Int) -> Int { fn(x) { x + 1 } }\n\
         fn w() / {L} -> Int {\n\
           let g = mk()\n\
           let a = lg(1)\n\
           let b = g(2)\n\
           a + b + apply_p(g, 3)\n\
         }\n\
         pub fn main() {\n\
           let r = handle { w() } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
           io.println(if r == 8 { \"8\" } else { \"wrong\" })\n\
         }\n",
        "8\n",
    );
    checks_and_prints(
        "effect L { fn lg(x: Int) -> Int }\n\
         fn apply_p(k: fn(Int) / {} -> Int, x: Int) -> Int { k(x) }\n\
         fn w() / {L} -> Int {\n\
           let h: fn(Int) -> Int = if True { fn(x) { x } } else { fn(x) { x + 1 } }\n\
           let a = lg(1)\n\
           let b = h(2)\n\
           a + b + apply_p(h, 3)\n\
         }\n\
         pub fn main() {\n\
           let r = handle { w() } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
           io.println(if r == 6 { \"6\" } else { \"wrong\" })\n\
         }\n",
        "6\n",
    );
}

#[test]
fn a_base_type_given_arguments_is_an_error() {
    // F4: `Int(String, Bool)` checked clean; `Unit(Int)` said "unknown type".
    for ann in ["Int(String, Bool)", "Unit(Int)"] {
        let err = check_err(&format!(
            "fn f(x: {ann}) -> Int {{ 1 }}\npub fn main() -> Int {{ 0 }}\n"
        ));
        assert!(err.contains("takes no type arguments"), "{ann}: {err}");
    }
}

#[test]
fn parenthesised_and_empty_types_parse() {
    // F5: the old skipper accepted `(Int)` and `()`; they mean Int and Unit.
    checks_and_prints(
        "fn f(x: (Int), u: ()) -> (Int) { x + 1 }\n\
         pub fn main() { io.println(if f(1, Unit) == 2 { \"2\" } else { \"wrong\" }) }\n",
        "2\n",
    );
}

#[test]
fn a_function_type_in_a_declaration_checks() {
    // F7 named function types in declarations "not supported yet"; async
    // step 1 (2026-10-09) supports them (replaces
    // `a_function_type_in_a_declaration_is_named_not_unknown`; the rules are
    // in tests/fn_fields.rs).
    for src in [
        "type Box { B(fn(Int) -> Int) }\npub fn main() -> Int { 0 }\n",
        "effect E { fn ap(f: fn(Int) -> Int) -> Int }\npub fn main() -> Int { 0 }\n",
    ] {
        assert!(check_source("t.elya", src).is_ok(), "{src}");
    }
}

#[test]
fn a_return_mismatch_reads_expected_the_annotation() {
    // F6: the body's type was passed as the "expected" side, so the message
    // was reversed ("expected `Int`, found `String`").
    let err = check_err("fn f(x: Int) -> String { x }\npub fn main() -> Int { 0 }\n");
    assert!(err.contains("expected `String`, found `Int`"), "{err}");
}
