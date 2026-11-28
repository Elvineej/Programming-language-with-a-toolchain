// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! Core in Elya (self-hosting part 5's first piece, spec
//! docs/superpowers/specs/2026-11-27-elya-backend-in-elya-design.md D3):
//! `selfhost/core.elya` prints exactly `elya core --dump` (`core::lower_module`
//! printed by `core::pretty_typed`) on every corpus program without imports
//! that checks, but those with a `Float` literal, and on the focused set in
//! tests/support/selfhost_core.rs.
//!
//! Negative controls, each run against the native twin with the Elya
//! program changed and then restored byte for byte (named by the first
//! program each fails on):
//! C1, `&&` lowered as `||`: selfhost/lex.elya.
//! C2, `!e` as `False == e`: selfhost/lex.elya.
//! C3, `-e` as `e - 0`: tests/int_wrapping.rs.
//! C4, a `let` typed by its value: examples/02_arith.elya.
//! C5, a statement's binder other than `_`: examples/03_cek.elya.
//! C6, a local shadowing an operation ignored: tests/core_lowering.rs.
//! C7, a match binder out of scope in its arm: focused #13.
//! C8, a tab not escaped: tests/programs/files_and_arguments.rs.
//! C9, `\r` decoded as a newline: focused #0.
//! C10, variables named afresh per function: examples/07_search.elya.
//! C11, the multi stamp dropped: tests/effect_syntax.rs.
//! C12, a `with any` clause keeping an operation name:
//! examples/09_record_any.elya.
//! C13, a bare constructor lowered as a variable: examples/03_cek.elya.
//! C15, a lambda's parameters out of scope: focused #5.
//! C16, `<=` printed `Lt`: selfhost/lex.elya.
//! C17, parametric types printed: examples/07_search.elya.
//! C18, a return clause's binder out of scope: tests/name_rules.rs.
//! C19, the types printed after the functions: examples/03_cek.elya.
//! C20, a clause's parameters out of scope: focused #14.
//! C21, a perform typed by the call, not the callee:
//! examples/04_async.elya.
//! C22, builtins lowered as calls: examples/02_arith.elya.
//! C14 (the applied resume's `let` at the call's span, not the callee's)
//! cannot be seen: the dump prints no spans.

// Only the module reader is used here.
#[allow(dead_code)]
#[path = "support/selfhost_check.rs"]
mod check_support;
#[path = "support/selfhost_parse.rs"]
mod parse_support;
#[path = "support/selfhost_core.rs"]
mod support;

use elya::eval::{Host, World};
use std::path::Path;

#[test]
fn core_in_elya_agrees() {
    let root = parse_support::root();
    let pass = std::fs::read_to_string(root.join("selfhost/core.elya")).expect("the pass");
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/core.elya", &pass, &mut check_support::reader);
    assert!(
        diagnostics.is_empty(),
        "Core front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let corpus = parse_support::corpus(Path::new(env!("CARGO_MANIFEST_DIR")));
    let scratch = parse_support::Scratch::new("core");
    let (mut compared, mut floats) = (0, 0);
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
        let Some(want) = support::want(&text) else {
            continue;
        };
        let path = scratch.put(&text);
        let host = Host {
            args: vec![path.into_os_string()],
            ..Host::default()
        };
        let (interpreter, result) = elya::eval::run_module_keeping_host(&module, World::Live, host);
        assert!(result.is_ok(), "{name}: {result:?}");
        if interpreter.output() == support::FLOAT || interpreter.output() == support::ESCAPED {
            floats += 1;
            continue;
        }
        parse_support::assert_dump(&name, interpreter.output(), &want);
        compared += 1;
    }
    println!("compared {compared}, {floats} with a Float literal skipped");
    assert!(
        compared >= 350 + support::FOCUSED.len(),
        "at least 350 corpus programs, and the focused set ({compared})"
    );
}
