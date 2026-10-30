//! A name clash is reported once, where it was made (2026-10-30, spec
//! `docs/superpowers/specs/2026-10-30-elya-name-clashes-design.md`). Each program
//! is checked against the standard modules and two user modules, and every
//! diagnostic is read by its code and the files its labels are in.
//!
//! Negative controls (each reverted; the files compared byte for byte after):
//! - K1, the module rewriter's root-function check before the operation's (D1):
//!   the operation test reads three E0200s in `elya/console.elya` again;
//! - K2, E0207 checked for an effect declared twice (D2): the effect test reads
//!   five E0207s in `elya/console.elya`;
//! - K3, `visible` without a file's own names (D3): the own-name test reads two
//!   E0605s again, and each kind of `every_kind_of_own_name_is_visible` its own
//!   (type one, effect two, operation two; checked with the CLI per kind);
//! - K4, `visible` without the names of the files this one imports (the review's
//!   F1): the imported-file test reads E0605 before its E0209.

/// `lib/a` imports `lib/b`; `lib/b` declares a type, a constructor, an effect
/// with an operation, and functions that use them.
const FILES: &[(&str, &str)] = &[
    (
        "lib/a",
        "import lib/b / {Tally}\npub fn a() -> Int { b.count(1) }\n",
    ),
    (
        "lib/b",
        "type Counted { Counted(Int) }\n\
         effect Tally { fn tally(n: Int) -> Int }\n\
         pub fn count(n: Int) -> Int { match Counted(n) { Counted(k) -> k } }\n\
         pub fn tallied(body) { handle { body() } with { Tally.tally(n) -> resume(n + 1) } }\n\
         pub fn ticks() / {Tally} -> Int { tally(1) + tally(2) }\n",
    ),
    // The review's F1: `lib/x` declares `T` first (reached through `lib/a2`),
    // `lib/y` declares it too.
    ("lib/x", "type T { TX }\n"),
    ("lib/a2", "import lib/x\n"),
    ("lib/y", "type T { TY }\npub fn y(t: T) -> Int { 0 }\n"),
];

/// Each diagnostic's code and the file of each of its labels, in order.
fn diagnostics(root: &str) -> Vec<(String, Vec<String>)> {
    let mut read = |path: &str| {
        FILES
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(p, t)| (format!("{p}.elya"), t.to_string()))
    };
    let (_, diags, sm) = elya::front_end_with("main.elya", root, &mut read);
    diags
        .iter()
        .map(|d| {
            let files = d
                .labels
                .iter()
                .map(|l| sm.file_of(l.span.start).0.to_string())
                .collect();
            (d.code.clone(), files)
        })
        .collect()
}

fn codes(ds: &[(String, Vec<String>)]) -> Vec<&str> {
    ds.iter().map(|(c, _)| c.as_str()).collect()
}

/// D1: the root's function named like a module's operation is the root's one
/// error; the module's own uses of its operation still mean the operation.
#[test]
fn an_operation_clash_is_one_error_in_the_root() {
    for import in ["elya/console / {Console}", "elya/host / {IO}"] {
        let ds = diagnostics(&format!(
            "import {import}\nfn console_line(s: String) -> Int {{ 1 }}\npub fn main() -> Int {{ 0 }}\n"
        ));
        assert_eq!(codes(&ds), ["E0205"], "{import}: {ds:?}");
        assert_eq!(ds[0].1, ["main.elya"], "{import}: {ds:?}");
    }
    // A user module's operation, used by that module, clashing the same way.
    let ds = diagnostics(
        "import lib/b / {Tally}\nfn tally(n: Int) -> Int { n }\npub fn main() -> Int { b.tallied(fn() { b.ticks() }) }\n",
    );
    assert_eq!(codes(&ds), ["E0205"], "{ds:?}");
    assert_eq!(ds[0].1, ["main.elya"], "{ds:?}");
}

/// D2: two declarations of one effect are one E0209; the module's handlers of it
/// are not judged against both declarations' operations.
#[test]
fn an_effect_clash_is_one_error() {
    let ds = diagnostics(
        "import elya/console / {Console}\neffect Console { fn speak() -> Unit }\npub fn main() -> Int { 0 }\n",
    );
    assert_eq!(codes(&ds), ["E0209"], "{ds:?}");
    assert_eq!(ds[0].1, ["main.elya", "elya/console.elya"], "{ds:?}");
    let ds = diagnostics(
        "import lib/a\neffect Tally { fn other() -> Int }\npub fn main() -> Int { a.a() }\n",
    );
    assert_eq!(codes(&ds), ["E0209"], "{ds:?}");
    assert_eq!(ds[0].1, ["main.elya", "lib/b.elya"], "{ds:?}");
}

/// D3: a file's own type is visible in it, though a module it imports through
/// another declared one first. The clash is E0209, for the type and for its
/// constructor, and nothing says "add `import lib/b`".
#[test]
fn a_files_own_name_is_never_e0605() {
    let ds = diagnostics(
        "import lib/a\ntype Counted { Counted(Int) }\npub fn main() -> Int { match Counted(2) { Counted(k) -> k + a.a() } }\n",
    );
    assert_eq!(codes(&ds), ["E0209", "E0209"], "{ds:?}");
    for (_, files) in &ds {
        assert_eq!(files, &["main.elya", "lib/b.elya"], "{ds:?}");
    }
    // The same with `lib/b` imported directly too (a measured row).
    let ds = diagnostics(
        "import lib/a\nimport lib/b / {Tally}\ntype Counted { Counted(Int) }\npub fn main() -> Int { match Counted(2) { Counted(k) -> k + a.a() } }\n",
    );
    assert_eq!(codes(&ds), ["E0209", "E0209"], "{ds:?}");
}

/// D3 for every kind of name (the review's F2: the test above reaches only the
/// constructor's path): a type in an annotation, an effect in a row, an
/// operation called. Each is the clash's one error, never E0605.
#[test]
fn every_kind_of_own_name_is_visible() {
    for (what, decl, uses, want) in [
        (
            "type",
            "type Counted { Mine(Int) }",
            "fn f(c: Counted) -> Int { 0 }\npub fn main() -> Int { f(Mine(1)) + a.a() }",
            "E0209",
        ),
        (
            "effect",
            "effect Tally { fn mine() -> Int }",
            "fn g() / {Tally} -> Int { mine() }\npub fn main() -> Int { handle { g() } with { Tally.mine() -> resume(1) } + a.a() }",
            "E0209",
        ),
        (
            "operation",
            "effect Mine { fn tally(n: Int) -> Int }",
            "fn h() / {Mine} -> Int { tally(1) }\npub fn main() -> Int { handle { h() } with { Mine.tally(n) -> resume(n) } + a.a() }",
            "E0202",
        ),
    ] {
        let ds = diagnostics(&format!("import lib/a\n{decl}\n{uses}\n"));
        assert_eq!(codes(&ds), [want], "{what}: {ds:?}");
    }
}

/// The review's F1: a name another file declared first is visible here when a
/// file this one imports declares it too. The clash is E0209, once.
#[test]
fn a_name_an_imported_file_declares_is_visible() {
    let ds = diagnostics(
        "import lib/a2\nimport lib/y\nfn f(t: T) -> Int { 0 }\npub fn main() -> Int { f(TY) + y.y(TY) }\n",
    );
    assert_eq!(codes(&ds), ["E0209"], "{ds:?}");
    assert_eq!(ds[0].1, ["lib/y.elya", "lib/x.elya"], "{ds:?}");
}

/// What stays: E0605 for a name used from a module this file does not import,
/// and each clash's one error where nothing cascades.
#[test]
fn the_errors_that_stay() {
    let ds = diagnostics(
        "import lib/a\npub fn main() -> Int { match Counted(2) { Counted(k) -> k + a.a() } }\n",
    );
    assert_eq!(codes(&ds), ["E0605", "E0605"], "{ds:?}");
    for (_, files) in &ds {
        assert_eq!(files, &["main.elya"], "{ds:?}");
    }
    // A constructor's clash with a standard module's (a measured row).
    let ds = diagnostics("import elya/replay\ntype T { TraceEnd }\npub fn main() -> Int { 0 }\n");
    assert_eq!(codes(&ds), ["E0209"], "{ds:?}");
    assert_eq!(ds[0].1, ["main.elya", "elya/replay.elya"], "{ds:?}");
    let ds = diagnostics(
        "import elya/console / {Console}\ntype ConsoleCounted { CC }\npub fn main() -> Int { 0 }\n",
    );
    assert_eq!(codes(&ds), ["E0209"], "{ds:?}");
    assert_eq!(ds[0].1, ["main.elya", "elya/console.elya"], "{ds:?}");
    // A public function's name is free too: functions are per file. (The E0606
    // the measurement saw here was the missing grant, not the name.)
    let ds = diagnostics(
        "import elya/console / {Console}\nfn silenced(x: Int) -> Int { x }\npub fn main() -> Int { silenced(0) }\n",
    );
    assert!(ds.is_empty(), "{ds:?}");
    // Functions are per file: a module's private or public function name is free.
    let ds = diagnostics(
        "import elya/replay\nimport lib/a\nfn lines(x: Int) -> Int { x }\nfn count(n: Int) -> Int { n }\npub fn main() -> Int { lines(1) + count(2) + a.a() }\n",
    );
    assert!(ds.is_empty(), "{ds:?}");
}
