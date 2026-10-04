//! Slice 5c-1: handler clause resolution. Op names are unique per module
//! (E0202), a clause must name a declared operation of the effect it names
//! (E0203), and an unqualified clause means its op's effect. Programs m1-m7 are
//! the spec's measured table (§0); each test notes what the build before this
//! slice did with it.

//!
//! Negative controls (plan Task 5), each reverted, each failing differently:
//! - C1, E0202 switched off: m7 falls back to its old E0400; m2 is caught a
//!   second time as E0203 ("`ping` is an operation of `B`, not `A`"), not the
//!   old E0420 -- the overwrite makes the clause check its backstop.
//! - C2, E0203's wrong-qualifier case off: only m3 fails, back to E0420.
//! - C3, the evaluator's strict `Some(effect)` match restored: both value tests
//!   fail, and natively only the differential (the evaluator side errors).
//! - C4, lowering's op_effects fallback removed: the core test fails, and both
//!   native corpus tests fail on the `unqualified-clause` row with the refusal;
//!   every front-end test still passes.

use elya::eval::{run_module_value, Value};
use elya::parse::parse_module;
use elya::types::infer_schemes;
use elya::{check_source, resolve, Session};

/// The front end's diagnostic codes, in the order `front_end` produces them:
/// resolve first, and inference only when resolve is clean.
fn codes(src: &str) -> Vec<String> {
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let rd = resolve::check(&session, &m);
    let diags = if rd.is_empty() {
        infer_schemes(&session, &m).1
    } else {
        rd
    };
    diags.into_iter().map(|d| d.code).collect()
}

fn rendered_error(src: &str) -> String {
    check_source("t.elya", src).expect_err("must be rejected")
}

fn value(src: &str) -> Value {
    assert!(codes(src).is_empty(), "must check clean: {:?}", codes(src));
    let (m, _) = parse_module(&Session::new(), src);
    run_module_value(&m).expect("evaluates").1
}

const HANDLE_ONE: &str = "fn one() { ask() }\n\
                          pub fn main() -> Int {\n\
                          \x20 handle { one() } with {\n\
                          \x20   CLAUSE\n\
                          \x20   return(x) -> x\n\
                          \x20 }\n\
                          }\n";

fn ask_program(decls: &str, clause: &str) -> String {
    format!("{decls}{}", HANDLE_ONE.replace("CLAUSE", clause))
}

// ---- Task 1: E0202 ----

#[test]
fn an_op_declared_by_two_effects_is_e0202() {
    // m2. Before: [E0420] "effect `B` is never handled" -- B's `ping` silently
    // replaced A's in every op index.
    let src = "effect A { fn ping() -> Int }\n\
               effect B { fn ping() -> Int }\n\
               fn one() { ping() }\n\
               pub fn main() -> Int {\n\
               \x20 handle { one() } with {\n\
               \x20   A.ping() -> resume(1)\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               }\n";
    assert_eq!(codes(src), ["E0202"]);
    let r = rendered_error(src);
    assert!(
        r.contains("operation `ping` is declared more than once"),
        "{r}"
    );
    assert!(r.contains("already declared by effect `A`"), "{r}");
}

#[test]
fn an_op_declared_twice_in_one_effect_is_e0202() {
    // m7. Before: [E0400, E0400] type mismatches.
    let src = ask_program(
        "effect Ask { fn ask() -> Int  fn ask() -> Bool }\n",
        "Ask.ask() -> resume(2)",
    );
    assert_eq!(codes(&src), ["E0202"]);
    assert!(rendered_error(&src).contains("already declared by effect `Ask`"));
}

// ---- Task 2: E0203 ----

#[test]
fn a_clause_naming_the_wrong_effect_is_e0203() {
    // m3. Before: [E0420] "effect `Ask` is never handled".
    let src = ask_program(
        "effect Ask { fn ask() -> Int }\neffect Other { fn other() -> Int }\n",
        "Other.ask() -> resume(2)",
    );
    assert_eq!(codes(&src), ["E0203"]);
    let r = rendered_error(&src);
    assert!(
        r.contains("`ask` is an operation of `Ask`, not `Other`"),
        "{r}"
    );
    assert!(r.contains("write `Ask.ask(…)`, or just `ask(…)`"), "{r}");
}

#[test]
fn a_clause_naming_an_undeclared_effect_is_e0203() {
    // m4. Before: [E0420] "effect `Ask` is never handled".
    let src = ask_program(
        "effect Ask { fn ask() -> Int }\n",
        "Nope.ask() -> resume(2)",
    );
    assert_eq!(codes(&src), ["E0203"]);
    let r = rendered_error(&src);
    assert!(r.contains("`Nope` is not a declared effect"), "{r}");
}

#[test]
fn a_clause_for_an_undeclared_op_is_e0203() {
    // m6. Before: accepted, the clause silently dead.
    let src = ask_program(
        "effect Ask { fn ask() -> Int }\n",
        "Ask.ask() -> resume(2)\n    Ask.nope() -> resume(3)",
    );
    assert_eq!(codes(&src), ["E0203"]);
    let r = rendered_error(&src);
    assert!(r.contains("no effect declares an operation `nope`"), "{r}");
}

// ---- Task 3: the evaluator ----

#[test]
fn an_unqualified_clause_runs_in_the_evaluator() {
    // m1. Before: checked clean, then E0300 "unhandled effect `ask` reached the
    // machine" at run time.
    let src = ask_program("effect Ask { fn ask() -> Int }\n", "ask() -> resume(2)");
    assert_eq!(value(&src), Value::Int(2));
}

#[test]
fn a_handler_mixing_qualified_and_unqualified_clauses_runs() {
    // Before: E0300 on `b`. 14 = 10 + 4; 20 would mean `b` took `a`'s clause.
    let src = "effect Two { fn a() -> Int  fn b() -> Int }\n\
               fn both() { a() + b() }\n\
               pub fn main() -> Int {\n\
               \x20 handle { both() } with {\n\
               \x20   Two.a() -> resume(10)\n\
               \x20   b() -> resume(4)\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               }\n";
    assert_eq!(value(src), Value::Int(14));
}

#[test]
fn an_unqualified_handler_over_two_effects_is_still_e0423() {
    // Guard: resolving bare clauses through the op's effect must not let one
    // handler cover two effects. Green before and after this slice.
    let src = "effect A { fn a() -> Int }\n\
               effect B { fn b() -> Int }\n\
               fn both() { a() + b() }\n\
               pub fn main() -> Int {\n\
               \x20 handle { both() } with {\n\
               \x20   a() -> resume(1)\n\
               \x20   b() -> resume(2)\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               }\n";
    assert!(
        codes(src).contains(&"E0423".to_string()),
        "{:?}",
        codes(src)
    );
}

// ---- Review follow-ups ----

#[test]
fn a_duplicate_op_does_not_cascade_into_a_false_e0203() {
    // Review: with `ping` declared by A and B, the clause `B.ping` was ALSO
    // reported as E0203 "`ping` is an operation of `A`, not `B`" -- false, B
    // declares it. The E0202 is the whole story.
    let src = "effect A { fn ping() -> Int }\n\
               effect B { fn ping() -> Int }\n\
               fn one() { ping() }\n\
               pub fn main() -> Int {\n\
               \x20 handle { one() } with {\n\
               \x20   B.ping() -> resume(1)\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               }\n";
    assert_eq!(codes(src), ["E0202"]);
}

#[test]
fn a_clause_binding_the_wrong_number_of_arguments_is_e0203() {
    // Review (pre-existing, reachable through either spelling): `ask` takes no
    // arguments, the clause binds one. Before: `check` ok, the evaluator failed
    // with "unbound variable `x`", and the native binary segfaulted.
    for clause in ["ask(x) -> resume(x)", "Ask.ask(x) -> resume(x)"] {
        let src = ask_program("effect Ask { fn ask() -> Int }\n", clause);
        assert_eq!(codes(&src), ["E0203"], "{clause}");
        let r = rendered_error(&src);
        assert!(
            r.contains("`ask` takes 0 arguments, but this clause binds 1"),
            "{clause}: {r}"
        );
        assert!(
            !r.contains("write `Ask.ask"),
            "{clause}: no qualifier fix-it: {r}"
        );
    }
}
