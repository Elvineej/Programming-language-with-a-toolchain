// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! `io.exit` natively (spec
//! `docs/superpowers/specs/2026-11-28-elya-io-exit-design.md` D1, D2, D4):
//! the program ends with the status, its output written and no value line,
//! from a direct function, a non-tail recursion and a handler's clause; a
//! status outside 0..255 fails by name; a replay exits alike.

#[allow(dead_code)] // each suite uses some of the programs
#[path = "../../../tests/programs/io_exit.rs"]
mod programs;
use programs::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-nexit-{}-{tag}", std::process::id()));
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
fn io_exit_natively_ends_with_its_status() {
    for (i, (src, out, status)) in [
        (MIDDLE, "before\n", 3),
        (DEEP, "x\n", 0),
        (HANDLER, "in\n", 7),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = temp_dir(&format!("status{i}"));
        let o = run(&build(src, &dir), &[]);
        assert_eq!(o.status.code(), Some(status), "{src:?}");
        assert_eq!(String::from_utf8_lossy(&o.stdout), out, "{src:?}");
        assert_eq!(String::from_utf8_lossy(&o.stderr), "", "{src:?}");
    }
}

#[test]
fn io_exit_natively_outside_its_range_fails_by_name() {
    for (i, (src, words, out)) in [(HIGH, HIGH_ERR, "x\n"), (LOW, LOW_ERR, "")]
        .into_iter()
        .enumerate()
    {
        let dir = temp_dir(&format!("range{i}"));
        let o = run(&build(src, &dir), &[]);
        assert_eq!(o.status.code(), Some(1), "{src:?}");
        assert_eq!(String::from_utf8_lossy(&o.stdout), out, "{src:?}");
        assert_eq!(String::from_utf8_lossy(&o.stderr), words, "{src:?}");
    }
}

#[test]
fn io_exit_natively_is_not_recorded_and_a_replay_exits_alike() {
    let dir = temp_dir("replay");
    let exe = build(MIDDLE, &dir);
    let trace = dir.join("t.trace");
    let first = run(&exe, &[("ELYA_RECORD", &trace)]);
    assert_eq!(first.status.code(), Some(3));
    let recorded = std::fs::read_to_string(&trace).expect("a trace");
    assert!(!recorded.contains("io.exit"), "{recorded}");
    let again = run(&exe, &[("ELYA_REPLAY", &trace)]);
    assert_eq!((again.status.code(), again.stdout), (Some(3), first.stdout));
}
