//! A linear buffer natively (2026-11-14, spec
//! `docs/superpowers/specs/2026-11-14-elya-buffer-design.md`): the evaluator's
//! text, in place, in linear time, with a collection at every allocation.

use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)] // each suite uses some of the programs
#[path = "../../../tests/programs/buffer.rs"]
mod programs;
use programs::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-nbuffer-{}-{tag}", std::process::id()));
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

#[test]
fn buffers_natively_are_the_evaluators_under_a_collection_at_every_allocation() {
    let dir = temp_dir("same");
    let exe = build(BUILDS, &dir, "builds");
    let want = format!("{}130\n", builds_out());
    for stress in [None, Some("1")] {
        let mut c = Command::new(&exe);
        if let Some(s) = stress {
            c.env("ELYA_GC_STRESS", s);
        }
        let out = c.output().expect("run");
        assert!(out.status.success(), "{stress:?}: {out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{stress:?}");
    }
    let o = elya::run_source_outcome_with("t.elya", BUILDS, &mut |_| None);
    assert_eq!(format!("{}{}\n", o.output, o.line.unwrap().unwrap()), want);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn building_text_with_a_buffer_is_linear_natively() {
    let dir = temp_dir("linear");
    let time = |n: u32| {
        let exe = build(&appends(n), &dir, &format!("a{n}"));
        let t = std::time::Instant::now();
        let out = Command::new(&exe).output().expect("run");
        let took = t.elapsed().as_secs_f64();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{}\n", 10 * n)
        );
        took
    };
    // 80,000 appends took 1.667 s with `<>` (spec D0); with a buffer, both
    // sizes are a few milliseconds, so the bound is on the larger: far below
    // the quadratic time, and linear within a generous factor.
    let large = time(800_000);
    let small = time(200_000).max(1e-3);
    assert!(large < 2.0, "800,000 appends took {large:.3} s");
    assert!(large / small < 10.0, "{large:.3} s against {small:.3} s");
    std::fs::remove_dir_all(&dir).ok();
}
