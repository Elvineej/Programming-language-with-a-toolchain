//! Failure as an effect: an empty type and an empty match (spec
//! `docs/superpowers/specs/2026-10-16-elya-failure-and-cancellation-design.md`).

use elya::check_source;
use elya::parse::parse_module;
use elya::Session;

const FAIL: &str = "type Never { }\n\
effect Fail { fn fail(reason: String) -> Never }\n\
fn raise(r) { match fail(r) { } }\n";

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
fn an_empty_match_on_an_empty_type_has_any_type() {
    // `raise` is used at Int and at Bool: the empty match's result is free.
    let src = format!(
        "{FAIL}fn div(a, b) {{ if b == 0 {{ raise(\"division by zero\") }} else {{ a / b }} }}\n\
         fn ok(b) {{ if b {{ True }} else {{ raise(\"no\") }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let x = handle {{ div(10, 2) + div(1, 0) }} with {{ Fail.fail(r) -> 0 - 1  return(v) -> v }}\n\
         \x20 let y = handle {{ if ok(True) {{ 7 }} else {{ 8 }} }} with {{ Fail.fail(r) -> 0  return(v) -> v }}\n\
         \x20 x * 10 + y\n\
         }}\n"
    );
    assert_eq!(value(&src), -3);
}

#[test]
fn an_empty_match_needs_a_type_with_no_constructors() {
    for scrut in ["Some(1)", "1", "\"s\""] {
        let src = format!(
            "type Option(a) {{ None, Some(a) }}\n\
             pub fn main() -> Int {{ match {scrut} {{ }} }}\n"
        );
        let err = check_source("t.elya", &src)
            .expect_err("must be rejected")
            .to_string();
        assert!(err.contains("E0430"), "{scrut}: {err}");
        assert!(err.contains("no constructors"), "{scrut}: {err}");
    }
}

#[test]
fn a_name_declared_twice_is_e0209() {
    // The review's finding: `type Never { A }` beside `type Never { }` put
    // `Never` among the empty types, so `match e { }` accepted `A` (the
    // evaluator then found no arm). Duplicates of every kind were silent
    // overwrites before (checked clean at fcd9cd4).
    for src in [
        "type Never { A }\ntype Never { }\nfn absurd(e: Never) -> Int { match e { } }\npub fn main() -> Int { absurd(A) }\n",
        "fn f() { 1 }\nfn f() { 2 }\npub fn main() -> Int { f() }\n",
        "effect E { fn a() -> Int }\neffect E { fn b() -> Int }\npub fn main() -> Int { 0 }\n",
        "type T { A }\ntype U { A, B }\npub fn main() -> Int { 0 }\n",
    ] {
        let err = check_source("t.elya", src)
            .expect_err("must be rejected")
            .to_string();
        assert!(err.contains("E0209"), "{src}: {err}");
    }
}
