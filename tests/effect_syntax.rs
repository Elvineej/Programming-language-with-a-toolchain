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
    let src = "effect multi Flip {\n\
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
// (The 3b test `effects_type_check_but_dont_evaluate_yet` was removed in 3c:
//  effects now evaluate on the CEK machine, so its "not evaluated yet" premise
//  is obsolete. Execution is verified by the output-checked golden corpus in
//  tests/effects_run.rs — the deliberate replacement for the missing oracle.)

#[test]
fn multi_declared_ops_is_keyed_by_operation_name_not_effect_name() {
    // Two `multi` effects (one single-op, one two-op) and one non-`multi`.
    let src = "effect multi Flip { fn flip() -> Bool }\n\
               effect multi Choice { fn pick() -> Int  fn stop() -> Unit }\n\
               effect State { fn get() -> String  fn set(v: String) -> Unit }\n";
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");

    let ops = elya::ast::multi_declared_ops(&m);

    // Keyed by OPERATION name, and every op of a multi effect is present —
    // the two-op effect proves the per-effect loop, not just the outer one.
    assert!(ops.contains("flip"), "{ops:?}");
    assert!(ops.contains("pick"), "{ops:?}");
    assert!(ops.contains("stop"), "{ops:?}");

    // Effect names must never leak into the set (this is the D4 distinction:
    // the set is op-keyed, unlike inference's effect-keyed `effect_multi`).
    assert!(!ops.contains("Flip"), "effect name leaked: {ops:?}");
    assert!(!ops.contains("Choice"), "effect name leaked: {ops:?}");

    // Operations of a non-`multi` effect are absent — both of them.
    assert!(!ops.contains("get"), "{ops:?}");
    assert!(!ops.contains("set"), "{ops:?}");

    assert_eq!(ops.len(), 3, "exactly the three multi ops: {ops:?}");
}
