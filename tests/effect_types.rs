//! Slice 3b: inferred effect rows in top-level function schemes. Pins the
//! printed form of ambient-row inference — concrete-effect functions get a
//! closed row, a pure relay is row-polymorphic, pure functions show no row.

use elya::{parse::parse_module, types::infer_schemes, Session};
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
fn effect_propagates_through_a_call() {
    // A function that calls a Log-performing function also performs {Log}.
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn greet(name) { log(name) }\n\
               fn twice() { greet(\"a\") greet(\"b\") }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["twice"], "fn() / {Log} -> Unit");
}
