//! `elya run --record` / `--replay` (spec 2026-10-16-elya-cli-record-replay):
//! the user-facing entry point, through the real binary.

use std::process::Command;

#[test]
fn elya_run_records_and_replays_a_trace() {
    let dir = std::env::temp_dir().join(format!("elya-run-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let prog = dir.join("clock.elya");
    std::fs::write(
        &prog,
        "pub fn main() {\n  let a = time.now_ms()\n  let _ = time.sleep_ms(3)\n  io.println(int.to_string(time.now_ms() - a))\n}\n",
    )
    .expect("write program");
    let trace = dir.join("trace.txt");
    let rec = Command::new(env!("CARGO_BIN_EXE_elya"))
        .arg("run")
        .arg("--record")
        .arg(&trace)
        .arg(&prog)
        .output()
        .expect("spawn elya run --record");
    assert!(rec.status.success(), "{rec:?}");
    let text = std::fs::read_to_string(&trace).expect("trace written");
    assert!(text.starts_with("elya trace 1\n"), "{text}");
    let rep = Command::new(env!("CARGO_BIN_EXE_elya"))
        .arg("run")
        .arg("--replay")
        .arg(&trace)
        .arg(&prog)
        .output()
        .expect("spawn elya run --replay");
    assert!(rep.status.success(), "{rep:?}");
    assert_eq!(rep.stdout, rec.stdout, "the replay printed something else");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn elya_run_refuses_misplaced_trace_arguments() {
    // The review's findings: a flag after the program ran live and wrote no
    // trace; a flag or the program itself as the trace path was accepted (and
    // `--record prog.elya prog.elya` would overwrite the source).
    let dir = std::env::temp_dir().join(format!("elya-run-cli-args-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let prog = dir.join("p.elya");
    let src = "pub fn main() { io.println(\"x\") }\n";
    std::fs::write(&prog, src).expect("write program");
    let p = prog.to_str().expect("utf-8 path");
    for args in [
        vec!["run", p, "--record", "t.tr"],
        vec!["run", "--record", "--replay", p],
        vec!["run", "--record", p, p],
        vec!["run", "--record", "t.tr", p, "extra"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_elya"))
            .args(&args)
            .current_dir(&dir)
            .output()
            .expect("spawn elya");
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
    }
    assert_eq!(std::fs::read_to_string(&prog).expect("still there"), src);
    std::fs::remove_dir_all(&dir).ok();
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-run-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Spec 2026-10-16-elya-differential-sweep D1: what a program printed before a
/// runtime error stays printed (it was dropped: the error path returned only
/// the diagnostic), as a native binary keeps it.
#[test]
fn elya_run_keeps_output_written_before_a_runtime_error() {
    let dir = scratch("keep");
    let prog = dir.join("div.elya");
    std::fs::write(
        &prog,
        "fn z(x) { x - x }\npub fn main() -> Int { io.println(\"before\")  1 / z(3) }\n",
    )
    .expect("write program");
    let out = Command::new(env!("CARGO_BIN_EXE_elya"))
        .arg("run")
        .arg(&prog)
        .output()
        .expect("spawn elya run");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "before\n");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("division by zero"),
        "{out:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// D2: `elya run --value` ends with main's value as the native shim prints it.
#[test]
fn elya_run_value_prints_main_as_a_native_binary_does() {
    let dir = scratch("value");
    let run = |name: &str, src: &str, args: &[&str]| {
        let prog = dir.join(name);
        std::fs::write(&prog, src).expect("write program");
        Command::new(env!("CARGO_BIN_EXE_elya"))
            .arg("run")
            .args(args)
            .arg(&prog)
            .output()
            .expect("spawn elya run --value")
    };
    let int = run(
        "int.elya",
        "pub fn main() -> Int { io.println(\"a\")  40 + 2 }\n",
        &["--value"],
    );
    assert!(int.status.success(), "{int:?}");
    assert_eq!(String::from_utf8_lossy(&int.stdout), "a\n42\n");
    let unit = run(
        "unit.elya",
        "pub fn main() { io.println(\"x\") }\n",
        &["--value"],
    );
    assert!(unit.status.success(), "{unit:?}");
    assert_eq!(String::from_utf8_lossy(&unit.stdout), "x\n0\n");
    let boolean = run(
        "bool.elya",
        "pub fn main() -> Bool { True }\n",
        &["--value"],
    );
    assert_eq!(boolean.status.code(), Some(1), "{boolean:?}");
    assert!(
        String::from_utf8_lossy(&boolean.stderr).contains("main's value is not an `Int` or `Unit`"),
        "{boolean:?}"
    );
    // A runtime error: the output so far, then no value line.
    let failing = run(
        "fail.elya",
        "fn z(x) { x - x }\npub fn main() -> Int { io.println(\"b\")  1 / z(1) }\n",
        &["--value"],
    );
    assert_eq!(failing.status.code(), Some(1), "{failing:?}");
    assert_eq!(String::from_utf8_lossy(&failing.stdout), "b\n");
    std::fs::remove_dir_all(&dir).ok();
}

// ---- A failing traced run keeps its output on stdout (2026-10-23, spec
// `2026-10-23-elya-traced-run-output-design.md` D1) ------------------------------
//
// Negative control (reverted, `cmp` against a saved copy): `trace_cmd` printing the
// output inside the error on failure (the old shape) fails both tests below with
// an empty stdout.

const FAILS: &str =
    "pub fn main() -> Int { let t = time.now_ms()  let z = t - t  io.println(\"x\")  10 / z }\n";
const ASKS: &str = "pub fn main() -> Int { io.println(\"y\")  let t = time.now_ms()  t }\n";

fn elya_run(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_elya"))
        .arg("run")
        .args(args)
        .output()
        .expect("spawn elya run")
}

#[test]
fn a_failing_recorded_run_keeps_its_output_on_stdout() {
    let dir = scratch("rec-fail");
    let prog = dir.join("f.elya");
    std::fs::write(&prog, FAILS).expect("write program");
    let trace = dir.join("f.trace");
    let out = elya_run(&["--record".as_ref(), trace.as_os_str(), prog.as_os_str()]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "x\n");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("division by zero"), "{err}");
    assert!(!err.starts_with('x'), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_failing_replayed_run_keeps_its_output_on_stdout() {
    let dir = scratch("rep-fail");
    let prog = dir.join("f.elya");
    std::fs::write(&prog, FAILS).expect("write program");
    let trace = dir.join("f.trace");
    let rec = elya_run(&["--record".as_ref(), trace.as_os_str(), prog.as_os_str()]);
    assert_eq!(rec.status.code(), Some(1), "{rec:?}");
    let rep = elya_run(&["--replay".as_ref(), trace.as_os_str(), prog.as_os_str()]);
    assert_eq!(rep.status.code(), Some(1), "{rep:?}");
    assert_eq!(String::from_utf8_lossy(&rep.stdout), "x\n");
    assert!(String::from_utf8_lossy(&rep.stderr).contains("division by zero"));
    // A divergence: the text before it stays too.
    let asks = dir.join("g.elya");
    std::fs::write(&asks, ASKS).expect("write program");
    let short = dir.join("short.trace");
    std::fs::write(&short, "elya trace 1\n").expect("write trace");
    let div = elya_run(&["--replay".as_ref(), short.as_os_str(), asks.as_os_str()]);
    assert_eq!(div.status.code(), Some(1), "{div:?}");
    assert_eq!(String::from_utf8_lossy(&div.stdout), "y\n");
    assert!(String::from_utf8_lossy(&div.stderr).contains("replay diverged"));
    std::fs::remove_dir_all(&dir).ok();
}
