//! Self-hosting, part 1 (2026-11-14, spec
//! `docs/superpowers/specs/2026-11-14-elya-files-and-arguments-design.md`): a
//! program reads and writes files, reads standard input and its arguments,
//! through six builtins that perform `IO`, fail by name, and are recorded and
//! replayed like `time.*`; `elya/files` gives a dependency less than the host.
//!
//! Negative controls (each reverted; the file compared byte for byte after): see
//! the plan, `docs/superpowers/plans/2026-11-14-elya-files-and-arguments.md`.

use elya::eval::Host;
use std::path::{Path, PathBuf};

#[allow(dead_code)] // each suite uses some of the programs
#[path = "programs/files_and_arguments.rs"]
mod programs;
use programs::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-files-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn host(args: &[&str], stdin: &[u8]) -> Host {
    Host {
        args: args.iter().map(|a| a.to_string()).collect(),
        stdin: Some(stdin.to_vec()),
    }
}

/// `src` run live with `host`: (output, value line) or (output, error).
fn run(src: &str, host: Host) -> elya::RunOutcome {
    elya::run_source_outcome_host("main.elya", src, &mut |_| None, host)
}

fn ok(o: &elya::RunOutcome) -> (String, String) {
    assert!(o.error.is_none(), "{:?}", o.error);
    let line = o.line.clone().expect("a value line").expect("data");
    (o.output.clone(), line)
}

#[test]
fn a_file_written_is_read_back_and_checked() {
    let dir = temp_dir("written");
    let o = run(&at(FILES, &dir), host(&[], b""));
    assert_eq!(
        ok(&o),
        (FILES_OUT.to_string(), FILES_TEXT.len().to_string())
    );
    assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"");
    std::fs::remove_dir_all(&dir).ok();
}

/// The bad files [`FAILURES`] reads.
fn plant(dir: &Path) {
    std::fs::write(dir.join("bad.txt"), BAD_UTF8).unwrap();
    std::fs::write(dir.join("nul.txt"), NUL_PATH).unwrap();
}

#[test]
fn reading_and_writing_fail_by_name() {
    let dir = temp_dir("failures");
    plant(&dir);
    for (src, message, before) in FAILURES {
        let o = run(&at(src, &dir), host(&[], b""));
        let e = o.error.unwrap_or_else(|| panic!("{src}: ran"));
        let message = at(message, &dir);
        assert!(e.contains(&message), "{src}: wanted `{message}`, got {e}");
        assert_eq!(o.output, *before, "{src}");
    }
    // Arguments there are, named by count.
    let o = run("pub fn main() { io.arg(2) }\n", host(&["a", "b"], b""));
    let e = o.error.expect("fails");
    assert!(e.contains("no argument 2 (the program has 2)"), "{e}");
    // Standard input that is not UTF-8.
    let o = run(READS_INPUT, host(&[], b"\xff\xfe"));
    let e = o.error.expect("fails");
    assert!(e.contains(INPUT_NOT_UTF8), "{e}");
    assert_eq!(o.output, "before\n");
    assert!(!dir.join("no").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn arguments_and_standard_input_come_from_the_host() {
    let o = run(ARGS, host(&ARGS_GIVEN, ARGS_STDIN.as_bytes()));
    assert_eq!(ok(&o), (ARGS_OUT.to_string(), "3".to_string()));
    let o = run(ARGS, host(&[], b""));
    assert_eq!(
        ok(&o),
        ("input: \nagain: []\n".to_string(), "0".to_string())
    );
}

#[test]
fn every_answer_is_recorded_and_replayed() {
    let dir = temp_dir("traced");
    let src = at(TRACED, &dir);
    let (trace, o) = elya::record_source_host(
        "main.elya",
        &src,
        &mut |_| None,
        host(&[TRACED_ARG], TRACED_STDIN.as_bytes()),
    );
    let trace = trace.expect("a trace");
    assert_eq!(trace, at(TRACED_TRACE, &dir));
    assert_eq!(ok(&o), (TRACED_OUT.to_string(), "1".to_string()));
    // D3: a replay answers from the trace and touches nothing. The file is
    // gone, and the replay neither reads it nor writes it again.
    std::fs::remove_file(dir.join("t.txt")).unwrap();
    let r = elya::replay_source("main.elya", &src, &trace);
    assert_eq!(ok(&r), (TRACED_OUT.to_string(), "1".to_string()));
    assert!(!dir.join("t.txt").exists(), "the replay wrote the file");
    // A hand-written trace is the world: what the file holds is what it says.
    let world = trace.replace(r#"= "a \"b\"\nc""#, r#"= "other""#);
    let r = elya::replay_source("main.elya", &src, &world);
    assert_eq!(ok(&r).0, "other|in\n|x y\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_replay_checks_each_answer_against_its_type() {
    for (src, trace, message) in [
        (
            "pub fn main() { io.file_exists(\"p\") }\n",
            "io.file_exists(\"p\") = \"yes\"",
            "replay entry 1: `\"yes\"` is not an answer of type Bool",
        ),
        (
            "pub fn main() { io.read_file(\"p\") }\n",
            "io.read_file(\"p\") = 5",
            "replay entry 1: `5` is not an answer of type String",
        ),
        (
            "pub fn main() { io.read_file(\"p\") }\n",
            "io.read_file(\"p\") = \"a\" \"b\"",
            "replay entry 1: `\"a\" \"b\"` is not an answer of type String",
        ),
        (
            "pub fn main() { io.read_stdin() }\n",
            "io.read_stdin() = \"\\q\"",
            "replay entry 1: `\"\\q\"` is not an answer of type String",
        ),
        (
            "pub fn main() { io.arg_count() }\n",
            "io.arg_count() = True",
            "replay entry 1: `True` is not an answer of type Int",
        ),
        (
            "pub fn main() { io.write_file(\"p\", \"x\") }\n",
            "io.write_file(\"p\", \"x\") = 0",
            "replay entry 1: `0` is not an answer of type Unit",
        ),
        (
            "pub fn main() { io.arg(0) }\n",
            "io.arg(1) = \"x\"",
            "replay diverged at entry 1: the program asked `io.arg(0)`, the trace holds `io.arg(1)`",
        ),
    ] {
        let o = elya::replay_source("main.elya", src, &format!("elya trace 1\n{trace}\n"));
        let e = o.error.unwrap_or_else(|| panic!("{src}: replayed"));
        assert!(e.contains(message), "{src}: wanted `{message}`, got {e}");
    }
    // Every answer type, read back.
    let src = "pub fn main() -> Int {\n  let e = io.file_exists(\"p\")\n  \
               io.println(io.read_file(\"p\") <> io.arg(0))\n  if e { io.arg_count() } else { 0 }\n}\n";
    let trace = "elya trace 1\nio.file_exists(\"p\") = True\nio.read_file(\"p\") = \"\\t\\\\\"\n\
                 io.arg(0) = \"é\"\nio.arg_count() = 7\n";
    let r = elya::replay_source("main.elya", src, trace);
    assert_eq!(ok(&r), ("\t\\é\n".to_string(), "7".to_string()));
}

/// The rendered errors of `root` with `files` beside it, or "" when it checks.
fn errors(root: &str, files: &[(&str, &str)]) -> String {
    let mut read = |path: &str| {
        files
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(p, t)| (format!("{p}.elya"), t.to_string()))
    };
    match elya::check_source_with("main.elya", root, &mut read) {
        Ok(()) => String::new(),
        Err(e) => e,
    }
}

#[test]
fn the_builtins_are_typed_and_perform_io() {
    for (src, wanted) in [
        ("pub fn main() { io.read_file(1) }\n", "String"),
        ("pub fn main() -> Int { io.read_file(\"a\") }\n", "String"),
        (
            "pub fn main() { io.write_file(\"a\") }\n",
            "wrong number of arguments",
        ),
        ("pub fn main() { io.arg(\"0\") }\n", "Int"),
        (
            "pub fn main() -> String { io.file_exists(\"a\") }\n",
            "Bool",
        ),
        (
            "pub fn main() { io.read_stdin(1) }\n",
            "wrong number of arguments",
        ),
    ] {
        let e = errors(src, &[]);
        assert!(!e.is_empty(), "{src}: checked");
        assert!(e.contains(wanted), "{src}: wanted `{wanted}` in {e}");
    }
    // A dependency granted `Console` alone cannot reach the files.
    let root = "import elya/console / {Console}\nimport elya/host / {IO}\nimport dep / {Console}\n\
                pub fn main() { host.console(fn() { dep.go() }) }\n";
    for body in [
        "console_line(io.read_file(\"/etc/passwd\"))",
        "io.write_file(\"x\", \"y\")  console_line(\"y\")",
        "console_line(if io.file_exists(\"x\") { \"y\" } else { \"n\" })",
        "console_line(io.read_stdin())",
        "console_line(io.arg(0))",
        "console_line(int.to_string(io.arg_count()))",
    ] {
        let dep = format!("import elya/console / {{Console}}\npub fn go() {{ {body} }}\n");
        let e = errors(root, &[("dep", &dep)]);
        assert!(
            e.contains("module `dep` can perform `IO`, which this import does not grant"),
            "{body}: {e}"
        );
    }
}

/// The `elya/files` handlers, all in memory: none needs `IO`.
const HANDLED: &str = r#"import elya/files / {Files}
import elya/console / {Console}
import elya/host / {IO}

fn job() -> String {
  let _ = write_file("notes", "hello")
  let seen = if file_exists("notes") { "yes" } else { "no" }
  let other = if file_exists("missing") { read_file("missing") } else { "-" }
  seen <> ":" <> read_file("notes") <> ":" <> other
}

fn names(m) -> String {
  match m { FilesEmpty -> ""  FilesEntry(n, _, rest) -> n <> ";" <> names(rest) }
}

fn value(r) -> String {
  match r { FilesRun(v, _) -> v  FilesMissing(p, m) -> "stopped at " <> p <> " [" <> names(m) <> "]" }
}

pub fn main() -> Int {
  host.console(fn() {
    console_line("in memory: " <> value(files.in_memory(FilesEmpty, fn() { job() })))
    let b = match files.in_memory(FilesEntry("notes", "old", FilesEmpty), fn() {
      files.read_only(fn() { read_file("notes") })
    }) {
      FilesRun(r, _) -> match r { FilesReadOnlyOk(v) -> "read " <> v  FilesReadOnlyDenied(p) -> "denied " <> p }
      FilesMissing(p, _) -> "missing " <> p
    }
    console_line("read only: " <> b)
    let c = match files.in_memory(FilesEmpty, fn() { files.read_only(fn() { job() }) }) {
      FilesRun(r, m) -> match r {
        FilesReadOnlyOk(v) -> "wrote " <> v
        FilesReadOnlyDenied(p) -> "denied " <> p <> " [" <> names(m) <> "]"
      }
      FilesMissing(p, _) -> "missing " <> p
    }
    console_line("read only: " <> c)
    let d = match files.in_memory(FilesEmpty, fn() { files.under("dir/", fn() { job() }) }) {
      FilesRun(_, m) -> names(m)
      FilesMissing(p, _) -> "missing " <> p
    }
    console_line("under: " <> d)
    let kept = FilesEntry("a", "1", FilesEmpty)
    console_line("gone: " <> value(files.in_memory(kept, fn() { write_file("b", "2")  read_file("gone") <> "!" })))
    1
  })
}
"#;

const HANDLED_OUT: &str = "in memory: yes:hello:-\nread only: read old\n\
                           read only: denied notes []\nunder: dir/notes;\n\
                           gone: stopped at gone [a;b;]\n";

#[test]
fn the_elya_files_handlers_give_a_dependency_less() {
    assert_eq!(errors(HANDLED, &[]), "");
    let o = run(HANDLED, Host::default());
    assert_eq!(ok(&o), (HANDLED_OUT.to_string(), "1".to_string()));
    // A dependency written against `Files` is granted `Files`, never `IO`.
    let root = "import elya/files / {Files}\nimport elya/host / {IO}\n\
                import dep / {Files}\n\
                pub fn main() { match files.in_memory(FilesEmpty, fn() { dep.go() }) { FilesRun(v, _) -> v  FilesMissing(p, _) -> p } }\n";
    let dep = "import elya/files / {Files}\npub fn go() -> String { write_file(\"k\", \"v\")  read_file(\"k\") }\n";
    assert_eq!(errors(root, &[("dep", dep)]), "");
    let sneaky =
        "import elya/files / {Files}\npub fn go() -> String { io.read_file(\"/etc/passwd\") }\n";
    assert!(errors(root, &[("dep", sneaky)])
        .contains("module `dep` can perform `IO`, which this import does not grant"));
}

#[test]
fn elya_host_gives_files_the_hosts_meaning() {
    let dir = temp_dir("host");
    let src = at(
        r#"import elya/files / {Files}
import elya/host / {IO}
fn job(p: String) -> String {
  let _ = write_file(p, "on disk")
  if file_exists(p) { read_file(p) } else { "missing" }
}
pub fn main() -> Int {
  io.println(host.files(fn() { job("@D@/h.txt") }))
  io.println(host.files(fn() { files.under("@D@/", fn() { job("u.txt") }) }))
  string.length(io.read_file("@D@/u.txt"))
}
"#,
        &dir,
    );
    assert_eq!(errors(&src, &[]), "");
    let o = run(&src, Host::default());
    assert_eq!(ok(&o), ("on disk\non disk\n".to_string(), "7".to_string()));
    assert_eq!(
        std::fs::read_to_string(dir.join("h.txt")).unwrap(),
        "on disk"
    );
    // `elya/host` needs `IO` granted, as for the console and the clock.
    let ungranted = src.replace("import elya/host / {IO}", "import elya/host");
    assert!(errors(&ungranted, &[])
        .contains("module `elya/host` can perform `IO`, which this import does not grant"));
    std::fs::remove_dir_all(&dir).ok();
}
