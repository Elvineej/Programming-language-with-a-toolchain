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

#[test]
fn one_generic_state_used_at_int_and_string() {
    // ONE `effect State(s)` used at Int (a counter) AND String (append) in one
    // program. Proves the effect is parametric over its state type — each use
    // instantiates its own `s` (perform/handle instantiation).
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               fn count() {\n\
                 let program = handle { let _ = set(7)  get() } with {\n\
                   State.get() -> fn(st) { (resume(st))(st) }\n\
                   State.set(v) -> fn(st) { (resume(Unit))(v) }\n\
                   return(x) -> fn(st) { x }\n\
                 }\n\
                 program(0)\n\
               }\n\
               fn label() {\n\
                 let program = handle { let _ = set(\"a\")  let x = get()  let _ = set(x <> \"b\")  get() } with {\n\
                   State.get() -> fn(st) { (resume(st))(st) }\n\
                   State.set(v) -> fn(st) { (resume(Unit))(v) }\n\
                   return(x) -> fn(st) { x }\n\
                 }\n\
                 program(\"init\")\n\
               }\n\
               pub fn main() { if count() == 7 { io.println(label()) } else { io.println(\"no\") } }\n";
    assert_eq!(run(src), "ab\n");
}
