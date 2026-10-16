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
