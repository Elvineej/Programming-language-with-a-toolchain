//! Slice 5c-2: name rules. A second clause for one op is E0204; a top-level
//! function named like an op is E0205; a LOCAL binding shadows an op,
//! lexically, in inference, the evaluator and Core lowering alike. Programs
//! n1-n5 are the spec's measured table (§0); each test notes the build before.

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
