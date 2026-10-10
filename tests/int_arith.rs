//! `Int` arithmetic is exact or fails by name (spec
//! `docs/superpowers/specs/2026-10-10-elya-checked-integer-arithmetic-design.md`, D1/D2).
//! Both evaluators share `apply_binop`/`apply_unop`; each test runs both.
//!
//! Negative control K3 (spec §2): `checked_mul` back to `*` makes
//! `integer_overflow_is_a_named_error` panic on `MAX * 2`. Applied, observed, reverted.

use elya::parse::parse_module;
use elya::Session;

const MIN: &str = "(0 - 9223372036854775807 - 1)";
const MAX: &str = "9223372036854775807";

fn module(src: &str) -> elya::ast::Module {
    elya::check_source("int.elya", src).unwrap_or_else(|e| panic!("check: {e}\n{src}"));
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    m
}

/// The runtime error message from both evaluators, which must agree.
fn both_fail(src: &str) -> String {
    let m = module(src);
    let cek = elya::eval::run_module_value(&m)
        .err()
        .unwrap_or_else(|| panic!("the CEK machine ran: {src}"));
    let tree = elya::eval::run_module_tree(&m)
        .err()
        .unwrap_or_else(|| panic!("the tree-walker ran: {src}"));
    assert_eq!(cek.diag.code, "E0300", "{src}");
    assert_eq!(cek.diag.message, tree.diag.message, "{src}");
    cek.diag.message
}

/// What `main` printed, on both evaluators (which must agree).
fn both_print(src: &str) -> String {
    let m = module(src);
    let cek = elya::eval::run_module(&m).unwrap_or_else(|e| panic!("{src}: {e:?}"));
    let tree = elya::eval::run_module_tree(&m).unwrap_or_else(|e| panic!("{src}: {e:?}"));
    assert_eq!(cek.output(), tree.output(), "{src}");
    cek.output().to_string()
}

fn is(expr: &str, expected: &str) -> String {
    format!(
        "pub fn main() {{ if {expr} == {expected} {{ io.println(\"yes\") }} else {{ io.println(\"no\") }} }}\n"
    )
}

#[test]
fn integer_overflow_is_a_named_error() {
    for expr in [
        format!("{MAX} + 1"),
        format!("{MIN} - 1"),
        format!("{MAX} * 2"),
        format!("{MIN} * (0 - 1)"),
        format!("{MIN} / (0 - 1)"),
        format!("-{MIN}"),
    ] {
        let src = format!("pub fn main() -> Int {{ {expr} }}\n");
        assert_eq!(both_fail(&src), "integer overflow", "{expr}");
    }
}

#[test]
fn min_remainder_minus_one_is_zero() {
    assert_eq!(both_print(&is(&format!("{MIN} % (0 - 1)"), "0")), "yes\n");
}

#[test]
fn division_truncates_toward_zero() {
    for (expr, v) in [
        ("(0 - 7) / 2", "(0 - 3)"),
        ("(0 - 7) % 2", "(0 - 1)"),
        ("7 % (0 - 2)", "1"),
        ("7 / 2", "3"),
        ("(0 - 7) / (0 - 2)", "3"),
        ("(0 - 7) % (0 - 2)", "(0 - 1)"),
        (&format!("{MAX} + {MIN}"), "(0 - 1)"),
    ] {
        assert_eq!(both_print(&is(expr, v)), "yes\n", "{expr}");
    }
}

#[test]
fn a_zero_divisor_is_named() {
    assert_eq!(
        both_fail("pub fn main() -> Int { 7 / 0 }\n"),
        "division by zero"
    );
    assert_eq!(
        both_fail("pub fn main() -> Int { 7 % 0 }\n"),
        "remainder by zero"
    );
}
