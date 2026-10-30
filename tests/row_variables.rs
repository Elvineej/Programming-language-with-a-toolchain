//! Row variables you can write (2026-10-30, spec
//! `docs/superpowers/specs/2026-10-30-elya-row-variables-design.md`, part 1): a
//! row may end in a named row variable, `/ {e}` or `/ {Log | e}`, shared across
//! its function's signature.
//!
//! Negative controls (each reverted; the file compared byte for byte after):
//! - K1, a fresh row variable at every mention (not shared): the two sharing
//!   tests fail (`first`'s program runs; the function row and the body's
//!   annotation share nothing);
//! - K2, an annotation's tail dropped (read as a closed row): the three tests
//!   that use a tail fail (E0423 and E0420 where none is due);
//! - K3, row unification's stale tails and `bind_row`'s overwrite back (the
//!   review's F1): the shared-tail test passes `Log` to `main` unreported;
//! - K4, the named declared row checked where the first draft did (after the
//!   ambient closes, with the last member's names; F2, F3): the own-functions
//!   and sharing tests fail.

fn codes(src: &str) -> Vec<String> {
    let (_, diags, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    diags.iter().map(|d| d.code.clone()).collect()
}

/// main's value, or the rendered error.
fn value(src: &str) -> Result<i64, String> {
    let o = elya::run_source_outcome("t.elya", src);
    match o.error {
        Some(e) => Err(e),
        None => Ok(o.value.expect("a value").expect("an Int")),
    }
}

const LOG: &str = "effect Log { fn say(s: String) -> Unit }\n\
                   fn quiet(body) { handle { body() } with { Log.say(s) -> resume(Unit) } }\n";

/// D1, D2: `/ {e}` names the parameter's row, and the function's own row is the
/// same one: `run` performs what `f` does, here `Log`.
#[test]
fn a_named_row_is_shared_across_a_signature() {
    let src = format!(
        "{LOG}fn run(f: fn() / {{e}} -> Int) / {{e}} -> Int {{ f() + 1 }}\n\
         pub fn main() -> Int {{ quiet(fn() {{ run(fn() {{ say(\"x\")  41 }}) }}) + run(fn() {{ 100 }}) }}\n"
    );
    assert_eq!(codes(&src), Vec::<String>::new(), "{src}");
    assert_eq!(value(&src), Ok(143));
}

/// D1: `{Log | e}` is `Log` and whatever `e` is.
#[test]
fn a_named_row_after_labels_adds_to_them() {
    let src = format!(
        "{LOG}fn logged(f: fn() / {{e}} -> Int) / {{Log | e}} -> Int {{ say(\"in\")  f() }}\n\
         effect Ask {{ fn ask() -> Int }}\n\
         pub fn main() -> Int {{ quiet(fn() {{ handle {{ logged(fn() {{ ask() }}) }} with {{ Ask.ask() -> resume(5) }} }}) }}\n"
    );
    assert_eq!(codes(&src), Vec::<String>::new(), "{src}");
    assert_eq!(value(&src), Ok(5));
}

/// D2: one name is one row, across positions. `first` returns `f`, but its
/// signature says the result may do what `g` does too (they share `e`): the
/// caller must then handle `Ask`, which `g` performs. Unnamed, the result's row
/// is `f`'s alone and the same program runs.
#[test]
fn a_named_row_is_one_row_across_positions() {
    let body = "pub fn main() -> Int { quiet(fn() { first(fn() { say(\"x\")  1 }, fn() { ask() })() }) }\n";
    let unnamed = format!("{LOG}effect Ask {{ fn ask() -> Int }}\nfn first(f, g) {{ f }}\n{body}");
    assert_eq!(value(&unnamed), Ok(1), "{unnamed}");
    let named = format!(
        "{LOG}effect Ask {{ fn ask() -> Int }}\n\
         fn first(f: fn() / {{e}} -> Int, g: fn() / {{e}} -> Int) -> fn() / {{e}} -> Int {{ f }}\n{body}"
    );
    assert_eq!(codes(&named), ["E0420"], "`Ask` is never handled:\n{named}");
}

/// D1's errors, each by name, one each (the review's F5: `{e, Log}` cascaded,
/// and the messages misled).
#[test]
fn a_row_variable_is_written_once_and_last() {
    let messages = |src: &str| -> Vec<String> {
        let (_, diags, _) = elya::front_end_with("t.elya", src, &mut |_| None);
        diags.iter().map(|d| d.message.clone()).collect()
    };
    for (row, want) in [
        ("{e | h}", "nothing follows a row variable"),
        (
            "{e, Log}",
            "a row variable comes after the effects and `|`: write `{Log | e}`",
        ),
        (
            "{Log, e}",
            "a row variable comes after the effects and `|`: write `{Log | e}`",
        ),
        (
            "{Log | Log}",
            "expected a row variable (lowercase) after `|`",
        ),
        ("{e |}", "nothing follows a row variable"),
        ("{_}", "a row variable needs a name"),
    ] {
        let src = format!(
            "effect Log {{ fn say(s: String) -> Unit }}\nfn f(g: fn() / {row} -> Int) -> Int {{ g() }}\npub fn main() -> Int {{ 0 }}\n"
        );
        assert_eq!(messages(&src), [want], "{row}");
    }
    let grant = messages("import elya/console / {e}\npub fn main() -> Int { 0 }\n");
    assert_eq!(grant, ["a grant names effects, not a row variable"]);
}

/// The review's F5: a lowercase name that is an effect's name, lowercased, is a
/// typo, not a row variable; and one name is a type or a row, not both.
#[test]
fn a_row_variable_is_not_a_misspelled_effect_or_a_type() {
    let src = format!(
        "{LOG}fn f(g: fn() / {{log}} -> Int) -> Int {{ g() }}\npub fn main() -> Int {{ 0 }}\n"
    );
    let (_, diags, _) = elya::front_end_with("t.elya", &src, &mut |_| None);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("`log`") && diags[0].helps.iter().any(|h| h.contains("`Log`")),
        "{diags:?}"
    );
    let both = "fn f(g: fn() / {a} -> a) -> Int { 0 }\npub fn main() -> Int { 0 }\n";
    let (_, diags, _) = elya::front_end_with("t.elya", both, &mut |_| None);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0].message.contains("`a`"), "{diags:?}");
}

/// The review's F1 (an old row-unification bug that names made writable): a
/// row and the same row with a label, unified, dropped the label, and a program
/// that checked clean failed at run time ("unhandled effect `say`"). Now the
/// label stays, in either argument order: `main` must handle `Log`.
#[test]
fn a_shared_tail_keeps_its_labels() {
    for (app, call) in [
        (
            "fn app(x: t, f: fn(t) -> Int) -> Int { f(x) }",
            "app(k, run)",
        ),
        (
            "fn app(f: fn(t) -> Int, x: t) -> Int { f(x) }",
            "app(run, k)",
        ),
    ] {
        let src = format!(
            "{LOG}{app}\n\
             fn bad(k: fn() / {{Log | e}} -> Int, run: fn(fn() / {{e}} -> Int) -> Int) -> Int {{ {call} }}\n\
             pub fn main() -> Int {{ bad(fn() {{ say(\"x\")  1 }}, fn(h) {{ h() }}) }}\n"
        );
        assert_eq!(codes(&src), ["E0420"], "{src}");
        let handled = src
            .replace(
                "pub fn main() -> Int { bad(",
                "pub fn main() -> Int { quiet(fn() { bad(",
            )
            .replace("h() }) }\n", "h() }) }) }\n");
        assert_eq!(value(&handled), Ok(1), "{handled}");
    }
    // `{Log | e}` and `{e}` are one row: `e` holds `Log`.
    let src = format!(
        "{LOG}fn f(g: fn() / {{Log | e}} -> Int) / {{e}} -> Int {{ g() }}\n\
         pub fn main() -> Int {{ quiet(fn() {{ f(fn() {{ say(\"x\")  7 }}) }}) }}\n"
    );
    assert_eq!(value(&src), Ok(7), "{src}");
}

/// The review's F2: a declared named row was unified after the ambient was
/// closed, so `e` was forced pure where it is written elsewhere. And F3: in a
/// recursive group, one function's names were read for another's.
#[test]
fn a_declared_named_row_is_its_own_functions() {
    let ask = "effect Ask { fn ask() -> Int }\n";
    for (decl, main, want) in [
        (
            "fn f(g: fn() / {e} -> Int) / {Log | e} -> Int { say(\"x\")  1 }",
            "pub fn main() -> Int { quiet(fn() { handle { f(fn() { ask() }) } with { Ask.ask() -> resume(5) } }) }",
            1,
        ),
        (
            "fn f(g: fn() / {e} -> Int) / {e} -> fn() / {e} -> Int { g }",
            "pub fn main() -> Int { quiet(fn() { f(fn() { say(\"x\")  3 })() }) }",
            3,
        ),
        (
            "fn f(body: fn() / {e} -> Int) / {e} -> Int { handle { body() } with any { op(o) -> 0 } }",
            "pub fn main() -> Int { handle { f(fn() { ask() + 1 }) } with { Ask.ask() -> resume(20) } }",
            0,
        ),
        (
            "fn a(f: fn() / {e} -> Int) / {e} -> Int { let u = b  f() }\nfn b(n: Int) / {e} -> Int { let u = a  n }",
            "pub fn main() -> Int { handle { a(fn() { ask() }) } with { Ask.ask() -> resume(5) } }",
            5,
        ),
    ] {
        let src = format!("{LOG}{ask}{decl}\n{main}\n");
        assert_eq!(value(&src), Ok(want), "{src}");
    }
}

/// Sharing, where only sharing decides (the review's F6: the tests above pass
/// with a fresh variable per mention). A function's row named like its
/// parameter's makes the caller handle what the argument does, though the body
/// never calls it; an annotation in the body shares the name too.
#[test]
fn a_name_is_shared_by_the_function_row_and_the_body() {
    let row = format!(
        "{LOG}fn run(f: fn() / {{e}} -> Int) / {{e}} -> Int {{ 1 }}\n\
         pub fn main() -> Int {{ run(fn() {{ say(\"x\")  1 }}) }}\n"
    );
    assert_eq!(codes(&row), ["E0420"], "{row}");
    let unnamed = format!(
        "{LOG}fn run(f: fn() -> Int) -> Int {{ 1 }}\n\
         pub fn main() -> Int {{ run(fn() {{ say(\"x\")  1 }}) }}\n"
    );
    assert_eq!(value(&unnamed), Ok(1), "{unnamed}");
    let body = format!(
        "{LOG}fn f(g: fn() / {{e}} -> Int) -> Int {{\n\
         \x20 let k: fn() / {{e}} -> Int = fn() {{ say(\"x\")  1 }}\n\
         \x20 g()\n\
         }}\n\
         pub fn main() -> Int {{ f(fn() {{ 2 }}) }}\n"
    );
    assert_eq!(
        codes(&body),
        ["E0420"],
        "`g`'s row holds `Log` through `k`:\n{body}"
    );
}

/// A declaration's row variable is part 2's (a row parameter): refused by name
/// until then, not "unknown effect `e`".
#[test]
fn a_declarations_row_variable_waits_for_part_2() {
    let src = "type T { T(fn() / {e} -> Int) }\neffect A { fn spawn(f: fn() / {e} -> Int) -> Int }\npub fn main() -> Int { 0 }\n";
    let (_, diags, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    assert_eq!(diags.len(), 2, "{diags:?}");
    for d in &diags {
        assert_eq!(d.code, "E0432");
        assert!(
            d.message.contains("cannot name a row variable"),
            "{}",
            d.message
        );
    }
}
