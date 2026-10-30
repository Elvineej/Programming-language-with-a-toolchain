//! Record and replay natively (2026-10-17, spec
//! `docs/superpowers/specs/2026-10-17-elya-native-record-replay-design.md`): a
//! native binary writes the trace `ELYA_RECORD` names and answers from the one
//! `ELYA_REPLAY` names, in the evaluator's format and with its failures' words.
//!
//! Negative controls (each reverted; the file compared byte for byte after):
//! - K1, a replay that never advances: the evaluator's trace diverges natively at
//!   entry 1, and a native trace does not replay (three tests).
//! - K2, recording writes no `time.sleep_ms` entry: the native trace has 3 lines,
//!   not 4.
//! - K3, no unread-entries check after `main`: `replay_failures...` sees exit 0
//!   where the evaluator fails.

use elya::core::lower_module;
use elya::parse::parse_module;
use elya::{run_source_recording, run_source_replaying, Session};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-trace-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn build(src: &str, dir: &Path, tag: &str) -> PathBuf {
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&session, &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    let core = lower_module(&m, &table).expect("lowers");
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
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

/// What `io.println` wrote: stdout without the value line the shim prints last.
fn text(out: &Output) -> String {
    let s = String::from_utf8(out.stdout.clone()).expect("UTF-8");
    let body = s.strip_suffix('\n').unwrap_or(&s);
    match body.rsplit_once('\n') {
        Some((t, _)) => format!("{t}\n"),
        None => String::new(),
    }
}

/// The message of a native failure: stderr is exactly `elya: <message>\n`.
fn failure(out: &Output, code: i32) -> String {
    let e = String::from_utf8(out.stderr.clone()).expect("UTF-8");
    assert_eq!(out.status.code(), Some(code), "{e}");
    e.strip_prefix("elya: ")
        .and_then(|m| m.strip_suffix('\n'))
        .unwrap_or_else(|| panic!("not a named failure: {e:?}"))
        .to_string()
}

/// Reads the clock twice around a sleep; prints the measured gap.
const CLOCK: &str = "pub fn main() {\n\
  let a = time.now_ms()\n\
  let _ = time.sleep_ms(5)\n\
  let b = time.now_ms()\n\
  let _ = io.println(if b - a >= 5 { \"slept\" } else { \"woke early\" })\n\
  io.println(int.to_string(b - a))\n\
}\n";

#[test]
fn an_evaluator_trace_replays_natively() {
    let dir = temp_dir("eval-to-native");
    let exe = build(CLOCK, &dir, "clock");
    let (trace, out) = run_source_recording("c.elya", CLOCK);
    let (trace, out) = (trace.expect("a trace"), out.expect("records"));
    let t = dir.join("t.trace");
    std::fs::write(&t, &trace).unwrap();
    let native = run(&exe, &[("ELYA_REPLAY", &t)]);
    assert!(native.status.success(), "{native:?}");
    assert_eq!(text(&native), out);
    // A hand-written trace is the world, natively too.
    std::fs::write(
        &t,
        "elya trace 1\ntime.now_ms() = 100\ntime.sleep_ms(5) = Unit\ntime.now_ms() = 250\n",
    )
    .unwrap();
    let native = run(&exe, &[("ELYA_REPLAY", &t)]);
    assert_eq!(text(&native), "slept\n150\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_native_trace_replays_in_both() {
    let dir = temp_dir("native-to-both");
    let exe = build(CLOCK, &dir, "clock");
    let t = dir.join("n.trace");
    let live = run(&exe, &[("ELYA_RECORD", &t)]);
    assert!(live.status.success(), "{live:?}");
    let trace = std::fs::read_to_string(&t).unwrap();
    let lines: Vec<&str> = trace.lines().collect();
    assert_eq!(lines.len(), 4, "{trace}");
    assert_eq!(lines[0], "elya trace 1");
    assert!(lines[1].starts_with("time.now_ms() = "), "{trace}");
    assert_eq!(lines[2], "time.sleep_ms(5) = Unit");
    let again = run(&exe, &[("ELYA_REPLAY", &t)]);
    assert_eq!(again.stdout, live.stdout);
    let eval = run_source_replaying("c.elya", CLOCK, &trace).expect("the evaluator replays it");
    assert_eq!(eval, text(&live));
    // The timers example, recorded natively, replays in the evaluator.
    let timers = include_str!("../../../examples/08_timers.elya");
    let exe = build(timers, &dir, "timers");
    let live = run(&exe, &[("ELYA_RECORD", &t)]);
    assert!(live.status.success(), "{live:?}");
    let trace = std::fs::read_to_string(&t).unwrap();
    assert!(trace.lines().count() > 10, "{trace}");
    let eval = run_source_replaying("t.elya", timers, &trace).expect("the evaluator replays it");
    assert_eq!(eval, text(&live));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn replay_failures_have_the_evaluators_words() {
    let dir = temp_dir("failures");
    let exe = build(CLOCK, &dir, "clock");
    let t = dir.join("f.trace");
    for trace in [
        "elya trace 1\ntime.now_ms() = 100\ntime.now_ms() = 101\ntime.now_ms() = 102\n",
        "elya trace 1\ntime.now_ms() = 100\n",
        "elya trace 1\ntime.now_ms() = 100\ntime.sleep_ms(5) = Unit\ntime.now_ms() = 250\ntime.now_ms() = 300\n",
        "elya trace 1\ntime.now_ms() = 100\ntime.sleep_ms(5) = Unit\ntime.now_ms() = 250\ntime.now_ms() = 300\ntime.now_ms() = 301\n",
        "elya trace 1\ntime.now_ms() = Unit\n",
        "elya trace 1\ntime.now_ms() = 99999999999999999999\n",
        "elya trace 1\ntime.now_ms() = 100\ntime.sleep_ms(5) = 0\n",
        "elya trace 1\ntime.now_ms() = 100\ntime.sleep_ms(5)\n",
    ] {
        std::fs::write(&t, trace).unwrap();
        let native = run(&exe, &[("ELYA_REPLAY", &t)]);
        let want = run_source_replaying("c.elya", CLOCK, trace).unwrap_err();
        let got = failure(&native, 1);
        assert!(want.contains(&got), "{trace:?}: native `{got}`, evaluator {want}");
    }
    // Read like the evaluator reads it: a byte-order mark, CRLF, blank lines,
    // spaces around ` = `, a `+` on an Int.
    let trace = "\u{feff}elya trace 1 \r\ntime.now_ms()  = +100\r\n\r\ntime.sleep_ms(5) = Unit \ntime.now_ms() = 250";
    std::fs::write(&t, trace).unwrap();
    let native = run(&exe, &[("ELYA_REPLAY", &t)]);
    assert_eq!(
        text(&native),
        run_source_replaying("c.elya", CLOCK, trace).unwrap()
    );
    std::fs::write(&t, "not a trace\n").unwrap();
    let native = run(&exe, &[("ELYA_REPLAY", &t)]);
    assert_eq!(
        failure(&native, 1),
        "not a trace: the first line must be `elya trace 1`"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_failing_native_run_still_writes_its_trace() {
    let src = "pub fn main() -> Int { let t = time.now_ms()  let z = t - t  io.println(\"x\")  10 / z }\n";
    let dir = temp_dir("failing");
    let exe = build(src, &dir, "fail");
    let t = dir.join("f.trace");
    let live = run(&exe, &[("ELYA_RECORD", &t)]);
    assert_eq!(failure(&live, 1), "division by zero");
    let trace = std::fs::read_to_string(&t).unwrap();
    assert_eq!(trace.lines().count(), 2, "{trace}");
    let again = run(&exe, &[("ELYA_REPLAY", &t)]);
    assert_eq!(failure(&again, 1), "division by zero");
    let eval = run_source_replaying("f.elya", src, &trace).unwrap_err();
    assert!(eval.contains("division by zero"), "{eval}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_environment_is_checked_before_the_program_runs() {
    let src = "pub fn main() -> Int { io.println(\"ran\")  1 }\n";
    let dir = temp_dir("env");
    let exe = build(src, &dir, "env");
    let t = dir.join("x.trace");
    let both = run(&exe, &[("ELYA_RECORD", &t), ("ELYA_REPLAY", &t)]);
    assert_eq!(
        failure(&both, 2),
        "ELYA_RECORD and ELYA_REPLAY cannot both be set"
    );
    assert!(both.stdout.is_empty());
    let missing = dir.join("missing.trace");
    let r = run(&exe, &[("ELYA_REPLAY", &missing)]);
    let m = failure(&r, 2);
    assert!(m.starts_with("cannot read the trace `"), "{m}");
    assert!(r.stdout.is_empty());
    let r = run(&exe, &[("ELYA_RECORD", &dir.join("no/such/dir/t"))]);
    let m = failure(&r, 2);
    assert!(m.starts_with("cannot write the trace `"), "{m}");
    // A program that asks nothing records just the header, and replays it.
    let r = run(&exe, &[("ELYA_RECORD", &t)]);
    assert!(r.status.success());
    assert_eq!(std::fs::read_to_string(&t).unwrap(), "elya trace 1\n");
    let r = run(&exe, &[("ELYA_REPLAY", &t)]);
    assert_eq!(String::from_utf8_lossy(&r.stdout), "ran\n1\n");
    std::fs::remove_dir_all(&dir).ok();
}

// ---- The review (2026-10-17) -------------------------------------------------

/// The replay of `trace` (bytes) natively and in the evaluator (as the CLI
/// reads a trace file: UTF-8 or refused): the same output, or both failing.
fn both_read(exe: &Path, dir: &Path, trace: &[u8]) -> (Output, Result<String, String>) {
    let t = dir.join("b.trace");
    std::fs::write(&t, trace).unwrap();
    let native = run(exe, &[("ELYA_REPLAY", &t)]);
    let eval = match std::str::from_utf8(trace) {
        Ok(s) => run_source_replaying("c.elya", CLOCK, s),
        Err(_) => Err("stream did not contain valid UTF-8".to_string()),
    };
    (native, eval)
}

#[test]
fn the_trace_is_read_as_the_evaluator_reads_it() {
    // M2 (Unicode whitespace trimmed, as `str::trim` trims), M3 (a trace that is
    // not UTF-8 is unreadable, exit 2), L1 (a NUL byte is a character).
    let dir = temp_dir("read");
    let exe = build(CLOCK, &dir, "clock");
    let good = "time.now_ms() = 100\ntime.sleep_ms(5) = Unit\ntime.now_ms() = 250\n";
    for ws in [
        "\u{a0}", "\u{2003}", "\u{85}", "\u{3000}", "\u{2029}", "\u{205f}",
    ] {
        for trace in [
            format!("elya trace 1{ws}\n{good}"),
            format!("elya trace 1\n{ws}\n{good}"),
            format!("elya trace 1\n{ws}time.now_ms() = 100{ws}\ntime.sleep_ms(5) = Unit\ntime.now_ms() = 250{ws}\n"),
        ] {
            let (native, eval) = both_read(&exe, &dir, trace.as_bytes());
            let eval = eval.unwrap_or_else(|e| panic!("{trace:?}: the evaluator refused: {e}"));
            assert!(native.status.success(), "{trace:?}: {native:?}");
            assert_eq!(text(&native), eval, "{trace:?}");
        }
    }
    let mut bad = format!("elya trace 1\n{good}").into_bytes();
    bad.extend_from_slice(b"#\xff = 1\n");
    let (native, eval) = both_read(&exe, &dir, &bad);
    assert!(eval.is_err());
    let m = failure(&native, 2);
    assert!(m.ends_with("stream did not contain valid UTF-8"), "{m}");
    for trace in [
        "elya trace 1\0junk\ntime.now_ms() = 100\n".to_string(),
        "elya trace 1\ntime.now_ms() = 5\0x\n".to_string(),
        "elya trace 1\ntime.now_ms() = 100\ntime.sleep_ms(5)\0zz = Unit\ntime.now_ms() = 250\n"
            .to_string(),
    ] {
        let (native, eval) = both_read(&exe, &dir, trace.as_bytes());
        let want = eval.unwrap_err();
        let got = failure(&native, 1);
        assert!(
            want.contains(&got),
            "{trace:?}: native `{got}`, evaluator {want}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_trace_that_cannot_be_written_stops_the_run() {
    // M1: a write that fails mid-run (a full disk) is named, exit 2, as the CLI
    // names one.
    let full = Path::new("/dev/full");
    if !full.exists() {
        return; // no such device on this platform; the open-time refusal is tested above
    }
    let dir = temp_dir("full");
    let exe = build(CLOCK, &dir, "clock");
    let r = run(&exe, &[("ELYA_RECORD", full)]);
    let m = failure(&r, 2);
    assert!(m.starts_with("cannot write the trace `/dev/full`: "), "{m}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_native_reader_splits_at_the_first_separator_outside_quotes() {
    // Spec 2026-10-24-elya-trace-text D2, natively: the same failure in the
    // same words as the evaluator's reader, for an answer holding ` = ` and for
    // a line that ends inside quotes.
    let dir = temp_dir("first-split");
    let exe = build(CLOCK, &dir, "clock");
    for trace in [
        "elya trace 1\ntime.now_ms() = \"a = b\"\n",
        "elya trace 1\ntime.now_ms(\"x) = 1\n",
    ] {
        let (native, eval) = both_read(&exe, &dir, trace.as_bytes());
        let want = eval.unwrap_err();
        let got = failure(&native, 1);
        assert!(
            want.contains(&got),
            "{trace:?}: native `{got}`, evaluator {want}"
        );
        assert!(!got.contains("diverged"), "{trace:?}: {got}");
    }
    std::fs::remove_dir_all(&dir).ok();
}
