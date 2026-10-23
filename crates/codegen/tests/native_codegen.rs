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

/// Runs a produced binary on the 8 MiB stack every test here was written
/// against (the Linux main thread's default before the program got its own
/// stack; spec 2026-10-16-elya-native-stack D3). A tail call that stops being
/// eliminated therefore still overflows at a million steps -- now by name. Only
/// the `DEEP` tests run on the default stack, through `Command::new`.
fn native(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    c.env("ELYA_STACK_MIB", "8");
    c
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

    let out = native(&exe).output().expect("run produced binary");
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

/// Since the program got its own stack (spec 2026-10-16-elya-native-stack), an
/// overflow of it is exit status 1 and the line `elya: native stack overflow:
/// ...` on stderr, which every assertion here prints; these two status checks
/// still name an overflow that happens anywhere else (the runtime's own frames
/// on the main thread).
///
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

/// The Linux half of the stack-overflow diagnosis (plan Task 11's platform
/// note): there an overflow is SIGSEGV, not STATUS_STACK_OVERFLOW. SIGSEGV is
/// NOT unique to an overflow -- a collector or codegen fault raises it too --
/// so this names the LIKELY cause. Used by the million-step tests only: the
/// corpus loops keep collecting per-case failures instead of panicking.
fn diagnose_crash(status: &std::process::ExitStatus, tag: &str) {
    diagnose_stack_overflow(status, tag);
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if status.signal() == Some(11) {
            panic!(
                "{tag}: SIGSEGV. Most likely the machine stack overflowed -- a tail call or \
                 resume that was supposed to keep the stack flat grew it instead -- but a \
                 collector or codegen fault raises the same signal."
            );
        }
    }
}

/// The three required assertions per case (§5): exit status, stdout, empty stderr.
fn assert_runs(exe: &Path, expected: &str) {
    let out = native(exe).output().expect("run produced binary");
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
    let out = native(exe).output().expect("run produced binary");
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
        let out = native(&exe).output().expect("run produced binary");
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
        let out = native(&exe).output().expect("run produced binary");
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
        let out = native(&exe).output().expect("run produced binary");
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
        let out = native(&exe).output().expect("run produced binary");
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
        let out = native(&exe).output().expect("run produced binary");
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
        generic_types: vec![],
        effect_params_used: Default::default(),
        effects: Default::default(),
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
            row: elya::types::EffectRow::pure(),
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
    let out = native(&exe).output().expect("run produced binary");
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
    /// Collections that ran inside a multi-shot copy (`elya_cont_copy`), when
    /// the runtime reports them (spec 2026-10-22-elya-multi-shot-sharing, the
    /// review's note 1).
    mid_copy: Option<i64>,
}

/// Run `exe` with statistics enabled, returning its RAW stdout and the
/// counters. Raw because §8.5 requires the stats path stay consistent with
/// §7.1's split, and trimming destroys the trailing newlines that split is
/// defined on — a check that trimmed and then re-added one would be asserting
/// its own reconstruction rather than byte preservation.
fn run_with_gc_stats_raw(exe: &Path, tag: &str) -> (String, GcStats) {
    let out = native(exe)
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
        mid_copy: line
            .split_whitespace()
            .find_map(|f| f.strip_prefix("mid_copy="))
            .map(|v| {
                v.parse()
                    .unwrap_or_else(|e| panic!("{tag}: `mid_copy=` is not a number: {e}"))
            }),
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
/// polymorphic, and before N7 part 2 `repr_ty` refused `Ty::Var(_)` for the
/// WHOLE module. That shape died before the `Unit` arm was ever reached, so it
/// would have proved nothing about it: N7 knocking, not N6 failing.
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
/// The `Ty::Var` half of the answer (an unconstrained type parameter) was a
/// refusal until N7 part 2 (2026-10-15); it lives beside its sibling in
/// `lib.rs`'s `mod tests`: `a3_an_unconstrained_polymorphic_effect_is_not_emitted`.
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
        let out = native(&exe).output().expect("run produced binary");
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
        let out = native(&exe).output().expect("run produced binary");
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

/// Task 12 controls 2a and 2b both fail this test, the same way (exit 1), as
/// they do the 400,000-deep constant-time-handler test (SIGSEGV): a deep chain
/// under collection is exposed by losing either the frames' rows or their
/// `next` bit. The pair is told apart by
/// `a_continuation_held_by_a_lambda_survives_a_collection`.
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

/// Programs from the 7b-3 independent review whose heap values were live
/// across allocations in CPS code without a root. Each runs under collection
/// pressure and is compared to the evaluator, text and value.
const CPS_ROOTING: &[(&str, &str)] = &[
    (
        // 5b-9b review: collections run INSIDE the lambda body, between a site
        // and later uses of the capture `l` and the local `m`. 45015 + 40000 +
        // 45016 = 130031.
        "lambda-body-collects-between-sites",
        "effect Ask { fn ask() -> Int }\n\
         type L { Nil, Cons(Int, L) }\n\
         fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
         fn len(l, a) { match l { Nil -> a  Cons(_, t) -> len(t, a + 1) } }\n\
         fn user() -> Int { let l = build(10000, Nil)  let f = fn(x) { let a = ask()  let junk = len(build(30000, Nil), 0)  let m = build(5000, Nil)  let b = ask()  x + a + b + len(l, 0) + len(m, 0) + junk }  f(1) + len(build(40000, Nil), 0) + f(2) }\n\
         fn prog() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n\
         pub fn main() -> Int { prog() }\n",
    ),
    (
        // 5b-9b: an effectful closure captures a heap list; it must survive a
        // 30,000-cell build between the two calls. (1 + 7 + 20000) + 30000 +
        // (2 + 7 + 20000) = 70017.
        "lambda-captures-survive-collection",
        "type L { Nil, Cons(Int, L) }\n\
         effect Ask { fn ask() -> Int }\n\
         fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
         fn lenacc(l, a) { match l { Nil -> a  Cons(_, t) -> lenacc(t, a + 1) } }\n\
         fn user() -> Int { let l = build(20000, Nil)  let f = fn(x) { x + ask() + lenacc(l, 0) }  f(1) + lenacc(build(30000, Nil), 0) + f(2) }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(r) -> r } }\n",
    ),
    (
        // 5b-9a: pattern binders live across a site inside a match arm, with
        // garbage churned every level so collections run mid-recursion. `t` is
        // a heap binder the frame must save AND trace. 8,030,000 predicted.
        "match-binders-survive-collection",
        "type L { Nil, Cons(Int, L) }\n\
         effect Ask { fn ask() -> Int }\n\
         fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
         fn sum(l) { match l { Nil -> 0  Cons(h, t) -> { let g = Cons(h, Cons(h, Cons(h, Nil)))  h + ask() + sum(t) } } }\n\
         pub fn main() -> Int { handle { sum(build(4000, Nil)) } with { Ask.ask() -> resume(7)  return(r) -> r } }\n",
    ),
    (
        // 5b-9a review: a VARIABLE-pattern heap binder (`o`, the whole
        // scrutinee) live across a site while a 30,000-cell list is built and
        // collected: 7 + 30000 + 50 = 30057.
        "match-var-binder-survives-collection",
        "type L { Nil, Cons(Int, L) }\n\
         effect Ask { fn ask() -> Int }\n\
         fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
         fn lenacc(l, a) { match l { Nil -> a  Cons(_, t) -> lenacc(t, a + 1) } }\n\
         fn user() -> Int { match build(50, Nil) { o -> ask() + lenacc(build(30000, Nil), 0) + lenacc(o, 0) } }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(r) -> r } }\n",
    ),
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
    (
        // 5b-10: a CPS handler frame saves the heap list `l`; a CPS clause holds
        // `m` and `l` across a site (`t()`) and a 30,000-cell build, and the
        // CPS return clause reads `l` again: 30007 + 20000 + 20000 + 7 + 30000 =
        // 100014.
        "cps-handler-frame-and-clause-survive-collection",
        "type L { Nil, Cons(Int, L) }
         effect S { fn get() -> Int }
         effect T { fn t() -> Int }
         fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }
         fn lenacc(l, a) { match l { Nil -> a  Cons(_, t) -> lenacc(t, a + 1) } }
         fn user() -> Int {
           let l = build(20000, Nil)
           handle { get() + lenacc(l, 0) } with {
             S.get() -> { let m = build(10000, Nil)  let x = t()  let junk = lenacc(build(30000, Nil), 0)  resume(x + lenacc(m, 0) + lenacc(l, 0)) + junk }
             return(r) -> r + lenacc(l, 0) + t()
           }
         }
         pub fn main() -> Int { handle { user() } with { T.t() -> resume(7)  return(r) -> r } }
",
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
        let out = native(&exe)
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

// ---- 5b-8 Task 8: handler dispatch, the continuation, resume (D10-D18) ----

/// (tag, program, expected). Expected values were worked out by hand before
/// the dispatch existed; the differential test holds them to the evaluator.
/// `deep-reinstall` (A6) is calibrated: 6 if the handler is re-found on every
/// perform, 2 if it is found once and lost. `frame-capture` is the 7b test
/// D15 moved here. `perform-reached-at-run-time` was D18's trap until now.
///
/// Task 12 control 1a (resume no longer re-installs the continuation's
/// handler) did NOT move `deep-reinstall`: it still printed 6. With one live
/// handler under D17, every resume in it runs while that handler is still
/// current, so in this design A6 cannot witness re-installation and the
/// plan's "prints 2" has no native counterpart. The witness is a resume that
/// runs AFTER its handle returned. Exactly those failed, all with exit 1:
/// `state-passing-lambda-resumes-later` here (its stderr, the one captured:
/// "the handler has no clause for this operation"), A9, and the
/// lambda-held-continuation test. Nothing else in the binary failed.
const HANDLER_8: &[(&str, &str, &str)] = &[
    (
        "deep-reinstall",
        "effect State { fn get() -> Int }\n\
         fn loop_body(n) { if n == 0 { 0 } else { get() + loop_body(n - 1) } }\n\
         pub fn main() -> Int {\n\
         \x20 handle { loop_body(3) } with {\n\
         \x20   State.get() -> resume(2)\n\
         \x20   return(x) -> x\n\
         \x20 }\n\
         }\n",
        "6",
    ),
    (
        // Slice 5c-1: an UNQUALIFIED clause means its op's effect; the native
        // back end gets it from Core, which always names the effect.
        "unqualified-clause",
        "effect Ask { fn ask() -> Int }\n\
         fn one() { ask() }\n\
         pub fn main() -> Int {\n\
         \x20 handle { one() } with {\n\
         \x20   ask() -> resume(2)\n\
         \x20   return(x) -> x\n\
         \x20 }\n\
         }\n",
        "2",
    ),
    (
        "frame-capture",
        "effect State { fn get() -> Int }\n\
         fn body() -> Int { get() + 1 }\n\
         pub fn main() -> Int {\n\
         \x20 handle { body() } with {\n\
         \x20   State.get() -> resume(41)\n\
         \x20   return(x) -> x\n\
         \x20 }\n\
         }\n",
        "42",
    ),
    (
        "two-ops-one-handler",
        "effect Ask { fn a() -> Int  fn b() -> Int }\n\
         fn body() -> Int { a() + b() }\n\
         pub fn main() -> Int { handle { body() } with { Ask.a() -> resume(10)  Ask.b() -> resume(20)  return(r) -> r } }\n",
        "30",
    ),
    (
        "op-with-an-argument",
        "effect Log { fn log(n: Int) -> Int }\n\
         fn body() -> Int { log(5) + log(6) }\n\
         pub fn main() -> Int { handle { body() } with { Log.log(n) -> resume(n * 2)  return(r) -> r } }\n",
        "22",
    ),
    (
        "non-tail-resume-sees-the-return-clause",
        "effect S { fn get() -> Int }\n\
         fn body() -> Int { get() + 1 }\n\
         pub fn main() -> Int { handle { body() } with { S.get() -> resume(1) + 100  return(r) -> r * 2 } }\n",
        "104",
    ),
    (
        "clause-that-does-not-resume",
        "effect S { fn get() -> Int }\n\
         fn body() -> Int { get() + 1 }\n\
         pub fn main() -> Int { handle { body() } with { S.get() -> 99  return(r) -> r * 2 } }\n",
        "99",
    ),
    (
        "state-passing-lambda-resumes-later",
        "effect St { fn get() -> Int }\n\
         fn prog(n) { if n == 0 { 0 } else { get() + prog(n - 1) } }\n\
         pub fn main() -> Int {\n\
         \x20 let f = handle { prog(10) } with {\n\
         \x20   St.get() -> fn(s) { (resume(s))(s + 1) }\n\
         \x20   return(x) -> fn(s) { x }\n\
         \x20 }\n\
         \x20 f(1)\n\
         }\n",
        "55",
    ),
    (
        "a3-polymorphic-effect-at-int",
        // A polymorphic effect performed at Int inside a handler (spec 11
        // A3, plan Task 8 Step 11a). No `set`: `resume(Unit)` would meet
        // codegen's existing refusal of a Unit literal, not dispatch.
        "effect State(s) { fn get() -> s }\n\
         fn w() -> Int { let x = get()  x * 10 }\n\
         pub fn main() -> Int { handle { w() } with { State.get() -> resume(4)  return(r) -> r } }\n",
        "40",
    ),
    (
        "sequential-handles-perform",
        "effect S { fn get() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 let x = handle { get() } with { S.get() -> resume(1)  return(v) -> v }\n\
         \x20 let y = handle { get() + 40 } with { S.get() -> resume(2)  return(v) -> v }\n\
         \x20 x + y\n\
         }\n",
        "43",
    ),
    (
        // A pure function called from a handled body runs its OWN handle;
        // when it returns, the outer handler must be current again for the
        // body's next perform.
        "dynamically-nested-handle",
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         fn g() -> Int { handle { t() + 1 } with { T.t() -> resume(100)  return(r) -> r } }\n\
         fn body() -> Int { let a = g()  a + get() }\n\
         pub fn main() -> Int { handle { body() } with { S.get() -> resume(5)  return(r) -> r } }\n",
        "106",
    ),
    (
        "perform-reached-at-run-time",
        "effect S { fn get() -> Int }\n\
         fn w(b) { if b { get() } else { 2 } }\n\
         pub fn main() -> Int {\n\
         \x20 handle { w(True) } with {\n\
         \x20   S.get() -> resume(1)\n\
         \x20   return(r) -> r\n\
         \x20 }\n\
         }\n",
        "1",
    ),
];

/// Slice 5b-9a: effectful code inside a `match`, natively. Every row was
/// refused "effectful call inside a match (not yet compiled natively)" before;
/// expected values were predicted before the first run.
///
/// Negative controls, each reverted, each failing differently:
/// - binding order reversed: COMPILE error "saved binding is not available"
///   on the binder rows. Predicted a wrong value, 1018 (`x * 100` is computed
///   before the site; only `y` would reload from `x`'s slot -- a full swap
///   would be 2018). The frame save cross-checks names against binding
///   indices, which is stronger;
/// - the resumption skips dispatch on a scrutinee site: the compiler panics
///   (a pointer reaches integer arithmetic);
/// - pattern binders bypass `St`: "saved binding is not available" again, on
///   the binder row and the GC row (`CPS_ROOTING`).
const MATCH_EFFECTS: &[(&str, &str, &str)] = &[
    (
        // a tail match whose arm performs.
        "match-tail-effect-arm",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         fn user(t) -> Int { match t { A -> ask()  B -> 0 } }\n\
         pub fn main() -> Int { handle { user(A) } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "7",
    ),
    (
        // the arm's site is non-tail: the resumption carries the arm's value up through the match (slot >= 1).
        "match-value-effect-arm",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         fn user(t) -> Int { let v = match t { A -> ask()  B -> 0 }  v + 1 }\n\
         pub fn main() -> Int { handle { user(A) } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "8",
    ),
    (
        // the pattern binders `x` and `y` are live across the site, so its frame saves them; asymmetric so a swapped binding order shows (1028 vs 2018).
        "match-binders-across-site",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         type P { P(Int, Int) }\n\
         fn user() -> Int { let r = match P(10, 20) { P(x, y) -> x * 100 + ask() + y }  r + 1 }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "1028",
    ),
    (
        // the site is the SCRUTINEE (slot 0): the resumption dispatches on the returned value.
        "match-effectful-scrutinee",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         fn choose() -> T { if ask() == 7 { B } else { A } }\n\
         fn user() -> Int { let r = match choose() { A -> 1  B -> 2 }  r * 10 }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "20",
    ),
    (
        // slot 0 again, with the match in tail position.
        "match-tail-effectful-scrutinee",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         fn choose() -> T { if ask() == 7 { B } else { A } }\n\
         fn user() -> Int { match choose() { A -> 1  B -> ask() } }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "7",
    ),
    (
        // variable and wildcard arms (5 + 8).
        "match-var-and-wild-arms",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         fn user(t) -> Int {\n\
         \x20 let a = match t { A -> ask()  other -> 5 }\n\
         \x20 let b = match t { A -> 0  _ -> ask() + 1 }\n\
         \x20 a + b\n\
         }\n\
         pub fn main() -> Int { handle { user(B) } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "13",
    ),
    (
        // review: a scrutinee site whose arms use an OUTER local (saved_at's slot-0 branch): 10 + 5.
        "match-scrutinee-site-outer-local",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         fn choose() -> T { if ask() == 7 { B } else { A } }\n\
         fn user() -> Int { let k = 5  let r = match choose() { A -> k  B -> k * 2 }  r + k }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "15",
    ),
    (
        // review: a scrutinee site, then binders live across a site in the arm: 700 + 7 + 3 + 1.
        "match-scrutinee-site-then-arm-binders",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         type P { P(Int, Int) }\n\
         fn mkp() -> P { P(ask(), 3) }\n\
         fn user() -> Int { let r = match mkp() { P(x, y) -> x * 100 + ask() + y }  r + 1 }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "711",
    ),
    (
        // review: an arm binder shadowing an outer name used after the match: (1 + 7) + 100.
        "match-binder-shadows-outer",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         type W { W(Int) }\n\
         fn user() -> Int { let x = 100  let r = match W(1) { W(x) -> x + ask() }  r + x }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "108",
    ),
    (
        // review: a Bool-valued effectful match (an i1 phi).
        "match-bool-valued",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         fn user(t) -> Int { let b = match t { A -> ask() == 7  B -> False }  if b { 1 } else { 0 } }\n\
         pub fn main() -> Int { handle { user(A) } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "1",
    ),
    (
        // review: every arm ends in a site, so the join block is unreachable: (7 + 1) * 2.
        "match-all-arms-sites",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         fn user(t) -> Int { let v = match t { A -> ask()  B -> ask() + 1 }  v * 2 }\n\
         pub fn main() -> Int { handle { user(B) } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "16",
    ),
    (
        // review: an arm after a catch-all (an E0431 warning, not an error) is never emitted.
        "match-arm-after-catch-all",
        "effect Ask { fn ask() -> Int }\n\
         type T { A, B }\n\
         fn user(t) -> Int { match t { A -> ask()  _ -> 5  B -> 9 } }\n\
         pub fn main() -> Int { handle { user(B) } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "5",
    ),
];

/// Runs a `(tag, src, expected)` corpus natively, collecting every failure.
fn run_value_corpus(corpus: &[(&str, &str, &str)], dir_name: &str) {
    let dir = temp_dir(dir_name);
    let mut failures = Vec::new();
    for (tag, src, expected) in corpus {
        let core = lower_src(src);
        let exe = match try_compile_and_link(&core, &dir, tag) {
            Ok(exe) => exe,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let out = native(&exe).output().expect("run produced binary");
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

/// Runs a corpus natively and compares each value with the evaluator's.
fn run_differential_corpus(corpus: &[(&str, &str, &str)], dir_name: &str) {
    let dir = temp_dir(dir_name);
    let mut failures = Vec::new();
    for (tag, src, _) in corpus {
        let core = lower_src(src);
        let exe = match try_compile_and_link(&core, &dir, tag) {
            Ok(exe) => exe,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let out = native(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let want = eval_main_int(src);
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

/// HANDOFF step 1 (2026-10-05): a call in a `match` arm in tail position was
/// an ORDINARY call -- the direct emitter's `lower_tail` handled `If`, `Let`
/// and `App`, not `Match` -- so a loop through a match grew the stack.
/// Pre-existing since 5b-4; Linux's 8 MiB hid it until a 30,000-deep row
/// overflowed Windows' 1 MiB (5b-9a). At a million it overflows Linux too.
fn run_deep(tag: &str, src: &str, want: &str) {
    let dir = temp_dir(tag);
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, tag);
    let out = native(&exe).output().expect("run produced binary");
    diagnose_crash(&out.status, tag);
    assert!(
        out.status.success(),
        "{tag}: binary exited {:?}",
        out.status
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), want, "{tag}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_tail_call_in_a_match_arm_is_eliminated_at_a_million() {
    run_deep(
        "tail-in-match",
        "type B { T, F }\n\
         fn flag(n) { if n == 0 { T } else { F } }\n\
         fn lp(n, acc) { match flag(n) { T -> acc  F -> lp(n - 1, acc + 1) } }\n\
         pub fn main() -> Int { lp(1000000, 0) }\n",
        "1000000",
    );
}

#[test]
fn a_tail_call_through_a_pattern_binder_walks_a_million_cells() {
    // The binder `t` feeds the tail call: the arm's field load must not keep
    // anything alive past the `musttail`.
    run_deep(
        "tail-in-match-binder",
        "type L { Nil, Cons(Int, L) }\n\
         fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
         fn len(l, a) { match l { Nil -> a  Cons(_, t) -> len(t, a + 1) } }\n\
         pub fn main() -> Int { len(build(1000000, Nil), 0) }\n",
        "1000000",
    );
}

#[test]
fn the_elya_cek_machine_runs_natively_a_hundred_thousand_deep() {
    // The Elya CEK machine's loop is `ev`/`co` tail calls in match arms. Before
    // the fix, N = 10,000 segfaulted. 100,000 * 100,001 / 2 = 5,000,050,000
    // (computed, not run through the evaluator: too slow at this depth).
    let src = include_str!("../../../examples/03_cek.elya");
    let src = format!(
        "{}pub fn main() -> Int {{ run(sum_to(100000)) }}\n",
        &src[..src.find("pub fn main").expect("example has a main")]
    );
    run_deep("elya-cek-deep", &src, "5000050000");
}

#[test]
fn the_elya_cek_machine_runs_natively() {
    // `examples/03_cek.elya`: a CEK machine written in Elya, compiled natively
    // and checked against the evaluator. N = 1,000 (500,500): the evaluator
    // side keeps N modest. The deep native run is
    // `the_elya_cek_machine_runs_natively_a_hundred_thousand_deep`.
    let src = include_str!("../../../examples/03_cek.elya");
    let src = format!(
        "{}pub fn main() -> Int {{ run(sum_to(1000)) }}\n",
        &src[..src.find("pub fn main").expect("example has a main")]
    );
    let dir = temp_dir("elya-cek");
    let core = lower_src(&src);
    let exe = compile_and_link(&core, &dir, "elya-cek");
    assert_runs(&exe, "500500");
    assert_eq!(eval_main_int(&src), "500500");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_match_arm_after_a_catch_all_compiles_natively() {
    // Found by the 5b-9a review, pre-existing in the DIRECT emitter: an arm
    // after a catch-all is only a warning (E0431), but the emitter kept going
    // past the terminal arm and LLVM rejected the module ("does not have
    // terminator"). The evaluator takes the catch-all: 5.
    let src = "type T { A, B }\n\
               pub fn main() -> Int { let t = B  match t { A -> 1  _ -> 5  B -> 9 } }\n";
    let dir = temp_dir("arm-after-catch-all");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "after-catch-all");
    assert_runs(&exe, "5");
    assert_eq!(eval_main_int(src), "5");
    std::fs::remove_dir_all(&dir).ok();
}

/// Slice 5b-9b: effectful lambdas and closure calls, natively. Every row was
/// refused "effectful lambda (not yet compiled natively)" before; values
/// predicted before the first run.
const EFFECTFUL_LAMBDAS: &[(&str, &str, &str)] = &[
    (
        // a lambda that performs, called once (s1).
        "lambda-direct-call",
        "effect Ask { fn ask() -> Int }\n\
         fn user() -> Int { let f = fn() { ask() }  f() }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "7",
    ),
    (
        // two calls of one closure: 8 + 9 (s2).
        "lambda-two-calls",
        "effect Ask { fn ask() -> Int }\n\
         fn user() -> Int { let f = fn(x) { x + ask() }  f(1) + f(2) }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "17",
    ),
    (
        // a pure function builds the effectful closure (s5).
        "lambda-returned-by-pure-fn",
        "effect Ask { fn ask() -> Int }\n\
         fn mk(n) { fn() { n + ask() } }\n\
         fn user() -> Int { let g = mk(3)  g() }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "10",
    ),
    (
        // a fresh closure every iteration (s6).
        "lambda-in-a-loop",
        "effect Ask { fn ask() -> Int }\n\
         fn lp(n, acc) { if n == 0 { acc } else { let f = fn() { ask() }  lp(n - 1, acc + f()) } }\n\
         pub fn main() -> Int { handle { lp(1000, 0) } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "7000",
    ),
    (
        // the capture `k` is used AFTER the site inside the lambda: (1 + 7 + 5) * 2.
        "lambda-capture-across-site",
        "effect Ask { fn ask() -> Int }\n\
         fn user() -> Int { let k = 5  let f = fn(x) { x + ask() + k }  f(1) * 2 }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "26",
    ),
    (
        // the effectful closure call is in tail position: the continuation is passed on.
        "lambda-tail-call",
        "effect Ask { fn ask() -> Int }\n\
         fn user() -> Int { let f = fn(x) { x + ask() }  f(10) }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "17",
    ),
    (
        // the parameter cap: closure + 3 + continuation = 5.
        "lambda-three-params",
        "effect Ask { fn ask() -> Int }\n\
         fn user() -> Int { let f = fn(a, b, c) { a * 100 + b * 10 + c + ask() }  f(1, 2, 3) }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "130",
    ),
    (
        // a pure lambda returns an effectful one.
        "lambda-made-by-a-pure-lambda",
        "effect Ask { fn ask() -> Int }\n\
         fn user() -> Int { let f = fn(x) { fn() { x + ask() } }  let g = f(4)  g() }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "11",
    ),
    (
        // the second call runs inside the first call's resumption.
        "lambda-called-in-resume-path",
        "effect Ask { fn ask() -> Int }\n\
         fn user() -> Int { let f = fn() { ask() }  let v = f() + f()  v }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "14",
    ),
    (
        // control K1 found the other rows blind to WHERE captures are bound: here
        // the only capture `k` is binding 2 of [a, b, k] but slot 0 of the
        // closure. (1 + 7 + 5) + 2 = 15.
        "lambda-capture-index-differs-from-slot",
        "effect Ask { fn ask() -> Int }\n\
         fn user(a, b) -> Int { let k = 5  let f = fn(x) { x + ask() + k }  f(a) + b }\n\
         pub fn main() -> Int { handle { user(1, 2) } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "15",
    ),
    (
        // control K4 found no row reaching the resumption path's closure call:
        // here the site `ask()` is the closure call's ARGUMENT. f(7) = 14.
        "lambda-call-on-a-resumption-path",
        "effect Ask { fn ask() -> Int }\n\
         fn user() -> Int { let f = fn(x) { x + ask() }  let v = f(ask())  v }\n\
         pub fn main() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "14",
    ),
    (
        // from the independent review: an inner lambda's parameter `k` shadows the outer `k` the outer lambda captures: g(100) = 1107, + 5 + 7, f(1000) + 5.
        "lambda-param-shadows-a-capture",
        "effect Ask { fn ask() -> Int }\n\
         fn user(k) -> Int { let f = fn(x) { let g = fn(k) { k + x + ask() }  g(100) + k + ask() }  f(1000) + k }\n\
         fn prog() -> Int { handle { user(5) } with { Ask.ask() -> resume(7)  return(x) -> x } }\n\
         pub fn main() -> Int { prog() }\n",
        "1124",
    ),
    (
        // from the review: an effectful lambda inside an effectful lambda, capturing a, c and d from three depths.
        "lambda-nested-captures-at-several-depths",
        "effect Ask { fn ask() -> Int }\n\
         fn user(a, b) -> Int { let c = 3  let f = fn(x) { let d = x * 2  let g = fn(y) { y + d + c + ask() + a }  g(1) + ask() + b + g(2) }  f(10) + c }\n\
         fn prog() -> Int { handle { user(100, 1000) } with { Ask.ask() -> resume(7)  return(x) -> x } }\n\
         pub fn main() -> Int { prog() }\n",
        "1273",
    ),
    (
        // from the review: a lambda built in a handler's return clause and called under another handler: 101 + 1000 + 7 + 1.
        "lambda-built-in-a-return-clause",
        "effect Ask { fn ask() -> Int }\n\
         fn inner(base) { handle { base + 1 } with { return(x) -> fn(y) { x + y + ask() } } }\n\
         fn user() -> Int { let g = inner(100)  g(1000) + 1 }\n\
         fn prog() -> Int { handle { user() } with { Ask.ask() -> resume(7)  return(x) -> x } }\n\
         pub fn main() -> Int { prog() }\n",
        "1109",
    ),
];

#[test]
fn an_effectful_closure_in_tail_position_loops_a_million_times() {
    // 5b-9b: `lp` tail-calls the effectful closure `f`, which tail-calls `lp`
    // back -- both jumps must be `musttail` with the continuation passed on, or
    // the native stack grows. 1,000,000 * 7.
    run_deep(
        "lambda-tail-loop",
        "effect Ask { fn ask() -> Int }\n\
         fn lp(n, acc) { if n == 0 { acc } else { let f = fn(m, a) { lp(m, a + ask()) }  f(n - 1, acc) } }\n\
         pub fn main() -> Int { handle { lp(1000000, 0) } with { Ask.ask() -> resume(7)  return(x) -> x } }\n",
        "7000000",
    );
}

#[test]
fn effectful_lambda_corpus_runs_natively() {
    run_value_corpus(EFFECTFUL_LAMBDAS, "effectful-lambdas");
}

#[test]
fn effectful_lambda_corpus_matches_the_evaluator() {
    run_differential_corpus(EFFECTFUL_LAMBDAS, "differential-effectful-lambdas");
}

#[test]
fn effectful_match_corpus_runs_natively() {
    run_value_corpus(MATCH_EFFECTS, "match-effects");
}

#[test]
fn effectful_match_corpus_matches_the_evaluator() {
    run_differential_corpus(MATCH_EFFECTS, "differential-match-effects");
}

#[test]
fn the_task8_handler_corpus_compiles_runs_and_prints_the_expected_answer() {
    let dir = temp_dir("handler-8");
    let mut failures = Vec::new();
    for (tag, src, expected) in HANDLER_8 {
        let core = lower_src(src);
        let exe = match try_compile_and_link(&core, &dir, tag) {
            Ok(exe) => exe,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let out = native(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() || got != *expected {
            failures.push(format!(
                "{tag}: {:?} stdout={got:?} want={expected} stderr={:?}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn native_output_matches_the_evaluator_across_the_task8_handler_corpus() {
    let dir = temp_dir("differential-handler-8");
    let mut failures = Vec::new();
    for (tag, src, _) in HANDLER_8 {
        let core = lower_src(src);
        let exe = match try_compile_and_link(&core, &dir, tag) {
            Ok(exe) => exe,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let out = native(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let want = eval_main_int(src);
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
fn a_local_named_like_an_op_is_called_natively() {
    // Slice 5c-2 (replaces D11's `a_function_named_like_an_op_is_not_what_a_
    // perform_calls`: a top-level `fn ping` beside op `ping` is now E0205). A
    // LOCAL shadows the op; the evaluator CALLS it, and native must too.
    let src = "effect E { fn ping() -> Int }\n\
         fn user() -> Int {\n\
         \x20 let ping = fn() { 5 }\n\
         \x20 ping()\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 let v = handle { user() } with {\n\
         \x20   E.ping() -> resume(1)\n\
         \x20   return(x) -> x\n\
         \x20 }\n\
         \x20 let z = if v == 1 { io.println(\"PERFORMED the op\") } else { io.println(\"CALLED the local\") }\n\
         \x20 0\n\
         }\n";
    let dir = temp_dir("shadow-5c2");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "shadow");
    let (text, value) = native_text_value(&exe, "shadow");
    assert_eq!(text, eval_main_text(src));
    assert_eq!(text, "CALLED the local\n");
    assert_eq!(value, eval_main_int(src));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn shadowing_ends_with_its_scope_natively() {
    // 5c-2 n5: the local inside the block (5), the op after it (1). 2 would be
    // the old ops-first rule; 10 a shadow leaking out of its block.
    let src = "effect E { fn ping() -> Int }\n\
         fn user() -> Int {\n\
         \x20 let a = { let ping = fn() { 5 }  ping() }\n\
         \x20 a + ping()\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { user() } with {\n\
         \x20   E.ping() -> resume(1)\n\
         \x20   return(x) -> x\n\
         \x20 }\n\
         }\n";
    let dir = temp_dir("shadow-scope-5c2");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "shadow-scope");
    assert_runs(&exe, "6");
    assert_eq!(eval_main_int(src), "6");
    std::fs::remove_dir_all(&dir).ok();
}

/// Task 12 control 2a (site frames tagged past the table, so `gc_mark`'s
/// `tag >= gc_n_ctors` skip fires: the frame is kept, nothing in it traced)
/// fails this test SILENTLY -- it printed 6287, not 2550, and exited 0 --
/// which is why the assertion is on the value. Control 2b (rows kept, only
/// the `next` bit cleared) PASSES it: the short chain here needs the frame's
/// saved heap value, not what lies behind it. That is the test that shows the
/// two tracing controls test two different things.
#[test]
fn a_continuation_held_by_a_lambda_survives_a_collection() {
    // D12: the handle returns a lambda that holds the continuation; a
    // collection runs before the lambda resumes it. The frames (each saving
    // a heap `Cons`) are reachable only through the lambda's capture.
    let src = "type L { Nil, Cons(Int, L) }\n\
         fn churn(n, acc) { if n == 0 { acc } else { let g = Cons(n, Cons(n, Cons(n, Nil)))  churn(n - 1, acc + 1) } }\n\
         fn head(l) { match l { Nil -> 0  Cons(h, _) -> h } }\n\
         effect St { fn get() -> Int }\n\
         fn prog(n) { if n == 0 { 0 } else { let c = Cons(n, Nil)  get() + prog(n - 1) + head(c) } }\n\
         pub fn main() -> Int {\n\
         \x20 let f = handle { prog(50) } with {\n\
         \x20   St.get() -> fn(s) { (resume(s))(s + 1) }\n\
         \x20   return(x) -> fn(s) { x }\n\
         \x20 }\n\
         \x20 let z = churn(30000, 0)\n\
         \x20 f(1) + z - 30000\n\
         }\n";
    let dir = temp_dir("cont-gc-8");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "cont-gc");
    let (stdout, stats) = run_with_gc_stats(&exe, "cont-gc");
    assert!(stats.collections >= 1, "no collection ran: {stats:?}");
    assert_eq!(
        stdout,
        eval_main_int(src),
        "native diverges from the evaluator"
    );
    assert_eq!(stdout, "2550");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_second_resume_traps_natively_and_errors_in_the_evaluator() {
    // D13: one-shot. Both sides must FAIL: the evaluator with E0425, native
    // with its named trap -- never a re-run that computes an answer.
    let src = "effect S { fn get() -> Int }\n\
         fn body() -> Int { get() }\n\
         pub fn main() -> Int { handle { body() } with { S.get() -> resume(1) + resume(2)  return(r) -> r } }\n";
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let eval = elya::eval::run_module_value(&m);
    assert!(
        matches!(&eval, Err(e) if format!("{e:?}").contains("E0425")),
        "the evaluator must refuse the second resume with E0425"
    );
    let dir = temp_dir("double-resume-8");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "double");
    let out = native(&exe).output().expect("run produced binary");
    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("elya: a one-shot continuation was resumed twice"),
        "stderr should name the trap, got: {stderr}"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).is_empty(),
        "no answer may be printed"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_handler_frame_keeps_its_saved_heap_values_across_a_collection() {
    // `s` is saved in the HANDLER frame (word 4, mask bit 3) and read by the
    // return clause. `mk` returns before the continuation is resumed, so
    // after that `s` is reachable ONLY through the lambda -> continuation
    // object -> frame chain -> handler frame; a collection runs in between.
    // (A first version kept `s` in scope at the handle site, where the site's
    // own roots kept it alive, so control T5 could not fail it -- measured.)
    let src = "type L { Nil, Cons(Int, L) }\n\
         fn churn(n, acc) { if n == 0 { acc } else { let g = Cons(n, Cons(n, Cons(n, Nil)))  churn(n - 1, acc + 1) } }\n\
         fn head(l) { match l { Nil -> 0  Cons(h, _) -> h } }\n\
         effect S { fn get() -> Int }\n\
         fn body() -> Int { get() + 1 }\n\
         fn mk() {\n\
         \x20 let s = Cons(42, Nil)\n\
         \x20 handle { body() } with {\n\
         \x20   S.get() -> fn(y) { (resume(10))(y) }\n\
         \x20   return(r) -> fn(y) { r + head(s) + y }\n\
         \x20 }\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 let f = mk()\n\
         \x20 let z = churn(30000, 0)\n\
         \x20 f(5) + z - 30000\n\
         }\n";
    let dir = temp_dir("handler-frame-gc-8");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "hfgc");
    let (stdout, stats) = run_with_gc_stats(&exe, "hfgc");
    assert!(stats.collections >= 1, "no collection ran: {stats:?}");
    assert_eq!(
        stdout,
        eval_main_int(src),
        "native diverges from the evaluator"
    );
    assert_eq!(stdout, "58");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_deep_non_tail_effectful_recursion_finds_its_handler_in_constant_time() {
    // 400,000 performs, each from a frame chain 400,000 deep at its deepest.
    // Walking the chain to the handler on every perform is quadratic: it did
    // not finish in 30 s at this N (measured before the fix; 80k took 15 s).
    // With the handler found in O(1) it takes well under a second. The 20 s
    // deadline makes a regression fail by NAME instead of hanging the suite.
    let src = "effect S { fn get() -> Int }\n\
         fn loop(n) { if n == 0 { 0 } else { get() + loop(n - 1) } }\n\
         pub fn main() -> Int { handle { loop(400000) } with { S.get() -> resume(1)  return(r) -> r } }\n";
    let dir = temp_dir("handler-lookup-8");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "deeploop");
    let mut child = native(&exe)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let status = loop {
        if let Some(st) = child.try_wait().expect("wait") {
            break st;
        }
        if std::time::Instant::now() > deadline {
            child.kill().ok();
            panic!("400k performs did not finish in 20 s: the handler lookup is not O(1)");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let mut out = String::new();
    use std::io::Read;
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut out)
        .expect("read");
    assert!(status.success(), "{status:?}");
    assert_eq!(out.trim(), "400000");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_growing_live_set_is_collected_a_logarithmic_number_of_times() {
    // A fixed 64K-word threshold re-marks a growing live set every 64K words,
    // so total marking is quadratic (measured before the fix: 46 collections
    // building this 1M list; 183 for the 1M-deep effectful loop, 6.8 s).
    // Collecting when allocation since the last collection reaches
    // max(64K, live) doubles the heap between collections: O(log n) of them.
    let src = "type L { Nil, Cons(Int, L) }\n\
         fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(1, acc)) } }\n\
         pub fn main() -> Int { let keep = build(1000000, Nil)  match keep { Nil -> 0  Cons(h, t) -> h } }\n";
    let dir = temp_dir("gc-doubling");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "build1m");
    let (stdout, stats) = run_with_gc_stats(&exe, "build1m");
    assert_eq!(stdout, "1");
    assert!(stats.collections >= 1, "{stats:?}");
    assert!(
        stats.collections <= 10,
        "a live set growing to 3M words must not be re-marked every 64K words: {stats:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A1 (plan Task 9). Three shapes, each chosen for one property dispatch can
/// get wrong: a NON-TAIL resume (the value flows back through `+` in the
/// clause body), TWO ops under one handler (the `(effect, op)` match must read
/// the op name), and TWO sequential handles (the handler record must be
/// scoped to its body). No `<>`, no `with multi`, every main Int-valued.
/// Values were predicted in writing before the first run.
///
/// Task 12 control 1b (dispatch always loads clause-table entry 0): `two-ops`
/// printed 20, not 14 -- both performs took `Two.a` and resumed with 10, the
/// number Task 9 predicted for exactly this defect -- while `ask-nontail`, one
/// op and so blind to it, still printed 3. (Both corpus loops stop at their
/// first failing row, so `two-handles` was not reached under the control.)
/// The same control also printed 20 for Task 8's `two-ops-one-handler`
/// (want 30) and stopped `dynamically-nested-handle` with exit 1 ("no
/// clause"): any program with a second op is exposed by it.
const HANDLER_CORPUS: &[(&str, &str, &str)] = &[
    (
        "ask-nontail",
        "effect Ask { fn ask() -> Int }\n\
         fn one() { ask() }\n\
         pub fn main() -> Int {\n\
         \x20 handle { one() } with {\n\
         \x20   Ask.ask() -> 1 + resume(2)\n\
         \x20   return(x) -> x\n\
         \x20 }\n\
         }\n",
        "3",
    ),
    (
        "two-ops",
        "effect Two { fn a() -> Int  fn b() -> Int }\n\
         fn both() { a() + b() }\n\
         pub fn main() -> Int {\n\
         \x20 handle { both() } with {\n\
         \x20   Two.a() -> resume(10)\n\
         \x20   Two.b() -> resume(4)\n\
         \x20   return(x) -> x\n\
         \x20 }\n\
         }\n",
        "14",
    ),
    (
        "two-handles",
        "effect Ask { fn ask() -> Int }\n\
         fn one() { ask() }\n\
         pub fn main() -> Int {\n\
         \x20 let x = handle { one() } with { Ask.ask() -> resume(1)  return(v) -> v }\n\
         \x20 let y = handle { one() } with { Ask.ask() -> resume(20)  return(v) -> v }\n\
         \x20 x + y\n\
         }\n",
        "21",
    ),
];

/// Both corpus loops COLLECT failures and report them together (PARKED,
/// Task 12): asserting inside the loop hid every row after the first failure,
/// so a control's footprint was never fully visible (control 1b never reached
/// `two-handles`). Same rows, same expected values, same comparisons.
#[test]
fn the_handler_corpus_compiles_and_runs() {
    let dir = temp_dir("handler-corpus");
    let mut failures = Vec::new();
    for (tag, src, expected) in HANDLER_CORPUS {
        let core = lower_src(src);
        let exe = match try_compile_and_link(&core, &dir, tag) {
            Ok(exe) => exe,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let out = native(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() || got != *expected {
            failures.push(format!(
                "{tag}: {:?} stdout={got:?} want={expected} stderr={:?}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn native_output_matches_the_evaluator_across_the_handler_corpus() {
    let dir = temp_dir("differential-handler");
    let mut failures = Vec::new();
    for (tag, src, _) in HANDLER_CORPUS {
        let core = lower_src(src);
        let exe = match try_compile_and_link(&core, &dir, tag) {
            Ok(exe) => exe,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let out = native(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        let native = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let want = eval_main_int(src);
        if !out.status.success() || native != want {
            failures.push(format!(
                "{tag}: {:?} native={native:?} evaluator={want}",
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

/// A1's second half: the exact bytes. The `io.println` sits INSIDE the clause
/// body, so the text also witnesses that the body ran once per perform -- two
/// lines, in order. Both halves are compared with the evaluator, and the text
/// is also pinned, so an empty-vs-empty comparison cannot pass vacuously.
#[test]
fn a_printing_clause_body_matches_the_evaluator_byte_for_byte() {
    let src = "effect Ask { fn ask() -> Int }\n\
               fn twice() { ask() + ask() }\n\
               pub fn main() -> Int {\n\
               \x20 handle { twice() } with {\n\
               \x20   Ask.ask() -> { io.println(\"asked\")  resume(1) }\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               }\n";
    let dir = temp_dir("handler-printing");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "handler-printing");
    let (text, value) = native_text_value(&exe, "handler-printing");
    assert_eq!(
        text,
        eval_main_text(src),
        "native println text diverges from the evaluator"
    );
    assert_eq!(
        value,
        eval_main_int(src),
        "native main value diverges from the evaluator"
    );
    assert_eq!(
        text, "asked\nasked\n",
        "the clause body must run once per perform, in order"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A4, the heap half of Invariant N8-1 (plan Task 10). Four N over an 8x
/// spread. `live` is the LEVEL -- visible words still live at the end of the
/// LAST collection -- where `freed` and `words_since_gc` are flows.
///
/// NO CONSTANT IS PINNED. The claim is that the level does not grow with N.
/// `collections > 0` and `live > 0` are guards that `live` was computed at
/// all, NOT invariant guards (spec 7.3, ground 2).
#[test]
fn a_tail_resuming_handler_settles_its_live_set() {
    let dir = temp_dir("handler-settles");
    let mut levels = Vec::new();
    for n in [25_000, 50_000, 100_000, 200_000] {
        let src = format!(
            "effect Tick {{ fn tick() -> Int }}\n\
             fn spin(n) {{ if n == 0 {{ 0 }} else {{ let _ = tick()  spin(n - 1) }} }}\n\
             pub fn main() -> Int {{\n\
             \x20 handle {{ spin({n}) }} with {{\n\
             \x20   Tick.tick() -> resume(1)\n\
             \x20   return(x) -> x\n\
             \x20 }}\n\
             }}\n"
        );
        let tag = format!("settles-{n}");
        let core = lower_src(&src);
        let exe = compile_and_link(&core, &dir, &tag);
        let (stdout, stats) = run_with_gc_stats(&exe, &tag);
        assert_eq!(stdout, "0", "{tag}: the loop must run to completion");
        assert!(
            stats.collections > 0,
            "{tag}: no collection happened, so `live` measures nothing"
        );
        assert!(
            stats.live > 0,
            "{tag}: live=0 means the level was never computed"
        );
        levels.push((n, stats.live));
    }
    println!("A4 levels: {levels:?}");
    let (_, first) = levels[0];
    for (n, live) in &levels {
        assert_eq!(
            *live, first,
            "live set must not grow with N (a growing level is a frame leak): {levels:?}, diverged at N={n}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A5. The same handler with the recursive call under `+` instead of in
/// tail position: each level holds a pending frame, so the captured chain
/// grows with N and the level must move. STRICT inequality -- an instrument
/// that a deliberately-growing control cannot move is measuring nothing.
///
/// Task 12 controls 2a and 2b both kill this control at N = 10,000 with
/// SIGSEGV. (The captured chain is the live set it measures; the crash is
/// consistent with that chain being freed while still in use, though a
/// signal alone does not prove it.)
#[test]
fn a_growing_control_moves_the_live_set() {
    let dir = temp_dir("handler-grows");
    let mut levels = Vec::new();
    // 10k/80k, not the plan's 5k/40k (8x kept; approved 2026-10-04): at 5k the
    // control allocates 60,004 words, under the 64K first-collection floor,
    // so it never collects and `live` measures nothing (measured).
    for n in [10_000, 80_000] {
        let src = format!(
            "effect Tick {{ fn tick() -> Int }}\n\
             fn spin(n) {{ if n == 0 {{ 0 }} else {{ tick() + spin(n - 1) }} }}\n\
             pub fn main() -> Int {{\n\
             \x20 handle {{ spin({n}) }} with {{\n\
             \x20   Tick.tick() -> resume(1)\n\
             \x20   return(x) -> x\n\
             \x20 }}\n\
             }}\n"
        );
        let tag = format!("grows-{n}");
        let core = lower_src(&src);
        let exe = compile_and_link(&core, &dir, &tag);
        let (stdout, stats) = run_with_gc_stats(&exe, &tag);
        assert_eq!(
            stdout,
            n.to_string(),
            "{tag}: each of the N performs resumes with 1"
        );
        assert!(
            stats.collections > 0,
            "{tag}: no collection happened, so `live` measures nothing"
        );
        levels.push((n, stats.live));
    }
    println!("A5 levels: {levels:?}");
    assert!(
        levels[1].1 > levels[0].1,
        "a growing control must move the live set, or A4's settling proves nothing: {levels:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The MAX_PARAMS edge, on purpose (PARKED: unverified on win64, where more
/// than 5 tailcc params aborted). `f` and `g` are CPS with 4 source params +
/// the continuation = 5; `f -> g` is a musttail CPS call at that width; the
/// `op3` clause takes 3 op args + the continuation + the handler frame = 5.
/// Predicted 20 before the first run: get -> 10, g(11, 2, 3, 4), op3 -> 16, + 4.
#[test]
fn effectful_functions_at_the_parameter_cap_run_natively() {
    let src = "effect E { fn get() -> Int  fn op3(a: Int, b: Int, c: Int) -> Int }\n\
               fn g(a, b, c, d) { op3(a, b, c) + d }\n\
               fn f(a, b, c, d) { let x = get()  g(a + x, b, c, d) }\n\
               pub fn main() -> Int {\n\
               \x20 handle { f(1, 2, 3, 4) } with {\n\
               \x20   E.get() -> resume(10)\n\
               \x20   E.op3(a, b, c) -> resume(a + b + c)\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               }\n";
    let dir = temp_dir("param-cap");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "param-cap");
    assert_runs(&exe, "20");
    assert_eq!(eval_main_int(src), "20");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_unit_literal_compiles_natively() {
    // `Unit` is the i64 word 0 (`repr_ty`); codegen refused the literal itself
    // until A9's source needed `resume(Unit)` (plan Task 11).
    let src = "fn f() { Unit }\n\
               pub fn main() -> Int { let u = f()  7 }\n";
    let dir = temp_dir("unit-literal");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "unit-literal");
    assert_runs(&exe, "7");
    assert_eq!(eval_main_int(src), "7");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_unit_main_prints_its_text_then_its_word() {
    // A Unit-valued main compiles: the shim prints main's word, which for Unit
    // is 0, after the program's text (plan Task 11: A9's main is
    // `io.println(...)`). The text is compared with the evaluator's output.
    let src = "pub fn main() { io.println(\"hi\") }\n";
    let dir = temp_dir("unit-main");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "unit-main");
    let (text, value) = native_text_value(&exe, "unit-main");
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (interp, _) = elya::eval::run_module_value(&m).expect("evaluator runs it");
    assert_eq!(
        text,
        interp.output(),
        "native text diverges from the evaluator"
    );
    assert_eq!(text, "hi\n");
    assert_eq!(value, "0", "Unit's word");
    std::fs::remove_dir_all(&dir).ok();
}

/// A9, the machine-stack half of Invariant N8-1 (plan Task 11) -- the other
/// half from Task 10's heap measurement; neither subsumes the other.
///
/// The source is copied verbatim from `tests/state_effect.rs`
/// (`state_tail_loop`) at N = 1,000,000. Its PEAK assertions are an evaluator
/// instrument and are deliberately not ported.
///
/// DEVIATION FROM A9, REPORTED NOT PATCHED: A9 says `assert_runs` "printing
/// exactly `x`", but main returns Unit and the shim prints main's word, so
/// stdout is "x\n0\n". The splitter separates them and the TEXT half is
/// asserted, which also pins the newline `assert_runs`'s `.trim()` would
/// drop. The value half is a shim artifact and is not pinned.
///
/// N MUST NOT BE LOWERED: at small N this completes even on an implementation
/// that grows the stack linearly (Task 12's fifth control shows exactly that).
///
/// Task 12, observed. Control 3b (the tail-position call loses `musttail`)
/// kills this test with SIGSEGV, named by `diagnose_crash` (and with it
/// every other million-deep tail test and three GC loops, 8 tests in all).
/// Control 3a lowers N to 1_000 and the test PASSES -- and still passes with
/// 3b applied too, so at that N the test cannot tell a flat stack from a
/// growing one: the tautology, demonstrated rather than asserted. Control 1a
/// (no handler re-installation at resume) fails it with exit 1: its resumes
/// run after the handle returned. Control 1b (always clause 0) fails it with
/// exit 1, where a SIGSEGV was predicted; its stderr was not captured.
#[test]
fn a_state_passing_tail_loop_is_bounded_natively_at_a_million() {
    let src = "effect State { fn get() -> String  fn set(v: String) -> Unit }\n\
               fn loop(n) { if n == 0 { get() } else { let _ = set(\"x\")  loop(n - 1) } }\n\
               pub fn main() {\n\
               \x20 let program = handle { loop(1000000) } with {\n\
               \x20   State.get() -> fn(s) { (resume(s))(s) }\n\
               \x20   State.set(v) -> fn(s) { (resume(Unit))(v) }\n\
               \x20   return(x) -> fn(s) { x }\n\
               \x20 }\n\
               \x20 io.println(program(\"init\"))\n\
               }\n";
    let dir = temp_dir("state-tail-million");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "state-tail-million");
    let out = native(&exe).output().expect("run produced binary");
    diagnose_crash(&out.status, "state-tail-million");
    assert!(out.status.success(), "binary exited {:?}", out.status);
    let (text, _unit_word) =
        split_text_and_value(&String::from_utf8_lossy(&out.stdout), "state-tail-million");
    assert_eq!(
        text, "x\n",
        "the state-passing tail loop must run to completion at N = 1_000_000 and print exactly one line"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ---- Slice 5b-10: nested handles and handles inside effectful code (D17 lifted) ----

/// Slice 5b-10 (spec `2026-10-08-elya-slice-5b10-nested-handles-design.md`).
/// Every row was refused by `elya build` before -- "handle nested inside another
/// handle", "handle inside an effectful function", or "calling convention
/// disagrees with the callee" (a handle that LEAKS an effect made its function
/// effectful while `contains_effect` called it direct). Expected values are the
/// evaluator's, measured before the first native run (spec §0).
///
/// Negative controls, each reverted (`cmp` against a saved copy), each failing
/// differently (rows by their spec names; m12 is the million-step test, r1 the
/// `CPS_ROOTING` row):
/// - K1, the dispatch walk stops at the current handler: m2, m5, m13 and the
///   three-level row exit 1 ("no clause for this operation"); m1 still passes
///   -- its clause runs with the outer handler already current (K2's job);
/// - K2, clauses run with their OWN handle current: m1, m6, m7, m8, m12, m13
///   and r1 never terminate (a clause's perform reaches itself);
/// - K3, a resume does not re-parent its handler: m5 alone, 101 for 8 -- the
///   resumed body's `t()` escapes the clause's handler to main's;
/// - K4, a CPS resume does not rebind `next`: m7 200 for 206, r1 70014 for
///   100014 -- the remainder of the resuming clause is skipped;
/// - K5, the tail-resume install in `clause_tail` removed: m2, m6, m7, the
///   three-level row and r1 exit 1. 5b-8's A6 finding was that no test
///   isolated this install; these rows do;
/// - K6, a CPS return clause leaves its own handle current: m13 12 for 111,
///   and m12 never terminates.
const NESTED_HANDLES: &[(&str, &str, &str)] = &[
    (
        // m1: `inner`'s clause performs S, which its own handle also handles; the clause runs OUTSIDE it, so the outer handle answers 5: (5 * 10) + 1.
        "clause-performs-to-the-outer-handler",
        "effect S { fn get() -> Int }\n\
         fn inner() -> Int {\n\
         \x20 handle { get() + 1 } with {\n\
         \x20   S.get() -> resume(get() * 10)\n\
         \x20   return(r) -> r\n\
         \x20 }\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { inner() } with {\n\
         \x20   S.get() -> resume(5)\n\
         \x20   return(r) -> r\n\
         \x20 }\n\
         }\n",
        "51",
    ),
    (
        // m2: the inner handle (T) is nested in the outer (S) body and its body performs S, one parent up: ((3 + 2) * 10) + 1.
        "handle-in-a-handle-body",
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 handle {\n\
         \x20   handle { get() + t() } with { T.t() -> resume(2)  return(r) -> r * 10 }\n\
         \x20 } with { S.get() -> resume(3)  return(r) -> r + 1 }\n\
         }\n",
        "51",
    ),
    (
        // m3: a handle that discharges everything, inside an effectful function: a direct nesting call there.
        "non-leaking-handle-in-an-effectful-fn",
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         fn g() -> Int {\n\
         \x20 let a = handle { get() } with { S.get() -> resume(1)  return(r) -> r }\n\
         \x20 a + t()\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { g() } with { T.t() -> resume(40)  return(r) -> r }\n\
         }\n",
        "41",
    ),
    (
        // m4: a handle inside a clause body: (7 * 2) + 1.
        "handle-in-a-clause",
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 handle { get() + 1 } with {\n\
         \x20   S.get() -> resume(handle { t() * 2 } with { T.t() -> resume(7)  return(r) -> r })\n\
         \x20   return(r) -> r\n\
         \x20 }\n\
         }\n",
        "15",
    ),
    (
        // m5: the clause resumes INSIDE its own T handle, so the resumed body's `t()` is answered there (7), not by main's (100): 1 + 7.
        "resume-inside-a-clause-handle",
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         fn body() -> Int { get() + t() }\n\
         fn mid() -> Int {\n\
         \x20 handle { body() } with {\n\
         \x20   S.get() -> handle { resume(1) } with { T.t() -> resume(7)  return(r) -> r }\n\
         \x20   return(r) -> r\n\
         \x20 }\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { mid() } with { T.t() -> resume(100)  return(r) -> r }\n\
         }\n",
        "8",
    ),
    (
        // m6: both performs of S reach a clause that performs T: (10 + 1) * 2.
        "leaking-clause-in-a-fn-with-a-parameter",
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         fn twice(n) {\n\
         \x20 handle { get() + get() } with {\n\
         \x20   S.get() -> resume(t() + n)\n\
         \x20   return(r) -> r\n\
         \x20 }\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { twice(1) } with { T.t() -> resume(10)  return(r) -> r }\n\
         }\n",
        "22",
    ),
    (
        // m7: the second clause's answer is the value of the FIRST clause's `resume` (next rebound at the resume): 3 + (3 + 200).
        "non-tail-resume-then-a-second-perform",
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         fn h() -> Int {\n\
         \x20 handle { get() + get() } with {\n\
         \x20   S.get() -> t() + resume(1)\n\
         \x20   return(r) -> r * 100\n\
         \x20 }\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { h() } with { T.t() -> resume(3)  return(r) -> r }\n\
         }\n",
        "206",
    ),
    (
        // m8: a return clause that performs an outer effect: 1 + 30.
        "return-clause-performs",
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         fn h() -> Int {\n\
         \x20 handle { get() } with {\n\
         \x20   S.get() -> resume(1)\n\
         \x20   return(r) -> r + t()\n\
         \x20 }\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { h() } with { T.t() -> resume(30)  return(r) -> r }\n\
         }\n",
        "31",
    ),
    (
        // m13: once the inner handle has returned, `get()` goes to the outer S handler: (1 + 10) + 100.
        "finished-leaking-handle-restores-the-outer",
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         fn g() -> Int {\n\
         \x20 let a = handle { get() + t() } with { S.get() -> resume(1)  return(r) -> r }\n\
         \x20 a + get()\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { handle { g() } with { S.get() -> resume(100)  return(r) -> r } } with { T.t() -> resume(10)  return(r) -> r }\n\
         }\n",
        "111",
    ),
    (
        // a() walks two parents, b() one: 1 + 2 * 10 + 3 * 100.
        "three-level-dispatch",
        "effect A { fn a() -> Int }\n\
         effect B { fn b() -> Int }\n\
         effect C { fn c() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 handle {\n\
         \x20   handle {\n\
         \x20     handle { a() + b() * 10 + c() * 100 } with { C.c() -> resume(3)  return(r) -> r }\n\
         \x20   } with { B.b() -> resume(2)  return(r) -> r }\n\
         \x20 } with { A.a() -> resume(1)  return(r) -> r }\n\
         }\n",
        "321",
    ),
    (
        // the inner handle (S) forwards `put` (P) to the outer, whose NON-TAIL resume must get the outer handle's answer: ((1 + 10) * 10 + 1000) * 2. (The 5b-10 review's partial-handler row, rewritten with two effects once E0207 made partial handlers an error.)
        "forwarded-op-non-tail-resume",
        "effect S { fn get() -> Int }\n\
         effect P { fn put(x: Int) -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 handle {\n\
         \x20   handle { get() + put(5) } with { S.get() -> resume(1)  return(r) -> r * 10 }\n\
         \x20 } with { P.put(x) -> resume(x * 2) + 1000  return(r) -> r * 2 }\n\
         }\n",
        "1220",
    ),
    (
        // the same, the outer clause aborting: (5 + 1000) + 1.
        "forwarded-op-outer-clause-aborts",
        "effect S { fn get() -> Int }\n\
         effect P { fn put(x: Int) -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 let v = handle {\n\
         \x20   handle { get() + put(5) } with { S.get() -> resume(1)  return(r) -> r * 10 }\n\
         \x20 } with { P.put(x) -> x + 1000  return(r) -> r * 2 }\n\
         \x20 v + 1\n\
         }\n",
        "1006",
    ),
    (
        // the aborting shape inside an effectful function, under a T handler: ((5 + 1000 + 10) + 1) * 3 + 7.
        "forwarded-op-abort-in-an-effectful-fn",
        "effect S { fn get() -> Int }\n\
         effect P { fn put(x: Int) -> Int }\n\
         effect T { fn t() -> Int }\n\
         fn f() -> Int {\n\
         \x20 let v = handle {\n\
         \x20   let a = handle { get() + put(5) } with { S.get() -> resume(1)  return(r) -> r * 10 }\n\
         \x20   a + t()\n\
         \x20 } with { P.put(x) -> x + 1000 + t()  return(r) -> r * 2 }\n\
         \x20 v + 1\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { f() * 3 } with { T.t() -> resume(10)  return(r) -> r + 7 }\n\
         }\n",
        "3055",
    ),
    (
        // `f`'s handle forwards P out of `f` (typed `{P}` now): ((1 + 10) * 10 + 1000 + 3) * 2.
        "forwarded-op-through-a-function",
        "effect S { fn get() -> Int }\n\
         effect P { fn put(x: Int) -> Int }\n\
         fn f() -> Int {\n\
         \x20 handle { get() + put(5) } with { S.get() -> resume(1)  return(r) -> r * 10 }\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { f() + 3 } with { P.put(x) -> resume(x * 2) + 1000  return(r) -> r * 2 }\n\
         }\n",
        "1226",
    ),
    (
        // The resume-row review: `g` resumes directly in one branch of its
        // clause and builds a resuming lambda in the other, then performs L.
        // The first version of the resume-row fix typed `g` pure (its row
        // merged with a lambda's that closed), and this build was refused
        // "calling convention disagrees": 7 + 1.
        "direct-resume-beside-a-resuming-lambda",
        "effect S { fn get() -> Int }\n\
         effect L { fn lg(x: Int) -> Int }\n\
         fn g(c) {\n\
         \x20 let f = handle { get() } with { S.get() -> if c { resume(0) } else { fn(s) { (resume(s))(s) } }  return(x) -> fn(s) { x } }\n\
         \x20 f(7) + lg(1)\n\
         }\n\
         pub fn main() -> Int { handle { g(False) } with { L.lg(x) -> resume(x)  return(r) -> r } }\n",
        "8",
    ),
];

#[test]
fn the_nested_handle_corpus_compiles_and_runs() {
    run_value_corpus(NESTED_HANDLES, "nested-handles");
}

#[test]
fn native_output_matches_the_evaluator_across_the_nested_handle_corpus() {
    run_differential_corpus(NESTED_HANDLES, "nested-handles-diff");
}

/// HANDOFF "`resume` carries its handle's row" (replaces
/// `an_escaped_resume_of_a_leaking_handle_is_refused_by_name`, which pinned the
/// refusal 5b-10 shipped while the front end typed `resume` effect-free): a
/// lambda that resumes a LEAKING handle's continuation is now typed with the
/// handled body's remaining row (`{T}`), so it is an effectful lambda, its
/// body a CPS region, and the CPS resume inside it compiles. The continuation
/// is resumed after its handle returned: 5 + 10 + 0.
#[test]
fn an_escaped_resume_of_a_leaking_handle_runs_natively() {
    let src = "effect S { fn get() -> Int }\n\
               effect T { fn t() -> Int }\n\
               fn g() -> Int {\n\
               \x20 let f = handle { get() + t() } with {\n\
               \x20   S.get() -> fn(s) { (resume(s))(s) }\n\
               \x20   return(x) -> fn(s) { x + 0 * t() }\n\
               \x20 }\n\
               \x20 f(5)\n\
               }\n\
               pub fn main() -> Int {\n\
               \x20 handle { g() } with { T.t() -> resume(10)  return(r) -> r }\n\
               }\n";
    assert_eq!(eval_main_int(src), "15");
    let dir = temp_dir("escaped-leaking-resume");
    let exe = compile_and_link(&lower_src(src), &dir, "escaped-leaking-resume");
    assert_runs(&exe, "15");
    std::fs::remove_dir_all(&dir).ok();
}

/// Slice 5b-10: a handle that leaks T, entered on every iteration of a tail
/// loop (a CPS handle as a continuation site, its frame linked to the loop's
/// continuation). A million iterations keep the machine stack flat and the
/// heap collectable: 3 * 1_000_000 + 2.
#[test]
fn a_leaking_handle_in_a_tail_loop_is_bounded_natively_at_a_million() {
    let src = "effect S { fn get() -> Int }\n\
               effect T { fn t() -> Int }\n\
               fn go(n, acc) {\n\
               \x20 if n == 0 { acc + t() } else {\n\
               \x20   let x = handle { get() + t() } with { S.get() -> resume(1)  return(r) -> r }\n\
               \x20   go(n - 1, acc + x)\n\
               \x20 }\n\
               }\n\
               pub fn main() -> Int {\n\
               \x20 handle { go(1000000, 0) } with { T.t() -> resume(2)  return(r) -> r }\n\
               }\n";
    let dir = temp_dir("leaking-handle-million");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "leaking-handle-million");
    let out = native(&exe).output().expect("run produced binary");
    diagnose_crash(&out.status, "leaking-handle-million");
    assert!(out.status.success(), "binary exited {:?}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "3000002");
    std::fs::remove_dir_all(&dir).ok();
}

// ---- Sub-effecting for function values (2026-10-08, the maintainer's decision) ----

/// Programs the front end newly accepts through sub-effecting, natively: value
/// and evaluator differential. Rejected by `check` before (E0423).
const SUB_EFFECTING: &[(&str, &str, &str)] = &[
    (
        // an `if` joins a pure lambda and one that recurses into an effectful
        // `go`; the joined node type names L, so BOTH lambdas compile with the
        // CPS convention.
        "pure-and-effectful-lambdas-joined",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn go(n) {\n\
         \x20 let a = lg(n)\n\
         \x20 let f = if n == 0 { fn(s) { s } } else { fn(s) { s + go(n - 1) } }\n\
         \x20 f(1) + a\n\
         }\n\
         pub fn main() -> Int { handle { go(2) } with { L.lg(x) -> resume(x)  return(r) -> r } }\n",
        "6",
    ),
    (
        // the resuming clause value performs T, the return clause's does not;
        // they join in the handle's result: ((1 * 2) + 10) * 2.
        "clause-values-with-different-rows",
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 handle {\n\
         \x20   let f = handle { get() + t() } with { S.get() -> fn(k) { (resume(k(1)))(k) }  return(x) -> fn(k) { k(x) } }\n\
         \x20   f(fn(z) { z * 2 })\n\
         \x20 } with { T.t() -> resume(10)  return(r) -> r }\n\
         }\n",
        "24",
    ),
    (
        // m10 of the 5b-10 spec, which the resume-row fix had made E0423.
        "pure-return-value-beside-a-resuming-one",
        "effect S { fn get() -> Int }\n\
         effect T { fn t() -> Int }\n\
         fn g() -> Int {\n\
         \x20 let f = handle { get() + t() } with {\n\
         \x20   S.get() -> fn(s) { (resume(s))(s) }\n\
         \x20   return(x) -> fn(s) { x }\n\
         \x20 }\n\
         \x20 f(5)\n\
         }\n\
         pub fn main() -> Int { handle { g() } with { T.t() -> resume(10)  return(r) -> r } }\n",
        "15",
    ),
];

#[test]
fn the_sub_effecting_corpus_compiles_and_runs() {
    run_value_corpus(SUB_EFFECTING, "sub-effecting");
}

#[test]
fn native_output_matches_the_evaluator_across_the_sub_effecting_corpus() {
    run_differential_corpus(SUB_EFFECTING, "sub-effecting-diff");
}

/// Sub-effecting lets a DIRECT closure (`f`, generic and pure) be used where an
/// effectful function is expected (the `if` joins it with a T-performing
/// lambda). Its code has the direct convention, so the CPS call of `g` jumped
/// into it with the wrong signature: measured 2 where the evaluator printed 13,
/// then refused by name. N7 part 1 (replaces
/// `a_direct_closure_used_where_an_effectful_one_is_expected_is_refused_by_name`):
/// the use is eta-expanded into an adapter lambda with the CPS convention.
#[test]
fn a_direct_closure_used_where_an_effectful_one_is_expected_runs_through_an_adapter() {
    let src = "effect T { fn t() -> Int }\n\
               fn h(c) {\n\
               \x20 let f = fn(x) { x + 1 }\n\
               \x20 let g = if c { f } else { fn(x) { t() + x } }\n\
               \x20 g(1)\n\
               }\n\
               pub fn main() -> Int { handle { h(True) + h(False) } with { T.t() -> resume(10)  return(r) -> r } }\n";
    assert_eq!(eval_main_int(src), "13");
    assert_native_matches(src, "13", "direct-closure-upcast");
}

/// The sub-effecting review's F3: the same upcast reached through a handler's
/// RETURN binder (`x`, the direct closure `mk()` returns, joined with an
/// effectful lambda): 2 where the evaluator printed 102, then refused by name.
/// N7 part 1 (replaces `a_direct_closure_upcast_through_a_return_binder_is_refused_by_name`):
/// the return binder is tracked, so its upcast gets the adapter too.
#[test]
fn a_direct_closure_upcast_through_a_return_binder_runs_through_an_adapter() {
    let src = "effect T { fn t() -> Int }\n\
               fn mk() { fn(x) { x + 1 } }\n\
               fn h(c) {\n\
               \x20 let g = handle { mk() } with { T.t() -> resume(0)  return(x) -> if c { x } else { fn(s) { t() + s } } }\n\
               \x20 g(1)\n\
               }\n\
               pub fn main() -> Int { handle { h(True) + 100 } with { T.t() -> resume(10)  return(r) -> r } }\n";
    assert_eq!(eval_main_int(src), "102");
    assert_native_matches(src, "102", "direct-closure-upcast-ret");
}

/// N7 part 1, measured on `main` at 5799ba9: the upcast one level down, in the
/// RESULT of a function value. The outer conventions agree (both direct), so
/// the outermost-only refusal let it through, and the inner direct closure was
/// called with the CPS convention: native printed 2 where the evaluator
/// printed 3306. The adapter now coerces every covariant layer.
#[test]
fn a_direct_closure_returned_where_an_effectful_one_is_expected_runs() {
    let src = "effect T { fn t() -> Int }\n\
               fn h(c) {\n\
               \x20 let f = fn() { fn(x) { x + 1 } }\n\
               \x20 let g = if c { f } else { fn() { fn(x) { t() + x } } }\n\
               \x20 let k = g()\n\
               \x20 k(1) * 3\n\
               }\n\
               pub fn main() -> Int { handle { h(True) + h(False) * 100 } with { T.t() -> resume(10)  return(r) -> r } }\n";
    assert_eq!(eval_main_int(src), "3306");
    assert_native_matches(src, "3306", "direct-closure-result-upcast");
}

fn assert_native_matches(src: &str, want: &str, tag: &str) {
    let dir = temp_dir(tag);
    let exe = try_compile_and_link(&lower_src(src), &dir, tag).unwrap_or_else(|e| panic!("{e}"));
    let out = native(&exe).output().expect("run produced binary");
    diagnose_stack_overflow(&out.status, tag);
    let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{tag}: {:?}", out.status);
    assert_eq!(got, want, "{tag}");
}

/// N7 part 1 (spec `2026-10-09-elya-n7a-convention-specialization-design.md`):
/// row-polymorphic functions used at user effects (each was refused by name,
/// "effect-polymorphic function used at a user effect") and direct closures
/// upcast to effectful types (refused, "direct function used where an
/// effectful one is expected"). Values are the evaluator's.
const CONVENTIONS: &[(&str, &str, &str)] = &[
    (
        // `w`'s own `lg` and `k`'s row: `r` is instantiated at nothing new, so
        // no clone is needed; the old guard refused it anyway.
        "pure-callback-into-a-relay",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn w(k) { k(0) + lg(1) }\n\
         pub fn main() -> Int {\n\
         \x20 handle { w(fn(x) { x + 5 }) } with { L.lg(x) -> resume(x * 10)  return(r) -> r }\n\
         }\n",
        "15",
    ),
    (
        "apply-at-pure-and-at-a-user-effect",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn apply(f) { f(1) + 1 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = apply(fn(x) { x * 7 })\n\
         \x20 let b = handle { apply(fn(x) { lg(x) }) } with { L.lg(x) -> resume(x + 40)  return(r) -> r }\n\
         \x20 a * 100 + b\n\
         }\n",
        "842",
    ),
    (
        "recursive-map-at-two-rows",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn mapl(f, n) { if n == 0 { 0 } else { f(n) + mapl(f, n - 1) } }\n\
         pub fn main() -> Int {\n\
         \x20 let pure = mapl(fn(x) { x }, 10)\n\
         \x20 let eff = handle { mapl(fn(x) { lg(x) }, 10) } with { L.lg(x) -> resume(x * 2)  return(r) -> r }\n\
         \x20 pure * 1000 + eff\n\
         }\n",
        "55110",
    ),
    (
        "let-bound-generic-lambda",
        "effect S { fn get() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 handle { let app = fn(g) { g() + 1 }  let h = fn() { get() }  app(h) * 2 + app(fn() { 3 }) } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n",
        "20",
    ),
    (
        "annotated-twice",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn twice(f: fn(Int) -> Int, x: Int) -> Int { f(f(x)) }\n\
         pub fn main() -> Int {\n\
         \x20 let a = twice(fn(x) { x * 3 }, 2)\n\
         \x20 let b = handle { twice(fn(x) { lg(x) + 1 }, 3) } with { L.lg(x) -> resume(x * 10)  return(r) -> r }\n\
         \x20 a * 1000 + b\n\
         }\n",
        "18311",
    ),
    (
        "mutual-recursion-over-a-callback",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn ev(f: fn(Int) -> Int, n: Int) -> Int { if n == 0 { 0 } else { f(n) + od(f, n - 1) } }\n\
         fn od(f: fn(Int) -> Int, n: Int) -> Int { if n == 0 { 0 } else { ev(f, n - 1) - f(n) } }\n\
         pub fn main() -> Int {\n\
         \x20 let a = ev(fn(x) { x }, 10)\n\
         \x20 let b = handle { ev(fn(x) { lg(x) }, 10) } with { L.lg(x) -> resume(x * 3)  return(r) -> r }\n\
         \x20 a * 1000 + b\n\
         }\n",
        "5015",
    ),
    (
        "a-generic-function-calls-a-generic-function",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn apply(f: fn(Int) -> Int) -> Int { f(1) }\n\
         fn apply2(f: fn(Int) -> Int) -> Int { apply(f) + apply(f) * 10 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = apply2(fn(x) { x + 1 })\n\
         \x20 let b = handle { apply2(fn(x) { lg(x) + 2 }) } with { L.lg(x) -> resume(x * 5)  return(r) -> r }\n\
         \x20 a * 1000 + b\n\
         }\n",
        "22077",
    ),
    (
        "three-instantiations-three-clones",
        "effect L { fn lg(x: Int) -> Int }\n\
         effect M { fn mm(x: Int) -> Int }\n\
         fn apply(f: fn(Int) -> Int) -> Int { f(2) + 1 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = handle { apply(fn(x) { lg(x) }) } with { L.lg(x) -> resume(x * 5)  return(r) -> r }\n\
         \x20 let b = handle { apply(fn(x) { mm(x) }) } with { M.mm(x) -> resume(x * 7)  return(r) -> r }\n\
         \x20 let c = handle { handle { apply(fn(x) { mm(x) + lg(x) }) } with { M.mm(x) -> resume(x * 7)  return(r) -> r } } with { L.lg(x) -> resume(x * 100)  return(r) -> r }\n\
         \x20 a * 10000 + b * 100 + c\n\
         }\n",
        "111715",
    ),
    (
        "a-generic-call-inside-an-effectful-lambda",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn apply(f: fn(Int) -> Int) -> Int { f(3) * 2 }\n\
         pub fn main() -> Int {\n\
         \x20 handle { let k = fn(y) { apply(fn(x) { lg(x) + y }) }  k(1) + k(2) } with { L.lg(x) -> resume(x * 10)  return(r) -> r }\n\
         }\n",
        "126",
    ),
    (
        // `guard` handles E around `f()`; at `{E, L}` the clone's handle
        // LEAKS L, so it is a CPS handle -- the leak analysis sees the label.
        "a-clone-whose-handle-leaks-the-instantiated-effect",
        "effect E { fn e() -> Int }\n\
         effect L { fn lg(x: Int) -> Int }\n\
         fn guard(f) { handle { f() } with { E.e() -> resume(5)  return(r) -> r + 1 } }\n\
         pub fn main() -> Int {\n\
         \x20 let a = guard(fn() { e() * 2 })\n\
         \x20 let b = handle { guard(fn() { e() + lg(4) }) } with { L.lg(x) -> resume(x * 10)  return(r) -> r }\n\
         \x20 a * 1000 + b\n\
         }\n",
        "11046",
    ),
    (
        "non-tail-recursion-through-a-clone",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn go(f: fn(Int) -> Int, n: Int) -> Int { if n == 0 { 0 } else { go(f, n - 1) + f(n) } }\n\
         pub fn main() -> Int {\n\
         \x20 handle { go(fn(x) { lg(x) }, 2000) } with { L.lg(x) -> resume(x)  return(r) -> r }\n\
         }\n",
        "2001000",
    ),
    (
        "upcast-of-a-let-bound-direct-closure",
        "effect T { fn t() -> Int }\n\
         fn pick(c) {\n\
         \x20 let f = fn(x) { x + 1 }\n\
         \x20 let g = if c { f } else { fn(x) { t() + x } }\n\
         \x20 g(10)\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { pick(True) + pick(False) } with { T.t() -> resume(2)  return(r) -> r }\n\
         }\n",
        "23",
    ),
    (
        "upcast-through-a-return-binder",
        "effect T { fn t() -> Int }\n\
         fn mk() { fn(x) { x + 1 } }\n\
         fn h(c) {\n\
         \x20 let g = handle { mk() } with { T.t() -> resume(0)  return(x) -> if c { x } else { fn(s) { t() + s } } }\n\
         \x20 g(1)\n\
         }\n\
         pub fn main() -> Int { handle { h(True) + 100 + h(False) * 1000 } with { T.t() -> resume(10)  return(r) -> r } }\n",
        "11102",
    ),
    (
        "a-direct-closure-passed-for-an-effectful-parameter",
        "effect T { fn t() -> Int }\n\
         fn h(f: fn(Int) / {T} -> Int) -> Int { f(1) + f(2) * 10 }\n\
         pub fn main() -> Int {\n\
         \x20 let p = fn(x) { x + 4 }\n\
         \x20 handle { h(p) + h(fn(x) { t() * x }) * 100 } with { T.t() -> resume(3)  return(r) -> r }\n\
         }\n",
        "6365",
    ),
    (
        "upcast-of-a-capturing-closure-saved-across-a-site",
        "effect T { fn t() -> Int }\n\
         fn h(c, n) {\n\
         \x20 let f = fn(x) { x + n }\n\
         \x20 let g = if c { f } else { fn(x) { t() + x } }\n\
         \x20 let a = g(10)\n\
         \x20 a + g(20) * 100\n\
         }\n\
         pub fn main() -> Int { handle { h(True, 1) + h(False, 2) * 100000 } with { T.t() -> resume(7)  return(r) -> r } }\n",
        "271702111",
    ),
    (
        "upcast-in-a-result-layer",
        "effect T { fn t() -> Int }\n\
         fn h(c) {\n\
         \x20 let f = fn() { fn(x) { x + 1 } }\n\
         \x20 let g = if c { f } else { fn() { fn(x) { t() + x } } }\n\
         \x20 let k = g()\n\
         \x20 k(1) * 3\n\
         }\n\
         pub fn main() -> Int { handle { h(True) + h(False) * 100 } with { T.t() -> resume(10)  return(r) -> r } }\n",
        "3306",
    ),
    (
        // 2026-10-09: a lambda relaying an ENCLOSING parameter keeps its row
        // open (it forced `f` pure, E0423); `wrap` is then effect-polymorphic
        // in its result and gets a clone at {L}.
        "a-returned-lambda-relays-an-enclosing-parameter",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn wrap(f: fn(Int) -> Int) { fn(x: Int) { f(x) + 1 } }\n\
         pub fn main() -> Int {\n\
         \x20 let a = wrap(fn(x) { x * 2 })\n\
         \x20 let b = handle { let w = wrap(fn(x) { lg(x) })  w(3) + w(4) } with { L.lg(x) -> resume(x * 10)  return(r) -> r }\n\
         \x20 a(5) * 1000 + b\n\
         }\n",
        "11072",
    ),
];

#[test]
fn the_conventions_corpus_compiles_and_runs() {
    run_value_corpus(CONVENTIONS, "conventions");
}

#[test]
fn native_output_matches_the_evaluator_across_the_conventions_corpus() {
    run_differential_corpus(CONVENTIONS, "conventions-diff");
}

/// The independent review of N7 part 1. Local clones were bound INSIDE the
/// scope of the binder they copy, so a free occurrence of the same name in the
/// lambda (an outer `let`, parameter, clause parameter, return binder or
/// top-level function) captured the generic lambda itself: compiler panics,
/// a segfault, "match failed", and 1007 for 12. And an upcast in a callee's
/// RESULT (`if c { f() } else { .. }`, pre-existing): 11 for 1511.
const CONVENTIONS_REVIEW: &[(&str, &str, &str)] = &[
    (
        "clone-captures-an-outer-int-of-the-same-name",
        "effect S { fn get() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 let x = 10\n\
         \x20 let x = fn(g) { g() + x }\n\
         \x20 handle { x(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n",
        "17",
    ),
    (
        "clone-captures-an-outer-adt-of-the-same-name",
        "effect S { fn get() -> Int }\n\
         type Opt { None, Some(Int) }\n\
         pub fn main() -> Int {\n\
         \x20 let x = Some(5)\n\
         \x20 let x = fn(g) { g() + match x { Some(n) -> n  _ -> 1000 } }\n\
         \x20 handle { x(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n",
        "12",
    ),
    (
        "clone-captures-an-outer-closure-of-the-same-name",
        "effect S { fn get() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 let x = fn(y) { y * 3 }\n\
         \x20 let x = fn(g) { g() + x(4) }\n\
         \x20 handle { x(fn() { get() }) + x(fn() { 1 }) * 100 } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n",
        "1319",
    ),
    (
        "clone-captures-a-top-level-function-of-the-same-name",
        "effect S { fn get() -> Int }\n\
         fn scale(n: Int) -> Int { n * 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let scale = fn(g) { g() + scale(2) }\n\
         \x20 handle { scale(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n",
        "2007",
    ),
    (
        "clone-captures-a-return-binder-of-the-same-name",
        "effect S { fn get() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 handle { 5 } with {\n\
         \x20   S.get() -> resume(1)\n\
         \x20   return(v) -> {\n\
         \x20     let v = fn(g) { g() + v }\n\
         \x20     handle { v(fn() { get() }) } with { S.get() -> resume(70)  return(r) -> r }\n\
         \x20   }\n\
         \x20 }\n\
         }\n",
        "75",
    ),
    (
        "clone-captures-a-clause-parameter-of-the-same-name",
        "effect S { fn get() -> Int }\n\
         effect K { fn kk(x: Int) -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 handle { kk(5) } with {\n\
         \x20   K.kk(x) -> {\n\
         \x20     let x = fn(g) { g() + x }\n\
         \x20     resume(handle { x(fn() { get() }) } with { S.get() -> resume(70)  return(r) -> r })\n\
         \x20   }\n\
         \x20   return(r) -> r\n\
         \x20 }\n\
         }\n",
        "75",
    ),
    (
        "clone-captures-a-parameter-of-the-same-name",
        "effect S { fn get() -> Int }\n\
         fn h(app) {\n\
         \x20 let app = fn(g) { g() + app }\n\
         \x20 handle { app(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n\
         pub fn main() -> Int { h(3) }\n",
        "10",
    ),
    (
        "clone-of-a-clone-of-the-same-name",
        "effect S { fn get() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 let k = 5\n\
         \x20 let k = fn(g) { g() + k }\n\
         \x20 let k = fn(g) { k(g) * 10 }\n\
         \x20 handle { k(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n",
        "120",
    ),
    (
        "result-upcast-at-a-callee",
        "effect T { fn t() -> Int }\n\
         fn h(c) {\n\
         \x20 let f = fn() { fn(x) { x + 1 } }\n\
         \x20 let k = if c { f() } else { fn(x) { t() + x } }\n\
         \x20 k(10)\n\
         }\n\
         pub fn main() -> Int { handle { h(True) + h(False) * 100 } with { T.t() -> resume(5)  return(r) -> r } }\n",
        "1511",
    ),
    (
        "result-upcast-at-a-two-parameter-callee",
        "effect T { fn t() -> Int }\n\
         fn h(c) {\n\
         \x20 let f = fn(a, b) { fn(x) { x + a + b } }\n\
         \x20 let k = if c { f(1, 2) } else { fn(x) { t() + x } }\n\
         \x20 k(10) + k(20)\n\
         }\n\
         pub fn main() -> Int { handle { h(True) + h(False) * 1000 } with { T.t() -> resume(5)  return(r) -> r } }\n",
        "40036",
    ),
    (
        "result-upcast-with-an-annotated-parameter",
        "effect S { fn get() -> Int }\n\
         fn run(c) {\n\
         \x20 let mk = fn(n) { fn(g: fn() / {} -> Int) { g() + n } }\n\
         \x20 let k = if c { mk(1) } else { fn(g: fn() / {} -> Int) { get() + g() } }\n\
         \x20 k(fn() { 5 })\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { run(True) + run(False) * 100 } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n",
        "1206",
    ),
];

#[test]
fn the_conventions_review_corpus_matches_the_evaluator() {
    run_value_corpus(CONVENTIONS_REVIEW, "conventions-review");
    run_differential_corpus(CONVENTIONS_REVIEW, "conventions-review-diff");
}

/// The review's finding 3 (pre-existing): an ALIAS of a generic local
/// (`let mk2 = mk`) gets no clone, and the result of `mk2(1)` was a direct
/// closure called with the CPS convention -- natively it hung where the
/// evaluator printed 8. The deep callee guard refuses it by name: the
/// result layer is an instantiation of `mk`'s open row at {S}.
///
/// Since 2026-10-22 `anf` substitutes the alias (spec
/// 2026-10-22-elya-one-shot-words-and-sweep-gaps D3), so that program runs
/// (the `ALIASES` row w1). A changed test, by that decision: the alias is kept
/// only when its body rebinds `mk`, which still meets the guard, so this
/// program rebinds it.
#[test]
fn an_alias_of_a_generic_local_used_at_a_user_effect_is_refused_by_name() {
    let src = "effect S { fn get() -> Int }\n\
               pub fn main() -> Int {\n\
               \x20 let mk = fn(n) { fn(g) { g() + n } }\n\
               \x20 let mk2 = mk\n\
               \x20 let k = { let mk = 0  mk2(1 + mk) }\n\
               \x20 handle { k(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
               }\n";
    assert_eq!(eval_main_int(src), "8");
    let dir = temp_dir("generic-alias");
    let err = try_compile_and_link(&lower_src(src), &dir, "generic-alias").unwrap_err();
    std::fs::remove_dir_all(&dir).ok();
    assert!(
        err.contains("effect-polymorphic function used at a user effect"),
        "{err}"
    );
}

/// Async step 1 (spec `2026-10-09-elya-async-step1-design.md`): function
/// types in declarations. A closure in a constructor field or an operation
/// argument, generator-style tasks built by a handler that stores `resume` in a
/// field, and a round-robin scheduler. `an-upcast-of-a-pattern-binder` was a
/// miscompile while pattern binders were untracked (4 for 304).
const FN_FIELDS: &[(&str, &str, &str)] = &[
    (
        "a-closure-in-a-field",
        "type B { B(fn(Int) -> Int) }\n\
         pub fn main() -> Int { let f = fn(x) { x + 1 }  match B(f) { B(g) -> g(41) } }\n",
        "42",
    ),
    (
        "a-written-row-in-a-field",
        "effect L { fn lg(x: Int) -> Int }\n\
         type B { B(fn(Int) / {L} -> Int) }\n\
         fn call(b) { match b { B(g) -> g(5) } }\n\
         pub fn main() -> Int { handle { call(B(fn(x) { lg(x) + 1 })) + call(B(fn(x) { x })) } with { L.lg(x) -> resume(x * 100)  return(r) -> r } }\n",
        "506",
    ),
    (
        "a-direct-closure-into-an-effectful-field",
        "effect L { fn lg(x: Int) -> Int }\n\
         type B { B(fn(Int) / {L} -> Int) }\n\
         fn call(b) { match b { B(g) -> g(5) } }\n\
         pub fn main() -> Int { let p = fn(x) { x + 7 }  handle { call(B(p)) } with { L.lg(x) -> resume(x * 100)  return(r) -> r } }\n",
        "12",
    ),
    (
        "an-upcast-of-a-pattern-binder",
        "effect L { fn lg(x: Int) -> Int }\n\
         type P { P(fn(Int) -> Int) }\n\
         fn h(c, b) { match b { P(g) -> { let k = if c { g } else { fn(x) { lg(x) } }  k(3) } } }\n\
         pub fn main() -> Int { handle { h(True, P(fn(x) { x + 1 })) + h(False, P(fn(x) { x })) } with { L.lg(x) -> resume(x * 100)  return(r) -> r } }\n",
        "304",
    ),
    (
        "an-operation-takes-a-function",
        "effect Ap { fn ap(f: fn(Int) -> Int, x: Int) -> Int }\n\
         pub fn main() -> Int { handle { ap(fn(x) { x * 2 }, 20) + 2 } with { Ap.ap(f, x) -> resume(f(x))  return(r) -> r } }\n",
        "42",
    ),
    (
        "a-generator-task",
        "effect Yield { fn yld() -> Unit }\n\
         type Task { Done(Int), Paused(fn() -> Task) }\n\
         fn count(n: Int, acc: Int) -> Int { if n == 0 { acc } else { let _ = yld()  count(n - 1, acc + n) } }\n\
         fn spawn(n: Int) -> Task { handle { count(n, 0) } with { Yield.yld() -> Paused(fn() { resume(Unit) })  return(x) -> Done(x) } }\n\
         fn finish(t: Task) -> Int { match t { Done(x) -> x  Paused(k) -> finish(k()) } }\n\
         fn run2(a: Task, b: Task) -> Int { match a { Done(x) -> x * 1000 + finish(b)  Paused(k) -> run2(b, k()) } }\n\
         pub fn main() -> Int { run2(spawn(3), spawn(5)) }\n",
        "6015",
    ),
    (
        "two-tasks-interleave-their-log",
        "effect Yield { fn yld() -> Unit }\n\
         effect Log { fn emit(d: Int) -> Unit }\n\
         type Task { Done(Int), Paused(fn() / {Log} -> Task) }\n\
         fn work(id: Int, n: Int) -> Int { if n == 0 { id } else { let _ = emit(id)  let _ = yld()  work(id, n - 1) } }\n\
         fn spawn(id: Int, n: Int) -> Task { handle { work(id, n) } with { Yield.yld() -> Paused(fn() { resume(Unit) })  return(x) -> Done(x) } }\n\
         fn finish(t: Task) -> Int { match t { Done(x) -> x  Paused(k) -> finish(k()) } }\n\
         fn run2(a: Task, b: Task) -> Int { match a { Done(x) -> x + finish(b)  Paused(k) -> run2(b, k()) } }\n\
         pub fn main() -> Int {\n\
         \x20 let f = handle { run2(spawn(1, 3), spawn(2, 2)) } with {\n\
         \x20   Log.emit(d) -> fn(acc) { (resume(Unit))(acc * 10 + d) }\n\
         \x20   return(x) -> fn(acc) { acc * 10 + x }\n\
         \x20 }\n\
         \x20 f(0)\n\
         }\n",
        "121213",
    ),
    (
        "fork-and-yield",
        "effect Async {\n\
         \x20 fn fork(f: fn() / {Async, Log} -> Unit) -> Unit\n\
         \x20 fn yld() -> Unit\n\
         }\n\
         effect Log { fn emit(d: Int) -> Unit }\n\
         \n\
         type Task { Done, Paused(fn() / {Log} -> Task), Forked(fn() / {Async, Log} -> Unit, fn() / {Log} -> Task) }\n\
         type Queue { Empty, Push(Task, Queue) }\n\
         \n\
         fn task(body: fn() / {Async, Log} -> Unit) -> Task {\n\
         \x20 handle { body() } with {\n\
         \x20   Async.yld() -> Paused(fn() { resume(Unit) })\n\
         \x20   Async.fork(f) -> Forked(f, fn() { resume(Unit) })\n\
         \x20   return(u) -> Done\n\
         \x20 }\n\
         }\n\
         \n\
         fn append(q: Queue, t: Task) -> Queue { match q { Empty -> Push(t, Empty)  Push(h, r) -> Push(h, append(r, t)) } }\n\
         \n\
         fn run(q: Queue) -> Int {\n\
         \x20 match q {\n\
         \x20   Empty -> 0\n\
         \x20   Push(t, rest) -> match t {\n\
         \x20     Done -> 1 + run(rest)\n\
         \x20     Paused(k) -> run(append(rest, k()))\n\
         \x20     Forked(child, parent) -> run(append(append(rest, parent()), task(child)))\n\
         \x20   }\n\
         \x20 }\n\
         }\n\
         \n\
         fn each(f: fn(Int) -> Unit, n: Int) -> Unit { if n == 0 { Unit } else { let _ = f(n)  each(f, n - 1) } }\n\
         \n\
         fn worker(id: Int) -> Unit { each(fn(i) { let _ = emit(id)  yld() }, 3) }\n\
         \n\
         pub fn main() -> Int {\n\
         \x20 let pure_sum = each(fn(i) { Unit }, 5)\n\
         \x20 let f = handle {\n\
         \x20   run(Push(task(fn() { let _ = fork(fn() { worker(2) })  worker(1) }), Empty))\n\
         \x20 } with {\n\
         \x20   Log.emit(d) -> fn(acc) { (resume(Unit))(acc * 10 + d) }\n\
         \x20   return(x) -> fn(acc) { acc * 10 + x }\n\
         \x20 }\n\
         \x20 f(0)\n\
         }\n",
        "1212122",
    ),
];

#[test]
fn the_fn_fields_corpus_matches_the_evaluator() {
    run_value_corpus(FN_FIELDS, "fn-fields");
    run_differential_corpus(FN_FIELDS, "fn-fields-diff");
}

#[test]
fn the_async_example_runs_natively() {
    // `examples/04_async.elya`: a scheduler written as an Elya handler, the
    // same bytes printed natively as by the evaluator, and the same count.
    let src = include_str!("../../../examples/04_async.elya");
    let dir = temp_dir("async-example");
    let exe = compile_and_link(&lower_src(src), &dir, "async");
    let (text, value) = native_text_value(&exe, "async");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(text, eval_main_text(src));
    assert!(
        text.starts_with("main: start\n  ping\nmain\n    pong\n"),
        "{text}"
    );
    assert_eq!(value, "3");
    assert_eq!(value, eval_main_int(src));
}

/// Native multi-shot handlers (2026-10-10, spec
/// `2026-10-10-elya-native-multi-shot-design.md`, §0's table). A `with multi`
/// resume re-enters a COPY of its captured frames (D2): m3 and m10 are the
/// shapes where the first run mutates a handler frame inside the captured
/// segment (an inner clause's non-tail resume rewrites its frame's `next`;
/// m10 also crosses a `parent` link), so running the chain itself twice is
/// wrong there -- the controls K1 (no copy: m3 prints 40200) and K2 (no
/// `parent` remap: m10 prints 10024, m3 still right) fail them.
const MULTI_SHOT: &[(&str, &str, &str)] = &[
    (
        "m1-two-branches",
        "effect multi Flip { fn flip() -> Bool }\n\
         fn choose() { if flip() { 1 } else { 2 } }\n\
         pub fn main() -> Int { handle choose() with multi { Flip.flip() -> resume(True) * 10 + resume(False) } }\n",
        "12",
    ),
    (
        "m2-two-flips",
        "effect multi Flip { fn flip() -> Bool }\n\
         fn choose() {\n\
         \x20 let a = if flip() { 1 } else { 0 }\n\
         \x20 let b = if flip() { 2 } else { 0 }\n\
         \x20 a * 10 + b\n\
         }\n\
         pub fn main() -> Int { handle choose() with multi { Flip.flip() -> resume(True) * 1000 + resume(False) } }\n",
        "12012000",
    ),
    (
        "m3-inner-handler-frame",
        "effect multi Flip { fn flip() -> Bool }\n\
         effect Ask { fn ask() -> Int }\n\
         fn inner() {\n\
         \x20 handle {\n\
         \x20   let x = ask()\n\
         \x20   let b = flip()\n\
         \x20   let y = ask()\n\
         \x20   if b { x + y } else { x * y }\n\
         \x20 } with { Ask.ask() -> resume(5) * 2 }\n\
         }\n\
         pub fn main() -> Int { handle inner() with multi { Flip.flip() -> resume(True) * 1000 + resume(False) } }\n",
        "40100",
    ),
    (
        "m4-backtracking",
        "effect multi Flip { fn flip() -> Bool }\n\
         fn count(n, acc) { if n == 0 { if acc == 7 { 1 } else { 0 } } else { if flip() { count(n - 1, acc + n) } else { count(n - 1, acc) } } }\n\
         pub fn main() -> Int { handle count(12, 0) with multi { Flip.flip() -> resume(True) + resume(False) } }\n",
        "5",
    ),
    (
        "m7-resume-in-a-lambda",
        "effect multi Flip { fn flip() -> Bool }\n\
         fn choose() { if flip() { 1 } else { 2 } }\n\
         pub fn main() -> Int { handle choose() with multi { Flip.flip() -> { let k = fn(b) { resume(b) }  k(True) * 10 + k(False) } } }\n",
        "12",
    ),
    (
        "m8-nested-multi",
        "effect multi Flip { fn flip() -> Bool }\n\
         effect multi Pick { fn pick() -> Int }\n\
         fn body() { let b = flip()  let n = pick()  if b { n } else { 0 - n } }\n\
         fn mid() { handle body() with multi { Pick.pick() -> resume(1) * 100 + resume(2) } }\n\
         pub fn main() -> Int { handle mid() with multi { Flip.flip() -> resume(True) * 100000 + resume(False) } }\n",
        "10199898",
    ),
    (
        "m10-parent-in-the-segment",
        "effect multi Flip { fn flip() -> Bool }\n\
         effect Ask { fn ask() -> Int }\n\
         effect Tell { fn tell(v: Int) -> Unit }\n\
         fn inner() {\n\
         \x20 handle {\n\
         \x20   handle {\n\
         \x20     let b = flip()\n\
         \x20     let y = ask()\n\
         \x20     let _ = tell(y)\n\
         \x20     if b { y } else { y + 1 }\n\
         \x20   } with { Tell.tell(v) -> resume(Unit) }\n\
         \x20 } with { Ask.ask() -> resume(5) * 2 }\n\
         }\n\
         pub fn main() -> Int { handle inner() with multi { Flip.flip() -> resume(True) * 1000 + resume(False) } }\n",
        "10012",
    ),
    (
        "m11-deep-chain",
        "effect multi Flip { fn flip() -> Bool }\n\
         fn sum(n) { if n == 0 { if flip() { 1 } else { 2 } } else { n + sum(n - 1) } }\n\
         fn go(i, acc) { if i == 0 { acc } else { go(i - 1, acc + handle sum(300) with multi { Flip.flip() -> resume(True) + resume(False) }) } }\n\
         pub fn main() -> Int { go(40, 0) }\n",
        "3612120",
    ),
    (
        "m12-many-runs",
        "effect multi Flip { fn flip() -> Bool }\n\
         fn count(n, acc) { if n == 0 { acc } else { if flip() { count(n - 1, acc + 1) } else { count(n - 1, acc) } } }\n\
         fn go(i, acc) { if i == 0 { acc } else { go(i - 1, acc + handle count(8, 0) with multi { Flip.flip() -> resume(True) + resume(False) }) } }\n\
         pub fn main() -> Int { go(1000, 0) }\n",
        "1024000",
    ),
    (
        // The shape the multi-shot review could not build (2026-10-15, unblocked by
        // recursion through a handle body): the captured chain holds 1000 handler
        // frames, and the copy maps every frame's `parent`.
        "m-deep-handler-chain",
        "effect multi Flip { fn flip() -> Bool }\neffect S { fn get() -> Int }\n\
         fn nest(n) { if n == 0 { if flip() { 1 } else { 2 } } else { handle { nest(n - 1) + get() } with { S.get() -> resume(1)  return(r) -> r } } }\n\
         pub fn main() -> Int { handle nest(1000) with multi { Flip.flip() -> resume(True) * 1000000 + resume(False) } }\n",
        "1001001002",
    ),
    (
        // Spec 2026-10-22-elya-multi-shot-sharing D2: a copy is the handler path,
        // so this is the shape whose copies are long -- 301 handler frames, copied
        // twice per run, 100 runs. The mid-copy collection test runs it (m11's
        // copies are one frame since D2, and it no longer collects at all).
        "m13-handler-chain-loop",
        "effect multi Flip { fn flip() -> Bool }\neffect S { fn get() -> Int }\n\
         fn nest(n) { if n == 0 { if flip() { 1 } else { 2 } } else { handle { nest(n - 1) + get() } with { S.get() -> resume(1)  return(r) -> r } } }\n\
         fn go(i, acc) { if i == 0 { acc } else { go(i - 1, acc + handle nest(300) with multi { Flip.flip() -> resume(True) + resume(False) }) } }\n\
         pub fn main() -> Int { go(100, 0) }\n",
        "60300",
    ),
];

#[test]
fn the_multi_shot_corpus_prints_the_evaluators_values() {
    run_value_corpus(MULTI_SHOT, "multi-shot");
}

#[test]
fn native_output_matches_the_evaluator_across_the_multi_shot_corpus() {
    run_differential_corpus(MULTI_SHOT, "multi-shot-diff");
}

/// Spec D1: re-entry keys on the HANDLER's `with multi`. A plain `with` over a
/// `multi`-declared effect is one-shot natively, as in the evaluator: it
/// compiles (5b-8 refused it by the declaration) and a second resume traps.
#[test]
fn a_plain_handler_over_a_multi_effect_is_one_shot_natively() {
    let dir = temp_dir("multi-plain");
    let once = "effect multi Flip { fn flip() -> Bool }\n\
                fn choose() { if flip() { 1 } else { 2 } }\n\
                pub fn main() -> Int { handle choose() with { Flip.flip() -> resume(True) } }\n";
    let exe = compile_and_link(&lower_src(once), &dir, "once");
    assert_runs(&exe, "1");
    let twice = "effect multi Flip { fn flip() -> Bool }\n\
                 fn choose() { if flip() { 1 } else { 2 } }\n\
                 pub fn main() -> Int { handle choose() with { Flip.flip() -> resume(True) + resume(False) } }\n";
    let session = Session::new();
    let (m, pd) = parse_module(&session, twice);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let eval = elya::eval::run_module_value(&m);
    assert!(
        matches!(&eval, Err(e) if format!("{e:?}").contains("E0425")),
        "the evaluator must refuse the second resume with E0425"
    );
    let exe = compile_and_link(&lower_src(twice), &dir, "twice");
    let out = native(&exe).output().expect("run produced binary");
    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("elya: a one-shot continuation was resumed twice"),
        "stderr should name the trap, got: {stderr}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Spec D4: copies are shallow -- a ONE-SHOT continuation saved in a
/// multi-shot segment is shared by both runs, so the second run's resume of it
/// fails: E0425 in the evaluator, the named trap natively.
#[test]
fn a_one_shot_continuation_saved_in_a_multi_shot_segment_fails_on_both_sides() {
    let src = "effect multi Flip { fn flip() -> Bool }\n\
               effect Ask { fn ask() -> Int }\n\
               fn inner() {\n\
               \x20 handle { let x = ask()  x + 1 } with { Ask.ask() -> if flip() { resume(5) } else { resume(6) } }\n\
               }\n\
               pub fn main() -> Int { handle inner() with multi { Flip.flip() -> resume(True) * 1000 + resume(False) } }\n";
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let eval = elya::eval::run_module_value(&m);
    assert!(
        matches!(&eval, Err(e) if format!("{e:?}").contains("E0425")),
        "the evaluator must refuse the second resume with E0425"
    );
    let dir = temp_dir("multi-shared-one-shot");
    let exe = compile_and_link(&lower_src(src), &dir, "shared");
    let out = native(&exe).output().expect("run produced binary");
    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("elya: a one-shot continuation was resumed twice"),
        "stderr should name the trap, got: {stderr}"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).is_empty(),
        "no answer may be printed"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Spec D5 (Invariant N8-1): copies are heap frames, so a multi-shot loop's
/// live set stays bounded across N -- a copy that stayed reachable would grow
/// it by at least a word per iteration.
///
/// Unlike A4's tail loop, the level is NOT equal across N: the last collection
/// lands at a different point of the 64-run search each time, with a different
/// number of copies pending (measured: 84, 80, 80, 89 words). So the bound is
/// read off N itself -- every level below the smallest N, which a leak of one
/// word per iteration would exceed -- and a growing control (the same loop
/// keeping one `Cons` per iteration) must break that bound, so the instrument
/// is shown to see growth.
#[test]
fn a_multi_shot_loops_live_set_stays_bounded() {
    let dir = temp_dir("multi-settles");
    let ns = [2_500i64, 5_000, 10_000, 20_000];
    let run = |n: i64, keep: bool| -> GcStats {
        let src = if keep {
            format!(
                "type L {{ Nil, Cons(Int, L) }}\n\
                 effect multi Flip {{ fn flip() -> Bool }}\n\
                 fn count(n, acc) {{ if n == 0 {{ acc }} else {{ if flip() {{ count(n - 1, acc + 1) }} else {{ count(n - 1, acc) }} }} }}\n\
                 fn sum(l) {{ match l {{ Nil -> 0  Cons(x, r) -> x + sum(r) }} }}\n\
                 fn go(i, acc) {{ if i == 0 {{ sum(acc) }} else {{ go(i - 1, Cons(handle count(6, 0) with multi {{ Flip.flip() -> resume(True) + resume(False) }}, acc)) }} }}\n\
                 pub fn main() -> Int {{ go({n}, Nil) }}\n"
            )
        } else {
            format!(
                "effect multi Flip {{ fn flip() -> Bool }}\n\
                 fn count(n, acc) {{ if n == 0 {{ acc }} else {{ if flip() {{ count(n - 1, acc + 1) }} else {{ count(n - 1, acc) }} }} }}\n\
                 fn go(i, acc) {{ if i == 0 {{ acc }} else {{ go(i - 1, acc + handle count(6, 0) with multi {{ Flip.flip() -> resume(True) + resume(False) }}) }} }}\n\
                 pub fn main() -> Int {{ go({n}, 0) }}\n"
            )
        };
        let tag = format!("multi-settles-{n}-{keep}");
        let exe = compile_and_link(&lower_src(&src), &dir, &tag);
        let (stdout, stats) = run_with_gc_stats(&exe, &tag);
        assert_eq!(stdout, (192 * n).to_string(), "{tag}");
        assert!(
            stats.collections > 0,
            "{tag}: no collection happened, so `live` measures nothing"
        );
        assert!(
            stats.live > 0,
            "{tag}: live=0 means the level was never computed"
        );
        stats
    };
    let levels: Vec<(i64, i64)> = ns.iter().map(|&n| (n, run(n, false).live)).collect();
    println!("multi-shot levels: {levels:?}");
    for (n, live) in &levels {
        assert!(
            *live < ns[0],
            "live set must not grow with N (a level of N words or more is a copy leak): {levels:?}, at N={n}"
        );
    }
    let control = run(ns[3], true).live;
    println!("growing control at N={}: {control}", ns[3]);
    assert!(
        control >= ns[0],
        "the growing control did not break the bound, so the instrument sees nothing: {control}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Spec D3: the copy allocates, so a collection can run in the middle of one.
/// m13 copies 301-frame handler paths 200 times and must collect; the answer
/// must still be the evaluator's. (Until 2026-10-22 this ran m11, whose
/// 300-frame chain was copied whole; since multi-shot sharing D2 m11's copies
/// are one frame and it never collects.)
#[test]
fn copies_survive_a_collection_mid_copy() {
    let (_, src, want) = MULTI_SHOT
        .iter()
        .find(|(t, _, _)| *t == "m13-handler-chain-loop")
        .expect("m13 in the corpus");
    let dir = temp_dir("multi-gc");
    let exe = compile_and_link(&lower_src(src), &dir, "multi-gc");
    let (stdout, stats) = run_with_gc_stats(&exe, "multi-gc");
    assert!(stats.collections > 0, "no collection ran: {stats:?}");
    // The claim itself, not only a collection somewhere (the review's note 1):
    // measured 6 of m13's 16 collections inside a copy.
    let mid = stats
        .mid_copy
        .unwrap_or_else(|| panic!("the runtime reports no `mid_copy=`: {stats:?}"));
    assert!(mid > 0, "no collection ran inside a copy: {stats:?}");
    assert_eq!(&stdout, want);
    std::fs::remove_dir_all(&dir).ok();
}

/// PARKED.md's program (2026-10-16): a `with multi` clause that resumes NON-tail,
/// over a non-tail recursion that performs at every level. Spec
/// `2026-10-22-elya-multi-shot-sharing-design.md`.
fn deep_multi_src(n: i64) -> String {
    format!(
        "effect multi Flip {{ fn flip() -> Bool }}\n\
         fn nest(n) {{ if n == 0 {{ 0 }} else {{ let b = flip()  if b {{ 1 + nest(n - 1) }} else {{ nest(n - 1) }} }} }}\n\
         pub fn main() -> Int {{ handle nest({n}) with multi {{ Flip.flip() -> 0 + resume(True) }} }}\n"
    )
}

/// Spec D2: a multi-shot resume shares the captured chain and copies only the
/// handler frames on its path, so the PARKED program's live set is linear in
/// its depth. Copying the whole chain kept a copy of `i` frames alive at level
/// `i` (§0: 65 126 164 words at N = 8 000, x4 per doubling).
#[test]
fn a_deep_multi_shot_chain_is_shared_not_copied() {
    let dir = temp_dir("multi-shared-chain");
    for n in [8_000i64, 16_000] {
        let tag = format!("multi-shared-{n}");
        let exe = compile_and_link(&lower_src(&deep_multi_src(n)), &dir, &tag);
        let (stdout, stats) = run_with_gc_stats(&exe, &tag);
        assert_eq!(stdout, n.to_string(), "{tag}");
        assert!(stats.collections > 0, "{tag}: no collection ran: {stats:?}");
        assert!(
            stats.live < 64 * n,
            "{tag}: the live set must be linear in the depth (under 64 words a level): {stats:?}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Spec D2 under a CPS `with multi` handle (it leaks `S`, so its resume is a
/// `musttail` jump, not a nesting call). Nothing keeps the originals alive here,
/// so the old copy showed as garbage, not as live words: blocks freed grew x4
/// per doubling (§0's CPS row: 8 017 989 at N = 4 000).
#[test]
fn a_deep_cps_multi_shot_chain_is_shared_not_copied() {
    let dir = temp_dir("multi-shared-cps");
    for n in [2_000i64, 4_000] {
        let src = format!(
            "effect multi Flip {{ fn flip() -> Bool }}\n\
             effect S {{ fn get() -> Int }}\n\
             fn nest(n) {{ if n == 0 {{ 0 }} else {{ let b = flip()  if b {{ get() + nest(n - 1) }} else {{ nest(n - 1) }} }} }}\n\
             fn run(n) {{ handle nest(n) with multi {{ Flip.flip() -> 0 + resume(True) }} }}\n\
             pub fn main() -> Int {{ handle run({n}) with {{ S.get() -> resume(1) }} }}\n"
        );
        let tag = format!("multi-shared-cps-{n}");
        let exe = compile_and_link(&lower_src(&src), &dir, &tag);
        let (stdout, stats) = run_with_gc_stats(&exe, &tag);
        assert_eq!(stdout, n.to_string(), "{tag}");
        assert!(
            stats.freed < 64 * n,
            "{tag}: the blocks a run frees must be linear in the depth (under 64 a level): {stats:?}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// PARKED.md's program at the depth it was found at: `elya: out of memory`
/// natively (§0, under a 4 GiB address-space limit), 0.29 s in the evaluator.
/// On the program's default stack, like the `DEEP` tests.
#[test]
fn the_parked_deep_multi_shot_program_runs_natively() {
    let src = deep_multi_src(20_000);
    let session = Session::new();
    let (m, pd) = parse_module(&session, &src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (_, eval) = elya::eval::run_module_value(&m).expect("the evaluator runs it");
    assert_eq!(format!("{eval:?}"), "Int(20000)");
    let dir = temp_dir("multi-parked");
    let exe = compile_and_link(&lower_src(&src), &dir, "parked");
    let out = Command::new(&exe).output().expect("run produced binary");
    assert!(
        out.status.success(),
        "{:?}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "20000\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// Spec D3: a handle's body no longer reaches its handler frame through
/// `next` (D1), so a frame that is current and saved nowhere else must be kept
/// by the collector's root for the current handler. Each body allocates 6-word
/// blocks, the size of a handler frame with one saved binding, so a freed frame
/// is reused under its own body and its return clause would run garbage. A
/// direct handle and a CPS one (it leaks `S`). A guard: before D1 the chain kept
/// the frame alive, so its red is control K4 (no root).
#[test]
fn a_handler_frame_reachable_only_as_current_survives_collections() {
    let dir = temp_dir("current-handler-root");
    let churn = "type B { B(Int, Int, Int, Int, Int) }\n\
                 effect E { fn e() -> Int }\n\
                 effect S { fn get() -> Int }\n\
                 fn churn(n, acc) { if n == 0 { acc } else { match B(n, n, n, n, n) { B(a, _, _, _, _) -> churn(n - 1, acc + a % 2) } } }\n";
    for (tag, main, want) in [
        (
            "direct",
            "fn work(s) { handle { churn(300000, 0) } with { E.e() -> resume(0)  return(r) -> r + s } }\n\
             pub fn main() -> Int { work(7) }\n",
            "150007",
        ),
        (
            "cps",
            "fn work(s) { handle { let x = get()  churn(300000, x) } with { E.e() -> resume(0)  return(r) -> r + s } }\n\
             pub fn main() -> Int { handle work(7) with { S.get() -> resume(1) } }\n",
            "150008",
        ),
    ] {
        let src = format!("{churn}{main}");
        let exe = compile_and_link(&lower_src(&src), &dir, tag);
        let (stdout, stats) = run_with_gc_stats(&exe, tag);
        assert!(stats.collections > 0, "{tag}: no collection ran: {stats:?}");
        assert_eq!(stdout, want, "{tag}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// `Int` arithmetic is exact or fails by name (2026-10-10, spec
/// `2026-10-10-elya-checked-integer-arithmetic-design.md`). `/` and `%` compile
/// natively; each value agrees with the evaluator. `ix` and `ex` take their
/// operands from a perform, so `/` and `%` also run through the CPS emitter.
/// Control K2 (no `-1` select in `%`): `min-rem-minus-one` dies on SIGFPE.
const INT_DIVISION: &[(&str, &str, &str)] = &[
    (
        "signs",
        "fn d(a, b) { a / b * 1000 + a % b }\n\
         pub fn main() -> Int { d(0 - 7, 2) * 1000000 + d(7, 0 - 2) * 1000 + d(0 - 7, 0 - 2) }\n",
        "-3003996001",
    ),
    (
        "min-rem-minus-one",
        "fn r(a, b) { a % b }\n\
         pub fn main() -> Int { r(0 - 9223372036854775807 - 1, 0 - 1) + 5 }\n",
        "5",
    ),
    (
        "min-div-one",
        "fn d(a, b) { a / b }\n\
         pub fn main() -> Int { d(0 - 9223372036854775807 - 1, 1) / 1000000000000 }\n",
        "-9223372",
    ),
    (
        "through-cps",
        "effect Ix { fn ix() -> Int }\n\
         fn ex(n) {\n let v = n * 100 + ix()\n v / 7 * 10 + v % 7\n }\n\
         pub fn main() -> Int { handle { ex(5) + ex(0 - 5) } with { Ix.ix() -> resume(3) } }\n",
        "6",
    ),
    (
        "lcg",
        "fn next(s) { (s * 1103515245 + 12345) % 2147483648 }\n\
         fn go(s, n) { if n == 0 { s } else { go(next(s), n - 1) } }\n\
         pub fn main() -> Int { go(2026, 1000) }\n",
        "",
    ),
];

#[test]
fn integer_division_runs_natively() {
    let dir = temp_dir("int-division");
    for (tag, src, expected) in INT_DIVISION {
        let exe = compile_and_link(&lower_src(src), &dir, tag);
        let out = native(&exe).output().expect("run produced binary");
        diagnose_crash(&out.status, tag);
        assert!(out.status.success(), "{tag}: exited {:?}", out.status);
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(got, eval_main_int(src), "{tag}: native vs evaluator");
        if !expected.is_empty() {
            assert_eq!(got, *expected, "{tag}");
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Every failing input stops natively with exit 1 and the named message, and
/// the evaluator stops with E0300 and the same words. The operands come
/// through a function so neither side can fold them away early.
/// Control K1 (`add` without the overflow check): `add` prints a wrapped value.
#[test]
fn integer_traps_are_named_on_both_sides() {
    const MIN: &str = "(0 - 9223372036854775807 - 1)";
    const MAX: &str = "9223372036854775807";
    let cases = [
        ("add", format!("f({MAX}, 1, 0)"), "integer overflow"),
        ("sub", format!("f({MIN}, 1, 1)"), "integer overflow"),
        ("mul", format!("f({MAX}, 2, 2)"), "integer overflow"),
        ("div-min", format!("f({MIN}, 0 - 1, 3)"), "integer overflow"),
        ("div-zero", "f(7, 0, 3)".to_string(), "division by zero"),
        ("rem-zero", "f(7, 0, 4)".to_string(), "remainder by zero"),
    ];
    let dir = temp_dir("int-traps");
    for (tag, call, name) in cases {
        let src = format!(
            "fn f(a, b, op) {{ if op == 0 {{ a + b }} else {{ if op == 1 {{ a - b }} else {{\n\
             if op == 2 {{ a * b }} else {{ if op == 3 {{ a / b }} else {{ a % b }} }} }} }} }}\n\
             pub fn main() -> Int {{ {call} }}\n"
        );
        let session = Session::new();
        let (m, pd) = parse_module(&session, &src);
        assert!(pd.is_empty(), "parse: {pd:?}");
        match elya::eval::run_module_value(&m) {
            Err(e) => {
                assert_eq!(e.diag.code, "E0300", "{tag}");
                assert_eq!(e.diag.message, name, "{tag}");
            }
            Ok((_, v)) => panic!("{tag}: the evaluator returned {v:?}"),
        }
        let exe = compile_and_link(&lower_src(&src), &dir, tag);
        let out = native(&exe).output().expect("run produced binary");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{tag}: {:?}, stdout {:?}, stderr {stderr:?}",
            out.status,
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(stderr, format!("elya: {name}\n"), "{tag}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Native pattern compilation (2026-10-10, spec
/// `2026-10-10-elya-native-pattern-compilation-design.md`, §0's table): nested
/// constructor patterns and literal patterns compile through a Core-to-Core pass
/// into the flat matches both emitters already support. Each value is the
/// evaluator's (and pinned where computed by hand).
/// p8 and p9 are the review's: with failure continuations copied they took
/// 9.2 s / 6.4 MB and 11.4 s / 6.3 MB to build (exponential in the arms).
const PATTERNS: &[(&str, &str, &str)] = &[
    (
        "p1-nested-fallthrough",
        "type E { A(Int), B(Int, Int) }\n\
         type L { Nil, Cons(E, L) }\n\
         fn f(l) { match l {\n\
         \x20 Cons(A(x), Cons(B(y, z), _)) -> x * 100 + y * 10 + z\n\
         \x20 Cons(A(x), _) -> x\n\
         \x20 Cons(B(y, z), rest) -> y + z + f(rest)\n\
         \x20 Nil -> 0\n\
         } }\n\
         pub fn main() -> Int { f(Cons(A(1), Cons(B(2, 3), Nil))) * 10000 + f(Cons(A(7), Nil)) * 100 + f(Cons(B(4, 5), Cons(A(6), Nil))) }\n",
        "1230715",
    ),
    (
        "p2-int-and-bool-literals",
        "fn g(n) { match n {\n\
         \x20 0 -> 100\n\
         \x20 1 -> 200\n\
         \x20 k -> k * 3\n\
         } }\n\
         fn h(b) { match b {\n\
         \x20 True -> 1\n\
         \x20 False -> 2\n\
         } }\n\
         pub fn main() -> Int { g(0) + g(1) + g(5) + h(True) * 1000 + h(False) * 10000 }\n",
        "21315",
    ),
    (
        "p3-literal-in-constructor",
        "type O { None, Some(Int) }\n\
         fn f(o) { match o {\n\
         \x20 Some(0) -> 10\n\
         \x20 Some(n) -> n\n\
         \x20 None -> 0 - 1\n\
         } }\n\
         pub fn main() -> Int { f(Some(0)) * 10000 + f(Some(42)) * 10 + f(None) }\n",
        "100419",
    ),
    (
        "p4-outer-name",
        "type P { P(Int, Q) }\n\
         type Q { Q(Int) }\n\
         fn f(p, x) { match p {\n\
         \x20 P(x, Q(1)) -> x\n\
         \x20 P(_, Q(_)) -> x\n\
         } }\n\
         pub fn main() -> Int { f(P(5, Q(1)), 9) * 100 + f(P(5, Q(2)), 9) }\n",
        "509",
    ),
    (
        "p5-effectful-arms",
        "effect Ask { fn ask() -> Int }\n\
         type O { None, Some(Int) }\n\
         type W { W(O) }\n\
         fn f(w) { match w {\n\
         \x20 W(Some(0)) -> ask()\n\
         \x20 W(Some(n)) -> n + ask()\n\
         \x20 W(None) -> ask() * 2\n\
         } }\n\
         pub fn main() -> Int { handle { f(W(Some(0))) * 10000 + f(W(Some(5))) * 100 + f(W(None)) } with { Ask.ask() -> resume(7) } }\n",
        "71214",
    ),
    (
        "p6-pairs-loop",
        "type L { Nil, Cons(Int, L) }\n\
         fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
         fn pairs(l, acc) { match l {\n\
         \x20 Cons(a, Cons(b, rest)) -> pairs(rest, acc + a * b)\n\
         \x20 Cons(a, Nil) -> acc + a\n\
         \x20 Nil -> acc\n\
         } }\n\
         pub fn main() -> Int { pairs(build(100001, Nil), 0) % 1000000007 }\n",
        "",
    ),
    (
        "p7-literal-in-lambda",
        "type O { None, Some(Int) }\n\
         pub fn main() -> Int {\n\
         \x20 let f = fn(o) { match o {\n\
         \x20   Some(1) -> 10\n\
         \x20   Some(m) -> m\n\
         \x20   None -> 0\n\
         \x20 } }\n\
         \x20 f(Some(1)) + f(Some(5)) * 100\n\
         }\n",
        "510",
    ),
    (
        "p8-simplifier",
        "type E { Num(Int), Add(E, E), Mul(E, E), Neg(E) }\n\
         fn simp(e) { match e {\n\
         \x20 Add(Num(0), x) -> x\n\
         \x20 Add(x, Num(0)) -> x\n\
         \x20 Mul(Num(1), x) -> x\n\
         \x20 Mul(x, Num(1)) -> x\n\
         \x20 Mul(Num(0), _) -> Num(0)\n\
         \x20 Mul(_, Num(0)) -> Num(0)\n\
         \x20 Neg(Neg(x)) -> x\n\
         \x20 Neg(Num(n)) -> Num(0 - n)\n\
         \x20 Add(Num(a), Num(b)) -> Num(a + b)\n\
         \x20 Mul(Num(a), Num(b)) -> Num(a * b)\n\
         \x20 other -> other\n\
         } }\n\
         fn ev(e) { match simp(e) {\n\
         \x20 Num(n) -> n\n\
         \x20 Add(a, b) -> ev(a) + ev(b)\n\
         \x20 Mul(a, b) -> ev(a) * ev(b)\n\
         \x20 Neg(a) -> 0 - ev(a)\n\
         } }\n\
         pub fn main() -> Int { ev(Add(Mul(Num(1), Neg(Neg(Num(5)))), Add(Num(0), Mul(Num(3), Num(4))))) }\n",
        "17",
    ),
    (
        "p9-six-nested-arms",
        "type L { Nil, Cons(Int, L) }\n\
         fn f(l) { match l {\n\
         \x20 Cons(0, Cons(1, Cons(2, Nil))) -> 1\n\
         \x20 Cons(1, Cons(2, Cons(3, Nil))) -> 2\n\
         \x20 Cons(2, Cons(3, Cons(4, Nil))) -> 3\n\
         \x20 Cons(3, Cons(4, Cons(5, Nil))) -> 4\n\
         \x20 Cons(4, Cons(5, Cons(6, Nil))) -> 5\n\
         \x20 Cons(5, Cons(6, Cons(7, Nil))) -> 6\n\
         \x20 _ -> 0\n\
         } }\n\
         fn mk(i) { Cons(i, Cons(i + 1, Cons(i + 2, Nil))) }\n\
         pub fn main() -> Int { f(mk(0)) * 1 + f(mk(1)) * 10 + f(mk(2)) * 100 + f(mk(3)) * 1000 + f(mk(4)) * 10000 + f(mk(5)) * 100000 + f(mk(6)) * 1000000 }\n",
        "654321",
    ),
];

#[test]
fn nested_and_literal_patterns_run_natively() {
    let dir = temp_dir("patterns");
    for (tag, src, expected) in PATTERNS {
        let exe = compile_and_link(&lower_src(src), &dir, tag);
        let out = native(&exe).output().expect("run produced binary");
        diagnose_crash(&out.status, tag);
        assert!(
            out.status.success(),
            "{tag}: exited {:?}, stderr {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(got, eval_main_int(src), "{tag}: native vs evaluator");
        if !expected.is_empty() {
            assert_eq!(got, *expected, "{tag}");
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Spec D3: a user binder is bound only once its whole arm has matched. `f`'s
/// first arm binds `x` and then fails on `Q(2)`; the second arm must read the
/// PARAMETER `x` (9), not the failed arm's field (5).
#[test]
fn a_failed_nested_arm_does_not_shadow_an_outer_name() {
    let src = PATTERNS
        .iter()
        .find(|(t, _, _)| *t == "p4-outer-name")
        .map(|(_, s, _)| *s)
        .unwrap();
    let dir = temp_dir("pattern-shadow");
    let exe = compile_and_link(&lower_src(src), &dir, "p4");
    assert_runs(&exe, "509");
    assert_eq!(eval_main_int(src), "509");
    std::fs::remove_dir_all(&dir).ok();
}

/// `examples/05_replay.elya` (2026-10-10): record, replay and a test double as
/// ordinary handlers; the same bytes and value natively as in the evaluator.
/// Needed `%` (checked arithmetic) and nested patterns (pattern compilation).
#[test]
fn the_replay_example_runs_natively() {
    let src = include_str!("../../../examples/05_replay.elya");
    let dir = temp_dir("replay-example");
    let exe = compile_and_link(&lower_src(src), &dir, "replay");
    let (text, value) = native_text_value(&exe, "replay");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(text, eval_main_text(src));
    assert_eq!(
        text,
        "test: 5 quick sixes score 60\nrecorded a live run\nreplay: same score\n\
         replay of the changed game: diverged at an 8-sided roll\n"
    );
    assert_eq!(value, "2106");
    assert_eq!(value, eval_main_int(src));
}

// ---- N7 part 2: type variables and parametric ADTs natively (2026-10-15) ----
// Spec `docs/superpowers/specs/2026-10-15-elya-n7b-type-monomorphization-design.md`.
// Every program below was refused natively before the slice ("unrepresentable
// type" or "parametric ADT"); the expected values are the evaluator's, measured
// first (spec §0).
//
// Negative controls (each reverted, `git diff --quiet` after):
// - K1, references not redirected to clones (`Mono::resolve` returns `x`): the
//   whole corpus is refused by name, "callee is not a top-level function" (the
//   generic originals are no longer emitted, so nothing answers the name).
// - K2, instance fields left unsubstituted: the compiler panics in inkwell (an
//   i64 where a pointer was expected) on all three tests -- not a named
//   refusal, but an unsubstituted field cannot leave the pass. K2b, the mask
//   alone wrong (descriptor rows skip `Cons.mono.*` fields): r20's binary exits
//   1 after a collection, the other two tests pass -- the collector test is
//   what has the teeth for the mask.
// - K3, ambiguous variables not defaulted: r26 refused, "unrepresentable type"
//   (r19 still passed: its variable sits under an instance name -- r26 was
//   added for this control).
// - K4, the effect-position check off: r24 compiles (a String typed as Unit).
// - K5, instances built eagerly: r18 hits "too many type instances".

const LIST: &str = "type List(a) { Nil, Cons(a, List(a)) }\n";

const TYPE_VARS: &[(&str, &str, &str)] = &[
    (
        "q1-twice",
        "fn twice(f, x) { f(f(x)) }\n\
         pub fn main() -> Int { twice(fn(n) { n * 3 + 1 }, 5) }\n",
        "49",
    ),
    (
        "q2-id-at-two-types",
        "fn id(x) { x }\n\
         pub fn main() -> Int { if id(True) { id(40) + 2 } else { 0 } }\n",
        "42",
    ),
    (
        "q3-compose",
        "fn compose(f, g) { fn(x) { g(f(x)) } }\n\
         pub fn main() -> Int {\n\
         \x20 let h = compose(fn(n) { n + 1 }, fn(n) { n * 10 })\n\
         \x20 let b = compose(fn(n) { n > 3 }, fn(c) { if c { 7 } else { 9 } })\n\
         \x20 h(4) + b(5)\n\
         }\n",
        "57",
    ),
    (
        "q4-list-at-two-types",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn len(l) { match l { Nil -> 0  Cons(_, t) -> 1 + len(t) } }\n\
         fn sum(l) { match l { Nil -> 0  Cons(h, t) -> h + sum(t) } }\n\
         pub fn main() -> Int { sum(Cons(1, Cons(2, Cons(3, Nil)))) * 100 + len(Cons(True, Nil)) }\n",
        "601",
    ),
    (
        "q5-map-fold-range",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn map(f, l) { match l { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(f, t)) } }\n\
         fn foldl(f, acc, l) { match l { Nil -> acc  Cons(h, t) -> foldl(f, f(acc, h), t) } }\n\
         fn range(a, b) { if a >= b { Nil } else { Cons(a, range(a + 1, b)) } }\n\
         pub fn main() -> Int {\n\
         \x20 let xs = map(fn(n) { Cons(n, Nil) }, range(0, 2000))\n\
         \x20 foldl(fn(acc, l) { match l { Nil -> acc  Cons(h, _) -> acc + h } }, 0, xs)\n\
         }\n",
        "1999000",
    ),
    (
        "q6-generic-local",
        "pub fn main() -> Int {\n\
         \x20 let id = fn(x) { x }\n\
         \x20 if id(True) { id(41) + 1 } else { 0 }\n\
         }\n",
        "42",
    ),
    (
        "q7-option",
        "type Option(a) { None, Some(a) }\n\
         fn get(o, d) { match o { None -> d  Some(x) -> x } }\n\
         pub fn main() -> Int { get(Some(3), 0) + get(None, 4) }\n",
        "7",
    ),
    (
        "q8-a-row-and-a-type-variable",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn twice(f, x) { f(f(x)) }\n\
         pub fn main() -> Int {\n\
         \x20 handle { twice(fn(n) { lg(n) + 1 }, 5) } with { L.lg(v) -> resume(v * 2)  return(r) -> r * 1000 }\n\
         }\n",
        "23000",
    ),
    (
        // Measured as `mk(5)(True)`; a computed callee is refused natively by
        // its own name (5b-1 §5.3, not a type question), so it is bound first.
        "q9-closures-over-generic-values",
        "fn mk(x) { let m = fn(y) { x }  m }\n\
         fn pair(a, b) { fn(k) { k(a, b) } }\n\
         pub fn main() -> Int { let p = pair(3, True)  let m5 = mk(5)  p(fn(a, b) { if b { a } else { 0 } }) + m5(True) }\n",
        "8",
    ),
    (
        "r11-a-generic-nil-at-two-types",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn len(l) { match l { Nil -> 0  Cons(_, t) -> 1 + len(t) } }\n\
         pub fn main() -> Int { let n = Nil  len(Cons(1, n)) * 10 + len(Cons(True, Cons(False, n))) }\n",
        "12",
    ),
    (
        "r13-a-generic-handler",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         effect St(s) { fn get() -> s  fn put(v: s) -> Int }\n\
         fn len(l) { match l { Nil -> 0  Cons(_, t) -> 1 + len(t) } }\n\
         fn twice_get(d) { handle { len(get()) + len(get()) } with { St.get() -> resume(d)  St.put(v) -> resume(0)  return(r) -> r } }\n\
         pub fn main() -> Int { twice_get(Cons(1, Cons(2, Nil))) * 10 + twice_get(Cons(True, Nil)) }\n",
        "42",
    ),
    (
        "r15-mutually-recursive-generics",
        "fn ev(f, x, n) { if n == 0 { f(x) } else { od(f, x, n - 1) } }\n\
         fn od(f, x, n) { if n == 0 { f(x) } else { ev(f, x, n - 1) } }\n\
         pub fn main() -> Int { let a = ev(fn(b) { if b { 1 } else { 2 } }, True, 7)  a * 100 + ev(fn(k) { k * 3 }, 11, 4) }\n",
        "133",
    ),
    (
        "r17-a-function-field-with-an-effect",
        "effect L { fn lg(x: Int) -> Int }\n\
         type Box(a) { Box(fn(a) / {L} -> a, a) }\n\
         fn open(b) { match b { Box(f, v) -> f(f(v)) } }\n\
         pub fn main() -> Int {\n\
         \x20 handle { open(Box(fn(n) { lg(n) + 1 }, 4)) } with { L.lg(v) -> resume(v * 3)  return(r) -> r }\n\
         }\n",
        "40",
    ),
    (
        "r18-a-non-regular-type",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         type Nest(a) { NNil, NCons(a, Nest(List(a))) }\n\
         fn depth(n) { match n { NNil -> 0  NCons(_, _) -> 1 } }\n\
         pub fn main() -> Int { depth(NCons(1, NCons(Cons(2, Nil), NNil))) }\n",
        "1",
    ),
    (
        "r19-an-ambiguous-variable",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn seven(l) { 7 }\n\
         pub fn main() -> Int { seven(Nil) + seven(Cons(Nil, Nil)) }\n",
        "14",
    ),
    (
        // Added by negative control K3: with no default, r19 still compiled (its
        // ambiguous variable sits under an instance name), so nothing native
        // needed the default. Here the ambiguous variable is a lambda's own
        // parameter type, which must be represented.
        "r26-an-ambiguous-lambda-parameter",
        "fn ignore(g) { 7 }\n\
         pub fn main() -> Int { ignore(fn(x) { x }) + ignore(fn(x) { 1 }) }\n",
        "14",
    ),
    (
        "r21-a-two-parameter-lambda",
        "fn pair(a, b) { fn(k) { k(a, b) } }\n\
         pub fn main() -> Int { let p = pair(3, True)  p(fn(a, b) { if b { a } else { 0 } }) }\n",
        "3",
    ),
    (
        "r22-a-generic-local-to-a-generic-local",
        "pub fn main() -> Int {\n\
         \x20 let id = fn(x) { x }\n\
         \x20 let ap = fn(f, y) { f(y) }\n\
         \x20 ap(id, 5) + (if ap(id, True) { 10 } else { 0 })\n\
         }\n",
        "15",
    ),
    (
        "r23-nested-generic-patterns",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn head_or(l, d) { match l { Nil -> d  Cons(Cons(x, _), _) -> x  Cons(Nil, _) -> d } }\n\
         pub fn main() -> Int { head_or(Cons(Cons(42, Nil), Nil), 0) + head_or(Cons(Nil, Nil), 1) }\n",
        "43",
    ),
    (
        "r25-a-generic-effect",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         effect Gen(a) { fn yield(x: a) -> Int }\n\
         fn each(l) { match l { Nil -> 0  Cons(h, t) -> yield(h) + each(t) } }\n\
         fn count(l) { handle { each(l) } with { Gen.yield(x) -> resume(1)  return(r) -> r } }\n\
         pub fn main() -> Int { count(Cons(1, Cons(2, Nil))) * 10 + count(Cons(True, Nil)) }\n",
        "21",
    ),
    // The review (2026-10-15), each red first.
    (
        // F1 (high, new): the constructor's instance was keyed at a closed row,
        // the matching parameter's at an open one -- two instances, two tag
        // sets, "match failed" natively.
        "f1-a-pure-closure-in-an-option",
        "type Option(a) { None, Some(a) }\n\
         fn total(o) { match o { None -> 0  Some(f) -> f(1) } }\n\
         pub fn main() -> Int { total(Some(fn(x) { x + 1 })) }\n",
        "2",
    ),
    (
        "f1-a-list-of-closures",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn run(l, x) { match l { Nil -> x  Cons(f, t) -> run(t, f(x)) } }\n\
         pub fn main() -> Int { run(Cons(fn(x) { x * 4 }, Cons(fn(x) { x + 1 }, Nil)), 5) }\n",
        "21",
    ),
    (
        "f1-a-ground-type-holding-closures",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         type Bag { Bag(List(fn(Int) -> Int)) }\n\
         fn first(b) { match b { Bag(l) -> match l { Nil -> 0  Cons(f, _) -> f(10) } } }\n\
         pub fn main() -> Int { first(Bag(Cons(fn(x) { x * 3 }, Nil))) }\n",
        "30",
    ),
    (
        // F2 (medium, a regression): an effect parameter no operation uses
        // carries no value, so defaulting it is sound; it was refused.
        "f2-a-phantom-effect-parameter",
        "effect E(a) { fn e() -> Int }\n\
         fn go() { e() + 1 }\n\
         pub fn main() -> Int { handle { go() + e() } with { E.e() -> resume(2)  return(r) -> r } }\n",
        "5",
    ),
    // A differential sweep (2026-10-15, an interactive session): the instance
    // key kept row LABELS, but `IO` is a label that does not change the
    // convention -- `Box(fn() / {IO} -> Int)` built in `main` and
    // `Box(fn() / {|r} -> Int)` matched in `call` were two instances, two tag
    // sets: "match failed" natively where the evaluator printed 42. (The
    // annotation puts `IO` in the type without printing; a closure that
    // prints fails the same way.)
    (
        "f3-an-io-closure-in-a-box",
        "type Box(a) { B(a) }\n\
         fn call(b) { match b { B(f) -> f() } }\n\
         pub fn main() -> Int { let g: fn() / {IO} -> Int = fn() { 42 }  call(B(g)) }\n",
        "42",
    ),
    (
        "f3-io-and-pure-closures-in-a-list",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn run_all(fs, x) { match fs { Nil -> 0  Cons(f, rest) -> f(x) + run_all(rest, x) } }\n\
         pub fn main() -> Int {\n\
         \x20 let p: fn(Int) / {IO} -> Int = fn(x) { x + 1 }\n\
         \x20 run_all(Cons(p, Cons(fn(x) { x * 10 }, Nil)), 5)\n\
         }\n",
        "56",
    ),
    (
        // The key keeps the CONVENTION: one type at both conventions is still
        // two instances, so a binder's field type says how to call it.
        "f3-one-type-at-both-conventions",
        "effect S { fn get() -> Int }\n\
         type List(a) { Nil, Cons(a, List(a)) }\n\
         fn run_all(fs) { match fs { Nil -> 0  Cons(f, rest) -> f() + run_all(rest) } }\n\
         pub fn main() -> Int {\n\
         \x20 let p: fn() / {IO} -> Int = fn() { 1 }\n\
         \x20 let a = run_all(Cons(p, Cons(fn() { 2 }, Nil)))\n\
         \x20 let b = handle { run_all(Cons(fn() { get() }, Nil)) } with { S.get() -> resume(100)  return(r) -> r }\n\
         \x20 a + b\n\
         }\n",
        "103",
    ),
];

#[test]
fn the_type_variable_corpus_runs_natively() {
    run_value_corpus(TYPE_VARS, "type-vars");
}

#[test]
fn the_type_variable_corpus_matches_the_evaluator() {
    run_differential_corpus(TYPE_VARS, "type-vars-diff");
}

/// r20: a `List(Option(Int))` of 100 000 built by a tail loop, then summed.
/// The `Some` cells are reachable only through the list's HEAD field, which is
/// a pointer in `List(Option(Int))`'s instance and not in `List(Int)`'s: the
/// mask must come from the substituted field type (negative control K2).
#[test]
fn a_parametric_list_survives_collection() {
    let src = format!(
        "{LIST}type Option(a) {{ None, Some(a) }}\n\
         fn build(i, acc) {{ if i == 0 {{ acc }} else {{ build(i - 1, Cons(Some(i), acc)) }} }}\n\
         fn total(l, s) {{ match l {{ Nil -> s  Cons(o, t) -> match o {{ None -> total(t, s)  Some(v) -> total(t, s + v) }} }} }}\n\
         pub fn main() -> Int {{ total(build(100000, Nil), 0) }}\n"
    );
    let dir = temp_dir("type-vars-gc");
    let exe = compile_and_link(&lower_src(&src), &dir, "r20");
    let (value, stats) = run_with_gc_stats(&exe, "r20");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(value, "5000050000");
    assert!(stats.collections > 0, "the collector never ran: {stats:?}");
}

// ---- Recursion through a handle body (2026-10-15) ----
// Spec `docs/superpowers/specs/2026-10-15-elya-handle-recursion-design.md`.
// Before it, none of these passed the front end. n1 builds a chain of 1000
// nested handler frames, and every `get` walks to the innermost one.
const HANDLE_RECURSION: &[(&str, &str, &str)] = &[
    (
        "n1-a-chain-of-1000-handlers",
        "effect S { fn get() -> Int }\neffect T { fn t() -> Int }\n\
         fn nest(n: Int) / {T} -> Int { if n == 0 { t() } else { handle { nest(n - 1) + get() } with { S.get() -> resume(1)  return(r) -> r } } }\n\
         pub fn main() -> Int { handle { nest(1000) } with { T.t() -> resume(0)  return(r) -> r } }\n",
        "1000",
    ),
    (
        "n6-an-outer-effect-under-300-handlers",
        "effect S { fn get() -> Int }\neffect T { fn t() -> Int }\n\
         fn h(n) { handle { if n == 0 { t() } else { h(n - 1) + get() } } with { S.get() -> resume(1)  return(r) -> r } }\n\
         pub fn main() -> Int { handle { h(300) } with { T.t() -> resume(7)  return(r) -> r * 10 } }\n",
        "3070",
    ),
    (
        "n8-the-innermost-handler-answers",
        "effect S { fn get() -> Int }\neffect T { fn t() -> Int }\n\
         fn h(n) { if n == 0 { get() + t() } else { handle { h(n - 1) } with { S.get() -> resume(n)  return(r) -> r + 1 } } }\n\
         pub fn main() -> Int { handle { handle { h(3) } with { S.get() -> resume(100)  return(r) -> r } } with { T.t() -> resume(1000)  return(r) -> r } }\n",
        "1004",
    ),
    (
        "m2-an-effectful-callback-relayed",
        "effect S { fn get() -> Int }\neffect L { fn lg(x: Int) -> Int }\n\
         fn h(f, n) { if n == 0 { 0 } else { handle { h(f, n - 1) + f() + get() } with { S.get() -> resume(1)  return(r) -> r } } }\n\
         pub fn main() -> Int { handle { h(fn() { lg(2) }, 10) } with { L.lg(x) -> resume(x * 2)  return(r) -> r } }\n",
        "50",
    ),
    (
        "m3-the-callback-meets-the-innermost-handler",
        "effect S { fn get() -> Int }\n\
         fn h(f, n) { if n == 0 { f() } else { handle { h(f, n - 1) + get() } with { S.get() -> resume(1)  return(r) -> r } } }\n\
         pub fn main() -> Int { handle { h(fn() { get() * 100 }, 3) } with { S.get() -> resume(5)  return(r) -> r } }\n",
        "103",
    ),
    // Recursion through a lambda (spec §5).
    (
        "g1-go-under-its-handler",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn go(n) { let f = fn(s) { s + (if n == 0 { 0 } else { go(n - 1) }) }  f(1) + lg(n) }\n\
         pub fn main() -> Int { handle { go(3) } with { L.lg(x) -> resume(x * 10)  return(r) -> r } }\n",
        "64",
    ),
    (
        "g2-a-task-spawning-itself",
        "effect Async { fn spawn(f: fn() / {Async} -> Int) -> Int  fn await(id: Int) -> Int }\n\
         fn fib(n: Int) / {Async} -> Int { if n < 2 { n } else { let a = spawn(fn() { fib(n - 1) })  let b = spawn(fn() { fib(n - 2) })  await(a) + await(b) } }\n\
         fn eager(f: fn() / {Async} -> Int) -> Int { handle { f() } with { Async.spawn(g) -> resume(eager(g))  Async.await(v) -> resume(v)  return(r) -> r } }\n\
         pub fn main() -> Int { eager(fn() { fib(15) }) }\n",
        "610",
    ),
];

#[test]
fn recursion_through_a_handle_body_runs_natively() {
    run_value_corpus(HANDLE_RECURSION, "handle-recursion");
    run_differential_corpus(HANDLE_RECURSION, "handle-recursion-diff");
}

/// `examples/06_structured.elya` (2026-10-15): spawn/await and a joining
/// scope as ordinary handlers over the generic `List` and `Pair`; a task that
/// spawns itself (recursion through a lambda) and a scope per call. The same
/// bytes and value natively as in the evaluator.
#[test]
fn the_structured_concurrency_example_runs_natively() {
    let src = include_str!("../../../examples/06_structured.elya");
    let dir = temp_dir("structured-example");
    let exe = compile_and_link(&lower_src(src), &dir, "structured");
    let (text, value) = native_text_value(&exe, "structured");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(text, eval_main_text(src));
    assert_eq!(
        text,
        "scope: start\n  ping\n  body: done, the children run on\n    pong\n  ping\n    pong\n  ping\n\
         scope: joined\nfib(10) by spawned tasks\npfib(10) by nested scopes\n"
    );
    assert_eq!(value, "25555");
    assert_eq!(value, eval_main_int(src));
}

/// `examples/07_search.elya` (2026-10-15): one backtracking search over a
/// `multi` effect, three handlers (all solutions, a count, the first one).
#[test]
fn the_search_example_runs_natively() {
    let src = include_str!("../../../examples/07_search.elya");
    let dir = temp_dir("search-example");
    let exe = compile_and_link(&lower_src(src), &dir, "search");
    let (text, value) = native_text_value(&exe, "search");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(text, eval_main_text(src));
    assert_eq!(
        text,
        "6 queens: 4 solutions, all collected\n8 queens: 92 solutions, counted\n\
         8 queens: the first solution, found without the others\n"
    );
    assert_eq!(value, "13215863724");
    assert_eq!(value, eval_main_int(src));
}

/// `examples/08_timers.elya` (2026-10-16): the structured scheduler with
/// `sleep`, a poll loop over timers, a `Clock` effect under a virtual clock
/// and the real one (`time.now_ms`/`time.sleep_ms`), and a deadlock named.
/// The same bytes and value natively as in the evaluator.
#[test]
fn the_timers_example_runs_natively() {
    let src = include_str!("../../../examples/08_timers.elya");
    let dir = temp_dir("timers-example");
    let exe = compile_and_link(&lower_src(src), &dir, "timers");
    let (text, value) = native_text_value(&exe, "timers");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(text, eval_main_text(src));
    assert_eq!(
        text,
        "virtual clock:\n  b answered (10 ms)\n  c answered (20 ms)\n  a answered (30 ms)\n\
         \x20 60 ms of requests in 30 virtual ms\na deadlock:\n\
         \x20 deadlock: tasks 1 0 2 can never finish\nreal clock:\n\
         \x20 b answered (10 ms)\n  c answered (20 ms)\n  a answered (30 ms)\n\
         \x20 at least 30 real ms\n"
    );
    assert_eq!(value, "60361");
    assert_eq!(value, eval_main_int(src));
}

/// A multi-shot copy of a chain of 100 000 handler frames, natively (the
/// evaluator is the oracle at 1000 in `MULTI_SHOT`). Measured 2026-10-15:
/// 0.004 s, 0.014 s, 0.138 s at 1000, 10 000, 100 000 frames -- linear.
#[test]
fn a_multi_shot_copy_of_100000_handler_frames() {
    let src = "effect multi Flip { fn flip() -> Bool }\neffect S { fn get() -> Int }\n\
               fn nest(n) { if n == 0 { if flip() { 1 } else { 2 } } else { handle { nest(n - 1) + get() } with { S.get() -> resume(1)  return(r) -> r } } }\n\
               pub fn main() -> Int { handle nest(100000) with multi { Flip.flip() -> resume(True) * 1000000 + resume(False) } }\n";
    let dir = temp_dir("deep-multi");
    let exe = compile_and_link(&lower_src(src), &dir, "deep-multi");
    let out = native(&exe).output().expect("runs");
    std::fs::remove_dir_all(&dir).ok();
    diagnose_crash(&out.status, "deep-multi");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "100001100002");
}

// ---- 2026-10-15: native strings (`<>`, `==`, `int.to_string`) ----------------

/// Spec `2026-10-15-elya-native-strings-design.md`. Every row was refused before:
/// "Concat", "Eq on an unrepresentable operand type", or the unknown builtin.
/// `(tag, source, text, value)`; both halves are also the evaluator's.
///
/// Negative controls, each reverted (`cmp` against a saved copy):
/// - C2, `==` on strings compares lengths only: `eq-same-length` fails (1 for 10);
/// - C3, `int.to_string` drops the sign: `int-to-string` fails.
///
/// C1 is recorded on `a_left_operand_survives_a_collection_in_the_right_one`.
const STRINGS: &[(&str, &str, &str, &str)] = &[
    (
        "concat-literals",
        "pub fn main() -> Int {\n  io.println(\"ab\" <> \"cd\")\n  0\n}\n",
        "abcd\n",
        "0",
    ),
    (
        "concat-computed-and-compared",
        "fn greet(n) { \"hi \" <> n }\n\
         pub fn main() -> Int {\n\
         \x20 let s = greet(\"ada\")\n\
         \x20 io.println(s <> \"!\")\n\
         \x20 if s == \"hi ada\" { 1 } else { 0 }\n\
         }\n",
        "hi ada!\n",
        "1",
    ),
    (
        "eq-same-length",
        "pub fn main() -> Int { (if \"abc\" == \"abd\" { 1 } else { 0 }) + (if \"abc\" != \"abd\" { 10 } else { 0 }) }\n",
        "",
        "10",
    ),
    (
        "eq-different-length",
        "pub fn main() -> Int { (if \"ab\" == \"abc\" { 1 } else { 0 }) + (if \"\" == \"\" { 10 } else { 0 }) }\n",
        "",
        "10",
    ),
    (
        "int-to-string",
        "pub fn main() -> Int {\n\
         \x20 io.println(int.to_string(0))\n\
         \x20 io.println(int.to_string(0 - 42))\n\
         \x20 io.println(\"n=\" <> int.to_string(12345))\n\
         \x20 io.println(int.to_string(0 - 9223372036854775807 - 1))\n\
         \x20 0\n\
         }\n",
        "0\n-42\nn=12345\n-9223372036854775808\n",
        "0",
    ),
    (
        "empty-strings",
        "pub fn main() -> Int {\n\
         \x20 io.println(\"\" <> \"x\" <> \"\")\n\
         \x20 if \"\" <> \"\" == \"\" { 1 } else { 0 }\n\
         }\n",
        "x\n",
        "1",
    ),
    (
        // A generic fold over a list of strings (N7 part 2 meets strings).
        "join-a-list-of-strings",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn join(l, sep) { match l { Nil -> \"\"  Cons(h, Nil) -> h  Cons(h, t) -> h <> sep <> join(t, sep) } }\n\
         pub fn main() -> Int {\n\
         \x20 io.println(join(Cons(\"a\", Cons(\"b\", Cons(\"c\", Nil))), \", \"))\n\
         \x20 0\n\
         }\n",
        "a, b, c\n",
        "0",
    ),
    (
        // Inside effectful code: the operand is a perform's answer.
        "to-string-of-an-effects-answer",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn show(n) { io.println(\"v=\" <> int.to_string(lg(n))) }\n\
         pub fn main() -> Int {\n\
         \x20 handle { show(3)  show(4)  0 } with { L.lg(x) -> resume(x * 10)  return(r) -> r }\n\
         }\n",
        "v=30\nv=40\n",
        "0",
    ),
    (
        // The RIGHT operand performs: the left one waits across the site.
        "concat-whose-right-operand-performs",
        "effect N { fn name() -> String }\n\
         fn hello() { \"hello, \" <> name() }\n\
         pub fn main() -> Int {\n\
         \x20 handle { io.println(hello())  0 } with { N.name() -> resume(\"ada\")  return(r) -> r }\n\
         }\n",
        "hello, ada\n",
        "0",
    ),
    // String-literal patterns compile natively since `==` on strings does
    // (`patterns.rs` turns a literal test into `==`; found by the review).
    (
        "string-literal-patterns",
        "fn cls(s) { match s { \"a\" -> 1  \"bb\" -> 2  _ -> 3 } }\n\
         pub fn main() -> Int { cls(\"a\") + 10 * cls(\"bb\") + 100 * cls(\"a\" <> \"b\") + 1000 * cls(\"\") }\n",
        "",
        "3321",
    ),
    (
        "a-nested-string-literal-pattern",
        "type Pair(a, b) { Pair(a, b) }\n\
         fn cls(p) { match p { Pair(\"x\", n) -> n  Pair(_, n) -> n * 2 } }\n\
         pub fn main() -> Int { cls(Pair(\"x\", 1)) + cls(Pair(\"y\" <> \"\", 10)) }\n",
        "",
        "21",
    ),
    (
        "a-string-pattern-on-a-performs-answer",
        "effect N { fn name() -> String }\n\
         fn cls() { match name() { \"a\" -> 1  \"bb\" -> 2  _ -> 3 } }\n\
         pub fn main() -> Int {\n\
         \x20 handle { cls() * 100 + cls() * 10 + cls() } with { N.name() -> resume(\"bb\")  return(r) -> r }\n\
         }\n",
        "",
        "222",
    ),
];

#[test]
fn native_strings_match_the_evaluator() {
    let dir = temp_dir("strings");
    let mut failures = Vec::new();
    for (tag, src, text, value) in STRINGS {
        let exe = match try_compile_and_link(&lower_src(src), &dir, tag) {
            Ok(exe) => exe,
            Err(e) => {
                failures.push(format!("{tag}: {e}"));
                continue;
            }
        };
        let out = native(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        if !out.status.success() {
            failures.push(format!(
                "{tag}: {:?} {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
            continue;
        }
        let got = split_text_and_value(&String::from_utf8_lossy(&out.stdout), tag);
        let want = ((*text).to_string(), (*value).to_string());
        let eval = (eval_main_text(src), eval_main_int(src));
        if got != want || got != eval {
            failures.push(format!(
                "{tag}: native={got:?} want={want:?} evaluator={eval:?}"
            ));
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Strings are heap values the collector must keep while they are live. 20 000
/// labels `"<i>-<2i>"` are built and kept in a list while collections run; each
/// is then compared with a freshly built copy. The right operand of the outer
/// `<>` is itself a `<>` that allocates while the left operand waits. (C1 does
/// not show here: a collection frees ~30 000 blocks and the free lists hand
/// back the OLDEST dead block first, so the left operand's block is almost
/// never reused before it is read -- the next test is built so that it is.)
#[test]
fn strings_survive_collection() {
    let src = format!(
        "{LIST}fn label(i) {{ int.to_string(i) <> (\"-\" <> int.to_string(i * 2)) }}\n\
         fn build(i, acc) {{ if i == 0 {{ acc }} else {{ build(i - 1, Cons(Pair(i, label(i)), acc)) }} }}\n\
         type Pair(a, b) {{ Pair(a, b) }}\n\
         fn check(l, n) {{ match l {{ Nil -> n  Cons(Pair(i, s), t) -> check(t, if s == label(i) {{ n + 1 }} else {{ n }}) }} }}\n\
         pub fn main() -> Int {{ check(build(20000, Nil), 0) }}\n"
    );
    let dir = temp_dir("strings-gc");
    let exe = compile_and_link(&lower_src(&src), &dir, "strings-gc");
    let (stdout, stats) = run_with_gc_stats(&exe, "strings-gc");
    assert_eq!(stdout, "20000", "every label survived and compares equal");
    assert!(
        stats.collections > 0,
        "the collector ran: {}",
        stats.collections
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The left operand of `<>` waits while the right one runs, and here the right
/// one collects. The left is a 54-byte literal (9 words), the only block of its
/// size class; `right()` churns 70 000 cells (collections) and then allocates
/// another 9-word string. A collection that does not see the left operand frees
/// it, and the free list hands exactly that block to `right()`'s literal: the
/// two operands alias and the comparison fails.
///
/// Negative control C1 (reverted, `cmp` clean): the left operand not rooted while
/// the right one is lowered -- this program answers 0 (the evaluator: 1), and
/// the collector reports `live=0` where it reports the rooted 9 words.
#[test]
fn a_left_operand_survives_a_collection_in_the_right_one() {
    let a = "A".repeat(54);
    let b = "B".repeat(54);
    let src = format!(
        "{LIST}fn churn(n) {{ if n == 0 {{ 0 }} else {{ let c = Cons(n, Nil)  churn(n - 1) }} }}\n\
         fn right() {{ let _ = churn(70000)  \"{b}\" }}\n\
         pub fn main() -> Int {{\n\
         \x20 let s = \"{a}\" <> right()\n\
         \x20 if s == \"{a}{b}\" {{ 1 }} else {{ 0 }}\n\
         }}\n"
    );
    assert_eq!(eval_main_int(&src), "1");
    let dir = temp_dir("strings-left-operand");
    let exe = compile_and_link(&lower_src(&src), &dir, "strings-left-operand");
    let (stdout, stats) = run_with_gc_stats(&exe, "strings-left-operand");
    assert_eq!(stdout, "1");
    assert!(
        stats.collections > 0,
        "the collector ran: {}",
        stats.collections
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The CPS emitter's half of the same obligation: in effectful code the left
/// operand of `<>` waits across a continuation site (the right one performs),
/// thousands of times, while collections run. The CPS emitter's own roots
/// around `<>` and `int.to_string` (C4, C5) are pinned by the next two tests.
#[test]
fn a_string_operand_survives_an_effect_site() {
    let src = "effect N { fn next(i: Int) -> String }\n\
               fn go(i, acc) { if i == 0 { acc } else { go(i - 1, if (int.to_string(i) <> next(i)) == (int.to_string(i) <> \"x\") { acc + 1 } else { acc }) } }\n\
               pub fn main() -> Int { handle { go(5000, 0) } with { N.next(i) -> resume(\"x\")  return(r) -> r } }\n";
    assert_eq!(eval_main_int(src), "5000");
    let dir = temp_dir("strings-cps-gc");
    let exe = compile_and_link(&lower_src(src), &dir, "strings-cps-gc");
    let (stdout, stats) = run_with_gc_stats(&exe, "strings-cps-gc");
    assert_eq!(stdout, "5000");
    assert!(
        stats.collections > 0,
        "the collector ran: {}",
        stats.collections
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// In a resumption function the frame being resumed is DEAD: the continuation
/// and the frame's saved bindings are reachable only through what `root_live`
/// pushes around an allocation. Here `<>` (the answer of `name()` and `"y"`) is
/// computed in the resumption, and `s`, a 54-byte string (a size class nothing
/// else uses) bound since the start, must survive every collection it triggers;
/// `u` allocates a same-size string each iteration, so a freed `s` is reused at
/// once and the comparison changes. Found by the review of native strings: the
/// author's own control C4 had failed nothing.
///
/// Negative control C4 (reverted, `cmp` clean): no `root_live` around a CPS
/// `<>` -- 7021 instead of 20000 (deterministic).
#[test]
fn cps_live_values_survive_a_concat_in_a_resumption() {
    let a = "A".repeat(54);
    let b = "B".repeat(54);
    let src = format!(
        "effect N {{ fn name() -> String }}\n\
         fn go(i, s, acc) {{ if i == 0 {{ acc }} else {{ let t = name() <> \"y\"  let u = \"{b}\"  go(i - 1, s, if s == u {{ acc }} else {{ acc + 1 }}) }} }}\n\
         pub fn main() -> Int {{ handle {{ go(20000, \"{a}\", 0) }} with {{ N.name() -> resume(\"q\") }} }}\n"
    );
    assert_eq!(eval_main_int(&src), "20000");
    let dir = temp_dir("strings-cps-concat-roots");
    let exe = compile_and_link(&lower_src(&src), &dir, "strings-cps-concat-roots");
    let (stdout, stats) = run_with_gc_stats(&exe, "strings-cps-concat-roots");
    assert_eq!(stdout, "20000");
    assert!(
        stats.collections > 0,
        "the collector ran: {}",
        stats.collections
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The same obligation for `int.to_string` of a perform's answer, computed in
/// the resumption.
///
/// Negative control C5 (reverted, `cmp` clean): no `root_live` around a CPS
/// `int.to_string` -- the count is wrong (5242 for 50 000 in the review).
#[test]
fn cps_live_values_survive_int_to_string_in_a_resumption() {
    let a = "A".repeat(54);
    let b = "B".repeat(54);
    let src = format!(
        "effect K {{ fn tick(n: Int) -> Int }}\n\
         fn go(i, s, acc) {{ if i == 0 {{ acc }} else {{ let t = int.to_string(tick(i))  let u = \"{b}\"  let w = \"\"  go(i - 1, s, if s == u {{ acc }} else {{ acc + 1 }}) }} }}\n\
         pub fn main() -> Int {{ handle {{ go(20000, \"{a}\", 0) }} with {{ K.tick(x) -> resume(x) }} }}\n"
    );
    assert_eq!(eval_main_int(&src), "20000");
    let dir = temp_dir("strings-cps-itos-roots");
    let exe = compile_and_link(&lower_src(&src), &dir, "strings-cps-itos-roots");
    let (stdout, stats) = run_with_gc_stats(&exe, "strings-cps-itos-roots");
    assert_eq!(stdout, "20000");
    assert!(
        stats.collections > 0,
        "the collector ran: {}",
        stats.collections
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ---- 2026-10-15: time, the first I/O source (async step 2, part 2) ----------

/// `time.now_ms` and `time.sleep_ms` natively (spec
/// `2026-10-15-elya-async-step2-timers-design.md` D3): the clock advances by at
/// least the sleep, in direct code and in a handler clause (the CPS emitter),
/// and a non-positive sleep returns at once. Each answer is 1 only if it does.
///
/// Negative control T3 (reverted, `cmp` clean): `elya_time_sleep_ms` returns at
/// once -- both rows answer 0.
#[test]
fn the_clock_advances_natively() {
    let rows = [
        (
            "direct",
            "pub fn main() -> Int {\n\
             \x20 let a = time.now_ms()\n\
             \x20 time.sleep_ms(20)\n\
             \x20 let b = time.now_ms()\n\
             \x20 time.sleep_ms(0 - 5)\n\
             \x20 if a >= 0 && b - a >= 20 && time.now_ms() >= b { 1 } else { 0 }\n\
             }\n",
        ),
        (
            "in-a-clause",
            "effect W { fn wait(ms: Int) -> Unit }\n\
             fn pause() { wait(15)  wait(10) }\n\
             pub fn main() -> Int {\n\
             \x20 let a = time.now_ms()\n\
             \x20 handle { pause() } with { W.wait(ms) -> { time.sleep_ms(ms)  resume(Unit) } }\n\
             \x20 if time.now_ms() - a >= 25 { 1 } else { 0 }\n\
             }\n",
        ),
    ];
    let dir = temp_dir("time-builtins");
    let mut failures = Vec::new();
    for (tag, src) in rows {
        assert_eq!(eval_main_int(src), "1", "{tag}: the evaluator");
        match try_compile_and_link(&lower_src(src), &dir, tag) {
            Ok(exe) => {
                let out = native(&exe).output().expect("run produced binary");
                let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !out.status.success() || got != "1" {
                    failures.push(format!("{tag}: {:?} {got:?}", out.status));
                }
            }
            Err(e) => failures.push(format!("{tag}: {e}")),
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---- A function's own row in Core (2026-10-16) ----
// Spec `docs/superpowers/specs/2026-10-16-elya-own-row-in-core-design.md`. Each
// was refused natively ("type variable only in an effect row"): `s` is generic
// only through the functions' own rows.
const OWN_ROW: &[(&str, &str, &str)] = &[
    (
        "r10-dup-at-int",
        "effect St(s) { fn get() -> s  fn put(v: s) -> Int }\n\
         fn dup() { put(get()) }\n\
         pub fn main() -> Int { handle { dup()  5 } with { St.get() -> resume(3)  St.put(v) -> resume(0)  return(r) -> r } }\n",
        "5",
    ),
    (
        "r24-dup-at-string",
        "effect St(s) { fn get() -> s  fn put(v: s) -> Int }\n\
         fn dup() { put(get()) }\n\
         pub fn main() -> Int { handle { dup()  5 } with { St.get() -> resume(\"x\")  St.put(v) -> resume(0)  return(r) -> r } }\n",
        "5",
    ),
    (
        // Defaulting `s` to `Unit` left the list unrooted: natively 4130074439
        // (a collected list summed), measured with the refusal switched off.
        // `len` counts by a tail loop: 200 000 non-tail calls overflow the
        // native stack (SIGSEGV), which is not what this row is about.
        "r27-a-list-across-collections",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         effect St(s) { fn get() -> s  fn put(v: s) -> Int }\n\
         fn build(i, acc) { if i == 0 { acc } else { build(i - 1, Cons(i, acc)) } }\n\
         fn count(l, k) { match l { Nil -> k  Cons(_, t) -> count(t, k + 1) } }\n\
         fn len(l) { count(l, 0) }\n\
         fn sum(l, s) { match l { Nil -> s  Cons(h, t) -> sum(t, s + h) } }\n\
         fn dup() { let v = get()  let n = len(build(200000, Nil))  put(v) + n }\n\
         pub fn main() -> Int { handle { dup() } with { St.get() -> resume(build(1000, Nil))  St.put(v) -> resume(sum(v, 0))  return(r) -> r } }\n",
        "700500",
    ),
    (
        "g1-through-generic-callees",
        "effect St(s) { fn get() -> s  fn put(v: s) -> Int }\n\
         fn getter() { get() }\n\
         fn putter(v) { put(v) }\n\
         fn dup3() { let v = getter()  putter(v) }\n\
         pub fn main() -> Int { handle { dup3() } with { St.get() -> resume(\"x\")  St.put(v) -> resume(0)  return(r) -> r } }\n",
        "0",
    ),
];

#[test]
fn row_only_type_variables_run_natively() {
    run_value_corpus(OWN_ROW, "own-row");
    run_differential_corpus(OWN_ROW, "own-row-diff");
}

/// `examples/10_failure.elya` (2026-10-16): failure as an effect answering
/// the empty `Never` (an empty match is a trap natively, never reached), a
/// `catch` handler, and scopes that drop their queued tasks when one fails.
#[test]
fn the_failure_example_runs_natively() {
    let src = include_str!("../../../examples/10_failure.elya");
    let dir = temp_dir("failure-example");
    let exe = compile_and_link(&lower_src(src), &dir, "failure");
    let (text, value) = native_text_value(&exe, "failure");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(text, eval_main_text(src));
    assert!(
        text.contains("  scope cancelled, 3 task(s) dropped\nscope failed: division by zero\n"),
        "{text}"
    );
    assert_eq!(value, "-123");
    assert_eq!(value, eval_main_int(src));
}

// ---- Computed callees and functions as values (2026-10-16) ----
// Spec `docs/superpowers/specs/2026-10-16-elya-callees-and-function-values-design.md`.
// Every program was refused natively ("computed callee" or "function used as a
// value"); values are the evaluator's.
const CALLEES: &[(&str, &str, &str)] = &[
    (
        "c1-computed-callees",
        "fn compose(f, g) { fn(x) { g(f(x)) } }\n\
         fn mk() { fn(y) { y * 2 } }\n\
         pub fn main() -> Int { (compose(fn(a) { a + 1 }, fn(b) { b * 10 }))(4) + (mk())(1) }\n",
        "52",
    ),
    (
        // The callee before the argument: each `lg` appends its digit below the
        // rest's, so the order is the number's last two digits (21).
        "c4-the-callee-first",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn pick(n) { let _ = lg(1)  fn(x) { x + n } }\n\
         fn arg(v) { let _ = lg(2)  v }\n\
         pub fn main() -> Int { handle { (pick(1))(arg(2)) } with { L.lg(v) -> resume(0) * 10 + v  return(r) -> r } }\n",
        "321",
    ),
    (
        "c3-a-function-in-a-local",
        "fn double(x) { x * 2 }\n\
         fn apply(f, x) { f(x) }\n\
         pub fn main() -> Int { let g = double  apply(g, 4) + g(1) }\n",
        "10",
    ),
    (
        "c5-a-function-in-a-field",
        "type Box { B(fn(Int) -> Int) }\n\
         fn inc(x) { x + 1 }\n\
         fn open(b) { match b { B(f) -> f(41) } }\n\
         pub fn main() -> Int { open(B(inc)) }\n",
        "42",
    ),
    (
        "c6-an-effectful-function-as-a-value",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn logd(x) { lg(x) + 1 }\n\
         fn twice(f, x) { f(f(x)) }\n\
         pub fn main() -> Int { handle { twice(logd, 5) } with { L.lg(v) -> resume(v * 2)  return(r) -> r } }\n",
        "23",
    ),
    (
        "c7-a-generic-function-as-a-value",
        "fn id(x) { x }\n\
         fn apply(f, x) { f(x) }\n\
         pub fn main() -> Int { if apply(id, True) { apply(id, 7) } else { 0 } }\n",
        "7",
    ),
    (
        // A local shadowing a top-level name is the local's value, not wrapped.
        "c8-a-shadowing-local",
        "fn inc(x) { x + 1 }\n\
         fn apply(f, x) { f(x) }\n\
         pub fn main() -> Int { let inc = fn(x) { x + 100 }  apply(inc, 1) }\n",
        "101",
    ),
    // The review's finding 1: a pure function passed where an effectful one is
    // expected. The wrapper's inner call had the WIDENED type, so a direct
    // function was called with the CPS convention ("calling convention
    // disagrees with the callee").
    (
        "r1-a-pure-function-at-an-effectful-type",
        "effect E { fn ask(n: Int) -> Int }\n\
         fn inc(x) { x + 1 }\n\
         fn apply(f, x) { f(x) }\n\
         pub fn main() -> Int { handle { ask(3) + apply(inc, 0) } with { E.ask(n) -> resume(n * 10)  return(r) -> r } }\n",
        "31",
    ),
    (
        "r2-pure-and-effectful-values-to-one-function",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn logd(x) { lg(x) + 1 }\n\
         fn inc(x) { x + 1 }\n\
         fn twice(f, x) { f(f(x)) }\n\
         pub fn main() -> Int { handle { twice(logd, 5) + twice(inc, 0) } with { L.lg(v) -> resume(v * 2)  return(r) -> r } }\n",
        "25",
    ),
    (
        "r3-a-pure-function-in-an-effectful-field",
        "effect E { fn ask(n: Int) -> Int }\n\
         type Box { B(fn(Int) / {E} -> Int) }\n\
         fn inc(x) { x + 1 }\n\
         fn open(b) { match b { B(f) -> f(ask(1)) } }\n\
         pub fn main() -> Int { handle { open(B(inc)) } with { E.ask(n) -> resume(n * 10)  return(r) -> r } }\n",
        "11",
    ),
];

#[test]
fn computed_callees_and_function_values_run_natively() {
    run_value_corpus(CALLEES, "callees");
}

#[test]
fn computed_callees_and_function_values_match_the_evaluator() {
    run_differential_corpus(CALLEES, "callees-diff");
}

// ---- Argument records past three parameters (2026-10-16) ----
// Spec `docs/superpowers/specs/2026-10-16-elya-argument-records-design.md`.
// Every program was refused natively by an arity cap; values are the
// evaluator's.
const ARITY: &[(&str, &str, &str)] = &[
    (
        "a1-six-parameters",
        "fn six(a, b, c, d, e, f) { a + b * 2 + c * 3 + d * 4 + e * 5 + f * 6 }\n\
         pub fn main() -> Int { six(1, 2, 3, 4, 5, 6) }\n",
        "91",
    ),
    (
        "a2-an-effectful-function-of-five",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn five(a, b, c, d, e) { lg(a) + b * 2 + c * 3 + d * 4 + e * 5 }\n\
         pub fn main() -> Int { handle { five(1, 2, 3, 4, 5) } with { L.lg(x) -> resume(x * 100)  return(r) -> r } }\n",
        "154",
    ),
    (
        "a3-a-lambda-of-five",
        "pub fn main() -> Int { let f = fn(a, b, c, d, e) { a - b + c - d + e * 10 }  f(1, 2, 3, 4, 5) }\n",
        "48",
    ),
    (
        "a4-an-effectful-lambda-of-four",
        "effect L { fn lg(x: Int) -> Int }\n\
         pub fn main() -> Int { handle { let f = fn(a, b, c, d) { lg(a) + b + c + d }  f(1, 2, 3, 4) } with { L.lg(x) -> resume(x * 100)  return(r) -> r } }\n",
        "109",
    ),
    (
        "a5-an-operation-of-four",
        "effect W { fn w(a: Int, b: Int, c: Int, d: Int) -> Int }\n\
         pub fn main() -> Int { handle { w(1, 2, 3, 4) + 1 } with { W.w(a, b, c, d) -> resume(a * 1000 + b * 100 + c * 10 + d)  return(r) -> r } }\n",
        "1235",
    ),
    (
        "a6-a-six-parameter-loop-of-100000",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn go(l, a, b, c, d, acc) { match l { Nil -> acc + a + b + c + d  Cons(h, t) -> go(t, a + 1, b, c, d, acc + h) } }\n\
         fn build(i, acc) { if i == 0 { acc } else { build(i - 1, Cons(i, acc)) } }\n\
         pub fn main() -> Int { go(build(100000, Nil), 0, 1, 2, 3, 0) }\n",
        "5000150006",
    ),
    (
        "a7-a-five-parameter-lambda-as-an-argument",
        "fn apply5(f, x) { f(x, x, x, x, x) }\n\
         pub fn main() -> Int { apply5(fn(a, b, c, d, e) { a + b + c + d + e }, 3) }\n",
        "15",
    ),
    // The review's finding: a function in a spilled position. The call keyed
    // its record by the argument's type (an open row), the callee by its
    // parameter's (closed): two records, "match failed" natively.
    (
        "r1-a-function-in-the-fourth-position",
        "fn k(n, s, acc, f) { acc + f(s) }\n\
         pub fn main() -> Int { k(3, 1, 0, fn(p) { p + 1 }) }\n",
        "2",
    ),
    (
        "r2-functions-in-a-spilled-tail-loop",
        "fn loop(n, s, l, f, g, acc) { if n == 0 { acc + l } else { loop(n - 1, f(s), l, f, g, acc + g(s)) } }\n\
         fn inc(x) { x + 1 }\n\
         pub fn main() -> Int { loop(100000, 0, 7, inc, fn(x) { x * 2 }, 0) }\n",
        "9999900007",
    ),
    (
        "r3-an-effectful-function-in-the-fourth-position",
        "effect L { fn lg(x: Int) -> Int }\n\
         fn k(n, s, acc, f) { acc + f(s) + n }\n\
         pub fn main() -> Int { handle { k(3, 1, 0, fn(p) { lg(p) + 1 }) } with { L.lg(v) -> resume(v * 10)  return(r) -> r } }\n",
        "14",
    ),
];

#[test]
fn long_parameter_lists_run_natively() {
    run_value_corpus(ARITY, "arity");
}

#[test]
fn long_parameter_lists_match_the_evaluator() {
    run_differential_corpus(ARITY, "arity-diff");
}

// ---------------------------------------------------------------------------
// Deep non-tail recursion natively (spec 2026-10-16-elya-native-stack). The
// program runs on its own 1 GiB stack (`elya_start`), not the main thread's;
// reaching its guard is a named failure. These tests use `Command::new`, so the
// DEFAULT stack, and clear `ELYA_STACK_MIB` in case the harness has it set.
//
// Negative controls (each reverted, `git diff --quiet` confirmed):
// - K1: the SIGSEGV handler treats every fault as foreign -> the named
//   overflow test fails with signal 11 and no diagnostic.
// - K2: `elya_start` calls its entry on the calling thread (no own stack) -> the DEEP rows fail with
//   SIGSEGV (rc 139), as measured before the slice.
// - K3: the runtime ignores `ELYA_STACK_MIB` -> the knob test fails (m2 runs
//   under "8", and "0" / "lots" run instead of being refused).
// ---------------------------------------------------------------------------

const LEN_SRC: &str = "type List { Nil, Cons(Int, List) }\n\
     fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
     fn len(l) { match l { Nil -> 0  Cons(_, t) -> 1 + len(t) } }\n";

fn deep_src(kind: &str, n: u64) -> String {
    match kind {
        "len" => format!("{LEN_SRC}pub fn main() -> Int {{ len(build({n}, Nil)) }}\n"),
        "foldr" => format!(
            "type List {{ Nil, Cons(Int, List) }}\n\
             fn build(n, acc) {{ if n == 0 {{ acc }} else {{ build(n - 1, Cons(n, acc)) }} }}\n\
             fn foldr(f, z, l) {{ match l {{ Nil -> z  Cons(h, t) -> f(h, foldr(f, z, t)) }} }}\n\
             pub fn main() -> Int {{ foldr(fn(a, b) {{ a + b }}, 0, build({n}, Nil)) }}\n"
        ),
        "resume" => format!(
            "effect Tick {{ fn tick() -> Unit }}\n\
             fn go(n) / {{Tick}} {{ if n == 0 {{ 0 }} else {{ let _ = tick()  go(n - 1) }} }}\n\
             pub fn main() -> Int {{ handle {{ go({n}) }} with {{ Tick.tick() -> {{ let r = resume(Unit)  r + 1 }} }} }}\n"
        ),
        other => panic!("no deep program {other}"),
    }
}

fn run_default_stack(exe: &Path) -> std::process::Output {
    Command::new(exe)
        .env_remove("ELYA_STACK_MIB")
        .output()
        .expect("run produced binary")
}

fn run_with_stack(exe: &Path, mib: &str) -> std::process::Output {
    Command::new(exe)
        .env("ELYA_STACK_MIB", mib)
        .output()
        .expect("run produced binary")
}

#[test]
fn deep_non_tail_recursion_runs_natively_a_million_deep() {
    let dir = temp_dir("deep-million");
    let mut failures = Vec::new();
    for (kind, want) in [
        ("len", "1000000"),
        ("foldr", "500000500000"),
        ("resume", "1000000"),
    ] {
        let core = lower_src(&deep_src(kind, 1_000_000));
        let exe = compile_and_link(&core, &dir, kind);
        let out = run_default_stack(&exe);
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        if !out.status.success() || got != want || !err.is_empty() {
            failures.push(format!(
                "{kind}: {:?} stdout={got:?} stderr={err:?}",
                out.status
            ));
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn deep_non_tail_recursion_matches_the_evaluator() {
    let dir = temp_dir("deep-diff");
    let mut failures = Vec::new();
    for kind in ["len", "foldr"] {
        let src = deep_src(kind, 200_000);
        let exe = compile_and_link(&lower_src(&src), &dir, kind);
        let out = run_default_stack(&exe);
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let want = eval_main_int(&src);
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        if !out.status.success() || got != want || !err.is_empty() {
            failures.push(format!(
                "{kind}: {:?} native={got:?} evaluator={want} stderr={err:?}",
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

const OVERFLOW_LINE: &str = "elya: native stack overflow: recursion deeper than the 8 MiB stack";

#[test]
fn a_native_stack_overflow_is_named() {
    let dir = temp_dir("deep-overflow");
    let src = "fn down(n) { 1 + down(n + 1) }\n\
               pub fn main() -> Int { io.println(\"before\")  down(0) }\n";
    let exe = compile_and_link(&lower_src(src), &dir, "down");
    let out = run_with_stack(&exe, "8");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            out.status.signal(),
            None,
            "killed by a signal: {:?}",
            out.status
        );
    }
    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "before\n");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim_end(),
        OVERFLOW_LINE
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn elya_stack_mib_sets_the_stack_and_is_checked() {
    let dir = temp_dir("deep-knob");
    let exe = compile_and_link(&lower_src(&deep_src("len", 200_000)), &dir, "len");
    // Under 8 MiB, 200 000 frames overflow (measured limit about 130 000), by name.
    let small = run_with_stack(&exe, "8");
    assert_eq!(small.status.code(), Some(1), "{:?}", small.status);
    assert_eq!(
        String::from_utf8_lossy(&small.stderr).trim_end(),
        OVERFLOW_LINE
    );
    // Under the default they run.
    let big = run_default_stack(&exe);
    assert!(big.status.success(), "{:?}", big.status);
    assert_eq!(String::from_utf8_lossy(&big.stdout).trim(), "200000");
    // Anything but a whole number of MiB in range is refused before the program runs.
    for bad in ["0", "lots", "8x", "-8", "1048577", ""] {
        let out = run_with_stack(&exe, bad);
        assert_eq!(out.status.code(), Some(1), "{bad:?}: {:?}", out.status);
        assert!(out.stdout.is_empty(), "{bad:?}: the program ran");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr).trim_end(),
            "elya: ELYA_STACK_MIB must be a whole number of MiB from 1 to 1048576",
            "{bad:?}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The 2026-10-16 review (M1): every Elya function was mangled `elya_<name>`,
/// the runtime's own namespace, so a function named like a runtime symbol
/// (`alloc`, `println`, and the new `start`) failed to link. Elya functions
/// are now `elya.<name>`, which no C symbol can be.
#[test]
fn elya_functions_may_be_named_like_runtime_symbols() {
    let src = "fn alloc(x) { x + 1 }\n\
               fn println(x) { x * 2 }\n\
               fn start(x) { x - 3 }\n\
               fn entry(x) { x + 10 }\n\
               fn gc_push(x) { x }\n\
               fn current_handler(x) { x }\n\
               pub fn main() -> Int { entry(start(println(alloc(gc_push(current_handler(20)))))) }\n";
    let dir = temp_dir("runtime-names");
    let core = lower_src(src);
    let exe = try_compile_and_link(&core, &dir, "names").expect("compiles and links");
    assert_runs(&exe, "49");
    assert_eq!(eval_main_int(src), "49");
    std::fs::remove_dir_all(&dir).ok();
}

/// The review (L1, L2): a fault outside the guard is not an overflow and still
/// kills by its signal, with nothing on stderr -- including a SIGSEGV sent by
/// `raise`/`kill`, which no faulting instruction re-raises. Driven by a C
/// harness linked with the runtime, since no Elya program faults that way.
#[cfg(unix)]
#[test]
fn foreign_faults_still_kill_by_their_signal() {
    use std::os::unix::process::ExitStatusExt;
    let dir = temp_dir("foreign-faults");
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime.c");
    for (tag, body, sig) in [
        ("null", "volatile int64_t *p = 0; return *p;", 11),
        ("raised-segv", "raise(SIGSEGV); return 0;", 11),
        ("raised-bus", "raise(SIGBUS); return 0;", 7),
    ] {
        let c = dir.join(format!("{tag}.c"));
        std::fs::write(
            &c,
            format!(
                "#include <stdint.h>\n#include <signal.h>\n\
                 int64_t elya_start(int64_t (*)(void));\n\
                 static int64_t entry(void) {{ {body} }}\n\
                 int main(void) {{ return (int)elya_start(entry); }}\n"
            ),
        )
        .expect("write harness");
        let exe = dir.join(tag);
        let linked = Command::new("clang")
            .arg(&c)
            .arg(&runtime)
            .arg("-pthread")
            .arg("-o")
            .arg(&exe)
            .output()
            .expect("spawn clang");
        assert!(
            linked.status.success(),
            "{tag}: {}",
            String::from_utf8_lossy(&linked.stderr)
        );
        let out = Command::new(&exe).output().expect("run harness");
        assert_eq!(out.status.signal(), Some(sig), "{tag}: {:?}", out.status);
        assert!(
            out.stderr.is_empty(),
            "{tag}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Structural equality natively (spec 2026-10-16-elya-structural-equality, D3):
// `elya_eq` walks two values with a worklist guided by the descriptor table.
//
// Negative controls (each reverted, `git diff --quiet` confirmed):
// - K2: `elya_eq` compares every field as a word (ignores `ptr_mask`) -> the
//   rows comparing separately built heap fields differ (q2, q3, q5, q6, q7);
//   q1, whose fields are words, still passes, as it should.
// - K3: `elya_eq` skips the string case -> q3 and q5 differ.
// ---------------------------------------------------------------------------

const EQ_DECLS: &str = "type L { N, C(Int, L) }\n\
type P { P(Int, Int) }\n\
type Box(a) { Box(a) }\n\
type S { S(String, L), T(Bool) }\n\
fn same(a, b) { a == b }\n\
fn build(n, acc) { if n == 0 { acc } else { build(n - 1, C(n, acc)) } }\n\
fn b(x) { if x { 1 } else { 0 } }\n";

const EQUALITY: &[(&str, &str, &str)] = &[
    (
        "q1-constructors",
        "pub fn main() -> Int { b(P(1, 2) == P(1, 2)) * 10 + b(P(1, 2) == P(2, 1)) }\n",
        "10",
    ),
    (
        "q2-lists-ne",
        "pub fn main() -> Int { b(C(1, N) != C(1, C(2, N))) * 10 + b(C(1, N) != C(1, N)) }\n",
        "10",
    ),
    (
        "q3-strings-in-fields",
        "pub fn main() -> Int { let s = \"ab\" <> \"c\"  b(S(s, C(1, N)) == S(\"abc\", C(1, N))) * 100 + b(S(\"abd\", N) == S(\"abc\", N)) * 10 + b(T(True) == S(\"x\", N)) }\n",
        "100",
    ),
    (
        "q4-unit-and-nullary",
        "pub fn main() -> Int { b(Unit == Unit) * 100 + b(N == N) * 10 + b(N == C(1, N)) }\n",
        "110",
    ),
    (
        "q5-generic-and-parametric",
        "pub fn main() -> Int { b(same(Box(C(3, N)), Box(C(3, N)))) * 100 + b(same(4, 4)) * 10 + b(same(Box(\"x\"), Box(\"y\"))) }\n",
        "110",
    ),
    (
        "q6-in-a-handler-clause",
        "effect Ask { fn ask(x: L) -> Bool }\n\
         fn go(n) / {Ask} { if ask(build(n, N)) { 1 } else { 0 } }\n\
         pub fn main() -> Int { handle { go(3) + go(2) * 10 } with { Ask.ask(l) -> resume(l == C(1, C(2, C(3, N))))  return(v) -> v } }\n",
        "1",
    ),
    (
        "q7-long-lists",
        "pub fn main() -> Int { let x = build(200000, N)  let y = build(200000, N)  let z = build(200000, C(7, N))  b(x == y) * 10 + b(x == z) }\n",
        "10",
    ),
];

fn with_eq_decls() -> Vec<(&'static str, String, &'static str)> {
    EQUALITY
        .iter()
        .map(|(t, s, w)| (*t, format!("{EQ_DECLS}{s}"), *w))
        .collect()
}

#[test]
fn structural_equality_runs_natively() {
    let rows = with_eq_decls();
    let corpus: Vec<(&str, &str, &str)> =
        rows.iter().map(|(t, s, w)| (*t, s.as_str(), *w)).collect();
    run_value_corpus(&corpus, "equality");
}

#[test]
fn structural_equality_matches_the_evaluator() {
    let rows = with_eq_decls();
    let corpus: Vec<(&str, &str, &str)> =
        rows.iter().map(|(t, s, w)| (*t, s.as_str(), *w)).collect();
    run_differential_corpus(&corpus, "equality-diff");
}

// ---- The sweep's smaller gaps (2026-10-22) ----
// Spec `docs/superpowers/specs/2026-10-22-elya-one-shot-words-and-sweep-gaps-design.md`.
//
// Negative controls (each reverted, `cmp` after):
// - K1, `anf` rule 3 off: the 13 CTOR_VALUES rows fail as before, the seven
//   monomorphic ones by SIGSEGV.
// - K2, rule 4 without its capture check: the `anf` capture test fails, and the
//   compiler panics in inkwell on w6 (an `Int` called).
// - K3, rule 4 off: w1-w4 are refused by name again.
// - K4, the evaluator's old label: `effects_run.rs`'s E0425 test fails.

/// D1: the sweep's four `both-fail-differently` programs. A second resume of a
/// one-shot continuation fails in the same words on both sides: the native
/// line after `elya: ` is the evaluator diagnostic's message, the sweep's rule
/// for "the same failure".
const ONE_SHOT_TRAPS: &[(&str, &str)] = &[
    (
        "o1-a-one-shot-effect",
        "effect S { fn get() -> Int }\n\
         fn body() -> Int { get() }\n\
         pub fn main() -> Int { handle { body() } with { S.get() -> resume(1) + resume(2)  return(r) -> r } }\n",
    ),
    (
        "o2-a-multi-effect-under-a-plain-with",
        "effect multi Flip { fn flip() -> Bool }\n\
         fn choose() { if flip() { 1 } else { 2 } }\n\
         pub fn main() -> Int { handle choose() with { Flip.flip() -> resume(True) + resume(False) } }\n",
    ),
    (
        "o3-a-one-shot-saved-in-a-multi-shot-segment",
        "effect multi Flip { fn flip() -> Bool }\n\
         effect Ask { fn ask() -> Int }\n\
         fn inner() {\n\
         \x20 handle { let x = ask()  x + 1 } with { Ask.ask() -> if flip() { resume(5) } else { resume(6) } }\n\
         }\n\
         pub fn main() -> Int { handle inner() with multi { Flip.flip() -> resume(True) * 1000 + resume(False) } }\n",
    ),
    (
        "o4-after-a-discarded-first-resume",
        "effect Twice {\n\
         \x20 fn t() -> Int\n\
         }\n\
         fn body() { t() }\n\
         pub fn main() {\n\
         \x20 let _ = handle body() with { Twice.t() -> { let _ = resume(1)  resume(2) } }\n\
         \x20 io.println(\"unreached\")\n\
         }\n",
    ),
];

#[test]
fn the_one_shot_trap_says_the_same_words_on_both_sides() {
    let dir = temp_dir("one-shot-words");
    let mut failures = Vec::new();
    for (tag, src) in ONE_SHOT_TRAPS {
        let session = Session::new();
        let (m, pd) = parse_module(&session, src);
        assert!(pd.is_empty(), "{tag}: parse: {pd:?}");
        let msg = match elya::eval::run_module_value(&m) {
            Err(e) if e.diag.code == "E0425" => e.diag.message.clone(),
            Err(e) => panic!("{tag}: the evaluator must fail with E0425, got {e:?}"),
            Ok((_, v)) => panic!("{tag}: the evaluator must fail, got {v:?}"),
        };
        let exe = compile_and_link(&lower_src(src), &dir, tag);
        let out = native(&exe).output().expect("run produced binary");
        let stderr = String::from_utf8_lossy(&out.stderr);
        let line = stderr
            .lines()
            .find_map(|l| l.strip_prefix("elya: "))
            .unwrap_or("");
        if out.status.code() != Some(1)
            || line != msg
            || msg != "a one-shot continuation was resumed twice"
            || !out.stdout.is_empty()
        {
            failures.push(format!(
                "{tag}: native {:?} {line:?}, evaluator {msg:?}",
                out.status
            ));
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(failures.is_empty(), "{failures:#?}");
}

/// D2: a constructor with fields used as a function value. Before, a
/// monomorphic one was a field-less construction typed as a function (a call
/// jumped through its tag word: SIGSEGV, v1-v3, v8-v10, v13) and a parametric
/// one missed the instance table ("parametric ADT", v4-v7, v11, v12). The
/// values are the evaluator's, measured first.
const CTOR_VALUES: &[(&str, &str, &str)] = &[
    (
        "v1-a-constructor-through-a-let",
        "type W { Wrap(Int) }\n\
         pub fn main() -> Int { let f = Wrap  match f(5) { Wrap(h) -> h } }\n",
        "5",
    ),
    (
        "v2-a-constructor-as-an-argument",
        "type W { Wrap(Int) }\n\
         fn ap(f, x) { f(x) }\n\
         pub fn main() -> Int { match ap(Wrap, 5) { Wrap(h) -> h } }\n",
        "5",
    ),
    (
        "v3-map-with-a-constructor",
        "type W { Wrap(Int) }\n\
         type List(a) { Nil, Cons(a, List(a)) }\n\
         fn map(xs, f) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
         fn sum(xs) { match xs { Nil -> 0  Cons(Wrap(h), t) -> h + sum(t) } }\n\
         pub fn main() -> Int { sum(map(Cons(1, Cons(2, Nil)), Wrap)) }\n",
        "3",
    ),
    (
        "v4-map-with-some",
        "type Option(a) { None, Some(a) }\n\
         type List(a) { Nil, Cons(a, List(a)) }\n\
         fn map(xs, f) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
         fn sum(xs) { match xs { Nil -> 0  Cons(Some(h), t) -> h + sum(t)  Cons(None, t) -> sum(t) } }\n\
         pub fn main() -> Int { sum(map(Cons(1, Cons(2, Nil)), Some)) }\n",
        "3",
    ),
    (
        "v5-some-through-a-let",
        "type Option(a) { None, Some(a) }\n\
         pub fn main() -> Int { let f = Some  match f(3) { Some(x) -> x  None -> 0 } }\n",
        "3",
    ),
    (
        "v6-a-two-field-generic-constructor",
        "type Pair(a, b) { P(a, b) }\n\
         fn ap2(f, x, y) { f(x, y) }\n\
         pub fn main() -> Int { match ap2(P, 3, True) { P(x, b) -> if b { x } else { 0 } } }\n",
        "3",
    ),
    (
        "v7-cons-as-a-fold-step",
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn foldr(f, z, l) { match l { Nil -> z  Cons(h, t) -> f(h, foldr(f, z, t)) } }\n\
         fn range(a, b) { if a >= b { Nil } else { Cons(a, range(a + 1, b)) } }\n\
         fn sum(l) { match l { Nil -> 0  Cons(h, t) -> h + sum(t) } }\n\
         pub fn main() -> Int { sum(foldr(Cons, Nil, range(0, 100))) }\n",
        "4950",
    ),
    (
        "v8-a-constructor-captured-by-a-closure",
        "type W { Wrap(Int) }\n\
         pub fn main() -> Int {\n\
         \x20 let w = Wrap\n\
         \x20 let mk = fn(n) { w(n * 2) }\n\
         \x20 match mk(21) { Wrap(h) -> h }\n\
         }\n",
        "42",
    ),
    (
        "v9-a-constructor-under-a-handle",
        "type W { Wrap(Int) }\n\
         effect L { fn lg(x: Int) -> Int }\n\
         fn ap(f, x) { f(lg(x)) }\n\
         pub fn main() -> Int { handle { match ap(Wrap, 4) { Wrap(h) -> h } } with { L.lg(v) -> resume(v + 1)  return(r) -> r * 10 } }\n",
        "50",
    ),
    (
        "v10-a-constructor-joined-with-an-effectful-lambda",
        "type W { Wrap(Int) }\n\
         effect L { fn lg(x: Int) -> Int }\n\
         fn ap(f, x) { f(x) }\n\
         fn pick(c) { if c { Wrap } else { fn(n) { Wrap(lg(n)) } } }\n\
         pub fn main() -> Int {\n\
         \x20 handle { match ap(pick(True), 4) { Wrap(a) -> match ap(pick(False), 4) { Wrap(b) -> a * 100 + b } } } with { L.lg(v) -> resume(v + 1)  return(r) -> r }\n\
         }\n",
        "405",
    ),
    (
        "v11-some-of-some",
        "type Option(a) { None, Some(a) }\n\
         type List(a) { Nil, Cons(a, List(a)) }\n\
         fn map(xs, f) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
         fn sum(xs) { match xs { Nil -> 0  Cons(Some(Some(h)), t) -> h + sum(t)  Cons(_, t) -> sum(t) } }\n\
         pub fn main() -> Int { sum(map(map(Cons(1, Cons(2, Cons(3, Nil))), Some), Some)) }\n",
        "6",
    ),
    (
        "v12-five-thousand-somes",
        "type Option(a) { None, Some(a) }\n\
         type List(a) { Nil, Cons(a, List(a)) }\n\
         fn map(xs, f) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
         fn range(a, b) { if a >= b { Nil } else { Cons(a, range(a + 1, b)) } }\n\
         fn sum(xs) { match xs { Nil -> 0  Cons(Some(h), t) -> h + sum(t)  Cons(None, t) -> sum(t) } }\n\
         pub fn main() -> Int { sum(map(range(0, 5000), Some)) }\n",
        "12497500",
    ),
    (
        "v13-a-two-field-constructor",
        "type P { P(Int, Int) }\n\
         fn ap2(f, x, y) { f(x, y) }\n\
         pub fn main() -> Int { let mk = P  match ap2(mk, 3, 4) { P(a, b) -> a * 10 + b } }\n",
        "34",
    ),
];

#[test]
fn constructors_used_as_values_run_natively() {
    run_value_corpus(CTOR_VALUES, "ctor-values");
}

#[test]
fn constructors_used_as_values_match_the_evaluator() {
    run_differential_corpus(CTOR_VALUES, "ctor-values-diff");
}

/// D3: an alias of a variable (`let mk2 = mk`). An alias of an effect-polymorphic
/// local, or an alias of an alias, used at a user effect was refused by name
/// ("effect-polymorphic function used at a user effect", w1-w4): the alias's own
/// binding referenced its target at the general row, so nothing was cloned at
/// the row its uses need. w5-w8 already ran; they guard the substitution (w6:
/// the target rebound under the alias, so the `let` is kept).
///
/// w9-w21 are the review's (2026-10-23), each right natively before the slice
/// and wrong or refused with the first version of rule 4:
/// - w9-w13: an upcast at or after the alias (5 for 15; refusals);
/// - w14-w21: an alias of a local that shadows a top-level function (2004 for
///   1007, a compiler panic in w16 and w21), where closure conversion resolves
///   the renamed name to the top-level function.
///
/// Rule 4 now renames only an alias of a row-polymorphic local whose name no
/// top-level function has.
const ALIASES: &[(&str, &str, &str)] = &[
    (
        "w1-an-alias-of-a-row-polymorphic-local",
        "effect S { fn get() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 let mk = fn(n) { fn(g) { g() + n } }\n\
         \x20 let mk2 = mk\n\
         \x20 let k = mk2(1)\n\
         \x20 handle { k(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n",
        "8",
    ),
    (
        "w2-two-aliases-at-two-rows",
        "effect S { fn get() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 let ap = fn(g) { g() + 1 }\n\
         \x20 let ap2 = ap\n\
         \x20 let ap3 = ap2\n\
         \x20 let a = ap3(fn() { 5 })\n\
         \x20 let b = handle { ap3(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
         \x20 a * 100 + b\n\
         }\n",
        "608",
    ),
    (
        "w3-an-alias-of-an-alias-of-a-top-level-function",
        "effect S { fn get() -> Int }\n\
         fn twice(g) { g() + g() }\n\
         pub fn main() -> Int {\n\
         \x20 let t = twice\n\
         \x20 let t2 = t\n\
         \x20 handle { t2(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n",
        "14",
    ),
    (
        "w4-an-alias-inside-a-lambda",
        "effect S { fn get() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 let ap = fn(g) { g() + 1 }\n\
         \x20 let run = fn(h) { let a = ap  a(h) }\n\
         \x20 handle { run(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n",
        "8",
    ),
    (
        "w5-a-type-only-alias",
        "pub fn main() -> Int { let id = fn(x) { x }  let id2 = id  id2(1) + (if id2(True) { 10 } else { 0 }) }\n",
        "11",
    ),
    (
        "w6-an-alias-whose-target-is-rebound",
        "pub fn main() -> Int {\n\
         \x20 let ap = fn(g) { g() + 1 }\n\
         \x20 let ap2 = ap\n\
         \x20 let r = { let ap = 5  ap2(fn() { ap }) }\n\
         \x20 r\n\
         }\n",
        "6",
    ),
    (
        "w7-one-alias-at-two-effects",
        "effect S { fn get() -> Int }\n\
         effect T { fn tick() -> Int }\n\
         fn twice(g) { g() + g() }\n\
         pub fn main() -> Int {\n\
         \x20 let t = twice\n\
         \x20 let a = handle { t(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
         \x20 let b = handle { t(fn() { tick() }) } with { T.tick() -> resume(100)  return(r) -> r }\n\
         \x20 a + b + t(fn() { 1 })\n\
         }\n",
        "216",
    ),
    (
        "w8-an-alias-of-a-parameter",
        "effect S { fn get() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 let ap = fn(g) { g() + 1 }\n\
         \x20 let k = fn(f) { let f2 = f  f2(fn() { get() }) }\n\
         \x20 handle { k(ap) } with { S.get() -> resume(7)  return(r) -> r }\n\
         }\n",
        "8",
    ),
    (
        "w9-an-alias-of-a-top-level-function-then-an-annotated-upcast",
        "effect E { fn e() -> Int }\n\
         fn mk() -> fn() -> Int { fn() { 5 } }\n\
         pub fn main() -> Int {\n\
         \x20 let m = mk\n\
         \x20 let q: fn() -> fn() / {E} -> Int = m\n\
         \x20 handle { let r = q()  r() + e() } with { E.e() -> resume(10) }\n\
         }\n",
        "15",
    ),
    (
        "w10-an-alias-of-a-top-level-function-then-an-upcast-by-a-join",
        "effect E { fn e() -> Int }\n\
         fn mk() -> fn() -> Int { fn() { 5 } }\n\
         pub fn main() -> Int {\n\
         \x20 let m = mk\n\
         \x20 let g = if True { m() } else { fn() { e() } }\n\
         \x20 handle { g() + e() } with { E.e() -> resume(10) }\n\
         }\n",
        "15",
    ),
    (
        "w11-alias-upcasts-throughout-main",
        "effect E { fn e() -> Int }\n\
         fn mk(n: Int) -> fn() -> Int { fn() { n } }\n\
         fn pick(c: Bool) -> Int {\n\
         \x20 let m = mk\n\
         \x20 let g = if c { m(5) } else { fn() { e() } }\n\
         \x20 g() + 1\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { pick(True) * 100 + pick(False) } with { E.e() -> resume(10) }\n\
         }\n",
        "611",
    ),
    (
        "w12-an-annotated-upcast-alias-of-a-local",
        "effect E { fn e() -> Int }\n\
         pub fn main() -> Int {\n\
         \x20 let p: fn(Int) -> Int = fn(x) { x + 1 }\n\
         \x20 let f: fn(Int) / {E} -> Int = p\n\
         \x20 handle { f(1) + e() } with { E.e() -> resume(10) }\n\
         }\n",
        "12",
    ),
    (
        "w13-an-annotated-upcast-alias-of-a-top-level-function",
        "effect E { fn e() -> Int }\n\
         fn inc(x: Int) -> Int { x + 1 }\n\
         pub fn main() -> Int {\n\
         \x20 let f: fn(Int) / {E} -> Int = inc\n\
         \x20 handle { f(1) + e() } with { E.e() -> resume(10) }\n\
         }\n",
        "12",
    ),
    (
        "w14-an-alias-of-a-local-that-shadows-a-top-level-function",
        "fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 let inc = fn(z) { z * 2 }\n\
         \x20 let g = inc\n\
         \x20 let f = fn(q) { g(q) }\n\
         \x20 a + f(3)\n\
         }\n",
        "1007",
    ),
    (
        "w15-an-alias-of-a-return-binder-that-shadows",
        "type W { Wrap(Int) }\n\
         fn unw(w) { match w { Wrap(v) -> v } }\n\
         fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 match Wrap(5) { Wrap(inc) -> { let g = inc  let f = fn(q) { g + q }  a + f(3) } }\n\
         }\n",
        "1009",
    ),
    (
        "w16-an-alias-of-a-shadowing-local-in-a-handle",
        "effect S { fn get() -> Int }\n\
         fn ap(g, x) { g(x) }\n\
         fn mk(n) { fn(g) { g() + n } }\n\
         pub fn main() -> Int {\n\
         \x20 let a = mk(1)(fn() { 1 })\n\
         \x20 a + handle { let mk = fn(z) { z * 2 + 1 }  let x = mk  if True { 0 } else { ap(x, 2) } } with { S.get() -> resume(3) }\n\
         }\n",
        "2",
    ),
    (
        "w17-an-alias-of-a-lambda-parameter-that-shadows",
        "fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 let run = fn(inc) { let g = inc  let f = fn(q) { g(q) }  f(3) }\n\
         \x20 a + run(fn(z) { z * 2 })\n\
         }\n",
        "1007",
    ),
    (
        "w18-an-alias-of-a-pattern-binder-that-shadows",
        "type B { B(fn(Int) -> Int) }\n\
         fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 match B(fn(z) { z * 2 }) { B(inc) -> { let g = inc  let f = fn(q) { g(q) }  a + f(3) } }\n\
         }\n",
        "1007",
    ),
    (
        "w19-an-alias-of-a-clause-parameter-that-shadows",
        "effect Ask { fn ask(f: fn(Int) -> Int) -> Int }\n\
         fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 handle { ask(fn(z) { z * 2 }) } with { Ask.ask(inc) -> { let g = inc  let f = fn(q) { g(q) }  resume(a + f(3)) } }\n\
         }\n",
        "1007",
    ),
    (
        "w20-an-alias-of-a-local-shadowing-twice",
        "fn twice(g) { g() + g() }\n\
         pub fn main() -> Int {\n\
         \x20 let a = twice(fn() { 1 })\n\
         \x20 let twice = fn(g) { g() * 100 }\n\
         \x20 let t = twice\n\
         \x20 let h = fn() { t(fn() { 2 }) }\n\
         \x20 a + h()\n\
         }\n",
        "202",
    ),
    (
        "w21-the-fuzzed-seed-1017",
        "effect S { fn get() -> Int }\n\
         effect multi C { fn ch() -> Bool }\n\
         type W { Wrap(Int) }\n\
         type P2 { P2(Int, Int) }\n\
         type Q4 { Q4(Int, Int, Int, Int) }\n\
         type Q5 { Q5(Int, Int, Int, Int, Int) }\n\
         type Option(a) { None, Some(a) }\n\
         type List(a) { Nil, Cons(a, List(a)) }\n\
         type Pair(a, b) { Pr(a, b) }\n\
         fn inc(x) { x + 1 }\n\
         fn dbl(x: Int) -> Int { x * 2 }\n\
         fn ap(g, x) { g(x) }\n\
         fn twice(g) { g() + g() }\n\
         fn mk(n) { fn(g) { g() + n } }\n\
         fn unw(w) { match w { Wrap(v) -> v } }\n\
         fn unp(p) { match p { P2(a, b) -> a * 3 + b } }\n\
         fn unq(q) { match q { Q4(a, b, c, d) -> a + b * 2 + c * 3 + d * 4 } }\n\
         fn unq5(q) { match q { Q5(a, b, c, d, e) -> a + b + c + d + e * 5 } }\n\
         fn uns(o) { match o { None -> 0  Some(v) -> v } }\n\
         fn sum(l) { match l { Nil -> 0  Cons(h, t) -> (h + sum(t)) % 1000 } }\n\
         fn map(l, f) { match l { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
         fn ap4(f, a, b, c, d) { f(a, b, c, d) }\n\
         pub fn main() -> Int {\n\
         \x20 let base0 = inc(1) + dbl(1) + twice(fn() { 1 }) + mk(1)(fn() { 1 }) + ap(inc, 1) + unw(Wrap(1))\n\
         \x20 base0 + handle { handle { uns(Some({ let g = { let mk = fn(z) { z * 2 + 1 }  let x = mk  if if unp(P2(1, 7)) > 4 { 0 + ((7) * 3 + (4)) % 1000 } else { ap(x, 2) } > 4 { 0 + (fn(g) { g() * 3 })(fn() { { let ap = 1  ap } }) } else { { let y = inc  let k = y  0 + ((1) * 3 + (6)) % 1000 } } }  { let ap = ap(inc, mk(inc(g))(fn() { if ch() { 8 } else { g } }))  if if { let h = 9  ap } > 4 { { let x = Q4  ap } } else { mk(g)(fn() { 1 }) } > 4 { inc(if ap > 4 { 7 } else { 1 }) } else { g } } })) } with { S.get() -> resume(3) } } with multi { C.ch() -> (resume(True) + resume(False)) % 1000 }\n\
         }\n",
        "27",
    ),
];

#[test]
fn aliases_run_natively() {
    run_value_corpus(ALIASES, "aliases");
}

#[test]
fn aliases_match_the_evaluator() {
    run_differential_corpus(ALIASES, "aliases-diff");
}

// ---- Captures by scope, and a wrapped function's result (2026-10-23) ----
// Spec `docs/superpowers/specs/2026-10-23-elya-scope-captures-and-wrapper-results-design.md`.

/// D1: a local that shadows a top-level function, where the top-level name is
/// ALSO used free in the same function (`inc(1)` before `let inc = ..`). Closure
/// conversion left every such name out of a lambda's captures: h1 and h4 gave
/// 2004 (the top-level `inc` ran), h8 and h9 were refused, q1 and p0 panicked the
/// compiler. The other rows already agreed and guard the rule.
const SHADOWED_TOP_LEVEL: &[(&str, &str, &str)] = &[
    (
        "h1-a-shadowing-let-captured-by-a-lambda",
        "fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 let inc = fn(z) { z * 2 }\n\
         \x20 let f = fn(q) { inc(q) }\n\
         \x20 a + f(3)\n\
         }\n",
        "1007",
    ),
    (
        "h2-a-shadowing-parameter",
        "fn inc(x) { x + 1000 }\n\
         fn run(inc) { let f = fn(q) { inc(q) }  f(3) }\n\
         pub fn main() -> Int { inc(1) + run(fn(z) { z * 2 }) }\n",
        "1007",
    ),
    (
        "h3-a-shadowing-parameter-also-called-directly",
        "fn inc(x) { x + 1000 }\n\
         fn run(inc) { let a = inc(1)  let f = fn(q) { inc(q) }  a + f(3) }\n\
         pub fn main() -> Int { run(fn(z) { z * 2 }) }\n",
        "8",
    ),
    (
        "h4-a-shadowing-pattern-binder-captured",
        "type B { B(fn(Int) -> Int) }\n\
         fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 match B(fn(z) { z * 2 }) { B(inc) -> { let f = fn(q) { inc(q) }  a + f(3) } }\n\
         }\n",
        "1007",
    ),
    (
        "h5-a-shadowing-let-in-a-handle-body",
        "effect S { fn get() -> Int }\n\
         fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 let inc = fn(z) { z * 2 }\n\
         \x20 a + handle { inc(get()) } with { S.get() -> resume(3) }\n\
         }\n",
        "1007",
    ),
    (
        "h6-a-shadowing-let-in-a-clause",
        "effect S { fn get() -> Int }\n\
         fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 let inc = fn(z) { z * 2 }\n\
         \x20 a + handle { get() } with { S.get() -> resume(inc(3)) }\n\
         }\n",
        "1007",
    ),
    (
        "h7-a-shadowing-let-in-a-return-clause",
        "effect S { fn get() -> Int }\n\
         fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 let inc = fn(z) { z * 2 }\n\
         \x20 a + handle { get() } with { S.get() -> resume(3)  return(r) -> inc(r) }\n\
         }\n",
        "1007",
    ),
    (
        "h8-a-shadowing-clause-parameter-captured",
        "effect S { fn get(n: Int) -> Int }\n\
         fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 a + handle { get(3) } with { S.get(inc) -> { let f = fn(q) { inc + q }  resume(f(0)) } }\n\
         }\n",
        "1004",
    ),
    (
        "h9-a-shadowing-int-captured",
        "fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 let inc = 7\n\
         \x20 let f = fn(q) { inc + q }\n\
         \x20 a + f(3)\n\
         }\n",
        "1011",
    ),
    (
        "h10-a-shadowing-let-with-no-free-use",
        "fn inc(x) { x + 1000 }\n\
         pub fn main() -> Int {\n\
         \x20 let inc = fn(z) { z * 2 }\n\
         \x20 let f = fn(q) { inc(q) }\n\
         \x20 f(3)\n\
         }\n",
        "6",
    ),
    (
        "q1-a-shadowing-let-passed-to-map-in-a-handle",
        "effect S { fn get() -> Int }\n\
         type W { Wrap(Int) }\n\
         type List(a) { Nil, Cons(a, List(a)) }\n\
         fn inc(x) { x + 1 }\n\
         fn unw(w) { match w { Wrap(v) -> v } }\n\
         fn sum(l) { match l { Nil -> 0  Cons(h, t) -> (h + sum(t)) % 1000 } }\n\
         fn map(l, f) { match l { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
         pub fn main() -> Int {\n\
         \x20 let a = inc(1)\n\
         \x20 a + handle { let inc = fn(z) { Wrap(z + 7) }  sum(map(map(Cons(3, Nil), inc), unw)) + get() } with { S.get() -> resume(3) }\n\
         }\n",
        "15",
    ),
    (
        "p0-a-shadowing-chain-in-a-handle",
        "effect S { fn get() -> Int }\n\
         fn mk(n) { fn(g) { g() + n } }\n\
         pub fn main() -> Int { handle { let mk = mk  let mk = fn(z) { z * 2 + 1 }  0 + (fn(g, z) { g(z) + 1 })(mk, 3) } with { S.get() -> resume(3) } }\n",
        "8",
    ),
];

#[test]
fn shadowed_top_level_names_run_natively() {
    run_value_corpus(SHADOWED_TOP_LEVEL, "shadowed-top-level");
}

#[test]
fn shadowed_top_level_names_match_the_evaluator() {
    run_differential_corpus(SHADOWED_TOP_LEVEL, "shadowed-top-level-diff");
}

/// D2: a top-level function wrapped as a value (`anf` rule 2) at a type whose
/// RESULT is upcast. The wrapper's inner call was typed with the use's result,
/// an effectful closure, while the function returns a direct one: u1-u3 gave 5,
/// u4 a pointer, u5 3. u6 (a local lambda) already agreed. u7 and u8 are the
/// review's: the wrapper is inside a row-polymorphic function, so at `anf` time
/// the use's result has an open row (5 for 15, 1505 for 1515).
const WRAPPER_RESULTS: &[(&str, &str, &str)] = &[
    (
        "u1-a-wrapped-result-at-a-parameter",
        "effect E { fn e() -> Int }\n\
         fn mk() -> fn() -> Int { fn() { 5 } }\n\
         fn ap2(f: fn() -> fn() / {E} -> Int) -> Int { let r = f()  r() + e() }\n\
         pub fn main() -> Int {\n\
         \x20 handle { ap2(mk) } with { E.e() -> resume(10) }\n\
         }\n",
        "15",
    ),
    (
        "u2-a-wrapped-result-joined-by-an-if",
        "effect E { fn e() -> Int }\n\
         fn mk() -> fn() -> Int { fn() { 5 } }\n\
         fn pick(c) { if c { mk } else { fn() { fn() { e() } } } }\n\
         pub fn main() -> Int { handle { let r = pick(True)()  let s = pick(False)()  r() * 100 + s() } with { E.e() -> resume(10) } }\n",
        "510",
    ),
    (
        "u3-a-wrapped-result-in-a-field",
        "effect E { fn e() -> Int }\n\
         type B { B(fn() -> fn() / {E} -> Int) }\n\
         fn mk() -> fn() -> Int { fn() { 5 } }\n\
         fn open(b) { match b { B(f) -> { let r = f()  r() + e() } } }\n\
         pub fn main() -> Int { handle { open(B(mk)) } with { E.e() -> resume(10) } }\n",
        "15",
    ),
    (
        "u4-two-wrapped-result-layers",
        "effect E { fn e() -> Int }\n\
         fn mk() -> fn() -> fn() -> Int { fn() { fn() { 5 } } }\n\
         fn ap(f: fn() -> fn() -> fn() / {E} -> Int) -> Int { let r = f()  let s = r()  s() + e() }\n\
         pub fn main() -> Int { handle { ap(mk) } with { E.e() -> resume(10) } }\n",
        "15",
    ),
    (
        "u5-a-wrapped-result-with-a-parameter",
        "effect E { fn e() -> Int }\n\
         fn mk(n) -> fn(Int) -> Int { fn(x) { x + n } }\n\
         fn ap(f: fn(Int) -> fn(Int) / {E} -> Int) -> Int { let r = f(1)  r(2) + e() }\n\
         pub fn main() -> Int { handle { ap(mk) } with { E.e() -> resume(10) } }\n",
        "13",
    ),
    (
        "u6-the-same-through-a-local-lambda",
        "effect E { fn e() -> Int }\n\
         fn mk() -> fn() -> Int { fn() { 5 } }\n\
         fn ap2(f: fn() -> fn() / {E} -> Int) -> Int { let r = f()  r() + e() }\n\
         pub fn main() -> Int { let m = fn() { mk() }  handle { ap2(m) } with { E.e() -> resume(10) } }\n",
        "15",
    ),
    (
        "u7-a-wrapper-inside-a-row-polymorphic-function",
        "effect E { fn e() -> Int }\n\
         fn mk() -> fn() -> Int { fn() { 5 } }\n\
         fn call2(f, g) { let r = f()  r() + g() }\n\
         fn wrap_it(g) { call2(mk, g) }\n\
         pub fn main() -> Int { handle { wrap_it(fn() { e() }) } with { E.e() -> resume(10) } }\n",
        "15",
    ),
    (
        "u8-the-same-at-two-rows",
        "effect E { fn e() -> Int }\n\
         fn mk() -> fn() -> Int { fn() { 5 } }\n\
         fn call2(f, g) { let r = f()  r() + g() }\n\
         fn wrap_it(g) { call2(mk, g) }\n\
         pub fn main() -> Int { wrap_it(fn() { 10 }) * 100 + handle { wrap_it(fn() { e() }) } with { E.e() -> resume(10) } }\n",
        "1515",
    ),
];

#[test]
fn wrapped_function_results_run_natively() {
    run_value_corpus(WRAPPER_RESULTS, "wrapper-results");
}

#[test]
fn wrapped_function_results_match_the_evaluator() {
    run_differential_corpus(WRAPPER_RESULTS, "wrapper-results-diff");
}
