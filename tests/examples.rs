//! Runs each example program and snapshots its output.

fn run_example(rel: &str) -> String {
    let path = format!("{}/examples/{rel}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).expect("read example");
    elya::run_source(rel, &text).expect("example should run cleanly")
}

#[test]
fn hello() {
    insta::assert_snapshot!("01_hello", run_example("01_hello.elya"));
}

#[test]
fn arith() {
    insta::assert_snapshot!("02_arith", run_example("02_arith.elya"));
}

#[test]
fn cek() {
    // A CEK machine written in Elya, running a lambda calculus (2026-10-05).
    insta::assert_snapshot!("03_cek", run_example("03_cek.elya"));
}

#[test]
fn async_scheduler() {
    // Async as an effect: a round-robin scheduler written as an ordinary
    // handler, and one `each` for sync and async code (2026-10-09).
    insta::assert_snapshot!("04_async", run_example("04_async.elya"));
}

#[test]
fn replay() {
    // Exact replay and handler-based testing: record, replay and a test double,
    // each an ordinary handler over the same program (2026-10-10).
    insta::assert_snapshot!("05_replay", run_example("05_replay.elya"));
}
