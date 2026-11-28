// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
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

#[derive(Clone)]
pub struct Case {
    pub name: String,
    pub text: String,
    pub dump: String,
    pub broken: bool,
}

#[derive(Default)]
pub struct Corpus {
    pub cases: Vec<Case>,
    pub files: usize,
    pub skipped: usize,
    pub embedded: usize,
    pub broken_files: usize,
    pub broken_embedded: usize,
    pub extra_embedded: usize,
    pub extra_broken_embedded: usize,
    pub focused: usize,
    pub broken_focused: usize,
    pub generated: usize,
    pub tokenless: usize,
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
    Some(Case {
        name,
        text,
        dump,
        broken: false,
    })
}

// Decode Rust string literals without evaluating Rust or running source programs.
// Both raw strings (arbitrary # delimiters) and normal escaped UTF-8 strings are
// included; comments and character literals are skipped. Invalid candidate
// strings are ignored and every result still passes the Rust Elya parser.
pub fn rust_strings(src: &str) -> Vec<(usize, String)> {
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

// Keep the original clean corpus API for the checker, which imports this helper.
// The parser suites use parity_corpus for the complete 4b proof below.
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
        ..Corpus::default()
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

pub fn case_label(case: &Case) -> String {
    let input = if case.text.len() <= 220 {
        format!("{:?}", case.text)
    } else {
        format!("{} bytes", case.text.len())
    };
    format!("{} ({input})", case.name)
}

fn case(name: String, text: String) -> Case {
    let (module, diagnostics) = elya::parse::parse_module(&elya::Session::new(), &text);
    let mut dump = elya::ast_dump::dump(&module, &text);
    dump.push_str(&elya::ast_dump::dump_diagnostics(&diagnostics));
    Case {
        name,
        text,
        dump,
        broken: !diagnostics.is_empty(),
    }
}

impl Corpus {
    pub fn largest_clean(&self) -> &Case {
        self.cases
            .iter()
            .filter(|c| !c.broken)
            .max_by_key(|c| c.text.len())
            .expect("clean corpus")
    }
    pub fn largest_broken(&self) -> &Case {
        self.cases
            .iter()
            .filter(|c| c.broken)
            .max_by_key(|c| c.text.len())
            .expect("broken corpus")
    }
    pub fn report(&self) {
        println!("CORPUS repository clean={} broken={}; original-scope embedded clean={} broken={}; expanded-scope embedded clean={} broken={}; focused clean={} broken={}; generated={} (N={} token positions per nonempty clean input; tokenless={} inputs); total={} bytes={}; largest clean={} ({} bytes); largest broken={} ({} bytes)",
            self.files, self.broken_files, self.embedded, self.broken_embedded,
            self.extra_embedded, self.extra_broken_embedded, self.focused, self.broken_focused,
            self.generated, DELETION_POSITIONS, self.tokenless, self.cases.len(),
            self.cases.iter().map(|c| c.text.len()).sum::<usize>(),
            self.largest_clean().name, self.largest_clean().text.len(),
            self.largest_broken().name, self.largest_broken().text.len());
    }
}

// One central token per nonempty clean input. Empty/comment-only inputs have
// no token to delete and are counted explicitly. N is kept small so evaluator
// coverage scales with the complete, automatically discovered clean corpus.
pub const DELETION_POSITIONS: usize = 1;

fn is_module_literal(text: &str) -> bool {
    use elya::lex::TokenKind as T;
    let (tokens, _) = elya::lex::lex(&elya::Session::new(), text);
    // Bare keywords in existing test files are test tags, not source programs;
    // actual bare-keyword truncations are explicit focused fixtures below.
    if tokens.len() < 2 {
        return false;
    }
    match &tokens[0].kind {
        T::KwPub | T::KwImport | T::KwLinear => true,
        T::KwFn => match tokens.get(1).map(|t| &t.kind) {
            Some(T::Lower(_) | T::Upper(_)) => true,
            Some(T::LParen) => tokens
                .windows(2)
                .any(|w| w[0].kind == T::RParen && w[1].kind == T::LBrace),
            _ => false,
        },
        T::KwType | T::KwEffect => {
            matches!(
                tokens.get(1).map(|t| &t.kind),
                Some(T::Lower(_) | T::Upper(_))
            ) && matches!(tokens.get(2).map(|t| &t.kind), Some(T::LParen | T::LBrace))
        }
        // A complete declaration may follow literal leading garbage. Require
        // an actual retained declaration/import, not just words such as `type`
        // in an assertion message or a closure in an expression-body snippet.
        _ => {
            if !text.contains('{') {
                return false;
            }
            let (module, _) = elya::parse::parse_module(&elya::Session::new(), text);
            !module.decls.is_empty() || !module.imports.is_empty()
        }
    }
}

/// The full parser proof. Its Rust nesting-limit fixtures need a larger stack
/// independently of whichever test (or future caller) requests the corpus.
pub fn parity_corpus(root: &Path) -> Corpus {
    let root = root.to_owned();
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(move || full_corpus(&root))
        .expect("parser corpus thread")
        .join()
        .unwrap()
}

fn full_corpus(root: &Path) -> Corpus {
    assert_eq!(
        rust_strings(
            "// \"no\"\nr###\"fn raw() {}\"###; \"fn cooked() {\\n1}\"; '\"'; /* \"no\" */"
        )
        .into_iter()
        .map(|(_, s)| s)
        .collect::<Vec<_>>(),
        ["fn raw() {}", "fn cooked() {\n1}"]
    );
    let mut corpus = Corpus::default();
    let mut paths = Vec::new();
    for dir in ["examples", "selfhost", "src/std", "tests"] {
        sources(&root.join(dir), "elya", true, &mut paths);
    }
    paths.sort();
    for path in paths {
        let c = case(
            path.strip_prefix(root).unwrap().display().to_string(),
            std::fs::read_to_string(&path).expect("repository Elya source"),
        );
        if c.broken {
            corpus.broken_files += 1;
        } else {
            corpus.files += 1;
        }
        corpus.cases.push(c);
    }
    let mut original_paths = Vec::new();
    sources(&root.join("tests"), "rs", false, &mut original_paths);
    sources(
        &root.join("tests/programs"),
        "rs",
        true,
        &mut original_paths,
    );
    let original_paths: BTreeSet<_> = original_paths.into_iter().collect();
    let mut rust_paths = Vec::new();
    for dir in [
        "tests",
        "crates/codegen/tests",
        "crates/cli/tests",
        "src",
        "crates/codegen/src",
        "crates/cli/src",
    ] {
        sources(&root.join(dir), "rs", true, &mut rust_paths);
    }
    // This file's focused programs are enumerated explicitly, not rediscovered.
    rust_paths.retain(|p| *p != root.join("tests/support/selfhost_parse.rs"));
    rust_paths.sort_by_key(|p| (!original_paths.contains(p), p.clone()));
    rust_paths.dedup();
    let mut seen = BTreeSet::new();
    for path in rust_paths {
        let rust = std::fs::read_to_string(&path).expect("Rust test source");
        let inline = path.starts_with(root.join("src"))
            || path.starts_with(root.join("crates/codegen/src"))
            || path.starts_with(root.join("crates/cli/src"));
        let test_start = if inline {
            match rust.find("#[cfg(test)]") {
                Some(i) => i,
                None => continue,
            }
        } else {
            0
        };
        for (offset, text) in rust_strings(&rust[test_start..]) {
            let offset = test_start + offset;
            let name = format!(
                "{}:byte{offset}",
                path.strip_prefix(root).unwrap().display()
            );
            // Keep every 4a clean input, even a format literal which happens to
            // be a valid module. Broken format templates and assertion snippets
            // are not instantiated source inputs and are filtered separately.
            let clean = (text.contains('{') || text.trim_start().starts_with("import "))
                .then(|| clean_case(name.clone(), text.clone(), true))
                .flatten();
            let c = match clean {
                Some(c) => c,
                None if ![
                    "format!(",
                    ".contains(",
                    ".find(",
                    ".rfind(",
                    ".starts_with(",
                    ".ends_with(",
                    ".replace(",
                    ".strip_prefix(",
                    ".strip_suffix(",
                    "Unsupported(",
                ]
                .iter()
                .any(|suffix| rust[..offset].trim_end().ends_with(suffix))
                    && is_module_literal(&text) =>
                {
                    case(name, text)
                }
                None => continue,
            };
            if !seen.insert(c.text.clone()) {
                continue;
            }
            match (original_paths.contains(&path), c.broken) {
                (true, false) => corpus.embedded += 1,
                (true, true) => corpus.broken_embedded += 1,
                (false, false) => corpus.extra_embedded += 1,
                (false, true) => corpus.extra_broken_embedded += 1,
            }
            corpus.cases.push(c);
        }
    }
    for (i, text) in focused_programs()
        .into_iter()
        .chain(extra_clean_programs())
        .enumerate()
    {
        let c = clean_case(format!("focused-clean-{i}"), text, false)
            .unwrap_or_else(|| panic!("focused clean program {i} must parse clean"));
        corpus.focused += 1;
        corpus.cases.push(c);
    }
    let mut focused = BTreeSet::new();
    let mut codes = BTreeSet::new();
    let mut messages = BTreeSet::new();
    for (name, text) in broken_programs() {
        assert!(
            focused.insert(text.clone()),
            "focused broken programs must be distinct: {name}"
        );
        let (_, diagnostics) = elya::parse::parse_module(&elya::Session::new(), &text);
        assert!(
            !diagnostics.is_empty(),
            "focused broken input {name} must fail: {text:?}"
        );
        for d in diagnostics {
            codes.insert(d.code.to_string());
            messages.insert(d.message);
        }
        corpus
            .cases
            .push(case(format!("focused-broken-{name}"), text));
        corpus.broken_focused += 1;
    }
    assert!(
        focused.len() >= 80,
        "at least 80 distinct focused broken programs"
    );
    for code in [
        "E0001", "E0002", "E0003", "E0100", "E0101", "E0102", "E0103",
    ] {
        assert!(codes.contains(code), "focused cases miss {code}");
    }
    // Read every message literal from the oracle. These predicates select its
    // diagnostic messages, excluding explanatory labels and help text.
    let parser = std::fs::read_to_string(root.join("src/parse.rs")).expect("Rust parser source");
    for (_, message) in rust_strings(parser.split("#[cfg(test)]").next().unwrap()) {
        if message.starts_with("expected ")
            || message.starts_with("a `with any`")
            || message.starts_with("a block ")
            || message.starts_with("a row variable ")
            || message.starts_with("operation ")
            || [
                "a grant names effects, not a row variable",
                "nothing follows a row variable",
                "unexpected end of file",
                "integer literal out of range",
                "expression nested too deeply",
                "an operator at the start of a line does not continue the line before",
            ]
            .contains(&message.as_str())
        {
            assert!(
                messages.contains(&message),
                "focused cases miss parser message {message:?}"
            );
        }
    }
    let clean: Vec<_> = corpus
        .cases
        .iter()
        .filter(|c| !c.broken)
        .map(|c| (c.name.clone(), c.text.clone()))
        .collect();
    for (name, text) in clean {
        let (tokens, diagnostics) = elya::lex::lex(&elya::Session::new(), &text);
        assert!(diagnostics.is_empty());
        if tokens.is_empty() {
            corpus.tokenless += 1;
            continue;
        }
        let n = DELETION_POSITIONS.min(tokens.len());
        let positions: BTreeSet<_> = (0..n).map(|i| (i + 1) * tokens.len() / (n + 1)).collect();
        for position in positions {
            let span = tokens[position].span;
            let mut deleted = text.clone();
            deleted.replace_range(span.start as usize..span.end as usize, "");
            corpus.cases.push(case(
                format!("delete-token-{position}-from-{name}"),
                deleted,
            ));
            corpus.generated += 1;
        }
    }
    corpus
}

// Every error branch has a named fixture; operator and nesting families follow.
fn broken_programs() -> Vec<(String, String)> {
    let mut cases: Vec<_> = [
        ("bad-let-next-statement", "fn f() {\n let x = +\n let y = 2\n y\n}"),
        ("bad-let-no-name", "fn f() { let = 1\n 2 }"),
        ("bad-let-no-equals", "fn f() { let x 1\n x }"),
        ("bad-let-type", "fn f() { let x: + = 1\n x }"),
        ("bad-let-no-value-eof", "fn f() { let x ="),
        ("bad-expression-eof", "fn f() { 1 +"),
        ("unclosed-block", "fn f() { 1"),
        ("unclosed-nested-block", "fn f() { { { 1"),
        ("unclosed-before-declaration", "fn f() { 1\nfn g() { 2 }"),
        ("unclosed-before-import", "fn f() { 1\nimport lib\nfn g() { 2 }"),
        ("unclosed-before-linear", "fn f() { 1\nlinear type T { T }"),
        ("unclosed-effect", "effect E { fn op() -> Int"),
        ("unclosed-type", "type T { A, B"),
        ("empty-block", "fn f() {}"),
        ("block-ends-let", "fn f() { let x = 1 }"),
        ("empty-nested-block", "fn f() { {} }"),
        ("empty-lambda-block", "fn f() { fn() {} }"),
        ("empty-if-branches", "fn f() { if True {} else {} }"),
        ("call-close", "fn f() { g(1 ]\n 2 }"),
        ("call-trailing-comma", "fn f() { g(1,)\n 2 }"),
        ("call-missing-argument", "fn f() { g(,1)\n 2 }"),
        ("parenthesis-close", "fn f() { (1 ]\n 2 }"),
        ("resume-open", "fn f() { resume 1\n 2 }"),
        ("resume-close", "fn f() { resume(1 ]\n 2 }"),
        ("resume-no-value", "fn f() { resume()\n 2 }"),
        ("qualified-member", "fn f() { m.1\n 2 }"),
        ("qualified-eof", "fn f() { m."),
        ("if-else", "fn f() { if True { 1 } }"),
        ("if-else-eof", "fn f() { if True { 1 }"),
        ("if-body-open", "fn f() { if True 1\n 2 }"),
        ("if-else-body-open", "fn f() { if True { 1 } else 2\n 3 }"),
        ("lambda-open", "fn f() { fn x\n 2 }"),
        ("lambda-param", "fn f() { fn(1) { 1 }\n 2 }"),
        ("lambda-param-close", "fn f() { fn(x y) { 1 }\n 2 }"),
        ("lambda-param-type", "fn f() { fn(x: +) { x }\n 2 }"),
        ("lambda-body-open", "fn f() { fn(x) x\n 2 }"),
        ("match-open", "fn f() { match x y\n 2 }"),
        ("match-arm-arrow", "fn f() { match x { A 1 }\n 2 }"),
        ("match-arm-body", "fn f() { match x { A -> + }\n 2 }"),
        ("pattern-bad", "fn f() { match x { + -> 1 }\n 2 }"),
        ("pattern-negative", "fn f() { match x { -x -> 1 }\n 2 }"),
        ("pattern-constructor-close", "fn f() { match x { C(x ] -> 1 }\n 2 }"),
        ("pattern-constructor-argument", "fn f() { match x { C(+) -> 1 }\n 2 }"),
        ("pattern-trailing-comma", "fn f() { match x { C(x,) -> 1 }\n 2 }"),
        ("handler-with", "fn f() { handle 1 { }\n 2 }"),
        ("handler-open", "fn f() { handle 1 with x\n 2 }"),
        ("handler-clause", "fn f() { handle 1 with { 1 }\n 2 }"),
        ("handler-effect-dot", "fn f() { handle 1 with { E op() -> 1 }\n 2 }"),
        ("handler-op-name", "fn f() { handle 1 with { E.1() -> 1 }\n 2 }"),
        ("handler-param-open", "fn f() { handle 1 with { op -> 1 }\n 2 }"),
        ("handler-param", "fn f() { handle 1 with { op(1) -> 1 }\n 2 }"),
        ("handler-param-close", "fn f() { handle 1 with { op(x y) -> 1 }\n 2 }"),
        ("handler-arrow", "fn f() { handle 1 with { op(x) 1 }\n 2 }"),
        ("handler-body", "fn f() { handle 1 with { op(x) -> + }\n 2 }"),
        ("handler-return-open", "fn f() { handle 1 with { return x -> x }\n 2 }"),
        ("handler-return-binder", "fn f() { handle 1 with { return(1) -> 1 }\n 2 }"),
        ("handler-return-close", "fn f() { handle 1 with { return(x y) -> 1 }\n 2 }"),
        ("handler-return-arrow", "fn f() { handle 1 with { return(x) x }\n 2 }"),
        ("handler-return-body", "fn f() { handle 1 with { return(x) -> + }\n 2 }"),
        ("handler-any-multi", "fn f() { handle 1 with multi any { op(o) -> 1 }\n 2 }"),
        ("handler-any-shape", "fn f() { handle 1 with any { E.op(o) -> 1 }\n 2 }"),
        ("handler-any-duplicate", "fn f() { handle 1 with any { op(o) -> 1 op(p) -> 2 }\n 2 }"),
        ("handler-any-missing", "fn f() { handle 1 with any { return(x) -> x }\n 2 }"),
        ("handler-layout-minus", "fn f() { handle 1 with { op(x) -> 1\n-2 }\n 3 }"),
        ("layout-final-minus", "fn f() { 1\n-"),
        ("layout-final-float-minus", "fn f() { 1.0\n-."),
        ("layout-plus", "fn f() { 1\n+ 2\n3 }"),
        ("layout-minus-spaced", "fn f() { 1\n- 2\n3 }"),
        ("layout-float-minus-spaced", "fn f() { 1.0\n-. 2.0\n3 }"),
        ("layout-comment", "fn f() { 1 /*\n*/ + 2\n3 }"),
        ("module-garbage", "fn f() { 1 } @ + ] fn g() { 2 }"),
        ("module-leading-garbage", "+ ] fn f() { 1 }"),
        ("module-lambda-not-decl", "fn f(a,,b) {\nfn(x) { x }\n}\nfn g() { 1 }"),
        ("pub-no-fn", "pub type T { T }\nfn g() { 1 }"),
        ("fn-name", "fn Upper() { 1 }\nfn g() { 2 }"),
        ("fn-param-open", "fn f x\nfn g() { 2 }"),
        ("fn-param-name", "fn f(1) { 1 }\nfn g() { 2 }"),
        ("fn-param-close", "fn f(x y) { 1 }\nfn g() { 2 }"),
        ("fn-borrow-param", "fn f(borrow +) { 1 }\nfn g() { 2 }"),
        ("fn-param-type", "fn f(x: +) { 1 }\nfn g() { 2 }"),
        ("fn-return-type", "fn f() -> + { 1 }\nfn g() { 2 }"),
        ("effect-name", "effect lower { }\nfn g() { 2 }"),
        ("effect-type-param", "effect E(Upper) { }\nfn g() { 2 }"),
        ("effect-type-param-close", "effect E(a b) { }\nfn g() { 2 }"),
        ("effect-open", "effect E ]\nfn g() { 2 }"),
        ("effect-op-name", "effect E { fn Upper() -> Int }\nfn g() { 2 }"),
        ("effect-op-param-open", "effect E { fn op ] }\nfn g() { 2 }"),
        ("effect-op-param-name", "effect E { fn op(1: Int) -> Int }\nfn g() { 2 }"),
        ("effect-op-no-param-type", "effect E { fn op(x) -> Int }\nfn g() { 2 }"),
        ("effect-op-param-type", "effect E { fn op(x: +) -> Int }\nfn g() { 2 }"),
        ("effect-op-param-close", "effect E { fn op(x: Int y: Int) -> Int }\nfn g() { 2 }"),
        ("effect-op-arrow", "effect E { fn op() Int }\nfn g() { 2 }"),
        ("effect-close", "effect E { + }\nfn g() { 2 }"),
        ("linear-type-keyword", "linear T { T }\nfn g() { 2 }"),
        ("type-name", "type lower { A }\nfn g() { 2 }"),
        ("type-type-param", "type T(Upper) { A }\nfn g() { 2 }"),
        ("type-param-close", "type T(a b) { A }\nfn g() { 2 }"),
        ("type-open", "type T ]\nfn g() { 2 }"),
        ("type-constructor", "type T { lower }\nfn g() { 2 }"),
        ("type-constructor-field", "type T { C(+) }\nfn g() { 2 }"),
        ("type-constructor-close", "type T { C(Int ] }\nfn g() { 2 }"),
        ("type-close", "type T { A + }\nfn g() { 2 }"),
        ("type-function-open", "fn f(x: fn Int) { 1 }"),
        ("type-function-close", "fn f(x: fn(Int ] -> Int) { 1 }"),
        ("type-function-arrow", "fn f(x: fn(Int) Int) { 1 }"),
        ("type-function-return", "fn f(x: fn(Int) -> +) { 1 }"),
        ("type-tuple-close", "fn f(x: (Int, Int)) { 1 }"),
        ("type-arg", "fn f(x: Array(+)) { 1 }"),
        ("import-segment", "import Upper\nfn g() { 1 }"),
        ("import-path-segment", "import lib/Upper\nfn g() { 1 }"),
        ("import-alias", "import lib as Upper\nfn g() { 1 }"),
        ("row-open", "fn f() / + { 1 }"),
        ("row-label", "fn f() / {+} { 1 }"),
        ("row-lower-mixed", "fn f() / {e, Log} { 1 }"),
        ("row-lower-after-label", "fn f() / {Log, e} { 1 }"),
        ("row-single-tail-bar", "fn f() / {e | h} { 1 }"),
        ("row-double-tail-bar", "fn f() / {Log | e | h} { 1 }"),
        ("row-tail-uppercase", "fn f() / {Log | Log} { 1 }"),
        ("row-tail-absent", "fn f() / {Log |} { 1 }"),
        ("row-tail-underscore", "fn f() / {_} { 1 }"),
        ("row-grant-variable", "import lib / {e}\nfn f() { 1 }"),
        ("row-argument-recovery", "fn f() / {E(+, Int), Log} { 1 }"),
        ("row-unclosed-argument", "fn f() / {E(Int, +), Log} { 1 }"),
        ("row-close", "fn f() / {Log + { 1 }"),
        ("row-close-eof", "fn f() / {Log"),
        ("row-empty-eof", "fn f() / {"),
        ("row-tail-eof", "fn f() / {Log |"),
        ("row-grant-tail-after-label", "import lib / {Log | e}\nfn f() { 1 }"),
        ("lex-unexpected-ascii", "fn f() { @ 1 }"),
        ("lex-unexpected-unicode", "fn f() { § 1 }"),
        ("lex-string-unclosed", "fn f() { \"unterminated"),
        ("lex-string-escape", "fn f() { \"\\q\" }"),
        ("lex-block-comment-unclosed", "fn f() { /* unterminated"),
        ("lex-zero-int-exponent", "fn f() { 0e-9999 }"),
        ("lex-int-exponent", "fn f() { 1e3 }"),
        ("lex-float-upper-adjacent", "fn f() { 1.7976931348623159e308 }"),
        ("lex-float-lower-adjacent", "fn f() { 2.4703282292062327e-324 }"),
        ("lex-float-overflow", "fn f() { 1.0e999 }"),
        ("lex-float-underflow", "fn f() { 1.0e-999 }"),
        ("lex-int-exponent-overflow", "fn f() { 1e999 }"),
        ("lex-negative-float-overflow", "fn f() { -1.0e999 }"),
        ("lex-parser-order", "fn f() { let x = +\n @ 1e3\n2 }"),
        ("integer-positive-range", "fn f() { 9223372036854775808 }"),
        ("integer-negative-range", "fn f() { -9223372036854775809 }"),
        ("integer-u128-range", "fn f() { 340282366920938463463374607431768211456 }"),
        ("integer-pattern-range", "fn f() { match 0 { 9223372036854775808 -> 1 -9223372036854775809 -> 2 } }"),
        ("recover-unclosed-paren-to-let", "fn f() {\n let x = g(1, +\n let y = 2\n y\n}"),
        ("recover-nested-brackets", "fn f() {\n let x = [ { + } ]\n let y = 2\n y\n}"),
        ("recover-unclosed-paren-to-brace", "fn f() {\n g(1, +\n}\nfn g() { 2 }"),
        ("recover-nested-brace", "fn f() {\n let x = g({ + })\n 2\n}"),
        ("recover-same-line-garbage", "fn f() {\n let x = + 99 g()\n 2\n}"),
        ("recover-next-expression", "fn f() {\n + 1\n g()\n 2\n}"),
        ("recover-mismatched-close", "fn f() {\n let x = g([ + ) ]\n let y = 2\n y\n}"),
        ("many-errors", "@ fn bad(+) { 1 }\nfn f() {\n let x = +\n let y = (1 ]\n 2\n}\nimport Upper\neffect lower {}\ntype T { lower }\nfn ok() { 3 }"),
    ].into_iter().map(|(name, text)| (name.to_owned(), text.to_owned())).collect();
    for op in [
        "||", "&&", "==", "!=", "<", "<=", ">", ">=", "<.", "<=.", ">.", ">=.", "<>", "+", "-",
        "+.", "-.", "*", "/", "%", "*.", "/.",
    ] {
        cases.push((
            format!("missing-rhs-{op}"),
            format!("fn f() {{\n let x = 1 {op}\n let y = 2\n y\n}}"),
        ));
    }
    for op in ["-", "-.", "!"] {
        cases.push((format!("missing-prefix-{op}"), format!("fn f() {{ {op} }}")));
    }
    for text in [
        "fn",
        "pub",
        "fn f(",
        "fn f() ->",
        "effect",
        "effect E(",
        "effect E { fn",
        "type",
        "type T {",
        "linear",
        "import",
        "import lib/",
        "import lib as",
    ] {
        cases.push((format!("truncated-{text}"), text.to_owned()));
    }
    let n = elya::parse::MAX_NESTING + 1;
    for (name, text) in [
        (
            "depth-parens",
            format!(
                "fn deep() {{ {}1{} }}\nfn later() {{ 2 }}",
                "(".repeat(n),
                ")".repeat(n)
            ),
        ),
        (
            "height-chain",
            format!(
                "fn deep() {{ {} }}\nfn later() {{ 2 }}",
                vec!["1"; n].join(" + ")
            ),
        ),
        (
            "height-calls",
            format!("fn deep() {{ x{} }}\nfn later() {{ 2 }}", "()".repeat(n)),
        ),
        (
            "depth-pattern",
            format!(
                "fn deep() {{ match x {{ {}C{} -> 1 }} }}\nfn later() {{ 2 }}",
                "C(".repeat(n),
                ")".repeat(n)
            ),
        ),
        (
            "depth-type",
            format!(
                "fn deep(x: {}Int) {{ 1 }}\nfn later() {{ 2 }}",
                "fn() -> ".repeat(n)
            ),
        ),
        (
            "depth-effect",
            format!(
                "effect E {{ fn first(x: {}Int) -> Int\nfn second() -> Int }}\nfn later() {{ 2 }}",
                "fn() -> ".repeat(n)
            ),
        ),
        (
            "depth-unbalanced-resume",
            format!("fn deep() {{ {}1\nfn later() {{ 2 }}", "(".repeat(n)),
        ),
    ] {
        cases.push((name.to_owned(), text));
    }
    for (name, prefix, suffix) in [
        ("depth-blocks", "{ ", " }"),
        ("depth-lambdas", "fn() { ", " }"),
        ("depth-matches", "match 1 { _ -> ", " }"),
        ("depth-ifs", "if True { ", " } else { 0 }"),
        ("depth-handlers", "handle ", " with { op() -> 0 }"),
    ] {
        cases.push((
            name.to_owned(),
            format!(
                "fn deep() {{ {}1{} }}\nfn later() {{ 2 }}",
                prefix.repeat(n),
                suffix.repeat(n)
            ),
        ));
    }
    cases.push(("float-upper-halfway".to_owned(), "fn f() { 179769313486231580793728971405303415079934132710037826936173778980444968292764750946649017977587207096330286416692887910946555547851940402630657488671505820681908902000708383676273854845817711531764475730270069855571366959622842914819860834936475292719074168444365510704342711559699508093042880177904174497792.0 }".to_owned()));
    cases.push(("float-upper-halfway-above".to_owned(), "fn f() { 179769313486231580793728971405303415079934132710037826936173778980444968292764750946649017977587207096330286416692887910946555547851940402630657488671505820681908902000708383676273854845817711531764475730270069855571366959622842914819860834936475292719074168444365510704342711559699508093042880177904174497793.0 }".to_owned()));
    cases.push(("float-lower-halfway".to_owned(), "fn f() { 0.0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000024703282292062327208828439643411068618252990130716238221279284125033775363510437593264991818081799618989828234772285886546332835517796989819938739800539093906315035659515570226392290858392449105184435931802849936536152500319370457678249219365623669863658480757001585769269903706311928279558551332927834338409351978015531246597263579574622766465272827220056374006485499977096599470454020828166226237857393450736339007967761930577506740176324673600968951340535537458516661134223766678604162159680461914467291840300530057530849048765391711386591646239524912623653881879636239373280423891018672348497668235089863388587925628302755995657524455507255189313690836254779186948667994968324049705821028513185451396213837722826145437693412532098591327667236328125 }".to_owned()));
    cases.push(("float-lower-halfway-below".to_owned(), "fn f() { 0.0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000024703282292062327208828439643411068618252990130716238221279284125033775363510437593264991818081799618989828234772285886546332835517796989819938739800539093906315035659515570226392290858392449105184435931802849936536152500319370457678249219365623669863658480757001585769269903706311928279558551332927834338409351978015531246597263579574622766465272827220056374006485499977096599470454020828166226237857393450736339007967761930577506740176324673600968951340535537458516661134223766678604162159680461914467291840300530057530849048765391711386591646239524912623653881879636239373280423891018672348497668235089863388587925628302755995657524455507255189313690836254779186948667994968324049705821028513185451396213837722826145437693412532098591327667236328124 }".to_owned()));
    cases
}

fn extra_clean_programs() -> Vec<String> {
    vec![
        "fn f() { 0.0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000024703282292062327208828439643411068618252990130716238221279284125033775363510437593264991818081799618989828234772285886546332835517796989819938739800539093906315035659515570226392290858392449105184435931802849936536152500319370457678249219365623669863658480757001585769269903706311928279558551332927834338409351978015531246597263579574622766465272827220056374006485499977096599470454020828166226237857393450736339007967761930577506740176324673600968951340535537458516661134223766678604162159680461914467291840300530057530849048765391711386591646239524912623653881879636239373280423891018672348497668235089863388587925628302755995657524455507255189313690836254779186948667994968324049705821028513185451396213837722826145437693412532098591327667236328126 }".to_owned(),
        "fn f() { 179769313486231580793728971405303415079934132710037826936173778980444968292764750946649017977587207096330286416692887910946555547851940402630657488671505820681908902000708383676273854845817711531764475730270069855571366959622842914819860834936475292719074168444365510704342711559699508093042880177904174497791.0 }".to_owned(),
        format!("fn f() {{ {}1{} }}", "(".repeat(elya::parse::MAX_NESTING - 1), ")".repeat(elya::parse::MAX_NESTING - 1)),
        format!("fn f() {{ {} }}", vec!["1"; elya::parse::MAX_NESTING].join(" + ")),
        "fn f() { 1.7976931348623157e308\n2.4703282292062328e-324\n0.0e999999999999999999 }".to_owned(),
    ]
}
