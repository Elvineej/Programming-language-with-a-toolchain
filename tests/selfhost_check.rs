//! The checker in Elya (plan docs/superpowers/plans/2026-11-15-elya-checker-in-elya.md,
//! step 4): `selfhost/check.elya` prints exactly `elya check --dump` on every
//! corpus program inside the stages built so far: F1 (first-order programs)
//! and F2a (declared types, constructors, `match`, the pure builtins,
//! annotations naming types and type variables).
//!
//! Negative controls, each run, reverted and compared byte for byte:
//! K1, every scheme left unquantified: fails on `scheme id fn(a) -> a`
//! (tests/core_lowering.rs) against `forall a. fn(a) -> a`.
//! K2, `<` typed `Int`: fails on a `type` line, `Int` against `Bool`.
//! K3, the diagnostics printed newest first: fails on a focused program
//! with two errors (no F1 corpus program has two, hence the focused set).
//! K4, a pattern's field-count message with its counts swapped: fails on
//! focused F2a #3.
//! K5, a field type with the wrong number of arguments made an error type:
//! fails on focused F2a #16 (written for it; it survived before).
//! K6, a builtin's callee type not recorded: fails on tests/float_text.rs.
//! K7, a `let` free to quantify the function's annotation variables (the
//! second review's D1, a defect it found): fails on focused F2a #17.

#[path = "support/selfhost_parse.rs"]
mod parse_support;
#[path = "support/selfhost_check.rs"]
mod support;

use elya::eval::{Host, World};
use std::path::Path;

/// The stages `selfhost/check.elya` covers.
const BUILT: u8 = 2;

#[test]
fn the_checker_in_elya_agrees_on_its_stages() {
    let root = parse_support::root();
    let checker = std::fs::read_to_string(root.join("selfhost/check.elya")).expect("the checker");
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/check.elya", &checker, &mut support::reader);
    assert!(
        diagnostics.is_empty(),
        "checker front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let corpus = parse_support::corpus(Path::new(env!("CARGO_MANIFEST_DIR")));
    let scratch = parse_support::Scratch::new("checker");
    let mut counts = [0usize; 7];
    let mut compared = 0;
    let focused = support::FOCUSED_F1
        .iter()
        .enumerate()
        .map(|(i, t)| (format!("focused F1 #{i}"), t.to_string()))
        .chain(
            support::FOCUSED_F2A
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("focused F2a #{i}"), t.to_string())),
        );
    let cases = corpus
        .cases
        .iter()
        .map(|c| (c.name.clone(), c.text.clone()))
        .chain(focused);
    for (name, text) in cases {
        let session = elya::Session::new();
        let (program, diags) = elya::resolved(&session, "main.elya", &text, &mut |_| None);
        if !diags.is_empty() {
            continue; // the resolver's errors are printed alone: not the checker's
        }
        let stage = support::stage(&program.module);
        counts[stage as usize] += 1;
        if stage > BUILT {
            continue;
        }
        let want = elya::check_dump::dump(&program.module);
        let path = scratch.put(&text);
        let host = Host {
            args: vec![path.into_os_string()],
            ..Host::default()
        };
        let (interpreter, result) = elya::eval::run_module_keeping_host(&module, World::Live, host);
        assert!(result.is_ok(), "{name}: {result:?}");
        parse_support::assert_dump(&name, interpreter.output(), &want);
        compared += 1;
    }
    println!(
        "compared {compared}; resolved cleanly by stage F1..F6: {:?}",
        &counts[1..]
    );
    assert!(
        compared >= 80 + support::FOCUSED_F1.len() + support::FOCUSED_F2A.len(),
        "F1 and F2a: at least 80 corpus programs, and the focused sets"
    );
}
