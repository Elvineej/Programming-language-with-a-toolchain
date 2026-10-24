//! Aliases that rebind their target (2026-10-24, spec
//! `docs/superpowers/specs/2026-10-24-elya-alias-renaming-design.md`): `anf`
//! substitutes an alias of a row-polymorphic local by renaming the body's own
//! binders of the target (D1), also when a top-level function shares its name
//! (D2). Each program is compiled, linked, RUN, and compared with the evaluator.
//!
//! Negative controls (each reverted; the file compared byte for byte after):
//! - K1, D1 renaming a `let` binder but not its uses: the captured `mk = 0`
//!   reaches a call as a closure, and the compiler panics (inkwell: a pointer
//!   where an `Int` was expected) in both tests.
//! - K2, D2 off (the old top-level-name guard back): a4 is refused by name
//!   ("effect-polymorphic function used at a user effect").

use elya::core::lower_module;
use elya::parse::parse_module;
use elya::Session;
use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-aliases-{}-{tag}", std::process::id()));
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
fn aliases_that_rebind_their_target_agree() {
    for (tag, src, want) in [
        ("a1", "effect S { fn get() -> Int }\npub fn main() -> Int {\n  let mk = fn(n) { fn(g) { g() + n } }\n  let mk2 = mk\n  let k = { let mk = 0  mk2(1 + mk) }\n  handle { k(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n}\n", "8"),
        ("a2", "effect S { fn get() -> Int }\npub fn main() -> Int {\n  let mk = fn(n) { fn(g) { g() + n } }\n  let mk2 = mk\n  let f = fn(mk) { mk2(mk) }\n  let k = f(1)\n  handle { k(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n}\n", "8"),
        ("a3", "effect S { fn get() -> Int }\ntype B { B(Int) }\npub fn main() -> Int {\n  let mk = fn(n) { fn(g) { g() + n } }\n  let mk2 = mk\n  let k = match B(1) { B(mk) -> mk2(mk) }\n  handle { k(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n}\n", "8"),
        ("a5", "effect S { fn get() -> Int }\npub fn main() -> Int {\n  let mk = fn(n) { fn(g) { g() + n } }\n  let mk2 = mk\n  let k = mk2(1)\n  let j = { let mk = 5  mk2(mk) }\n  handle { k(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r } + j(fn() { 100 })\n}\n", "113"),
    ] {
        assert_eq!(agree(src, tag).end, Ok(want.to_string()), "{tag}");
    }
}

#[test]
fn an_alias_named_like_a_top_level_function_agrees() {
    let a4 = "effect S { fn get() -> Int }\nfn mk(x: Int) -> Int { x }\npub fn main() -> Int {\n  let mk = fn(n) { fn(g) { g() + n } }\n  let mk2 = mk\n  let k = mk2(1)\n  handle { k(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n}\n";
    assert_eq!(agree(a4, "a4").end, Ok("8".into()));
    // The review's shape of 2026-10-22: a shadowing `y` planted inside a lambda,
    // a top-level `y` of another meaning (2004 for 1007 before captures by scope).
    let planted = "effect S { fn get() -> Int }\n\
                   fn y(n: Int) -> Int { n * 1000 }\n\
                   pub fn main() -> Int {\n\
                   \x20 let y = fn(n) { fn(g) { g() + n } }\n\
                   \x20 let x = y\n\
                   \x20 let k = (fn(v) { let y = v  x(y) })(1000)\n\
                   \x20 handle { k(fn() { get() }) } with { S.get() -> resume(7) }\n\
                   }\n";
    assert_eq!(agree(planted, "planted").end, Ok("1007".into()));
}
