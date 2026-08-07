//! Slice 3b: well-formed effect programs parse, resolve, AND type-check cleanly
//! — effects are now typed with row inference and handler discharge. Evaluation
//! is still deferred to 3c, so `run_source` reports the "not evaluated yet"
//! placeholder (asserted below).
//!
//! (In 3a these same shapes stopped at the E0499 "not type-checked yet" gate;
//! Task 5 removes that placeholder — the feature landing, not an accommodation.)

use elya::{parse::parse_module, resolve, types, Session};

/// Diagnostic codes emitted by each front-end stage, in order.
struct Stages {
    parse: Vec<String>,
    resolve: Vec<String>,
    types: Vec<String>,
}

fn stages(src: &str) -> Stages {
    let s = Session::new();
    let (module, pdiags) = parse_module(&s, src);
    Stages {
        parse: pdiags.iter().map(|d| d.code.clone()).collect(),
        resolve: resolve::check(&s, &module)
            .iter()
            .map(|d| d.code.clone())
            .collect(),
        types: types::infer(&s, &module)
            .iter()
            .map(|d| d.code.clone())
            .collect(),
    }
}

/// Assert: parses clean, resolves clean, and type-checks clean (no diagnostics
/// at any stage). Effects are typed and their effect discharged.
fn assert_type_checks_clean(src: &str) {
    let st = stages(src);
    assert!(
        st.parse.is_empty(),
        "unexpected parse diags: {:?}",
        st.parse
    );
    assert!(
        st.resolve.is_empty(),
        "unexpected resolve diags: {:?}",
        st.resolve
    );
    assert!(st.types.is_empty(), "unexpected type diags: {:?}", st.types);
}

#[test]
fn basic_handler_type_checks() {
    let src = "effect Log {\n\
               \x20 fn log(msg: String) -> Unit\n\
               }\n\
               fn prog() {\n\
               \x20 handle log(\"hi\") with {\n\
               \x20   Log.log(m) -> resume(Unit)\n\
               \x20   return(r) -> r\n\
               \x20 }\n\
               }\n";
    assert_type_checks_clean(src);
}

#[test]
fn multi_handler_type_checks() {
    let src = "effect Flip {\n\
               \x20 fn flip() -> Bool\n\
               }\n\
               fn prog() {\n\
               \x20 handle flip() with multi {\n\
               \x20   Flip.flip() -> resume(True)\n\
               \x20 }\n\
               }\n";
    assert_type_checks_clean(src);
}

#[test]
fn nested_handlers_type_check() {
    // An inner handler discharges Log; the outer discharges Warn. Both effects
    // are handled, so nothing escapes.
    let src = "effect Log {\n\
               \x20 fn log(msg: String) -> Unit\n\
               }\n\
               effect Warn {\n\
               \x20 fn warn(msg: String) -> Unit\n\
               }\n\
               fn prog() {\n\
               \x20 handle (handle log(\"a\") with { Log.log(m) -> resume(Unit) }) with {\n\
               \x20   Warn.warn(m) -> resume(Unit)\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               }\n";
    assert_type_checks_clean(src);
}

#[test]
fn effects_type_check_but_dont_evaluate_yet() {
    // The whole point of 3b: effects type-check. Evaluation is 3c, so running
    // the program still reports the machine's "not evaluated yet" placeholder.
    let src = "effect Log {\n\
               \x20 fn log(msg: String) -> Unit\n\
               }\n\
               pub fn main() {\n\
               \x20 handle log(\"hi\") with {\n\
               \x20   Log.log(m) -> resume(Unit)\n\
               \x20   return(r) -> r\n\
               \x20 }\n\
               }\n";
    assert!(
        elya::check_source("t.elya", src).is_ok(),
        "effect program should type-check in 3b"
    );
    let err = elya::run_source("t.elya", src).unwrap_err();
    assert!(
        err.contains("not evaluated yet"),
        "expected the 3c eval placeholder, got: {err}"
    );
}
