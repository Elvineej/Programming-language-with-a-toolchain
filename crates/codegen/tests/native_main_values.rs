//! What `main` is worth, natively (2026-10-30, spec
//! `docs/superpowers/specs/2026-10-30-elya-main-values-design.md`): a native
//! binary's last line is `main`'s value as `elya run --value` writes it, for
//! every data type, and a value that holds a function fails in the same words.
//!
//! Negative controls (each reverted; the file compared byte for byte after):
//! - K1, `main`'s type left out of the reflection table (`reflect.rs`): both tests
//!   fail to build, "main's type is not in the reflection table".
//! - K2, no default instance of a generic `main` (`mono.rs`): "no `main`".
//! - K3, the shim's `%lld` line printed after the literal: `True` then `0`.
//!
//! The review's findings, each red before its fix (so the fix's control):
//! - F1, `literal::show` without the constructor's field count: the function test
//!   reads `Ok("Some")` from the evaluator (and `native_any.rs`'s
//!   `showing_a_function_fails_by_name`, `K.k(Yes)`).
//! - F2, `@elya_entry` without its `elya_trace_finish` call: the replay test reads
//!   `asked\nTrue\n`, the line before the unread entry's failure.
//! - L1, `elya_any_show` without its `fflush(stdout)`: the stream test reads the
//!   failure before `out`.

use std::process::Command;

fn no_files(_: &str) -> Option<(String, String)> {
    None
}

/// What `elya run --value` shows: the output, then the value line, or the
/// failure's words.
fn value_run(src: &str) -> (String, Result<String, String>) {
    let o = elya::run_source_outcome_with("main.elya", src, &mut no_files);
    assert!(o.error.is_none(), "{:?}", o.error);
    (o.output, o.line.expect("a value line"))
}

/// `src` compiled and linked: the temporary directory and the binary in it.
fn build(src: &str, tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let (module, diags, _) = elya::front_end_with("main.elya", src, &mut no_files);
    assert!(diags.is_empty(), "{tag}: {diags:?}");
    let session = elya::Session::new();
    let (diags, table) = elya::types::infer_typed_table(&session, &module);
    assert!(diags.is_empty(), "{tag}: {diags:?}");
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let dir = std::env::temp_dir().join(format!("elya-mainv-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).unwrap_or_else(|e| panic!("{tag}: {e}"));
    elya_codegen::link(&obj, &exe).expect("link");
    (dir, exe)
}

/// The native binary's stdout and, on failure, its `elya: ` message.
fn native_run(src: &str, tag: &str) -> (String, Result<String, String>) {
    let (dir, exe) = build(src, tag);
    let out = Command::new(&exe).output().expect("run the binary");
    std::fs::remove_dir_all(&dir).ok();
    let stdout = String::from_utf8(out.stdout).expect("UTF-8 stdout");
    let stderr = String::from_utf8(out.stderr).expect("UTF-8 stderr");
    if out.status.success() {
        let body = stdout
            .strip_suffix('\n')
            .unwrap_or_else(|| panic!("{tag}: {stdout:?}"));
        let (text, line) = match body.rsplit_once('\n') {
            Some((t, v)) => (format!("{t}\n"), v.to_string()),
            None => (String::new(), body.to_string()),
        };
        (text, Ok(line))
    } else {
        assert_eq!(out.status.code(), Some(1), "{tag}: {stderr}");
        let msg = stderr
            .strip_prefix("elya: ")
            .and_then(|s| s.strip_suffix('\n'))
            .unwrap_or_else(|| panic!("{tag}: not a named failure: {stderr:?}"));
        (stdout, Err(msg.to_string()))
    }
}

const DECLS: &str = "type Option(a) { None, Some(a) }\n\
                     type List(a) { Nil, Cons(a, List(a)) }\n\
                     type B { Box(Float) }\n\
                     type F { Holds(fn(Int) -> Int) }\n";

#[test]
fn every_data_main_prints_its_literal() {
    for (body, want) in [
        ("7", "7"),
        ("Unit", "0"),
        ("True", "True"),
        ("1.5", "1.5"),
        // `0.0 *. -1.0`, not `-0.0`: after `io.println(..)` on the same line a
        // `-` is a subtraction.
        ("0.0 *. -1.0", "-0.0"),
        ("\"hi\\n\\\"x\\\"\"", "\"hi\\n\\\"x\\\"\""),
        ("Some(3)", "Some(3)"),
        ("Cons(1, Cons(2, Nil))", "Cons(1, Cons(2, Nil))"),
        ("Box(1.0 /. 0.0)", "Box(inf)"),
        // Generic mains, at their defaults (D3).
        ("None", "None"),
        ("Nil", "Nil"),
    ] {
        let src = format!("{DECLS}pub fn main() {{ io.println(\"out\")  {body} }}\n");
        let (text, line) = value_run(&src);
        assert_eq!(
            (text.as_str(), line.as_deref()),
            ("out\n", Ok(want)),
            "{body}"
        );
        assert_eq!(native_run(&src, "data"), (text, line), "{body} natively");
    }
}

#[test]
fn a_main_that_holds_a_function_fails_by_name() {
    for body in [
        "fn(x: Int) { x }",
        "Holds(fn(x) { x + 1 })",
        "Some(fn(x: Int) { x })",
        // A generic function: instantiated at its default, then refused the same way.
        "fn(x) { x }",
        // A constructor used as a function (the review's F1: the evaluator
        // printed `Some`, a field-less value of the same name).
        "Some",
        "Cons(Some, Nil)",
    ] {
        let src = format!("{DECLS}pub fn main() {{ io.println(\"out\")  {body} }}\n");
        let (text, line) = value_run(&src);
        assert_eq!(text, "out\n", "{body}");
        assert_eq!(
            line,
            Err("main's value is a function, which is not data".to_string()),
            "{body}"
        );
        assert_eq!(native_run(&src, "fns"), (text, line), "{body} natively");
    }
}

/// The review's F2: the value line comes after the replay's check that every
/// entry was read, as in the evaluator, which checks before it writes the line.
/// Natively the line was written first (`False`, then the failure), and a
/// function main failed on its value instead of on the unread entry.
#[test]
fn a_value_line_comes_after_the_replay_check() {
    for (tag, body) in [("bool", "t > 0"), ("fn", "fn(x: Int) { x + t }")] {
        let src =
            format!("pub fn main() {{ let t = time.now_ms()  io.println(\"asked\")  {body} }}\n");
        let trace = "elya trace 1\ntime.now_ms() = 5\ntime.now_ms() = 6\n";
        let want = elya::run_source_replaying("main.elya", &src, trace).unwrap_err();
        let (dir, exe) = build(&src, tag);
        let t = dir.join("t.trace");
        std::fs::write(&t, trace).unwrap();
        let out = Command::new(&exe)
            .env_remove("ELYA_RECORD")
            .env("ELYA_REPLAY", &t)
            .output()
            .expect("run");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "asked\n", "{tag}");
        assert_eq!(out.status.code(), Some(1), "{tag}");
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err
            .strip_prefix("elya: ")
            .and_then(|m| m.strip_suffix('\n'))
            .unwrap_or(&err);
        assert!(msg.contains("unread trace entry"), "{tag}: {msg}");
        assert!(
            want.contains(msg),
            "{tag}: native `{msg}`, evaluator {want}"
        );
    }
}

/// The review's L1: the program's output reaches a shared stream before the
/// failure's words, as from the evaluator.
#[cfg(unix)]
#[test]
fn the_output_comes_before_a_value_failure_on_one_stream() {
    let src = format!("{DECLS}pub fn main() {{ io.println(\"out\")  fn(x: Int) {{ x }} }}\n");
    let (dir, exe) = build(&src, "stream");
    let out = Command::new("sh")
        .arg("-c")
        .arg("\"$0\" 2>&1")
        .arg(&exe)
        .output()
        .expect("run through sh");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "out\nelya: main's value is a function, which is not data\n"
    );
}
