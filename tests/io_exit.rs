// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! `io.exit` in the evaluator (spec
//! `docs/superpowers/specs/2026-11-28-elya-io-exit-design.md` D1-D3): the run
//! ends with the status, its output kept, no value and no error; a status
//! outside 0..255 fails in the native runtime's words; nothing is recorded.

#[allow(dead_code)] // each suite uses some of the programs
#[path = "programs/io_exit.rs"]
mod programs;
use programs::*;

fn run(src: &str) -> elya::RunOutcome {
    elya::run_source_outcome_host("main.elya", src, &mut |_| None, elya::eval::Host::default())
}

#[test]
fn io_exit_ends_the_run_with_its_status() {
    for (src, out, status) in [
        (MIDDLE, "before\n", 3),
        (DEEP, "x\n", 0),
        (HANDLER, "in\n", 7),
    ] {
        let o = run(src);
        assert!(o.error.is_none(), "{src:?}: {:?}", o.error);
        assert_eq!(
            (o.output.as_str(), o.status),
            (out, Some(status)),
            "{src:?}"
        );
        assert!(o.line.is_none() && o.value.is_none(), "{src:?}: a value");
    }
}

#[test]
fn io_exit_outside_its_range_fails_by_name() {
    for (src, words) in [(HIGH, HIGH_ERR), (LOW, LOW_ERR)] {
        let o = run(src);
        assert_eq!(o.status, None, "{src:?}");
        let e = o.error.expect("a failure");
        let want = words.trim_end().strip_prefix("elya: ").expect("the prefix");
        assert!(e.contains(want), "{src:?}: {e}");
    }
}

#[test]
fn io_exit_is_not_recorded_and_a_replay_exits_alike() {
    let (trace, o) = elya::record_source("main.elya", MIDDLE);
    assert_eq!(o.status, Some(3));
    let trace = trace.expect("a trace");
    assert!(!trace.contains("io.exit"), "{trace}");
    let r = elya::replay_source("main.elya", MIDDLE, &trace);
    assert_eq!((r.output.as_str(), r.status), ("before\n", Some(3)));
}
