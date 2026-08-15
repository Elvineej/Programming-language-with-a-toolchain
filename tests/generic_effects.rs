//! Slice 4c-2: generic (parametric) effects — `effect State(s)` usable at any
//! state type. CEK-only + output-verified for runs; `E0423` for a conflicting
//! instantiation. There is no cek==tree oracle behind effect evaluation, so every
//! case pins a concrete output or a specific diagnostic code.

use elya::{check_source, run_source, Session};

fn run(src: &str) -> String {
    assert!(
        check_source("t.elya", src).is_ok(),
        "should type-check: {:?}",
        check_source("t.elya", src)
    );
    let _ = Session::new();
    run_source("t.elya", src).expect("should run")
}

#[test]
fn generic_effect_declaration_is_accepted() {
    // Declaring `effect State(s)` no longer trips E0404; a program that only
    // declares it (and does something unrelated) type-checks and runs.
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               pub fn main() { io.println(\"ok\") }\n";
    assert_eq!(run(src), "ok\n");
}
