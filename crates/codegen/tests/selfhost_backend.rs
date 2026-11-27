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
//! G2, a parametric type's instances keyed by the constructor alone:
//! focused #32.
//! G3, an instance's fields left at the declaration's variables:
//! tests/adt.rs (the IR does not build).
//! G5, a `let` of a generic value not refused: focused #35.
//! G6, one clone of a generic function for every instance: tests/types.rs
//! (the IR does not build).
//! H2, a closure's heap captures left out of its row's mask: focused #36
//! (collecting at every allocation).
//! H3, a closure called without itself as the first argument: tests/adt.rs.
//! H4, a call through a closure in tail position not `musttail`: focused
//! #38 (a hundred million deep).
//! H6, an instance keyed with its effect rows: focused #37 (a generic
//! function's row variable left in a key).
//! I1, every array untraced: focused #40 (collecting at every allocation).
//! I3, a buffer's pieces swapped: selfhost/lex.elya.
//! I4, `io.file_exists` inverted: tests/files_and_arguments.rs.
//! I5, an array's element stored unwidened: selfhost/lex.elya (the IR does
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

use std::path::{Path, PathBuf};
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

fn run(exe: &Path, stress: Option<&str>) -> Output {
    let mut cmd = Command::new(exe);
    // In the scratch directory: a program that writes a file writes it there.
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.env_remove("ELYA_RECORD")
        .env_remove("ELYA_REPLAY")
        .env_remove("ELYA_GC_STRESS")
        .stdin(std::process::Stdio::null());
    // A collection at every Nth allocation: a heap value the program did not
    // root is freed and poisoned, and reaching it stops the program.
    if let Some(n) = stress {
        cmd.env("ELYA_GC_STRESS", n);
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
        let path = scratch.put(&text);
        let out = Command::new(&emitter)
            .arg(&path)
            .arg(root.join("src/std"))
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
        let (want, got) = (run(&rust_exe, None), run(&elya_exe, None));
        assert_eq!(
            shown(&got),
            shown(&want),
            "{name}: the Elya-built program (left) and the Rust-built one (right)"
        );
        assert_eq!(
            // Every allocation for the focused programs, written for it; every
            // 9973rd for the corpus's, some of which build lists of a million.
            shown(&run(
                &elya_exe,
                Some(if name.starts_with("focused") {
                    "1"
                } else {
                    "9973"
                })
            )),
            shown(&want),
            "{name}: the Elya-built program collecting often (left) and the Rust-built one (right)"
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

/// Stage B8 (spec D6): the self-hosted sources themselves, each built twice,
/// by the Rust back end and by the back end in Elya, run on the repository's
/// Elya files: the two builds print the same and exit the same. The back end
/// in Elya is among them, so its Elya-built twin must write the same IR as
/// its Rust-built self, its own source included (the back end's fixed point).
#[test]
fn the_self_hosted_sources_built_by_the_back_end_in_elya_agree() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(self_hosted)
        .expect("back end test thread")
        .join()
        .unwrap();
}

fn build_both(root: &Path, emitter: &Path, scratch: &Path, tool: &str) -> (PathBuf, PathBuf) {
    let source = root.join("selfhost").join(format!("{tool}.elya"));
    let text = std::fs::read_to_string(&source).expect("a self-hosted source");
    let (module, diagnostics, sources) = elya::front_end_with(
        &format!("selfhost/{tool}.elya"),
        &text,
        &mut check_support::reader,
    );
    assert!(
        diagnostics.is_empty(),
        "{tool}: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let (_, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let rust = scratch.join(format!("{tool}-rust{}", std::env::consts::EXE_SUFFIX));
    let object = scratch.join(format!("{tool}-rust.o"));
    elya_codegen::compile_module(&core, &object).expect("compiles");
    elya_codegen::link(&object, &rust).expect("links");
    let out = Command::new(emitter)
        .arg(&source)
        .arg(root.join("src/std"))
        .output()
        .expect("run the back end");
    let ir = String::from_utf8(out.stdout).expect("UTF-8");
    let ir = ir.strip_suffix("0\n").expect("main's Unit line");
    assert!(
        !ir.starts_with("unsupported "),
        "{tool}: the back end in Elya refuses it: {ir}"
    );
    let ll = scratch.join(format!("{tool}.ll"));
    std::fs::write(&ll, ir).expect("write the IR");
    let elya = scratch.join(format!("{tool}-elya{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::link(&ll, &elya).unwrap_or_else(|e| panic!("{tool}: the IR does not build: {e}"));
    (rust, elya)
}

fn self_hosted() {
    let root = parse_support::root();
    let scratch = parse_support::Scratch::new("native-self-hosted");
    let pass = std::fs::read_to_string(root.join("selfhost/emit.elya")).expect("the back end");
    let (module, _, _) =
        elya::front_end_with("selfhost/emit.elya", &pass, &mut check_support::reader);
    let (_, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let object = scratch.path().join("emitter.o");
    let emitter = scratch
        .path()
        .join(format!("emitter{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &object).expect("the back end compiles");
    elya_codegen::link(&object, &emitter).expect("the back end links");
    let mut inputs = Vec::new();
    for dir in ["examples", "selfhost", "src/std", "tests/ui"] {
        let mut names: Vec<_> = std::fs::read_dir(root.join(dir))
            .expect("a source directory")
            .map(|e| e.expect("an entry").path())
            .filter(|p| p.extension().is_some_and(|x| x == "elya"))
            .collect();
        names.sort();
        inputs.extend(names);
    }
    let tools = [
        "lex", "parse", "exhaust", "affine", "check", "core", "resolve", "emit",
    ];
    let mut compared = 0;
    for tool in tools {
        let (rust, elya) = build_both(&root, &emitter, scratch.path(), tool);
        // The back end itself on the programs it compiles in a few seconds:
        // the examples and the self-hosted sources it compiles, its own last.
        let these: Vec<&PathBuf> = if tool == "emit" {
            inputs
                .iter()
                .filter(|p| {
                    p.starts_with(root.join("examples"))
                        || p.ends_with("lex.elya")
                        || p.ends_with("emit.elya")
                })
                .collect()
        } else {
            inputs.iter().collect()
        };
        for input in these {
            let run = |exe: &Path| {
                Command::new(exe)
                    .arg(input)
                    .arg(root.join("src/std"))
                    .current_dir(scratch.path())
                    .env_remove("ELYA_RECORD")
                    .env_remove("ELYA_REPLAY")
                    .output()
                    .expect("run a build")
            };
            assert_eq!(
                shown(&run(&elya)),
                shown(&run(&rust)),
                "{tool} on {}: built by the back end in Elya (left), by Rust's (right)",
                input.display()
            );
            compared += 1;
        }
    }
    println!(
        "{compared} runs of the {} self-hosted tools agree",
        tools.len()
    );
    assert!(compared >= 200, "the inputs ({compared})");
}
