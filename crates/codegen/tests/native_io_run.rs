//! Running a program natively (2026-11-27, spec
//! `docs/superpowers/specs/2026-11-27-elya-the-switch-design.md` D1): the
//! statuses the evaluator answers, the child's output in order with the
//! program's, the failures in the evaluator's words, and a replay running
//! nothing.

#[allow(dead_code)] // each suite uses some of the programs
#[path = "../../../tests/programs/io_run.rs"]
mod programs;
use programs::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-nrun-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn build(src: &str, dir: &Path) -> PathBuf {
    let (m, pd, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    assert!(pd.is_empty(), "front end: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    let core = elya::core::lower_module(&m, &table).expect("lowers");
    let obj = dir.join("p.o");
    let exe = dir.join(format!("p{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).expect("compile_module");
    elya_codegen::link(&obj, &exe).expect("link");
    exe
}

fn run(exe: &Path, env: &[(&str, &Path)]) -> Output {
    let mut c = Command::new(exe);
    c.env_remove("ELYA_RECORD").env_remove("ELYA_REPLAY");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run the binary")
}

#[test]
fn io_run_natively_answers_statuses_in_order() {
    let dir = temp_dir("statuses");
    let o = run(&build(&statuses(), &dir), &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    // The child's own line ends as its platform's do (`\r\n` on Windows).
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n"),
        STATUSES_OUT
    );
}

#[test]
fn io_run_natively_fails_by_name() {
    for (i, (src, words, out)) in [
        (MISSING, MISSING_ERR, "x\n"),
        (EMPTY, EMPTY_ERR, ""),
        (NUL, NUL_ERR, ""),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = temp_dir(&format!("fails{i}"));
        let o = run(&build(src, &dir), &[]);
        assert_eq!(o.status.code(), Some(1), "{src:?}");
        assert_eq!(String::from_utf8_lossy(&o.stdout), out, "{src:?}");
        assert_eq!(String::from_utf8_lossy(&o.stderr), words, "{src:?}");
    }
}

#[test]
fn io_run_is_recorded_and_a_replay_runs_nothing() {
    let dir = temp_dir("replay");
    let mark = dir.join("ran.txt");
    let mark_text = mark.display().to_string().replace('\\', "/");
    let src = format!(
        "pub fn main() -> Int {{ io.run(\"{}\") }}\n",
        shell(&format!("echo x > {mark_text}"))
    );
    let exe = build(&src, &dir);
    let trace = dir.join("t.trace");
    let first = run(&exe, &[("ELYA_RECORD", &trace)]);
    assert!(first.status.success());
    assert!(mark.exists(), "the program ran");
    let recorded = std::fs::read_to_string(&trace).expect("a trace");
    assert!(
        recorded.contains("io.run(") && recorded.contains(") = 0"),
        "{recorded}"
    );
    std::fs::remove_file(&mark).expect("remove the mark");
    let again = run(&exe, &[("ELYA_REPLAY", &trace)]);
    assert_eq!(again.stdout, first.stdout);
    assert!(!mark.exists(), "a replay runs nothing");
}
