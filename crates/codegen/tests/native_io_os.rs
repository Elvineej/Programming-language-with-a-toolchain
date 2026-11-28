// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! `io.os()` natively (spec
//! `docs/superpowers/specs/2026-11-28-elya-io-os-design.md` D1-D3): the
//! answer the evaluator gives on the same platform, recorded, and a replay
//! answering the trace's.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const PROGRAM: &str = "pub fn main() / {IO} { io.println(io.os()) }\n";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-nos-{}-{tag}", std::process::id()));
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

fn run(exe: &Path, env: &[(&str, &Path)]) -> Output {
    let mut c = Command::new(exe);
    c.env_remove("ELYA_RECORD").env_remove("ELYA_REPLAY");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run the binary")
}

#[test]
fn io_os_natively_answers_as_the_evaluator_does() {
    let dir = temp_dir("answer");
    let o = run(&build(PROGRAM, &dir), &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let want = elya::run_source_outcome("main.elya", PROGRAM).output;
    assert_eq!(String::from_utf8_lossy(&o.stdout), format!("{want}0\n"));
}

#[test]
fn io_os_natively_is_recorded_and_replayed() {
    let dir = temp_dir("replay");
    let exe = build(PROGRAM, &dir);
    let trace = dir.join("t.trace");
    let first = run(&exe, &[("ELYA_RECORD", &trace)]);
    assert!(first.status.success());
    let recorded = std::fs::read_to_string(&trace).expect("a trace");
    assert!(recorded.contains("io.os() = \""), "{recorded}");
    let other = dir.join("other.trace");
    std::fs::write(&other, "elya trace 1\nio.os() = \"windows\"\n").expect("write a trace");
    let again = run(&exe, &[("ELYA_REPLAY", &other)]);
    assert_eq!(String::from_utf8_lossy(&again.stdout), "windows\n0\n");
}
