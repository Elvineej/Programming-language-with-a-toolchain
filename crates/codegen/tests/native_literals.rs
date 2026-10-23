//! Literals at the edges, and a layout rule, natively (2026-10-23, spec
//! `docs/superpowers/specs/2026-10-23-elya-literals-and-layout-design.md`).
//! Every program is compiled, linked and RUN, and its outcome compared with the
//! evaluator's: the same text and value, or the same failure in the same words.
use elya::core::lower_module;
use elya::parse::parse_module;
use elya::Session;
use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-literals-{}-{tag}", std::process::id()));
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

#[test]
fn literals_agree() {
    let progs = [
        "pub fn main() -> Int { -9223372036854775808 }\n",
        "pub fn main() -> Int { -9223372036854775808 / 1000000000000 + - 7 }\n",
        // A new line, so the `-` starts the tail (D4); on one line it would subtract.
        "pub fn main() -> Int { io.println(\"before\")\n  -(-9223372036854775808) }\n",
        "fn f(x: Int) -> Int { match x { -1 -> 10  _ -> 20 } }\n\
         pub fn main() -> Int { f(0 - 1) * 100 + f(1) }\n",
        "fn f(x: Int) -> Int { match x {\n  0 -> 5\n  -1 -> 7\n  _ -> 9 } }\n\
         pub fn main() -> Int { f(0 - 1) * 100 + f(0) * 10 + f(3) }\n",
        "fn f(x: Int) -> Int { match x { -9223372036854775808 -> 1  _ -> 2 } }\n\
         pub fn main() -> Int { f(0 - 9223372036854775807 - 1) * 10 + f(0) }\n",
    ];
    for (i, p) in progs.iter().enumerate() {
        agree(p, &format!("lit{i}"));
    }
}

#[test]
fn layout_agrees() {
    let progs = [
        "fn f(x: Int) -> Int {\n  let y = 4\n  -x * y\n}\npub fn main() -> Int { f(3) }\n",
        "fn g(x: Int) -> Int { x + 1 }\n\
         fn f(x: Int) -> Int {\n  let h = g\n  (h)(x)\n}\npub fn main() -> Int { f(3) }\n",
        "fn id(x: Int) -> Int { x }\n\
         pub fn main() -> Int { id({ let a = 1\n  -a }) }\n",
    ];
    for (i, p) in progs.iter().enumerate() {
        agree(p, &format!("layout{i}"));
    }
}
