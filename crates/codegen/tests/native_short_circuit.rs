//! `&&` and `||` short-circuit natively (spec 2026-11-15-elya-short-circuit):
//! the evaluator's answers on the same programs (tests/short_circuit.rs).

use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)] // each suite uses some of the programs
#[path = "../../../tests/programs/short_circuit.rs"]
mod programs;
use programs::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-nshort-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn build(src: &str, dir: &Path, tag: &str) -> PathBuf {
    let (m, pd, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    assert!(pd.is_empty(), "front end: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    let core = elya::core::lower_module(&m, &table).expect("lowers");
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).expect("compile_module");
    elya_codegen::link(&obj, &exe).expect("link");
    exe
}

fn stdout(src: &str, tag: &str) -> String {
    let dir = temp_dir(tag);
    let exe = build(src, &dir, tag);
    let out = Command::new(&exe).output().expect("run");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{tag}: {out:?}");
    String::from_utf8(out.stdout).expect("utf-8")
}

#[test]
fn short_circuits_natively_as_the_evaluator_does() {
    assert_eq!(stdout(SKIPPED, "skipped"), format!("{SKIPPED_VALUE}\n"));
    assert_eq!(stdout(ORDER, "order"), format!("{ORDER_OUT}0\n"));
    assert_eq!(stdout(GUARD, "guard"), format!("{GUARD_VALUE}\n"));
    assert_eq!(stdout(TAIL, "tail"), format!("{TAIL_VALUE}\n"));
    assert_eq!(stdout(MULTI, "multi"), format!("{MULTI_VALUE}\n"));
}
