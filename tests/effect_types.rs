//! Slice 3b: inferred effect rows in top-level function schemes. Pins the
//! printed form of ambient-row inference — concrete-effect functions get a
//! closed row, a pure relay is row-polymorphic, pure functions show no row.

use elya::diag::{render, Diagnostic, Severity};
use elya::span::SourceMap;
use elya::{check_source, parse::parse_module, run_source, types, types::infer_schemes, Session};
use std::collections::HashMap;

fn schemes(src: &str) -> (HashMap<String, String>, Vec<String>) {
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse diags: {pd:?}");
    let (schemes, diags) = infer_schemes(&Session::new(), &m);
    let map: HashMap<String, String> = schemes.into_iter().collect();
    let codes: Vec<String> = diags.iter().map(|d| d.code.clone()).collect();
    (map, codes)
}

#[test]
fn pure_function_has_no_row() {
    let (s, d) = schemes("fn add(a, b) { a + b }\n");
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["add"], "fn(Int, Int) -> Int");
}

#[test]
fn concrete_effect_function_has_closed_row() {
    // `greet` performs exactly {Log}; its inferred row is closed, not {Log|e}.
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn greet(name) { log(name) }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["greet"], "fn(String) / {Log} -> Unit");
}

#[test]
fn builtin_println_performs_io() {
    let (s, d) = schemes("fn hi() { io.println(\"x\") }\n");
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["hi"], "fn() / {IO} -> Unit");
}

#[test]
fn relay_is_row_polymorphic() {
    // `run_it`'s effect equals its argument's effect — parametric row
    // polymorphism, no subtyping (spec §3.5).
    let (s, d) = schemes("fn run_it(g) { g() }\n");
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["run_it"], "forall a b. fn(fn() / {b} -> a) / {b} -> a");
}

#[test]
fn concrete_effect_lambda_row_is_closed() {
    // A returned concrete-effect lambda infers a MINIMAL closed row {Log} —
    // not a spurious `forall a. ... {Log | a}` from an unclosed tail (4b-2 §2).
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn make_logger() { fn(n) { log(n) } }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["make_logger"], "fn() -> fn(String) / {Log} -> Unit");
}

#[test]
fn relay_lambda_stays_row_polymorphic() {
    // The relay path must be preserved: a function that relays a callback's
    // effects keeps its open, row-polymorphic tail. This is the guard against
    // Fork A over-closing a genuine relay (an error in the opposite direction).
    let src = "fn relay(f, x) { f(x) }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(
        s["relay"],
        "forall a b c. fn(fn(a) / {c} -> b, a) / {c} -> b"
    );
}

#[test]
fn concrete_state_use_names_the_arg_in_the_row() {
    // Slice 4c-2: `f` uses State at String concretely -> the row carries the
    // argument, State(String) (proving the type arg rides in the row).
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               fn f() { set(get() <> \"x\") }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["f"], "fn() / {State(String)} -> Unit");
}

#[test]
fn effect_arg_only_type_var_is_generalized() {
    // Slice 4c-2: `s` is reachable ONLY through the effect argument {State(s)} —
    // not in params or result. Generalizing it REQUIRES free_vars to descend into
    // effect arguments. No `forall` here would mean the descent is missing.
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               fn touch() { let _ = get()  Unit }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["touch"], "forall a. fn() / {State(a)} -> Unit");
}

#[test]
fn relay_plus_own_effect_row_is_the_known_limitation() {
    // Fork C (4b-2 §6): a function that BOTH performs its own effect (Log) AND
    // relays a callback. The MINIMAL row would be `fn(fn(String)/{b}->a, ..)
    // / {Log | b} -> a` — Log on `both`, the callback's row independent (`{b}`).
    //
    // KNOWN LIMITATION (pre-existing 3b, documented not fixed — see the deferred
    // obligation in the effects spec): the shared-ambient inference leaks `Log`
    // onto the *parameter* `f`'s row too, so `f` is over-constrained to `{Log|b}`.
    // This pins the CURRENT (wrong-but-sound) behavior; a future call-site row
    // fix will flip this assertion to the minimal `{b}` form above.
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn both(f, x) { let _ = log(x)  f(x) }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(
        s["both"],
        "forall a b. fn(fn(String) / {Log | b} -> a, String) / {Log | b} -> a"
    );
}

#[test]
fn handle_discharges_the_effect() {
    // A function whose body performs Log but handles it is pure again.
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn safe() { handle log(\"x\") with { Log.log(m) -> resume(Unit) return(r) -> r } }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["safe"], "fn() -> Unit");
}

#[test]
fn resume_arg_must_match_operation_result_type() {
    // Flip.flip : () -> Bool, so `resume` takes a Bool; resuming with an Int is
    // a type error — resume's type is derived from the clause's operation.
    let src = "effect Flip { fn flip() -> Bool }\n\
               fn prog() { handle flip() with { Flip.flip() -> resume(1) } }\n";
    let (_s, d) = schemes(src);
    assert!(
        d.iter().any(|c| c == "E0400"),
        "expected E0400 from resume type mismatch: {d:?}"
    );
}

#[test]
fn effect_propagates_through_a_call() {
    // A function that calls a Log-performing function also performs {Log}.
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn greet(name) { log(name) }\n\
               fn twice() { greet(\"a\") greet(\"b\") }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["twice"], "fn() / {Log} -> Unit");
}

#[test]
fn run_it_instantiates_at_different_effects() {
    // The same row-polymorphic run_it is applied to a Log-performing function
    // AND a pure one — parametric row polymorphism, no subtyping (spec §3.6).
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn run_it(g) { g() }\n\
               fn noisy() { log(\"x\") }\n\
               fn quiet() { 1 }\n\
               fn use_both() { run_it(noisy) run_it(quiet) }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    // use_both performs {Log} (via run_it(noisy)); run_it(quiet) is pure.
    assert_eq!(s["use_both"], "fn() / {Log} -> Int");
}

#[test]
fn handled_effect_in_main_is_clean() {
    // Discharge end-to-end: main handles the only user effect, so nothing
    // escapes — no E0420 (contrast tests/ui/unhandled_effect.elya).
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn greet() { log(\"hi\") }\n\
               pub fn main() {\n\
                 handle greet() with { Log.log(m) -> resume(Unit) return(r) -> r }\n\
               }\n";
    let (_s, d) = schemes(src);
    assert!(
        d.is_empty(),
        "handled effect should type-check clean: {d:?}"
    );
}

// ---- Slice 3d: E0426 cleanup lint + non-fatal warning semantics ----

/// The full diagnostics from inference (not just codes) — for severity/wording.
fn infer_diags(src: &str) -> Vec<Diagnostic> {
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse diags: {pd:?}");
    types::infer(&Session::new(), &m)
}

// A `with multi` handler whose body performs {IO} (io.println after the flip).
const MULTI_OVER_IO: &str = "effect Flip { fn flip() -> Bool }\n\
    fn noisy() { let x = flip()  let _ = io.println(\"tick\")  x }\n\
    pub fn main() {\n\
      let _ = handle noisy() with multi { Flip.flip() -> resume(True) }\n\
      io.println(\"done\")\n\
    }\n";

#[test]
fn multi_over_io_warns_e0426() {
    let diags = infer_diags(MULTI_OVER_IO);
    let w = diags
        .iter()
        .find(|d| d.code == "E0426")
        .expect("expected E0426");
    assert_eq!(w.severity, Severity::Warning, "E0426 must be a warning");
    let sm = SourceMap::new("t.elya", MULTI_OVER_IO);
    let rendered = render(&diags, &sm);
    assert!(rendered.contains("E0426"), "{rendered}");
    assert!(
        rendered.contains("IO"),
        "should name the effect: {rendered}"
    );
    for bad in ["%r", "%e", "%row"] {
        assert!(!rendered.contains(bad), "leaked token {bad}: {rendered}");
    }
}

#[test]
fn multi_over_io_still_compiles_and_runs() {
    // E0426 is a lint: the program type-checks and runs (warning is non-fatal).
    assert!(
        check_source("t.elya", MULTI_OVER_IO).is_ok(),
        "warning must not fail check_source"
    );
    assert_eq!(
        run_source("t.elya", MULTI_OVER_IO).expect("should run"),
        "tick\ndone\n"
    );
}

// A closure that performs {Flip} + {IO}, relayed through `run` and handled by a
// `multi` handler: the observable IO is duplicated across resumes -> E0426. The
// same closure under a one-shot handler must NOT warn (4b-2 §5).
const MULTI_OVER_CLOSURE_IO: &str = "effect Flip { fn flip() -> Bool }\n\
    fn run(f) { f() }\n\
    pub fn main() {\n\
      let _ = handle run(fn() { let x = flip()  let _ = io.println(\"tick\")  x }) with multi { Flip.flip() -> resume(True) }\n\
      io.println(\"done\")\n\
    }\n";

#[test]
fn multi_over_closure_io_warns_e0426() {
    let diags = infer_diags(MULTI_OVER_CLOSURE_IO);
    let w = diags
        .iter()
        .find(|d| d.code == "E0426")
        .expect("expected E0426 for a closure performing IO under multi");
    assert_eq!(w.severity, Severity::Warning, "E0426 must be a warning");
}

#[test]
fn one_shot_over_closure_io_does_not_warn() {
    // Same closure, but a default (one-shot) handler: no duplication, no E0426.
    let src = "effect Flip { fn flip() -> Bool }\n\
        fn run(f) { f() }\n\
        pub fn main() {\n\
          let _ = handle run(fn() { let x = flip()  let _ = io.println(\"tick\")  x }) with { Flip.flip() -> resume(True) }\n\
          io.println(\"done\")\n\
        }\n";
    let diags = infer_diags(src);
    assert!(
        !diags.iter().any(|d| d.code == "E0426"),
        "one-shot handler must not warn E0426: {diags:?}"
    );
}

#[test]
fn severity_partition_is_locked() {
    // Lock the Error/Warning partition against regression: a warning-only program
    // succeeds; an error program still fails.
    assert!(
        check_source("t.elya", MULTI_OVER_IO).is_ok(),
        "warning => success"
    );
    let err_src = "pub fn main() { let _ = 1 + \"a\"\n io.println(\"x\") }\n";
    assert!(check_source("e.elya", err_src).is_err(), "error => failure");
}

#[test]
fn one_shot_over_io_does_not_warn() {
    let src = "effect Flip { fn flip() -> Bool }\n\
        fn noisy() { let x = flip()  let _ = io.println(\"tick\")  x }\n\
        fn prog() { handle noisy() with { Flip.flip() -> resume(True) } }\n";
    assert!(
        !infer_diags(src).iter().any(|d| d.code == "E0426"),
        "one-shot handler must not warn E0426"
    );
}

#[test]
fn multi_pure_body_does_not_warn() {
    // choose() performs {Flip} but no observable IO — no duplication, no E0426.
    let src = "effect Flip { fn flip() -> Bool }\n\
        fn choose() { if flip() { \"a\" } else { \"b\" } }\n\
        fn prog() { handle choose() with multi { Flip.flip() -> resume(True) <> resume(False) } }\n";
    assert!(
        !infer_diags(src).iter().any(|d| d.code == "E0426"),
        "no IO in body => no E0426"
    );
}
