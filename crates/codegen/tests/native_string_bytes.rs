//! Strings indexed by byte, natively (2026-10-30, spec
//! `docs/superpowers/specs/2026-10-24-elya-trace-text-design.md` D1-D3). Every
//! program is compiled, linked and RUN, and its outcome compared with the
//! evaluator's: the same text and value, or the same failure in the same words.
//!
//! Negative controls (each reverted; `runtime.c` compared byte for byte after):
//! - K1, `elya_str_slice` without its UTF-8 boundary check:
//!   `string_failures_agree_natively` answers 1 where the evaluator fails
//!   ("1..2 splits a UTF-8 character").
//! - K3, `elya_str_byte_at` without its range check: the same test answers 0
//!   (a padding byte) where the evaluator fails ("index 7 is outside ...").

use std::process::Command;

/// What a run printed, and how it ended: main's value, or the failure's message.
#[derive(Debug, PartialEq)]
struct Outcome {
    text: String,
    end: Result<String, String>,
}

fn no_files(_: &str) -> Option<(String, String)> {
    None
}

fn evaluate(src: &str) -> Outcome {
    let (module, diags, _) = elya::front_end_with("main.elya", src, &mut no_files);
    assert!(diags.is_empty(), "{diags:?}");
    let (interp, r) = elya::eval::run_module_keeping(&module, elya::eval::World::Live);
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
    let (module, diags, _) = elya::front_end_with("main.elya", src, &mut no_files);
    assert!(diags.is_empty(), "{tag}: {diags:?}");
    let session = elya::Session::new();
    let (diags, table) = elya::types::infer_typed_table(&session, &module);
    assert!(diags.is_empty(), "{tag}: {diags:?}");
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let dir = std::env::temp_dir().join(format!("elya-strbytes-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).unwrap_or_else(|e| panic!("{tag}: {e}"));
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

fn agree(src: &str, tag: &str) -> Outcome {
    let want = evaluate(src);
    let got = run_native(src, tag);
    assert_eq!(got, want, "{tag}: native vs evaluator\n{src}");
    got
}

#[test]
fn string_builtins_agree_natively() {
    let rows = [
        (
            "bytes",
            "pub fn main() -> Int {\n\
             \x20 let s = \"h\u{e9}llo\"\n\
             \x20 string.length(s) * 1000000 + string.byte_at(s, 1) * 1000 + string.byte_at(s, 2)\n\
             }\n",
            "6195169",
        ),
        (
            "slices",
            "pub fn main() -> Int {\n\
             \x20 let s = \"h\u{e9}llo\"\n\
             \x20 io.println(string.slice(s, 0, 1))\n\
             \x20 io.println(string.slice(s, 1, 3))\n\
             \x20 io.println(\"[\" <> string.slice(s, 6, 6) <> \"]\")\n\
             \x20 io.println(string.slice(s, 0, string.length(s)))\n\
             \x20 string.length(string.slice(s, 3, 6))\n\
             }\n",
            "3",
        ),
        // The CPS emitter: the builtins inside a function that performs, and
        // a slice live across a perform.
        (
            "cps",
            "effect Ask { fn ask() -> String }\n\
             fn count(n: Int) / {Ask} -> Int {\n\
             \x20 let s = ask()\n\
             \x20 let head = string.slice(s, 0, 2)\n\
             \x20 let t = ask()\n\
             \x20 if n == 0 { string.length(head) + string.byte_at(t, 0) } else { string.length(head <> t) + count(n - 1) }\n\
             }\n\
             pub fn main() -> Int { handle { count(3) } with { Ask.ask() -> resume(\"abc\u{e9}\") } }\n",
            "",
        ),
        // Slices through many collections: a 3000-byte string sliced at every
        // offset (4.5 MB of slices; 17 collections natively), each slice's
        // first bytes read while the string stays live.
        (
            "collect",
            "fn grow(s: String, n: Int) -> String { if n == 0 { s } else { grow(s <> int.to_string(n % 10), n - 1) } }\n\
             fn head(s: String, i: Int, acc: Int) -> Int { if i == 8 || i == string.length(s) { acc } else { head(s, i + 1, acc * 3 + string.byte_at(s, i)) } }\n\
             fn walk(s: String, i: Int, acc: Int) -> Int { if i == string.length(s) { acc } else { walk(s, i + 1, (acc + head(string.slice(s, i, string.length(s)), 0, 0)) % 1000003) } }\n\
             pub fn main() -> Int { walk(grow(\"\", 3000), 0, 0) }\n",
            "498929",
        ),
    ];
    for (tag, src, want) in rows {
        let got = agree(src, tag);
        if !want.is_empty() {
            assert_eq!(got.end, Ok(want.to_string()), "{tag}");
        }
    }
}

#[test]
fn string_failures_agree_natively() {
    for (i, call) in [
        "string.byte_at(\"abc\", 7)",
        "string.byte_at(\"abc\", -1)",
        "string.length(string.slice(\"abc\", 2, 9))",
        "string.length(string.slice(\"abc\", 2, 1))",
        "string.length(string.slice(\"abc\", -1, 2))",
        "string.length(string.slice(\"\u{e9}t\u{e9}\", 1, 2))",
        "string.length(string.slice(\"\u{e9}t\u{e9}\", 0, 4))",
    ]
    .iter()
    .enumerate()
    {
        let src = format!("pub fn main() -> Int {{ io.println(\"before\")  {call} }}\n");
        let got = agree(&src, &format!("fail{i}"));
        assert!(got.end.is_err(), "{call}: {got:?}");
        assert_eq!(got.text, "before\n", "{call}");
    }
}

#[test]
fn replay_parse_agrees_natively() {
    let src = r#"import elya/replay
effect Ask {
  fn ask(q: String) -> String
  fn n(k: Int) -> Int
}

fn prog() / {Ask} -> Int {
  let a = ask("x = y")
  let b = ask("p")
  string.length(a) * 100 + string.length(b) * 10 + n(3)
}

fn live(body: fn() / {Ask} -> Recorded(Int)) -> Recorded(Int) {
  handle { body() } with {
    Ask.ask(q) -> resume(if q == "p" { "plain" } else { "p = \"q\"\né" })
    Ask.n(k) -> resume(k * 2)
    return(x) -> x
  }
}

fn why(text: String) -> Int {
  match replay.parse(text) {
    TraceParsed(t) -> 0
    TraceUnreadable(n, w) -> {
      io.println(w)
      n
    }
  }
}

pub fn main() -> Int {
  let bad = why("elya trace 2\n") * 100 + why("elya trace 1\nA.b()\n") * 10 + why("elya trace 1\nA.a(\"x = 1) = 2\n")
  match live(fn() { replay.record(fn() { prog() }) }) {
    Recorded(x, t) -> {
      io.println(replay.text(t))
      match replay.parse(replay.text(t)) {
        TraceParsed(u) -> {
          let same = if u == t { 1 } else { 0 }
          match replay.replay(fn() { prog() }, u) {
            Replayed(y) -> bad * 1000000000 + x * 10 + same + y * 100000
            Diverged(a, b) -> 0 - 1
            Unconsumed(y, r) -> 0 - 2
          }
        }
        TraceUnreadable(line, w) -> 0 - 3
      }
    }
  }
}
"#;
    let got = agree(src, "parse");
    assert_eq!(got.end, Ok("122105610561".to_string()), "{got:?}");
}
