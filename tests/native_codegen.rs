//! Slice 5b-1 — the execution proof. Every test here produces a native binary,
//! RUNS it, and asserts on exit status, stdout, and stderr. Proof is execution,
//! never IR inspection: no insta snapshot of LLVM IR exists anywhere in this
//! slice, and no test may skip (no #[ignore], no toolchain-probe early return).
#![cfg(feature = "codegen")]

use std::path::PathBuf;
use std::process::Command;

use inkwell::context::Context;
use inkwell::targets::{FileType, InitializationConfig, RelocMode, Target, TargetMachine};
use inkwell::AddressSpace;
use inkwell::OptimizationLevel;

/// A unique per-test directory under the OS temp dir — never a tracked path
/// (spec §5). Keyed by process id plus a caller-supplied tag; removed by the
/// caller on success. No `tempfile` dependency: this is the whole harness.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-codegen-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn toolchain_smoke() {
    // Task 1 (spec §9): the entire novel risk isolated before any Elya code —
    // hand-build a trivial LLVM module (no Core, no Elya): @elya_main returns
    // the constant 3; the generated @main shim prints it via printf("%lld\n").
    // Verifies the module, emits an object, links with clang, runs the binary,
    // and asserts stdout is "3" — proving object emission, C-runtime linking,
    // and the %lld round-trip on THIS platform before anything depends on them.
    let ctx = Context::create();
    let i64t = ctx.i64_type();
    let i32t = ctx.i32_type();
    let i8t = ctx.i8_type();
    let module = ctx.create_module("smoke");

    let fn_ty = i64t.fn_type(&[], false);
    let elya_main = module.add_function("elya_main", fn_ty, None);
    let entry = ctx.append_basic_block(elya_main, "entry");
    let b = ctx.create_builder();
    b.position_at_end(entry);
    b.build_return(Some(&i64t.const_int(3, true))).unwrap();

    // One external symbol (printf) and one format-string global — the §4 runtime.
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let printf_ty = i32t.fn_type(&[ptrt.into(), i64t.into()], true);
    let printf = module.add_function("printf", printf_ty, None);

    let fmt_bytes: &[u8] = b"%lld\n\0";
    let fmt_const = i8t.const_array(
        &fmt_bytes
            .iter()
            .map(|c| i8t.const_int(*c as u64, false))
            .collect::<Vec<_>>(),
    );
    let fmt = module.add_global(fmt_const.get_type(), Some(AddressSpace::default()), ".fmt");
    fmt.set_initializer(&fmt_const);
    fmt.set_constant(true);
    fmt.set_unnamed_addr(true);

    let shim = module.add_function("main", i32t.fn_type(&[], false), None);
    let shim_entry = ctx.append_basic_block(shim, "entry");
    b.position_at_end(shim_entry);
    let v = b
        .build_call(elya_main, &[], "v")
        .unwrap()
        .try_as_basic_value()
        .left()
        .expect("elya_main returns a value");
    b.build_call(printf, &[fmt.as_pointer_value().into(), v.into()], "p")
        .unwrap();
    b.build_return(Some(&i32t.const_int(0, false))).unwrap();

    module.verify().expect("hand-built module verifies");

    Target::initialize_native(&InitializationConfig::default()).expect("init native target");
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).expect("host target");
    let machine = target
        .create_target_machine(
            &triple,
            "",
            "",
            OptimizationLevel::None,
            RelocMode::Default,
            inkwell::targets::CodeModel::Default,
        )
        .expect("host target machine");

    let dir = temp_dir("toolchain-smoke");
    let obj = dir.join("smoke.o");
    let exe = dir.join(format!("smoke{}", std::env::consts::EXE_SUFFIX));
    machine
        .write_to_file(&module, FileType::Object, &obj)
        .expect("emit object");

    let linked = Command::new("clang")
        .arg(&obj)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("spawn clang");
    assert!(
        linked.status.success(),
        "clang failed: {}",
        String::from_utf8_lossy(&linked.stderr)
    );

    let out = Command::new(&exe).output().expect("run produced binary");
    assert!(out.status.success(), "binary exited {:?}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "3");
    assert!(
        String::from_utf8_lossy(&out.stderr).is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();
}
