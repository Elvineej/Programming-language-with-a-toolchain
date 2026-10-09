//! E0207 (the maintainer's decision, 2026-10-08): a handler gives a clause for
//! EVERY operation of the effect it handles.
//!
//! Before, a partial handler checked clean and the type checker treated it as
//! discharging the whole effect, while at run time the uncovered operations went
//! to an outer handler -- or to none: `check` passed and the evaluator stopped
//! with "internal: unhandled effect `put` reached the machine" (found by the
//! 5b-10 review; natively it compiled to silent wrong values until `cps::Fx`
//! learned to treat it as leaking).
//!
//! Negative control (reverted, `cmp` against a saved copy): the check removed --
//! the three rejection tests fail (they check clean); the others still pass.

use elya::check_source;

const S: &str = "effect S { fn get() -> Int  fn put(x: Int) -> Int }\n";

fn err(src: &str) -> String {
    check_source("t.elya", &format!("{S}{src}")).expect_err("should be rejected")
}

fn ok(src: &str) {
    let r = check_source("t.elya", &format!("{S}{src}"));
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn a_handler_missing_an_operation_is_e0207_and_names_it() {
    let e = err("pub fn main() -> Int {\n\
                   handle { get() } with { S.get() -> resume(1)  return(r) -> r }\n\
                 }\n");
    assert!(e.contains("E0207"), "{e}");
    assert!(e.contains("`put`"), "names the missing op: {e}");
    assert!(e.contains("S.put(a)"), "offers the clause to add: {e}");
}

#[test]
fn the_unsound_partial_handler_in_a_pure_typed_function_is_rejected() {
    // Checked clean before; the evaluator stopped on an unhandled `put`.
    let e = err("fn f() -> Int {\n\
                   handle { get() + put(5) } with { S.get() -> resume(1)  return(r) -> r * 10 }\n\
                 }\n\
                 pub fn main() -> Int { f() }\n");
    assert!(e.contains("E0207"), "{e}");
}

#[test]
fn a_partial_handler_nested_in_a_full_one_is_still_rejected() {
    // Forwarding the rest outward was the other option; the maintainer chose
    // the error.
    let e = err("pub fn main() -> Int {\n\
                   handle {\n\
                     handle { get() + put(5) } with { S.get() -> resume(1)  return(r) -> r }\n\
                   } with { S.get() -> resume(100)  S.put(x) -> resume(x)  return(r) -> r }\n\
                 }\n");
    assert!(e.contains("E0207"), "{e}");
    assert_eq!(e.matches("E0207").count(), 1, "only the inner handler: {e}");
}

#[test]
fn a_handler_covering_every_operation_checks() {
    ok("pub fn main() -> Int {\n\
          handle { get() + put(5) } with { S.get() -> resume(1)  S.put(x) -> resume(x)  return(r) -> r }\n\
        }\n");
}

#[test]
fn an_operation_named_by_a_malformed_clause_is_not_reported_twice() {
    // `put` binds the wrong number of arguments: E0203 says so, and E0207 does
    // not also call it missing.
    let e = err("pub fn main() -> Int {\n\
                   handle { get() } with { S.get() -> resume(1)  S.put() -> resume(0)  return(r) -> r }\n\
                 }\n");
    assert!(e.contains("E0203"), "{e}");
    assert!(!e.contains("E0207"), "{e}");
}

#[test]
fn a_handler_with_only_a_return_clause_handles_no_effect() {
    ok("pub fn main() -> Int { handle { 1 } with { return(r) -> r + 1 } }\n");
}
