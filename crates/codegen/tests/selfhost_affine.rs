// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! The affine pass in Elya, natively (self-hosting part 4e): every program
//! of tests/selfhost_affine.rs agrees with `elya affine --dump`, and the
//! largest is timed against Rust. Native entry points append main's value
//! line (`0\n` for Unit), removed before comparing.

// Only the module reader is used here.
#[allow(dead_code)]
#[path = "../../../tests/support/selfhost_check.rs"]
mod check_support;
#[path = "../../../tests/support/selfhost_parse.rs"]
mod parse_support;
#[path = "../../../tests/support/selfhost_affine.rs"]
mod support;

use std::process::Command;
use std::time::{Duration, Instant};

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort_unstable();
    times[times.len() / 2]
}

#[test]
fn the_affine_pass_in_elya_agrees_natively_and_is_measured() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(native_proof)
        .expect("native affine test thread")
        .join()
        .unwrap();
}

fn native_proof() {
    let root = parse_support::root();
    let pass = std::fs::read_to_string(root.join("selfhost/affine.elya")).expect("the pass");
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/affine.elya", &pass, &mut check_support::reader);
    assert!(
        diagnostics.is_empty(),
        "affine pass front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let scratch = parse_support::Scratch::new("native-affine");
    let (diagnostics, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diagnostics.is_empty(), "typed affine pass: {diagnostics:?}");
    let core = elya::core::lower_module(&module, &table).expect("the pass lowers to Core");
    let object = scratch.path().join("affine.o");
    let executable = scratch
        .path()
        .join(format!("affine{}", std::env::consts::EXE_SUFFIX));
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
    let mut compared = 0;
    for (name, text) in texts {
        let (program, diags) =
            elya::resolved(&elya::Session::new(), "main.elya", &text, &mut |_| None);
        // A program importing modules is merged with them (std's too): what
        // reads the imports in Elya is part 4c's.
        let imports = !elya::parse::parse_module(&elya::Session::new(), &text)
            .0
            .imports
            .is_empty();
        if !diags.is_empty() || imports {
            continue;
        }
        let want = elya::check_dump::affine_facts(&elya::affine::analyze(&program.module));
        let path = scratch.put(&text);
        parse_support::assert_dump(&name, &run(&path), &want);
        compared += 1;
        if largest.as_ref().map_or(true, |(_, t)| t.len() < text.len()) {
            largest = Some((name, text));
        }
    }
    assert!(
        compared >= 500 + support::FOCUSED.len(),
        "natively: at least 500 corpus programs, and the focused set ({compared})"
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
        let (program, _) = elya::resolved(&elya::Session::new(), "main.elya", &text, &mut |_| None);
        let _ = std::hint::black_box(elya::check_dump::affine_facts(&elya::affine::analyze(
            &program.module,
        )));
        rust.push(start.elapsed());
    }
    println!(
        "compared {compared} natively; largest {name} ({} bytes): native {:?} (process), Rust {:?} (resolve and the pass)",
        text.len(),
        median(native),
        median(rust)
    );
}
