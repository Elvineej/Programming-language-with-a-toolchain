// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! The affine pass in Elya (self-hosting part 4e, spec
//! docs/superpowers/specs/2026-11-15-elya-checker-in-elya-design.md D2):
//! `selfhost/affine.elya` prints exactly `elya affine --dump` on every corpus
//! program that resolves cleanly, and on the focused set in
//! tests/support/selfhost_affine.rs.
//!
//! Negative controls, each run against this test with the Elya program
//! changed and then restored byte for byte (named by the first program each
//! fails on):
//! A1, two uses in sequence naming the earlier: examples/02_arith.elya.
//! A2, the arms' smaller count kept: examples/03_cek.elya.
//! A3, counts added without the larger frame first (Rust's swap decides
//! which use a merged count names): selfhost/lex.elya.
//! A4, a lambda's uses not many: examples/04_async.elya.
//! A5, a `let`-bound lambda's call counting its body many: tests/adt.rs.
//! A6, a clause's uses counted once: examples/10_failure.elya.
//! A7, a finished binding's many dropped: examples/02_arith.elya.
//! A8, the arms of an `if` added, not the larger: examples/03_cek.elya.
//! A9, a borrowed argument counted as a use: selfhost/lex.elya.
//! B1, a binding given as an argument given away where it is written, not
//! when the call is made: tests/array.rs.
//! B2, the `then` branch's gone forgotten: focused #9.
//! B3, a lambda's captures not given away: focused #10.
//! B4, a use from outside the body taken as a borrow: tests/array.rs.
//! C1, a `borrow` parameter's first use, borrows included: selfhost/lex.elya.
//! C2, a call by name taken as a value use: selfhost/lex.elya.
//! D1, a point holding nothing waiting: examples/07_search.elya.
//! D2, a constructor call a point: examples/07_search.elya.
//! D3, the earlier first use losing a join: focused #12.
//! D4, a resume not recorded: examples/07_search.elya.
//! D5, an escaping handler's body not a body apart: survived; focused #20.
//! D6, a capture reaching every open lambda: focused #13.
//! D7, a waiting variable without what it captures: focused #13.
//! D8, a `handle`'s body not continuing into its return clause: survived;
//! focused #21.
//! One survives and cannot be caught: D9 (a `match`'s arms joined onto what
//! follows it rather than onto nothing), since every arm starts from what
//! follows, so the join holds it either way, and a pattern's variables are
//! never live after the `match`.

// Only the module reader is used here.
#[allow(dead_code)]
#[path = "support/selfhost_check.rs"]
mod check_support;
#[path = "support/selfhost_parse.rs"]
mod parse_support;
#[path = "support/selfhost_affine.rs"]
mod support;

use elya::eval::{Host, World};
use std::path::Path;

#[test]
fn the_affine_pass_in_elya_agrees() {
    let root = parse_support::root();
    let pass = std::fs::read_to_string(root.join("selfhost/affine.elya")).expect("the pass");
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/affine.elya", &pass, &mut check_support::reader);
    assert!(
        diagnostics.is_empty(),
        "affine pass front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let corpus = parse_support::corpus(Path::new(env!("CARGO_MANIFEST_DIR")));
    let scratch = parse_support::Scratch::new("affine");
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
        if !diags.is_empty() || imports {
            continue; // the resolver's errors are printed alone
        }
        let want = elya::check_dump::affine_facts(&elya::affine::analyze(&program.module));
        let path = scratch.put(&text);
        let host = Host {
            args: vec![path.into_os_string()],
            ..Host::default()
        };
        let (interpreter, result) = elya::eval::run_module_keeping_host(&module, World::Live, host);
        assert!(result.is_ok(), "{name}: {result:?}");
        parse_support::assert_dump(&name, interpreter.output(), &want);
        compared += 1;
        if want.contains("\nheld ") || want.starts_with("held ") {
            multi += 1;
        }
    }
    println!("compared {compared}, {multi} with points held");
    assert!(
        compared >= 500 + support::FOCUSED.len(),
        "at least 500 corpus programs, and the focused set ({compared})"
    );
}
