//! The programs of `tests/files_and_arguments.rs`, shared with
//! `crates/codegen/tests/native_files.rs` (no tests here). `@D@` stands for a
//! directory the test made; `at` puts it in.

/// `src` with `@D@` replaced by `dir`, its separators `/` (Windows takes them,
/// and a `\` in an Elya string would be an escape).
pub fn at(src: &str, dir: &std::path::Path) -> String {
    src.replace("@D@", &dir.display().to_string().replace('\\', "/"))
}

/// A file written, asked about, read back, emptied; a directory is not a file.
pub const FILES: &str = r#"pub fn main() -> Int {
  let p = "@D@/a.txt"
  let before = io.file_exists(p)
  io.write_file(p, "line one\nline two\t\"quoted\" \\ é\n")
  let after = io.file_exists(p)
  let text = io.read_file(p)
  io.println(if before { "there before" } else { "not there before" })
  io.println(if after { "there after" } else { "not there after" })
  io.println(text)
  io.write_file(p, "")
  io.println("[" <> io.read_file(p) <> "]")
  io.println(if io.file_exists("@D@") { "a directory is a file" } else { "a directory is not a file" })
  string.length(text)
}
"#;

/// What [`FILES`] prints, and the file it leaves (empty).
pub const FILES_OUT: &str = "not there before\nthere after\nline one\nline two\t\"quoted\" \\ é\n\n[]\na directory is not a file\n";

/// The text [`FILES`] writes first.
pub const FILES_TEXT: &str = "line one\nline two\t\"quoted\" \\ é\n";

/// Every argument, then standard input twice (the second read is empty).
pub const ARGS: &str = r#"fn show(i: Int, n: Int) -> Int {
  if i < n { io.println(int.to_string(i) <> ": " <> io.arg(i))  show(i + 1, n) } else { n }
}
pub fn main() -> Int {
  let n = show(0, io.arg_count())
  let input = io.read_stdin()
  io.println("input: " <> input)
  io.println("again: [" <> io.read_stdin() <> "]")
  n
}
"#;

/// The arguments and input [`ARGS`] is given, and what it prints.
pub const ARGS_GIVEN: [&str; 3] = ["one", "two words", "ünï"];
pub const ARGS_STDIN: &str = "from stdin\n";
pub const ARGS_OUT: &str = "0: one\n1: two words\n2: ünï\ninput: from stdin\n\nagain: []\n";

/// One question of each kind, in a known order (D3), for record and replay.
pub const TRACED: &str = r#"pub fn main() -> Int {
  let p = "@D@/t.txt"
  io.write_file(p, "a \"b\"\nc")
  let here = io.file_exists(p)
  let t = io.read_file(p)
  let s = io.read_stdin()
  io.println(t <> "|" <> s <> "|" <> io.arg(0))
  if here { io.arg_count() } else { 0 }
}
"#;

/// [`TRACED`] given the argument `x y` and the input `in\n`: its trace and output.
pub const TRACED_ARG: &str = "x y";
pub const TRACED_STDIN: &str = "in\n";
pub const TRACED_TRACE: &str = r#"elya trace 1
io.write_file("@D@/t.txt", "a \"b\"\nc") = Unit
io.file_exists("@D@/t.txt") = True
io.read_file("@D@/t.txt") = "a \"b\"\nc"
io.read_stdin() = "in\n"
io.arg(0) = "x y"
io.arg_count() = 1
"#;
pub const TRACED_OUT: &str = "a \"b\"\nc|in\n|x y\n";

/// Files read and written inside a handler and after a perform: the CPS code's
/// path natively.
pub const IN_HANDLERS: &str = r#"effect Ask { fn ask(path: String) -> String }
fn job() -> Int {
  let a = ask("@D@/c1.txt")
  io.write_file("@D@/c2.txt", a <> "+" <> a)
  let back = io.read_file("@D@/c2.txt")
  io.println(back)
  string.length(back)
}
pub fn main() -> Int {
  io.write_file("@D@/c1.txt", "ab")
  handle { job() } with {
    Ask.ask(p) -> resume(io.read_file(p))
    return(x) -> x * 10
  }
}
"#;

/// Programs that fail by name (D2), the message's text after `elya: `, and
/// what they printed before it. `@D@/bad.txt` holds bytes that are not UTF-8
/// and `@D@/nul.txt` a path with a NUL byte in it (the test writes both).
pub const FAILURES: &[(&str, &str, &str)] = &[
    (
        "pub fn main() { io.println(\"before\")  io.read_file(\"@D@/missing.txt\") }\n",
        "cannot read `@D@/missing.txt`: no such file or directory",
        "before\n",
    ),
    (
        "pub fn main() { io.read_file(\"@D@\") }\n",
        "cannot read `@D@`: it is a directory",
        "",
    ),
    (
        "pub fn main() { io.write_file(\"@D@\", \"x\") }\n",
        "cannot write `@D@`: it is a directory",
        "",
    ),
    (
        "pub fn main() { io.write_file(\"@D@/no/such/f.txt\", \"x\") }\n",
        "cannot write `@D@/no/such/f.txt`: no such file or directory",
        "",
    ),
    (
        "pub fn main() { io.read_file(io.read_file(\"@D@/nul.txt\")) }\n",
        "cannot read `x\0y`: a path cannot hold a NUL byte",
        "",
    ),
    (
        "pub fn main() { io.write_file(io.read_file(\"@D@/nul.txt\"), \"z\") }\n",
        "cannot write `x\0y`: a path cannot hold a NUL byte",
        "",
    ),
    (
        "pub fn main() { io.read_file(\"@D@/bad.txt\") }\n",
        "`@D@/bad.txt` is not UTF-8",
        "",
    ),
    (
        "pub fn main() { io.println(int.to_string(io.arg_count()))  io.arg(0) }\n",
        "no argument 0 (the program has 0)",
        "0\n",
    ),
    (
        "pub fn main() { io.arg(0 - 1) }\n",
        "no argument -1 (the program has 0)",
        "",
    ),
];

/// The bytes of `@D@/bad.txt` (an overlong encoding, then a lone continuation
/// byte) and of `@D@/nul.txt`.
pub const BAD_UTF8: &[u8] = b"ok \xc0\x80 \x80";
pub const NUL_PATH: &[u8] = b"x\0y";

/// Reads standard input, which the test makes not UTF-8.
pub const READS_INPUT: &str = "pub fn main() { io.println(\"before\")  io.read_stdin() }\n";
pub const INPUT_NOT_UTF8: &str = "standard input is not UTF-8";

/// What a machine with no files, no arguments and no input answers.
pub const NO_WORLD: &str = r#"pub fn main() {
  io.println(if io.file_exists("a.txt") { "a file" } else { "no file" })
  io.println(int.to_string(io.arg_count()))
  io.println("[" <> io.read_stdin() <> "]")
  io.read_file("a.txt")
}
"#;
pub const NO_WORLD_OUT: &str = "no file\n0\n[]\n";
pub const NO_WORLD_FAILURE: &str = "cannot read `a.txt`: this platform has no files";
