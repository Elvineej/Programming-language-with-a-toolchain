//! Record and replay from the command line (spec
//! `docs/superpowers/specs/2026-10-16-elya-cli-record-replay-design.md`): the
//! trace is the host's answers, and replaying them reproduces the run.

use elya::{run_source_recording, run_source_replaying};

/// Reads the clock twice around a sleep; prints the measured gap, which
/// differs between live runs.
const CLOCK: &str = "pub fn main() {\n\
  let a = time.now_ms()\n\
  let _ = time.sleep_ms(5)\n\
  let b = time.now_ms()\n\
  let _ = io.println(if b - a >= 5 { \"slept\" } else { \"woke early\" })\n\
  io.println(int.to_string(b - a))\n\
}\n";

#[test]
fn a_recorded_run_replays_to_the_same_output() {
    let (trace, out) = run_source_recording("c.elya", CLOCK);
    let (trace, out) = (trace.expect("a trace"), out.expect("records"));
    assert!(out.starts_with("slept\n"), "{out}");
    let lines: Vec<&str> = trace.lines().collect();
    assert_eq!(lines[0], "elya trace 1");
    assert_eq!(lines.len(), 4, "{trace}");
    assert!(lines[1].starts_with("time.now_ms() = "), "{trace}");
    assert_eq!(lines[2], "time.sleep_ms(5) = Unit");
    let again = run_source_replaying("c.elya", CLOCK, &trace).expect("replays");
    assert_eq!(again, out);
}

#[test]
fn the_trace_is_the_world() {
    // A trace written by hand: the program sees exactly these answers.
    let trace = "elya trace 1\ntime.now_ms() = 100\ntime.sleep_ms(5) = Unit\ntime.now_ms() = 250\n";
    let out = run_source_replaying("c.elya", CLOCK, trace).expect("replays");
    assert_eq!(out, "slept\n150\n");
}

#[test]
fn a_divergence_stops_by_name_at_its_entry() {
    let trace = "elya trace 1\ntime.now_ms() = 100\ntime.now_ms() = 101\ntime.now_ms() = 102\n";
    let err = run_source_replaying("c.elya", CLOCK, trace).unwrap_err();
    assert!(err.contains("replay diverged at entry 2"), "{err}");
    assert!(err.contains("time.sleep_ms(5)"), "{err}");
    let short = "elya trace 1\ntime.now_ms() = 100\n";
    let err = run_source_replaying("c.elya", CLOCK, short).unwrap_err();
    assert!(err.contains("the trace has ended"), "{err}");
}

#[test]
fn unread_entries_are_an_error() {
    let trace = "elya trace 1\ntime.now_ms() = 100\ntime.sleep_ms(5) = Unit\ntime.now_ms() = 250\ntime.now_ms() = 300\n";
    let err = run_source_replaying("c.elya", CLOCK, trace).unwrap_err();
    assert!(err.contains("1 unread trace entry, from entry 4"), "{err}");
    assert!(err.contains("time.now_ms() = 300"), "{err}");
}

#[test]
fn an_answer_of_the_wrong_type_is_refused() {
    let trace = "elya trace 1\ntime.now_ms() = Unit\n";
    let err = run_source_replaying("c.elya", CLOCK, trace).unwrap_err();
    assert!(err.contains("not an answer of type Int"), "{err}");
    let err = run_source_replaying("c.elya", CLOCK, "not a trace\n").unwrap_err();
    assert!(err.contains("elya trace 1"), "{err}");
}

#[test]
fn a_failing_run_still_writes_its_trace() {
    // The review's finding: the trace was written only when `main` returned,
    // and a failing run is the one most worth replaying.
    let src = "pub fn main() -> Int { let t = time.now_ms()  let z = t - t  10 / z }\n";
    let (trace, out) = run_source_recording("f.elya", src);
    let trace = trace.expect("a trace even though the run failed");
    assert!(out.unwrap_err().contains("division by zero"));
    assert_eq!(trace.lines().count(), 2, "{trace}");
    let err = run_source_replaying("f.elya", src, &trace).unwrap_err();
    assert!(
        err.contains("division by zero"),
        "the replay fails the same way: {err}"
    );
}

#[test]
fn a_trace_is_read_ignoring_surrounding_whitespace() {
    let trace = "\u{feff}elya trace 1 \r\ntime.now_ms()  = 100\r\n\r\ntime.sleep_ms(5) = Unit \ntime.now_ms() = 250";
    let out = run_source_replaying("c.elya", CLOCK, trace).expect("replays");
    assert_eq!(out, "slept\n150\n");
}

#[test]
fn a_line_splits_at_the_first_separator_outside_quotes() {
    // Spec 2026-10-24-elya-trace-text D2: the question is `time.now_ms()` and
    // the answer `"a = b"`, which is not an `Int`. The last ` = ` would have
    // made the question `time.now_ms() = "a`, a divergence.
    let trace = "elya trace 1\ntime.now_ms() = \"a = b\"\n";
    let err = run_source_replaying("c.elya", CLOCK, trace).unwrap_err();
    assert!(err.contains("not an answer of type Int"), "{err}");
    assert!(!err.contains("diverged"), "{err}");
    // A line that ends inside quotes before any separator.
    let trace = "elya trace 1\ntime.now_ms(\"x) = 1\n";
    let err = run_source_replaying("c.elya", CLOCK, trace).unwrap_err();
    assert!(
        err.contains("trace line 2: a string is not closed"),
        "{err}"
    );
}
