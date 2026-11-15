//! Differential proof for selfhost/parse.elya on programs parsed without errors.
//!
//! Negative controls, each run (exit 101), reverted byte for byte, and checked
//! with `git diff --quiet`:
//! K1: `+` binding power 50 -> 60: repository_and_embedded_programs_agree
//! fails on examples/06_structured.elya, line 244 (`Binary *` vs `Binary +`).
//! K2: call span ends at callee: the same test fails on examples/01_hello.elya,
//! line 9 (`Call 41 51` vs `Call 41 67`); the native largest-input test fails too.
//! K3: consume `borrow` but record false: the same test fails on
//! selfhost/lex.elya, line 1807 (`Param -` vs `Param borrow`).

#[path = "support/selfhost_parse.rs"]
mod support;

use elya::eval::{Host, World};
use std::path::Path;

#[test]
fn repository_and_embedded_programs_agree() {
    let parser = support::parser_source();
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/parse.elya", &parser, &mut support::reader);
    assert!(
        diagnostics.is_empty(),
        "parser front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let corpus = support::corpus(Path::new(env!("CARGO_MANIFEST_DIR")));
    assert!(corpus.embedded >= 60, "at least 60 clean embedded programs");
    let scratch = support::Scratch::new("evaluator");
    for case in &corpus.cases {
        let path = scratch.put(&case.text);
        let host = Host {
            args: vec![path.into_os_string()],
            ..Host::default()
        };
        let (interpreter, result) = elya::eval::run_module_keeping_host(&module, World::Live, host);
        assert!(result.is_ok(), "{}: {result:?}", case.name);
        support::assert_dump(&case.name, interpreter.output(), &case.dump);
    }
    println!(
        "compared {} clean repository files ({} skipped with parse errors); {} clean embedded programs; {} focused programs; {} total cases / {} total bytes; largest {} ({} bytes)",
        corpus.files,
        corpus.skipped,
        corpus.embedded,
        corpus.cases.len() - corpus.files - corpus.embedded,
        corpus.cases.len(),
        corpus.cases.iter().map(|c| c.text.len()).sum::<usize>(),
        corpus.largest().name,
        corpus.largest().text.len()
    );
}
