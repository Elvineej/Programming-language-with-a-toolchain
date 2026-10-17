//! Unary operators in Core, and explicit wrapping and bit arithmetic (2026-10-17,
//! spec `docs/superpowers/specs/2026-10-17-elya-unary-and-wrapping-design.md`).

use elya::parse::parse_module;
use elya::Session;

fn errors(src: &str) -> String {
    match elya::check_source("t.elya", src) {
        Ok(()) => String::new(),
        Err(e) => e,
    }
}

/// main's Int, or the failure's message.
fn eval(src: &str) -> Result<i64, String> {
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "{pd:?}");
    let (_, r) = elya::eval::run_module_keeping(&m, elya::eval::World::Live);
    match r {
        Ok(elya::eval::Value::Int(n)) => Ok(n),
        Ok(v) => panic!("main is not an Int: {v:?}"),
        Err(e) => Err(e.diag.message),
    }
}

const BUILTINS: &[&str] = &[
    "wrapping_add",
    "wrapping_sub",
    "wrapping_mul",
    "and",
    "or",
    "xor",
    "shl",
    "shr",
    "ushr",
];

#[test]
fn the_builtins_are_typed() {
    for b in BUILTINS {
        let ok = format!(
            "fn f(a: Int, b: Int) -> Int {{ int.{b}(a, b) }}\npub fn main() -> Int {{ f(1, 2) }}\n"
        );
        assert_eq!(errors(&ok), "", "{b}");
        for bad in [
            format!("pub fn main() -> Int {{ int.{b}(1, True) }}\n"),
            format!("pub fn main() -> String {{ int.{b}(1, 2) }}\n"),
            format!("pub fn main() -> Int {{ int.{b}(1) }}\n"),
        ] {
            assert!(!errors(&bad).is_empty(), "{b}: {bad} checked clean");
        }
    }
    // Pure: usable where no effect is allowed.
    let src = "fn h(x: Int) / {} -> Int { int.xor(int.wrapping_mul(x, 3), 5) }\npub fn main() -> Int { h(2) }\n";
    assert_eq!(errors(src), "");
}

#[test]
fn the_evaluator_wraps_and_shifts() {
    const MAX: i64 = i64::MAX;
    const MIN: i64 = i64::MIN;
    let max = "9223372036854775807";
    let min = "(0 - 9223372036854775807 - 1)";
    let cases: Vec<(String, Result<i64, String>)> = vec![
        (format!("int.wrapping_add({max}, 1)"), Ok(MIN)),
        (format!("int.wrapping_sub({min}, 1)"), Ok(MAX)),
        (format!("int.wrapping_mul({max}, 2)"), Ok(-2)),
        (
            "int.wrapping_mul(1099511628211, 1099511628211)".into(),
            Ok(1099511628211i64.wrapping_mul(1099511628211)),
        ),
        ("int.wrapping_add(2, 3)".into(), Ok(5)),
        ("int.and(12, 10)".into(), Ok(8)),
        ("int.or(12, 10)".into(), Ok(14)),
        ("int.xor(12, 10)".into(), Ok(6)),
        ("int.xor(0 - 1, 5)".into(), Ok(-6)),
        ("int.shl(1, 63)".into(), Ok(MIN)),
        ("int.shl(3, 0)".into(), Ok(3)),
        ("int.shr(0 - 16, 2)".into(), Ok(-4)),
        ("int.ushr(0 - 16, 60)".into(), Ok(15)),
        ("int.shr(0 - 1, 63)".into(), Ok(-1)),
        ("int.ushr(0 - 1, 63)".into(), Ok(1)),
        (
            "int.shl(1, 64)".into(),
            Err("int.shl: a shift by 64 is outside 0..=63".into()),
        ),
        (
            "int.shr(1, 0 - 1)".into(),
            Err("int.shr: a shift by -1 is outside 0..=63".into()),
        ),
        (
            "int.ushr(1, 100)".into(),
            Err("int.ushr: a shift by 100 is outside 0..=63".into()),
        ),
        // Unary operators, as before (D1 changes Core, not the evaluator).
        (format!("0 - (-{max})"), Ok(MAX)),
        (format!("-{min}"), Err("integer overflow".into())),
        ("if !(1 < 2) { 1 } else { 2 }".into(), Ok(2)),
    ];
    for (e, want) in cases {
        let src = format!("pub fn main() -> Int {{ {e} }}\n");
        assert_eq!(eval(&src), want, "{e}");
    }
}

#[test]
fn unary_operators_lower_to_core() {
    let src = "fn neg(x: Int) -> Int { -x }\n\
               fn not(b: Bool) -> Bool { !b }\n\
               pub fn main() -> Int { if not(False) { neg(3) } else { 0 } }\n";
    assert_eq!(errors(src), "");
    let session = Session::new();
    let (m, _) = parse_module(&session, src);
    let (diags, table) = elya::types::infer_typed_table(&session, &m);
    assert!(diags.is_empty(), "{diags:?}");
    let core = elya::core::lower_module(&m, &table).expect("unary operators lower");
    let body = |n: &str| {
        let f = core.fns.iter().find(|f| f.name == n).unwrap();
        format!("{:?}", f.body.kind)
    };
    // D1: `-x` is `0 - x`, `!b` is `b == False`.
    let neg = body("neg");
    assert!(neg.starts_with("Prim(Sub, [CoreExpr {"), "{neg}");
    assert!(neg.contains("Lit(Int(0))"), "{neg}");
    let not = body("not");
    assert!(not.starts_with("Prim(Eq, [CoreExpr {"), "{not}");
    assert!(not.contains("Lit(Bool(false))"), "{not}");
}

#[test]
fn a_builtin_used_as_a_value_is_wrapped_at_its_arity() {
    // The review's L1: E0208's help wrapped every builtin in a one-parameter
    // function, an arity error for the two-argument ones (and `time.now_ms`).
    for (b, wrap) in [
        ("int.xor", "fn(a, b) { int.xor(a, b) }"),
        ("int.shl", "fn(a, b) { int.shl(a, b) }"),
        ("int.to_string", "fn(a) { int.to_string(a) }"),
        ("time.now_ms", "fn() { time.now_ms() }"),
    ] {
        let src = format!("pub fn main() -> Int {{ let f = {b}  0 }}\n");
        let text = errors(&src);
        assert!(text.contains("E0208"), "{b}: {text}");
        assert!(text.contains(wrap), "{b}: {text}");
    }
}
