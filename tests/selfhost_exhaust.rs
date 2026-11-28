// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! Exhaustiveness in Elya (self-hosting part 4d, spec
//! docs/superpowers/specs/2026-11-15-elya-checker-in-elya-design.md D2):
//! `selfhost/exhaust.elya` prints exactly `elya exhaust --dump` on every
//! corpus program that resolves without an error, ill-typed ones included,
//! and on the focused set in tests/support/selfhost_exhaust.rs.
//!
//! Negative controls, each run against this test with the Elya program
//! changed and then restored byte for byte (named by the first program each
//! fails on):
//! E1, the last missing constructor named, not the first: focused #17.
//! E2, `False` tried before `True`: focused #18.
//! E3, a wildcard row dropped when specializing: focused #4.
//! E4, the default matrix keeping constructor rows: selfhost/lex.elya.
//! E5, an arm checked against itself too: examples/03_cek.elya.
//! E6, E0430 printed before the match's E0431s: focused #3.
//! E7, a lambda's body not walked: focused #10.
//! E8, a return clause not walked: focused #10.
//! E9, a witness's arguments joined without a space:
//! tests/ui/nonexhaustive_list.elya.
//! E10, a rebuilt witness taking its arguments from the tail: focused #14.
//! E11, a column's signature from its last head: focused #7.
//! E12, a call's arguments walked right to left: focused #11.
//! E13, a ragged row out of columns taken as a wildcard: focused #19.
//! E14, an empty `match` checked: examples/10_failure.elya.
//! E15, a scrutinee walked after the arms: focused #12.
//! E16, `Unit` taken as infinite: focused #4.
//! E17, an `if` condition not walked: focused #11.
//! E18, a resume's argument not walked: focused #10.

// Only the module reader is used here.
#[allow(dead_code)]
#[path = "support/selfhost_check.rs"]
mod check_support;
#[path = "support/selfhost_parse.rs"]
mod parse_support;
#[path = "support/selfhost_exhaust.rs"]
mod support;

use elya::eval::{Host, World};
use std::path::Path;

#[test]
fn exhaustiveness_in_elya_agrees() {
    let root = parse_support::root();
    let pass = std::fs::read_to_string(root.join("selfhost/exhaust.elya")).expect("the pass");
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/exhaust.elya", &pass, &mut check_support::reader);
    assert!(
        diagnostics.is_empty(),
        "exhaustiveness pass front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let corpus = parse_support::corpus(Path::new(env!("CARGO_MANIFEST_DIR")));
    let scratch = parse_support::Scratch::new("exhaust");
    let mut compared = 0;
    let mut multi = 0;
    let cases = corpus
        .cases
        .iter()
        .map(|c| (c.name.clone(), c.text.clone()))
        .chain(
            support::FOCUSED
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("focused #{i}"), t.to_string())),
        );
    for (name, text) in cases {
        let (program, diags) =
            elya::resolved(&elya::Session::new(), "main.elya", &text, &mut |_| None);
        // A program importing modules is merged with them (std's too): what
        // reads the imports in Elya is part 4c's.
        let imports = !elya::parse::parse_module(&elya::Session::new(), &text)
            .0
            .imports
            .is_empty();
        if diags
            .iter()
            .any(|d| d.severity == elya::diag::Severity::Error)
            || imports
        {
            continue; // the resolver's errors are printed alone
        }
        let want = elya::ast_dump::dump_diagnostics(&elya::exhaust::check(&program.module));
        let path = scratch.put(&text);
        let host = Host {
            args: vec![path.into_os_string()],
            ..Host::default()
        };
        let (interpreter, result) = elya::eval::run_module_keeping_host(&module, World::Live, host);
        assert!(result.is_ok(), "{name}: {result:?}");
        parse_support::assert_dump(&name, interpreter.output(), &want);
        compared += 1;
        if !want.is_empty() {
            multi += 1;
        }
    }
    println!("compared {compared}, {multi} with diagnostics");
    assert!(
        compared >= 500 + support::FOCUSED.len(),
        "at least 500 corpus programs, and the focused set ({compared})"
    );
}
