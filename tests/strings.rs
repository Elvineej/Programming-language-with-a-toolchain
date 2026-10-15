//! `int.to_string` (2026-10-15, spec `2026-10-15-elya-native-strings-design.md` D1): the
//! one way to turn a number into text. A pure, module-qualified builtin like
//! `io.println`; decimal, with a leading `-` for a negative number, `i64::MIN`
//! included. Native `<>`, `==` and this builtin are pinned against the evaluator in
//! `crates/codegen/tests/native_codegen.rs`.

use elya::{check_source, run_source};

#[test]
fn int_to_string_writes_decimal_text() {
    let src = "pub fn main() {\n\
                 io.println(int.to_string(0))\n\
                 io.println(int.to_string(0 - 42))\n\
                 io.println(\"n=\" <> int.to_string(12345))\n\
                 io.println(int.to_string(0 - 9223372036854775807 - 1))\n\
               }\n";
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
    assert_eq!(
        run_source("t.elya", src).expect("runs"),
        "0\n-42\nn=12345\n-9223372036854775808\n"
    );
}

#[test]
fn int_to_string_is_pure_and_takes_an_int() {
    // Pure: a function declared `/ {}` may call it.
    let pure = "fn label(n: Int) / {} -> String { \"#\" <> int.to_string(n) }\n\
                pub fn main() { io.println(label(7)) }\n";
    assert!(
        check_source("t.elya", pure).is_ok(),
        "{:?}",
        check_source("t.elya", pure)
    );
    assert_eq!(run_source("t.elya", pure).expect("runs"), "#7\n");
    // An Int, not a String.
    let wrong = "pub fn main() { io.println(int.to_string(\"7\")) }\n";
    let err = check_source("t.elya", wrong).expect_err("a String is not an Int");
    assert!(format!("{err:?}").contains("E0400"), "{err:?}");
}

#[test]
fn a_misspelt_builtin_is_still_unknown() {
    let src = "pub fn main() { io.println(int.to_strin(1)) }\n";
    let err = check_source("t.elya", src).expect_err("no such builtin");
    assert!(format!("{err:?}").contains("E0201"), "{err:?}");
}
