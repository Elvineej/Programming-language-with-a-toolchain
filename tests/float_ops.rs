//! `Float`, part 2: comparisons, negation and conversions (2026-10-24, spec
//! `docs/superpowers/specs/2026-10-24-elya-float-part-2-design.md`), in the
//! evaluator. `crates/codegen/tests/native_float.rs` holds the differential
//! twins.
//!
//! Negative controls: K4-K6 in `native_float.rs` (natively); in the evaluator
//! each rule is the Rust operation itself.

/// What a program printed, or its error's text.
fn run(src: &str) -> Result<String, String> {
    let o = elya::run_source_outcome("t.elya", src);
    match o.error {
        None => Ok(o.output),
        Some(e) => Err(e),
    }
}

fn prints(body: &str) -> String {
    let src = format!("pub fn main() {{\n{body}\n}}\n");
    run(&src).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

fn b(e: &str) -> String {
    format!("io.println(if {e} {{ \"T\" }} else {{ \"F\" }})")
}

#[test]
fn float_comparisons_are_ieee_ordered() {
    let nan = "(0.0 /. 0.0)";
    let rows = [
        ("1.5 <. 2.0", "T"),
        ("2.0 <. 2.0", "F"),
        ("2.0 <=. 2.0", "T"),
        ("3.0 >. 2.0", "T"),
        ("2.0 >=. 2.5", "F"),
        ("-0.0 <. 0.0", "F"),
        ("-0.0 >=. 0.0", "T"),
        ("(-1.0 /. 0.0) <. -1000000.0", "T"),
    ];
    for (e, want) in rows {
        assert_eq!(prints(&b(e)), format!("{want}\n"), "{e}");
    }
    for op in ["<.", "<=.", ">.", ">=."] {
        assert_eq!(prints(&b(&format!("{nan} {op} 1.0"))), "F\n", "NaN {op}");
        assert_eq!(prints(&b(&format!("1.0 {op} {nan}"))), "F\n", "{op} NaN");
    }
    // `<` stays `Int` only.
    let e = run("pub fn main() -> Int { if 1.0 < 2.0 { 1 } else { 0 } }\n").unwrap_err();
    assert!(e.contains("E0400"), "{e}");
}

#[test]
fn float_negation_flips_the_sign() {
    assert_eq!(prints("io.println(float.to_string(-.0.0))"), "-0.0\n");
    assert_eq!(
        prints("let x = 1.5  io.println(float.to_string(-.x))"),
        "-1.5\n"
    );
    assert_eq!(
        prints("let x = -2.5  io.println(float.to_string(-.x *. 2.0))"),
        "5.0\n"
    );
    assert_eq!(
        prints("io.println(float.to_string(-.(1.0 /. 0.0)))"),
        "-inf\n"
    );
    // A glued `-.` at a line's start starts a statement.
    assert_eq!(
        prints("let x = 4.0\n  let y = x\n  -.y\n  io.println(float.to_string(-.y))"),
        "-4.0\n"
    );
    // `-.` on an Int is a type error.
    let e = run("pub fn main() -> Int { let n = 1  let m = -.n  0 }\n").unwrap_err();
    assert!(e.contains("E0400"), "{e}");
}

#[test]
fn conversions_round_and_fail_by_name() {
    let show = |e: &str| prints(&format!("io.println(float.to_string({e}))"));
    assert_eq!(show("float.of_int(3)"), "3.0\n");
    assert_eq!(show("float.of_int(-7)"), "-7.0\n");
    // 2^53 + 1 rounds to even.
    assert_eq!(
        show("float.of_int(9007199254740993)"),
        "9007199254740992.0\n"
    );
    let int = |e: &str| prints(&format!("io.println(int.to_string({e}))"));
    assert_eq!(int("float.truncate(2.7)"), "2\n");
    assert_eq!(int("float.truncate(-2.7)"), "-2\n");
    assert_eq!(int("float.round(2.5)"), "3\n");
    assert_eq!(int("float.round(-2.5)"), "-3\n");
    assert_eq!(int("float.round(2.4999)"), "2\n");
    assert_eq!(
        int("float.truncate(-9223372036854775808.0)"),
        "-9223372036854775808\n"
    );
    let fails = |e: &str| run(&format!("pub fn main() -> Int {{ {e} }}\n")).unwrap_err();
    let e = fails("float.round(0.0 /. 0.0)");
    assert!(e.contains("float.round: NaN has no Int value"), "{e}");
    let e = fails("float.truncate(9223372036854775808.0)");
    assert!(
        e.contains("float.truncate: 9223372036854776000.0 is outside Int's range"),
        "{e}"
    );
    let e = fails("float.truncate(1.0 /. 0.0)");
    assert!(
        e.contains("float.truncate: inf is outside Int's range"),
        "{e}"
    );
    // The measured programs.
    let newton = "fn abs(x: Float) -> Float { if x <. 0.0 { -.x } else { x } }\n\
                  fn sqrt(a: Float, g: Float) -> Float { let n = (g +. a /. g) /. 2.0  if abs(n -. g) <. 0.000000001 { n } else { sqrt(a, n) } }\n\
                  pub fn main() { io.println(float.to_string(sqrt(2.0, 1.0))) }\n";
    assert_eq!(run(newton).unwrap(), "1.414213562373095\n");
    let mean = "type L { Nil, Cons(Int, L) }\n\
                fn sum(xs: L) -> Int { match xs { Nil -> 0  Cons(x, r) -> x + sum(r) } }\n\
                fn len(xs: L) -> Int { match xs { Nil -> 0  Cons(_, r) -> 1 + len(r) } }\n\
                pub fn main() { let xs = Cons(1, Cons(2, Cons(4, Nil)))  io.println(float.to_string(float.of_int(sum(xs)) /. float.of_int(len(xs)))) }\n";
    assert_eq!(run(mean).unwrap(), "2.3333333333333335\n");
}
