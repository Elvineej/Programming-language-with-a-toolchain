//! Literals at the edges, and a layout rule (2026-10-23, spec
//! `docs/superpowers/specs/2026-10-23-elya-literals-and-layout-design.md`).
//!
//! D1: a `-` directly before a numeric literal is part of it, so MIN is a literal.
//! D2: an integer literal out of range is E0002. D3: negative literals are patterns.
//! D4: in braces, a newline before an infix operator or a call's `(` ends the
//! expression; a line starting with a glued `-` or with `(` is a new statement, any
//! other operator starting a line is E0101.
//!
//! Negative controls (each reverted; `src/parse.rs` compared byte for byte after):
//! - K1, the fold off in expressions (D1): `min_is_a_literal` fails with E0002, and
//!   `the_fold_is_on_the_literal` on `- 5`.
//! - K2, the positive bound one too wide (D2): `an_out_of_range_literal_is_e0002`
//!   fails (`9223372036854775808` checks clean).
//! - K3, the layout stop off (`newline_before` never true, D4): l1 fails with E0200
//!   "unresolved name `y`", m6 with E0100, and E0101 is never reported.
//! - K4, the glued-`-` test off (every `-` starting a line is E0101): l1, m6 and the
//!   block in call arguments fail with E0101.

use elya::ast::Expr;
use elya::parse::{parse_expr_str, parse_module};
use elya::Session;

fn errors(src: &str) -> String {
    match elya::check_source("t.elya", src) {
        Ok(()) => String::new(),
        Err(e) => e,
    }
}

/// main's Int, or the failure's message.
fn eval(src: &str) -> Result<i64, String> {
    let e = errors(src);
    assert_eq!(e, "", "the front end rejected:\n{src}");
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "{pd:?}");
    let (_, r) = elya::eval::run_module_keeping(&m, elya::eval::World::Live);
    match r {
        Ok(elya::eval::Value::Int(n)) => Ok(n),
        Ok(v) => panic!("main is not an Int: {v:?}"),
        Err(e) => Err(e.diag.message),
    }
}

fn main_is(body: &str) -> String {
    format!("pub fn main() -> Int {{ {body} }}\n")
}

#[test]
fn min_is_a_literal() {
    assert_eq!(eval(&main_is("-9223372036854775808")), Ok(i64::MIN));
    assert_eq!(
        eval(&main_is("-9223372036854775808 / 1000000000000")),
        Ok(-9223372)
    );
    assert_eq!(eval(&main_is("- 7 * 3")), Ok(-21));
    let e = eval(&main_is("-(-9223372036854775808)")).unwrap_err();
    assert!(e.contains("integer overflow"), "{e}");
}

#[test]
fn an_out_of_range_literal_is_e0002() {
    for src in [
        main_is("9223372036854775808"),
        main_is("99999999999999999999"),
        main_is("-99999999999999999999"),
        main_is("-9223372036854775809"),
        "fn f(x: Int) -> Int { x - 9223372036854775808 }\npub fn main() -> Int { f(1) }\n"
            .to_string(),
        main_is("-(9223372036854775808)"),
        "fn f(x: Int) -> Int { match x { 9223372036854775808 -> 1  _ -> 2 } }\n\
         pub fn main() -> Int { f(1) }\n"
            .to_string(),
    ] {
        let e = errors(&src);
        assert!(e.contains("E0002"), "{src}\n{e}");
        assert!(!e.contains("E0001"), "{src}\n{e}");
        assert!(e.contains("integer literal out of range"), "{e}");
    }
    // The largest literal is still one.
    assert_eq!(eval(&main_is("9223372036854775807")), Ok(i64::MAX));
}

#[test]
fn negative_literals_are_patterns() {
    let m5 = "fn f(x: Int) -> Int { match x { -1 -> 10  _ -> 20 } }\n\
              pub fn main() -> Int { f(0 - 1) * 100 + f(1) }\n";
    assert_eq!(eval(m5), Ok(1020));
    let m6 = "fn f(x: Int) -> Int { match x {\n  0 -> 5\n  -1 -> 7\n  _ -> 9 } }\n\
              pub fn main() -> Int { f(0 - 1) * 100 + f(0) * 10 + f(3) }\n";
    assert_eq!(eval(m6), Ok(759));
    let min = "fn f(x: Int) -> Int { match x { -9223372036854775808 -> 1  _ -> 2 } }\n\
               pub fn main() -> Int { f(0 - 9223372036854775807 - 1) * 10 + f(0) }\n";
    assert_eq!(eval(min), Ok(12));
}

#[test]
fn a_line_starting_with_a_prefix_minus_or_a_paren_is_a_new_statement() {
    let l1 = "fn f(x: Int) -> Int {\n  let y = 4\n  -x * y\n}\npub fn main() -> Int { f(3) }\n";
    assert_eq!(eval(l1), Ok(-12));
    let l2 = "fn g(x: Int) -> Int { x + 1 }\n\
              fn f(x: Int) -> Int {\n  let h = g\n  (h)(x)\n}\npub fn main() -> Int { f(3) }\n";
    assert_eq!(eval(l2), Ok(4));
    // After a block-shaped expression too.
    let l7 = "fn f(x: Int) -> Int {\n  let y = if x > 0 { 1 } else { 2 }\n  -y\n}\n\
              pub fn main() -> Int { f(3) }\n";
    assert_eq!(eval(l7), Ok(-1));
}

#[test]
fn an_operator_starting_a_line_is_e0101() {
    for (line, op) in [
        ("+ 1", "+"),
        ("- 1", "-"),
        ("* 2", "*"),
        ("== 1", "=="),
        ("<> \"a\"", "<>"),
    ] {
        let src = format!(
            "fn f(x: Int) -> Int {{\n  let y = x\n    {line}\n  y\n}}\npub fn main() -> Int {{ f(3) }}\n"
        );
        let e = errors(&src);
        assert!(e.contains("E0101"), "{src}\n{e}");
        assert!(
            e.contains("an operator at the start of a line does not continue the line before"),
            "{e}"
        );
        assert!(
            e.contains(&format!("end the line before with `{op}`")),
            "{e}"
        );
    }
    // In a match arm, as in a block.
    let src = "fn f(b: Bool) -> Bool { match b {\n  True -> b\n    && False\n  False -> b } }\n\
               pub fn main() -> Int { 1 }\n";
    assert!(errors(src).contains("E0101"), "{}", errors(src));
}

#[test]
fn continuations_that_stay() {
    let l4 = "fn f(x: Int) -> Int {\n  let y = x +\n    1\n  y\n}\npub fn main() -> Int { f(3) }\n";
    assert_eq!(eval(l4), Ok(4));
    let l5 = "fn g(a: Int, b: Int) -> Int { a - b }\n\
              fn f(x: Int) -> Int {\n  let y = g(x,\n    1)\n  y\n}\npub fn main() -> Int { f(3) }\n";
    assert_eq!(eval(l5), Ok(2));
    let l6 =
        "fn f(x: Int) -> Int {\n  let y = (x\n    - 1)\n  y\n}\npub fn main() -> Int { f(3) }\n";
    assert_eq!(eval(l6), Ok(2));
    let args = "fn g(a: Int, b: Int) -> Int { a * 10 + b }\n\
                pub fn main() -> Int { g(4\n  - 1,\n  2) }\n";
    assert_eq!(eval(args), Ok(32));
    // A block inside parentheses is braces again.
    let block = "fn id(x: Int) -> Int { x }\n\
                 pub fn main() -> Int { id({ let a = 1\n  -a }) }\n";
    assert_eq!(eval(block), Ok(-1));
}

#[test]
fn the_fold_is_on_the_literal() {
    let parse = |s: &str| {
        let (e, d) = parse_expr_str(&Session::new(), s);
        assert!(d.is_empty(), "{s}: {d:?}");
        e.expect("an expression").node
    };
    assert!(matches!(parse("- 5"), Expr::Int(-5)), "{:?}", parse("- 5"));
    assert!(matches!(parse("-(5)"), Expr::Unary { .. }));
    assert!(matches!(parse("-x"), Expr::Unary { .. }));
    assert!(
        matches!(parse("-1.5"), Expr::Float(x) if x == -1.5),
        "{:?}",
        parse("-1.5")
    );
    // `op.show` writes MIN; pasted back into a program it is MIN.
    let src = "effect Q { fn take(n: Int) -> Int }\n\
               pub fn main() {\n\
                 handle { let _ = take(-9223372036854775808)\n Unit } with any {\n\
                   op(o) -> { io.println(op.show(o))\n resume(op.decode(o, \"0\")) }\n\
                 }\n\
               }\n";
    assert_eq!(errors(src), "");
    let out = elya::run_source("t.elya", src).expect("runs");
    assert_eq!(out, "Q.take(-9223372036854775808)\n");
    let arg = out
        .trim()
        .trim_start_matches("Q.take(")
        .trim_end_matches(')');
    assert_eq!(eval(&main_is(arg)), Ok(i64::MIN));
}

// The review (2026-10-23), each red first.

/// R1: a float literal folds its sign (D1); `-` applied to a float expression is
/// still `Int`'s negation, a type error (the float operators are their own).
#[test]
fn a_negative_float_is_a_literal_and_float_negation_stays_an_error() {
    assert_eq!(errors(&main_is("let y = -1.5  0")), "");
    for body in ["let y = -(1.5)  0", "let x = 1.5  let y = -x  0"] {
        let e = errors(&main_is(body));
        assert!(e.contains("E0400"), "{body}\n{e}");
    }
}

/// R2: an E0101 in a `let` is the only diagnostic (no second error at the same
/// operator from the module level).
#[test]
fn an_e0101_in_a_let_is_one_diagnostic() {
    let src = "pub fn main() -> Int {\n  let y = 1\n    + 1\n  y\n}\n";
    let e = errors(src);
    assert!(e.contains("E0101"), "{e}");
    assert_eq!(e.matches("Error").count(), 1, "{e}");
}

/// R3: inside parentheses a `match` scrutinee and a `handle` body are parentheses
/// (D4); only the arms and the clauses are braces.
#[test]
fn parentheses_keep_a_match_scrutinee_and_a_handle_body_free() {
    let m = "pub fn main() -> Int {\n  let x = (match 1\n    + 1 { 2 -> 5  _ -> 6 })\n  x\n}\n";
    assert_eq!(eval(m), Ok(5));
    let h = "effect E { fn get(x: Int) -> Int }\n\
             pub fn main() -> Int {\n  let x = (handle get(1)\n    + 1 with { E.get(x) -> resume(x * 10) })\n  x\n}\n";
    assert_eq!(eval(h), Ok(11));
}

/// R4: a handler's clauses are no statements, so a glued `-` starting a line there
/// cannot begin one: it is E0101 too.
#[test]
fn a_minus_starting_a_line_in_a_handler_is_e0101() {
    let src = "effect E { fn get(x: Int) -> Int }\n\
               fn h(g) { handle g() with {\n    E.get(x) -> resume(x)\n    return(r) -> r\n      -1\n} }\n\
               pub fn main() -> Int { h(fn() { get(1) }) }\n";
    let e = errors(src);
    assert!(e.contains("E0101"), "{e}");
}
