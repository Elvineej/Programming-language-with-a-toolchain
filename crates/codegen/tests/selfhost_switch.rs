//! The switch (self-hosting part 6, spec
//! docs/superpowers/specs/2026-11-27-elya-the-switch-design.md D3): the driver
//! in Elya, `selfhost/elya.elya`, built by the Rust compiler (stage 1),
//! builds itself (stage 2), and stage 2 builds it again (stage 3): stages 2
//! and 3 write the same IR, byte for byte, the compiler's fixed point. Each
//! stage also builds the examples it can, to programs that behave as the
//! Rust-built ones do, and refuses the others by name.
//!
//! On Unix only: the driver runs `clang` with the Rust back end's Unix flags
//! (`-no-pie`, `-pthread`; spec D2) until it knows the platform.
#![cfg(unix)]

#[allow(dead_code)]
#[path = "../../../tests/support/selfhost_check.rs"]
mod check_support;
#[path = "../../../tests/support/selfhost_parse.rs"]
mod parse_support;

use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn elya_builds_itself_to_a_fixed_point() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(switch)
        .expect("switch test thread")
        .join()
        .unwrap();
}

fn exe(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// `compiler build <source> -o <out>`, run in the repository, and its output.
fn stage_build(root: &Path, compiler: &Path, source: &Path, out: &Path) -> (bool, String) {
    let o = Command::new(compiler)
        .current_dir(root)
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(out)
        .arg("--runtime")
        .arg(root.join("crates/codegen/src"))
        .arg("--std")
        .arg(root.join("src/std"))
        .output()
        .expect("run a stage");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    (out.exists(), text)
}

fn switch() {
    let root = parse_support::root();
    let scratch = parse_support::Scratch::new("native-switch");
    let dir = scratch.path();
    // Stage 1: the Rust compiler builds the driver.
    let source = root.join("selfhost/elya.elya");
    let text = std::fs::read_to_string(&source).expect("the driver");
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/elya.elya", &text, &mut check_support::reader);
    assert!(
        diagnostics.is_empty(),
        "driver: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    let (_, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let stage1 = exe(dir, "elya1");
    elya_codegen::compile_module(&core, &dir.join("elya1.o")).expect("compiles");
    elya_codegen::link(&dir.join("elya1.o"), &stage1).expect("links");
    // Stages 2 and 3: the driver built by itself, twice.
    let stage2 = exe(dir, "elya2");
    let (built, out) = stage_build(&root, &stage1, &source, &stage2);
    assert!(built, "stage 1 does not build the driver: {out}");
    let stage3 = exe(dir, "elya3");
    let (built, out) = stage_build(&root, &stage2, &source, &stage3);
    assert!(built, "stage 2 does not build the driver: {out}");
    let ir2 = std::fs::read(format!("{}.ll", stage2.display())).expect("stage 2's IR");
    let ir3 = std::fs::read(format!("{}.ll", stage3.display())).expect("stage 3's IR");
    assert!(
        ir2 == ir3,
        "stages 2 and 3 write different IR: no fixed point"
    );
    // Each stage builds the examples as the Rust compiler does, or refuses one
    // by name.
    let mut examples: Vec<_> = std::fs::read_dir(root.join("examples"))
        .expect("examples")
        .map(|e| e.expect("an entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "elya"))
        .collect();
    examples.sort();
    let mut agreed = 0;
    for example in &examples {
        let name = example.file_stem().unwrap().to_string_lossy().to_string();
        let rust_text = std::fs::read_to_string(example).expect("an example");
        let (m, d, _) =
            elya::front_end_with(&example.display().to_string(), &rust_text, &mut |_| None);
        if d.iter().any(|x| x.severity == elya::diag::Severity::Error) {
            continue;
        }
        let (_, t) = elya::types::infer_typed_table(&elya::Session::new(), &m);
        let rust_exe = exe(dir, &format!("{name}-rust"));
        let c = elya::core::lower_module(&m, &t).expect("lowers");
        elya_codegen::compile_module(&c, &dir.join(format!("{name}-rust.o"))).expect("compiles");
        elya_codegen::link(&dir.join(format!("{name}-rust.o")), &rust_exe).expect("links");
        let want = Command::new(&rust_exe).output().expect("run");
        for (stage, compiler) in [("2", &stage2), ("3", &stage3)] {
            let out_exe = exe(dir, &format!("{name}-stage{stage}"));
            let (built, said) = stage_build(&root, compiler, example, &out_exe);
            if !built {
                assert!(
                    said.contains("unsupported"),
                    "stage {stage} on {name}: neither built nor refused by name: {said}"
                );
                continue;
            }
            let got = Command::new(&out_exe).output().expect("run");
            assert_eq!(
                (got.stdout.clone(), got.stderr.clone(), got.status.code()),
                (want.stdout.clone(), want.stderr.clone(), want.status.code()),
                "stage {stage} on {name}"
            );
            agreed += 1;
        }
    }
    println!("the fixed point holds; {agreed} builds of the examples agree");
    assert!(
        agreed >= 6,
        "the examples built by stages 2 and 3 ({agreed})"
    );
}
