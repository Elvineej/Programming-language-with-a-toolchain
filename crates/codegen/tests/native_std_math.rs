// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! The same `elya/math` programs and expected results compiled natively.

#[path = "../../../tests/programs/math.rs"]
mod cases;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-native-math-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run(source: &str, dir: &Path) -> Output {
    let (module, diagnostics, sources) =
        elya::front_end_with("math_cases.elya", source, &mut |_| None);
    assert!(
        diagnostics.is_empty(),
        "{}",
        elya::diag::render(&diagnostics, &sources)
    );
    let (diagnostics, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let core = elya::core::lower_module(&module, &table).expect("lower the program");
    let object = dir.join("math.o");
    let binary = dir.join(format!("math{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &object).expect("compile math");
    elya_codegen::link(&object, &binary).expect("link math");
    Command::new(binary).output().expect("run math")
}

#[test]
fn shared_math_cases() {
    let dir = temp_dir("cases");
    for (source, expected) in cases::programs() {
        let output = run(&source, &dir);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, format!("{expected}0\n").as_bytes());
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn shared_math_failures() {
    let dir = temp_dir("failures");
    for &(expression, message) in cases::FAILURES {
        let output = run(&cases::failure_program(expression), &dir);
        assert_eq!(output.status.code(), Some(1), "{expression}: {output:?}");
        assert!(output.stdout.is_empty(), "{expression}");
        assert_eq!(output.stderr, format!("elya: {message}\n").as_bytes());
    }
    std::fs::remove_dir_all(dir).ok();
}
