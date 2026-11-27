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
//! E1, the arguments computed before a call's next one not rooted:
//! focused #16 (collecting at every allocation).
//! E2, the scope's heap values not rooted: tests/string_bytes.rs.
//! E4, `==` on strings inverted: focused #17.
//! E5, a shift's range unchecked: focused #21.
//! E6, `int.shr` logical: focused #20.
//! E7, a binary operator's left operand not rooted: tests/crosscheck.rs.
//! F1, a constructor's tag compared unequal: examples/03_cek.elya.
//! F2, fields stored from word 0, over the tag: examples/03_cek.elya.
//! F3, a heap field left out of its descriptor's mask: examples/03_cek.elya
//! (collecting at every allocation).
//! F5, a pattern's fields read one word late: examples/03_cek.elya.
//! F7, data compared as strings: focused #28.
//! F8, a string pattern matching anything: focused #26.
//! F9, a constructor's fields unrooted at its allocation:
//! examples/03_cek.elya (collecting at every allocation).
//! F10, a pattern's binder left unbound: examples/03_cek.elya (the IR does
//! not build).
//! E3 (a call's own arguments not rooted) cannot be seen: the runtime roots
//! its arguments before it allocates (`elya_str_concat`, `elya_str_slice`)
//! and a function roots its parameters as its scope; they stay rooted, as
//! the Rust back end roots them.

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

fn run(exe: &Path, stress: bool) -> Output {
    let mut cmd = Command::new(exe);
    cmd.env_remove("ELYA_RECORD")
        .env_remove("ELYA_REPLAY")
        .env_remove("ELYA_GC_STRESS")
        .stdin(std::process::Stdio::null());
    // A collection at every allocation: a heap value the program did not
    // root is freed and poisoned, and reaching it stops the program.
    if stress {
        cmd.env("ELYA_GC_STRESS", "1");
    }
    cmd.output().expect("run a built program")
}

fn shown(o: &Output) -> (String, String, Option<i32>) {
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
        o.status.code(),
    )
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
        let (want, got) = (run(&rust_exe, false), run(&elya_exe, false));
        assert_eq!(
            shown(&got),
            shown(&want),
            "{name}: the Elya-built program (left) and the Rust-built one (right)"
        );
        assert_eq!(
            shown(&run(&elya_exe, true)),
            shown(&want),
            "{name}: the Elya-built program collecting at every allocation (left) and the Rust-built one (right)"
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
