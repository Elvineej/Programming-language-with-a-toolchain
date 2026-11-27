//! Modules and name resolution in Elya, natively (self-hosting part 4c,
//! spec docs/superpowers/specs/2026-11-15-elya-resolution-in-elya-design.md
//! D4, D5): `selfhost/resolve.elya` compiled and run on every program the
//! evaluator's test compares, each equal to `elya resolve --dump` byte for
//! byte (only the native runtime's final Unit value line is removed), and
//! the largest program timed against Rust.

#[path = "../../../tests/support/selfhost_parse.rs"]
mod parse_support;
#[path = "../../../tests/support/selfhost_resolve.rs"]
mod support;

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort_unstable();
    times[times.len() / 2]
}

#[test]
fn every_program_agrees_natively_and_the_largest_is_measured() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(native_proof)
        .expect("native resolver test thread")
        .join()
        .unwrap();
}

fn run(executable: &Path, root: &Path, std: &Path, name: &str, want: &str) -> Duration {
    let start = Instant::now();
    let output = Command::new(executable)
        .arg(support::elya_path(root))
        .arg(support::elya_path(std))
        .env_remove("ELYA_RECORD")
        .env_remove("ELYA_REPLAY")
        .output()
        .expect("run the native resolver");
    let elapsed = start.elapsed();
    assert!(
        output.status.success(),
        "{name}: native resolver: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "{name}: {output:?}");
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 output");
    let dump = stdout
        .strip_suffix("0\n")
        .expect("the native runtime appends main's Unit value line");
    parse_support::assert_dump(name, dump, want);
    elapsed
}

fn native_proof() {
    let root = parse_support::root();
    let source = support::resolver_source();
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/resolve.elya", &source, &mut support::reader);
    assert!(
        diagnostics.is_empty(),
        "resolver front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let scratch = parse_support::Scratch::new("resolve-native");
    let (diagnostics, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diagnostics.is_empty(), "typed resolver: {diagnostics:?}");
    let core = elya::core::lower_module(&module, &table).expect("the resolver lowers to Core");
    let object = scratch.path().join("resolve.o");
    let executable = scratch
        .path()
        .join(format!("resolve{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &object).expect("the native resolver compiles");
    elya_codegen::link(&object, &executable).expect("the native resolver links");
    let std = root.join("src/std");
    let corpus = support::corpus_programs(&root);
    let multi = support::multi_file_programs(&root);
    let focused = support::focused();
    let mut compared = 0;
    for (i, p) in corpus.iter().chain(&multi).chain(&focused).enumerate() {
        let path = support::materialise(scratch.path(), i, p);
        let (want, _) = support::oracle(&path);
        run(&executable, &path, &std, &p.name, &want);
        compared += 1;
    }
    // The largest program: the checker in Elya with the parser and the
    // lexer it imports, in place.
    let largest = root.join("selfhost/check.elya");
    let (want, _) = support::oracle(&largest);
    let native: Vec<_> = (0..7)
        .map(|_| run(&executable, &largest, &std, "selfhost/check.elya", &want))
        .collect();
    let text = std::fs::read_to_string(&largest).expect("the checker");
    let name = largest.display().to_string();
    let mut rust_resolve = Vec::new();
    let mut rust_all = Vec::new();
    for _ in 0..101 {
        let all = Instant::now();
        let start = Instant::now();
        let dir = largest.parent().unwrap().to_path_buf();
        let mut read = |module: &str| {
            let file = dir.join(format!("{module}.elya"));
            std::fs::read_to_string(&file)
                .ok()
                .map(|t| (file.display().to_string(), t))
        };
        let (program, diags) = elya::resolved(
            &elya::Session::new(),
            &name,
            std::hint::black_box(&text),
            &mut read,
        );
        rust_resolve.push(start.elapsed());
        let mut dump = elya::ast_dump::dump(&program.module, &elya::merged_text(&program.sources));
        dump.push_str(&elya::ast_dump::dump_diagnostics(&diags));
        rust_all.push(all.elapsed());
        parse_support::assert_dump("Rust benchmark", &dump, &want);
    }
    println!(
        "MEASURED compared={compared} (corpus={} multi-file={} focused={}); largest=selfhost/check.elya with parse and lex ({} bytes of root, {} dump bytes); native read+lex+parse+load+resolve+dump+process median={:?} (7 runs); Rust read+lex+parse+load+resolve median={:?}; Rust with the dump median={:?} (101 runs each; Cargo test profile, native O2)",
        corpus.len(),
        multi.len(),
        focused.len(),
        text.len(),
        want.len(),
        median(native),
        median(rust_resolve),
        median(rust_all)
    );
}
