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
