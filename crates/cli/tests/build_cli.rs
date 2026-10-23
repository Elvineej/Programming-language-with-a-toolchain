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

/// Found by the 2026-10-16 sweep: `elya build` asserted that inference returned
/// no diagnostics at all, so a program with only a warning (E0426) panicked
/// the debug binary instead of building.
#[test]
fn a_program_with_only_warnings_builds() {
    let dir = temp_dir("warning-only");
    let src = dir.join("multi.elya");
    std::fs::write(
        &src,
        "effect multi Flip { fn flip() -> Bool }\n\
         fn noisy() { let x = flip()  let _ = io.println(\"tick\")  x }\n\
         pub fn main() {\n\
         \x20 let _ = handle noisy() with multi { Flip.flip() -> resume(True) }\n\
         \x20 io.println(\"done\")\n\
         }\n",
    )
    .expect("write program");
    let exe = dir.join(format!("multi{}", std::env::consts::EXE_SUFFIX));
    let out = Command::new(env!("CARGO_BIN_EXE_elya"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("spawn elya build");
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("E0426"),
        "the warning is still shown: {out:?}"
    );
    let run = Command::new(&exe).output().expect("run produced binary");
    assert!(run.status.success(), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "tick\ndone\n0\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// Spec 2026-10-23-elya-traced-run-output D1: a traced run prints on stdout what
/// the native binary prints, and exits alike, failing or not.
#[test]
fn a_traced_run_prints_what_the_native_binary_prints() {
    let dir = temp_dir("traced");
    let fails = "pub fn main() -> Int { let t = time.now_ms()  let z = t - t  io.println(\"x\")  10 / z }\n";
    let asks = "pub fn main() -> Int { io.println(\"y\")  let t = time.now_ms()  t }\n";
    let mut exes = Vec::new();
    for (name, src) in [("f", fails), ("g", asks)] {
        let prog = dir.join(format!("{name}.elya"));
        std::fs::write(&prog, src).unwrap();
        let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        elya_build(&[prog.as_os_str(), "-o".as_ref(), exe.as_os_str()]);
        exes.push((prog, exe));
    }
    let cli = |flag: &str, trace: &Path, prog: &Path| {
        Command::new(env!("CARGO_BIN_EXE_elya"))
            .arg("run")
            .arg(flag)
            .arg(trace)
            .arg(prog)
            .output()
            .expect("spawn elya run")
    };
    let native = |var: &str, trace: &Path, exe: &Path| {
        Command::new(exe)
            .env(var, trace)
            .output()
            .expect("run the binary")
    };
    let same = |what: &str, c: &std::process::Output, n: &std::process::Output| {
        assert_eq!(c.status.code(), n.status.code(), "{what}: {c:?} {n:?}");
        // The native binary ends a successful run with main's value line.
        let n_out = String::from_utf8_lossy(&n.stdout).into_owned();
        let n_text = if n.status.success() {
            n_out
                .trim_end_matches('\n')
                .rsplit_once('\n')
                .map_or(String::new(), |(t, _)| format!("{t}\n"))
        } else {
            n_out
        };
        assert_eq!(String::from_utf8_lossy(&c.stdout), n_text, "{what}");
    };
    let (f, f_exe) = &exes[0];
    let (g, g_exe) = &exes[1];
    let t_cli = dir.join("cli.trace");
    let t_nat = dir.join("native.trace");
    same(
        "a failing record",
        &cli("--record", &t_cli, f),
        &native("ELYA_RECORD", &t_nat, f_exe),
    );
    same(
        "its replay",
        &cli("--replay", &t_cli, f),
        &native("ELYA_REPLAY", &t_cli, f_exe),
    );
    let short = dir.join("short.trace");
    std::fs::write(&short, "elya trace 1\n").unwrap();
    same(
        "a diverging replay",
        &cli("--replay", &short, g),
        &native("ELYA_REPLAY", &short, g_exe),
    );
    let full = dir.join("full.trace");
    std::fs::write(&full, "elya trace 1\ntime.now_ms() = 7\n").unwrap();
    same(
        "a successful replay",
        &cli("--replay", &full, g),
        &native("ELYA_REPLAY", &full, g_exe),
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Modules part 1 (spec `2026-10-23-elya-modules-part-1-design.md` D7): `elya
/// build` reads a program's imports, and the binary prints what `elya run` does.
#[test]
fn elya_build_reads_imported_files() {
    let dir = temp_dir("modules");
    std::fs::write(
        dir.join("util.elya"),
        "fn helper() -> Int { 2 }\npub fn twice(x: Int) -> Int { x * helper() }\n",
    )
    .unwrap();
    let prog = dir.join("main.elya");
    std::fs::write(
        &prog,
        "import util\nfn helper() -> Int { 1 }\npub fn main() -> Int { io.println(\"go\")  util.twice(20) + helper() }\n",
    )
    .unwrap();
    let exe = dir.join(format!("main{}", std::env::consts::EXE_SUFFIX));
    elya_build(&[prog.as_os_str(), "-o".as_ref(), exe.as_os_str()]);
    let native = Command::new(&exe).output().expect("run the binary");
    assert!(native.status.success(), "{native:?}");
    assert_eq!(String::from_utf8_lossy(&native.stdout), "go\n41\n");
    let run = Command::new(env!("CARGO_BIN_EXE_elya"))
        .args(["run".as_ref(), "--value".as_ref(), prog.as_os_str()])
        .output()
        .expect("spawn elya run --value");
    assert_eq!(run.stdout, native.stdout);
    std::fs::remove_dir_all(&dir).ok();
}
