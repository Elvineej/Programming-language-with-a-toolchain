// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! `float.parse`, `float.to_bits`, `float.of_bits` natively (spec
//! `docs/superpowers/specs/2026-11-28-elya-float-in-elya-design.md` D1, D4):
//! the evaluator's answers, and its refusals in the runtime's words.

#[allow(dead_code)] // each suite uses some of the programs
#[path = "../../../tests/programs/float_bits.rs"]
mod programs;
use programs::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-nfb-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn build(src: &str, dir: &Path) -> PathBuf {
    let (m, pd, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    assert!(pd.is_empty(), "front end: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    let core = elya::core::lower_module(&m, &table).expect("lowers");
    let obj = dir.join("p.o");
    let exe = dir.join(format!("p{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).expect("compile_module");
    elya_codegen::link(&obj, &exe).expect("link");
    exe
}

fn run(exe: &Path) -> Output {
    Command::new(exe).output().expect("run the binary")
}

#[test]
fn float_bits_natively_as_the_evaluator() {
    let dir = temp_dir("bits");
    let o = run(&build(&bits_program(), &dir));
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        format!("{}0\n", bits_out())
    );
}

#[test]
fn float_parse_natively_refuses_other_texts_by_name() {
    let dir = temp_dir("refused");
    for (i, t) in REFUSED.iter().enumerate() {
        let d = dir.join(i.to_string());
        std::fs::create_dir_all(&d).expect("dir");
        let o = run(&build(&refused_program(t), &d));
        assert_eq!(o.status.code(), Some(1), "{t:?}");
        assert_eq!(
            String::from_utf8_lossy(&o.stderr),
            format!("elya: float.parse: `{t}` is not a number\n"),
            "{t:?}"
        );
    }
}
