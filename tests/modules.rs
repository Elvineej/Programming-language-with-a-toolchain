//! Modules, part 1: a program in more than one file (2026-10-23, spec
//! `docs/superpowers/specs/2026-10-23-elya-modules-part-1-design.md`).
//!
//! Every program here is read through an in-memory reader: a map from a module
//! path (`a/b`) to its text, shown as `a/b.elya`.
//!
//! Negative controls (each reverted; `src/modules.rs` compared byte for byte after):
//! - K1, the rename ignoring locals (a parameter named like a module function):
//!   `functions_do_not_cross_files_unqualified` fails (the parameter is renamed).
//! - K2, visibility off: `a_name_from_a_module_not_imported_is_e0605` (no E0605).
//! - K3, the `pub` check off: `private_and_unknown_functions_are_e0602_and_e0603`.
//! - K4, every file parsed at span base 0: `a_diagnostic_names_its_file` (the error
//!   is drawn in the root file).
//!
//! Modules part 2 (spec `docs/superpowers/specs/2026-10-24-elya-modules-part-2-design.md`):
//! - K5, polarity ignored (every label counted): `a_callback_s_effect_is_not_the_module_s`
//!   (the handler library `sand` is E0606 for `Net`). Rerun after the review's fix
//!   (labels in `Interface::label`): the same failure.
//! - K6, the grant check off: `an_import_grants_only_its_list`,
//!   `a_callback_s_effect_is_not_the_module_s` and `grants_are_transitive` (no E0606).

use elya::Session;

/// A reader over `files`: module path -> (file name, text).
fn reader<'a>(
    files: &'a [(&'a str, &'a str)],
) -> impl FnMut(&str) -> Option<(String, String)> + 'a {
    move |path: &str| {
        files
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(p, t)| (format!("{p}.elya"), t.to_string()))
    }
}

/// The rendered errors of `root` with `files` beside it, or "" when it checks.
fn errors(root: &str, files: &[(&str, &str)]) -> String {
    let mut read = reader(files);
    match elya::check_source_with("main.elya", root, &mut read) {
        Ok(()) => String::new(),
        Err(e) => e,
    }
}

/// What the program printed and main's value; panics on an error.
fn run(root: &str, files: &[(&str, &str)]) -> (String, i64) {
    let e = errors(root, files);
    assert_eq!(e, "", "the front end rejected:\n{root}");
    let mut read = reader(files);
    let o = elya::run_source_outcome_with("main.elya", root, &mut read);
    assert!(o.error.is_none(), "{:?}", o.error);
    (o.output, o.value.expect("a value").expect("an Int"))
}

fn has(e: &str, code: &str, words: &str) {
    assert!(e.contains(code), "expected {code}:\n{e}");
    assert!(e.contains(words), "expected `{words}`:\n{e}");
}

const UTIL: &str = "pub fn twice(x: Int) -> Int { x * 2 }\nfn hidden() -> Int { 1 }\n";

#[test]
fn qualified_calls_run() {
    let files = [("util", UTIL)];
    let m1 = "import util\npub fn main() -> Int { util.twice(21) }\n";
    assert_eq!(run(m1, &files).1, 42);
    let m2 = "import util as u\npub fn main() -> Int { u.twice(21) }\n";
    assert_eq!(run(m2, &files).1, 42);
    let value = "import util\npub fn main() -> Int { let g = util.twice  g(5) }\n";
    assert_eq!(run(value, &files).1, 10);
}

#[test]
fn imports_resolve_by_path_once() {
    let files = [
        ("a/b", "pub fn three() -> Int { 3 }\n"),
        (
            "util",
            "effect Tick { fn tick() -> Unit }\npub fn twice(x: Int) -> Int { x * 2 }\n",
        ),
        (
            "lib",
            "import util\npub fn quad(x: Int) -> Int { util.twice(util.twice(x)) }\n",
        ),
    ];
    let root = "import a/b\nimport util\nimport lib\n\
                pub fn main() -> Int { b.three() * 100 + lib.quad(util.twice(1)) }\n";
    assert_eq!(run(root, &files).1, 308);
}

#[test]
fn a_missing_module_is_e0600() {
    let root = "import nothing/here\npub fn main() -> Int { 1 }\n";
    has(&errors(root, &[]), "E0600", "no module `nothing/here`");
    let root = "import elya/net\npub fn main() -> Int { 1 }\n";
    has(&errors(root, &[]), "E0600", "no module `elya/net`");
    let root = "import elya/io\npub fn main() -> Int { io.println(\"a\")  1 }\n";
    assert_eq!(run(root, &[]), ("a\n".to_string(), 1));
}

#[test]
fn an_import_cycle_is_e0601() {
    let files = [
        ("a", "import b\npub fn f() -> Int { 1 }\n"),
        ("b", "import a\npub fn g() -> Int { 2 }\n"),
    ];
    let root = "import a\npub fn main() -> Int { a.f() }\n";
    has(&errors(root, &files), "E0601", "an import cycle");
    // The root is the module `main` (its file's stem): importing it is a cycle.
    let files = [("c", "import main\npub fn f() -> Int { 1 }\n")];
    let root = "import c\npub fn main() -> Int { c.f() }\n";
    has(&errors(root, &files), "E0601", "an import cycle");
}

#[test]
fn private_and_unknown_functions_are_e0602_and_e0603() {
    let files = [("util", UTIL)];
    let root = "import util\npub fn main() -> Int { util.hidden() }\n";
    has(
        &errors(root, &files),
        "E0602",
        "`hidden` is private to module `util`",
    );
    let root = "import util\npub fn main() -> Int { util.thrice(1) }\n";
    has(
        &errors(root, &files),
        "E0603",
        "module `util` has no function `thrice`",
    );
}

#[test]
fn functions_do_not_cross_files_unqualified() {
    let files = [("util", UTIL)];
    let m4 = "import util\npub fn main() -> Int { twice(21) }\n";
    has(&errors(m4, &files), "E0200", "unresolved name `twice`");
    let files = [(
        "util",
        "fn helper() -> Int { 2 }\n\
         pub fn f() -> Int { helper() }\n\
         pub fn g(helper: Int) -> Int { helper + 100 }\n",
    )];
    let root = "import util\nfn helper() -> Int { 1 }\n\
                pub fn main() -> Int { helper() * 10 + util.f() + util.g(5) * 1000 }\n";
    assert_eq!(run(root, &files).1, 105_012);
}

#[test]
fn an_import_name_is_unique_e0604() {
    let files = [
        ("x/io", "pub fn f() -> Int { 1 }\n"),
        ("x/util", "pub fn f() -> Int { 1 }\n"),
        ("y/util", "pub fn f() -> Int { 2 }\n"),
    ];
    let root = "import x/io\npub fn main() -> Int { 1 }\n";
    has(&errors(root, &files), "E0604", "`io`");
    let root = "import x/util\nimport y/util\npub fn main() -> Int { 1 }\n";
    has(&errors(root, &files), "E0604", "`util`");
    // An alias makes the second one usable.
    let root =
        "import x/util\nimport y/util as w\npub fn main() -> Int { util.f() + w.f() * 10 }\n";
    assert_eq!(run(root, &files).1, 21);
}

#[test]
fn types_and_effects_cross_an_import() {
    let files = [(
        "util",
        "type Opt(a) { No, Yes(a) }\n\
         effect Log { fn say(line: String) -> Unit }\n\
         pub fn find(n: Int) / {Log} -> Opt(Int) {\n  say(\"looking\")\n  if n > 0 { Yes(n) } else { No }\n}\n",
    )];
    // `/ {Log}`: `find` performs `Log` (modules part 2 D2; the import had no
    // list before part 2 -- a CHANGED PROGRAM, the expectation unchanged).
    let root = "import util / {Log}\n\
                fn get(o: Opt(Int)) -> Int { match o { Yes(n) -> n  No -> 0 } }\n\
                pub fn main() -> Int {\n\
                  handle {\n\
                    handle { get(util.find(7)) } with any { op(o) -> { io.println(op.show(o))  resume(op.perform(o)) } }\n\
                  } with { Log.say(line) -> { io.println(line)  resume(Unit) } }\n\
                }\n";
    assert_eq!(
        run(root, &files),
        ("Log.say(\"looking\")\nlooking\n".to_string(), 7)
    );
}

#[test]
fn a_name_from_a_module_not_imported_is_e0605() {
    let files = [
        (
            "b",
            "type Box { Box(Int) }\neffect Ping { fn ping() -> Int }\npub fn mk() -> Box { Box(1) }\n",
        ),
        (
            "a",
            "import b\npub fn mk() -> Int { match b.mk() { Box(n) -> n } }\n",
        ),
    ];
    for body in [
        "fn f() -> Int { match Box(2) { Box(n) -> n } }",
        "fn f(x: Box) -> Int { 1 }",
        "fn f() / {Ping} -> Int { 1 }",
        "fn f() -> Int { handle { 1 } with { Ping.ping() -> resume(1) } }",
    ] {
        let root = format!("import a\n{body}\npub fn main() -> Int {{ a.mk() }}\n");
        has(&errors(&root, &files), "E0605", "module `b`");
    }
    // The module that does import `b` may use them.
    assert_eq!(
        run("import a\npub fn main() -> Int { a.mk() }\n", &files).1,
        1
    );
}

#[test]
fn a_type_declared_in_two_files_is_e0209() {
    let files = [("util", "type Opt { No }\npub fn f() -> Int { 1 }\n")];
    let root = "import util\ntype Opt { None }\npub fn main() -> Int { util.f() }\n";
    has(&errors(root, &files), "E0209", "Opt");
}

#[test]
fn a_diagnostic_names_its_file() {
    let files = [("util", "pub fn f() -> Int { True }\n")];
    let root = "import util\npub fn main() -> Int { util.f() }\n";
    let e = errors(root, &files);
    assert!(e.contains("E0400"), "{e}");
    assert!(e.contains("util.elya:1:"), "{e}");
    assert!(!e.contains("main.elya"), "{e}");
}

/// Part 1 refused every list (E0606 "not checked yet"); part 2 checks it (spec
/// 2026-10-24-elya-modules-part-2 D2): a list may grant more than the module
/// uses. CHANGED EXPECTATION, by that decision.
#[test]
fn an_allow_list_is_e0606() {
    let files = [("util", UTIL)];
    let root = "import util / {IO}\npub fn main() -> Int { util.twice(1) }\n";
    assert_eq!(errors(root, &files), "");
    assert_eq!(run(root, &files).1, 2);
}

#[test]
fn a_single_file_program_is_unchanged() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("elya") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let (alone, d1) = elya::parse::parse_module(&Session::new(), &text);
        let mut none = |_: &str| None;
        let (program, d2) = elya::modules::load(&Session::new(), "x.elya", &text, &mut none);
        assert_eq!(
            elya::ast::pretty(&alone),
            elya::ast::pretty(&program.module),
            "{path:?}"
        );
        assert_eq!(d1.len(), d2.len(), "{path:?}");
        seen += 1;
    }
    assert!(seen >= 10, "{seen}");
}

// ---- The review (2026-10-23), each red first ---------------------------------

/// R1: types, constructors, effects and operations are separate name spaces, as
/// in one file; a constructor `Box` here is not another module's type `Box`.
#[test]
fn namespaces_stay_separate_across_files() {
    let files = [
        (
            "c",
            "type Box { MkBox(Int) }\neffect Ping { fn ping() -> Int }\npub fn one() -> Int { 1 }\n",
        ),
        ("b", "import c\npub fn two() -> Int { match MkBox(2) { MkBox(n) -> n } }\n"),
    ];
    let root = "import b\ntype Wrap { Box(Int) }\ntype Ping { Pong }\n\
                fn h(p: Ping) -> Int { match p { Pong -> 10 } }\n\
                pub fn main() -> Int { match Box(2) { Box(n) -> n * 100 + h(Pong) + b.two() } }\n";
    assert_eq!(run(root, &files).1, 212);
}

/// R2: E0205 holds in a module: its function named like an operation would
/// otherwise silently be called where the operation was meant.
#[test]
fn a_function_named_like_an_operation_is_e0205_in_a_module() {
    let files = [(
        "util",
        "effect Ask { fn ask() -> Int }\npub fn ask() -> Int { 41 }\npub fn use_ask() -> Int { ask() }\n",
    )];
    let root = "import util\n\
                pub fn main() -> Int { handle { util.use_ask() } with { Ask.ask() -> resume(1000) } }\n";
    has(
        &errors(root, &files),
        "E0205",
        "has the name of an operation",
    );
}

/// R3: a failed import or a refused qualified name is one diagnostic, not a
/// second E0201 "unknown builtin" on the same name.
#[test]
fn a_refused_module_reference_is_one_diagnostic() {
    let files = [("util", UTIL)];
    for (root, code) in [
        (
            "import util\npub fn main() -> Int { util.hidden() + util.nope() }\n",
            "E0602",
        ),
        ("import nope\npub fn main() -> Int { nope.f() }\n", "E0600"),
    ] {
        let e = errors(root, &files);
        assert!(e.contains(code), "{e}");
        assert!(!e.contains("E0201"), "{e}");
    }
}

/// R4: an error at the end of a module's file (an unclosed brace) is drawn in
/// that file.
#[test]
fn an_error_at_the_end_of_a_module_names_its_file() {
    let files = [("util", "pub fn f(x: Int) -> Int {\n  x\n")];
    let root = "import util\npub fn main() -> Int { util.f(1) }\n";
    let e = errors(root, &files);
    assert!(e.contains("util.elya:"), "{e}");
}

/// R5: the messages name the real problem.
#[test]
fn messages_name_the_real_problem() {
    let e = errors("import elya/io as x\npub fn main() -> Int { 1 }\n", &[]);
    has(&e, "E0604", "keeps its name `io`");
    // A module cannot see the root's types, and cannot import the root.
    let files = [("util", "pub fn f(x: Mine) -> Int { 1 }\n")];
    let root = "import util\ntype Mine { M }\npub fn main() -> Int { util.f(M) }\n";
    let e = errors(root, &files);
    has(&e, "E0605", "the root program");
    assert!(!e.contains("add `import main`"), "{e}");
}

// ---- modules, part 2: the effect allow-list (spec 2026-10-24) -------------

const STEAL: &str =
    "pub fn parse(s: String) -> Int { let t = time.now_ms()  io.println(\"phoning home\")  3 }\n";
const NET: &str =
    "effect Net { fn get(u: String) -> Int }\npub fn fetch() -> Int { get(\"x\") + 1 }\n";

#[test]
fn an_import_grants_only_its_list() {
    let files = [("steal", STEAL)];
    let m1 = "import steal\npub fn main() -> Int { steal.parse(\"{}\") }\n";
    has(
        &errors(m1, &files),
        "E0606",
        "module `steal` can perform `IO`, which this import does not grant",
    );
    let granted = "import steal / {IO}\npub fn main() -> Int { steal.parse(\"{}\") }\n";
    assert_eq!(run(granted, &files), ("phoning home\n".to_string(), 3));
    // A returned closure that prints is the module's I/O too.
    let files = [(
        "mk",
        "pub fn later() -> fn() -> Int { fn() { io.println(\"later\")  7 } }\n",
    )];
    let m3 = "import mk\npub fn main() -> Int { let f = mk.later()  f() }\n";
    has(&errors(m3, &files), "E0606", "can perform `IO`");
    // A user effect is a capability like `IO`.
    let files = [("net", NET)];
    let m4 = "import net\npub fn main() -> Int { handle { net.fetch() } with { Net.get(u) -> resume(41) } }\n";
    has(&errors(m4, &files), "E0606", "can perform `Net`");
    let m4ok = "import net / {Net}\npub fn main() -> Int { handle { net.fetch() } with { Net.get(u) -> resume(41) } }\n";
    assert_eq!(run(m4ok, &files).1, 42);
    // Handled inside the module: nothing escapes, nothing to grant.
    let files = [(
        "quiet",
        "effect Net { fn get(u: String) -> Int }\nfn fetch() -> Int { get(\"x\") + 1 }\n\
         pub fn sandboxed() -> Int { handle { fetch() } with { Net.get(u) -> resume(41) } }\n",
    )];
    let m5 = "import quiet\npub fn main() -> Int { quiet.sandboxed() }\n";
    assert_eq!(run(m5, &files).1, 42);
}

#[test]
fn a_callback_s_effect_is_not_the_module_s() {
    // A handler library receives a closure performing `Net`: the caller's.
    let files = [(
        "sand",
        "effect Net { fn get(u: String) -> Int }\n\
         pub fn sandbox(body: fn() / {Net} -> Int) -> Int { handle { body() } with { Net.get(u) -> resume(5) } }\n",
    )];
    let root =
        "import sand\npub fn main() -> Int { sand.sandbox(fn() { get(\"a\") + get(\"b\") }) }\n";
    assert_eq!(run(root, &files).1, 10);
    // A library handing its callback a closure that prints: its own I/O.
    let files = [(
        "cb",
        "pub fn with_log(k: fn(fn(String) -> Int) -> Int) -> Int { k(fn(s) { io.println(s)  1 }) }\n",
    )];
    let root = "import cb\npub fn main() -> Int { cb.with_log(fn(log) { log(\"x\") }) }\n";
    has(&errors(root, &files), "E0606", "can perform `IO`");
    // Data holding a closure that prints, returned: its own I/O.
    let files = [(
        "boxed",
        "type Box { Box(fn() / {IO} -> Int) }\n\
         pub fn make() -> Box { Box(fn() { io.println(\"boxed\")  1 }) }\n",
    )];
    let root = "import boxed\npub fn main() -> Int { match boxed.make() { Box(f) -> f() } }\n";
    has(&errors(root, &files), "E0606", "can perform `IO`");
}

#[test]
fn grants_are_transitive() {
    // `lib` is granted `IO` by its own import and lets it escape: the root must
    // grant it to `lib`.
    let files = [
        ("steal", STEAL),
        (
            "lib",
            "import steal / {IO}\npub fn go() -> Int { steal.parse(\"x\") + 1 }\n",
        ),
    ];
    let root = "import lib\npub fn main() -> Int { lib.go() }\n";
    has(
        &errors(root, &files),
        "E0606",
        "module `lib` can perform `IO`",
    );
    let root_ok = "import lib / {IO}\npub fn main() -> Int { lib.go() }\n";
    assert_eq!(run(root_ok, &files).1, 4);
    // `lib` importing `steal` without the grant: `lib`'s own import is refused.
    let files = [
        ("steal", STEAL),
        (
            "lib",
            "import steal\npub fn go() -> Int { steal.parse(\"x\") + 1 }\n",
        ),
    ];
    let root = "import lib / {IO}\npub fn main() -> Int { lib.go() }\n";
    has(
        &errors(root, &files),
        "E0606",
        "module `steal` can perform `IO`",
    );
    // `lib` handling `net`'s effect inside: a pure interface, nothing to grant.
    let files = [
        ("net", NET),
        (
            "lib",
            "import net / {Net}\npub fn go() -> Int { handle { net.fetch() } with { Net.get(u) -> resume(1) } }\n",
        ),
    ];
    let root = "import lib\npub fn main() -> Int { lib.go() }\n";
    assert_eq!(run(root, &files).1, 2);
}

#[test]
fn the_list_names_visible_effects() {
    let files = [("util", UTIL)];
    has(
        &errors(
            "import util / {Nope}\npub fn main() -> Int { util.twice(1) }\n",
            &files,
        ),
        "E0607",
        "no effect `Nope` here",
    );
    has(
        &errors(
            "import util / {IO, IO}\npub fn main() -> Int { util.twice(1) }\n",
            &files,
        ),
        "E0607",
        "`IO` is listed twice",
    );
    has(
        &errors(
            "import elya/io / {IO}\npub fn main() -> Int { 1 }\n",
            &files,
        ),
        "E0608",
        "a standard module takes no effect list",
    );
    // An effect declared in the importing file, or in a module it imports, is
    // visible.
    let files = [("net", NET)];
    let root = "import net / {Net}\neffect Mine { fn m() -> Int }\nimport net as n2 / {Mine}\n\
                pub fn main() -> Int { handle { net.fetch() } with { Net.get(u) -> resume(1) } }\n";
    let e = errors(root, &files);
    assert!(!e.contains("E0607"), "{e}");
}

/// The review's S1-S3: a closure the module makes can leave through an
/// effect's operations or a label's type arguments; each is the module's
/// capability (spec 2026-10-24 D1, operations and label arguments).
#[test]
fn closures_through_operations_are_the_module_s() {
    // S1: the module's handler resumes the importer's body with a closure that
    // prints; `Ask` is only in the callback's row.
    let files = [(
        "m",
        "effect Ask { fn ask() -> fn() / {IO} -> Int }\n\
         pub fn sandbox(body) { handle { body() } with { Ask.ask() -> resume(fn() { io.println(\"pwned\")  1 }) } }\n",
    )];
    let root = "import m\npub fn main() -> Int { m.sandbox(fn() { let f = ask()  f() }) }\n";
    has(&errors(root, &files), "E0606", "can perform `IO`");
    // S2: an operation's argument is a closure that prints.
    let files = [(
        "m",
        "effect Give { fn give(f: fn() / {IO} -> Int) -> Int }\n\
         pub fn go() -> Int { give(fn() { io.println(\"pwned\")  1 }) }\n",
    )];
    let root = "import m / {Give}\npub fn main() -> Int { handle { m.go() } with { Give.give(f) -> resume(f()) } }\n";
    has(&errors(root, &files), "E0606", "can perform `IO`");
    let ok = "import m / {Give, IO}\npub fn main() -> Int { handle { m.go() } with { Give.give(f) -> resume(f()) } }\n";
    assert_eq!(run(ok, &files), ("pwned\n".to_string(), 1));
    // S3: a label's type argument carries it.
    let files = [(
        "m",
        "effect St(s) { fn get() -> s  fn put(x: s) -> Unit }\n\
         pub fn go() { put(fn() { io.println(\"pwned\")  1 })  0 }\n",
    )];
    let root = "import m / {St}\npub fn main() -> Int {\n\
                let k = handle { m.go() } with { St.get() -> fn(s) { resume(s)(s) }  St.put(x) -> fn(s) { resume(Unit)(x) }  return(v) -> fn(s) { s() + v } }\n\
                k(fn() { 0 })\n}\n";
    has(&errors(root, &files), "E0606", "can perform `IO`");
}

/// The review's quality finding: the help names the function as the importing
/// file writes it (through its alias).
#[test]
fn the_help_names_the_function_through_the_alias() {
    let files = [("a/b", "pub fn go() -> Int { io.println(\"x\")  1 }\n")];
    let root = "import a/b as c\npub fn main() -> Int { c.go() }\n";
    has(&errors(root, &files), "E0606", "`c.go` performs it");
}
