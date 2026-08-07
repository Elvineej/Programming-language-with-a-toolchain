//! Slice 3a integration gate: effect *syntax* is parsed and name-resolved,
//! but not yet type-checked. A well-formed effect program must parse cleanly,
//! resolve cleanly, and produce exactly the "effects aren't type-checked yet"
//! diagnostic (E0499) from the inference stage — nothing else.
//!
//! Coverage per the Slice 3 plan: a basic handler, a `with multi` handler, and
//! a lexically *nested* handler all reach the same E0499 gate.

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

/// Assert: parses clean, resolves clean, and the only type diagnostics are
/// E0499 (at least one). This is the whole Slice 3a contract in one place.
fn assert_reaches_e0499_gate(src: &str) {
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
    assert!(
        !st.types.is_empty() && st.types.iter().all(|c| c == "E0499"),
        "expected only E0499 from inference, got: {:?}",
        st.types
    );
}

#[test]
fn basic_handler_reaches_e0499() {
    let src = "effect Log {\n\
               \x20 fn log(msg: String) -> Unit\n\
               }\n\
               fn prog() {\n\
               \x20 handle log(\"hi\") with {\n\
               \x20   Log.log(m) -> resume(m)\n\
               \x20   return(r) -> r\n\
               \x20 }\n\
               }\n";
    assert_reaches_e0499_gate(src);
}

#[test]
fn multi_handler_reaches_e0499() {
    let src = "effect Flip {\n\
               \x20 fn flip() -> Bool\n\
               }\n\
               fn prog() {\n\
               \x20 handle flip() with multi {\n\
               \x20   Flip.flip() -> resume(True)\n\
               \x20 }\n\
               }\n";
    assert_reaches_e0499_gate(src);
}

#[test]
fn nested_handler_reaches_e0499() {
    // A handler lexically nested inside another handler's clause body.
    let src = "effect Ask {\n\
               \x20 fn ask() -> Int\n\
               }\n\
               effect Log {\n\
               \x20 fn log(n: Int) -> Unit\n\
               }\n\
               fn prog() {\n\
               \x20 handle ask() with {\n\
               \x20   Ask.ask() -> handle log(1) with {\n\
               \x20     Log.log(n) -> resume(0)\n\
               \x20   }\n\
               \x20 }\n\
               }\n";
    assert_reaches_e0499_gate(src);
}
