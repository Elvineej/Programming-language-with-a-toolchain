//! Diagnostics at the edges (2026-10-24, spec
//! `docs/superpowers/specs/2026-10-24-elya-diagnostics-at-the-edges-design.md`):
//! byte offsets in the renderer (D1), both declarations of a duplicate (D2), one
//! parser error per position (D3), and the nesting limit, E0102 (D4).
//!
//! The deep programs run on a 256 MiB thread: the library compiles on its
//! caller's stack (D5 gives the CLI its own), and a test thread has 2 MiB.
//!
//! Negative controls (each reverted; the files compared byte for byte after):
//! - K1, D1 off (`Config::default()`): `a_label_after_non_ascii_text_is_under_its_bytes`
//!   (the header at `3:9`).
//! - K2, D3 off: `an_if_cut_off_by_the_end_of_the_file_is_one_error` (a second
//!   "expected `}`").
//! - K3, D4's height off (open calls counted only): the process aborts (a
//!   100 000-term chain reaches the later passes).
//! - K4 (D5 off) is in `crates/cli/tests/run_cli.rs`'s
//!   `elya_run_at_the_nesting_limit_prints_its_value` (an abort).

use elya::span::SourceMap;

/// `rendered` without its ANSI colour codes.
fn plain(rendered: &str) -> String {
    let mut out = String::new();
    let mut esc = false;
    for c in rendered.chars() {
        match (esc, c) {
            (false, '\u{1b}') => esc = true,
            (true, 'm') => esc = false,
            (true, _) => {}
            (false, c) => out.push(c),
        }
    }
    out
}

/// The diagnostics `check` reports, as `code message` lines, in order.
fn diags(src: &str) -> Vec<String> {
    let rendered = match elya::check_source("t.elya", src) {
        Ok(()) => return Vec::new(),
        Err(e) => e,
    };
    plain(&rendered)
        .lines()
        .filter_map(|l| {
            let rest = l.trim_start().strip_prefix('[')?;
            let (code, msg) = rest.split_once("] Error: ")?;
            Some(format!("{code} {msg}"))
        })
        .collect()
}

/// Run `f` on a 256 MiB thread (the deep programs recurse on the host stack).
fn big<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("the thread finished")
}

/// main's value under `elya run`, or the rendered error.
fn value(src: &str) -> Result<i64, String> {
    let o = elya::run_source_outcome_with("t.elya", src, &mut |_: &str| None);
    match o.error {
        Some(e) => Err(plain(&e)),
        None => Ok(o.value.expect("a value").expect("an Int")),
    }
}

const E0102: &str = "E0102 expression nested too deeply";

// ---- D1 ----

#[test]
fn a_label_after_non_ascii_text_is_under_its_bytes() {
    let src = "fn main() -> Int {\n  let s = \"h\u{e9}llo w\u{f6}rld\"\n  1 + nope\n}\n";
    let out = plain(&elya::check_source("d1.elya", src).unwrap_err());
    // `nope` is the 7th character of line 3; the two 2-byte characters before
    // it (on line 2) used to shift it to 3:9 and the label onto `pe\n}`.
    assert!(out.contains("d1.elya:3:7"), "{out}");
    assert!(out.contains("1 + nope"), "{out}");
}

// ---- D2 ----

/// The diagnostics of a program `main.elya` importing `b` (text `b`).
fn two_files(root: &str, b: &str) -> (Vec<elya::diag::Diagnostic>, SourceMap) {
    let b = b.to_string();
    let mut read = move |path: &str| {
        if path == "b" {
            Some(("b.elya".to_string(), b.clone()))
        } else {
            None
        }
    };
    let (_, d, sources) = elya::front_end_with("main.elya", root, &mut read);
    (d, sources)
}

#[test]
fn a_type_declared_in_two_files_names_both() {
    let (d, sources) = two_files(
        "import b\ntype T { B }\npub fn main() -> Int { b.g() }\n",
        "pub fn g() -> Int { 2 }\ntype T { A }\n",
    );
    let e: Vec<_> = d.iter().filter(|d| d.code == "E0209").collect();
    assert_eq!(e.len(), 1, "{d:?}");
    let labels = &e[0].labels;
    assert_eq!(labels.len(), 2, "{labels:?}");
    let files: Vec<&str> = labels
        .iter()
        .map(|l| sources.file_of(l.span.start).0)
        .collect();
    assert_eq!(files, ["main.elya", "b.elya"]);
    assert_eq!(labels[1].message, "first declared here");
}

#[test]
fn an_operation_declared_twice_names_both() {
    let src =
        "effect A { fn op() -> Int }\neffect B { fn op() -> Int }\npub fn main() -> Int { 1 }\n";
    let (_, d, _) = elya::front_end_with("t.elya", src, &mut |_: &str| None);
    let e: Vec<_> = d.iter().filter(|d| d.code == "E0202").collect();
    assert_eq!(e.len(), 1, "{d:?}");
    let labels = &e[0].labels;
    assert_eq!(labels.len(), 2, "{labels:?}");
    assert!(labels[1].span.start < 28, "{labels:?}"); // in `effect A`
    assert!(labels[0].span.start > 28, "{labels:?}"); // in `effect B`
    assert_eq!(labels[1].message, "first declared here");
}

// ---- D3 ----

#[test]
fn an_if_cut_off_by_the_end_of_the_file_is_one_error() {
    assert_eq!(
        diags("fn main() -> Int { if 1 == 1 { 1 }"),
        ["E0100 expected `else` (if is an expression)"]
    );
    // Not at the end of the file, two missing tokens at one place are two
    // errors (the review's F1: `else` and `f`'s `}` are both missing).
    assert_eq!(
        diags(
            "fn f() -> Int { if 1 == 1 { 1 }\nfn g() -> Int { 2 }\npub fn main() -> Int { g() }\n"
        ),
        [
            "E0100 expected `else` (if is an expression)",
            "E0100 expected `}`"
        ]
    );
    // Not at the end of the file, two real mistakes are still two.
    assert_eq!(
        diags("fn main() -> Int { if 1 == 1 { 1 } }\nfn f() -> Int { ( }\n").len(),
        2
    );
}

// ---- D4 ----

/// `n` levels of each shape, as a whole program.
fn shapes(n: usize) -> Vec<(&'static str, String)> {
    let main = |body: String| format!("fn main() -> Int {{ {body} }}\n");
    vec![
        (
            "brackets",
            main(format!("{}1{}", "(".repeat(n), ")".repeat(n))),
        ),
        ("a chain", main(vec!["1"; n].join(" + "))),
        (
            "blocks",
            main(format!("{}1{}", "{ ".repeat(n), " }".repeat(n))),
        ),
        (
            "lambdas",
            main(format!(
                "let f = {}1{}\n  0",
                "fn() { ".repeat(n),
                " }".repeat(n)
            )),
        ),
        (
            "matches",
            main(format!(
                "{}1{}",
                "match 1 { _ -> ".repeat(n),
                " }".repeat(n)
            )),
        ),
        (
            "ifs",
            main(format!(
                "{}1{}",
                "if 1 == 1 { ".repeat(n),
                " } else { 0 }".repeat(n)
            )),
        ),
        (
            "calls",
            format!(
                "fn i(x: Int) -> Int {{ x }}\n{}",
                main(format!("{}1{}", "i(".repeat(n), ")".repeat(n)))
            ),
        ),
        (
            "patterns",
            format!(
                "type O {{ S(O), N }}\n{}",
                main(format!(
                    "match N {{\n  {}N{} -> 1\n  _ -> 0\n  }}",
                    "S(".repeat(n),
                    ")".repeat(n)
                ))
            ),
        ),
        (
            "types",
            format!(
                "fn f(x: {}Int) -> Int {{ 0 }}\n{}",
                "fn() -> ".repeat(n),
                main("0".to_string())
            ),
        ),
    ]
}

#[test]
fn every_deep_shape_is_one_e0102() {
    big(|| {
        for (name, src) in shapes(100_000) {
            assert_eq!(diags(&src), [E0102], "{name}");
        }
    });
}

#[test]
fn the_limit_is_1024_levels() {
    big(|| {
        // A chain of n terms is n levels; n brackets around `1` are n + 1.
        let chain = |n: usize| format!("fn main() -> Int {{ {} }}\n", vec!["1"; n].join(" + "));
        assert_eq!(value(&chain(1024)), Ok(1024));
        assert_eq!(diags(&chain(1025)), [E0102]);
        let brackets = |n: usize| {
            format!(
                "fn main() -> Int {{ {}1{} }}\n",
                "(".repeat(n),
                ")".repeat(n)
            )
        };
        assert_eq!(value(&brackets(1023)), Ok(1));
        assert_eq!(diags(&brackets(1024)), [E0102]);
    });
}

#[test]
fn after_e0102_the_next_declaration_is_still_checked() {
    big(|| {
        let src = format!(
            "fn deep() -> Int {{ {}1{} }}\nfn later() -> Int {{ nope }}\npub fn main() -> Int {{ deep() + later() }}\n",
            "(".repeat(1100),
            ")".repeat(1100)
        );
        assert_eq!(diags(&src), [E0102, "E0200 unresolved name `nope`"]);
    });
}

#[test]
fn after_e0102_in_an_effect_its_own_operations_are_skipped() {
    // The review's F2: the resume point is a declaration outside every `{`
    // opened since the declaration began, not the operation `fn two`.
    big(|| {
        let src = format!(
            "effect E {{ fn op(g: {}Int) -> Int\n  fn two() -> Int }}\npub fn main() -> Int {{ nope }}\n",
            "fn() -> ".repeat(1100)
        );
        assert_eq!(diags(&src), [E0102, "E0200 unresolved name `nope`"]);
    });
}
