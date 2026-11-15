//! Native proof and timing for the parser in Elya. Native entry points append
//! main's value line (`0\n` for Unit); that runtime line alone is removed before
//! comparing the program's printed dump with the Rust parser.

#[path = "../../../tests/support/selfhost_parse.rs"]
mod support;

use std::process::Command;
use std::time::{Duration, Instant};

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort_unstable();
    times[times.len() / 2]
}

#[test]
fn largest_input_agrees_natively_and_is_measured() {
    // Compiler passes run on the same 1 GiB stack as the CLI, independently of
    // the test runner's small default stack (including on Windows).
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(native_proof)
        .expect("native parser test thread")
        .join()
        .unwrap();
}

fn native_proof() {
    let root = support::root();
    let parser = support::parser_source();
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/parse.elya", &parser, &mut support::reader);
    assert!(
        diagnostics.is_empty(),
        "parser front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let scratch = support::Scratch::new("native");
    let (diagnostics, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diagnostics.is_empty(), "typed parser: {diagnostics:?}");
    let core = elya::core::lower_module(&module, &table).expect("parser lowers to Core");
    let object = scratch.path().join("parse.o");
    let executable = scratch
        .path()
        .join(format!("parse{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &object).expect("native parser compiles");
    elya_codegen::link(&object, &executable).expect("native parser links");
    let corpus = support::corpus(&root);
    let largest = corpus.largest();
    let path = scratch.put(&largest.text);
    let mut native_times = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        let output = Command::new(&executable)
            .arg(&path)
            .env_remove("ELYA_RECORD")
            .env_remove("ELYA_REPLAY")
            .output()
            .expect("run native parser");
        native_times.push(start.elapsed());
        assert!(
            output.status.success(),
            "native parser: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).expect("native parser output is UTF-8");
        let dump = stdout
            .strip_suffix("0\n")
            .expect("native runtime appends main's Unit value line");
        support::assert_dump(&largest.name, dump, &largest.dump);
    }
    let mut rust_parse_times = Vec::new();
    let mut rust_dump_times = Vec::new();
    for _ in 0..101 {
        let start = Instant::now();
        let (module, diagnostics) =
            elya::parse::parse_module(&elya::Session::new(), std::hint::black_box(&largest.text));
        rust_parse_times.push(start.elapsed());
        assert!(diagnostics.is_empty(), "Rust benchmark parses clean");
        let start = Instant::now();
        let dump = elya::ast_dump::dump(std::hint::black_box(&module), &largest.text);
        rust_dump_times.push(start.elapsed());
        support::assert_dump("Rust benchmark", &dump, &largest.dump);
    }
    println!(
        "MEASURED largest={} bytes={}; native file+lex+parse+dump+process median={:?} (7 runs); Rust lex+parse median={:?}; Rust dump median={:?} (101 runs each; Cargo test profile, native O2)",
        largest.name,
        largest.text.len(),
        median(native_times),
        median(rust_parse_times),
        median(rust_dump_times)
    );
}
