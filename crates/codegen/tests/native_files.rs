//! Files, standard input and arguments natively (2026-11-14, spec
//! `docs/superpowers/specs/2026-11-14-elya-files-and-arguments-design.md`): a
//! binary does what the evaluator does with the same world, fails with its
//! words, takes its own command line and input, and records and replays the
//! evaluator's trace.
//!
//! Negative controls: see the plan,
//! `docs/superpowers/plans/2026-11-14-elya-files-and-arguments.md`.

use elya::eval::Host;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[allow(dead_code)] // each suite uses some of the programs
#[path = "../../../tests/programs/files_and_arguments.rs"]
mod programs;
use programs::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-nfiles-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn build(src: &str, dir: &Path, tag: &str) -> PathBuf {
    let (m, pd, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    assert!(pd.is_empty(), "front end: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    let core = elya::core::lower_module(&m, &table).expect("lowers");
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).expect("compile_module");
    elya_codegen::link(&obj, &exe).expect("link");
    exe
}

/// The binary run with `args`, `stdin` and `env`.
fn run(exe: &Path, args: &[&str], stdin: &[u8], env: &[(&str, &Path)]) -> Output {
    let mut c = Command::new(exe);
    c.args(args)
        .env_remove("ELYA_RECORD")
        .env_remove("ELYA_REPLAY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        c.env(k, v);
    }
    let mut child = c.spawn().expect("run the binary");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    child.wait_with_output().expect("wait")
}

/// What the evaluator prints for `src` with this world, then main's value line:
/// what a native binary writes on stdout.
fn evaluator(src: &str, args: &[&str], stdin: &[u8]) -> elya::RunOutcome {
    let host = Host {
        args: args.iter().map(|a| a.to_string()).collect(),
        stdin: Some(stdin.to_vec()),
    };
    elya::run_source_outcome_host("t.elya", src, &mut |_| None, host)
}

fn stdout_of(o: &elya::RunOutcome) -> String {
    assert!(o.error.is_none(), "{:?}", o.error);
    format!(
        "{}{}\n",
        o.output,
        o.line.clone().expect("a line").expect("data")
    )
}

/// A native failure's message: stderr is exactly `elya: <message>\n`, status 1.
fn failure(out: &Output) -> String {
    let e = String::from_utf8(out.stderr.clone()).expect("UTF-8");
    assert_eq!(out.status.code(), Some(1), "{e}");
    e.strip_prefix("elya: ")
        .and_then(|m| m.strip_suffix('\n'))
        .unwrap_or_else(|| panic!("not a named failure: {e:?}"))
        .to_string()
}

#[test]
fn files_natively_are_the_evaluators() {
    let exes = temp_dir("same-exes");
    for (tag, src, args, stdin) in [
        ("files", FILES, &[][..], ""),
        ("args", ARGS, &ARGS_GIVEN[..], ARGS_STDIN),
        ("no-args", ARGS, &[][..], ""),
        ("handlers", IN_HANDLERS, &[][..], ""),
    ] {
        // Each back end in a directory of its own; the files they leave compared.
        let (e_dir, n_dir) = (temp_dir(&format!("{tag}-e")), temp_dir(&format!("{tag}-n")));
        let want = stdout_of(&evaluator(&at(src, &e_dir), args, stdin.as_bytes()));
        let exe = build(&at(src, &n_dir), &exes, tag);
        let got = run(&exe, args, stdin.as_bytes(), &[]);
        assert!(got.status.success(), "{tag}: {got:?}");
        assert_eq!(String::from_utf8_lossy(&got.stdout), want, "{tag}");
        let listing = |d: &Path| {
            let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(d)
                .unwrap()
                .map(|e| {
                    let e = e.unwrap();
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        std::fs::read(e.path()).unwrap(),
                    )
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(listing(&n_dir), listing(&e_dir), "{tag}: the files left");
        std::fs::remove_dir_all(&e_dir).ok();
        std::fs::remove_dir_all(&n_dir).ok();
    }
    std::fs::remove_dir_all(&exes).ok();
}

#[test]
fn native_failures_have_the_evaluators_words() {
    let dir = temp_dir("failures");
    std::fs::write(dir.join("bad.txt"), BAD_UTF8).unwrap();
    std::fs::write(dir.join("nul.txt"), NUL_PATH).unwrap();
    for (i, (src, message, before)) in FAILURES.iter().enumerate() {
        let src = at(src, &dir);
        let exe = build(&src, &dir, &format!("f{i}"));
        let native = run(&exe, &[], b"", &[]);
        assert_eq!(failure(&native), at(message, &dir), "{src}");
        assert_eq!(String::from_utf8_lossy(&native.stdout), *before, "{src}");
        let e = evaluator(&src, &[], b"").error.expect("fails");
        assert!(e.contains(&at(message, &dir)), "{src}: the evaluator: {e}");
    }
    let exe = build("pub fn main() { io.arg(2) }\n", &dir, "arg2");
    assert_eq!(
        failure(&run(&exe, &["a", "b"], b"", &[])),
        "no argument 2 (the program has 2)"
    );
    let exe = build(READS_INPUT, &dir, "input");
    let native = run(&exe, &[], b"\xff\xfe", &[]);
    assert_eq!(failure(&native), INPUT_NOT_UTF8);
    assert_eq!(String::from_utf8_lossy(&native.stdout), "before\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_native_binary_reads_its_own_arguments_and_input() {
    let dir = temp_dir("own");
    let exe = build(ARGS, &dir, "args");
    let out = run(&exe, &ARGS_GIVEN, ARGS_STDIN.as_bytes(), &[]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{ARGS_OUT}3\n")
    );
    // An argument that looks like a flag is the program's, as any other.
    let out = run(&exe, &["--x", ""], b"", &[]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "0: --x\n1: \ninput: \nagain: []\n2\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_traces_are_the_evaluators_both_ways() {
    let dir = temp_dir("traces");
    let src = at(TRACED, &dir);
    let exe = build(&src, &dir, "traced");
    let t = dir.join("n.trace");
    let live = run(
        &exe,
        &[TRACED_ARG],
        TRACED_STDIN.as_bytes(),
        &[("ELYA_RECORD", &t)],
    );
    assert!(live.status.success(), "{live:?}");
    assert_eq!(
        String::from_utf8_lossy(&live.stdout),
        format!("{TRACED_OUT}1\n")
    );
    let trace = std::fs::read_to_string(&t).unwrap();
    assert_eq!(trace, at(TRACED_TRACE, &dir), "the native trace");
    // The native trace replays in the evaluator ...
    let r = elya::replay_source("t.elya", &src, &trace);
    assert!(r.error.is_none(), "{:?}", r.error);
    assert_eq!(r.output, TRACED_OUT);
    // ... and a hand-written one natively, its answers read at their types.
    let world = trace
        .replace(r#"= "a \"b\"\nc""#, r#"= "\t\\é\r""#)
        .replace("= True", "= False")
        .replace("io.arg_count() = 1\n", "");
    std::fs::write(&t, &world).unwrap();
    let again = run(&exe, &[], b"", &[("ELYA_REPLAY", &t)]);
    assert!(again.status.success(), "{again:?}");
    assert_eq!(
        String::from_utf8_lossy(&again.stdout),
        "\t\\é\r|in\n|x y\n0\n"
    );
    let eval = elya::replay_source("t.elya", &src, &world);
    assert_eq!(
        format!("{}0\n", eval.output),
        String::from_utf8_lossy(&again.stdout)
    );
    // Wrong types and divergences in the evaluator's words.
    for (from, to) in [
        ("= True", "= 1"),
        (r#"= "in\n""#, "= Unit"),
        ("io.arg(0)", "io.arg(1)"),
        ("= 1\n", "= \"1\"\n"),
    ] {
        let bad = trace.replace(from, to);
        std::fs::write(&t, &bad).unwrap();
        let native = run(&exe, &[], b"", &[("ELYA_REPLAY", &t)]);
        let e = elya::replay_source("t.elya", &src, &bad)
            .error
            .expect("fails");
        let got = failure(&native);
        assert!(
            e.contains(&got),
            "{from} -> {to}: native `{got}`, evaluator {e}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_native_replay_touches_no_file() {
    let dir = temp_dir("untouched");
    let src = at(TRACED, &dir);
    let exe = build(&src, &dir, "traced");
    let t = dir.join("e.trace");
    std::fs::write(&t, at(TRACED_TRACE, &dir)).unwrap();
    assert!(!dir.join("t.txt").exists());
    let r = run(&exe, &[], b"", &[("ELYA_REPLAY", &t)]);
    assert!(r.status.success(), "{r:?}");
    assert_eq!(
        String::from_utf8_lossy(&r.stdout),
        format!("{TRACED_OUT}1\n")
    );
    assert!(!dir.join("t.txt").exists(), "the replay wrote the file");
    std::fs::remove_dir_all(&dir).ok();
}
