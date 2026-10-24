//! Structural equality (spec
//! `docs/superpowers/specs/2026-10-16-elya-structural-equality-design.md`): `==`
//! and `!=` compare data, structurally; comparing anything that is not data is
//! E0436 at check time, through equality type variables (D1).
//!
//! Negative controls (each reverted, `git diff --quiet` confirmed):
//! - K1: `bind` stops propagating a variable's mark -> the instantiation rows
//!   (`same`, `same2`, the local `let`) check clean and this file's E0436 test
//!   fails on them.

use elya::check_source;
use elya::parse::parse_module;
use elya::Session;

const DECLS: &str = "type L { N, C(Int, L) }\n\
type B { B(fn(Int) -> Int) }\n\
type Box(a) { Box(a) }\n\
type S { S(String, L) }\n\
fn same(a, b) { a == b }\n\
fn same2(a, b) { same(a, b) }\n\
fn inc(x) { x + 1 }\n";

fn value(src: &str) -> i64 {
    if let Err(d) = check_source("t.elya", src) {
        panic!("should check clean: {d}\n{src}");
    }
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "{pd:?}");
    match elya::eval::run_module_value(&m).expect("runs").1 {
        elya::eval::Value::Int(n) => n,
        v => panic!("not an Int: {v:?}"),
    }
}

#[test]
fn comparing_what_is_not_data_is_e0436() {
    let rejected = [
        ("two lambdas", "if (fn(x) { x }) == (fn(x) { x }) { 1 } else { 0 }"),
        ("a closure with itself", "let f = fn(x) { x }  if f == f { 1 } else { 0 }"),
        ("a top-level function as a value", "if inc != inc { 1 } else { 0 }"),
        ("a function in a field", "if B(inc) == B(inc) { 1 } else { 0 }"),
        ("a function as a type argument", "if Box(inc) == Box(inc) { 1 } else { 0 }"),
        ("a generic function at a function type", "if same(inc, inc) { 1 } else { 0 }"),
        ("two levels of generic functions", "if same2(inc, fn(y) { y }) { 1 } else { 0 }"),
        (
            "a generic local `let`",
            "let eq = fn(a, b) { a == b }  if eq(1, 1) { if eq(inc, inc) { 1 } else { 0 } } else { 0 }",
        ),
    ];
    for (what, body) in rejected {
        let src = format!("{DECLS}pub fn main() -> Int {{ {body} }}\n");
        let err = match check_source("t.elya", &src) {
            Ok(()) => panic!("{what}: checked clean\n{src}"),
            Err(d) => d.to_string(),
        };
        assert!(err.contains("E0436"), "{what}: {err}");
        assert!(err.contains("only data can be compared"), "{what}: {err}");
    }
    // Equality type variables stay generic: `same` at every data type.
    let accepted = "pub fn main() -> Int {\n\
         \x20 let a = if same(1, 1) { 1 } else { 0 }\n\
         \x20 let b = if same2(C(1, N), C(1, C(2, N))) { 0 } else { 10 }\n\
         \x20 let c = if same(\"x\", \"x\") { 100 } else { 0 }\n\
         \x20 let d = if same(Box(S(\"s\", N)), Box(S(\"s\", N))) { 1000 } else { 0 }\n\
         \x20 let e = if Unit == Unit { 10000 } else { 0 }\n\
         \x20 a + b + c + d + e\n\
         }\n";
    assert_eq!(value(&format!("{DECLS}{accepted}")), 11111);
}

#[test]
fn the_evaluator_compares_long_lists_iteratively() {
    // Measured before the slice: a host stack overflow (the derived comparison
    // recursed once per cell).
    let src = format!(
        "{DECLS}fn build(n, acc) {{ if n == 0 {{ acc }} else {{ build(n - 1, C(n, acc)) }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let x = build(1000000, N)\n\
         \x20 let y = build(1000000, N)\n\
         \x20 let z = build(1000000, C(7, N))\n\
         \x20 let r = if x == y {{ 1 }} else {{ 0 }}\n\
         \x20 r + (if x != z {{ 10 }} else {{ 0 }})\n\
         }}\n"
    );
    assert_eq!(value(&src), 11);
}

/// The review: an E0436 found when a marked variable is bound far from its
/// comparison now labels that comparison too.
#[test]
fn the_error_names_the_comparison_it_reaches() {
    let src = "fn f(x) { let r = x == x  x(1) }\npub fn main() -> Int { f(fn(y) { y }) }\n";
    let err = check_source("t.elya", src)
        .expect_err("must be rejected")
        .to_string();
    assert!(err.contains("E0436"), "{err}");
    assert!(err.contains("the comparison it reaches"), "{err}");
}

/// The review: whether two values are shared must not decide `==`. A NaN is
/// not equal to itself, in a field as alone (the derived comparison answered
/// so; a pointer shortcut briefly did not).
#[test]
fn sharing_never_decides_equality() {
    let src = "type F { F(Float) }\n\
               pub fn main() -> Int {\n\
               \x20 let n = 0.0 /. 0.0\n\
               \x20 let x = F(n)\n\
               \x20 let a = if n == n { 1 } else { 0 }\n\
               \x20 let b = if x == x { 10 } else { 0 }\n\
               \x20 let c = if F(1.0) == F(1.0) { 100 } else { 0 }\n\
               \x20 a + b + c\n\
               }\n";
    assert_eq!(value(src), 100);
}
