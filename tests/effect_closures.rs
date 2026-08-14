//! Slice 4b-2: effect-carrying closures relayed through higher-order functions.
//! CEK-only (the tree-walker does not evaluate effects), so every case is
//! output-verified — a concrete program output or diagnostic — because there is
//! no cek==tree oracle behind effect evaluation.

use elya::{check_source, run_source, Session};

/// Type-check (must be clean), then run on the CEK; return the output.
fn run(src: &str) -> String {
    assert!(
        check_source("t.elya", src).is_ok(),
        "should type-check: {:?}",
        check_source("t.elya", src)
    );
    let _ = Session::new();
    run_source("t.elya", src).expect("should run")
}

/// The rendered compile error for a program that must fail the front end.
fn check_err(src: &str) -> String {
    check_source("t.elya", src).expect_err("should fail to type-check")
}

#[test]
fn effectful_closure_relayed_and_handled_at_call_site() {
    // `apply` relays the closure's Log; the handler at the call site resumes
    // log("hi") with "hi!", which flows back through the closure and `apply`.
    let src = "effect Log { fn log(msg: String) -> String }\n\
               fn apply(f, x) { f(x) }\n\
               pub fn main() {\n\
                 let r = handle {\n\
                   apply(fn(n) { log(n) }, \"hi\")\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \"!\")\n\
                   return(x) -> x\n\
                 }\n\
                 io.println(r)\n\
               }\n";
    assert_eq!(run(src), "hi!\n");
}

#[test]
fn unhandled_relayed_effect_is_e0420() {
    // No handler anywhere: the relayed Log survives to main and is E0420.
    let src = "effect Log { fn log(msg: String) -> String }\n\
               fn apply(f, x) { f(x) }\n\
               pub fn main() { let _ = apply(fn(n) { log(n) }, \"hi\")\n io.println(\"x\") }\n";
    let err = check_err(src);
    assert!(
        err.contains("E0420"),
        "expected E0420 (unhandled effect): {err}"
    );
    assert!(err.contains("Log"), "should name the effect: {err}");
}

#[test]
fn let_bound_hof_lambda_is_row_polymorphic() {
    // The earn-out: a LET-BOUND `apply` (non-top-level) used at a pure row AND
    // at {Log} in one program. Only type-checks if its row var is generalized.
    let src = "effect Log { fn log(msg: String) -> String }\n\
               pub fn main() {\n\
                 let apply = fn(f, x) { f(x) }\n\
                 let a = apply(fn(n) { n + 1 }, 10)\n\
                 let r = handle {\n\
                   apply(fn(n) { log(n) }, \"hi\")\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \"!\")\n\
                   return(x) -> x\n\
                 }\n\
                 if a == 11 { io.println(r) } else { io.println(\"no\") }\n\
               }\n";
    assert_eq!(run(src), "hi!\n");
}

#[test]
fn closure_performs_against_call_site_handler_not_definition() {
    // `g` is DEFINED in main with no handler around it, then CALLED inside
    // `call_it`'s handle. Its Log resolves to the call-site handler (dynamic
    // scoping), which appends "-A" — so the output is "inner-A".
    let src = "effect Log { fn log(msg: String) -> String }\n\
               fn call_it(f) {\n\
                 handle { f(\"inner\") } with {\n\
                   Log.log(m) -> resume(m <> \"-A\")\n\
                   return(x) -> x\n\
                 }\n\
               }\n\
               pub fn main() {\n\
                 let g = fn(n) { log(n) }\n\
                 io.println(call_it(g))\n\
               }\n";
    assert_eq!(run(src), "inner-A\n");
}

#[test]
fn relay_plus_own_effect_runs_when_callback_also_performs() {
    // Fork C (4b-2 §6): a relay-plus-own-effect fn works when the callback
    // performs the same effect. `both` logs "own" (discarded) then relays
    // f("arg") = log("arg") -> resume("arg.") -> flows back out.
    let src = "effect Log { fn log(msg: String) -> String }\n\
               fn both(f, x) { let _ = log(\"own\")  f(x) }\n\
               pub fn main() {\n\
                 let r = handle {\n\
                   both(fn(n) { log(n) }, \"arg\")\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \".\")\n\
                   return(x) -> x\n\
                 }\n\
                 io.println(r)\n\
               }\n";
    assert_eq!(run(src), "arg.\n");
}

#[test]
fn relay_plus_own_effect_rejects_pure_callback_known_limitation() {
    // KNOWN LIMITATION (pre-existing 3b, documented not fixed): because the
    // shared ambient leaks `Log` onto the callback's row (see
    // `relay_plus_own_effect_row_is_the_known_limitation` in effect_types.rs),
    // `both` wrongly requires its callback to ALSO perform `Log`. A pure
    // callback is therefore rejected with E0423, even though it is valid (`both`
    // performs `Log` regardless of `f`). This pins the current wrong-but-sound
    // behavior; the deferred call-site row fix will make this program compile.
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn both(f, x) { let _ = log(x)  f(x) }\n\
               pub fn main() {\n\
                 let r = handle {\n\
                   let _ = both(fn(n) { n }, \"hi\")\n\
                   \"ok\"\n\
                 } with {\n\
                   Log.log(m) -> resume(Unit)\n\
                   return(x) -> x\n\
                 }\n\
                 io.println(r)\n\
               }\n";
    let err = check_err(src);
    assert!(
        err.contains("E0423"),
        "known limitation: pure callback rejected by row leak: {err}"
    );
}
