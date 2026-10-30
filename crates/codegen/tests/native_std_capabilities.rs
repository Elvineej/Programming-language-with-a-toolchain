//! `elya/console`, `elya/clock` and `elya/host` natively (2026-10-24, spec
//! `docs/superpowers/specs/2026-10-24-elya-capabilities-part-3-design.md`): the
//! programs of `tests/std_capabilities.rs` compiled, linked and RUN, their
//! output compared with the evaluator's.

use std::process::Command;

#[path = "../../../tests/programs/std_capabilities.rs"]
mod shared;

fn native_equals_evaluator(tag: &str, root: &str) {
    let dep = shared::DEP.to_string();
    let mut read = move |path: &str| (path == "dep").then(|| ("dep.elya".to_string(), dep.clone()));
    let o = elya::run_source_outcome_with("main.elya", root, &mut read);
    assert!(o.error.is_none(), "{:?}", o.error);
    let want = format!("{}{}\n", o.output, o.line.unwrap().unwrap());
    let (module, diags, _) = elya::front_end_with("main.elya", root, &mut read);
    assert!(diags.is_empty(), "{diags:?}");
    let session = elya::Session::new();
    let (diags, table) = elya::types::infer_typed_table(&session, &module);
    assert!(diags.is_empty(), "{diags:?}");
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let dir = std::env::temp_dir().join(format!("elya-std-cap-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let obj = dir.join("c.o");
    let exe = dir.join(format!("c{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).unwrap_or_else(|e| panic!("{e}"));
    elya_codegen::link(&obj, &exe).expect("link");
    let out = Command::new(&exe).output().expect("run the binary");
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{tag}");
}

#[test]
fn the_standard_capabilities_run_natively() {
    native_equals_evaluator("given", shared::GIVEN_LESS);
    native_equals_evaluator("attenuated", shared::ATTENUATED);
}
