//! Slice 3c: output-verified execution of effect programs on the CEK machine.
//!
//! The tree-walker recurses on the host stack and CANNOT capture continuations,
//! so it is NOT an oracle for effect programs (spec §4.6). This corpus is the
//! sole safety net for effect-program correctness, so it is built per handler
//! *behavior* — one clause per distinct semantics — and every case asserts a
//! concrete expected output, not mere absence of a crash.

use elya::{run_source, Session};

fn run(src: &str) -> String {
    // Front-end must be clean (types check effects since 3b), then the machine runs.
    assert!(
        elya::check_source("t.elya", src).is_ok(),
        "should type-check: {:?}",
        elya::check_source("t.elya", src)
    );
    let _ = Session::new();
    run_source("t.elya", src).expect("should run")
}

#[test]
fn handle_over_pure_body_applies_return_clause() {
    // The body performs no operation; the handler discharges and the return
    // clause transforms the body's value (here, identity).
    let src = "effect Log {\n\
               \x20 fn log(msg: String) -> Unit\n\
               }\n\
               pub fn main() {\n\
               \x20 io.println(handle \"hi\" with {\n\
               \x20   Log.log(m) -> resume(Unit)\n\
               \x20   return(x) -> x\n\
               \x20 })\n\
               }\n";
    assert_eq!(run(src), "hi\n");
}
