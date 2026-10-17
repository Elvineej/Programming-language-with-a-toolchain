//! Unary operators natively, and explicit wrapping and bit arithmetic
//! (2026-10-17, spec `docs/superpowers/specs/2026-10-17-elya-unary-and-wrapping-design.md`).
//! Every program is compiled, linked and RUN, and its outcome compared with the
//! evaluator's: the same text and value, or the same failure in the same words.
//!
//! Negative controls (each reverted; the file compared byte for byte after):
//! - K1, `-e` lowered as `0 + e` (core.rs): `unary_operators_agree` fails on its
//!   first program (235 natively, 365 in the evaluator).
//! - K2, `int.shr` compiled as the logical shift: `wrapping_and_bits_agree` fails on
//!   `int.shr(0 - 16, 2) * 10`, natively an overflow.
//! - K3, the shift range check off: `int.shl(1, 64)` answers natively where the
//!   evaluator fails by name.

use elya::core::lower_module;
use elya::parse::parse_module;
use elya::Session;
use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-arith-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn lower(src: &str) -> elya::core::CoreModule {
    if let Err(e) = elya::check_source("t.elya", src) {
        panic!("the front end rejected the program: {e}\n{src}");
    }
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&session, &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    lower_module(&m, &table).unwrap_or_else(|e| panic!("core lowering failed: {e:?}"))
}

/// What a run printed, and how it ended: main's value, or the failure's message.
#[derive(Debug, PartialEq)]
struct Outcome {
    text: String,
    end: Result<String, String>,
}

fn evaluate(src: &str) -> Outcome {
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (interp, r) = elya::eval::run_module_keeping(&m, elya::eval::World::Live);
    let end = match r {
        Ok(elya::eval::Value::Int(n)) => Ok(n.to_string()),
        Ok(other) => panic!("main must be an Int, got {other:?}"),
        Err(e) => Err(e.diag.message),
    };
    Outcome {
        text: interp.output().to_string(),
        end,
    }
}

fn run_native(src: &str, tag: &str) -> Outcome {
    let core = lower(src);
    let dir = temp_dir(tag);
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    if let Err(e) = elya_codegen::compile_module(&core, &obj) {
        panic!("{tag}: {e}");
    }
    elya_codegen::link(&obj, &exe).expect("link");
    let out = Command::new(&exe).output().expect("run the binary");
    std::fs::remove_dir_all(&dir).ok();
    let stdout = String::from_utf8(out.stdout).expect("UTF-8 stdout");
    let stderr = String::from_utf8(out.stderr).expect("UTF-8 stderr");
    if out.status.success() {
        assert!(stderr.is_empty(), "{tag}: stderr: {stderr}");
        let body = stdout
            .strip_suffix('\n')
            .unwrap_or_else(|| panic!("{tag}: no value line: {stdout:?}"));
        let (text, value) = match body.rsplit_once('\n') {
            Some((t, v)) => (format!("{t}\n"), v.to_string()),
            None => (String::new(), body.to_string()),
        };
        Outcome {
            text,
            end: Ok(value),
        }
    } else {
        assert_eq!(out.status.code(), Some(1), "{tag}: {stderr}");
        let msg = stderr
            .strip_prefix("elya: ")
            .and_then(|s| s.strip_suffix('\n'))
            .unwrap_or_else(|| panic!("{tag}: not a named failure: {stderr:?}"));
        Outcome {
            text: stdout,
            end: Err(msg.to_string()),
        }
    }
}

/// The program natively and in the evaluator: the same outcome, returned.
fn agree(src: &str, tag: &str) -> Outcome {
    let want = evaluate(src);
    let got = run_native(src, tag);
    assert_eq!(got, want, "{tag}: native vs evaluator\n{src}");
    got
}

const MIN: &str = "(0 - 9223372036854775807 - 1)";

#[test]
fn unary_operators_agree() {
    let progs: Vec<String> = vec![
        "fn neg(x: Int) -> Int { -x }\npub fn main() -> Int { neg(5) + neg(0 - 7) * 10 + -(-3) * 100 }\n".into(),
        "fn not(b: Bool) -> Bool { !b }\n\
         pub fn main() -> Int { let a = if not(True) { 1 } else { 2 }  let b = if !!(1 < 2) { 10 } else { 20 }  a + b }\n".into(),
        format!("pub fn main() -> Int {{ -9223372036854775807 - 1 + -{MIN} }}\n"),
        format!("fn neg(x: Int) -> Int {{ -x }}\npub fn main() -> Int {{ io.println(\"before\")  neg({MIN}) }}\n"),
        // In effectful code (the CPS emitter) and in a closure.
        "effect W { fn roll(s: Int) -> Int  fn ok() -> Bool }\n\
         fn game() / {W} -> Int { let a = -roll(6)  if !ok() { a } else { a * 2 } }\n\
         pub fn main() -> Int {\n\
           let f = fn(x: Int) { -x }\n\
           f(handle { game() } with { W.roll(s) -> resume(s + 1)  W.ok() -> resume(False) })\n\
         }\n".into(),
    ];
    for (i, p) in progs.iter().enumerate() {
        agree(p, &format!("unary{i}"));
    }
}

#[test]
fn wrapping_and_bits_agree() {
    let mut progs: Vec<String> = Vec::new();
    for (e, _) in [
        ("int.wrapping_add(9223372036854775807, 1)", ()),
        (&*format!("int.wrapping_sub({MIN}, 1)"), ()),
        ("int.wrapping_mul(9223372036854775807, 2)", ()),
        ("int.wrapping_mul(1099511628211, 1099511628211)", ()),
        ("int.and(12, 10) * 100 + int.or(12, 10)", ()),
        ("int.xor(0 - 1, 5)", ()),
        ("int.shl(1, 63)", ()),
        ("int.shl(3, 0) + int.shr(0 - 16, 2) * 10", ()),
        (
            "int.ushr(0 - 16, 60) + int.ushr(0 - 1, 63) * 100 + int.shr(0 - 1, 63) * 1000",
            (),
        ),
        ("int.shl(1, 64)", ()),
        ("int.shr(1, 0 - 1)", ()),
        ("int.ushr(1, 100)", ()),
    ] {
        progs.push(format!(
            "fn id(x: Int) -> Int {{ x }}\npub fn main() -> Int {{ io.println(\"start\")  id({e}) }}\n"
        ));
    }
    // FNV-1a over a list, and a xorshift generator: the programs D2 is for.
    progs.push(
        "type L { Nil, Cons(Int, L) }\n\
         fn fnv(xs: L, h: Int) -> Int { match xs { Nil -> h  Cons(x, r) -> fnv(r, int.wrapping_mul(int.xor(h, x), 1099511628211)) } }\n\
         fn xs(s: Int) -> Int { let a = int.xor(s, int.shl(s, 13))  let b = int.xor(a, int.ushr(a, 7))  int.xor(b, int.shl(b, 17)) }\n\
         fn run(n: Int, s: Int, acc: Int) -> Int { if n == 0 { int.xor(s, acc) } else { run(n - 1, xs(s), int.wrapping_add(acc, s)) } }\n\
         pub fn main() -> Int {\n\
           let h = fnv(Cons(104, Cons(105, Cons(33, Nil))), int.wrapping_sub(0, 3750763034362895579))\n\
           io.println(int.to_string(h))\n\
           int.xor(h, run(100000, 88172645463325252, 0))\n\
         }\n"
            .to_string(),
    );
    // In effectful code: an operand that performs.
    progs.push(
        "effect W { fn roll(s: Int) -> Int }\n\
         fn g() / {W} -> Int { int.wrapping_mul(roll(6), int.shl(roll(1), roll(2))) }\n\
         pub fn main() -> Int { handle { g() } with { W.roll(s) -> resume(s + 1) } }\n"
            .to_string(),
    );
    progs.push(
        "effect W { fn roll(s: Int) -> Int }\n\
         fn g() / {W} -> Int { int.shr(5, roll(70)) }\n\
         pub fn main() -> Int { handle { g() } with { W.roll(s) -> resume(s) } }\n"
            .to_string(),
    );
    for (i, p) in progs.iter().enumerate() {
        agree(p, &format!("wrap{i}"));
    }
}
