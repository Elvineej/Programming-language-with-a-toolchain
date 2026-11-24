//! The checker in Elya (plan docs/superpowers/plans/2026-11-15-elya-checker-in-elya.md,
//! step 4): `selfhost/check.elya` prints exactly `elya check --dump` on every
//! corpus program inside the stages built so far: F1 (first-order programs)
//! F2a (declared types, constructors, `match`, the pure builtins,
//! annotations naming types and type variables), F2b (higher order:
//! lambdas, functions as values, function types, with effect rows' tails)
//! F4a (effects: rows with labels, performs, handlers, `resume`, the
//! host's builtins, declared rows, `main`'s discharge) and F4b (`with
//! multi`, named row variables, row parameters of declared types).
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
//! K14, the last label along a row's chain kept, not the first: fails on
//! tests/handle_recursion.rs.
//! K15, a conflict into an ambient row not reported: fails on
//! tests/handle_recursion.rs.
//! K16, a declared label never performed not reported: tests/modules.rs.
//! K17, `main` discharging every effect: tests/ui/unhandled_effect.elya.
//! K18, `resume` not rerunning the handled body's labels: src/std/clock.elya.
//! K19, an escaped `resume` not taking the clauses row:
//! tests/handle_recursion.rs.
//! K20, a handle passing its handled effect on: examples/04_async.elya.
//! K21, an inclusion adding no labels: survived the corpus; fails on the
//! focused F4a program written for it (`fn pick(x) { x }`).
//! K22, a call dropping its callee row's labels: examples/04_async.elya.
//! K23, an absorbed phantom tail's rest made meaningful: tests/modules.rs.
//! K25, the clauses row taking nothing: tests/resume_row.rs.
//! K28, an unknown effect in a row accepted: tests/fn_fields.rs.
//! Three survive and cannot be caught inside F4a: K24 (a `let` free to
//! quantify the resume rows) and K27 (the resume's types), since without
//! parametric effects those rows and types are made at the handle's level
//! and binding only lowers them, so no `let` inside a clause sees them
//! above its level; K26 (the first operation of a name wins), since two
//! operations of one name are a resolution error (E0202).
//! The review's mutation testing (39 mutants) left six more surviving the
//! corpus, each now failing on a focused F4a program after K21's: M4 (a
//! written row keeping its first label of a name, not its last), M11
//! (`resume` not rerunning the return clause), M17 (the clauses row closed
//! though it became the ambient's tail), M19 (a failing inclusion not
//! killed: `flush` never ends), M30 (no flush before closing the return
//! clause's row), M37 (an inclusion's label keeping its own span, not the
//! call's).
//! F4b's, each failing on the first program named:
//! K29, `with multi` over a one-shot effect accepted:
//! tests/ui/multi_on_oneshot.elya.
//! K30, no E0426 warning: tests/effect_types.rs.
//! K31, E0426 whatever the body performs: examples/07_search.elya.
//! K32, a named row fresh at each mention: tests/row_variables.rs.
//! K33, a row variable spelled like an effect accepted: focused F4b #9.
//! K34, the kinds in one pass (no fixpoint): focused F4b #19.
//! K35, a declared row's unused label not reported: focused F4b #7.
//! K36, a `let` quantifying the named rows: survived; focused F4b #27.
//! K37, the named rows checked after the residuals close:
//! tests/row_variables.rs.
//! K38, a row argument printed as its phantom function type:
//! tests/row_parameters.rs.
//! K39, an undeclared row variable in a declaration's argument reported as
//! E0400: survived; focused F4b #29.
//! K40, the body's named rows not kept for the declared row: survived;
//! focused F4b #30.
//! K42, a row parameter's argument a type variable: focused F4b #15.
//! K43, a type and a row of one name accepted where a type goes: focused
//! F4b #11.
//! One survives and cannot be caught: K41 (the declared row's labels at
//! their written spans, not the performed ones'), since every label of that
//! row is already in the performed row, so unifying never moves one into a
//! tail, and a label's span is read only from a row it was moved into.

#[path = "support/selfhost_parse.rs"]
mod parse_support;
#[path = "support/selfhost_check.rs"]
mod support;

use elya::eval::{Host, World};
use std::path::Path;

/// The stages `selfhost/check.elya` covers.
const BUILT: u8 = 5;

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
    let mut counts = [0usize; 9];
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
        )
        .chain(
            support::FOCUSED_F4A
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("focused F4a #{i}"), t.to_string())),
        )
        .chain(
            support::FOCUSED_F4B
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("focused F4b #{i}"), t.to_string())),
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
        "compared {compared}; resolved cleanly by stage 1..8: {:?}",
        &counts[1..]
    );
    assert!(
        compared
            >= 365
                + support::FOCUSED_F1.len()
                + support::FOCUSED_F2A.len()
                + support::FOCUSED_F2B.len()
                + support::FOCUSED_F4A.len()
                + support::FOCUSED_F4B.len(),
        "F1 to F4b: at least 365 corpus programs, and the focused sets"
    );
}
