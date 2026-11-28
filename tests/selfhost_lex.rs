//! Differential proof for selfhost/lex.elya. E0002 is deliberately excluded:
//! the port preserves numeric spelling and does not parse float magnitudes.

use elya::eval::{Host, World};
use elya::lex::{lex, TokenKind};
use elya::Session;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "programs/std_capabilities.rs"]
mod capabilities_programs;
#[allow(dead_code)]
#[path = "programs/files_and_arguments.rs"]
mod files_programs;

const LEXER: &str = include_str!("../selfhost/lex.elya");

fn escaped(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        out.push_str(match c {
            '\n' => "\\n",
            '\t' => "\\t",
            '\r' => "\\r",
            '\\' => "\\\\",
            '"' => "\\\"",
            _ => {
                out.push(c);
                continue;
            }
        });
    }
    out.push('"');
    out
}

fn rust_dump(text: &str) -> String {
    let (tokens, diagnostics) = lex(&Session::new(), text);
    let mut out = String::new();
    for t in tokens {
        let (kind, payload) = match &t.kind {
            TokenKind::Int(_) => (
                "Int".to_string(),
                Some(text[t.span.start as usize..t.span.end as usize].replace('_', "")),
            ),
            TokenKind::Float(_) => (
                "Float".to_string(),
                Some(text[t.span.start as usize..t.span.end as usize].replace('_', "")),
            ),
            TokenKind::Str(s) => ("Str".to_string(), Some(escaped(s))),
            TokenKind::Lower(s) => ("Lower".to_string(), Some(s.clone())),
            TokenKind::Upper(s) => ("Upper".to_string(), Some(s.clone())),
            TokenKind::IntExponent(_) => panic!("lex must normalize IntExponent to Float"),
            other => (format!("{other:?}"), None),
        };
        write!(out, "{kind} {} {}", t.span.start, t.span.end).unwrap();
        if let Some(payload) = payload {
            write!(out, " {payload}").unwrap();
        }
        out.push('\n');
    }
    for d in diagnostics {
        if d.code != "E0002" {
            let span = d.labels[0].span;
            writeln!(out, "error {} {} {}", d.code, span.start, span.end).unwrap();
        }
    }
    out
}

fn assert_dump(name: &str, actual: &str, expected: &str) {
    if actual == expected {
        return;
    }
    let a: Vec<_> = actual.split_inclusive('\n').collect();
    let e: Vec<_> = expected.split_inclusive('\n').collect();
    let i = (0..a.len().max(e.len()))
        .find(|&i| a.get(i) != e.get(i))
        .unwrap();
    panic!(
        "{name}: first differing line {}\nElya: {:?}\nRust: {:?}",
        i + 1,
        a.get(i),
        e.get(i)
    );
}

// RAII cleanup also runs on a failing comparison. Separate test names make the
// directories independent under Cargo's parallel test runner.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("elya-selfhost-lex-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn put(&self, text: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join("input.txt");
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn host(path: &Path) -> Host {
    Host {
        args: vec![path.as_os_str().to_owned()],
        ..Host::default()
    }
}

fn run(program: &str, path: &Path) -> String {
    let o = elya::run_source_outcome_host("selfhost/lex.elya", program, &mut |_| None, host(path));
    assert!(o.error.is_none(), "{}: {:?}", path.display(), o.error);
    assert_eq!(o.value, Some(Ok(0)), "main returns Unit");
    o.output
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "elya") {
            out.push(path);
        }
    }
}

fn programs() -> Vec<(&'static str, &'static str)> {
    use capabilities_programs as c;
    use files_programs as f;
    let mut cases = vec![
        ("FILES", f::FILES),
        ("ARGS", f::ARGS),
        ("TRACED", f::TRACED),
        ("IN_HANDLERS", f::IN_HANDLERS),
        ("READS_INPUT", f::READS_INPUT),
        ("NO_WORLD", f::NO_WORLD),
        ("DEP", c::DEP),
        ("GIVEN_LESS", c::GIVEN_LESS),
        ("ATTENUATED", c::ATTENUATED),
    ];
    cases.extend(f::FAILURES.iter().map(|(src, _, _)| ("FAILURES", *src)));
    cases
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort_unstable();
    times[times.len() / 2]
}

fn benchmark(path: &Path, text: &str) {
    // Compile once, outside timing. Elya measurements include evaluator setup,
    // io.read_file and dump formatting, but exclude the compiler front end.
    let (module, diags, _) = elya::front_end_with("selfhost/lex.elya", LEXER, &mut |_| None);
    assert!(diags.is_empty(), "{diags:?}");
    let expected = rust_dump(text);
    let mut elya_times = Vec::new();
    for _ in 0..3 {
        let t = Instant::now();
        let (interp, result) =
            elya::eval::run_module_keeping_host(&module, World::Live, host(path));
        elya_times.push(t.elapsed());
        assert!(result.is_ok(), "{result:?}");
        assert_dump("benchmark", interp.output(), &expected);
    }
    let mut rust_times = Vec::new();
    let mut lex_times = Vec::new();
    for _ in 0..101 {
        let t = Instant::now();
        let actual = std::hint::black_box(rust_dump(std::hint::black_box(text)));
        rust_times.push(t.elapsed());
        assert_dump("Rust benchmark", &actual, &expected);
        let t = Instant::now();
        std::hint::black_box(lex(&Session::new(), std::hint::black_box(text)));
        lex_times.push(t.elapsed());
    }
    println!(
        "largest: {} ({} bytes); median Elya evaluator+file+dump {:?} (3 runs), Rust dump {:?}, Rust lex {:?} (101 runs; current Cargo test profile)",
        path.display(), text.len(), median(elya_times), median(rust_times), median(lex_times)
    );
}

#[test]
fn repository_and_shared_programs_agree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths = Vec::new();
    // Include tests/row_parameters too: every repository .elya source, not just UI.
    for dir in ["examples", "src/std", "selfhost", "tests"] {
        sources(&root.join(dir), &mut paths);
    }
    paths.sort();
    let mut bytes = 0;
    let mut largest = (PathBuf::new(), String::new());
    for path in &paths {
        let text = std::fs::read_to_string(path).unwrap();
        let actual = run(LEXER, path);
        assert_dump(&path.display().to_string(), &actual, &rust_dump(&text));
        bytes += text.len();
        if text.len() > largest.1.len() {
            largest = (path.clone(), text);
        }
    }
    let scratch = Scratch::new("programs");
    let programs = programs();
    let mut program_bytes = 0;
    for (name, text) in &programs {
        let path = scratch.put(text);
        assert_dump(name, &run(LEXER, &path), &rust_dump(text));
        program_bytes += text.len();
    }
    println!(
        "compared {} repository files / {} bytes; {} embedded programs / {} bytes",
        paths.len(),
        bytes,
        programs.len(),
        program_bytes
    );
    benchmark(&largest.0, &largest.1);
}

// Length-prefixed records let thousands of malformed inputs run independently
// through lex_dump without re-running the compiler front end for every record.
// The regular repository test above exercises the shipped main unchanged.
const BATCH_MAIN: &str = r#"
fn record_length(s: String, i: Int, n: Int) -> Int {
  let c = string.byte_at(s, i)
  if c == 58 { n } else { record_length(s, i + 1, n * 10 + c - 48) }
}
fn record_start(s: String, i: Int) -> Int {
  if string.byte_at(s, i) == 58 { i + 1 } else { record_start(s, i + 1) }
}
fn records(s: String, i: Int, count: Int) {
  if i == string.length(s) { Unit } else {
    let n = record_length(s, i, 0)
    let start = record_start(s, i)
    io.println("case " <> int.to_string(count))
    io.println(lex_dump(string.slice(s, start, start + n)))
    records(s, start + n, count + 1)
  }
}
pub fn main() { records(io.read_file(io.arg(0)), 0, 0) }
"#;

fn enumerate(cases: &mut Vec<String>, prefix: &str, alphabet: &[&str], depth: usize) {
    cases.push(prefix.to_string());
    if depth != 0 {
        for c in alphabet {
            enumerate(cases, &format!("{prefix}{c}"), alphabet, depth - 1);
        }
    }
}

#[test]
fn edge_cases_agree() {
    let mut cases: Vec<String> = [
        "", " \t\r\n\x0c", "\x0b\0\x01\x1f\x7f @#$;`\\&",
        "pub fn let import if else type linear match case effect trait impl handle with multi return const True False Unit",
        "pubx fn_ let1 _ _Abc A1 Truex False_ Unit2 aZ09 _9",
        "+. -. *. /. <. <=. >. >=. == != <= >= -> => <> |> && || | .. + - * / % = < > ! ? . , : ( ) { } [ ]",
        "1..2 1.5e3 1e19 0x? 0xFF 0b10 1_000 00 1__ 1_.2_ 1_0.2_0e+3_ 1__e1_",
        "1e 1e+ 1e- 1e_2 1.2e 1.2e+ 1.2e- 1.2e_2 .5 1. 1...2 -2E-3 1.2.3 1._2",
        "999999999999999999999999999999999999999999999 1.0e9999 1.0e-9999 1e9999 0.0e-9999",
        "\"\" \"plain\" \"é中😀\" \"\\n\\t\\r\\\\\\\"\"",
        "\"raw\nline\tand\rcarriage\"", "\"bad\\qescape\" after", "\"bad\\é\" after",
        "\"bad\\\r\" after", "\"a\\\nb\"", "\"unterminated", "\"dangling\\",
        "// comment\r \"x\" /*\nlet", "/* closed */let", "/**/", "**/", "/*", "/**",
        "/***/", "/****/", "/*****/", "/*a**/", "/*a***/let", "/*unterminated\nlet x = 1",
        "/* outer /* inner */ tail */", "/* *é/ **/", "é中😀 a§b \"nul\0del\x7f\"",
        "\u{85}\u{a0}\u{1680}\u{2000}\u{2028}\u{2029}\u{feff}",
    ].into_iter().map(str::to_string).collect();
    // Check every ASCII byte in isolation, both escaped and unescaped in strings.
    for b in 0..=127u8 {
        let c = char::from(b);
        cases.extend([c.to_string(), format!("\"{c}\""), format!("\"\\{c}\"")]);
    }
    // Exhaust the short regex paths, including UTF-8 and LF/CR transitions.
    enumerate(&mut cases, "/*", &["*", "/", "a", "é", "\n"], 5);
    enumerate(&mut cases, "\"", &["\"", "\\", "n", "é", "\n", "\r"], 4);
    enumerate(&mut cases, "1", &["1", "_", ".", "e", "+", "-"], 4);
    enumerate(&mut cases, "1.0", &["1", "_", ".", "e", "+", "-"], 4);
    // Many openers, no closing delimiter: a failed comment must not restart.
    cases.push(format!("/*{}", "a/*".repeat(4096)));
    cases.push(format!("\"{}\"", "a_é\\n".repeat(4096)));
    cases.push("1_".repeat(4096)); // long source spelling, no numeric parsing
    let count = cases.len();
    // logos 0.14 recurses on the long mixed UTF-8/escaped string in debug
    // builds. Only the Rust oracle gets a larger host stack; Elya runs below
    // on Cargo's ordinary test thread, including that same long literal.
    let (packed, expected, bytes) = std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(move || {
            let mut packed = String::new();
            let mut expected = String::new();
            let mut bytes = 0;
            for (i, text) in cases.iter().enumerate() {
                write!(packed, "{}:{text}", text.len()).unwrap();
                writeln!(expected, "case {i}\n{}", rust_dump(text)).unwrap();
                bytes += text.len();
            }
            (packed, expected, bytes)
        })
        .unwrap()
        .join()
        .unwrap();
    let scratch = Scratch::new("edges");
    let path = scratch.put(packed);
    let (functions, _) = LEXER.split_once("pub fn main()").unwrap();
    let program = format!("{functions}{BATCH_MAIN}");
    assert_dump(
        "edge corpus (case numbers identify the input)",
        &run(&program, &path),
        &expected,
    );
    println!(
        "compared {} independent edge cases / {} source bytes",
        count, bytes
    );
}

#[test]
fn sample_and_invalid_file_boundary() {
    let o =
        elya::run_source_outcome_host("selfhost/lex.elya", LEXER, &mut |_| None, Host::default());
    assert!(o.error.is_none(), "{:?}", o.error);
    assert_dump(
        "sample",
        &o.output,
        &rust_dump("pub fn main() { 1e19 + 2 }\n"),
    );
    // Both lexer APIs accept UTF-8 strings. Raw invalid bytes are rejected at
    // the file boundary, rather than replaced or assigned invented token spans.
    let scratch = Scratch::new("invalid-utf8");
    let path = scratch.put("");
    assert_dump("empty file", &run(LEXER, &path), "");
    let path = scratch.put(files_programs::BAD_UTF8);
    let o = elya::run_source_outcome_host("selfhost/lex.elya", LEXER, &mut |_| None, host(&path));
    assert!(o.error.unwrap().contains("not UTF-8"));
    assert!(o.output.is_empty());
}
