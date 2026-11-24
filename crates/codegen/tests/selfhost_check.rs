//! The checker in Elya, natively (plan
//! docs/superpowers/plans/2026-11-15-elya-checker-in-elya.md, step 4): every
//! program of the built stages agrees with `elya check --dump`, and the
//! largest is timed against Rust. Native entry points append main's value
//! line (`0\n` for Unit), removed before comparing.

#[path = "../../../tests/support/selfhost_parse.rs"]
mod parse_support;
#[path = "../../../tests/support/selfhost_check.rs"]
mod support;

use std::process::Command;
use std::time::{Duration, Instant};

const BUILT: u8 = 7;

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort_unstable();
    times[times.len() / 2]
}

#[test]
fn the_checker_in_elya_agrees_natively_and_is_measured() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(native_proof)
        .expect("native checker test thread")
        .join()
        .unwrap();
}

fn native_proof() {
    let root = parse_support::root();
    let checker = std::fs::read_to_string(root.join("selfhost/check.elya")).expect("the checker");
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/check.elya", &checker, &mut support::reader);
    assert!(
        diagnostics.is_empty(),
        "checker front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let scratch = parse_support::Scratch::new("native-check");
    let (diagnostics, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diagnostics.is_empty(), "typed checker: {diagnostics:?}");
    let core = elya::core::lower_module(&module, &table).expect("checker lowers to Core");
    let object = scratch.path().join("check.o");
    let executable = scratch
        .path()
        .join(format!("check{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &object).expect("native checker compiles");
    elya_codegen::link(&object, &executable).expect("native checker links");
    let run = |path: &std::path::Path| {
        let output = Command::new(&executable)
            .arg(path)
            .env_remove("ELYA_RECORD")
            .env_remove("ELYA_REPLAY")
            .output()
            .expect("run native checker");
        assert!(
            output.status.success(),
            "native checker: {}",
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
            support::FOCUSED_F1
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("focused F1 #{i}"), t.to_string())),
        )
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
        );
    let mut largest: Option<(String, String, String)> = None;
    let mut compared = 0;
    for (name, text) in texts {
        let (program, diags) =
            elya::resolved(&elya::Session::new(), "main.elya", &text, &mut |_| None);
        if !diags.is_empty() || support::stage(&program.module) > BUILT {
            continue;
        }
        let want = elya::check_dump::dump(&program.module);
        let path = scratch.put(&text);
        parse_support::assert_dump(&name, &run(&path), &want);
        compared += 1;
        if largest
            .as_ref()
            .map_or(true, |(_, t, _)| t.len() < text.len())
        {
            largest = Some((name, text, want));
        }
    }
    assert!(
        compared
            >= 420
                + support::FOCUSED_F1.len()
                + support::FOCUSED_F2A.len()
                + support::FOCUSED_F2B.len()
                + support::FOCUSED_F4A.len()
                + support::FOCUSED_F4B.len()
                + support::FOCUSED_F4C.len()
                + support::FOCUSED_F4D.len(),
        "F1 to F4d natively: at least 420 corpus programs, and the focused sets ({compared})"
    );
    let (name, text, _) = largest.expect("a program in the built stages");
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
        let _ = std::hint::black_box(elya::check_dump::dump(&program.module));
        rust.push(start.elapsed());
    }
    println!(
        "compared {compared} natively; largest {name} ({} bytes): native {:?} (process), Rust {:?} (resolve and check)",
        text.len(),
        median(native),
        median(rust)
    );
}
