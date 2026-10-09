//! Slice 4b-3: the closure-arc row capstone. Proves the value restriction keeps a
//! non-value's EFFECT ROW monomorphic (two-sided teeth), and that row polymorphism
//! composes through recursive combinators. CEK-only + output-verified: every case
//! pins a concrete output or a specific diagnostic code, because there is no
//! cek==tree oracle behind effect evaluation.

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
fn value_bound_relay_is_row_polymorphic() {
    // POSITIVE teeth: `g` is a lambda (a VALUE), so its effect-row variable is
    // generalized -> row-polymorphic -> usable at a pure row AND at {Log} in one
    // program. Both thunks return String (so only the ROW varies across uses).
    let src = "effect Log { fn log(msg: String) -> String }\n\
               pub fn main() {\n\
                 let g = fn(thunk) { thunk() }\n\
                 let a = g(fn() { \"pure\" })\n\
                 let r = handle {\n\
                   g(fn() { log(\"hi\") })\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \"!\")\n\
                   return(x) -> x\n\
                 }\n\
                 if a == \"pure\" { io.println(r) } else { io.println(\"no\") }\n\
               }\n";
    assert_eq!(run(src), "hi!\n");
}

#[test]
fn nonvalue_bound_relay_row_stays_monomorphic() {
    // NEGATIVE teeth (the guard): the SAME program, but `g`'s RHS is a CALL
    // (`make_relay()`) — a non-value. The value restriction must NOT generalize
    // its row, so `g` is monomorphic: the first use fixes the row to pure, the
    // second use needs {Log} and conflicts.
    //
    // ROW-DIMENSION ISOLATION: both thunks return String, so `g`'s result type
    // unifies cleanly (no E0400). The sole conflict is the effect row at the
    // SECOND use, so the diagnostic must be E0423 (effect-row mismatch) — that is
    // the proof the gate holds the ROW dimension specifically.
    let src = "effect Log { fn log(msg: String) -> String }\n\
               fn make_relay() { fn(thunk) { thunk() } }\n\
               pub fn main() {\n\
                 let g = make_relay()\n\
                 let a = g(fn() { \"pure\" })\n\
                 let r = handle {\n\
                   g(fn() { log(\"hi\") })\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \"!\")\n\
                   return(x) -> x\n\
                 }\n\
                 if a == \"pure\" { io.println(r) } else { io.println(\"no\") }\n\
               }\n";
    let err = check_err(src);
    // Since sub-effecting (2026-10-08) the pure thunk fits the shared row, so
    // the conflict surfaces one step later: the monomorphic row, shared by
    // both uses, carries Log into `main` (E0420). Either way the guard holds:
    // the row was NOT generalized. Approved expected-value change (E0423 ->
    // E0423 or E0420).
    assert!(
        err.contains("E0423") || err.contains("E0420"),
        "must be a ROW error (the row is not generalized): {err}"
    );
    assert!(
        !err.contains("E0400"),
        "types must unify cleanly — the sole conflict is the effect row: {err}"
    );
}

#[test]
fn map_relays_effectful_callback() {
    // Row-poly `map` relays the callback's Log through recursion; each element is
    // logged and resumed with "!", producing ["a!","b!","c!"] -> "a!b!c!".
    let src = "effect Log { fn log(msg: String) -> String }\n\
               type List(a) { Nil, Cons(a, List(a)) }\n\
               fn map(xs, f) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
               fn concat_all(xs) { match xs { Nil -> \"\"  Cons(h, t) -> h <> concat_all(t) } }\n\
               pub fn main() {\n\
                 let xs = Cons(\"a\", Cons(\"b\", Cons(\"c\", Nil)))\n\
                 let ys = handle {\n\
                   map(xs, fn(s) { log(s) })\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \"!\")\n\
                   return(x) -> x\n\
                 }\n\
                 io.println(concat_all(ys))\n\
               }\n";
    assert_eq!(run(src), "a!b!c!\n");
}

#[test]
fn fold_relays_effectful_callback() {
    // Row-poly `fold` relays the callback's Log while accumulating; each element
    // is logged and resumed with ".", producing "a.b.c.".
    let src = "effect Log { fn log(msg: String) -> String }\n\
               type List(a) { Nil, Cons(a, List(a)) }\n\
               fn fold(xs, acc, f) { match xs { Nil -> acc  Cons(h, t) -> fold(t, f(acc, h), f) } }\n\
               pub fn main() {\n\
                 let xs = Cons(\"a\", Cons(\"b\", Cons(\"c\", Nil)))\n\
                 let total = handle {\n\
                   fold(xs, \"\", fn(acc, s) { acc <> log(s) })\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \".\")\n\
                   return(x) -> x\n\
                 }\n\
                 io.println(total)\n\
               }\n";
    assert_eq!(run(src), "a.b.c.\n");
}
