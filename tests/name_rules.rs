//! Slice 5c-2: name rules. A second clause for one op is E0204; a top-level
//! function named like an op is E0205; a LOCAL binding shadows an op,
//! lexically, in inference, the evaluator and Core lowering alike. Programs
//! n1-n5 are the spec's measured table (§0); each test notes the build before.
//!
//! Negative controls (plan Task 3), each reverted, each failing differently:
//! - K1, E0204 off: the two duplicate-clause tests read `[]`.
//! - K2, E0205 off: the fn-named-like-an-op test reads `[]`.
//! - K3, inference op-first again: NO value test fails (the evaluator calls
//!   the local whatever inference decided); natively n5 is refused as an
//!   "effectful closure call", and the type test reads `fn() / {E} -> Int`.
//!   Predicted E0400 or a value failure: a miss, and the reason the type
//!   test exists.
//! - K4, evaluator op-first again: values 1, 1, 2 (the old answers); natively
//!   only the differential side fails.
//! - K5, lowering ignores the resolver's set: the core test sees 1 Perform,
//!   native n3 prints "PERFORMED", native n5 prints 2.

use elya::eval::{run_module_value, Value};
use elya::parse::parse_module;
use elya::types::infer_schemes;
use elya::{check_source, resolve, Session};

fn codes(src: &str) -> Vec<String> {
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let rd = resolve::check(&session, &m);
    let diags = if rd.is_empty() {
        infer_schemes(&session, &m).1
    } else {
        rd
    };
    diags.into_iter().map(|d| d.code).collect()
}

fn rendered_error(src: &str) -> String {
    check_source("t.elya", src).expect_err("must be rejected")
}

fn value(src: &str) -> Value {
    assert!(codes(src).is_empty(), "must check clean: {:?}", codes(src));
    let (m, _) = parse_module(&Session::new(), src);
    run_module_value(&m).expect("evaluates").1
}

/// `body` runs under a handler whose `ping` resumes with 1; main is its value.
fn under_ping_handler(decls: &str, body: &str) -> String {
    format!(
        "effect E {{ fn ping() -> Int }}\n{decls}\
         pub fn main() -> Int {{\n\
         \x20 handle {{ {body} }} with {{\n\
         \x20   E.ping() -> resume(1)\n\
         \x20   return(x) -> x\n\
         \x20 }}\n\
         }}\n"
    )
}

// ---- Task 1: E0204, E0205 ----

#[test]
fn a_second_clause_for_one_op_is_e0204() {
    // n1. Before: accepted, first clause won.
    let src = "effect Ask { fn ask() -> Int }\n\
               fn one() { ask() }\n\
               pub fn main() -> Int {\n\
               \x20 handle { one() } with {\n\
               \x20   Ask.ask() -> resume(2)\n\
               \x20   Ask.ask() -> resume(5)\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               }\n";
    assert_eq!(codes(src), ["E0204"]);
    let r = rendered_error(src);
    assert!(
        r.contains("`ask` already has a clause in this handler"),
        "{r}"
    );
}

#[test]
fn a_second_clause_spelled_differently_is_still_e0204() {
    // Op names are unique (5c-1), so `ask()` and `Ask.ask()` are one op.
    let src = "effect Ask { fn ask() -> Int }\n\
               fn one() { ask() }\n\
               pub fn main() -> Int {\n\
               \x20 handle { one() } with {\n\
               \x20   ask() -> resume(2)\n\
               \x20   Ask.ask() -> resume(5)\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               }\n";
    assert_eq!(codes(src), ["E0204"]);
}

#[test]
fn a_function_named_like_an_op_is_e0205() {
    // n2. Before: accepted; every `ping()` performed, so `fn ping` was dead.
    let src = under_ping_handler(
        "fn ping() -> Int { 5 }\nfn user() -> Int { ping() }\n",
        "user()",
    );
    assert_eq!(codes(&src), ["E0205"]);
    let r = rendered_error(&src);
    assert!(
        r.contains("function `ping` has the name of an operation of `E`"),
        "{r}"
    );
}

// ---- Task 2: locals before ops ----

#[test]
fn a_let_bound_local_shadows_an_op() {
    // n3. Before: 1 (the op performed).
    let src = under_ping_handler(
        "fn user() -> Int {\n  let ping = fn() { 5 }\n  ping()\n}\n",
        "user()",
    );
    assert_eq!(value(&src), Value::Int(5));
}

#[test]
fn a_parameter_shadows_an_op() {
    // n4. Before: 1.
    let src = under_ping_handler("fn user(ping) { ping() }\n", "user(fn() { 5 })");
    assert_eq!(value(&src), Value::Int(5));
}

#[test]
fn shadowing_ends_with_its_scope() {
    // n5. Before: 2 (both performed). 6 = the local's 5 inside the block, the
    // op's 1 after it; 10 would mean the shadow leaked out of its block.
    let src = under_ping_handler(
        "fn user() -> Int {\n  let a = { let ping = fn() { 5 }  ping() }\n  a + ping()\n}\n",
        "user()",
    );
    assert_eq!(value(&src), Value::Int(6));
}

#[test]
fn a_shadowed_call_does_not_perform_in_its_inferred_type() {
    // Control K3 found that inference's choice is invisible to every value
    // test -- the evaluator calls the local whatever inference decided -- and
    // shows only in the TYPE: op-first inference typed `user` as performing
    // `E` while it never does. So the type itself is pinned.
    let src = under_ping_handler(
        "fn user() -> Int {\n  let ping = fn() { 5 }\n  ping()\n}\n",
        "user()",
    );
    let (m, pd) = parse_module(&Session::new(), &src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (schemes, diags) = infer_schemes(&Session::new(), &m);
    assert!(diags.is_empty(), "{diags:?}");
    let user = schemes
        .iter()
        .find(|(n, _)| n == "user")
        .map(|(_, s)| s.as_str());
    assert_eq!(user, Some("fn() -> Int"));
}

// ---- Review follow-ups ----

#[test]
fn a_local_named_like_a_multi_op_is_not_a_multi_shot_perform_to_affine() {
    // Review: the affine checker (a fourth layer) still took any call named
    // like a `multi` op as a perform, so this program -- whose `flip()` is the
    // LOCAL -- was a false E0429 "used after a multi-shot perform". Renaming
    // the local checked clean.
    let src = "effect multi Flip { fn flip() -> Bool }\n\
               linear type Tok { Tok }\n\
               fn use1(t) { match t { Tok -> \"ok\" } }\n\
               pub fn main() { let t = Tok  let flip = fn() { True }  let _ = flip()  io.println(use1(t)) }\n";
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
}

#[test]
fn a_clause_already_reported_e0203_draws_no_e0204() {
    // Review: a malformed clause (wrong arity) is not a second clause for the
    // op -- reporting E0204 too told the CORRECT clause it "can never run".
    let src = "effect Ask { fn ask() -> Int }\n\
               fn one() { ask() }\n\
               pub fn main() -> Int {\n\
               \x20 handle { one() } with {\n\
               \x20   Ask.ask(a) -> resume(a)\n\
               \x20   Ask.ask() -> resume(2)\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               }\n";
    assert_eq!(codes(src), ["E0203"]);
}

#[test]
fn every_kind_of_local_binder_shadows_an_op() {
    // Review coverage: clause parameter, return binder, match binder, a lambda
    // capturing a shadowing local, and a self-reference in the `let` value
    // (evaluated before the binding, so it is still the op: 1 + 10 = 11).
    let cases: [(&str, &str, i64); 4] = [
        (
            "fn user() -> Int { let ping = fn() { 5 }  let g = fn() { ping() }  g() }\n",
            "user()",
            5,
        ),
        (
            "fn user() -> Int { let ping = { let p = ping()  fn() { p + 10 } }  ping() }\n",
            "user()",
            11,
        ),
        (
            "type Box { B(Int) }\nfn user() -> Int { match B(7) { B(ping) -> ping } }\n",
            "user()",
            7,
        ),
        (
            "fn user() -> Int { let ping = fn() { 5 }  if True { ping() } else { 0 } }\n",
            "user()",
            5,
        ),
    ];
    for (decls, body, want) in cases {
        let src = under_ping_handler(decls, body);
        assert_eq!(value(&src), Value::Int(want), "{decls}");
    }
    // Return binder: `return(ping) -> ping()` calls the binder (a closure).
    let src = "effect E { fn ping() -> Int }\n\
               pub fn main() -> Int {\n\
               \x20 handle { fn() { 5 } } with {\n\
               \x20   E.ping() -> resume(1)\n\
               \x20   return(ping) -> ping()\n\
               \x20 }\n\
               }\n";
    assert_eq!(value(src), Value::Int(5));
}
