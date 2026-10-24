//! Capabilities, part 3: attenuation in the standard library (2026-10-24, spec
//! `docs/superpowers/specs/2026-10-24-elya-capabilities-part-3-design.md`).
//! `elya/console` (`Console`) and `elya/clock` (`WallClock`) are a vocabulary a
//! dependency is written against and granted instead of `IO`; their handlers
//! attenuate it; `elya/host` turns it into `IO` and needs `IO` granted.
//!
//! Negative controls (each reverted; `src/std/` compared byte for byte after):
//! - K1, `console.limited` letting every line out (`if True`):
//!   `the_attenuating_handlers` fails.
//! - K2, `elya/host` declaring the console effect itself instead of importing
//!   it: E0209 and E0202 in every test importing both.

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

/// `e` is E0606 for `effect` at the import of `module`.
fn refused(e: &str, module: &str, effect: &str) {
    assert!(
        e.contains(&format!(
            "module `{module}` can perform `{effect}`, which this import does not grant"
        )),
        "expected E0606 for `{module}` and `{effect}`:\n{e}"
    );
}

#[path = "programs/std_capabilities.rs"]
mod programs;
use programs::{ATTENUATED, DEP, GIVEN_LESS};

#[test]
fn a_dependency_gets_lines_and_a_clock_not_io() {
    let (out, v) = run(GIVEN_LESS, &[("dep", DEP)]);
    assert_eq!(out, "[dep] report 1 at 1000\n[dep] report 2 at 1000\n");
    assert_eq!(v, 6);
}

#[test]
fn the_attenuating_handlers() {
    let (out, v) = run(ATTENUATED, &[("dep", DEP)]);
    assert_eq!(
        out,
        "report 1 at 10\nreport 2 at 15\n\
         report 1 at 0\nreport 2 at 0\nreport 3 at 0\n\
         captured:\nreport 5 at 7\nreport 6 at 7\n\n\
         report 4 at 75\n"
    );
    // a = 12 (three lines made, two got out), b = 12 * 100 + 3 lines,
    // c = 10 + 12, d = 18 (no line), e = 8.
    assert_eq!(v, 12 * 100000 + 1203 * 100 + 22 + 18 + 8);
}

#[test]
fn the_host_needs_io_granted() {
    let root = "import elya/console / {Console}
import elya/host
pub fn main() -> Int { host.console(fn() { console_line(\"hi\")  1 }) }
";
    refused(&errors(root, &[]), "elya/host", "IO");
    let granted = root.replace("import elya/host\n", "import elya/host / {IO}\n");
    assert_eq!(run(&granted, &[]), ("hi\n".to_string(), 1));
}

#[test]
fn a_dependency_cannot_reach_io_through_them() {
    let root = "import elya/console / {Console}\nimport elya/host / {IO}\nimport dep / {Console}\n\
                pub fn main() -> Int { host.console(fn() { dep.go() }) }\n";
    // Printing itself.
    let direct = "import elya/console / {Console}\npub fn go() -> Int { io.println(\"pwned\")  console_line(\"ok\")  1 }\n";
    refused(&errors(root, &[("dep", direct)]), "dep", "IO");
    // Through the host's handler, which it may import with `IO` itself.
    let via_host = "import elya/console / {Console}\nimport elya/host / {IO}\n\
                    pub fn go() -> Int { host.console(fn() { console_line(\"pwned\")  1 }) }\n";
    refused(&errors(root, &[("dep", via_host)]), "dep", "IO");
    // Granted only what it uses, it runs.
    let honest =
        "import elya/console / {Console}\npub fn go() -> Int { console_line(\"ok\")  1 }\n";
    assert_eq!(run(root, &[("dep", honest)]), ("ok\n".to_string(), 1));
}

#[test]
fn a_relaying_handler_needs_its_effect_granted() {
    // D4: the relaying handlers (`prefixed`, `limited`, `counted`) perform
    // `Console`, and an import is checked against the whole interface.
    let root = "import elya/console
import elya/host / {IO}
pub fn main() -> Int { host.console(fn() { console.silenced(fn() { 1 }) }) }
";
    refused(&errors(root, &[]), "elya/console", "Console");
    let granted = root.replace("import elya/console\n", "import elya/console / {Console}\n");
    assert_eq!(run(&granted, &[]), (String::new(), 1));
}
