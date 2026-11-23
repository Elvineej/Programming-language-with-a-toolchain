//! The checker in Elya (plan docs/superpowers/plans/2026-11-15-elya-checker-in-elya.md,
//! step 4): `selfhost/check.elya` prints exactly `elya check --dump` on every
//! corpus program inside the stages built so far: F1 (first-order programs)
//! F2a (declared types, constructors, `match`, the pure builtins,
//! annotations naming types and type variables), and F2b (higher order:
//! lambdas, functions as values, function types, with effect rows' tails).
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
//! K8, a residual ambient row never closed: fails on
//! tests/ui/nonexhaustive_int.elya (`/ {a}` on a pure function).
//! K9, a value's closed arrows not opened (`open_covariant`): fails on
//! focused F2b #20, written for it (it survived the corpus).
//! K10, equality marks not carried by instantiation: fails on focused F2b #8.
//! K11, no group member's row deferred inside a lambda: fails on focused
//! F2b #18 (`fix`).
//! K12, a `let` quantifying no rows: fails on focused F2b #21, written for
//! it (it survived the corpus).
//! K13, a `let` free to quantify a deferred lambda's row (the third
//! review's defect): fails on focused F2b #22.

#[path = "support/selfhost_parse.rs"]
mod parse_support;
#[path = "support/selfhost_check.rs"]
mod support;

use elya::eval::{Host, World};
use std::path::Path;

/// The stages `selfhost/check.elya` covers.
const BUILT: u8 = 3;

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
        )
        .chain(
            support::FOCUSED_F2B
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("focused F2b #{i}"), t.to_string())),
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
        compared
            >= 110
                + support::FOCUSED_F1.len()
                + support::FOCUSED_F2A.len()
                + support::FOCUSED_F2B.len(),
        "F1, F2a and F2b: at least 110 corpus programs, and the focused sets"
    );
}
