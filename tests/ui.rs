//! UI tests: fixtures annotated with `//~ ERROR[Ennnn] substring`.

use std::fs;

struct Expectation {
    code: String,
    substring: String,
}

fn parse_expectations(src: &str) -> Vec<Expectation> {
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix("//~ ERROR[") {
            let (code, tail) = rest.split_once(']').expect("malformed //~ ERROR");
            out.push(Expectation {
                code: code.to_string(),
                substring: tail.trim().to_string(),
            });
        }
    }
    out
}

fn check_fixture(name: &str) {
    let path = format!("{}/tests/ui/{name}", env!("CARGO_MANIFEST_DIR"));
    let src = fs::read_to_string(&path).expect("read fixture");
    let expects = parse_expectations(&src);
    assert!(!expects.is_empty(), "fixture has no expectations: {name}");
    let rendered = elya::check_source(name, &src).expect_err("fixture should produce diagnostics");
    for e in &expects {
        assert!(
            rendered.contains(&e.code),
            "missing code {} in output:\n{rendered}",
            e.code
        );
        assert!(
            rendered.contains(&e.substring),
            "missing text `{}` in output:\n{rendered}",
            e.substring
        );
    }
    // Invariant (spec §9): no raw inference tail-variable tokens leak (types or
    // effect rows).
    for bad in ["%r", "%e", "%s", "%t", "%v", "%row"] {
        assert!(
            !rendered.contains(bad),
            "diagnostic leaked internal token `{bad}`:\n{rendered}"
        );
    }
}

#[test]
fn unresolved() {
    check_fixture("unresolved.elya");
}

#[test]
fn bad_builtin() {
    check_fixture("bad_builtin.elya");
}

#[test]
fn type_mismatch() {
    check_fixture("type_mismatch.elya");
}

#[test]
fn occurs_check() {
    check_fixture("occurs_check.elya");
}

#[test]
fn bad_arity() {
    check_fixture("bad_arity.elya");
}

#[test]
fn non_bool_cond() {
    check_fixture("non_bool_cond.elya");
}

#[test]
fn resume_outside_handler() {
    check_fixture("resume_outside_handler.elya");
}

#[test]
fn unhandled_effect() {
    check_fixture("unhandled_effect.elya");
}

#[test]
fn impure_pure_fn() {
    check_fixture("impure_pure_fn.elya");
}

#[test]
fn row_mismatch() {
    check_fixture("row_mismatch.elya");
}

#[test]
fn partial_ctor() {
    check_fixture("partial_ctor.elya");
}

#[test]
fn multi_on_oneshot() {
    check_fixture("multi_on_oneshot.elya");
}

#[test]
fn affine_double_use() {
    check_fixture("affine_double_use.elya");
}

#[test]
fn affine_multi_capture() {
    check_fixture("affine_multi_capture.elya");
}

#[test]
fn nonexhaustive_list() {
    check_fixture("nonexhaustive_list.elya");
}

#[test]
fn nonexhaustive_nested() {
    check_fixture("nonexhaustive_nested.elya");
}

#[test]
fn nonexhaustive_int() {
    check_fixture("nonexhaustive_int.elya");
}
