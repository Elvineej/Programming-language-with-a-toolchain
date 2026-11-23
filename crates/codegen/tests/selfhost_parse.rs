//! Native parser parity and timings. Only the native runtime's final Unit value
//! line is removed; every byte printed by the Elya parser itself is compared.

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
    let corpus = support::parity_corpus(&root);
    corpus.report();
    for case in &corpus.cases {
        run_case(&executable, &scratch, case);
    }
    measure(&executable, &scratch, "clean", corpus.largest_clean());
    measure(&executable, &scratch, "broken", corpus.largest_broken());
}

fn run_case(
    executable: &std::path::Path,
    scratch: &support::Scratch,
    case: &support::Case,
) -> Duration {
    let path = scratch.put(&case.text);
    let start = Instant::now();
    let output = Command::new(executable)
        .arg(&path)
        .env_remove("ELYA_RECORD")
        .env_remove("ELYA_REPLAY")
        .output()
        .expect("run native parser");
    let elapsed = start.elapsed();
    assert!(
        output.status.success(),
        "{}: native parser: {}",
        support::case_label(case),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "{}: {output:?}", case.name);
    let stdout = String::from_utf8(output.stdout).expect("native parser output is UTF-8");
    let dump = stdout
        .strip_suffix("0\n")
        .expect("native runtime appends main's Unit value line");
    support::assert_dump(&support::case_label(case), dump, &case.dump);
    elapsed
}

fn measure(
    executable: &std::path::Path,
    scratch: &support::Scratch,
    label: &str,
    largest: &support::Case,
) {
    let native_times = (0..7)
        .map(|_| run_case(executable, scratch, largest))
        .collect();
    let mut rust_parse_times = Vec::new();
    let mut rust_dump_times = Vec::new();
    let mut rust_combined_times = Vec::new();
    for _ in 0..101 {
        let combined = Instant::now();
        let start = Instant::now();
        let (module, diagnostics) =
            elya::parse::parse_module(&elya::Session::new(), std::hint::black_box(&largest.text));
        rust_parse_times.push(start.elapsed());
        let start = Instant::now();
        let mut dump = elya::ast_dump::dump(std::hint::black_box(&module), &largest.text);
        dump.push_str(&elya::ast_dump::dump_diagnostics(&diagnostics));
        rust_dump_times.push(start.elapsed());
        rust_combined_times.push(combined.elapsed());
        support::assert_dump("Rust benchmark", &dump, &largest.dump);
    }
    println!("MEASURED {label} largest={} bytes={}; native file+lex+parse+dump+process median={:?} (7 runs); Rust lex+parse median={:?}; Rust tree+diagnostics dump median={:?}; Rust lex+parse+dump median={:?} (101 runs each; Cargo test profile, native O2)", largest.name, largest.text.len(), median(native_times), median(rust_parse_times), median(rust_dump_times), median(rust_combined_times));
}
