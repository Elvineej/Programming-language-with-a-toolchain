//! Slice 3c: output-verified execution of effect programs on the CEK machine.
//!
//! The tree-walker recurses on the host stack and CANNOT capture continuations,
//! so it is NOT an oracle for effect programs (spec §4.6). The Slice-2 `cek ==
//! tree` cross-check therefore stays effect-*free* (retained, unchanged, still
//! green), and THIS corpus is the sole safety net for effect-program
//! correctness — so it is built per handler *behavior*, and every case asserts a
//! concrete expected output, not mere absence of a crash:
//!
//!   (a) non-resuming / Exn-style   -> `non_resuming_clause_is_exception_like`
//!   (b) one-shot resume            -> `one_shot_resume_passes_a_value_back`
//!   (d) nested / innermost-match   -> `nested_handlers_match_innermost`
//!   (e) tail-resumptive            -> `tail_resumptive_loop_runs`
//!   plus: handle-over-pure-body, and the one-shot E0425 negative test.
//!
//! (c) multi-shot re-invocation is deliberately NOT covered here — it lands in
//! Slice 3d (output-verified). Its absence is intentional, not an oversight.

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

/// A program that type-checks but fails at runtime — returns the rendered error.
fn run_err(src: &str) -> String {
    assert!(
        elya::check_source("t.elya", src).is_ok(),
        "should type-check: {:?}",
        elya::check_source("t.elya", src)
    );
    run_source("t.elya", src).expect_err("should fail at runtime")
}

#[test]
fn one_shot_resume_passes_a_value_back() {
    // (b) one-shot resume: the operation `ask` is resumed once with "ada", so
    // `ask()` yields "ada" and `greet` returns "hi ada".
    let src = "effect Ask {\n\
               \x20 fn ask() -> String\n\
               }\n\
               fn greet() { \"hi \" <> ask() }\n\
               pub fn main() {\n\
               \x20 io.println(handle greet() with {\n\
               \x20   Ask.ask() -> resume(\"ada\")\n\
               \x20   return(x) -> x\n\
               \x20 })\n\
               }\n";
    assert_eq!(run(src), "hi ada\n");
}

#[test]
fn non_resuming_clause_is_exception_like() {
    // (a) non-resuming / Exn-style: `fail` performs; its clause never resumes,
    // so the captured continuation is dropped and the clause value is the result.
    let src = "effect Exn {\n\
               \x20 fn fail() -> String\n\
               }\n\
               fn risky(b) { if b == 0 { fail() } else { \"ok\" } }\n\
               pub fn main() {\n\
               \x20 io.println(handle risky(0) with {\n\
               \x20   Exn.fail() -> \"caught\"\n\
               \x20   return(x) -> x\n\
               \x20 })\n\
               }\n";
    assert_eq!(run(src), "caught\n");
}

#[test]
fn nested_handlers_match_innermost() {
    // (d) nested handlers: `a` is caught by the inner handler, `b` propagates
    // past it to the outer one — correct innermost-matching with deep handlers.
    let src = "effect A {\n\
               \x20 fn a() -> String\n\
               }\n\
               effect B {\n\
               \x20 fn b() -> String\n\
               }\n\
               fn both() { a() <> b() }\n\
               pub fn main() {\n\
               \x20 io.println(handle (handle both() with { A.a() -> resume(\"[a]\") }) with {\n\
               \x20   B.b() -> resume(\"[b]\")\n\
               \x20 })\n\
               }\n";
    assert_eq!(run(src), "[a][b]\n");
}

#[test]
fn tail_resumptive_loop_runs() {
    // (e) tail-resumptive: the clause body IS `resume(...)` (tail position),
    // driven by a small recursion. (Bounded-depth is asserted in 3e; 3c checks
    // only that the output is correct.)
    let src = "effect Gen {\n\
               \x20 fn yield_() -> Bool\n\
               }\n\
               fn run(n) { if n == 0 { \"end\" } else { if yield_() { run(n - 1) } else { \"stop\" } } }\n\
               pub fn main() {\n\
               \x20 io.println(handle run(3) with { Gen.yield_() -> resume(True) })\n\
               }\n";
    assert_eq!(run(src), "end\n");
}

#[test]
fn resuming_twice_in_a_one_shot_handler_is_e0425() {
    // One-shot enforcement: a non-`multi` clause that resumes twice in
    // unconditional sequence is a runtime E0425 on the second resume.
    let src = "effect Twice {\n\
               \x20 fn t() -> Int\n\
               }\n\
               fn body() { t() }\n\
               pub fn main() {\n\
               \x20 let _ = handle body() with { Twice.t() -> { let _ = resume(1)  resume(2) } }\n\
               \x20 io.println(\"unreached\")\n\
               }\n";
    let err = run_err(src);
    assert!(err.contains("E0425"), "expected E0425, got: {err}");
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
