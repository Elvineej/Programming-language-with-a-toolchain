// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! The programs `selfhost/resolve.elya` is compared on (spec
//! 2026-11-15-elya-resolution-in-elya D4), and its oracle: what `elya
//! resolve --dump` prints (crates/cli/src/main.rs, `resolve_dump_cmd`).

#![allow(dead_code)] // Each back end uses only the helpers it needs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The module and resolution codes (D4), counted over every program.
pub const CODES: &[&str] = &[
    "E0200", "E0201", "E0202", "E0203", "E0204", "E0205", "E0206", "E0207", "E0208", "E0209",
    "E0210", "E0211", "E0432", "E0600", "E0601", "E0602", "E0603", "E0604", "E0605", "E0607",
    "E0608",
];

/// One program: a root (written as `main.elya`) and the files beside it.
#[derive(Clone)]
pub struct Program {
    pub name: String,
    pub root: String,
    pub files: Vec<(String, String)>,
}

/// A path as the resolver in Elya is given it: its separators `/`. The
/// resolver splits a root's directory at `/` (it cannot ask the platform);
/// Windows takes either separator, and Rust's oracle reads the same files.
#[allow(dead_code)]
pub fn elya_path(p: &std::path::Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// The selfhost files the resolver imports (`parse`, and `lex` through it).
pub fn reader(name: &str) -> Option<(String, String)> {
    let file = match name {
        "parse" => "selfhost/parse.elya",
        "lex" => "selfhost/lex.elya",
        _ => return None,
    };
    let root = super::parse_support::root();
    let text = std::fs::read_to_string(root.join(file)).ok()?;
    Some((file.to_owned(), text))
}

pub fn resolver_source() -> String {
    std::fs::read_to_string(super::parse_support::root().join("selfhost/resolve.elya"))
        .expect("selfhost/resolve.elya must exist")
}

/// The CLI's reader: a module `a/b` is the file `a/b.elya` next to the root.
fn files(program: &Path) -> impl FnMut(&str) -> Option<(String, String)> {
    let dir = program.parent().map(Path::to_path_buf).unwrap_or_default();
    move |module: &str| {
        let file = dir.join(format!("{module}.elya"));
        std::fs::read_to_string(&file)
            .ok()
            .map(|text| (file.display().to_string(), text))
    }
}

/// What `elya resolve --dump <path>` prints, and the diagnostics' codes.
pub fn oracle(path: &Path) -> (String, Vec<String>) {
    let text = std::fs::read_to_string(path).expect("the root");
    let name = path.display().to_string();
    let (program, diags) = elya::resolved(&elya::Session::new(), &name, &text, &mut files(path));
    let merged = elya::merged_text(&program.sources);
    let mut out = elya::ast_dump::dump(&program.module, &merged);
    out.push_str(&elya::ast_dump::dump_diagnostics(&diags));
    (out, diags.into_iter().map(|d| d.code).collect())
}

/// Writes `p` in its own directory `dir/c<i>`; returns the root's path.
pub fn materialise(dir: &Path, i: usize, p: &Program) -> PathBuf {
    let case = dir.join(format!("c{i}"));
    std::fs::create_dir_all(&case).expect("case directory");
    for (path, text) in &p.files {
        let file = case.join(format!("{path}.elya"));
        std::fs::create_dir_all(file.parent().unwrap()).expect("module directory");
        std::fs::write(&file, text).expect("write module");
    }
    let root = case.join("main.elya");
    std::fs::write(&root, &p.root).expect("write root");
    root
}

/// Every program of the parser's corpus, a root alone.
pub fn corpus_programs(root: &Path) -> Vec<Program> {
    super::parse_support::parity_corpus(root)
        .cases
        .into_iter()
        .map(|c| Program {
            name: format!("corpus {}", c.name),
            root: c.text,
            files: Vec::new(),
        })
        .collect()
}

struct Lit {
    start: usize,
    end: usize,
    text: String,
}

/// The string literals of Rust source with their extents (the parser
/// corpus's extractor, `rust_strings`, gives each one's start; its end is
/// found by scanning the literal again).
fn literals(src: &str) -> Vec<Lit> {
    super::parse_support::rust_strings(src)
        .into_iter()
        .map(|(start, text)| Lit {
            start,
            end: literal_end(src, start),
            text,
        })
        .collect()
}

fn literal_end(src: &str, start: usize) -> usize {
    let b = src.as_bytes();
    if b[start] == b'r' {
        let mut q = start + 1;
        while b[q] == b'#' {
            q += 1;
        }
        let hashes = &src[start + 1..q];
        let close = format!("\"{hashes}");
        return q + 1 + src[q + 1..].find(&close).expect("raw literal closes") + close.len();
    }
    let mut i = start + 1;
    while b[i] != b'"' {
        i += if b[i] == b'\\' { 2 } else { 1 };
    }
    i + 1
}

fn is_module_path(s: &str) -> bool {
    !s.is_empty()
        && s.split('/').all(|seg| {
            seg.chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
                && seg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

fn ident_at(src: &str, at: usize) -> &str {
    let end = src[at..]
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map_or(src.len(), |n| at + n);
    &src[at..end]
}

fn skip_ws(src: &str, mut at: usize) -> usize {
    while at < src.len() && src.as_bytes()[at].is_ascii_whitespace() {
        at += 1;
    }
    at
}

/// `const NAME: &str = "...";` and `= concat!("...", ...)`.
fn consts(src: &str, lits: &[Lit]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut from = 0;
    while let Some(n) = src[from..].find("const ") {
        let at = from + n + 6;
        from = at;
        let name = ident_at(src, at);
        let rest = &src[at + name.len()..];
        let Some(rest) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix("&str") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = src.len() - rest.len();
        let value = skip_ws(src, value);
        if src[value..].starts_with("concat!(") {
            let close = value + src[value..].find(");").unwrap_or(src.len() - value);
            let text: String = lits
                .iter()
                .filter(|l| l.start > value && l.end <= close)
                .map(|l| l.text.as_str())
                .collect();
            out.insert(name.to_string(), text);
        } else if let Some(l) = lits.iter().find(|l| l.start == value) {
            out.insert(name.to_string(), l.text.clone());
        }
    }
    out
}

/// A module given in a test: its path and text, and where it was written.
struct Given {
    start: usize,
    end: usize,
    path: String,
    text: String,
}

/// The value after a tuple's or a call's comma: a literal or a constant.
fn value_at(
    src: &str,
    at: usize,
    lits: &[Lit],
    consts: &BTreeMap<String, String>,
) -> Option<(String, usize)> {
    let at = skip_ws(src, at);
    if let Some(l) = lits.iter().find(|l| l.start == at) {
        return Some((l.text.clone(), l.end));
    }
    let at = if src[at..].starts_with('&') {
        at + 1
    } else {
        at
    };
    let name = ident_at(src, at);
    if name.is_empty() {
        return None;
    }
    let end = at + name.len();
    // A `let` of a literal before it (the nearest), else a constant.
    let local = [format!("let {name} ="), format!("let {name}: &str =")]
        .iter()
        .filter_map(|pat| src[..at].rfind(pat.as_str()).map(|d| d + pat.len()))
        .max()
        .and_then(|v| {
            let v = skip_ws(src, v);
            lits.iter().find(|l| l.start == v).map(|l| l.text.clone())
        });
    local
        .or_else(|| consts.get(name).cloned())
        .map(|t| (t, end))
}

/// `("a/b", text)` tuples and `dir.join("a/b.elya"), text` writes.
fn given(src: &str, lits: &[Lit], consts: &BTreeMap<String, String>) -> Vec<Given> {
    let mut out = Vec::new();
    for l in lits {
        let before = src[..l.start].trim_end();
        let after = skip_ws(src, l.end);
        if !src[after..].starts_with(',') {
            if let Some(stem) = l.text.strip_suffix(".elya") {
                let after_close = skip_ws(src, after + 1);
                if before.ends_with("join(")
                    && src[after..].starts_with(')')
                    && is_module_path(stem)
                    && src[after_close..].starts_with(',')
                {
                    if let Some((text, end)) = value_at(src, after_close + 1, lits, consts) {
                        out.push(Given {
                            start: l.start,
                            end,
                            path: stem.to_string(),
                            text,
                        });
                    }
                }
            }
            continue;
        }
        if !before.ends_with('(') || !is_module_path(&l.text) {
            continue;
        }
        let Some((text, end)) = value_at(src, after + 1, lits, consts) else {
            continue;
        };
        let close = skip_ws(src, end);
        if src[close..].starts_with(')') {
            out.push(Given {
                start: before.len() - 1,
                end: close + 1,
                path: l.text.clone(),
                text,
            });
        }
    }
    out
}

/// Whether `text` is a program importing a user module.
fn imports_a_user_module(text: &str) -> bool {
    let (m, _) = elya::parse::parse_module(&elya::Session::new(), text);
    m.imports
        .iter()
        .any(|i| i.node.path.first().map(String::as_str) != Some("elya"))
}

/// Every multi-file program in the Rust tests: within each test, every
/// root importing a user module, over each set of modules the test gives
/// (adjacent tuples, or a constant table the test names), and the
/// two-file helper of tests/diagnostics_edges.rs.
pub fn multi_file_programs(root: &Path) -> Vec<Program> {
    let mut paths = Vec::new();
    for dir in ["tests", "crates/codegen/tests", "crates/cli/tests"] {
        for entry in std::fs::read_dir(root.join(dir)).expect("tests directory") {
            let p = entry.expect("entry").path();
            if p.extension().is_some_and(|e| e == "rs") {
                paths.push(p);
            }
        }
    }
    paths.sort();
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for path in paths {
        let src = std::fs::read_to_string(&path).expect("Rust test");
        let label = path.strip_prefix(root).unwrap().display().to_string();
        for p in programs_of(&label, &src, path.parent().unwrap()) {
            if seen.insert((p.root.clone(), p.files.clone())) {
                out.push(p);
            }
        }
    }
    out
}

fn programs_of(label: &str, src: &str, dir: &Path) -> Vec<Program> {
    let lits = literals(src);
    let mut consts = consts(src, &lits);
    // The constants of a `#[path = "..."]` module (tests/programs/*.rs).
    for (at, _) in src.match_indices("#[path = \"") {
        let file = &src[at + 10..];
        let Some(end) = file.find('"') else { continue };
        if let Ok(other) = std::fs::read_to_string(dir.join(&file[..end])) {
            let other_lits = literals(&other);
            for (k, v) in self::consts(&other, &other_lits) {
                consts.entry(k).or_insert(v);
            }
        }
    }
    let given = given(src, &lits, &consts);
    // Adjacent tuples form one set.
    let mut groups: Vec<Vec<&Given>> = Vec::new();
    for g in &given {
        let joined = groups.last().and_then(|last| last.last()).is_some_and(|p| {
            src[p.end..g.start]
                .lines()
                .map(|l| l.split("//").next().unwrap())
                .all(|l| l.chars().all(|c| c.is_whitespace() || c == ','))
        });
        if joined {
            groups.last_mut().unwrap().push(g);
        } else {
            groups.push(vec![g]);
        }
    }
    let tests: Vec<usize> = src.match_indices("#[test]").map(|(i, _)| i).collect();
    let mut out = Vec::new();
    for (t, &start) in tests.iter().enumerate() {
        let end = tests.get(t + 1).copied().unwrap_or(src.len());
        let region = &src[start..end];
        // The sets written in the test, and the constant tables it names.
        let mut sets: Vec<&Vec<&Given>> = groups
            .iter()
            .filter(|g| g[0].start >= start && g[0].start < end)
            .collect();
        for g in &groups {
            if g[0].start >= tests[0] {
                continue;
            }
            // A table before the tests: a helper's reader may read it.
            sets.push(g);
        }
        let mut roots: Vec<String> = lits
            .iter()
            .filter(|l| l.start >= start && l.end <= end)
            .map(|l| l.text.clone())
            .collect();
        for (name, text) in &consts {
            if region.contains(name.as_str()) {
                roots.push(text.clone());
            }
        }
        roots.retain(|r| imports_a_user_module(r));
        for (k, set) in sets.iter().enumerate() {
            let files: BTreeMap<String, String> = set
                .iter()
                .filter(|g| g.path != "main")
                .map(|g| (g.path.clone(), g.text.clone()))
                .collect();
            for (j, r) in roots.iter().enumerate() {
                out.push(Program {
                    name: format!("{label} test {t} set {k} root {j}"),
                    root: r.clone(),
                    files: files.clone().into_iter().collect(),
                });
            }
        }
        // `two_files(root, b)`.
        let mut from = start;
        while let Some(n) = src[from..end].find("two_files(") {
            let at = from + n + "two_files(".len();
            from = at;
            let Some((r, after)) = value_at(src, at, &lits, &consts) else {
                continue;
            };
            let comma = skip_ws(src, after);
            if !src[comma..].starts_with(',') {
                continue;
            }
            if let Some((b, _)) = value_at(src, comma + 1, &lits, &consts) {
                out.push(Program {
                    name: format!("{label} test {t} two_files"),
                    root: r,
                    files: vec![("b".to_string(), b)],
                });
            }
        }
    }
    out
}

fn program(name: &str, root: &str, files: &[(&str, &str)]) -> Program {
    Program {
        name: format!("focused {name}"),
        root: root.to_string(),
        files: files
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect(),
    }
}

const UTIL: &str = "pub fn twice(x: Int) -> Int { x * 2 }\nfn hidden() -> Int { 1 }\n";

/// The focused set: each code and each renaming rule.
pub fn focused() -> Vec<Program> {
    let u = [("util", UTIL)];
    vec![
        // Renaming: a qualified call, an alias, a module's own calls.
        program("qualified call", "import util\npub fn main() -> Int { util.twice(21) }\n", &u),
        program("alias", "import util as u\npub fn main() -> Int { u.twice(21) }\n", &u),
        program("qualified value", "import util\npub fn main() -> Int { let g = util.twice\n g(5) }\n", &u),
        program(
            "own calls renamed",
            "import m\npub fn main() -> Int { m.f(1) }\n",
            &[("m", "pub fn f(x: Int) -> Int { g(x) + h() }\nfn g(y: Int) -> Int { y }\nfn h() -> Int { g(2) }\n")],
        ),
        // Shadowing: each binder hides a module function of its name.
        program(
            "shadowed by a parameter",
            "import m\npub fn main() -> Int { m.f(1) }\n",
            &[("m", "fn g() -> Int { 1 }\npub fn f(g: Int) -> Int { g + h(g) }\nfn h(x) { g() }\n")],
        ),
        program(
            "shadowed by a let",
            "import m\npub fn main() -> Int { m.f() }\n",
            &[("m", "fn g() -> Int { 1 }\npub fn f() -> Int { let a = g()\n let g = 2\n a + g }\n")],
        ),
        program(
            "a let's value sees the function",
            "import m\npub fn main() -> Int { m.f() }\n",
            &[("m", "fn g() -> Int { 1 }\npub fn f() -> Int { let g = g()\n g }\n")],
        ),
        program(
            "shadowed in a block only",
            "import m\npub fn main() -> Int { m.f() }\n",
            &[("m", "fn g() -> Int { 1 }\npub fn f() -> Int { let a = { let g = 3\n g }\n a + g() }\n")],
        ),
        program(
            "shadowed by a lambda parameter",
            "import m\npub fn main() -> Int { m.f() }\n",
            &[("m", "fn g() -> Int { 1 }\npub fn f() -> Int { let k = fn(g) { g }\n k(g()) }\n")],
        ),
        program(
            "shadowed by a match binder",
            "import m\npub fn main() -> Int { m.f(m.mk()) }\n",
            &[("m", "type B { B(Int) }\npub fn mk() -> B { B(1) }\nfn g() -> Int { 1 }\npub fn f(b: B) -> Int { match b { B(g) -> g } + g() }\n")],
        ),
        program(
            "shadowed by a clause parameter and a return binder",
            "import m\npub fn main() -> Int { m.f() }\n",
            &[("m", "effect Ask { fn ask(n: Int) -> Int }\nfn g() -> Int { 1 }\npub fn f() -> Int { handle ask(g()) with { Ask.ask(g) -> resume(g) return(g) -> g } }\n")],
        ),
        program(
            "a root function keeps its name",
            "import m\nfn helper() -> Int { 2 }\npub fn main() -> Int { m.f() + helper() }\n",
            &[("m", "pub fn f() -> Int { 1 }\n")],
        ),
        // An import of an import, and a module imported twice.
        program(
            "import of an import",
            "import a\npub fn main() -> Int { a.f() }\n",
            &[("a", "import lib/b\npub fn f() -> Int { b.g() }\n"), ("lib/b", "pub fn g() -> Int { 3 }\n")],
        ),
        program(
            "an import's import is not the root's",
            "import a\npub fn main() -> Int { a.f() + b.g() }\n",
            &[("a", "import b\npub fn f() -> Int { b.g() }\n"), ("b", "pub fn g() -> Int { 3 }\n")],
        ),
        program(
            "one module, two names, loaded once",
            "import a\nimport a as c\nimport d\npub fn main() -> Int { a.f() + c.f() + d.h() }\n",
            &[("a", "pub fn f() -> Int { 1 }\n"), ("d", "import a\npub fn h() -> Int { a.f() }\n")],
        ),
        program(
            "a diamond, files in the order reached",
            "import l\nimport r\npub fn main() -> Float { l.f() +. r.g() }\n",
            &[("l", "import base\npub fn f() -> Float { base.x() }\n"), ("r", "import base\npub fn g() -> Float { 2.5_0 +. base.x() }\n"), ("base", "pub fn x() -> Float { 1.0e1 }\n")],
        ),
        // Standard modules.
        program("a std module", "import elya/map\npub fn main() -> Int { let m: Array(MapSlot(Int)) = map.new()\n map.size(m) }\n", &[]),
        program("a std module aliased", "import elya/map as tbl\npub fn main() -> Int { let m: Array(MapSlot(Int)) = tbl.new()\n tbl.size(m) }\n", &[]),
        program(
            "a std module from a module",
            "import a\npub fn main() -> Int { a.f() }\n",
            &[("a", "import elya/map\npub fn f() -> Int { let m: Array(MapSlot(Int)) = map.new()\n map.size(m) }\n")],
        ),
        program("builtin modules", "import elya/io\nimport elya/int\npub fn main() { io.println(int.to_string(1)) }\n", &[]),
        // E0600.
        program("E0600 no file", "import nope\npub fn main() -> Int { 1 }\n", &[]),
        program("E0600 no std module", "import elya/nope\npub fn main() -> Int { 1 }\n", &[]),
        program("E0600 refused once", "import nope\npub fn main() -> Int { nope.f() + nope.g }\n", &[]),
        program("E0600 std not refused", "import elya/nope\npub fn main() -> Int { nope.f() }\n", &[]),
        program("E0600 in a module", "import a\npub fn main() -> Int { a.f() }\n", &[("a", "import gone\npub fn f() -> Int { gone.f() }\n")]),
        program("E0600 a module path", "import lib/deep/x\npub fn main() -> Int { x.f() }\n", &[]),
        // E0601.
        program("E0601 the root", "import main\npub fn main() -> Int { main.main() }\n", &[]),
        program(
            "E0601 two modules",
            "import a\npub fn main() -> Int { a.f() }\n",
            &[("a", "import b\npub fn f() -> Int { b.g() }\n"), ("b", "import a\npub fn g() -> Int { a.f() }\n")],
        ),
        program(
            "E0601 through the root",
            "import a\npub fn main() -> Int { a.f() }\n",
            &[("a", "import main\npub fn f() -> Int { main.main() }\n")],
        ),
        // E0602, E0603.
        program("E0602 private", "import util\npub fn main() -> Int { util.hidden() }\n", &u),
        program("E0603 no function", "import util\npub fn main() -> Int { util.nothing(1) }\n", &u),
        program("E0603 as a value", "import util\npub fn main() -> Int { let f = util.nothing\n 1 }\n", &u),
        // E0604.
        program("E0604 std renamed", "import elya/io as out\npub fn main() -> Int { 1 }\n", &[]),
        program("E0604 a builtin's name", "import util as io\npub fn main() -> Int { 1 }\n", &u),
        program("E0604 another import", "import util\nimport lib/util\npub fn main() -> Int { util.twice(1) }\n", &u),
        program("E0604 std renamed with a list", "import elya/io as out / {IO}\npub fn main() -> Int { 1 }\n", &[]),
        // E0605.
        program(
            "E0605 a type not imported",
            "import a\nimport b\npub fn main() -> Int { 0 }\n",
            &[("a", "type T { TA }\n"), ("b", "pub fn f(t: T) -> Int { 0 }\nfn g() { TA }\n")],
        ),
        program(
            "E0605 the root's type",
            "import b\ntype R { RC }\npub fn main() -> Int { 0 }\n",
            &[("b", "fn f(r: R) -> Int { match r { RC -> 0 } }\n")],
        ),
        program(
            "E0605 an effect and an operation",
            "import a\nimport b\npub fn main() -> Int { 0 }\n",
            &[("a", "effect Tick { fn tick() -> Unit }\n"), ("b", "fn f() / {Tick} -> Unit { tick() }\nfn g() -> Int { handle 1 with { Tick.tick() -> resume(Unit) } }\n")],
        ),
        program(
            "E0605 visible through an import",
            "import b\npub fn main() -> Int { b.f(b.mk()) }\n",
            &[("a", "type T { TA }\n"), ("b", "import a\npub fn mk() -> T { TA }\npub fn f(t: T) -> Int { match t { TA -> 0 } }\n")],
        ),
        program(
            "E0200 a root function from a module",
            "import b\nfn secret() -> Int { 1 }\npub fn main() -> Int { b.f() }\n",
            &[("b", "pub fn f() -> Int { secret() }\n")],
        ),
        // E0607, E0608.
        program("E0607 listed twice", "import util / {IO, IO}\npub fn main() -> Int { 1 }\n", &u),
        program("E0607 not visible", "import util / {Tock}\npub fn main() -> Int { 1 }\n", &u),
        program(
            "E0607 visible effects",
            "import a / {Tick}\nimport b / {Tick, Tock}\npub fn main() -> Int { 1 }\n",
            &[("a", "effect Tick { fn tick() -> Unit }\n"), ("b", "pub fn f() -> Int { 1 }\n")],
        ),
        program("E0607 on a missing module", "import gone / {Gone}\npub fn main() -> Int { 1 }\n", &[]),
        program("E0608 std with a list", "import elya/io / {IO}\nimport elya/zz / {}\npub fn main() -> Int { 1 }\n", &[]),
        // E0205 in a module, E0209 across files.
        program(
            "E0205 in a module",
            "import a\npub fn main() -> Int { 1 }\n",
            &[("a", "effect Ask { fn ask() -> Int }\nfn ask() -> Int { 1 }\n")],
        ),
        program(
            "E0209 a type in two files",
            "import b\ntype T { B }\npub fn main() -> Int { b.g() }\n",
            &[("b", "pub fn g() -> Int { 2 }\ntype T { A }\n")],
        ),
        // Parse errors in an imported file, with its spans.
        program(
            "a module's parse error",
            "import a\npub fn main() -> Int { a.f() }\n",
            &[("a", "pub fn f() -> Int { 1 +\n}\nfn g( { }\n")],
        ),
        // Resolution codes in one file.
        program("E0200", "fn f() { x }\nfn g(a) { a + b }\n", &[]),
        program("E0201 called and as a value", "fn f() { io.nope(1) }\nfn g() { let h = io.nope\n h }\n", &[]),
        program("E0202", "effect A { fn op() -> Int }\neffect B { fn op() -> Int }\nfn f() { op() }\n", &[]),
        program(
            "E0203 each way",
            "effect A { fn a(x: Int) -> Int }\neffect B { fn b() -> Int }\nfn f() { handle 1 with { zz() -> 1  B.a(x) -> 1  Q.a(x) -> 2  a() -> 3  Q.zz() -> 4 } }\n",
            &[],
        ),
        program("E0204", "effect A { fn a() -> Int }\nfn f() { handle 1 with { a() -> 1  A.a() -> 2 } }\n", &[]),
        program("E0205", "effect A { fn a() -> Int }\nfn a() -> Int { 1 }\n", &[]),
        program("E0206", "effect A { fn a(x: Int, y: Int) -> Int }\nfn f() { let g = a\n let h = fn(a) { a }\n g }\n", &[]),
        program(
            "E0207",
            "effect A { fn a() -> Int  fn b(n: Int) -> Int  fn c() -> Int }\nfn f() { handle 1 with { a() -> 1 } }\nfn g() { handle 1 with { a() -> 1  b(n) -> 2  c() -> 3 } }\n",
            &[],
        ),
        program("E0208", "fn f() { let g = int.to_string\n let h = array.push\n g }\n", &[]),
        program(
            "E0209 each kind",
            "fn f() { 1 }\nfn f() { 2 }\ntype T { C }\ntype T { D, C }\neffect E { fn e() -> Int }\neffect E { fn e2() -> Int }\neffect IO { fn io_op() -> Int }\n",
            &[],
        ),
        program("E0210", "fn f() { resume(1) }\nfn g() { handle resume(2) with { return(x) -> resume(x) } }\n", &[]),
        program("E0211", "type Buffer { B }\ntype Array { A }\n", &[]),
        program("E0432", "fn f(x) { match x { Nope -> 1  Some(y) -> y  _ -> 0 } }\n", &[]),
        program(
            "resume in a lambda inside a clause",
            "effect A { fn a() -> Int }\nfn f() { handle a() with { a() -> { let k = fn() { resume(1) }\n k() } } }\n",
            &[],
        ),
        program(
            "a with-any handler",
            "fn f() { handle 1 with any { op(o) -> op.perform(o) return(x) -> x } }\nfn g() { handle 1 with any { op(o) -> zz(o) } }\n",
            &[],
        ),
        program(
            "a local shadows an operation",
            "effect A { fn a() -> Int }\nfn f(a) { a() + a }\n",
            &[],
        ),
        // Both passes' diagnostics, the modules' first (K3), and R3's filter
        // beside an unfiltered E0201 (K2).
        program(
            "both passes",
            "import nope\nimport util\npub fn main() -> Int { zz + nope.f() + util.nothing() + io.zz() }\n",
            &u,
        ),
        program(
            "a module's resolution errors",
            "import a\npub fn main() -> Int { a.f() }\n",
            &[("a", "pub fn f() -> Int { y + io.zz() + Nope }\n")],
        ),
        program(
            "a cycle's qualified use is refused",
            "import a\npub fn main() -> Int { a.f() }\n",
            &[("a", "import main as m\npub fn f() -> Int { m.g() }\n")],
        ),
        program(
            "a failed import and a later bind of its name",
            "import nope\nimport util as nope\npub fn main() -> Int { nope.twice(1) + nope.zz() }\n",
            &u,
        ),
        program(
            "a module's operation called and handled",
            "import a\npub fn main() -> Int { handle a.f() with { A.ask() -> resume(1) } }\n",
            &[("a", "effect A { fn ask() -> Int }\npub fn f() / {A} -> Int { ask() + 1 }\n")],
        ),
        program(
            "a module's constructor used by the root",
            "import a\npub fn main() -> Int { match a.mk() { Pt(x) -> x } + h(Pt(1)) }\nfn h(p: P) -> Int { 0 }\n",
            &[("a", "type P { Pt(Int) }\npub fn mk() -> P { Pt(2) }\n")],
        ),
        // Review 2026-11-27: one program per surviving mutant.
        program("E0605 constructors beginning with A or Z, and of one letter", "import a\nimport b\npub fn main() -> Int { 0 }\n",
            &[("a", "type T { Zed, Apple, Q }\n"), ("b", "fn g() { Zed }\nfn h() { Apple }\nfn k() { Q }\n")]),
        program("E0607 an effect of a module that imports another", "import b / {Tick}\npub fn main() -> Int { 1 }\n",
            &[("b", "import c\neffect Tick { fn tick() -> Unit }\n"), ("c", "pub fn f() -> Int { 1 }\n")]),
        program("E0607 lists on a builtin and a std module", "import elya/io / {Zap}\nimport elya/console / {Zap2}\npub fn main() -> Int { 1 }\n", &[]),
        program("names declared in two files, used where declared", "import a\nimport b\npub fn main() -> Int { 0 }\n",
            &[("a", "type T { C1 }\neffect E { fn op() -> Int }\neffect F { fn x() -> Int }\n"),
              ("b", "type U { C1 }\neffect G { fn op() -> Int }\neffect F { fn y() -> Int }\nfn g() / {F} -> Int { match C1 { C1 -> op() } }\nfn h() -> Int { handle 1 with { G.op() -> 1 } }\n")]),
        program("E0205 in the root of a program with modules", "import util\neffect A { fn ask() -> Int }\nfn ask() -> Int { 1 }\npub fn main() -> Int { 1 }\n", &u),
        program("E0205 in two modules", "import a\nimport b\npub fn main() -> Int { 1 }\n",
            &[("a", "effect A { fn ask() -> Int }\nfn ask() -> Int { 1 }\n"), ("b", "effect B { fn tick() -> Int }\nfn tick() -> Int { 1 }\n")]),
        program("E0605 visible through an import that declares it too", "import x\nimport z\npub fn main() -> Int { 0 }\n",
            &[("x", "type T { A }\n"), ("y", "type T { B }\n"), ("z", "import y\nfn f(t: T) -> Int { 0 }\n")]),
        program("E0605 in a label's argument", "import a\nimport b\npub fn main() -> Int { 0 }\n",
            &[("a", "type T { TA }\n"), ("b", "fn f() / {Tick(T)} -> Int { 0 }\n")]),
        program("E0605 in an operation's signature and a return type", "import a\nimport b\npub fn main() -> Int { 0 }\n",
            &[("a", "type T { TA }\n"), ("b", "effect E { fn op(n: T) -> Int  fn op2() -> T }\nfn g() -> T { TA }\n")]),
        program("a let in a block without a tail", "import m\npub fn main() -> Int { m.f() }\n",
            &[("m", "fn g() -> Int { 1 }\npub fn f() -> Int { let u = if True { let g = 1 } else { Unit }\n g() }\n")]),
        program("a module's operation named like a root function", "import a\nfn ask() -> Int { 1 }\npub fn main() -> Int { a.f() }\n",
            &[("a", "effect Ask { fn ask() -> Int }\npub fn f() / {Ask} -> Int { ask() }\n")]),
        program("a clause parameter's scope ends with its clause", "import m\npub fn main() -> Int { m.f() }\n",
            &[("m", "effect A { fn a(x: Int) -> Int  fn b() -> Int }\nfn g() -> Int { 1 }\npub fn f() -> Int { handle a(1) with { a(g) -> resume(g)  b() -> resume(g()) } + g() }\n")]),
        program("a match arm's scope is its own", "import m\npub fn main() -> Int { m.f(m.mk()) }\n",
            &[("m", "type B { B(Int) }\npub fn mk() -> B { B(1) }\nfn h() -> Int { 1 }\npub fn f(b: B) -> Int { let h = 1\n match b { B(z) -> z } + h }\n")]),
        program("a return clause's binder is its own", "import m\npub fn main() -> Int { m.f() }\n",
            &[("m", "fn g() -> Int { 1 }\nfn h() -> Int { 1 }\npub fn f() -> Int { let h = 1\n handle 1 with { return(g) -> g } + g() + h }\n")]),
    ]
}

/// Each code's count over `codes`, one program at most once per code.
pub fn count_codes(per_program: &[Vec<String>]) -> BTreeMap<&'static str, usize> {
    let mut out = BTreeMap::new();
    for code in CODES {
        out.insert(
            *code,
            per_program
                .iter()
                .filter(|cs| cs.iter().any(|c| c == code))
                .count(),
        );
    }
    out
}
