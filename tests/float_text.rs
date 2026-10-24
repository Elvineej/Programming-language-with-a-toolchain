//! `float.to_string` (2026-10-23, spec
//! `docs/superpowers/specs/2026-10-23-elya-float-natively-design.md` D5): a
//! float's text is its literal text, the one `op.show` writes.

fn errors(src: &str) -> String {
    match elya::check_source("t.elya", src) {
        Ok(()) => String::new(),
        Err(e) => e,
    }
}

/// The evaluator's measured texts (spec §0), each value through `float.to_string`.
pub const TABLE: &[(&str, &str)] = &[
    ("1.5", "1.5"),
    ("0.0 -. 0.0", "0.0"),
    ("-0.0", "-0.0"),
    ("0.1 +. 0.2", "0.30000000000000004"),
    ("1.0 /. 0.0", "inf"),
    ("-1.0 /. 0.0", "-inf"),
    ("0.0 /. 0.0", "nan"),
    ("10000000000000000000000.0 *. 10000000000000000000.0", "100000000000000000000000000000000000000000.0"),
    ("9007199254740993.0", "9007199254740992.0"),
    ("1.0 /. 3.0", "0.3333333333333333"),
    ("0.0000001", "0.0000001"),
    ("0.000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000005", "0.000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000005"),
    ("123456789012345680000.0", "123456789012345680000.0"),
    ("2.0", "2.0"),
];

#[test]
fn float_to_string_writes_the_literal_text() {
    for (expr, text) in TABLE {
        let src = format!("pub fn main() {{ io.println(float.to_string({expr})) }}\n");
        assert_eq!(errors(&src), "", "{src}");
        let out = elya::run_source("t.elya", &src).expect("runs");
        assert_eq!(out, format!("{text}\n"), "{expr}");
    }
}

#[test]
fn float_to_string_is_typed() {
    let pure = "fn f(x: Float) / {} -> String { float.to_string(x) }\n\
                pub fn main() { io.println(f(2.5)) }\n";
    assert_eq!(errors(pure), "");
    for bad in [
        "pub fn main() { io.println(float.to_string(True)) }\n",
        "pub fn main() { io.println(float.to_string(1)) }\n",
        "pub fn main() { io.println(float.to_string(1.0, 2.0)) }\n",
        "pub fn main() -> Int { float.to_string(1.0) }\n",
    ] {
        let e = errors(bad);
        assert!(e.contains("E04") || e.contains("E0203"), "{bad}\n{e}");
        assert!(!e.contains("E0201"), "{bad}\n{e}");
    }
}
