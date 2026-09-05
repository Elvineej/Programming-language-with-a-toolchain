//! Slice 5b-1 — the execution proof. Every test here produces a native binary,
//! RUNS it, and asserts on exit status, stdout, and stderr. Proof is execution,
//! never IR inspection: no insta snapshot of LLVM IR exists anywhere in this
//! slice, and no test may skip (no #[ignore], no toolchain-probe early return).

use elya::core::lower_module;
use elya::parse::parse_module;
use elya::Session;
use std::path::{Path, PathBuf};
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

/// The §5 corpus: (tag, source). Four cases so the proof is not "a program
/// that prints a hardcoded 3".
const CORPUS: &[(&str, &str)] = &[
    ("spine", "pub fn main() { 1 + 2 }\n"),
    (
        "lets",
        "pub fn main() {\n  let x = 6\n  let y = 7\n  x * y\n}\n",
    ),
    ("nesting", "pub fn main() { (2 + 3) * 4 - 5 }\n"),
    ("negative", "pub fn main() { 3 - 10 }\n"),
];

/// Parse → full front-end check → raw type table → lower. Runs the REAL
/// pipeline: `check_source` exercises resolve + inference + exhaustiveness +
/// affinity exactly as `elya check` does; only the table/lowering half is
/// repeated here because `front_end` is private and the frozen table is the
/// public accessor's product.
fn lower_src(src: &str) -> elya::core::CoreModule {
    assert!(
        elya::check_source("corpus.elya", src).is_ok(),
        "front end rejected corpus program: {src}"
    );
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&session, &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    lower_module(&m, &table).expect("corpus program must lower to Core")
}

#[test]
fn corpus_lowers_to_core_through_the_real_pipeline() {
    // Task 2 (spec §6.3): measure, don't audit. Each corpus program must reach
    // CoreModule unchanged. LowerError::Untyped(span) would be a recorder gap ON
    // THIS SLICE'S PATH — fix it in inference (record the synthesized node's type
    // at its span, the perform-callee shape from 5a-2 Task 3). LowerError::
    // Unsupported means the subset was drawn wrong — narrow the corpus, never
    // widen core.rs. Expected: all four lower today, zero gaps closed.
    // Slice 5b-3 §7.3: the `core.fns.len() == 1` assertion below is scoped to
    // CORPUS on purpose — the 5b-1 arithmetic programs are single-function and
    // stay that way. Multi-function lowering is asserted by
    // `the_function_corpus_lowers_to_multi_function_core`.
    for (tag, src) in CORPUS {
        let core = lower_src(src);
        assert_eq!(core.fns.len(), 1, "{tag}: expected exactly one fn");
        assert_eq!(core.fns[0].name, "main", "{tag}");
        assert!(core.fns[0].params.is_empty(), "{tag}");
    }
}

/// Compile + link through the LIBRARY API into `dir`. Task 5 adds the CLI path.
fn compile_and_link(core: &elya::core::CoreModule, dir: &Path, tag: &str) -> PathBuf {
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(core, &obj).expect("compile_module");
    elya_codegen::link(&obj, &exe).expect("link");
    exe
}

/// Windows STATUS_STACK_OVERFLOW. Observing it from a corpus binary means one
/// thing: a call that must have been eliminated was not. Inert on other
/// platforms, where no exit code collides with it.
const STACK_OVERFLOW: i32 = 0xC00000FDu32 as i32; // -1073741571

/// Spec §7.2: the distinct failure signal, diagnosed BY NAME. Without this, a
/// tail-call regression surfaces as `binary exited ExitStatus(3221225725)` —
/// a number nobody recognizes — instead of naming its own cause.
fn diagnose_stack_overflow(status: &std::process::ExitStatus, tag: &str) {
    if status.code() == Some(STACK_OVERFLOW) {
        panic!(
            "{tag}: STATUS_STACK_OVERFLOW (0x{:08X}). A tail call that `musttail` \
             was supposed to eliminate grew the machine stack instead. This is the \
             tail-call guarantee failing, not a generic crash.",
            STACK_OVERFLOW as u32
        );
    }
}

/// The three required assertions per case (§5): exit status, stdout, empty stderr.
fn assert_runs(exe: &Path, expected: &str) {
    let out = Command::new(exe).output().expect("run produced binary");
    diagnose_stack_overflow(&out.status, &exe.display().to_string());
    assert!(out.status.success(), "binary exited {:?}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected);
    assert!(
        String::from_utf8_lossy(&out.stderr).is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn spine_prints_three() {
    let core = lower_src(CORPUS[0].1);
    let dir = temp_dir("spine");
    let exe = compile_and_link(&core, &dir, CORPUS[0].0);
    assert_runs(&exe, "3"); // the spine
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn lets_and_mul_print_forty_two() {
    let core = lower_src(CORPUS[1].1);
    let dir = temp_dir("lets");
    let exe = compile_and_link(&core, &dir, CORPUS[1].0);
    assert_runs(&exe, "42"); // Let, Var, Mul
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn nesting_and_sub_print_fifteen() {
    let core = lower_src(CORPUS[2].1);
    let dir = temp_dir("nesting");
    let exe = compile_and_link(&core, &dir, CORPUS[2].0);
    assert_runs(&exe, "15"); // nesting, precedence, Sub
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn negative_result_prints_minus_seven() {
    let core = lower_src(CORPUS[3].1);
    let dir = temp_dir("negative");
    let exe = compile_and_link(&core, &dir, CORPUS[3].0);
    assert_runs(&exe, "-7"); // signed negatives survive %lld
    std::fs::remove_dir_all(&dir).ok();
}

/// The 5b-2 §5 corpus: (tag, source, expected stdout). Seven programs, each
/// aimed at one thing the diamond can get wrong.
const CONTROL_FLOW_CORPUS: &[(&str, &str, &str)] = &[
    // 1-2: both directions of the same branch, so a diamond that always takes
    // one side fails one of them.
    (
        "if_true",
        "pub fn main() { if 1 < 2 { 10 } else { 20 } }\n",
        "10",
    ),
    (
        "if_false",
        "pub fn main() { if 2 < 1 { 10 } else { 20 } }\n",
        "20",
    ),
    // 3: a binding live across the branch — the env must survive the diamond.
    (
        "if_over_a_binding",
        "pub fn main() {\n  let x = 5\n  if x > 3 { x * 2 } else { 0 }\n}\n",
        "10",
    ),
    // 4: THE PHI TRAP, on both sides. `a` nests inside the then-branch, `b`
    // inside the else-branch; an outer phi that names `then`/`else` instead of
    // the inner join blocks miscompiles or fails the verifier.
    (
        "nested_if",
        "pub fn main() {\n  let x = 7\n  let a = if x > 0 { if x > 5 { 100 } else { 50 } } else { 0 }\n  let b = if x < 0 { 0 } else { if x > 5 { 7 } else { 3 } }\n  a + b\n}\n",
        "107",
    ),
    // 5: all six predicates, each at its boundary case, so swapping SLT for SLE
    // (or SGT for SGE) changes the answer. Plus one signedness probe: under an
    // unsigned compare `(0 - 1) < 1` is false, and the total drops to 6.
    (
        "predicates",
        "pub fn main() {\n  let a = if 1 < 1 { 1 } else { 0 }\n  let b = if 1 < 2 { 1 } else { 0 }\n  let c = if 1 <= 1 { 1 } else { 0 }\n  let d = if 2 <= 1 { 1 } else { 0 }\n  let e = if 1 > 1 { 1 } else { 0 }\n  let f = if 2 > 1 { 1 } else { 0 }\n  let g = if 1 >= 1 { 1 } else { 0 }\n  let h = if 1 >= 2 { 1 } else { 0 }\n  let i = if 1 == 1 { 1 } else { 0 }\n  let j = if 1 == 2 { 1 } else { 0 }\n  let k = if 1 != 2 { 1 } else { 0 }\n  let m = if 1 != 1 { 1 } else { 0 }\n  let n = if (0 - 1) < 1 { 1 } else { 0 }\n  a + b + c + d + e + f + g + h + i + j + k + m + n\n}\n",
        "7",
    ),
    // 6: Eq on Bool operands — the i1 path through the polymorphic operator.
    (
        "bool_equality",
        "pub fn main() { if True == False { 1 } else { 2 } }\n",
        "2",
    ),
    // 7: strict and/or over two comparisons, both truth values of each.
    (
        "and_or",
        "pub fn main() {\n  let a = if 1 < 2 && 3 > 4 { 1 } else { 0 }\n  let b = if 1 < 2 && 3 < 4 { 1 } else { 0 }\n  let c = if 1 > 2 || 3 > 4 { 1 } else { 0 }\n  let d = if 1 > 2 || 3 < 4 { 1 } else { 0 }\n  a + b + c + d\n}\n",
        "2",
    ),
];

/// The 5b-3 §7.1 corpus, non-tail half: (tag, source, expected stdout). Six
/// programs, each aimed at one thing multi-function emission can get wrong.
/// The two deep tail-recursive programs live in TAIL_CORPUS (Task 4) because
/// they only pass once `musttail` is emitted.
const FUNCTION_CORPUS: &[(&str, &str, &str)] = &[
    (
        "two_functions",
        "fn add3(x) { x + 3 }\npub fn main() { add3(4) }\n",
        "7",
    ),
    (
        "five_params",
        "fn add5(a, b, c, d, e) { a + b + c + d + e }\npub fn main() { add5(1, 2, 3, 4, 5) }\n",
        "15",
    ),
    (
        // Environments are per-function: both `f` and `g` bind a parameter
        // named `x`, and `f` shadows its own with a `let`. If the value
        // environment leaked across the call, or the shadow were not restored,
        // this prints something other than 50. f(10) = 10*2 + 20 = 40; g(10) =
        // 10 + f(10) = 50.
        "distinct_envs",
        "fn f(x) {\n  let x = x * 2\n  x + 20\n}\nfn g(x) { x + f(x) }\npub fn main() { g(10) }\n",
        "50",
    ),
    (
        // Calls as operands, including a call whose argument is a call.
        // dbl(dbl(3)) + dbl(1) = 12 + 2 = 14.
        "call_in_operand_position",
        "fn dbl(x) { x * 2 }\npub fn main() { dbl(dbl(3)) + dbl(1) }\n",
        "14",
    ),
    (
        // An i1 crosses the call boundary and is consumed as an `if`
        // condition. Also §5.5's live proof that `require_int` applies to
        // `main` ALONE: `is_pos` returns Bool and must compile.
        "bool_across_a_call",
        "fn is_pos(n) { n > 0 }\npub fn main() { if is_pos(3) { 1 } else { 0 } }\n",
        "1",
    ),
    (
        // Ordinary (non-tail) recursion. Deliberately shallow — spec §6.3
        // Limitation L1: native non-tail recursion grows the machine stack,
        // which is bounded differently from the evaluator's Kont stack, and the
        // harness compares answers, not resource behavior. sum(100) = 5050.
        "shallow_non_tail_recursion",
        "fn sum(n) { if n == 0 { 0 } else { n + sum(n - 1) } }\npub fn main() { sum(100) }\n",
        "5050",
    ),
];

#[test]
fn the_function_corpus_compiles_runs_and_prints_the_expected_answer() {
    let dir = temp_dir("functions");
    for (tag, src, expected) in FUNCTION_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        assert_runs(&exe, expected);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_output_matches_the_evaluator_across_the_function_corpus() {
    // The fidelity teeth, extended to N2. The expected strings above are a
    // human's arithmetic; this asserts against what the CEK evaluator actually
    // computes, so a wrong expectation cannot make a wrong compiler look right.
    let dir = temp_dir("differential-functions");
    for (tag, src, _) in FUNCTION_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let out = Command::new(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        assert!(
            out.status.success(),
            "{tag}: binary exited {:?}",
            out.status
        );
        let native = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(
            native,
            eval_main_int(src),
            "{tag}: native output diverges from the evaluator"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_function_corpus_lowers_to_multi_function_core() {
    // Spec §7.2's recorder-totality assertion for the new key class, stated
    // directly: `lower_src` unwraps `lower_module`, so a `LowerError::Untyped`
    // from a parameter span fails here by name. Also the §7.3 counterpart to
    // `corpus_lowers_to_core_through_the_real_pipeline`, which stays scoped to
    // the single-function CORPUS.
    for (tag, src, _) in FUNCTION_CORPUS {
        let core = lower_src(src);
        assert!(core.fns.iter().any(|f| f.name == "main"), "{tag}: no main");
        for f in &core.fns {
            assert!(
                f.params.len() <= 5,
                "{tag}: {} exceeds the arity cap",
                f.name
            );
        }
    }
    let multi = lower_src(FUNCTION_CORPUS[0].1);
    assert_eq!(
        multi.fns.len(),
        2,
        "two_functions must lower to two Core fns"
    );
}

/// The reference side of the differential check (§5): what the CEK evaluator
/// says `main` is worth, rendered the way the native print shim prints it
/// (`printf("%lld\n", …)`, trimmed by the caller).
///
/// `elya run` cannot serve here — it observes only `io.println` output and
/// discards main's value — which is why `run_module_value` exists.
fn eval_main_int(src: &str) -> String {
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (_, v) = elya::eval::run_module_value(&m).expect("evaluator must run corpus program");
    match v {
        elya::eval::Value::Int(n) => n.to_string(),
        other => panic!("corpus main must evaluate to an Int, got {other:?}"),
    }
}

#[test]
fn control_flow_corpus_compiles_links_and_runs() {
    let dir = temp_dir("control-flow");
    for (tag, src, expected) in CONTROL_FLOW_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        assert_runs(&exe, expected);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_output_matches_the_evaluator_across_the_control_flow_corpus() {
    // The fidelity teeth for the whole back-end arc: "native must match the
    // evaluator" stops being a remembered rule and becomes an enforced test.
    // This is what would have caught the strictness divergence automatically.
    let dir = temp_dir("differential");
    for (tag, src, _) in CONTROL_FLOW_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let out = Command::new(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        assert!(
            out.status.success(),
            "{tag}: binary exited {:?}",
            out.status
        );
        let native = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(
            native,
            eval_main_int(src),
            "{tag}: native output diverges from the evaluator"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_differential_check_also_covers_the_arithmetic_corpus() {
    // The 5b-1 programs predate the check; running them through it costs one
    // loop and means the whole native surface is covered, not just the new part.
    let dir = temp_dir("differential-arith");
    for (tag, src) in CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let out = Command::new(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        assert!(
            out.status.success(),
            "{tag}: binary exited {:?}",
            out.status
        );
        let native = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(native, eval_main_int(src), "{tag}: native diverges");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The 5b-3 §7.1 corpus, tail half: (tag, source, expected stdout). These two
/// are the reason this slice exists. Both recur one million deep; without
/// `musttail` both overflow the 1 MiB Windows stack long before returning.
const TAIL_CORPUS: &[(&str, &str, &str)] = &[
    (
        "deep_self_tail_recursion",
        "fn down(n) { if n == 0 { 0 } else { down(n - 1) } }\npub fn main() { down(1000000) }\n",
        "0",
    ),
    (
        // `main`'s call to `ev` sits in `if`-condition position, so it is an
        // ordinary non-tail call — one frame, which is fine. The million-deep
        // recursion is the ev<->od pair, and both of those calls are in tail
        // position. The `if` wrapper is what keeps `main` Int-returning, which
        // §5.5's `require_int` demands while `ev` itself returns Bool.
        "deep_mutual_tail_recursion",
        "fn ev(n) { if n == 0 { True } else { od(n - 1) } }\n\
         fn od(n) { if n == 0 { False } else { ev(n - 1) } }\n\
         pub fn main() { if ev(1000000) { 1 } else { 0 } }\n",
        "1",
    ),
];

#[test]
fn mutual_tail_recursion_at_one_million_is_eliminated() {
    // THE primary acceptance criterion for this slice.
    //
    // Self-recursion is NOT sufficient evidence. A self-call can be turned into
    // a branch back to the entry block, so `down(1000000)` could pass with no
    // tail-call machinery at all — the compiler would have proved something
    // weaker than what we claim. `ev` and `od` cannot be looped without merging
    // the two functions, so only the mutual case proves that `musttail`, and
    // not an accidental loop rewrite, is what bounds the stack.
    //
    // The proof is black-box: the binary exits 0 and prints the right answer.
    // The distinct failure signal is STATUS_STACK_OVERFLOW, diagnosed by name
    // inside `assert_runs`.
    let (tag, src, expected) = TAIL_CORPUS[1];
    assert_eq!(
        tag, "deep_mutual_tail_recursion",
        "TAIL_CORPUS was reordered"
    );
    let dir = temp_dir("mutual-tail");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, tag);
    assert_runs(&exe, expected);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn self_tail_recursion_at_one_million_is_eliminated() {
    // The corroborating case, weaker on its own than the mutual one above but
    // cheap and a useful bisection point: if this passes and the mutual test
    // fails, the two-pass declaration scheme is what broke, not `musttail`.
    let (tag, src, expected) = TAIL_CORPUS[0];
    assert_eq!(tag, "deep_self_tail_recursion", "TAIL_CORPUS was reordered");
    let dir = temp_dir("self-tail");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, tag);
    assert_runs(&exe, expected);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_output_matches_the_evaluator_across_the_tail_corpus() {
    // Both bounds, on the same two programs (spec §6.2). `elya run` bounds these
    // by TCE in the CEK machine, enforced at test time by tests/tce.rs's K_MAX;
    // native bounds them by `musttail`, enforced at build time by the LLVM
    // verifier. This asserts the two agree on the ANSWER — §6.3's L1 is explicit
    // that the harness does not compare resource behavior.
    let dir = temp_dir("differential-tail");
    for (tag, src, _) in TAIL_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let out = Command::new(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        assert!(
            out.status.success(),
            "{tag}: binary exited {:?}",
            out.status
        );
        let native = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(
            native,
            eval_main_int(src),
            "{tag}: native output diverges from the evaluator"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The 5b-4 §7 corpus: (tag, source, expected stdout). Monomorphic ADTs only —
/// construct → match → extract an Int, so `require_int` on `main` stays satisfied.
const ADT_CORPUS: &[(&str, &str, &str)] = &[
    (
        "option_extract",
        "type Opt { None, Some(Int) }\npub fn main() { match Some(42) { None -> 0  Some(x) -> x } }\n",
        "42",
    ),
    (
        // Recursive: the Succ field is a *pointer* to another Nat — the load-bearing
        // representation fact (§6).
        "recursive_nat",
        "type Nat { Zero, Succ(Nat) }\nfn len(n) { match n { Zero -> 0  Succ(m) -> 1 + len(m) } }\npub fn main() { len(Succ(Succ(Succ(Zero)))) }\n",
        "3",
    ),
    (
        // Three constructors, so the join phi has N=3 incoming edges, not two.
        "three_way",
        "type T { A, B, C(Int) }\npub fn main() { match C(7) { A -> 1  B -> 2  C(x) -> x } }\n",
        "7",
    ),
];

#[test]
fn the_adt_corpus_compiles_runs_and_prints_the_expected_answer() {
    let dir = temp_dir("adts");
    for (tag, src, expected) in ADT_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        assert_runs(&exe, expected);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_output_matches_the_evaluator_across_the_adt_corpus() {
    let dir = temp_dir("differential-adts");
    for (tag, src, _) in ADT_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let out = Command::new(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        assert!(
            out.status.success(),
            "{tag}: binary exited {:?}",
            out.status
        );
        let native = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(
            native,
            eval_main_int(src),
            "{tag}: native output diverges from the evaluator"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_nested_match_joins_through_an_n_armed_phi() {
    // A match nested inside an arm leaves the builder in the INNER match's join
    // block; the outer phi must read that block back with get_insert_block(), not
    // assume the arm's body block (the N3 nested-if trap, one dimension wider).
    let src = "type T { A, B, C(T) }\nfn f(n) { match n { A -> 0  B -> 1  C(m) -> match m { A -> 10  B -> 11  C(_) -> 12 } } }\npub fn main() { f(C(C(B))) }\n";
    let dir = temp_dir("nested-match");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "nested-match");
    assert_runs(&exe, "12");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_failed_match_traps_with_a_named_error() {
    // The default block is unreachable for well-typed programs (exhaustiveness),
    // so this hand-builds a Core whose match covers only `A` while the scrutinee
    // is `B` — the fall-through. The binary must exit non-zero with the
    // `elya_match_fail` message, not crash generically and not hit `unreachable`.
    use elya::core::{
        CoreArm, CoreCtor, CoreExpr, CoreFn, CoreKind, CoreLit, CoreModule, CorePat, CoreType,
    };
    use elya::span::Span;
    use elya::types::{Ty, TyCon};
    use std::rc::Rc;

    let core = CoreModule {
        types: vec![CoreType {
            name: "T".into(),
            ctors: vec![
                CoreCtor {
                    name: "A".into(),
                    fields: vec![],
                },
                CoreCtor {
                    name: "B".into(),
                    fields: vec![],
                },
            ],
        }],
        fns: vec![CoreFn {
            name: "main".into(),
            params: Rc::from([]),
            body: CoreExpr {
                span: Span::EMPTY,
                ty: Ty::Base(TyCon::Int),
                kind: CoreKind::Match(
                    Rc::new(CoreExpr {
                        span: Span::EMPTY,
                        ty: Ty::Con("T".into(), vec![]),
                        kind: CoreKind::Ctor("B".into(), Rc::from([])),
                    }),
                    Rc::from([CoreArm {
                        pat: CorePat::Ctor("A".into(), Rc::from([])),
                        body: CoreExpr {
                            span: Span::EMPTY,
                            ty: Ty::Base(TyCon::Int),
                            kind: CoreKind::Lit(CoreLit::Int(0)),
                        },
                    }]),
                ),
            },
        }],
    };

    let dir = temp_dir("trap");
    let exe = compile_and_link(&core, &dir, "trap");
    let out = Command::new(&exe).output().expect("run produced binary");
    assert!(
        !out.status.success(),
        "a failed match must exit non-zero, got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("elya: match failed"),
        "stderr should name the trap, got: {stderr}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The collector's own counters, read back from a run with `ELY_GC_STATS=1`.
/// That variable is the only reason `elya_gc_report` prints anything: every
/// other test in this file asserts stderr is EMPTY, and that stays true.
#[derive(Debug)]
struct GcStats {
    collections: i64,
    freed: i64,
    /// Visible words still live at the end of the LAST collection -- the LEVEL,
    /// where `freed` and `words_since_gc` are flows (5b-6 s11, obligation T7).
    live: i64,
}

/// Run `exe` with statistics enabled, returning its stdout and the counters.
fn run_with_gc_stats(exe: &Path, tag: &str) -> (String, GcStats) {
    let out = Command::new(exe)
        .env("ELY_GC_STATS", "1")
        .output()
        .expect("run produced binary");
    diagnose_stack_overflow(&out.status, tag);
    assert!(
        out.status.success(),
        "{tag}: binary exited {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let line = stderr
        .lines()
        .find(|l| l.starts_with("elya-gc:"))
        .unwrap_or_else(|| panic!("{tag}: no elya-gc line in stderr: {stderr}"));
    let field = |key: &str| -> i64 {
        line.split_whitespace()
            .find_map(|f| f.strip_prefix(key))
            .unwrap_or_else(|| panic!("{tag}: no `{key}` field in: {line}"))
            .parse()
            .unwrap_or_else(|e| panic!("{tag}: `{key}` is not a number in `{line}`: {e}"))
    };
    let stats = GcStats {
        collections: field("collections="),
        freed: field("freed="),
        live: field("live="),
    };
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        stats,
    )
}

#[test]
fn an_unbounded_allocating_loop_collects_and_frees() {
    // 5b-5 Task 3, tooth one: three million words allocated by a program whose
    // live set never exceeds one object, because the loop discards each
    // `Link(End)` the moment it is built.
    //
    // TWO assertions, and the second is what makes the first mean something.
    // `collections > 0` on its own also passes for a run that merely reached the
    // end, and a collection that marked everything and reclaimed nothing would
    // satisfy it while the heap grew without bound. `freed > 0` is the half that
    // says memory actually came back.
    //
    // The tail call is what keeps the machine stack flat while the heap churns;
    // if that regressed, this arrives as STATUS_STACK_OVERFLOW and
    // `diagnose_stack_overflow` names it rather than printing a bare number.
    let src = "type Node { End, Link(Node) }\nfn loop(n) { if n == 0 { 0 } else { let _ = Link(End)  loop(n - 1) } }\npub fn main() { loop(1000000) }\n";
    let dir = temp_dir("gc-unbounded");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "gc-unbounded");
    let (stdout, stats) = run_with_gc_stats(&exe, "gc-unbounded");
    assert_eq!(stdout, "0");
    assert!(stats.collections > 0, "the collector never ran: {stats:?}");
    assert!(
        stats.freed > 0,
        "collections ran but reclaimed nothing: {stats:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ordinary_programs_finish_without_collecting() {
    // Tooth two, and it is the one that keeps tooth one honest. A threshold low
    // enough to trip on everyday programs would satisfy `collections > 0`
    // trivially, proving that the constant is small rather than that the
    // collector works. `GC_THRESHOLD_WORDS` is pinned at `1 << 16` against a
    // measured corpus peak of SEVEN words, so nothing here is near the line —
    // and this test is what would notice if a future change moved it there.
    let dir = temp_dir("gc-no-trip");
    for (tag, src, expected) in ADT_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let (stdout, stats) = run_with_gc_stats(&exe, tag);
        assert_eq!(&stdout, expected, "{tag}");
        assert_eq!(
            stats.collections, 0,
            "{tag}: an ordinary program collected — the threshold is too low: {stats:?}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_live_binding_survives_collection_across_a_call() {
    // The root-discipline proof. `a` is live in `main`'s environment across a
    // NON-TAIL call that allocates megabytes; nothing on the heap points at it,
    // only main's frame does. Unless the call site roots the caller's bindings,
    // the mark phase cannot see `a` and sweep reclaims it while it is still live.
    //
    // The payload is what gives this teeth, and that was learned the hard way. An
    // earlier version of this test matched on a constructor TAG and passed even
    // with rooting disabled: a swept block's tag slot holds the free-list link,
    // which fails the first arm and falls into the second, printing the right
    // answer by luck. `12345` cannot be forged — a recycled block is re-zeroed,
    // and a block reused as a `Link` holds a pointer. Confirmed by construction:
    // with `gc_root_env` stubbed to push nothing, this program dies in
    // `elya_match_fail` instead of printing.
    //
    // `churn` is tail-recursive, so the machine stack stays flat while the heap
    // crosses the threshold several times over.
    let src = "type L { End, Link(L) }\ntype Box { B(Int) }\nfn churn(n) { if n == 0 { 0 } else { let _ = Link(End)  churn(n - 1) } }\npub fn main() { let a = B(12345)  let _ = churn(100000)  match a { B(x) -> x } }\n";
    let dir = temp_dir("gc-roots");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "gc-roots");
    let (stdout, stats) = run_with_gc_stats(&exe, "gc-roots");
    assert!(
        stats.collections > 0,
        "no collection happened across the call, so nothing was proved: {stats:?}"
    );
    assert_eq!(
        stdout, "12345",
        "the live binding did not survive collection"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 5b-6 §3. A closure captures a local, outlives the scope that created it (the
/// only scope-ender in Elya is a function return), and computes with the captured
/// value when called later. This is the slice's basic claim, checked by running.
#[test]
fn a_closure_captures_and_is_called_natively() {
    let src = "fn wrap(k) { fn(x) { x + k } }\npub fn main() { let f = wrap(10)  f(32) }\n";
    let dir = temp_dir("clos-basic");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "clos-basic");
    assert_runs(&exe, "42");
    std::fs::remove_dir_all(&dir).ok();
}

/// 5b-6 §5.2. A closure called in TAIL position recurs to a depth that would
/// exhaust the stack under a plain call. The C-ii probe measured that `musttail`
/// through a loaded code pointer under `tailcc` emits a real indirect tail jump
/// (`jmpq *%rax`) rather than degrading silently to a call; this is that
/// measurement re-checked by execution, at 1,000,000 frames.
#[test]
fn a_closure_tail_call_recurs_in_bounded_stack() {
    let src = "fn mk() { fn(n) { if n == 0 { 7 } else { down(n - 1) } } }\n\
               fn down(n) { if n == 0 { 7 } else { down(n - 1) } }\n\
               pub fn main() { let f = mk()  f(1000000) }\n";
    let dir = temp_dir("clos-tail");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "clos-tail");
    assert_runs(&exe, "7");
    std::fs::remove_dir_all(&dir).ok();
}

/// CR-3, direction (a) — the predicate this trips is `gc_mark`'s
/// `tag >= gc_n_ctors` skip (`runtime.c:141`).
///
/// Without a descriptor row for the closure's synthetic tag, the mark phase hits
/// that `continue` and never traces the closure's captures. `wrap` exists because
/// a function return is the only scope-ender in Elya: after it, the list is
/// reachable ONLY through the closure. `churn` then forces a real collection, and
/// the captured `Cons(7, Nil)` is swept while still live.
///
/// This fails SILENTLY on the un-fixed build — it prints a wrong `Int`, it does
/// not crash — which is why the assertion is on the VALUE, not on survival.
#[test]
fn a_closure_capture_survives_collection_descriptor_row_present() {
    let src = "type L { Nil, Cons(Int, L) }\n\
               fn head_or(d, xs) { match xs { Nil -> d  Cons(h, t) -> h } }\n\
               fn wrap(xs) { fn(d) { head_or(d, xs) } }\n\
               fn churn(n) { if n == 0 { 0 } else { let _ = Cons(1, Nil)  churn(n - 1) } }\n\
               pub fn main() { let f = wrap(Cons(7, Nil))  let _ = churn(100000)  f(0) }\n";
    let dir = temp_dir("clos-gc-tag");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "clos-gc-tag");
    let (stdout, stats) = run_with_gc_stats(&exe, "clos-gc-tag");
    assert!(
        stats.collections > 0,
        "no collection happened across the call, so nothing was proved: {stats:?}"
    );
    assert_eq!(
        stdout, "7",
        "the captured list did not survive collection — the closure's tag has no \
         descriptor row, so gc_mark skipped it"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// CR-3, direction (b) — the predicate this trips is the descriptor table's
/// pointer-mask test, `matches!(f, Ty::Con(..))`, un-widened for `Ty::Fn`.
///
/// `outer` captures `inner`, whose type is `Ty::Fn`. Under the un-widened
/// predicate `inner`'s mask bit is CLEAR, so `outer`'s row says "not a pointer"
/// and `inner` is never traced — even though `outer` itself has a descriptor row
/// and is traced fine. An ADT capture cannot show this: `Ty::Con` already sets the
/// bit, so an ADT test passes on the un-fixed build and controls nothing.
///
/// On the un-fixed build this dereferences a swept block. The exit code is
/// deliberately NOT pinned (an access violation is not a defined outcome); what
/// is pinned is that it does not print 42.
#[test]
fn a_captured_closure_is_traced_mask_covers_ty_fn() {
    let src = "fn churn(n) { if n == 0 { 0 } else { let _ = mk(n)  churn(n - 1) } }\n\
               fn mk(k) { fn(z) { z + k } }\n\
               fn wrap(k) { let inner = fn(x) { x + k }\n\
                            let outer = fn(y) { inner(y) }\n\
                            outer }\n\
               pub fn main() { let f = wrap(10)  let _ = churn(100000)  f(32) }\n";
    let dir = temp_dir("clos-gc-mask");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "clos-gc-mask");
    let (stdout, stats) = run_with_gc_stats(&exe, "clos-gc-mask");
    assert!(
        stats.collections > 0,
        "no collection happened across the call, so nothing was proved: {stats:?}"
    );
    assert_eq!(
        stdout, "42",
        "the captured CLOSURE did not survive collection — its mask bit was clear \
         because the predicate only recognised Ty::Con"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 5b-6 s11, obligation T7. The steady-state live set of a program whose live data
/// does NOT grow with its iteration count is INDEPENDENT of that count. The same
/// program is run at four counts spanning an 8x range and the `live` figures are
/// asserted equal -- which makes the claim without pinning a magic number, so there
/// is no constant here a future change could be tempted to nudge and no expected
/// value to edit.
///
/// FOUR points rather than two, deliberately. Two figures agreeing is weak evidence
/// that a level SETTLES: the last collection of a single pair could land at the same
/// loop phase by luck. Agreement across an 8x spread is the settling claim itself.
/// `a_growing_live_set_moves_the_instrument` is the other half -- it proves this
/// equality is capable of failing, so satisfying it means something.
///
/// This is the instrument obligation T7 will be measured with. It is built now,
/// while acyclicity makes refcount/tracing divergence unconstructible, so the day a
/// cycle becomes constructible (N8, `Value::Resume` holding captured frames) the
/// measurement already exists rather than being invented under pressure.
#[test]
fn the_live_set_settles_independent_of_iteration_count() {
    let prog = |n: i64| {
        format!(
            "type L {{ Nil, Cons(Int, L) }}\n\
             fn churn(n) {{ if n == 0 {{ 0 }} else {{ let _ = Cons(1, Nil)  churn(n - 1) }} }}\n\
             pub fn main() {{ let keep = Cons(5, Cons(6, Nil))  let _ = churn({n})  \
             match keep {{ Nil -> 0  Cons(h, t) -> h }} }}\n"
        )
    };
    let mut seen: Vec<(i64, i64)> = Vec::new();
    for n in [50000i64, 100000, 200000, 400000] {
        let tag = format!("gc-live-{n}");
        let dir = temp_dir(&tag);
        let core = lower_src(&prog(n));
        let exe = compile_and_link(&core, &dir, &tag);
        let (stdout, stats) = run_with_gc_stats(&exe, &tag);
        assert_eq!(stdout, "5", "{tag}: the retained list did not survive");
        assert!(
            stats.collections > 0,
            "{tag}: no collection happened, so `live` was never computed: {stats:?}"
        );
        assert!(
            stats.live > 0,
            "{tag}: a live retained list must contribute live words: {stats:?}"
        );
        seen.push((n, stats.live));
        std::fs::remove_dir_all(&dir).ok();
    }
    println!("live by iteration count: {seen:?}");
    let first = seen[0].1;
    assert!(
        seen.iter().all(|&(_, live)| live == first),
        "the live set must not grow with the iteration count -- it settles: {seen:?}"
    );
}

/// The control that makes `the_live_set_settles_independent_of_iteration_count`
/// mean something. That test asserts `live` figures are EQUAL, and an instrument
/// stuck at a constant -- or reporting a number unrelated to the live set -- would
/// satisfy it vacuously. Here the retained data DOES grow with the iteration count,
/// so `live` must move. If it does not, the equality next door proves nothing.
///
/// `build` is tail-recursive with the list in its accumulator, so the machine stack
/// stays flat while the retained chain crosses the threshold repeatedly. Nothing is
/// discarded, so every collection marks everything and frees nothing -- exactly the
/// shape that separates a LEVEL from a flow.
///
/// The assertion is a strict inequality, not a pinned figure: the claim is that the
/// instrument tracks the level, not that it equals any particular number.
#[test]
fn a_growing_live_set_moves_the_instrument() {
    let prog = |n: i64| {
        format!(
            "type L {{ Nil, Cons(Int, L) }}\n\
             fn build(n, acc) {{ if n == 0 {{ acc }} else {{ build(n - 1, Cons(1, acc)) }} }}\n\
             pub fn main() {{ let keep = build({n}, Nil)  \
             match keep {{ Nil -> 0  Cons(h, t) -> h }} }}\n"
        )
    };
    let mut seen: Vec<(i64, i64)> = Vec::new();
    for n in [30000i64, 70000] {
        let tag = format!("gc-grow-{n}");
        let dir = temp_dir(&tag);
        let core = lower_src(&prog(n));
        let exe = compile_and_link(&core, &dir, &tag);
        let (stdout, stats) = run_with_gc_stats(&exe, &tag);
        assert_eq!(stdout, "1", "{tag}: the retained list did not survive");
        assert!(
            stats.collections > 0,
            "{tag}: no collection happened, so `live` was never computed: {stats:?}"
        );
        seen.push((n, stats.live));
        std::fs::remove_dir_all(&dir).ok();
    }
    println!("live by retained-list length: {seen:?}");
    assert!(
        seen[1].1 > seen[0].1,
        "a live set that grows with the iteration count must move `live` -- the \
         instrument is not tracking the level: {seen:?}"
    );
}
