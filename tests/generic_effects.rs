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

fn check_err(src: &str) -> String {
    check_source("t.elya", src).expect_err("should fail to type-check")
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

#[test]
fn same_effect_at_two_types_in_one_scope_is_e0423() {
    // `fs` performs State(String), `fi` performs State(Int); composed in one
    // computation the row carries State at two types -> E0423. A name-only row
    // would silently accept this — the test proves the type arg rides in the row.
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               fn fs() { set(get() <> \"x\") }\n\
               fn fi() { set(get() + 1) }\n\
               pub fn main() {\n\
                 let _ = handle { let _ = fs()  fi() } with {\n\
                   State.get() -> fn(st) { (resume(st))(st) }\n\
                   State.set(v) -> fn(st) { (resume(Unit))(v) }\n\
                   return(x) -> fn(st) { x }\n\
                 }\n\
                 io.println(\"x\")\n\
               }\n";
    let err = check_err(src);
    assert!(
        err.contains("E0423"),
        "conflicting State instantiations must be a row mismatch: {err}"
    );
}
