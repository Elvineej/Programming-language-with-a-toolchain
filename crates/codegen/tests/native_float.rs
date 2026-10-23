//! `Float` natively (2026-10-23, spec
//! `docs/superpowers/specs/2026-10-23-elya-float-natively-design.md`). Every
//! program is compiled, linked and RUN, and its outcome compared with the
//! evaluator's: the same text and value, or the same failure in the same words.
//! The printer is proven against the evaluator's text on edge values and on
//! random bit patterns through a C harness linked with `runtime.c` (D3).
//!
//! Negative controls (each reverted; `runtime.c` compared byte for byte after):
//! - K1, `elya_eq` comparing float fields by bits (no float masks, D2):
//!   `float_equality_agrees` fails (`Box(nan) == Box(nan)` true natively).
//! - K2, the printer's neighbour step off (D3): the harness finds 45 values
//!   that differ, all next to a power of two (`0060000000000000` first).
//! - K3, `elya_any_check` ignoring a NaN: `with_any_shows_encodes_and_decodes_floats`
//!   (`o == o` is `same` natively, `differs` in the evaluator).
//!
//! Found on the way: `%.*e` rounds a tie half to even, Rust's shortest form half
//! up (2^-25 writes `...95313`); the printer rounds from the exact expansion.
use elya::core::lower_module;
use elya::parse::parse_module;
use elya::Session;
use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-float-{}-{tag}", std::process::id()));
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

/// `show(x)` prints a float's text.
const SHOW: &str = "fn show(x: Float) { io.println(float.to_string(x)) }\n";

#[test]
fn float_arithmetic_agrees() {
    let body = [
        "show(1.5 +. 2.25)",
        "show(1.0 -. 3.5)",
        "show(0.1 +. 0.2)",
        "show(2.5 *. -4.0)",
        "show(1.0 /. 3.0)",
        "show(1.0 /. 0.0)",
        "show(-1.0 /. 0.0)",
        "show(0.0 /. 0.0)",
        "show(-0.0)",
        "show(0.0 -. 0.0)",
        "show(10000000000000000000000.0 *. 10000000000000000000.0)",
        "show(9007199254740993.0)",
        "show(0.0000001 *. 0.0000001)",
    ]
    .join("\n  ");
    let src = format!("{SHOW}pub fn main() -> Int {{\n  {body}\n  0\n}}\n");
    let got = agree(&src, "arith");
    assert!(got.text.contains("0.30000000000000004\n"), "{}", got.text);
}

#[test]
fn float_equality_agrees() {
    let src = "type Box { Box(Float) }\n\
               type Two { Two(Int, Float) }\n\
               type L { Nil, Cons(Float, L) }\n\
               fn b(c: Bool) -> Int { if c { 1 } else { 0 } }\n\
               pub fn main() -> Int {\n\
                 let nan = 0.0 /. 0.0\n\
                 b(1.5 == 1.5) + b(nan == nan) * 10 + b(nan != nan) * 100 + b(-0.0 == 0.0) * 1000 +\n\
                   b(Box(nan) == Box(nan)) * 10000 + b(Box(-0.0) == Box(0.0)) * 100000 +\n\
                   b(Two(1, 2.0) == Two(1, 2.0)) * 1000000 + b(Two(1, 2.0) != Two(1, 2.5)) * 10000000 +\n\
                   b(Cons(1.0, Cons(nan, Nil)) == Cons(1.0, Cons(nan, Nil))) * 100000000\n\
               }\n";
    let got = agree(src, "eq");
    assert_eq!(got.end, Ok("11101101".to_string()));
}

#[test]
fn floats_live_in_fields_captures_and_frames() {
    let src = format!(
        "{SHOW}type P {{ P(Float, Int, Float) }}\n\
         effect Ask {{ fn ask(x: Float) -> Float }}\n\
         fn scale(k: Float) -> fn(Float) -> Float {{ fn(x: Float) {{ x *. k }} }}\n\
         fn twice() / {{Ask}} -> Float {{ let a = ask(1.5)  let b = ask(a)  a +. b }}\n\
         pub fn main() -> Int {{\n\
           let p = P(0.5, 7, -2.0)\n\
           match p {{ P(x, n, y) -> show(x +. y) }}\n\
           let f = scale(3.0)\n\
           show(f(0.25))\n\
           show(handle {{ twice() }} with {{ Ask.ask(x) -> resume(x *. 2.0) }})\n\
           0\n\
         }}\n"
    );
    let got = agree(&src, "live");
    assert_eq!(got.text, "-1.5\n0.75\n9.0\n");
}

#[test]
fn with_any_shows_encodes_and_decodes_floats() {
    let src = "effect Q { fn f(x: Float) -> Float  fn g(x: Float) -> Float }\n\
               fn prog() / {Q} -> Float { let a = f(0.0 /. 0.0)  let b = f(-0.0)  g(a +. b) }\n\
               pub fn main() -> Int {\n\
                 let r = handle { prog() } with any {\n\
                   op(o) -> {\n\
                     io.println(op.show(o))\n\
                     io.println(if o == o { \"same\" } else { \"differs\" })\n\
                     let v = if op.name(o) == \"Q.f\" { op.decode(o, \"inf\") } else { op.decode(o, \" -.5 \") }\n\
                     io.println(op.encode(o, v))\n\
                     resume(v)\n\
                   }\n\
                 }\n\
                 io.println(float.to_string(r))\n\
                 0\n\
               }\n";
    let got = agree(src, "any");
    assert_eq!(
        got.text,
        "Q.f(nan)\ndiffers\ninf\nQ.f(-0.0)\nsame\ninf\nQ.g(inf)\nsame\n-0.5\n-0.5\n"
    );
    for (text, tag) in [
        ("1.", "dot"),
        ("1.2.3", "dots"),
        ("1e5", "exp"),
        ("7", "int"),
    ] {
        let src = format!(
            "effect Q {{ fn f() -> Float }}\n\
             pub fn main() -> Int {{\n\
               let r = handle {{ f() }} with any {{ op(o) -> resume(op.decode(o, \"{text}\")) }}\n\
               io.println(float.to_string(r))\n\
               0\n\
             }}\n"
        );
        agree(&src, tag);
    }
}

/// The C harness: reads one 16-hex-digit bit pattern per line, writes
/// `elya_float_text` of it.
const HARNESS: &str = r#"
#include <stdint.h>
#include <stdio.h>
#include <string.h>
int64_t elya_float_text(int64_t bits, char *out);
int main(void) {
    char line[64];
    char out[1200];
    while (fgets(line, sizeof line, stdin)) {
        unsigned long long b;
        if (sscanf(line, "%llx", &b) != 1) return 2;
        int64_t n = elya_float_text((int64_t)b, out);
        fwrite(out, 1, (size_t)n, stdout);
        fputc('\n', stdout);
    }
    return 0;
}
"#;

#[test]
fn the_printer_matches_the_evaluator() {
    let dir = temp_dir("printer");
    let c = dir.join("harness.c");
    std::fs::write(&c, HARNESS).unwrap();
    let exe = dir.join(format!("harness{}", std::env::consts::EXE_SUFFIX));
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime.c");
    let mut cc = Command::new("clang");
    cc.arg(&c).arg(&runtime).arg("-o").arg(&exe);
    if !cfg!(windows) {
        cc.arg("-lpthread");
    }
    let built = cc.output().expect("spawn clang");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    // Edge values, then random bit patterns (xorshift64*, a fixed seed).
    let mut values: Vec<u64> = [
        0.0f64,
        -0.0,
        1.0,
        -1.0,
        0.1,
        0.2,
        0.3,
        1.5,
        2.0,
        0.5,
        1e-7,
        1e-300,
        1e300,
        f64::MIN_POSITIVE,
        f64::MAX,
        f64::MIN,
        f64::EPSILON,
        9007199254740992.0,
        9007199254740993.0,
        1e21,
        1e22,
        1e23,
        123456789012345680000.0,
        5e-324,
        1.0 / 3.0,
        2.0f64.powi(-1022),
        2.0f64.powi(-1074),
        f64::INFINITY,
        f64::NEG_INFINITY,
    ]
    .iter()
    .map(|x| x.to_bits())
    .collect();
    values.push(f64::NAN.to_bits());
    values.push(f64::NAN.to_bits() | (1 << 63));
    // Powers of two and their neighbours: the uneven-gap cases (D3).
    for e in 0..2046u64 {
        let p = e << 52;
        values.extend([p, p + 1, p.wrapping_sub(1) & !(1 << 63)]);
    }
    let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
    for _ in 0..200_000 {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        values.push(s.wrapping_mul(0x2545_F491_4F6C_DD1D));
    }
    let input: String = values.iter().map(|b| format!("{b:016x}\n")).collect();
    let mut child = Command::new(&exe)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("run the harness");
    // Fed from a thread: the harness's output fills its pipe long before its
    // input is all written.
    let mut stdin = child.stdin.take().unwrap();
    let feeder = std::thread::spawn(move || {
        use std::io::Write;
        stdin.write_all(input.as_bytes()).unwrap();
    });
    let out = child.wait_with_output().unwrap();
    feeder.join().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), values.len());
    let mut bad = Vec::new();
    for (b, got) in values.iter().zip(&lines) {
        let want = elya::literal::float_text(f64::from_bits(*b));
        if *got != want {
            bad.push(format!("{b:016x}: native `{got}`, evaluator `{want}`"));
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(
        bad.is_empty(),
        "{} differ, first: {:?}",
        bad.len(),
        &bad[..bad.len().min(5)]
    );
}
