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

#[test]
fn a3_polymorphic_effects_are_refused_nowhere_in_the_front_end() {
    // Acceptance criterion A3, which the spec marked UNCERTAIN: where does a
    // polymorphic effect get refused?
    //
    // Prediction, recorded before measuring: the front end rejects it somewhere
    // upstream of codegen, with no confident claim about where; the stated
    // fallback was that nothing may refuse it.
    //
    // Measured: the fallback. Nothing refuses it, at any stage. Both programs
    // below are clean through parse, resolve and types, and `check_source`
    // returns Ok. The declaration additionally lowers to Core successfully.
    //
    // That is a real gap, not a curiosity. The back end is monomorphic-only
    // (5b-4 6), and parametric ADTs are at least silently skipped by the
    // `t.params.is_empty()` guard at src/core.rs:143. Effects have no analogous
    // guard: `lower_module`'s Decl::Effect arm refuses `effect IO` and nothing
    // else. A polymorphic effect therefore reaches codegen unannounced.
    //
    // This test pins the measurement rather than the desired behaviour. If a
    // refusal is added later it fails loudly, which is the point: the next
    // person to touch this should see that A3's answer moved.

    // (a) Declaration alone: clean front end, and Core lowering accepts it.
    let decl = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }
                pub fn main() -> Int { 0 }
";
    assert!(
        elya::check_source("a3-decl.elya", decl).is_ok(),
        "A3: a polymorphic effect declaration was refused by the front end"
    );
    let st = stages(decl);
    assert!(
        st.parse.is_empty() && st.resolve.is_empty() && st.types.is_empty(),
        "A3 declaration: parse={:?} resolve={:?} types={:?}",
        st.parse,
        st.resolve,
        st.types
    );
    let (m, _) = parse_module(&Session::new(), decl);
    let (_, table) = types::infer_typed_table(&Session::new(), &m);
    assert!(
        elya::core::lower_module(&m, &table).is_ok(),
        "A3: Core lowering refused a polymorphic effect declaration"
    );

    // (b) The load-bearing half: a declaration nothing uses proves little. This
    // one invokes a polymorphic op and handles it. Core lowering refuses every
    // handler until Task 5, so the question here is strictly the front end's --
    // asserting on lowering would couple this test to that task.
    let used = "effect State(s) {
                  fn get() -> s
                  fn set(v: s) -> Unit
                }
                fn prog() {
                  handle get() with {
                    State.get() -> resume(7)
                    return(r) -> r
                  }
                }
";
    assert!(
        elya::check_source("a3-use.elya", used).is_ok(),
        "A3: using and handling a polymorphic op was refused by the front end"
    );
    let st = stages(used);
    assert!(
        st.parse.is_empty() && st.resolve.is_empty() && st.types.is_empty(),
        "A3 use+handle: parse={:?} resolve={:?} types={:?}",
        st.parse,
        st.resolve,
        st.types
    );
}
