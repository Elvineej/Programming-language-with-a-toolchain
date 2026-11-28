// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! Core in Elya, natively (self-hosting part 5's first piece): the native
//! twin of tests/selfhost_core.rs, which agrees with `elya core --dump` in the
//! evaluator, and the pass's speed against Rust's front end and lowering.

#[allow(dead_code)]
#[path = "../../../tests/support/selfhost_check.rs"]
mod check_support;
#[path = "../../../tests/support/selfhost_parse.rs"]
mod parse_support;
#[path = "../../../tests/support/selfhost_core.rs"]
mod support;

use std::process::Command;
use std::time::{Duration, Instant};

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort_unstable();
    times[times.len() / 2]
}

#[test]
fn core_in_elya_agrees_natively_and_is_measured() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(native_proof)
        .expect("native Core test thread")
        .join()
        .unwrap();
}

fn native_proof() {
    let root = parse_support::root();
    let pass = std::fs::read_to_string(root.join("selfhost/core.elya")).expect("the pass");
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/core.elya", &pass, &mut check_support::reader);
    assert!(
        diagnostics.is_empty(),
        "Core front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let scratch = parse_support::Scratch::new("native-core");
    let (diagnostics, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diagnostics.is_empty(), "typed Core pass: {diagnostics:?}");
    let core = elya::core::lower_module(&module, &table).expect("the pass lowers to Core");
    let object = scratch.path().join("core.o");
    let executable = scratch
        .path()
        .join(format!("core{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &object).expect("native pass compiles");
    elya_codegen::link(&object, &executable).expect("native pass links");
    let run = |path: &std::path::Path| {
        let output = Command::new(&executable)
            .arg(path)
            .env_remove("ELYA_RECORD")
            .env_remove("ELYA_REPLAY")
            .output()
            .expect("run native pass");
        assert!(
            output.status.success(),
            "native pass: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("UTF-8");
        stdout
            .strip_suffix("0\n")
            .expect("native runtime appends main's Unit value line")
            .to_owned()
    };
    let corpus = parse_support::corpus(&root);
    let texts = corpus
        .cases
        .iter()
        .map(|c| (c.name.clone(), c.text.clone()))
        .chain(
            support::FOCUSED
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("focused #{i}"), t.to_string())),
        );
    let mut largest: Option<(String, String)> = None;
    let (mut compared, mut escaped) = (0, 0);
    for (name, text) in texts {
        let Some(want) = support::want(&text) else {
            continue;
        };
        let path = scratch.put(&text);
        let got = run(&path);
        if got == support::ESCAPED {
            escaped += 1;
            continue;
        }
        parse_support::assert_dump(&name, &got, &want);
        compared += 1;
        if largest.as_ref().map_or(true, |(_, t)| t.len() < text.len()) {
            largest = Some((name, text));
        }
    }
    assert!(
        compared >= 350 + support::FOCUSED.len(),
        "natively: at least 350 corpus programs, and the focused set ({compared}, {escaped} with a character Debug escapes)"
    );
    let (name, text) = largest.expect("a program");
    let path = scratch.put(&text);
    let mut native = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        let _ = run(&path);
        native.push(start.elapsed());
    }
    let mut rust = Vec::new();
    for _ in 0..101 {
        let start = Instant::now();
        let _ = std::hint::black_box(support::want(&text));
        rust.push(start.elapsed());
    }
    println!(
        "compared {compared} natively; largest {name} ({} bytes): native {:?} (process), Rust {:?} (the front end and lowering)",
        text.len(),
        median(native),
        median(rust)
    );
}
