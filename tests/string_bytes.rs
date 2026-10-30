//! Strings indexed by byte (2026-10-30, spec
//! `docs/superpowers/specs/2026-10-24-elya-trace-text-design.md` D1):
//! `string.length`, `string.byte_at` and `string.slice`, pure, each failure by
//! name. A `String` is UTF-8 on both back ends, so a slice may not split a
//! character.

/// What the program printed, and main's value or the rendered error.
fn run(src: &str) -> (String, Result<i64, String>) {
    let o = elya::run_source_outcome("t.elya", src);
    if let Some(e) = o.error {
        return (o.output, Err(e));
    }
    (o.output, Ok(o.value.expect("a value").expect("an Int")))
}

fn errors(src: &str) -> String {
    match elya::check_source("t.elya", src) {
        Ok(()) => String::new(),
        Err(e) => e,
    }
}

#[test]
fn length_and_byte_at_read_bytes() {
    // `é` is two bytes, C3 A9.
    let src = "pub fn main() -> Int {\n\
               \x20 let s = \"h\u{e9}llo\"\n\
               \x20 string.length(s) * 1000000 + string.byte_at(s, 1) * 1000 + string.byte_at(s, 2)\n\
               }\n";
    assert_eq!(run(src).1, Ok(6_195_169));
    let src = "pub fn main() -> Int { string.length(\"\") * 10 + string.byte_at(\"a\", 0) }\n";
    assert_eq!(run(src).1, Ok(97));
}

#[test]
fn slice_takes_the_bytes_between_two_boundaries() {
    let src = "pub fn main() -> Int {\n\
               \x20 let s = \"h\u{e9}llo\"\n\
               \x20 io.println(string.slice(s, 0, 1))\n\
               \x20 io.println(string.slice(s, 1, 3))\n\
               \x20 io.println(\"[\" <> string.slice(s, 6, 6) <> \"]\")\n\
               \x20 io.println(string.slice(s, 0, string.length(s)))\n\
               \x20 string.length(string.slice(s, 3, 6))\n\
               }\n";
    let (out, v) = run(src);
    assert_eq!(v, Ok(3), "{out}");
    assert_eq!(out, "h\n\u{e9}\n[]\nh\u{e9}llo\n");
}

#[test]
fn each_failure_has_its_name() {
    for (call, words) in [
        (
            "string.byte_at(\"abc\", 7)",
            "string.byte_at: index 7 is outside a string of 3 bytes",
        ),
        (
            "string.byte_at(\"abc\", -1)",
            "string.byte_at: index -1 is outside a string of 3 bytes",
        ),
        (
            "string.length(string.slice(\"abc\", 2, 9))",
            "string.slice: 2..9 is not a range in a string of 3 bytes",
        ),
        (
            "string.length(string.slice(\"abc\", 2, 1))",
            "string.slice: 2..1 is not a range in a string of 3 bytes",
        ),
        (
            "string.length(string.slice(\"abc\", -1, 2))",
            "string.slice: -1..2 is not a range in a string of 3 bytes",
        ),
        (
            "string.length(string.slice(\"\u{e9}t\u{e9}\", 1, 2))",
            "string.slice: 1..2 splits a UTF-8 character",
        ),
        (
            "string.length(string.slice(\"\u{e9}t\u{e9}\", 0, 4))",
            "string.slice: 0..4 splits a UTF-8 character",
        ),
    ] {
        let src = format!("pub fn main() -> Int {{ {call} }}\n");
        let (_, v) = run(&src);
        let e = v.expect_err(call);
        assert!(e.contains("E0300"), "{call}: {e}");
        assert!(e.contains(words), "{call}: expected `{words}`:\n{e}");
    }
}

#[test]
fn the_builtins_are_typed_and_only_called() {
    let e = errors("pub fn main() -> Int { string.length(5) }\n");
    assert!(e.contains("E0400"), "{e}");
    let e = errors("pub fn main() -> Int { string.byte_at(\"a\", True) }\n");
    assert!(e.contains("E0400"), "{e}");
    let e = errors("pub fn main() -> Int { string.slice(\"a\", 0, 1) }\n");
    assert!(e.contains("E0400"), "a slice is a String, not an Int: {e}");
    let e = errors("pub fn main() -> Int { string.width(\"a\") }\n");
    assert!(e.contains("E0201"), "{e}");
    let e = errors("pub fn main() -> Int { let f = string.slice  0 }\n");
    assert!(e.contains("E0208"), "{e}");
    assert!(
        e.contains("fn(a, b, c) { string.slice(a, b, c) }"),
        "the help at the builtin's own arity: {e}"
    );
}

#[test]
fn they_are_pure() {
    // Usable in a function that performs nothing: a failure is not an effect.
    let src = "fn first(s: String) / {} -> Int { string.byte_at(string.slice(s, 0, 1), 0) }\n\
               pub fn main() -> Int { first(\"Z\") }\n";
    assert_eq!(errors(src), "");
    assert_eq!(run(src).1, Ok(90));
}

#[test]
fn the_module_imports_like_int() {
    let src = "import elya/string\npub fn main() -> Int { string.length(\"ab\") }\n";
    assert_eq!(errors(src), "");
    assert_eq!(run(src).1, Ok(2));
    let e = errors("import elya/string as s\npub fn main() -> Int { 1 }\n");
    assert!(e.contains("E0604"), "{e}");
    assert!(e.contains("keeps its name `string`"), "{e}");
    let files = [("text/string", "pub fn f() -> Int { 1 }\n")];
    let mut read = move |path: &str| {
        files
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(p, t)| (format!("{p}.elya"), t.to_string()))
    };
    let e = elya::check_source_with(
        "main.elya",
        "import text/string\npub fn main() -> Int { 1 }\n",
        &mut read,
    )
    .expect_err("a user import named `string`");
    assert!(e.contains("E0604"), "{e}");
    assert!(e.contains("`string`"), "{e}");
}
