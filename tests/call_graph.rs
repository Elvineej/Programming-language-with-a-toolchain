//! Inference's call graph must see every call. `collect_refs_expr` ended in
//! `_ => {}`, so a call inside a `match` arm, a lambda or a handler was no edge:
//! the caller could be generalised BEFORE its callee was typed, giving it a
//! `forall a` result -- unsound. Found 2026-10-05 writing a CEK machine in
//! Elya (`ev : forall a. fn(Term, Env, Kont) -> a`). Before the fix each
//! program below passed `elya check` and failed at run time ("io.println
//! expects a single String"); declaring the callee first made the checker
//! report E0400 -- the result depended on declaration order.

use elya::parse::parse_module;
use elya::types::infer_schemes;
use elya::Session;

fn codes(src: &str) -> Vec<String> {
    let s = Session::new();
    let (m, pd) = parse_module(&s, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let rd = elya::resolve::check(&s, &m);
    assert!(rd.is_empty(), "resolve: {rd:?}");
    infer_schemes(&s, &m)
        .1
        .into_iter()
        .map(|d| d.code)
        .collect()
}

fn scheme_of(src: &str, name: &str) -> String {
    let s = Session::new();
    let (m, _) = parse_module(&s, src);
    let (schemes, diags) = infer_schemes(&s, &m);
    assert!(diags.is_empty(), "{diags:?}");
    schemes
        .into_iter()
        .find(|(n, _)| n == name)
        .expect("scheme")
        .1
}

#[test]
fn a_call_inside_a_match_arm_is_an_edge() {
    let src = "type T { A }\nfn f(t) { match t { A -> g(t) } }\nfn g(t) { 5 }\n\
               pub fn main() { io.println(f(A)) }\n";
    assert!(
        codes(src).contains(&"E0400".to_string()),
        "{:?}",
        codes(src)
    );
}

#[test]
fn a_call_inside_a_lambda_is_an_edge() {
    let src = "fn f(x) { let h = fn() { g(x) }  h() }\nfn g(x) { x + 1 }\n\
               pub fn main() { io.println(f(1)) }\n";
    assert!(
        codes(src).contains(&"E0400".to_string()),
        "{:?}",
        codes(src)
    );
}

#[test]
fn a_call_inside_a_handler_is_an_edge() {
    // Body, op clause and return clause each hide the ONLY use of `y`: a call
    // to the later `g`, which forces `y : Int`. A missing edge left `f` generic
    // in `y`, so `f("s")` checked clean. (The first version of this test let
    // the handler's result type decide instead, and a control showed it could
    // not see a missing clause edge.)
    for (body, clause, ret) in [
        ("g(y)", "resume(1)", "x"),
        ("ask()", "resume(g(y))", "x"),
        ("ask()", "resume(1)", "x + g(y)"),
    ] {
        let src = format!(
            "effect Ask {{ fn ask() -> Int }}\n\
             fn f(y) {{ handle {{ {body} }} with {{ Ask.ask() -> {clause}  return(x) -> {ret} }} }}\n\
             fn g(z) {{ z + 1 }}\n\
             pub fn main() -> Int {{ f(\"s\") }}\n"
        );
        assert!(
            codes(&src).contains(&"E0400".to_string()),
            "{body} / {clause} / {ret}: {:?}",
            codes(&src)
        );
    }
}

#[test]
fn mutually_recursive_functions_through_match_arms_share_one_type() {
    // The CEK machine's shape: `ev` only ever answers through `co`.
    let src = "type T { L(Int), N(T) }\n\
               fn ev(t) { match t { L(n) -> co(n)  N(u) -> ev(u) } }\n\
               fn co(n) { if n == 0 { 0 } else { ev(L(n - 1)) } }\n";
    assert_eq!(scheme_of(src, "ev"), "fn(T) -> Int");
}
