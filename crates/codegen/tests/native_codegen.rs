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

const PRINTING_CORPUS: &[(&str, &str, &str, &str)] = &[
    (
        "println-hello",
        "pub fn main() { io.println(\"hello\") 42 }\n",
        "hello\n",
        "42",
    ),
    (
        "println-two-lines",
        "pub fn main() { io.println(\"hello\") io.println(\"world\") 7 }\n",
        "hello\nworld\n",
        "7",
    ),
    (
        "println-empty",
        "pub fn main() { io.println(\"\") 9 }\n",
        "\n",
        "9",
    ),
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

/// §7.1's split, as ONE rule so every caller parses printing output the same
/// way: the shim prints main's Int last, so the final newline ends the value
/// line and everything before it is io.println text.
///
/// The `None` arm is load-bearing. An absent boundary means main printed
/// nothing, which is exactly what a silenced io.println looks like; panicking
/// here would report the C5-a control as a harness failure instead of as the
/// text mismatch it is.
fn split_text_and_value(stdout: &str, tag: &str) -> (String, String) {
    let without_result_newline = stdout
        .strip_suffix('\n')
        .unwrap_or_else(|| panic!("{tag}: stdout has no result newline: {stdout:?}"));
    match without_result_newline.rsplit_once('\n') {
        Some((text, value)) => (format!("{text}\n"), value.to_string()),
        None => (String::new(), without_result_newline.to_string()),
    }
}

fn native_text_value(exe: &Path, tag: &str) -> (String, String) {
    let out = Command::new(exe).output().expect("run produced binary");
    diagnose_stack_overflow(&out.status, tag);
    assert!(
        out.status.success(),
        "{tag}: binary exited {:?}",
        out.status
    );
    assert!(
        out.stderr.is_empty(),
        "{tag}: stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("native stdout must be UTF-8");
    split_text_and_value(&stdout, tag)
}

#[test]
fn io_println_writes_exact_text_then_main_value() {
    let dir = temp_dir("printing-corpus");
    for (tag, src, expected_text, expected_value) in PRINTING_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        assert_eq!(
            native_text_value(&exe, tag),
            ((*expected_text).to_string(), (*expected_value).to_string()),
            "{tag}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_output_matches_the_evaluator_across_the_printing_corpus() {
    // §7.1's split is deliberately mechanical: the shim prints main's Int last,
    // while the evaluator retains io.println text in Interp's output buffer.
    // Both halves are compared so neither an empty-vs-empty text check nor a
    // right-text/wrong-value result can pass vacuously.
    let dir = temp_dir("differential-printing");
    for (tag, src, _, _) in PRINTING_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let (text, value) = native_text_value(&exe, tag);
        assert_eq!(
            text,
            eval_main_text(src),
            "{tag}: native println text diverges from the evaluator"
        );
        assert_eq!(
            value,
            eval_main_int(src),
            "{tag}: native main value diverges from the evaluator"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn io_println_preserves_string_bytes() {
    let dir = temp_dir("printing-content");
    for (tag, content) in [
        ("raw-nul", "a\0b"),
        ("eight-bytes", "abcdefgh"),
        ("utf8-whitespace", "  héλlo  "),
        ("embedded-newline", "first\nsecond"),
        ("embedded-crlf", "first\r\nsecond"),
    ] {
        let src = format!("pub fn main() {{ io.println(\"{content}\") 5 }}\n");
        let core = lower_src(&src);
        let exe = compile_and_link(&core, &dir, tag);
        assert_eq!(
            native_text_value(&exe, tag),
            (format!("{content}\n"), "5".to_string()),
            "{tag}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
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
fn string_literals_allocate_and_main_returns_forty_two() {
    // Content is not observable until native println lands. These cases smoke
    // the allocation path, including lengths that need NUL room and raw NUL.
    for (tag, literal) in [
        ("hi", "hi"),
        ("empty", ""),
        ("eight", "abcdefgh"),
        ("embedded-nul", "a\0b"),
    ] {
        let src = format!("pub fn main() {{\n  let s = \"{literal}\"\n  42\n}}\n");
        let core = lower_src(&src);
        let dir = temp_dir(&format!("string-literal-{tag}"));
        let exe = compile_and_link(&core, &dir, tag);
        assert_runs(&exe, "42");
        std::fs::remove_dir_all(&dir).ok();
    }
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

/// The evaluator's buffered program-output channel, alongside the Int
/// result used for the native shim's final line. Keeping the Int assertion here
/// makes the §7.1 split meaningful: native stdout always ends in that value line.
fn eval_main_text(src: &str) -> String {
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (interp, v) = elya::eval::run_module_value(&m).expect("evaluator must run corpus program");
    match v {
        elya::eval::Value::Int(_) => interp.output().to_string(),
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
    /// Visible words still live at the end of the LAST collection — the LEVEL,
    /// where `freed` and `words_since_gc` are flows (5b-6 §11, obligation T7).
    live: i64,
}

/// Run `exe` with statistics enabled, returning its RAW stdout and the
/// counters. Raw because §8.5 requires the stats path stay consistent with
/// §7.1's split, and trimming destroys the trailing newlines that split is
/// defined on — a check that trimmed and then re-added one would be asserting
/// its own reconstruction rather than byte preservation.
fn run_with_gc_stats_raw(exe: &Path, tag: &str) -> (String, GcStats) {
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
    (String::from_utf8_lossy(&out.stdout).to_string(), stats)
}

/// The trimmed view the non-printing GC corpora already use, as a wrapper over
/// the raw form so both share one run and one parse of the counters.
fn run_with_gc_stats(exe: &Path, tag: &str) -> (String, GcStats) {
    let (stdout, stats) = run_with_gc_stats_raw(exe, tag);
    (stdout.trim().to_string(), stats)
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

/// N6 §2 / §10.3, direction (b). A String in an ADT field is traced ONLY by
/// the descriptor mask; if `is_heap_ty` omits `Ty::Base(TyCon::Str)`, its bit is
/// clear, so collection sweeps the still-live string and a same-sized `Two`
/// allocation recycles its block. The proof is the string CONTENT, not survival.
#[test]
fn a_string_in_an_adt_field_survives_collection() {
    // `"hello"` occupies three visible words: tag, length, and bytes plus its
    // NUL. `Two(Int, Int)` has the same three-word block size, so after a
    // collection it deterministically overwrites a swept string with a zero
    // length. `b` itself remains rooted; this isolates tracing through Mk's
    // descriptor mask from shadow-stack rooting.
    let src = "type Box { Mk(String) }\n\
               type Waste { Two(Int, Int) }\n\
               fn churn(n) { if n == 0 { 0 } else { let _ = Two(0, 0)  churn(n - 1) } }\n\
               pub fn main() { let b = Mk(\"hello\")  let _ = churn(100000)  \
               let _ = match b { Mk(s) -> io.println(s) }  0 }\n";
    let dir = temp_dir("string-adt-gc-mask");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "string-adt-gc-mask");
    let (stdout, stats) = run_with_gc_stats(&exe, "string-adt-gc-mask");
    assert!(
        stats.collections > 0,
        "no collection happened, so the String field was never tested: {stats:?}"
    );
    assert_eq!(
        stdout, "hello\n0",
        "the String field did not survive collection — its descriptor mask bit \
         was clear because is_heap_ty did not recognise Ty::Base(TyCon::Str)"
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

/// 5b-6 §11, obligation T7. The steady-state live set of a program whose live data
/// does NOT grow with its iteration count is INDEPENDENT of that count. The same
/// program is run at four counts spanning an 8x range and the `live` figures are
/// asserted equal — which makes the claim without pinning a magic number, so there
/// is no constant here a future change could be tempted to nudge and no expected
/// value to edit.
///
/// FOUR points rather than two, deliberately. Two figures agreeing is weak evidence
/// that a level SETTLES: the last collection of a single pair could land at the same
/// loop phase by luck. Agreement across an 8x spread is the settling claim itself.
/// `a_growing_live_set_moves_the_instrument` is the other half — it proves this
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
        "the live set must not grow with the iteration count — it settles: {seen:?}"
    );
}

/// The control that makes `the_live_set_settles_independent_of_iteration_count`
/// mean something. That test asserts `live` figures are EQUAL, and an instrument
/// stuck at a constant — or reporting a number unrelated to the live set — would
/// satisfy it vacuously. Here the retained data DOES grow with the iteration count,
/// so `live` must move. If it does not, the equality next door proves nothing.
///
/// `build` is tail-recursive with the list in its accumulator, so the machine stack
/// stays flat while the retained chain crosses the threshold repeatedly. Nothing is
/// discarded, so every collection marks everything and frees nothing — exactly the
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
        "a live set that grows with the iteration count must move `live` — the \
         instrument is not tracking the level: {seen:?}"
    );
}

/// §8.3 / §10 item 8. `Unit` is the one base type whose word conversion is an
/// identity, and its arms in `value_to_word`/`word_to_value` were dead until
/// `io.println` gave the language any way to produce a `Unit` value. Here a
/// `Unit`-typed ADT field round-trips through BOTH arms: `Mk(u)` stores the
/// word, the match binding reads it back, and re-storing that binding stores a
/// word that was itself read back. Under the `_` fallthrough the store side
/// calls `.into_pointer_value()` on an `IntValue` and panics the compiler.
#[test]
fn a_unit_typed_adt_field_round_trips_through_the_word_helpers() {
    let src = "type Box { Mk(Unit) }\n\
               pub fn main() { let u = io.println(\"x\")  let b = Mk(u)  \
               let c = match b { Mk(v) -> Mk(v) }  match c { Mk(w) -> 5 } }\n";
    let dir = temp_dir("unit-adt-field");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "unit-adt-field");
    let (text, value) = native_text_value(&exe, "unit-adt-field");
    assert_eq!(
        text, "x\n",
        "unit-adt-field: the println that produced the Unit lost its text"
    );
    assert_eq!(
        value, "5",
        "unit-adt-field: the Unit field did not survive the store/read round trip"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// §8.3 / §10 item 8, the capture side. `u` is `Unit`, so the capture-store arm
/// converts an i64 zero without a `ptrtoint` and the capture-read arm returns it
/// without an `inttoptr`. Both sites funnel through the same helper pair, so this
/// and the field test together cover all four §8.3 conversion sites.
///
/// The lambda is inline in `main` rather than returned from a helper, and `z` is
/// pinned to `Int` on purpose. A helper like `fn wrap(u) { fn(z) { u } }` is
/// polymorphic, and `repr_ty` refuses `Ty::Var(_)` for the WHOLE module — see
/// `a_polymorphic_function_refuses_the_whole_module`. That shape dies before the
/// `Unit` arm is ever reached, so it would prove nothing about it: N7 knocking,
/// not N6 failing.
#[test]
fn a_unit_typed_closure_capture_round_trips_through_the_word_helpers() {
    let src = "pub fn main() { let u = io.println(\"x\")  \
               let f = fn(z) { let _ = z + 1  u }  \
               let _ = f(1)  7 }\n";
    let dir = temp_dir("unit-capture");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "unit-capture");
    let (text, value) = native_text_value(&exe, "unit-capture");
    assert_eq!(
        text, "x\n",
        "unit-capture: the println that produced the Unit lost its text"
    );
    assert_eq!(
        value, "7",
        "unit-capture: the captured Unit word did not read back intact"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// §8.5. The stats path also returns stdout, so it must parse printing output by
/// §7.1's rule and not a second one of its own. It shares that rule literally:
/// the raw run feeds `split_text_and_value`, the same function the differential
/// uses, so the two cannot drift. Raw rather than trimmed because trimming
/// destroys the trailing newline the split is defined on, and a check that
/// trimmed and then re-added one would assert its own reconstruction.
///
/// Allocations stay trivial so the run never reaches `GC_THRESHOLD_WORDS`
/// (§7.3: never lower it) — asserted, not assumed.
#[test]
fn a_printing_program_under_gc_stats_splits_by_the_same_rule() {
    let (tag, src, want_text, want_value) = PRINTING_CORPUS[0];
    let dir = temp_dir("stats-printing-split");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, tag);
    let (stdout, stats) = run_with_gc_stats_raw(&exe, tag);
    let (text, value) = split_text_and_value(&stdout, tag);
    assert_eq!(
        text, want_text,
        "{tag}: the stats path parsed different println text than §7.1's split"
    );
    assert_eq!(
        value, want_value,
        "{tag}: the stats path parsed a different value line than §7.1's split"
    );
    assert_eq!(
        stats.collections, 0,
        "{tag}: the consistency gate tripped the collector, so it is no longer \
         the trivial-allocation check §7.3 requires: {stats:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A3 (spec §11), through codegen as the plan's `Ok` branch requires. Task 3
/// measured that nothing in the front end refuses a polymorphic effect; this is
/// the same declaration carried the rest of the way. Prediction, written before
/// the run: Core carries no effect declarations, so codegen never sees one, and
/// the program compiles, links, runs and prints main's `0`. Measured: exactly
/// that. Not a miscompile and not a refusal — the effect is invisible here.
///
/// The `Ty::Var` half of the answer (an unconstrained type parameter) is a
/// refusal and produces no binary, so it lives beside its sibling in `lib.rs`'s
/// `mod tests`: `a3_an_unconstrained_polymorphic_effect_meets_the_ty_var_refusal`.
#[test]
fn a3_a_polymorphic_effect_declaration_compiles_and_runs_natively() {
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               pub fn main() -> Int { 0 }\n";
    let dir = temp_dir("a3-poly-decl");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "a3-poly-decl");
    assert_runs(&exe, "0");
    std::fs::remove_dir_all(&dir).ok();
}

// ---- 5b-8 Task 7b-3: the effectful calling convention, frames, handle entry,
// the return clause and the handler frame (D10, D14, D16-D18). No perform
// happens at run time in this corpus -- dispatch is Task 8 -- but every body
// calls an effectful function, so the CPS convention, per-site frames and
// the handler frame all execute.

/// `compile_and_link`, reporting a refusal instead of panicking, so a corpus
/// loop can collect every case's outcome.
fn try_compile_and_link(
    core: &elya::core::CoreModule,
    dir: &Path,
    tag: &str,
) -> Result<PathBuf, String> {
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(core, &obj).map_err(|e| format!("compile: {e:?}"))?;
    elya_codegen::link(&obj, &exe).map_err(|e| format!("link: {e:?}"))?;
    Ok(exe)
}

const S_W: &str = "effect S { fn get() -> Int }\n\
                   fn w(b) { if b { get() } else { 2 } }\n";

/// (tag, program after S_W, expected). Expected values were measured with the
/// evaluator before any emitter code was written (7b-3 predictions).
const HANDLE_7B3: &[(&str, &str, &str)] = &[
    (
        "tail-cps-in-body",
        "pub fn main() -> Int {\n\
         \x20 handle { w(False) } with {\n\
         \x20   S.get() -> resume(1)\n\
         \x20   return(r) -> r * 10\n\
         \x20 }\n\
         }\n",
        "20",
    ),
    (
        // A non-tail effectful call in `u` (a site saving `a` and `x`) and one
        // in the handled body (a site saving `y`).
        "site-saves-local-and-temp",
        "fn u(x) {\n\
         \x20 let a = x * 3\n\
         \x20 a + w(False) + x\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 let y = 7\n\
         \x20 handle { y + u(5) } with {\n\
         \x20   S.get() -> resume(1)\n\
         \x20   return(r) -> r * 10\n\
         \x20 }\n\
         }\n",
        "290",
    ),
    (
        "return-reads-local",
        "pub fn main() -> Int {\n\
         \x20 let m = 3\n\
         \x20 handle { w(False) } with {\n\
         \x20   S.get() -> resume(1)\n\
         \x20   return(r) -> r * m\n\
         \x20 }\n\
         }\n",
        "6",
    ),
    (
        "no-return-clause",
        "pub fn main() -> Int {\n\
         \x20 handle { w(False) + 1 } with {\n\
         \x20   S.get() -> resume(1)\n\
         \x20 }\n\
         }\n",
        "3",
    ),
    (
        "sequential-handles",
        "pub fn main() -> Int {\n\
         \x20 let x = handle { w(False) } with { S.get() -> resume(1)  return(v) -> v }\n\
         \x20 let y = handle { w(False) + 40 } with { S.get() -> resume(1)  return(v) -> v * 2 }\n\
         \x20 x + y\n\
         }\n",
        "86",
    ),
    (
        // The resumption reads an already-evaluated operand (`x * 3`) back
        // out of the frame as a temporary, not by recomputing it.
        "site-saves-a-temporary",
        "fn v(x) { (x * 3) + w(False) }\n\
         pub fn main() -> Int {\n\
         \x20 handle { v(5) } with {\n\
         \x20   S.get() -> resume(1)\n\
         \x20   return(r) -> r\n\
         \x20 }\n\
         }\n",
        "17",
    ),
    (
        // The hole is an `if` condition: the resumption branches.
        "site-in-an-if-condition",
        "pub fn main() -> Int {\n\
         \x20 handle { if w(False) == 2 { 10 } else { 20 } } with {\n\
         \x20   S.get() -> resume(1)\n\
         \x20   return(r) -> r + 1\n\
         \x20 }\n\
         }\n",
        "11",
    ),
    (
        // Two sites in one operand list: the second site's frame saves the
        // first site's value, which the resumption holds as `cur` (found by
        // the 7b-3 review: it was refused as "saved temporary is not
        // available").
        "two-sites-in-one-expression",
        "pub fn main() -> Int {\n\
         \x20 handle { w(False) + w(False) * w(False) } with {\n\
         \x20   S.get() -> resume(1)\n\
         \x20   return(r) -> r\n\
         \x20 }\n\
         }\n",
        "6",
    ),
    (
        "two-sites-in-one-call",
        "fn u(a, b) { a * 100 + b }\n\
         pub fn main() -> Int {\n\
         \x20 handle { u(w(False), w(False) + 1) } with {\n\
         \x20   S.get() -> resume(1)\n\
         \x20   return(r) -> r\n\
         \x20 }\n\
         }\n",
        "203",
    ),
    (
        // D16's regression pin: an effect-polymorphic function used at a pure
        // lambda stays direct and keeps working.
        "apply-pure-stays-direct",
        "fn apply(f) { f(1) + 1 }\n\
         pub fn main() -> Int { apply(fn(x) { x * 10 }) }\n",
        "11",
    ),
];

const DEEP_FRAMES: &str = "type L { Nil, Cons(Int, L) }\n\
     effect S { fn get() -> Int }\n\
     fn w(b) { if b { get() } else { 2 } }\n\
     fn head(l) { match l { Nil -> 0  Cons(h, _) -> h } }\n\
     fn deep(n, b) {\n\
     \x20 if n == 0 { w(b) } else {\n\
     \x20   let junk = Cons(1, Cons(2, Cons(3, Cons(4, Nil))))\n\
     \x20   let c = Cons(n, Nil)\n\
     \x20   let r = deep(n - 1, b)\n\
     \x20   r + head(c)\n\
     \x20 }\n\
     }\n\
     pub fn main() -> Int {\n\
     \x20 handle { deep(4000, False) } with {\n\
     \x20   S.get() -> resume(1)\n\
     \x20   return(r) -> r\n\
     \x20 }\n\
     }\n";

#[test]
fn the_7b3_handle_corpus_compiles_runs_and_prints_the_expected_answer() {
    // Every case runs and is reported before the assertion, so a negative
    // control shows exactly WHICH cases it breaks (one assertion path per case).
    let dir = temp_dir("handle-7b3");
    let mut failures = Vec::new();
    for (tag, prog, expected) in HANDLE_7B3 {
        let src = format!("{S_W}{prog}");
        let core = lower_src(&src);
        let exe = match try_compile_and_link(&core, &dir, tag) {
            Ok(exe) => exe,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let out = Command::new(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() || got != *expected {
            failures.push(format!(
                "{tag}: {:?} stdout={got:?} want={expected}",
                out.status
            ));
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn native_output_matches_the_evaluator_across_the_7b3_handle_corpus() {
    let dir = temp_dir("differential-handle-7b3");
    let mut failures = Vec::new();
    for (tag, prog, _) in HANDLE_7B3 {
        let src = format!("{S_W}{prog}");
        let core = lower_src(&src);
        let exe = match try_compile_and_link(&core, &dir, tag) {
            Ok(exe) => exe,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let out = Command::new(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let want = eval_main_int(&src);
        if !out.status.success() || got != want {
            failures.push(format!(
                "{tag}: {:?} native={got:?} evaluator={want}",
                out.status
            ));
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(
        failures.is_empty(),
        "native diverges from the evaluator: {failures:#?}"
    );
}

#[test]
fn frames_holding_heap_values_survive_collections_and_match_the_evaluator() {
    // D10's rows under load: 4000 non-tail effectful calls deep, each frame
    // saving a heap `Cons` across the call while garbage forces collections.
    // A frame row that failed to trace its saved value would let the
    // collector reuse it, and `head(c)` would read garbage.
    let dir = temp_dir("deep-frames-7b3");
    let core = lower_src(DEEP_FRAMES);
    let exe = compile_and_link(&core, &dir, "deep");
    let (stdout, stats) = run_with_gc_stats(&exe, "deep");
    assert!(stats.collections >= 1, "no collection ran: {stats:?}");
    assert_eq!(
        stdout,
        eval_main_int(DEEP_FRAMES),
        "native diverges from the evaluator"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_perform_reached_at_run_time_traps_with_a_named_error() {
    // D18: until Task 8, a perform compiles to a named trap -- a loud stop,
    // never a wrong answer.
    let dir = temp_dir("perform-trap-7b3");
    let src = format!(
        "{S_W}pub fn main() -> Int {{\n  handle {{ w(True) }} with {{\n    \
         S.get() -> resume(1)\n    return(r) -> r\n  }}\n}}\n"
    );
    let core = lower_src(&src);
    let exe = compile_and_link(&core, &dir, "trap");
    let out = Command::new(&exe).output().expect("run produced binary");
    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("elya: perform: native effect dispatch is not implemented yet"),
        "stderr should name the trap, got: {stderr}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Programs from the 7b-3 independent review whose heap values were live
/// across allocations in CPS code without a root. Each runs under collection
/// pressure and is compared to the evaluator, text and value.
const CPS_ROOTING: &[(&str, &str)] = &[
    (
        // D-1: the value a site returned, held by the resumption while a
        // later operand (`garbage(100)`) allocates.
        "hole-value-across-a-later-operand",
        "type L { Nil, Cons(Int, L) }\n\
         effect S { fn get() -> Int }\n\
         fn w(b) { if b { get() } else { 2 } }\n\
         fn mkl(n, b) { let x = w(b)  Cons(n + x, Nil) }\n\
         fn garbage(n) { if n == 0 { Cons(7777, Nil) } else { let j = Cons(n, Cons(n, Nil))  garbage(n - 1) } }\n\
         fn head(l) { match l { Nil -> 0  Cons(h, _) -> h } }\n\
         fn pair(a, z) { head(a) + head(z) - 7777 }\n\
         fn lp(i, acc, b) { if i == 0 { acc } else { lp(i - 1, acc + pair(mkl(i, b), garbage(100)), b) } }\n\
         pub fn main() -> Int { handle { lp(2000, 0, False) } with { S.get() -> resume(1)  return(r) -> r } }\n",
    ),
    (
        // D-3: a string literal operand of a resumption, live while a later
        // operand allocates.
        "string-literal-in-a-resumption",
        "type L { Nil, Cons(Int, L) }\n\
         effect S { fn get() -> Int }\n\
         fn w(b) { if b { get() } else { 2 } }\n\
         fn head(l) { match l { Nil -> 0  Cons(h, _) -> h } }\n\
         fn garbage(n) { if n == 0 { Cons(7777, Nil) } else { let j = Cons(n, Cons(n, Nil))  garbage(n - 1) } }\n\
         fn pr(n, s, l) { let z = io.println(s)  n + head(l) }\n\
         pub fn main() -> Int {\n\
         \x20 handle { pr(w(False), \"hello\", garbage(12000)) } with { S.get() -> resume(1)  return(r) -> r }\n\
         }\n",
    ),
    (
        // A heap binding SHADOWED inside an effectful region, live while pure
        // code allocates -- before and after the resumption (found by the
        // review of the shadowing fix; the direct-emitter half is pinned by
        // `a_shadowed_heap_binding_stays_rooted_and_in_scope`).
        "shadowed-binding-in-an-effectful-region",
        "type L { Nil, Cons(Int, L) }\n\
         fn churn(n, acc) { if n == 0 { acc } else { let g = Cons(n, Cons(n, Cons(n, Nil)))  churn(n - 1, acc + 1) } }\n\
         fn head(l) { match l { Nil -> 0  Cons(h, _) -> h } }\n\
         effect S { fn get() -> Int }\n\
         fn w(b) { if b { get() } else { 2 } }\n\
         fn u(x) {\n\
         \x20 let s = Cons(42, Nil)\n\
         \x20 let r = { let s = Cons(7, Nil)  let z0 = churn(30000, 0)  let q = w(False)  let z = churn(30000, 0) + z0 - 30000  head(s) + z + q }\n\
         \x20 r + head(s) + x\n\
         }\n\
         pub fn main() -> Int { handle { u(1) } with { S.get() -> resume(3)  return(r) -> r } }\n",
    ),
    (
        // D-3: a string literal LEFT of the site is re-lowered by the
        // resumption, and must be rooted while a later operand allocates.
        "string-literal-left-of-a-site",
        "type L { Nil, Cons(Int, L) }\n\
         effect S { fn get() -> Int }\n\
         fn w(b) { if b { get() } else { 2 } }\n\
         fn head(l) { match l { Nil -> 0  Cons(h, _) -> h } }\n\
         fn garbage(n) { if n == 0 { Cons(7777, Nil) } else { let j = Cons(n, Cons(n, Nil))  garbage(n - 1) } }\n\
         fn pr(s, n, l) { let z = io.println(s)  n + head(l) }\n\
         pub fn main() -> Int {\n\
         \x20 handle { pr(\"hello\", w(False), garbage(12000)) } with { S.get() -> resume(1)  return(r) -> r }\n\
         }\n",
    ),
    (
        // D-3: the same, in forward CPS code on a path that takes no site.
        "string-literal-before-an-untaken-site",
        "type L { Nil, Cons(Int, L) }\n\
         effect S { fn get() -> Int }\n\
         fn w(b) { if b { get() } else { 2 } }\n\
         fn head(l) { match l { Nil -> 0  Cons(h, _) -> h } }\n\
         fn garbage(n) { if n == 0 { Cons(7777, Nil) } else { let j = Cons(n, Cons(n, Nil))  garbage(n - 1) } }\n\
         fn pr(s, l, n) { let z = io.println(s)  n + head(l) }\n\
         fn go(c) { pr(\"hello\", garbage(12000), if c { w(False) } else { 1 }) }\n\
         pub fn main() -> Int {\n\
         \x20 handle { go(False) } with { S.get() -> resume(1)  return(r) -> r }\n\
         }\n",
    ),
];

#[test]
fn heap_values_live_across_cps_operands_survive_collections() {
    let dir = temp_dir("cps-rooting-7b3");
    let mut failures = Vec::new();
    for (tag, src) in CPS_ROOTING {
        let core = lower_src(src);
        let exe = match try_compile_and_link(&core, &dir, tag) {
            Ok(exe) => exe,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let out = Command::new(&exe)
            .env("ELY_GC_STATS", "1")
            .output()
            .expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if !out.status.success() {
            failures.push(format!("{tag}: {:?} stderr={stderr:?}", out.status));
            continue;
        }
        if !stderr
            .lines()
            .any(|l| l.starts_with("elya-gc:") && !l.contains("collections=0"))
        {
            failures.push(format!("{tag}: no collection ran: {stderr:?}"));
        }
        let (text, value) = split_text_and_value(&stdout, tag);
        let (want_text, want_value) = (eval_main_text(src), eval_main_int(src));
        if text != want_text || value != want_value {
            failures.push(format!(
                "{tag}: native text={text:?} value={value} evaluator text={want_text:?} value={want_value}"
            ));
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A heap binding SHADOWED by an inner binding of the same name stays live and
/// must stay rooted while the inner scope allocates (found 2026-10-03 while
/// reading the emitter for 5b-8 7b-3; pre-existing since the collector landed).
const SHADOW_PRELUDE: &str = "type L { Nil, Cons(Int, L) }\n\
     fn churn(n, acc) { if n == 0 { acc } else { let g = Cons(n, Cons(n, Cons(n, Nil)))  churn(n - 1, acc + 1) } }\n\
     fn head(l) { match l { Nil -> 0  Cons(h, _) -> h } }\n";

const SHADOWING: &[(&str, &str)] = &[
    (
        "let-in-value-position",
        "pub fn main() -> Int {\n\
         \x20 let s = Cons(42, Nil)\n\
         \x20 let r = { let s = Cons(7, Nil)  let z = churn(30000, 0)  head(s) + z }\n\
         \x20 r + head(s)\n\
         }\n",
    ),
    (
        "let-in-tail-position",
        "fn f(s) { let r = { let s = Cons(7, Nil)  let z = churn(30000, 0)  head(s) + z }  r + head(s) }\n\
         pub fn main() -> Int { f(Cons(42, Nil)) }\n",
    ),
    (
        // The arm used to REMOVE the outer binding after the arm instead of
        // restoring it ("unbound var" at compile time).
        "match-arm-binder",
        "pub fn main() -> Int {\n\
         \x20 let s = Cons(42, Nil)\n\
         \x20 let r = match Cons(7, Cons(8, Nil)) { Nil -> 0  Cons(h, s) -> head(s) + churn(30000, 0) }\n\
         \x20 r + head(s)\n\
         }\n",
    ),
];

#[test]
fn a_shadowed_heap_binding_stays_rooted_and_in_scope() {
    let dir = temp_dir("shadowing");
    let mut failures = Vec::new();
    for (tag, prog) in SHADOWING {
        let src = format!("{SHADOW_PRELUDE}{prog}");
        let core = lower_src(&src);
        let exe = match try_compile_and_link(&core, &dir, tag) {
            Ok(exe) => exe,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let (stdout, stats) = run_with_gc_stats(&exe, tag);
        if stats.collections < 1 {
            failures.push(format!("{tag}: no collection ran"));
        }
        let want = eval_main_int(&src);
        if stdout != want {
            failures.push(format!("{tag}: native={stdout} evaluator={want}"));
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{failures:#?}");
}
