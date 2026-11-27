//! The checker in Elya (plan docs/superpowers/plans/2026-11-15-elya-checker-in-elya.md,
//! step 4): `selfhost/check.elya` prints exactly `elya check --dump` on every
//! corpus program inside the stages built so far: F1 (first-order programs)
//! F2a (declared types, constructors, `match`, the pure builtins,
//! annotations naming types and type variables), F2b (higher order:
//! lambdas, functions as values, function types, with effect rows' tails)
//! F4a (effects: rows with labels, performs, handlers, `resume`, the
//! host's builtins, declared rows, `main`'s discharge), F4b (`with
//! multi`, named row variables, row parameters of declared types), F4c
//! (parametric effects: labels with arguments), F4d (`with any`, `op.*`)
//! and F5 (duplicability: `linear` types, `Buffer`, `Array`, borrows).
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
//! F4c's:
//! K44, conflicting label arguments unified, not reported:
//! tests/generic_effects.rs.
//! K45, shared labels read once, not until none is new: survived; focused
//! F4c #41.
//! K46, duplicates along a chain not settled: survived; focused F4c #35.
//! K47, a row whose labels mention its tail added label by label:
//! src/std/async.elya.
//! K48, a row's occurs check blind to its labels' arguments: the checker
//! recurses without end (Rust's E0424), on tests/occurs_in_rows.rs.
//! K50, labels' arguments not resolved: src/std/async.elya.
//! K51, an unwritten label's arguments none, not fresh: survived; focused
//! F4c #37.
//! K52, the clauses not at the handler's instantiation:
//! tests/effect_syntax.rs.
//! K53, a perform's parameters not instantiated:
//! tests/row_parameters/round_robin.elya.
//! K54, a member inclusion's shared labels' arguments not unified:
//! tests/handle_recursion.rs.
//! K55, a declared row's written arguments not checked (Rust's 2.47):
//! focused F4c #3.
//! K56, a label mentioning the named row allowed in it: survived; focused
//! F4c #38.
//! K59, an effect's own label unwritten in its declaration an error:
//! src/std/async.elya.
//! K60, a label printed without its arguments: src/std/async.elya.
//! K61, a label's arguments' variables not free: tests/core_lowering.rs.
//! K62, an inclusion's labels relayed without their arguments: survived;
//! focused F4c #40.
//! K63, two ground function arguments never a conflict: survived; focused
//! F4c #39.
//! K64, the free rows skipping a label's arguments:
//! tests/row_parameters/round_robin.elya.
//! K49 (a bound row's labels' arguments keeping their levels) and K57 (the
//! members' duplicates not settled at 2.6) survived the corpus and were
//! argued uncatchable; the review caught both: focused F4c #42 (a lambda's
//! `let` quantifying a variable inside the function's row) and #51.
//! One survives, argued: K58 (a `let` quantifying a resume row's label
//! variables), as K24 and K27: every binding lowers what it binds to the
//! bound variable's level (K49's lowering among them), so what a resume row
//! reaches sits at the handle's level, below any `let` in a clause.
//! The F4c review's two defects, each fixed with a focused program: a
//! settled chain not compressed, so its duplicate was reported again where
//! the next row met it (focused F4c #52; K83 in F4d's list), and a clause
//! for another effect's operation instantiated at the handled effect's
//! parameters, not its own (#53).
//! F4d's:
//! K65, `IO` kept in the handled row: focused F4d #4.
//! K66, the body's `IO` not passed on: survived; focused F4d #27.
//! K67, forwarding never seen: tests/any_handler.rs.
//! K68, the handled rows never checked for data: tests/any_handler.rs.
//! K69, a generically handled tail not checked where instantiated:
//! tests/any_handler.rs.
//! K70, a forwarding handler's own row checked where instantiated:
//! tests/any_handler.rs.
//! K71, no E0435: tests/any_handler.rs.
//! K72, E0435 blind to the bindings outside: tests/any_handler.rs.
//! K73, E0435 blind to the ambient's labels: survived; focused F4d #28 (a
//! handle inside another's body, whose ambient no binding holds).
//! K74, every clause's answer type the same: examples/09_record_any.elya.
//! K75, `op.perform` performing nothing: examples/09_record_any.elya.
//! K76, an operation not data for `==`: tests/any_handler.rs.
//! K77, a `with any` resume rerunning every label of the body:
//! tests/literals_layout.rs.
//! K78, a `let` quantifying a handled row not yet checked:
//! tests/any_handler.rs.
//! K80, an operation printed as its constructor:
//! examples/09_record_any.elya.
//! K81, a clause's answer type taken as data: focused F4d #12.
//! One survives and cannot be caught: K79 (an effect's operations checked
//! unsorted), since each operation not data gives the same message at the
//! same span, so their order is not seen.
//! The F4d review's two defects, each fixed with a control: K82, every
//! binding walked for E0435, those a later `let` of the same name hides in
//! its block too (Rust walks each scope's open entries): focused F4d #40;
//! K83, a settled chain not compressed, so its duplicate was reported again
//! where the next row met it: focused F4d #41.
//! F5's:
//! K84, binding a duplicable variable not checked:
//! tests/ui/affine_through_generic.elya.
//! K85, a binding used many times not checked: tests/ui/affine_double_use.elya.
//! K86, a read after the binding was given away not checked: tests/array.rs.
//! K87, a `borrow` parameter given away not checked: tests/array.rs.
//! K88, a borrowing function as a value not checked: focused F5 #10.
//! K89, a `multi` point not requiring what it holds:
//! tests/ui/affine_multi_capture.elya.
//! K90, a point's open tail not holding: focused F5 #13.
//! K91, a row's holds dropped when it is bound: focused F5 #13.
//! K92, an instantiation not copying duplicable marks:
//! tests/ui/affine_through_generic.elya.
//! K93, an instantiation not copying holds: focused F5 #13.
//! K94, a `let` quantifying what a row holds (the review's U-B): survived;
//! focused F5 #31.
//! K95, an array's element not required duplicable: tests/array.rs.
//! K96, a declared type holding a linear one duplicable: focused F5 #17.
//! K97, a function type not known duplicable: survived; focused F5 #35.
//! K98, the affine diagnostics shown beside other errors: focused F5 #15.
//! K99, a use blamed more than once: survived; focused F5 #32.
//! K100, a generalized `let`'s quantified variables marked: survived;
//! focused F5 #33.
//! K101, a handle's resumes not points: survived; focused F5 #34.
//! K102, `Buffer` not a type name: selfhost/lex.elya.

#[path = "support/selfhost_parse.rs"]
mod parse_support;
#[path = "support/selfhost_check.rs"]
mod support;

use elya::eval::{Host, World};
use std::path::Path;

/// The stages `selfhost/check.elya` covers.
const BUILT: u8 = 8;

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
    let mut counts = [0usize; 10];
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
        )
        .chain(
            support::FOCUSED_F4C
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("focused F4c #{i}"), t.to_string())),
        )
        .chain(
            support::FOCUSED_F4D
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("focused F4d #{i}"), t.to_string())),
        )
        .chain(
            support::FOCUSED_F5
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("focused F5 #{i}"), t.to_string())),
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
        "compared {compared}; resolved cleanly by stage 1..9: {:?}",
        &counts[1..]
    );
    assert!(
        compared
            >= 450
                + support::FOCUSED_F1.len()
                + support::FOCUSED_F2A.len()
                + support::FOCUSED_F2B.len()
                + support::FOCUSED_F4A.len()
                + support::FOCUSED_F4B.len()
                + support::FOCUSED_F4C.len()
                + support::FOCUSED_F4D.len()
                + support::FOCUSED_F5.len(),
        "F1 to F5: at least 450 corpus programs, and the focused sets"
    );
}
