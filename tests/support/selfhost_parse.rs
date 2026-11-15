//! Shared corpus and comparison for the evaluator and native self-hosted parser.

#![allow(dead_code)] // Each back end uses only the helpers it needs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const LEXER: &str = include_str!("../../selfhost/lex.elya");

pub fn parser_source() -> String {
    std::fs::read_to_string(root().join("selfhost/parse.elya"))
        .expect("selfhost/parse.elya must exist")
}

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("selfhost").is_dir())
        .expect("repository root")
        .to_path_buf()
}

pub fn reader(name: &str) -> Option<(String, String)> {
    (name == "lex").then(|| ("selfhost/lex.elya".to_owned(), LEXER.to_owned()))
}

pub fn assert_dump(name: &str, actual: &str, expected: &str) {
    if actual == expected {
        return;
    }
    let actual: Vec<_> = actual.split_inclusive('\n').collect();
    let expected: Vec<_> = expected.split_inclusive('\n').collect();
    let i = (0..actual.len().max(expected.len()))
        .find(|&i| actual.get(i) != expected.get(i))
        .expect("different dumps have a different line");
    panic!(
        "{name}: first differing line {}\nElya: {:?}\nRust: {:?}",
        i + 1,
        actual.get(i),
        expected.get(i)
    );
}

pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("elya-selfhost-parse-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create scratch directory");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn put(&self, text: &str) -> PathBuf {
        let path = self.0.join("input.elya");
        std::fs::write(&path, text).expect("write parser input");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Case {
    pub name: String,
    pub text: String,
    pub dump: String,
}

pub struct Corpus {
    pub cases: Vec<Case>,
    pub files: usize,
    pub skipped: usize,
    pub embedded: usize,
}

impl Corpus {
    pub fn largest(&self) -> &Case {
        self.cases
            .iter()
            .max_by_key(|case| case.text.len())
            .expect("a nonempty corpus")
    }
}

fn sources(dir: &Path, extension: &str, recurse: bool, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("source directory") {
        let path = entry.expect("source directory entry").path();
        if recurse && path.is_dir() {
            sources(&path, extension, true, out);
        } else if path.extension().is_some_and(|ext| ext == extension) {
            out.push(path);
        }
    }
}

fn clean_case(name: String, text: String, require_module: bool) -> Option<Case> {
    let (module, diagnostics) = elya::parse::parse_module(&elya::Session::new(), &text);
    if !diagnostics.is_empty()
        || (require_module && module.decls.is_empty() && module.imports.is_empty())
    {
        return None;
    }
    let dump = elya::ast_dump::dump(&module, &text);
    Some(Case { name, text, dump })
}

// Decode Rust string literals without evaluating Rust or running source programs.
// Both raw strings (arbitrary # delimiters) and normal escaped UTF-8 strings are
// included; comments and character literals are skipped. Invalid candidate
// strings are ignored and every result still passes the Rust Elya parser.
fn rust_strings(src: &str) -> Vec<(usize, String)> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"//") {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            i += 2;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                if bytes[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if bytes[i] == b'\'' {
            // A Rust char closes after one scalar or one escape. Lifetimes do
            // not have that closing quote and are ordinary source here.
            let mut end = i + 1;
            if end < bytes.len() && bytes[end] == b'\\' {
                end += 2;
                if bytes.get(end) == Some(&b'{') {
                    while end < bytes.len() && bytes[end] != b'}' {
                        end += 1;
                    }
                    end += 1;
                }
            } else if let Some(ch) = src.get(end..).and_then(|s| s.chars().next()) {
                end += ch.len_utf8();
            }
            if bytes.get(end) == Some(&b'\'') {
                i = end + 1;
                continue;
            }
        }
        let start = i;
        if bytes[i] == b'r' {
            let mut quote = i + 1;
            while bytes.get(quote) == Some(&b'#') {
                quote += 1;
            }
            if bytes.get(quote) == Some(&b'"') {
                let hashes = quote - i - 1;
                let mut end = quote + 1;
                while end < bytes.len() {
                    if bytes[end] == b'"'
                        && bytes.get(end + 1..end + 1 + hashes) == Some(&bytes[i + 1..quote])
                    {
                        out.push((start, src[quote + 1..end].to_owned()));
                        i = end + 1 + hashes;
                        break;
                    }
                    end += 1;
                }
                if i != start {
                    continue;
                }
            }
        }
        if bytes[i] == b'"' {
            i += 1;
            let mut text = String::new();
            let mut valid = true;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] != b'\\' {
                    let ch = src[i..].chars().next().expect("UTF-8 scalar");
                    text.push(ch);
                    i += ch.len_utf8();
                    continue;
                }
                i += 1;
                let Some(&escape) = bytes.get(i) else {
                    valid = false;
                    break;
                };
                i += 1;
                match escape {
                    b'n' => text.push('\n'),
                    b'r' => text.push('\r'),
                    b't' => text.push('\t'),
                    b'0' => text.push('\0'),
                    b'\\' => text.push('\\'),
                    b'"' => text.push('"'),
                    b'\'' => text.push('\''),
                    b'\n' | b'\r' => {
                        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
                            i += 1;
                        }
                    }
                    b'x' => {
                        if let Some(hex) = src.get(i..i + 2) {
                            if let Ok(n) = u8::from_str_radix(hex, 16) {
                                text.push(char::from(n));
                                i += 2;
                            } else {
                                valid = false;
                            }
                        } else {
                            valid = false;
                        }
                    }
                    b'u' if bytes.get(i) == Some(&b'{') => {
                        let begin = i + 1;
                        while i < bytes.len() && bytes[i] != b'}' {
                            i += 1;
                        }
                        let code = src[begin..i].replace('_', "");
                        if let Some(ch) =
                            u32::from_str_radix(&code, 16).ok().and_then(char::from_u32)
                        {
                            text.push(ch);
                        } else {
                            valid = false;
                        }
                        i += usize::from(i < bytes.len());
                    }
                    _ => valid = false,
                }
            }
            if valid && bytes.get(i) == Some(&b'"') {
                out.push((start, text));
            }
            i += usize::from(i < bytes.len());
        } else {
            i += 1;
        }
    }
    out
}

// Additional focused cases make otherwise rare span and grammar edges explicit.
// The main embedded corpus below is discovered from all tests/*.rs and
// tests/programs/*.rs, including tests/ast_dump.rs's pinned EVERY program.
fn focused_programs() -> Vec<String> {
    let mut cases = vec![
        "".to_owned(),
        "// only a comment\n".to_owned(),
        "import lib/dep as d / {}\nfn f() { Unit }".to_owned(),
        "fn f(borrow xs: Array(Int), borrow x, y: fn(Int) / {} -> Int) / {} -> Unit { Unit }".to_owned(),
        "linear type T(e, a) { C(Task({Log | e}), fn(a) / {e} -> a), D }".to_owned(),
        "effect multi E(e, a) { fn op(x: Task({Log | e}), k: fn(a) / {e} -> a) -> a }".to_owned(),
        "fn f() { let x: Task({Log | e}) = C()\nmatch x { -9223372036854775808 -> 0  0001 -> 1  True -> 2  False -> 3  Unit -> 4  _ -> 5 } }".to_owned(),
        "fn f() { handle f() with any { op(o) -> resume(o) return(x) -> x } }".to_owned(),
        "fn f() { handle { f() } with multi { E.op(x) -> { resume(x) } return(x) -> x } }".to_owned(),
        "fn f() { ((1.25))\n(1.0e3)\n-0001\n-(1)\n-.1.0\n!False }".to_owned(),
        "fn f() { f(\n1,\n2\n)\n(f)()\nfn(x: Array(Int)) { array.length(x) } }".to_owned(),
        "fn f() { match x { C(D(x, _), Y) -> x\n\"a\\n\\t\\r\\\\\\\"λ\" -> 1\nvar -> 2 } }".to_owned(),
        "fn f() { 1\n-2\n(-3\n+4)\n(5\n(6)) }".to_owned(),
        "fn f() { if True { { Unit } } else { if False { Unit } else { Unit } } }".to_owned(),
    ];
    // Every binding-power level is tested on both sides of every operator.
    // All expressions only need to parse; their types are deliberately irrelevant.
    for op in [
        "+", "-", "*", "/", "%", "+.", "-.", "*.", "/.", "==", "!=", "<", "<=", ">", ">=", "<.",
        "<=.", ">.", ">=.", "<>", "&&", "||",
    ] {
        cases.push(format!("fn f() {{ a {op} b {op} c }}"));
        cases.push(format!("fn f() {{ a {op} b + c * d == e && f || g }}"));
        cases.push(format!("fn f() {{ a || b && c == d + e * f {op} g }}"));
    }
    cases
}

pub fn corpus(root: &Path) -> Corpus {
    // Verify the extractor's distinguishing cases before trusting its count.
    assert_eq!(
        rust_strings(
            "// \"no\"\nr###\"fn raw() {}\"###; \"fn cooked() {\\n1}\"; '\"'; /* \"no\" */"
        )
        .into_iter()
        .map(|(_, s)| s)
        .collect::<Vec<_>>(),
        ["fn raw() {}", "fn cooked() {\n1}"]
    );
    let mut paths = Vec::new();
    for dir in ["examples", "selfhost", "src/std", "tests"] {
        sources(&root.join(dir), "elya", true, &mut paths);
    }
    paths.sort();
    let mut corpus = Corpus {
        cases: Vec::new(),
        files: 0,
        skipped: 0,
        embedded: 0,
    };
    for path in paths {
        let text = std::fs::read_to_string(&path).expect("repository Elya source");
        if let Some(case) = clean_case(
            path.strip_prefix(root).unwrap().display().to_string(),
            text,
            false,
        ) {
            corpus.files += 1;
            corpus.cases.push(case);
        } else {
            corpus.skipped += 1;
        }
    }
    let mut rust_paths = Vec::new();
    sources(&root.join("tests"), "rs", false, &mut rust_paths);
    sources(&root.join("tests/programs"), "rs", false, &mut rust_paths);
    rust_paths.sort();
    let mut seen = BTreeSet::new();
    for path in rust_paths {
        let rust = std::fs::read_to_string(&path).expect("Rust test source");
        for (offset, text) in rust_strings(&rust) {
            if text.contains('{') || text.trim_start().starts_with("import ") {
                if let Some(case) = clean_case(
                    format!(
                        "{}:byte{offset}",
                        path.strip_prefix(root).unwrap().display()
                    ),
                    text,
                    true,
                ) {
                    if seen.insert(case.text.clone()) {
                        corpus.embedded += 1;
                        corpus.cases.push(case);
                    }
                }
            }
        }
    }
    for (i, text) in focused_programs().into_iter().enumerate() {
        let case = clean_case(format!("focused-{i}"), text, false)
            .unwrap_or_else(|| panic!("focused parser program {i} must parse clean"));
        corpus.cases.push(case);
    }
    corpus
}
