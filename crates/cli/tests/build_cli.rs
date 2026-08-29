//! Slice 5b-1 Task 5 — the CLI-driven proof. Everything above this file proves
//! the *library* path (Core → LLVM → object → link → run); this proves the
//! user-facing entry point: a real `elya build prog.elya` producing a native
//! binary that runs and prints the right number.
//!
//! Deliberately self-contained: it spawns the `elya` binary and then the binary
//! that `elya` produced, so it depends on neither `elya` nor `elya-codegen` and
//! therefore never links LLVM. Proof is execution (spec §0) — no IR is inspected,
//! and nothing here can skip.
#![cfg(feature = "codegen")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-cli-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn assert_runs(exe: &Path, expected: &str) {
    let out = Command::new(exe).output().expect("run produced binary");
    assert!(out.status.success(), "binary exited {:?}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected);
    assert!(
        String::from_utf8_lossy(&out.stderr).is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Invoke `elya build` and assert it succeeded, returning its stdout.
fn elya_build(args: &[&std::ffi::OsStr]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_elya"))
        .arg("build")
        .args(args)
        .output()
        .expect("spawn elya build");
    assert!(
        out.status.success(),
        "elya build failed ({:?})\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).is_empty(),
        "elya build wrote to stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn elya_build_cli_produces_runnable_binary() {
    // The strongest case drives the REAL CLI (§5): the proof covers the
    // user-facing entry point, not just the library API.
    let dir = temp_dir("explicit-out");
    let src = dir.join("prog.elya");
    std::fs::write(&src, "pub fn main() { 1 + 2 }\n").expect("write source");
    let exe = dir.join(format!("prog{}", std::env::consts::EXE_SUFFIX));

    let stdout = elya_build(&[src.as_os_str(), "-o".as_ref(), exe.as_os_str()]);
    assert!(stdout.contains("built"), "unexpected stdout: {stdout}");
    assert_runs(&exe, "3");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn elya_build_defaults_output_next_to_the_source() {
    // With no `-o`, the output is the input stem plus the platform exe suffix.
    let dir = temp_dir("default-out");
    let src = dir.join("prog.elya");
    let prog = "pub fn main() {
  let x = 6
  let y = 7
  x * y
}
";
    std::fs::write(&src, prog).expect("write source");

    elya_build(&[src.as_os_str()]);

    let exe = dir.join(format!("prog{}", std::env::consts::EXE_SUFFIX));
    assert!(exe.exists(), "expected default output at {}", exe.display());
    assert_runs(&exe, "42");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn elya_build_reports_front_end_errors_and_emits_nothing() {
    // `elya build` runs the front end exactly as `elya check`: a type error is
    // rendered to stderr, the exit status is failure, and no binary appears.
    let dir = temp_dir("bad");
    let src = dir.join("bad.elya");
    std::fs::write(&src, "pub fn main() { 1 + nope }\n").expect("write source");
    let exe = dir.join(format!("bad{}", std::env::consts::EXE_SUFFIX));

    let out = Command::new(env!("CARGO_BIN_EXE_elya"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("spawn elya build");

    assert!(!out.status.success(), "a broken program must not build");
    assert!(
        !String::from_utf8_lossy(&out.stderr).is_empty(),
        "expected a rendered diagnostic on stderr"
    );
    assert!(
        !exe.exists(),
        "no binary may be produced for a broken program"
    );

    std::fs::remove_dir_all(&dir).ok();
}
