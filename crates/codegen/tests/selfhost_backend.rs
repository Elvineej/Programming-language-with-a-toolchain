//! The back end in Elya (self-hosting part 5, spec
//! docs/superpowers/specs/2026-11-27-elya-backend-in-elya-design.md D2):
//! every corpus program in the stage's fragment is built twice, by the Rust
//! back end and from the LLVM IR text `selfhost/emit.elya` writes (linked
//! with the same runtime), and the two programs must print the same standard
//! output and standard error and exit the same way. A program outside the
//! fragment is refused by name (`unsupported ...`) and not compared.
//!
//! Negative controls, each run with `selfhost/emit.elya` changed and then
//! restored byte for byte (named by the first program each fails on):
//! D1, `/` without its zero check: tests/int_arith.rs.
//! D2, `%` by -1 unguarded: focused #12.
//! D3, a tail call not `musttail`: focused #13 (a hundred million deep).
//! D4, an `if`'s phi arms swapped: tests/annotations.rs.
//! D5, orderings unsigned: focused #14.
//! D6, a `Bool` main printed as a number: tests/eval_value.rs.
//! D7, `*` unchecked: focused #15.
//! D8, a quote in a string constant unescaped: focused #10 (the IR does
//! not build).
//! D9, `MIN / -1` unchecked: focused #6.

// Only the module reader is used here.
#[allow(dead_code)]
#[path = "../../../tests/support/selfhost_check.rs"]
mod check_support;
#[path = "../../../tests/support/selfhost_parse.rs"]
mod parse_support;
#[path = "../../../tests/support/selfhost_backend.rs"]
mod support;

use std::path::Path;
use std::process::{Command, Output};

#[test]
fn the_back_end_in_elya_agrees_with_rusts() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(proof)
        .expect("back end test thread")
        .join()
        .unwrap();
}

fn run(exe: &Path) -> Output {
    Command::new(exe)
        .env_remove("ELYA_RECORD")
        .env_remove("ELYA_REPLAY")
        .env_remove("ELYA_GC_STRESS")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run a built program")
}

/// The Rust back end's program for `text`, or `None` when it does not build
/// (a program the front end refuses, or one the back end refuses).
fn rust_build(text: &str, exe: &Path) -> Option<()> {
    let (module, diags, _) = elya::front_end_with("main.elya", text, &mut |_| None);
    if diags
        .iter()
        .any(|d| d.severity == elya::diag::Severity::Error)
    {
        return None;
    }
    let (_, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    let core = elya::core::lower_module(&module, &table).ok()?;
    let object = exe.with_extension("o");
    elya_codegen::compile_module(&core, &object).ok()?;
    elya_codegen::link(&object, exe).ok()
}

fn proof() {
    let root = parse_support::root();
    let pass = std::fs::read_to_string(root.join("selfhost/emit.elya")).expect("the back end");
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/emit.elya", &pass, &mut check_support::reader);
    assert!(
        diagnostics.is_empty(),
        "back end front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let scratch = parse_support::Scratch::new("native-backend");
    let (diagnostics, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diagnostics.is_empty(), "typed back end: {diagnostics:?}");
    let core = elya::core::lower_module(&module, &table).expect("the back end lowers to Core");
    let object = scratch.path().join("emit.o");
    let emitter = scratch
        .path()
        .join(format!("emit{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &object).expect("the back end compiles");
    elya_codegen::link(&object, &emitter).expect("the back end links");

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
    let (mut compared, mut refused) = (0, 0);
    let rust_exe = scratch
        .path()
        .join(format!("rust{}", std::env::consts::EXE_SUFFIX));
    let elya_exe = scratch
        .path()
        .join(format!("elya{}", std::env::consts::EXE_SUFFIX));
    for (name, text) in texts {
        // A program importing modules is merged with them: part 4c's.
        if !elya::parse::parse_module(&elya::Session::new(), &text)
            .0
            .imports
            .is_empty()
        {
            continue;
        }
        let path = scratch.put(&text);
        let out = Command::new(&emitter)
            .arg(&path)
            .output()
            .expect("run the back end");
        assert!(
            out.status.success(),
            "{name}: the back end failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let ir = String::from_utf8(out.stdout).expect("UTF-8");
        let ir = ir
            .strip_suffix("0\n")
            .expect("native runtime appends main's Unit value line");
        if ir.starts_with("unsupported ") {
            refused += 1;
            continue;
        }
        if rust_build(&text, &rust_exe).is_none() {
            continue;
        }
        let ll = scratch.path().join("program.ll");
        std::fs::write(&ll, ir).expect("write the IR");
        if let Err(e) = elya_codegen::link(&ll, &elya_exe) {
            panic!("{name}: the IR does not build: {e}\n{ir}");
        }
        let (want, got) = (run(&rust_exe), run(&elya_exe));
        assert_eq!(
            (
                String::from_utf8_lossy(&got.stdout),
                String::from_utf8_lossy(&got.stderr),
                got.status.code()
            ),
            (
                String::from_utf8_lossy(&want.stdout),
                String::from_utf8_lossy(&want.stderr),
                want.status.code()
            ),
            "{name}: the Elya-built program (left) and the Rust-built one (right)"
        );
        compared += 1;
    }
    println!("compared {compared}, {refused} outside the stage");
    assert!(
        compared >= support::FLOOR + support::FOCUSED.len(),
        "at least {} corpus programs, and the focused set ({compared})",
        support::FLOOR
    );
}
