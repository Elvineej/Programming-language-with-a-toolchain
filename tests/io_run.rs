//! Running a program (2026-11-27, spec
//! `docs/superpowers/specs/2026-11-27-elya-the-switch-design.md` D1) in the
//! evaluator: `io.run` answers the exit status and fails by name in the
//! native runtime's words. In the evaluator a program's own output is
//! printed when its run ends, so only the statuses are compared here; the
//! order of the streams is the native suite's
//! (crates/codegen/tests/native_io_run.rs).

#[allow(dead_code)] // each suite uses some of the programs
#[path = "programs/io_run.rs"]
mod programs;
use programs::*;

fn run(src: &str) -> elya::RunOutcome {
    elya::run_source_outcome_host("main.elya", src, &mut |_| None, elya::eval::Host::default())
}

#[test]
fn io_run_answers_exit_statuses() {
    let o = run(&statuses());
    assert!(o.error.is_none(), "{:?}", o.error);
    assert_eq!(o.output, "0\n3\nbefore\nafter 0\n");
}

#[test]
fn io_run_fails_by_name() {
    for (src, words) in [(MISSING, MISSING_ERR), (EMPTY, EMPTY_ERR), (NUL, NUL_ERR)] {
        let o = run(src);
        let e = format!("{:?}", o.error.expect("a failure"));
        let want = words
            .trim_end()
            .strip_prefix("elya: ")
            .expect("the runtime's prefix");
        assert!(
            e.contains(&want.replace('\0', "\\0")) || e.contains(want),
            "{src:?}: {e}"
        );
    }
}
