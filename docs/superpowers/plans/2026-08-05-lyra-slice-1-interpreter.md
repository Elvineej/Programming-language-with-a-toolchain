# Lyra Slice 1 — Tree-Walking Interpreter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a running Lyra interpreter that executes a small subset of the language (literals, arithmetic, `let`, `if/else`, function definitions and calls, and `io.println`) end-to-end through lexer → parser → name-check → tree-walking evaluator, with the test and diagnostics infrastructure that later slices build on.

**Architecture:** One Rust crate `lyra` (library + binary). Each compiler pass is a pure function `fn(&Session, In) -> (Out, Vec<Diagnostic>)` living in its own module; modules are layered (`span/diag → lex → ast → parse → resolve → eval → cli`) and a test enforces the layering so a future workspace split stays mechanical. Slice 1 uses a plain recursive tree-walker (no type checker); the CEK refactor and real types arrive in Slice 2.

**Tech Stack:** Rust (edition 2021), `logos` (lexer), `ariadne` (diagnostic rendering), `insta` (snapshot tests). Dev tooling: `rustfmt`, `clippy`, a local `scripts/check` script wired to a git pre-push hook.

## Global Constraints

- **Language/edition:** Rust, `edition = "2021"`. Toolchain channel `stable` pinned via `rust-toolchain.toml`; MSRV floor 1.75.
- **Dependencies (runtime):** exactly `logos = "0.14"` and `ariadne = "0.4"`. No others in Slice 1. **Dev-dependencies:** `insta = "1"`.
- **Single crate:** `lyra`, library `src/lib.rs` + binary `src/main.rs`. No workspace yet.
- **Pass signature rule:** every pass is `pub fn name(&Session, In) -> (Out, Vec<Diagnostic>)` (or `-> Vec<Diagnostic>` when it produces no value). No global mutable state, no `thread_local`, no `lazy_static`. Shared state lives in `Session`.
- **Spans everywhere:** every token and every AST node carries a `Span` from the moment it is created.
- **Module layer order (pinned):** `span = 0, diag = 0, lex = 1, ast = 1, parse = 2, resolve = 3, types = 4, core = 5, eval = 6, main/cli = 7`. A module may only reference modules at a **strictly lower** layer (equal-layer `span`/`diag` and `lex`/`ast` references are allowed). `tests/arch/layering.rs` enforces this.
- **Diagnostic code scheme:** `E00xx` lexing, `E01xx` parsing, `E02xx` name resolution, `E03xx` runtime. Codes `E042x` are reserved for effects (Slice 3) — do not use them here.
- **Error handling:** passes never `panic!`/`unwrap()` on malformed input; they emit `Diagnostic`s. `unwrap()` is allowed only on invariants that cannot fail given earlier passes, with a `// invariant:` comment.
- **Git:** strictly local; **no remote**, no push. Commit after every task's final step.
- **Line endings:** repo is authored LF; `.gitattributes` normalizes.

---

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` | Crate manifest, deps, lib+bin targets |
| `rust-toolchain.toml` | Pin stable channel |
| `.gitignore`, `.gitattributes` | Ignore `/target`; normalize LF |
| `scripts/check.sh`, `scripts/check.ps1` | Local CI (fmt + clippy + test) |
| `src/lib.rs` | `Session`, pipeline wiring (`run_source`, `check_source`), module declarations |
| `src/main.rs` | CLI: `lyra run <file>` / `lyra check <file>` |
| `src/span.rs` | `Span`, `Spanned<T>`, `SourceMap` (offset→line/col) |
| `src/diag.rs` | `Severity`, `Label`, `Diagnostic`, `render()` |
| `src/lex.rs` | `TokenKind`, `Token`, `lex()` |
| `src/ast.rs` | AST node types (Slice 1 subset) + `pretty()` |
| `src/parse.rs` | Recursive-descent + Pratt parser with recovery |
| `src/resolve.rs` | Name checking, builtin table, `E02xx` diagnostics |
| `src/eval.rs` | `Value`, `Env`, tree-walker, builtins, depth instrumentation |
| `examples/*.lyra` | Slice-1-runnable example programs (golden tests) |
| `tests/examples.rs` | Runs `examples/*.lyra`, snapshots output |
| `tests/ui.rs` + `tests/ui/*.lyra` | `//~ ERROR[Ennnn]` diagnostic fixtures |
| `tests/arch/layering.rs` | Module-layer DAG enforcement |
| `tests/tce.rs` | Depth-instrumentation "grow" control (bounded assertions land Slice 2) |

---

## Task 1: Project scaffold

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.gitignore`, `.gitattributes`
- Create: `src/lib.rs`, `src/main.rs`
- Create: `scripts/check.sh`, `scripts/check.ps1`

**Interfaces:**
- Produces: crate `lyra` builds; `lib.rs` declares empty modules `span, diag, lex, ast, parse, resolve, eval`; `pub struct Session` placeholder.

- [ ] **Step 1: Create the Cargo manifest**

`Cargo.toml`:
```toml
[package]
name = "lyra"
version = "0.0.1"
edition = "2021"
rust-version = "1.75"

[lib]
name = "lyra"
path = "src/lib.rs"

[[bin]]
name = "lyra"
path = "src/main.rs"

[dependencies]
logos = "0.14"
ariadne = "0.4"

[dev-dependencies]
insta = "1"
```

- [ ] **Step 2: Pin toolchain and add repo hygiene files**

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

`.gitignore`:
```gitignore
/target
**/*.rs.bk
```

`.gitattributes`:
```gitattributes
* text=auto eol=lf
```

- [ ] **Step 3: Create the library root with empty modules**

`src/lib.rs`:
```rust
//! The Lyra compiler (Slice 1: tree-walking interpreter).

pub mod span;
pub mod diag;
pub mod lex;
pub mod ast;
pub mod parse;
pub mod resolve;
pub mod eval;

/// Shared, explicitly-threaded compiler state. No globals live outside this.
#[derive(Debug, Default)]
pub struct Session {}

impl Session {
    pub fn new() -> Self {
        Session {}
    }
}
```

Create empty module files so the crate compiles: `src/span.rs`, `src/diag.rs`, `src/lex.rs`, `src/ast.rs`, `src/parse.rs`, `src/resolve.rs`, `src/eval.rs`, each containing only a doc comment line (e.g. `//! Spans.`).

- [ ] **Step 4: Create a minimal binary**

`src/main.rs`:
```rust
fn main() {
    eprintln!("lyra: no command yet (scaffold)");
    std::process::exit(2);
}
```

- [ ] **Step 5: Create the local check scripts**

`scripts/check.sh`:
```sh
#!/usr/bin/env sh
set -e
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

`scripts/check.ps1`:
```powershell
$ErrorActionPreference = "Stop"
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit 1 }
cargo clippy --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit 1 }
cargo test --all
if ($LASTEXITCODE -ne 0) { exit 1 }
```

- [ ] **Step 6: Verify it builds and commit**

Run: `cargo build`
Expected: compiles with no errors.

```bash
git add -A
git commit -m "chore: scaffold lyra crate (lib+bin, deps, check scripts)"
```

---

## Task 2: `span` module

**Files:**
- Modify: `src/span.rs`
- Test: inline `#[cfg(test)]` in `src/span.rs`

**Interfaces:**
- Produces:
  - `pub struct Span { pub start: u32, pub end: u32 }` with `Span::new(u32,u32)`, `merge(self, Span) -> Span`, `Span::EMPTY`.
  - `pub struct Spanned<T> { pub node: T, pub span: Span }` with `pub fn spanned<T>(node: T, span: Span) -> Spanned<T>`.
  - `pub struct SourceMap` with `SourceMap::new(name, text)`, `location(&self, offset: u32) -> (u32, u32)` (1-based line, col), `text(&self) -> &str`, `name(&self) -> &str`.

- [ ] **Step 1: Write the failing test**

In `src/span.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_maps_offset_to_line_and_col() {
        let sm = SourceMap::new("t.lyra", "ab\ncd\n");
        assert_eq!(sm.location(0), (1, 1)); // 'a'
        assert_eq!(sm.location(1), (1, 2)); // 'b'
        assert_eq!(sm.location(3), (2, 1)); // 'c'
        assert_eq!(sm.location(4), (2, 2)); // 'd'
    }

    #[test]
    fn merge_covers_both_spans() {
        let a = Span::new(2, 5);
        let b = Span::new(7, 9);
        assert_eq!(a.merge(b), Span::new(2, 9));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib span`
Expected: FAIL — `SourceMap`, `Span` not found.

- [ ] **Step 3: Write the implementation**

Prepend to `src/span.rs`:
```rust
//! Byte spans, spanned nodes, and offset→line/col lookup.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const EMPTY: Span = Span { start: 0, end: 0 };

    pub fn new(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    pub fn merge(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

pub fn spanned<T>(node: T, span: Span) -> Spanned<T> {
    Spanned { node, span }
}

/// Owns one source file's text and precomputed line offsets.
#[derive(Debug)]
pub struct SourceMap {
    name: String,
    text: String,
    line_starts: Vec<u32>,
}

impl SourceMap {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceMap {
        let text = text.into();
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        SourceMap { name: name.into(), text, line_starts }
    }

    /// 1-based (line, column) for a byte offset.
    pub fn location(&self, offset: u32) -> (u32, u32) {
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let col = offset - self.line_starts[line_idx] + 1;
        (line_idx as u32 + 1, col)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib span`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/span.rs
git commit -m "feat(span): spans, spanned nodes, source map line/col lookup"
```

---

## Task 3: `diag` module

**Files:**
- Modify: `src/diag.rs`
- Test: inline `#[cfg(test)]` in `src/diag.rs`

**Interfaces:**
- Consumes: `span::{Span, SourceMap}`.
- Produces:
  - `pub enum Severity { Error, Warning }`
  - `pub struct Label { pub span: Span, pub message: String }`
  - `pub struct Diagnostic { pub severity, pub code: String, pub message: String, pub labels: Vec<Label>, pub helps: Vec<String> }`
  - Builders: `Diagnostic::error(code: &str, message: impl Into<String>)`, `.with_label(Span, impl Into<String>)`, `.with_help(impl Into<String>)`.
  - `pub fn render(diags: &[Diagnostic], src: &SourceMap) -> String`.

- [ ] **Step 1: Write the failing test**

In `src/diag.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::{SourceMap, Span};

    #[test]
    fn builder_sets_fields() {
        let d = Diagnostic::error("E0200", "unresolved name `foo`")
            .with_label(Span::new(0, 3), "not found in this scope")
            .with_help("did you mean `food`?");
        assert_eq!(d.code, "E0200");
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.labels.len(), 1);
        assert_eq!(d.helps.len(), 1);
    }

    #[test]
    fn render_mentions_code_and_message() {
        let sm = SourceMap::new("t.lyra", "foo\n");
        let d = Diagnostic::error("E0200", "unresolved name `foo`")
            .with_label(Span::new(0, 3), "not found");
        let out = render(&[d], &sm);
        assert!(out.contains("E0200"), "render output: {out}");
        assert!(out.contains("unresolved name"), "render output: {out}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib diag`
Expected: FAIL — `Diagnostic` not found.

- [ ] **Step 3: Write the implementation**

Prepend to `src/diag.rs`:
```rust
//! Structured diagnostics and human-readable rendering.

use crate::span::{SourceMap, Span};
use ariadne::{Color, Label as AriadneLabel, Report, ReportKind, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub labels: Vec<Label>,
    pub helps: Vec<String>,
}

impl Diagnostic {
    pub fn error(code: &str, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code: code.to_string(),
            message: message.into(),
            labels: Vec::new(),
            helps: Vec::new(),
        }
    }

    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Diagnostic {
        self.labels.push(Label { span, message: message.into() });
        self
    }

    pub fn with_help(mut self, message: impl Into<String>) -> Diagnostic {
        self.helps.push(message.into());
        self
    }
}

/// Render diagnostics to a string. Tests assert on structured fields, not on
/// this output; this is for humans. If the installed `ariadne` API differs
/// slightly, adapt these calls — the substring assertions in tests are stable.
pub fn render(diags: &[Diagnostic], src: &SourceMap) -> String {
    let mut buf = Vec::new();
    let id = src.name();
    for d in diags {
        let kind = match d.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warning => ReportKind::Warning,
        };
        let offset = d.labels.first().map(|l| l.span.start as usize).unwrap_or(0);
        let mut report = Report::build(kind, id, offset)
            .with_code(&d.code)
            .with_message(&d.message);
        for l in &d.labels {
            report = report.with_label(
                AriadneLabel::new((id, l.span.start as usize..l.span.end as usize))
                    .with_message(&l.message)
                    .with_color(Color::Red),
            );
        }
        for h in &d.helps {
            report = report.with_help(h);
        }
        let _ = report
            .finish()
            .write((id, Source::from(src.text())), &mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib diag`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/diag.rs
git commit -m "feat(diag): diagnostic model + ariadne rendering"
```

---

## Task 4: `lex` module

**Files:**
- Modify: `src/lex.rs`
- Test: inline `#[cfg(test)]` in `src/lex.rs`

**Interfaces:**
- Consumes: `span::Span`, `diag::Diagnostic`, `Session`.
- Produces:
  - `pub enum TokenKind { ... }` (see impl) with `Int(i64)`, `Float(f64)`, `Str(String)`, `Lower(String)`, `Upper(String)`, keyword variants, operator/punctuation variants.
  - `pub struct Token { pub kind: TokenKind, pub span: Span }`
  - `pub fn lex(session: &Session, text: &str) -> (Vec<Token>, Vec<Diagnostic>)`

- [ ] **Step 1: Write the failing test**

In `src/lex.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    fn kinds(text: &str) -> Vec<TokenKind> {
        let (toks, diags) = lex(&Session::new(), text);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        toks.into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lexes_keywords_idents_and_ops() {
        use TokenKind::*;
        assert_eq!(
            kinds("pub fn main"),
            vec![KwPub, KwFn, Lower("main".into())]
        );
        assert_eq!(kinds("1 + 2"), vec![Int(1), Plus, Int(2)]);
    }

    #[test]
    fn lexes_string_and_qualified_call_tokens() {
        use TokenKind::*;
        assert_eq!(
            kinds(r#"io.println("hi")"#),
            vec![
                Lower("io".into()), Dot, Lower("println".into()),
                LParen, Str("hi".into()), RParen
            ]
        );
    }

    #[test]
    fn function_keyword_does_not_eat_identifiers() {
        use TokenKind::*;
        assert_eq!(kinds("function"), vec![Lower("function".into())]);
    }

    #[test]
    fn unexpected_char_produces_diagnostic_not_panic() {
        let (_toks, diags) = lex(&Session::new(), "let x = §");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "E0001");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib lex`
Expected: FAIL — `lex`/`TokenKind` not found.

- [ ] **Step 3: Write the implementation**

Prepend to `src/lex.rs`:
```rust
//! Lexer: source text → spanned tokens.

use crate::diag::Diagnostic;
use crate::span::Span;
use crate::Session;
use logos::Logos;

#[derive(Logos, Clone, Debug, PartialEq)]
#[logos(skip r"[ \t\r\n\f]+")]
#[logos(skip r"//[^\n]*")]
#[logos(skip r"/\*([^*]|\*[^/])*\*/")]
pub enum TokenKind {
    #[regex(r"[0-9][0-9_]*", |lex| lex.slice().replace('_', "").parse().ok())]
    Int(i64),
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*", |lex| lex.slice().replace('_', "").parse().ok())]
    Float(f64),
    #[regex(r#""([^"\\]|\\.)*""#, |lex| unescape(lex.slice()))]
    Str(String),

    // Keywords (higher priority than Lower via exact-token match).
    #[token("pub")] KwPub,
    #[token("fn")] KwFn,
    #[token("let")] KwLet,
    #[token("import")] KwImport,
    #[token("if")] KwIf,
    #[token("else")] KwElse,
    #[token("type")] KwType,
    #[token("case")] KwCase,
    #[token("effect")] KwEffect,
    #[token("trait")] KwTrait,
    #[token("impl")] KwImpl,
    #[token("handle")] KwHandle,
    #[token("with")] KwWith,
    #[token("multi")] KwMulti,
    #[token("return")] KwReturn,
    #[token("const")] KwConst,
    #[token("True")] True,
    #[token("False")] False,
    #[token("Unit")] Unit,

    #[regex(r"[a-z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Lower(String),
    #[regex(r"[A-Z][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Upper(String),

    // Multi-char operators must precede their single-char prefixes.
    #[token("+.")] PlusDot,
    #[token("-.")] MinusDot,
    #[token("*.")] StarDot,
    #[token("/.")] SlashDot,
    #[token("==")] EqEq,
    #[token("!=")] NotEq,
    #[token("<=")] Le,
    #[token(">=")] Ge,
    #[token("->")] Arrow,
    #[token("=>")] FatArrow,
    #[token("<>")] Concat,
    #[token("|>")] Pipe,
    #[token("&&")] AmpAmp,
    #[token("||")] PipePipe,
    #[token("..")] DotDot,

    #[token("+")] Plus,
    #[token("-")] Minus,
    #[token("*")] Star,
    #[token("/")] Slash,
    #[token("%")] Percent,
    #[token("=")] Eq,
    #[token("<")] Lt,
    #[token(">")] Gt,
    #[token("!")] Bang,
    #[token("?")] Question,
    #[token(".")] Dot,
    #[token(",")] Comma,
    #[token(":")] Colon,
    #[token("(")] LParen,
    #[token(")")] RParen,
    #[token("{")] LBrace,
    #[token("}")] RBrace,
    #[token("[")] LBracket,
    #[token("]")] RBracket,
}

fn unescape(raw: &str) -> Option<String> {
    // raw includes surrounding quotes.
    let inner = &raw[1..raw.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                _ => return None,
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub fn lex(_session: &Session, text: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut tokens = Vec::new();
    let mut diags = Vec::new();
    let mut lexer = TokenKind::lexer(text);
    while let Some(result) = lexer.next() {
        let range = lexer.span();
        let span = Span::new(range.start as u32, range.end as u32);
        match result {
            Ok(kind) => tokens.push(Token { kind, span }),
            Err(()) => diags.push(
                Diagnostic::error("E0001", "unexpected character")
                    .with_label(span, "not a valid Lyra token"),
            ),
        }
    }
    (tokens, diags)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib lex`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/lex.rs
git commit -m "feat(lex): logos lexer with spanned tokens and E0001 recovery"
```

---

## Task 5: `ast` module

**Files:**
- Modify: `src/ast.rs`
- Test: inline `#[cfg(test)]` in `src/ast.rs`

**Interfaces:**
- Consumes: `span::Spanned`.
- Produces (all `#[derive(Clone, Debug, PartialEq)]`):
  - `pub struct Module { pub imports: Vec<Spanned<Import>>, pub decls: Vec<Spanned<Decl>> }`
  - `pub struct Import { pub path: Vec<String>, pub alias: Option<String> }`
  - `pub enum Decl { Fn(FnDecl) }`
  - `pub struct FnDecl { pub is_pub: bool, pub name: String, pub params: Vec<Spanned<Param>>, pub effect_row: Vec<String>, pub body: Spanned<Block> }`
  - `pub struct Param { pub name: String }`
  - `pub struct Block { pub stmts: Vec<Spanned<Stmt>>, pub tail: Option<Box<Spanned<Expr>>> }`
  - `pub enum Stmt { Let { name: String, value: Spanned<Expr> }, Expr(Spanned<Expr>) }`
  - `pub enum Expr { Int(i64), Float(f64), Str(String), Bool(bool), Unit, Var(String), Qualified { module: String, name: String }, Call { callee: Box<Spanned<Expr>>, args: Vec<Spanned<Expr>> }, Unary { op: UnOp, expr: Box<Spanned<Expr>> }, Binary { op: BinOp, lhs: Box<Spanned<Expr>>, rhs: Box<Spanned<Expr>> }, If { cond: Box<Spanned<Expr>>, then_block: Spanned<Block>, else_block: Spanned<Block> }, Block(Block) }`
  - `pub enum BinOp { Add, Sub, Mul, Div, Rem, AddF, SubF, MulF, DivF, Eq, Ne, Lt, Le, Gt, Ge, Concat, And, Or }`
  - `pub enum UnOp { Neg, Not }`
  - `pub fn pretty(m: &Module) -> String` (compact s-expression form, for snapshot tests).

> **Slice-1 note:** the parser will *parse* type annotations, return types, and the effect row, but the AST here only retains effect names (`effect_row: Vec<String>`) and drops type annotations — Slice 1 has no type checker. Types return in Slice 2.

- [ ] **Step 1: Write the failing test**

In `src/ast.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::{spanned, Span};

    fn sp<T>(node: T) -> Spanned<T> {
        spanned(node, Span::EMPTY)
    }

    #[test]
    fn pretty_prints_binary_expr() {
        let e = Expr::Binary {
            op: BinOp::Add,
            lhs: Box::new(sp(Expr::Int(1))),
            rhs: Box::new(sp(Expr::Int(2))),
        };
        let m = Module {
            imports: vec![],
            decls: vec![sp(Decl::Fn(FnDecl {
                is_pub: false,
                name: "f".into(),
                params: vec![],
                effect_row: vec![],
                body: sp(Block { stmts: vec![], tail: Some(Box::new(sp(e))) }),
            }))],
        };
        assert_eq!(pretty(&m), "(module (fn f () (block (+ 1 2))))");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib ast`
Expected: FAIL — types not found.

- [ ] **Step 3: Write the implementation**

Prepend to `src/ast.rs` the type definitions from the Interfaces block above (each with `#[derive(Clone, Debug, PartialEq)]`), plus:
```rust
//! Abstract syntax tree (Slice 1 subset). Every node is `Spanned`.

use crate::span::Spanned;

// ... (type definitions from Interfaces) ...

pub fn pretty(m: &Module) -> String {
    let mut s = String::from("(module");
    for d in &m.decls {
        s.push(' ');
        pretty_decl(&d.node, &mut s);
    }
    s.push(')');
    s
}

fn pretty_decl(d: &Decl, s: &mut String) {
    match d {
        Decl::Fn(f) => {
            s.push_str(&format!("(fn {} (", f.name));
            for (i, p) in f.params.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                s.push_str(&p.node.name);
            }
            s.push_str(") ");
            pretty_block(&f.body.node, s);
            s.push(')');
        }
    }
}

fn pretty_block(b: &Block, s: &mut String) {
    s.push_str("(block");
    for st in &b.stmts {
        s.push(' ');
        match &st.node {
            Stmt::Let { name, value } => {
                s.push_str(&format!("(let {} ", name));
                pretty_expr(&value.node, s);
                s.push(')');
            }
            Stmt::Expr(e) => pretty_expr(&e.node, s),
        }
    }
    if let Some(tail) = &b.tail {
        s.push(' ');
        pretty_expr(&tail.node, s);
    }
    s.push(')');
}

fn pretty_expr(e: &Expr, s: &mut String) {
    match e {
        Expr::Int(n) => s.push_str(&n.to_string()),
        Expr::Float(x) => s.push_str(&x.to_string()),
        Expr::Str(v) => s.push_str(&format!("{v:?}")),
        Expr::Bool(b) => s.push_str(if *b { "True" } else { "False" }),
        Expr::Unit => s.push_str("Unit"),
        Expr::Var(name) => s.push_str(name),
        Expr::Qualified { module, name } => s.push_str(&format!("{module}.{name}")),
        Expr::Call { callee, args } => {
            s.push_str("(call ");
            pretty_expr(&callee.node, s);
            for a in args {
                s.push(' ');
                pretty_expr(&a.node, s);
            }
            s.push(')');
        }
        Expr::Unary { op, expr } => {
            s.push_str(&format!("({} ", unop_sym(*op)));
            pretty_expr(&expr.node, s);
            s.push(')');
        }
        Expr::Binary { op, lhs, rhs } => {
            s.push_str(&format!("({} ", binop_sym(*op)));
            pretty_expr(&lhs.node, s);
            s.push(' ');
            pretty_expr(&rhs.node, s);
            s.push(')');
        }
        Expr::If { cond, then_block, else_block } => {
            s.push_str("(if ");
            pretty_expr(&cond.node, s);
            s.push(' ');
            pretty_block(&then_block.node, s);
            s.push(' ');
            pretty_block(&else_block.node, s);
            s.push(')');
        }
        Expr::Block(b) => pretty_block(b, s),
    }
}

fn binop_sym(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*", BinOp::Div => "/",
        BinOp::Rem => "%", BinOp::AddF => "+.", BinOp::SubF => "-.", BinOp::MulF => "*.",
        BinOp::DivF => "/.", BinOp::Eq => "==", BinOp::Ne => "!=", BinOp::Lt => "<",
        BinOp::Le => "<=", BinOp::Gt => ">", BinOp::Ge => ">=", BinOp::Concat => "<>",
        BinOp::And => "&&", BinOp::Or => "||",
    }
}

fn unop_sym(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib ast`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add src/ast.rs
git commit -m "feat(ast): Slice-1 AST node types and s-expression pretty printer"
```

---

## Task 6: `parse` — Pratt expression parser

**Files:**
- Modify: `src/parse.rs`
- Test: inline `#[cfg(test)]` in `src/parse.rs`

**Interfaces:**
- Consumes: `lex::{Token, TokenKind, lex}`, `ast::*`, `span::{Span, Spanned, spanned}`, `diag::Diagnostic`, `Session`.
- Produces (this task provides the internal parser plus one public entry used by the test):
  - `struct Parser<'a>` with a token cursor.
  - `pub fn parse_expr_str(session: &Session, text: &str) -> (Option<Spanned<ast::Expr>>, Vec<Diagnostic>)` — test-only convenience that lexes then parses a single expression.
  - Internal: `fn expr(&mut self, min_bp: u8) -> Option<Spanned<Expr>>` (Pratt), `fn atom(&mut self)`, precedence via `fn infix_bp(kind) -> Option<(u8,u8,BinOp)>`.

- [ ] **Step 1: Write the failing test**

In `src/parse.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::pretty_expr_public;
    use crate::Session;

    fn p(text: &str) -> String {
        let (e, diags) = parse_expr_str(&Session::new(), text);
        assert!(diags.is_empty(), "diags: {diags:?}");
        pretty_expr_public(&e.unwrap().node)
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(p("1 + 2 * 3"), "(+ 1 (* 2 3))");
        assert_eq!(p("1 - 2 - 3"), "(- (- 1 2) 3)");
        assert_eq!(p("(1 + 2) * 3"), "(* (+ 1 2) 3)");
    }

    #[test]
    fn calls_and_qualified() {
        assert_eq!(p(r#"io.println("hi")"#), r#"(call io.println "hi")"#);
        assert_eq!(p("f(1, 2)"), "(call f 1 2)");
    }
}
```

Add to `src/ast.rs` a public wrapper so the test can print a bare expr (append):
```rust
pub fn pretty_expr_public(e: &Expr) -> String {
    let mut s = String::new();
    pretty_expr(e, &mut s);
    s
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib parse`
Expected: FAIL — `parse_expr_str` not found.

- [ ] **Step 3: Write the implementation**

Prepend to `src/parse.rs`:
```rust
//! Recursive-descent (declarations) + Pratt (expressions) parser.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::lex::{lex, Token, TokenKind};
use crate::span::{spanned, Span, Spanned};
use crate::Session;

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    diags: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Parser<'a> {
        Parser { tokens, pos: 0, diags: Vec::new() }
    }

    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos).map(|t| &t.kind)
    }

    fn peek_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or(Span::EMPTY)
    }

    fn bump(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.peek() == Some(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.diags
            .push(Diagnostic::error("E0100", msg).with_label(span, "here"));
    }

    // Pratt expression parser. `min_bp` is the minimum binding power.
    fn expr(&mut self, min_bp: u8) -> Option<Spanned<Expr>> {
        let mut lhs = self.atom()?;
        loop {
            let Some(kind) = self.peek() else { break };
            // postfix call: `expr(...)`
            if *kind == TokenKind::LParen {
                lhs = self.finish_call(lhs)?;
                continue;
            }
            let Some((l_bp, r_bp, op)) = infix_bp(kind) else { break };
            if l_bp < min_bp {
                break;
            }
            self.bump(); // operator
            let rhs = self.expr(r_bp)?;
            let span = lhs.span.merge(rhs.span);
            lhs = spanned(
                Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) },
                span,
            );
        }
        Some(lhs)
    }

    fn finish_call(&mut self, callee: Spanned<Expr>) -> Option<Spanned<Expr>> {
        let start = callee.span;
        self.bump(); // '('
        let mut args = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                let arg = self.expr(0)?;
                args.push(arg);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        let end = self.peek_span();
        if !self.eat(&TokenKind::RParen) {
            self.error(end, "expected `)` to close call");
            return None;
        }
        Some(spanned(
            Expr::Call { callee: Box::new(callee), args },
            start.merge(end),
        ))
    }

    fn atom(&mut self) -> Option<Spanned<Expr>> {
        let span = self.peek_span();
        match self.peek()?.clone() {
            TokenKind::Int(n) => { self.bump(); Some(spanned(Expr::Int(n), span)) }
            TokenKind::Float(x) => { self.bump(); Some(spanned(Expr::Float(x), span)) }
            TokenKind::Str(s) => { self.bump(); Some(spanned(Expr::Str(s), span)) }
            TokenKind::True => { self.bump(); Some(spanned(Expr::Bool(true), span)) }
            TokenKind::False => { self.bump(); Some(spanned(Expr::Bool(false), span)) }
            TokenKind::Unit => { self.bump(); Some(spanned(Expr::Unit, span)) }
            TokenKind::Minus => {
                self.bump();
                let e = self.expr(100)?;
                let s = span.merge(e.span);
                Some(spanned(Expr::Unary { op: UnOp::Neg, expr: Box::new(e) }, s))
            }
            TokenKind::Bang => {
                self.bump();
                let e = self.expr(100)?;
                let s = span.merge(e.span);
                Some(spanned(Expr::Unary { op: UnOp::Not, expr: Box::new(e) }, s))
            }
            TokenKind::LParen => {
                self.bump();
                let e = self.expr(0)?;
                let end = self.peek_span();
                if !self.eat(&TokenKind::RParen) {
                    self.error(end, "expected `)`");
                    return None;
                }
                Some(e)
            }
            TokenKind::Lower(name) => {
                self.bump();
                if self.eat(&TokenKind::Dot) {
                    let member_span = self.peek_span();
                    match self.peek()?.clone() {
                        TokenKind::Lower(member) => {
                            self.bump();
                            Some(spanned(
                                Expr::Qualified { module: name, name: member },
                                span.merge(member_span),
                            ))
                        }
                        _ => {
                            self.error(member_span, "expected identifier after `.`");
                            None
                        }
                    }
                } else {
                    Some(spanned(Expr::Var(name), span))
                }
            }
            _ => {
                self.error(span, "expected an expression");
                None
            }
        }
    }
}

/// Binding powers: (left_bp, right_bp, op). Higher binds tighter.
fn infix_bp(kind: &TokenKind) -> Option<(u8, u8, BinOp)> {
    use TokenKind as T;
    let (l, r, op) = match kind {
        T::PipePipe => (10, 11, BinOp::Or),
        T::AmpAmp => (20, 21, BinOp::And),
        T::EqEq => (30, 31, BinOp::Eq),
        T::NotEq => (30, 31, BinOp::Ne),
        T::Lt => (30, 31, BinOp::Lt),
        T::Le => (30, 31, BinOp::Le),
        T::Gt => (30, 31, BinOp::Gt),
        T::Ge => (30, 31, BinOp::Ge),
        T::Concat => (40, 41, BinOp::Concat),
        T::Plus => (50, 51, BinOp::Add),
        T::Minus => (50, 51, BinOp::Sub),
        T::PlusDot => (50, 51, BinOp::AddF),
        T::MinusDot => (50, 51, BinOp::SubF),
        T::Star => (60, 61, BinOp::Mul),
        T::Slash => (60, 61, BinOp::Div),
        T::Percent => (60, 61, BinOp::Rem),
        T::StarDot => (60, 61, BinOp::MulF),
        T::SlashDot => (60, 61, BinOp::DivF),
        _ => return None,
    };
    Some((l, r, op))
}

/// Test/convenience entry: parse a single expression from source text.
pub fn parse_expr_str(
    session: &Session,
    text: &str,
) -> (Option<Spanned<Expr>>, Vec<Diagnostic>) {
    let (tokens, mut diags) = lex(session, text);
    let mut p = Parser::new(&tokens);
    let e = p.expr(0);
    diags.append(&mut p.diags);
    (e, diags)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib parse`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/parse.rs src/ast.rs
git commit -m "feat(parse): Pratt expression parser (precedence, calls, qualified)"
```

---

## Task 7: `parse` — declarations, blocks, and `if`

**Files:**
- Modify: `src/parse.rs`
- Test: inline `#[cfg(test)]` in `src/parse.rs`

**Interfaces:**
- Produces: `pub fn parse_module(session: &Session, text: &str) -> (Module, Vec<Diagnostic>)`.
- Internal: `fn module(&mut self)`, `fn import(&mut self)`, `fn fn_decl(&mut self, is_pub)`, `fn block(&mut self)`, `fn stmt(&mut self)`, `fn if_expr(&mut self)`, `fn skip_type_annotation(&mut self)`, `fn effect_row(&mut self) -> Vec<String>`.

- [ ] **Step 1: Write the failing test**

Add to `src/parse.rs` tests module:
```rust
    #[test]
    fn parses_hello_world_module() {
        let src = "import lyra/io\n\npub fn main() / {IO} {\n  io.println(\"Hello, Lyra!\")\n}\n";
        let (m, diags) = parse_module(&Session::new(), src);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(
            crate::ast::pretty(&m),
            r#"(module (fn main () (block (call io.println "Hello, Lyra!"))))"#
        );
        // effect row parsed but only names retained
        if let Decl::Fn(f) = &m.decls[0].node {
            assert_eq!(f.effect_row, vec!["IO".to_string()]);
            assert!(f.is_pub);
        } else {
            panic!("expected fn");
        }
    }

    #[test]
    fn parses_let_and_if() {
        let src = "fn f() {\n  let x = 1\n  if x { 2 } else { 3 }\n}\n";
        let (m, diags) = parse_module(&Session::new(), src);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(
            crate::ast::pretty(&m),
            "(module (fn f () (block (let x 1) (if x (block 2) (block 3)))))"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib parse::tests::parses`
Expected: FAIL — `parse_module` not found.

- [ ] **Step 3: Write the implementation**

Add these methods to `impl<'a> Parser<'a>` and add `if_expr` handling to `atom` (insert a `TokenKind::KwIf => self.if_expr()` arm and a `TokenKind::LBrace => self.block_expr()` arm at the top of `atom`'s match):

```rust
    fn module(&mut self) -> Module {
        let mut imports = Vec::new();
        let mut decls = Vec::new();
        while self.peek().is_some() {
            match self.peek() {
                Some(TokenKind::KwImport) => {
                    if let Some(i) = self.import() {
                        imports.push(i);
                    } else {
                        self.recover_to_decl();
                    }
                }
                Some(TokenKind::KwPub) | Some(TokenKind::KwFn) => {
                    let is_pub = self.eat(&TokenKind::KwPub);
                    if let Some(d) = self.fn_decl(is_pub) {
                        decls.push(d);
                    } else {
                        self.recover_to_decl();
                    }
                }
                _ => {
                    let span = self.peek_span();
                    self.error(span, "expected `import`, `fn`, or `pub fn`");
                    self.recover_to_decl();
                }
            }
        }
        Module { imports, decls }
    }

    fn import(&mut self) -> Option<Spanned<Import>> {
        let start = self.peek_span();
        self.bump(); // import
        let mut path = Vec::new();
        loop {
            let seg_span = self.peek_span();
            match self.peek()?.clone() {
                TokenKind::Lower(seg) => { self.bump(); path.push(seg); }
                _ => { self.error(seg_span, "expected module path segment"); return None; }
            }
            if !self.eat(&TokenKind::Slash) {
                break;
            }
        }
        let end = self.peek_span();
        Some(spanned(Import { path, alias: None }, start.merge(end)))
    }

    fn fn_decl(&mut self, is_pub: bool) -> Option<Spanned<Decl>> {
        let start = self.peek_span();
        if !self.eat(&TokenKind::KwFn) {
            self.error(start, "expected `fn`");
            return None;
        }
        let name = match self.peek()?.clone() {
            TokenKind::Lower(n) => { self.bump(); n }
            _ => { self.error(self.peek_span(), "expected function name"); return None; }
        };
        if !self.eat(&TokenKind::LParen) {
            self.error(self.peek_span(), "expected `(`");
            return None;
        }
        let mut params = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                let pspan = self.peek_span();
                match self.peek()?.clone() {
                    TokenKind::Lower(pn) => {
                        self.bump();
                        if self.eat(&TokenKind::Colon) {
                            self.skip_type_annotation();
                        }
                        params.push(spanned(Param { name: pn }, pspan));
                    }
                    _ => { self.error(pspan, "expected parameter name"); return None; }
                }
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        if !self.eat(&TokenKind::RParen) {
            self.error(self.peek_span(), "expected `)`");
            return None;
        }
        // optional effect row: `/ { Name, Name }`
        let effect_row = if self.eat(&TokenKind::Slash) {
            self.effect_row()
        } else {
            Vec::new()
        };
        // optional return type: `-> Type`
        if self.eat(&TokenKind::Arrow) {
            self.skip_type_annotation();
        }
        let body = self.block()?;
        let end = body.span;
        Some(spanned(
            Decl::Fn(FnDecl { is_pub, name, params, effect_row, body }),
            start.merge(end),
        ))
    }

    /// Parse `{ Name, Name }` after `/`, keeping only effect head names.
    fn effect_row(&mut self) -> Vec<String> {
        let mut names = Vec::new();
        if !self.eat(&TokenKind::LBrace) {
            self.error(self.peek_span(), "expected `{` for effect row");
            return names;
        }
        if self.peek() != Some(&TokenKind::RBrace) {
            loop {
                match self.peek().cloned() {
                    Some(TokenKind::Upper(n)) => {
                        self.bump();
                        // skip optional effect type args: `(...)`
                        if self.eat(&TokenKind::LParen) {
                            self.skip_balanced_parens();
                        }
                        names.push(n);
                    }
                    _ => { self.error(self.peek_span(), "expected effect name"); break; }
                }
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.eat(&TokenKind::RBrace);
        names
    }

    /// Slice 1 has no type checker: consume a type annotation and discard it.
    fn skip_type_annotation(&mut self) {
        // A type is: Upper [ ( ... ) ] | Lower | ( ... ) | fn ( ... ) -> Type
        match self.peek().cloned() {
            Some(TokenKind::Upper(_)) | Some(TokenKind::Lower(_)) | Some(TokenKind::Unit) => {
                self.bump();
                if self.eat(&TokenKind::LParen) {
                    self.skip_balanced_parens();
                }
            }
            Some(TokenKind::LParen) => {
                self.bump();
                self.skip_balanced_parens();
            }
            _ => { /* nothing to skip */ }
        }
    }

    fn skip_balanced_parens(&mut self) {
        let mut depth = 1;
        while depth > 0 {
            match self.bump().map(|t| t.kind.clone()) {
                Some(TokenKind::LParen) => depth += 1,
                Some(TokenKind::RParen) => depth -= 1,
                None => break,
                _ => {}
            }
        }
    }

    fn block(&mut self) -> Option<Spanned<Block>> {
        let start = self.peek_span();
        if !self.eat(&TokenKind::LBrace) {
            self.error(start, "expected `{`");
            return None;
        }
        let mut stmts = Vec::new();
        let mut tail = None;
        while self.peek().is_some() && self.peek() != Some(&TokenKind::RBrace) {
            if self.peek() == Some(&TokenKind::KwLet) {
                if let Some(s) = self.let_stmt() {
                    stmts.push(s);
                } else {
                    break;
                }
            } else {
                let e = self.expr(0)?;
                // if a `}` follows, this expression is the block's tail value
                if self.peek() == Some(&TokenKind::RBrace) {
                    tail = Some(Box::new(e));
                } else {
                    let span = e.span;
                    stmts.push(spanned(Stmt::Expr(e), span));
                }
            }
        }
        let end = self.peek_span();
        self.eat(&TokenKind::RBrace);
        Some(spanned(Block { stmts, tail }, start.merge(end)))
    }

    fn let_stmt(&mut self) -> Option<Spanned<Stmt>> {
        let start = self.peek_span();
        self.bump(); // let
        let name = match self.peek()?.clone() {
            TokenKind::Lower(n) => { self.bump(); n }
            _ => { self.error(self.peek_span(), "expected binding name"); return None; }
        };
        if self.eat(&TokenKind::Colon) {
            self.skip_type_annotation();
        }
        if !self.eat(&TokenKind::Eq) {
            self.error(self.peek_span(), "expected `=` in let binding");
            return None;
        }
        let value = self.expr(0)?;
        let span = start.merge(value.span);
        Some(spanned(Stmt::Let { name, value }, span))
    }

    fn if_expr(&mut self) -> Option<Spanned<Expr>> {
        let start = self.peek_span();
        self.bump(); // if
        let cond = self.expr(0)?;
        let then_block = self.block()?;
        if !self.eat(&TokenKind::KwElse) {
            self.error(self.peek_span(), "expected `else` (if is an expression)");
            return None;
        }
        let else_block = self.block()?;
        let span = start.merge(else_block.span);
        Some(spanned(
            Expr::If {
                cond: Box::new(cond),
                then_block,
                else_block,
            },
            span,
        ))
    }

    fn block_expr(&mut self) -> Option<Spanned<Expr>> {
        let b = self.block()?;
        Some(spanned(Expr::Block(b.node), b.span))
    }

    fn recover_to_decl(&mut self) {
        // Synchronize: skip tokens until a declaration keyword or EOF.
        while let Some(k) = self.peek() {
            if matches!(k, TokenKind::KwFn | TokenKind::KwPub | TokenKind::KwImport) {
                return;
            }
            self.bump();
        }
    }
}

pub fn parse_module(session: &Session, text: &str) -> (Module, Vec<Diagnostic>) {
    let (tokens, mut diags) = lex(session, text);
    let mut p = Parser::new(&tokens);
    let m = p.module();
    diags.append(&mut p.diags);
    (m, diags)
}
```

In `atom`, add these two arms at the **start** of the `match self.peek()?.clone()`:
```rust
            TokenKind::KwIf => self.if_expr(),
            TokenKind::LBrace => self.block_expr(),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib parse`
Expected: PASS (all parse tests).

- [ ] **Step 5: Commit**

```bash
git add src/parse.rs
git commit -m "feat(parse): modules, fn decls, effect-row/type parsing, blocks, let, if"
```

---

## Task 8: `parse` — error recovery reports multiple errors

**Files:**
- Modify: `src/parse.rs`
- Test: inline `#[cfg(test)]` in `src/parse.rs`

**Interfaces:**
- No new public functions; verifies `recover_to_decl` lets one file report multiple diagnostics and still parse later declarations.

- [ ] **Step 1: Write the failing test**

Add to `src/parse.rs` tests:
```rust
    #[test]
    fn recovers_and_reports_multiple_errors() {
        // first fn is broken (missing name), second fn is fine and must still parse
        let src = "fn () { 1 }\npub fn ok() { 2 }\n";
        let (m, diags) = parse_module(&Session::new(), src);
        assert!(!diags.is_empty(), "expected at least one diagnostic");
        assert!(diags.iter().all(|d| d.code == "E0100"));
        // recovery reached the good declaration
        let names: Vec<_> = m.decls.iter().filter_map(|d| match &d.node {
            Decl::Fn(f) => Some(f.name.clone()),
        }).collect();
        assert!(names.contains(&"ok".to_string()), "names: {names:?}");
    }
```

- [ ] **Step 2: Run test to verify it fails or passes**

Run: `cargo test --lib parse::tests::recovers`
Expected: PASS if Task 7's `recover_to_decl` is correct. If it FAILS (e.g., recovery consumes the good decl), fix `recover_to_decl` so it stops *before* consuming a decl keyword (it already checks `peek` before `bump`), and ensure `module()` calls `recover_to_decl` after a failed `fn_decl`.

- [ ] **Step 3: Fix if needed**

If the good declaration is missing, the bug is that after a failed `fn_decl` the parser is positioned on a token that `recover_to_decl` immediately returns on, causing an infinite non-advance. Guard `module()`'s loop: if a branch made no progress, `bump()` once before recovering. Add a progress check:
```rust
        while self.peek().is_some() {
            let before = self.pos;
            // ... existing match ...
            if self.pos == before {
                self.bump(); // ensure forward progress
            }
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib parse`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/parse.rs
git commit -m "feat(parse): error recovery reports multiple errors and resyncs"
```

---

## Task 9: `resolve` module (name checking)

**Files:**
- Modify: `src/resolve.rs`
- Test: inline `#[cfg(test)]` in `src/resolve.rs`

**Interfaces:**
- Consumes: `ast::*`, `diag::Diagnostic`, `span::Span`, `Session`.
- Produces:
  - `pub fn check(session: &Session, module: &Module) -> Vec<Diagnostic>` — emits `E0200` for unresolved variable references and `E0201` for calling an unknown builtin `module.member`.
  - `pub fn builtins() -> &'static [&'static str]` — the qualified builtin names known in Slice 1: `["io.println"]`.

- [ ] **Step 1: Write the failing test**

In `src/resolve.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_module;
    use crate::Session;

    fn diags(src: &str) -> Vec<crate::diag::Diagnostic> {
        let (m, pdiags) = parse_module(&Session::new(), src);
        assert!(pdiags.is_empty(), "parse diags: {pdiags:?}");
        check(&Session::new(), &m)
    }

    #[test]
    fn undefined_variable_is_e0200() {
        let d = diags("fn f() { x }\n");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0200");
    }

    #[test]
    fn params_lets_and_fns_resolve() {
        let d = diags("fn g() { 1 }\nfn f(a) { let b = a\n g() }\n");
        assert!(d.is_empty(), "unexpected: {d:?}");
    }

    #[test]
    fn unknown_builtin_is_e0201() {
        let d = diags(r#"fn f() { io.nope("x") }"#);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0201");
    }

    #[test]
    fn known_builtin_resolves() {
        let d = diags(r#"fn f() { io.println("x") }"#);
        assert!(d.is_empty(), "unexpected: {d:?}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib resolve`
Expected: FAIL — `check` not found.

- [ ] **Step 3: Write the implementation**

Prepend to `src/resolve.rs`:
```rust
//! Name resolution: checks that every reference resolves to a param, a
//! local `let`, a top-level function, or a known builtin.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::Span;
use crate::Session;
use std::collections::HashSet;

pub fn builtins() -> &'static [&'static str] {
    &["io.println"]
}

pub fn check(_session: &Session, module: &Module) -> Vec<Diagnostic> {
    let mut fn_names: HashSet<String> = HashSet::new();
    for d in &module.decls {
        match &d.node {
            Decl::Fn(f) => {
                fn_names.insert(f.name.clone());
            }
        }
    }
    let mut cx = Cx { fns: &fn_names, diags: Vec::new() };
    for d in &module.decls {
        match &d.node {
            Decl::Fn(f) => {
                let mut scope: Vec<HashSet<String>> = vec![HashSet::new()];
                for p in &f.params {
                    scope.last_mut().unwrap().insert(p.node.name.clone());
                }
                cx.check_block(&f.body.node, &mut scope);
            }
        }
    }
    cx.diags
}

struct Cx<'a> {
    fns: &'a HashSet<String>,
    diags: Vec<Diagnostic>,
}

impl<'a> Cx<'a> {
    fn resolves_var(&self, name: &str, scope: &[HashSet<String>]) -> bool {
        scope.iter().rev().any(|s| s.contains(name)) || self.fns.contains(name)
    }

    fn check_block(&mut self, b: &Block, scope: &mut Vec<HashSet<String>>) {
        scope.push(HashSet::new());
        for st in &b.stmts {
            match &st.node {
                Stmt::Let { name, value } => {
                    self.check_expr(&value.node, value.span, scope);
                    scope.last_mut().unwrap().insert(name.clone());
                }
                Stmt::Expr(e) => self.check_expr(&e.node, e.span, scope),
            }
        }
        if let Some(tail) = &b.tail {
            self.check_expr(&tail.node, tail.span, scope);
        }
        scope.pop();
    }

    fn check_expr(&mut self, e: &Expr, span: Span, scope: &mut Vec<HashSet<String>>) {
        match e {
            Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Unit => {}
            Expr::Var(name) => {
                if !self.resolves_var(name, scope) {
                    self.diags.push(
                        Diagnostic::error("E0200", format!("unresolved name `{name}`"))
                            .with_label(span, "not found in this scope"),
                    );
                }
            }
            Expr::Qualified { module, name } => {
                let full = format!("{module}.{name}");
                if !builtins().contains(&full.as_str()) {
                    self.diags.push(
                        Diagnostic::error("E0201", format!("unknown builtin `{full}`"))
                            .with_label(span, "no such function"),
                    );
                }
            }
            Expr::Call { callee, args } => {
                self.check_expr(&callee.node, callee.span, scope);
                for a in args {
                    self.check_expr(&a.node, a.span, scope);
                }
            }
            Expr::Unary { expr, .. } => self.check_expr(&expr.node, expr.span, scope),
            Expr::Binary { lhs, rhs, .. } => {
                self.check_expr(&lhs.node, lhs.span, scope);
                self.check_expr(&rhs.node, rhs.span, scope);
            }
            Expr::If { cond, then_block, else_block } => {
                self.check_expr(&cond.node, cond.span, scope);
                self.check_block(&then_block.node, scope);
                self.check_block(&else_block.node, scope);
            }
            Expr::Block(b) => self.check_block(b, scope),
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib resolve`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/resolve.rs
git commit -m "feat(resolve): name checking with E0200/E0201 and builtin table"
```

---

## Task 10: `eval` — values, environment, expression evaluation

**Files:**
- Modify: `src/eval.rs`
- Test: inline `#[cfg(test)]` in `src/eval.rs`

**Interfaces:**
- Consumes: `ast::*`, `diag::Diagnostic`, `span::Span`.
- Produces:
  - `pub enum Value { Int(i64), Float(f64), Str(String), Bool(bool), Unit }` (closures/functions added in Task 11).
  - `pub struct RuntimeError { pub diag: Diagnostic }`
  - `pub struct Interp { output: String, depth: usize, max_depth: usize, ... }` with `Interp::new()`, `pub fn output(&self) -> &str`, `pub fn max_depth(&self) -> usize`.
  - `fn eval_expr(&mut self, e: &Expr, span: Span, env: &Env) -> Result<Value, RuntimeError>`.
  - `type Env` = a scope chain of `HashMap<String, Value>`.

- [ ] **Step 1: Write the failing test**

In `src/eval.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_expr_str;
    use crate::Session;

    fn eval_str(text: &str) -> Value {
        let (e, diags) = parse_expr_str(&Session::new(), text);
        assert!(diags.is_empty(), "parse diags: {diags:?}");
        let e = e.unwrap();
        let mut interp = Interp::new();
        let env = Env::new();
        interp.eval_expr(&e.node, e.span, &env).unwrap()
    }

    #[test]
    fn arithmetic() {
        assert_eq!(eval_str("1 + 2 * 3"), Value::Int(7));
        assert_eq!(eval_str("(1 + 2) * 3"), Value::Int(9));
        assert_eq!(eval_str("10 - 3 - 2"), Value::Int(5));
    }

    #[test]
    fn float_arithmetic_and_concat() {
        assert_eq!(eval_str("1.5 +. 2.0"), Value::Float(3.5));
        assert_eq!(eval_str(r#""a" <> "b""#), Value::Str("ab".into()));
    }

    #[test]
    fn comparison_and_if() {
        assert_eq!(eval_str("if 1 < 2 { 10 } else { 20 }"), Value::Int(10));
        assert_eq!(eval_str("if 2 < 1 { 10 } else { 20 }"), Value::Int(20));
    }

    #[test]
    fn division_by_zero_is_runtime_error_not_panic() {
        let (e, _) = parse_expr_str(&Session::new(), "1 / 0");
        let e = e.unwrap();
        let mut interp = Interp::new();
        let env = Env::new();
        let err = interp.eval_expr(&e.node, e.span, &env).unwrap_err();
        assert_eq!(err.diag.code, "E0300");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib eval`
Expected: FAIL — `Interp`, `Value`, `Env` not found.

- [ ] **Step 3: Write the implementation**

Prepend to `src/eval.rs`:
```rust
//! Slice-1 tree-walking interpreter.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::Span;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Unit,
}

#[derive(Debug)]
pub struct RuntimeError {
    pub diag: Diagnostic,
}

fn rt(span: Span, msg: impl Into<String>) -> RuntimeError {
    RuntimeError {
        diag: Diagnostic::error("E0300", msg).with_label(span, "during evaluation"),
    }
}

/// A scope chain of variable bindings.
#[derive(Clone, Debug, Default)]
pub struct Env {
    scopes: Vec<HashMap<String, Value>>,
}

impl Env {
    pub fn new() -> Env {
        Env { scopes: vec![HashMap::new()] }
    }

    fn child(&self) -> Env {
        let mut e = self.clone();
        e.scopes.push(HashMap::new());
        e
    }

    fn define(&mut self, name: &str, v: Value) {
        self.scopes.last_mut().unwrap().insert(name.to_string(), v);
    }

    fn lookup(&self, name: &str) -> Option<Value> {
        self.scopes.iter().rev().find_map(|s| s.get(name).cloned())
    }
}

pub struct Interp {
    output: String,
    depth: usize,
    max_depth: usize,
}

impl Interp {
    pub fn new() -> Interp {
        Interp { output: String::new(), depth: 0, max_depth: 0 }
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    pub fn eval_expr(
        &mut self,
        e: &Expr,
        span: Span,
        env: &Env,
    ) -> Result<Value, RuntimeError> {
        self.depth += 1;
        self.max_depth = self.max_depth.max(self.depth);
        let result = self.eval_inner(e, span, env);
        self.depth -= 1;
        result
    }

    fn eval_inner(
        &mut self,
        e: &Expr,
        span: Span,
        env: &Env,
    ) -> Result<Value, RuntimeError> {
        match e {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(x) => Ok(Value::Float(*x)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Unit => Ok(Value::Unit),
            Expr::Var(name) => env
                .lookup(name)
                .ok_or_else(|| rt(span, format!("unbound variable `{name}`"))),
            Expr::Unary { op, expr } => {
                let v = self.eval_expr(&expr.node, expr.span, env)?;
                match (op, v) {
                    (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(-n)),
                    (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
                    (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    _ => Err(rt(span, "type error in unary operator")),
                }
            }
            Expr::Binary { op, lhs, rhs } => {
                let l = self.eval_expr(&lhs.node, lhs.span, env)?;
                let r = self.eval_expr(&rhs.node, rhs.span, env)?;
                eval_binop(*op, l, r, span)
            }
            Expr::If { cond, then_block, else_block } => {
                match self.eval_expr(&cond.node, cond.span, env)? {
                    Value::Bool(true) => self.eval_block(&then_block.node, env),
                    Value::Bool(false) => self.eval_block(&else_block.node, env),
                    _ => Err(rt(cond.span, "if condition must be a Bool")),
                }
            }
            Expr::Block(b) => self.eval_block(b, env),
            // Calls handled in Task 11 (needs function values).
            Expr::Call { .. } | Expr::Qualified { .. } => {
                Err(rt(span, "calls not yet supported (Task 11)"))
            }
        }
    }

    fn eval_block(&mut self, b: &Block, env: &Env) -> Result<Value, RuntimeError> {
        let mut local = env.child();
        for st in &b.stmts {
            match &st.node {
                Stmt::Let { name, value } => {
                    let v = self.eval_expr(&value.node, value.span, &local)?;
                    local.define(name, v);
                }
                Stmt::Expr(e) => {
                    self.eval_expr(&e.node, e.span, &local)?;
                }
            }
        }
        match &b.tail {
            Some(tail) => self.eval_expr(&tail.node, tail.span, &local),
            None => Ok(Value::Unit),
        }
    }
}

fn eval_binop(op: BinOp, l: Value, r: Value, span: Span) -> Result<Value, RuntimeError> {
    use BinOp::*;
    use Value::*;
    match (op, l, r) {
        (Add, Int(a), Int(b)) => Ok(Int(a + b)),
        (Sub, Int(a), Int(b)) => Ok(Int(a - b)),
        (Mul, Int(a), Int(b)) => Ok(Int(a * b)),
        (Div, Int(_), Int(0)) => Err(rt(span, "division by zero")),
        (Div, Int(a), Int(b)) => Ok(Int(a / b)),
        (Rem, Int(_), Int(0)) => Err(rt(span, "remainder by zero")),
        (Rem, Int(a), Int(b)) => Ok(Int(a % b)),
        (AddF, Float(a), Float(b)) => Ok(Float(a + b)),
        (SubF, Float(a), Float(b)) => Ok(Float(a - b)),
        (MulF, Float(a), Float(b)) => Ok(Float(a * b)),
        (DivF, Float(a), Float(b)) => Ok(Float(a / b)),
        (Concat, Str(a), Str(b)) => Ok(Str(a + &b)),
        (Eq, a, b) => Ok(Bool(a == b)),
        (Ne, a, b) => Ok(Bool(a != b)),
        (Lt, Int(a), Int(b)) => Ok(Bool(a < b)),
        (Le, Int(a), Int(b)) => Ok(Bool(a <= b)),
        (Gt, Int(a), Int(b)) => Ok(Bool(a > b)),
        (Ge, Int(a), Int(b)) => Ok(Bool(a >= b)),
        (And, Bool(a), Bool(b)) => Ok(Bool(a && b)),
        (Or, Bool(a), Bool(b)) => Ok(Bool(a || b)),
        _ => Err(rt(span, "type error in binary operator")),
    }
}

impl Default for Interp {
    fn default() -> Self {
        Interp::new()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib eval`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/eval.rs
git commit -m "feat(eval): values, env, arithmetic/if/block evaluation, E0300 runtime errors"
```

---

## Task 11: `eval` — functions, calls, and `io.println`

**Files:**
- Modify: `src/eval.rs`
- Test: inline `#[cfg(test)]` in `src/eval.rs`

**Interfaces:**
- Produces: `pub fn run_module(module: &Module) -> Result<Interp, RuntimeError>` — installs top-level functions, calls `main` (0-arg), returns the `Interp` (whose `.output()` holds accumulated `io.println` text). Extends the `Call`/`Qualified` handling in `eval_inner`.

- [ ] **Step 1: Write the failing test**

Add to `src/eval.rs` tests:
```rust
    use crate::parse::parse_module;

    fn run(src: &str) -> String {
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "parse diags: {d:?}");
        run_module(&m).unwrap().output().to_string()
    }

    #[test]
    fn hello_world_prints() {
        let src = "pub fn main() / {IO} {\n  io.println(\"Hello, Lyra!\")\n}\n";
        assert_eq!(run(src), "Hello, Lyra!\n");
    }

    #[test]
    fn user_functions_and_calls() {
        let src = "fn double(x) { x + x }\npub fn main() { io.println(int_show(double(21))) }\n";
        // int_show is not a builtin in Slice 1; use a numeric echo via a helper fn instead:
        let src = "fn double(x) { x + x }\nfn add(a, b) { a + b }\npub fn main() { let _ = add(double(20), 2)\n io.println(\"ok\") }\n";
        assert_eq!(run(src), "ok\n");
    }
```

> Note: `int.show` is a stdlib function that does not exist until later slices; Slice 1 tests avoid printing integers directly and print string literals to observe control flow.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib eval::tests::hello_world_prints`
Expected: FAIL — `run_module` not found; `Call` returns the Task-10 placeholder error.

- [ ] **Step 3: Write the implementation**

Add a function-value variant and call support. First extend `Value`:
```rust
// Add to `enum Value`:
    Func(std::rc::Rc<FnDecl>),
```
Add `use std::rc::Rc;` at the top of `eval.rs`. Because `Value` now contains `FnDecl` (which derives `PartialEq`), this still compiles.

Replace the `Call`/`Qualified` arm in `eval_inner` with:
```rust
            Expr::Qualified { module, name } => {
                // Builtins are only meaningful when called; a bare reference is an error.
                Err(rt(span, format!("`{module}.{name}` must be called")))
            }
            Expr::Call { callee, args } => {
                // Builtin call: io.println(...)
                if let Expr::Qualified { module, name } = &callee.node {
                    let full = format!("{module}.{name}");
                    let mut vals = Vec::new();
                    for a in args {
                        vals.push(self.eval_expr(&a.node, a.span, env)?);
                    }
                    return self.call_builtin(&full, vals, span);
                }
                // User function call.
                let callee_val = self.eval_expr(&callee.node, callee.span, env)?;
                let Value::Func(func) = callee_val else {
                    return Err(rt(callee.span, "value is not callable"));
                };
                if func.params.len() != args.len() {
                    return Err(rt(
                        span,
                        format!(
                            "`{}` expects {} argument(s), got {}",
                            func.name,
                            func.params.len(),
                            args.len()
                        ),
                    ));
                }
                let mut call_env = self.globals_env.child();
                for (p, a) in func.params.iter().zip(args) {
                    let v = self.eval_expr(&a.node, a.span, env)?;
                    call_env.define(&p.node.name, v);
                }
                self.eval_block(&func.body.node, &call_env)
            }
```

Add a `globals_env` field to `Interp` and a builtin dispatcher + `run_module`:
```rust
// In `struct Interp`, add:
    globals_env: Env,

// In `Interp::new`, initialize:
        Interp { output: String::new(), depth: 0, max_depth: 0, globals_env: Env::new() }

impl Interp {
    fn call_builtin(
        &mut self,
        full: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match full {
            "io.println" => {
                let [Value::Str(s)] = &args[..] else {
                    return Err(rt(span, "io.println expects a single String"));
                };
                self.output.push_str(s);
                self.output.push('\n');
                Ok(Value::Unit)
            }
            _ => Err(rt(span, format!("unknown builtin `{full}`"))),
        }
    }
}

pub fn run_module(module: &Module) -> Result<Interp, RuntimeError> {
    let mut interp = Interp::new();
    // Install top-level functions as values in the globals env.
    for d in &module.decls {
        match &d.node {
            Decl::Fn(f) => {
                interp
                    .globals_env
                    .define(&f.name, Value::Func(Rc::new(f.clone())));
            }
        }
    }
    // Find and call `main`.
    let main = module.decls.iter().find_map(|d| match &d.node {
        Decl::Fn(f) if f.name == "main" => Some(f.clone()),
        _ => None,
    });
    let Some(main) = main else {
        return Err(rt(Span::EMPTY, "no `main` function found"));
    };
    let env = interp.globals_env.clone();
    interp.eval_block(&main.body.node, &env)?;
    Ok(interp)
}
```

> When variables reference top-level functions (e.g. `double` in a call), lookups resolve through `globals_env` because call environments are children of `globals_env`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib eval`
Expected: PASS (all eval tests).

- [ ] **Step 5: Commit**

```bash
git add src/eval.rs
git commit -m "feat(eval): user functions, calls, io.println builtin, run_module entry"
```

---

## Task 12: pipeline + CLI + example files

**Files:**
- Modify: `src/lib.rs`, `src/main.rs`
- Create: `examples/01_hello.lyra`, `examples/02_arith.lyra`
- Test: inline `#[cfg(test)]` in `src/lib.rs`

**Interfaces:**
- Produces:
  - `pub fn run_source(name: &str, text: &str) -> Result<String, String>` — full pipeline (lex→parse→resolve→run); on success returns program output; on failure returns rendered diagnostics.
  - `pub fn check_source(name: &str, text: &str) -> Result<(), String>` — lex+parse+resolve only; `Ok(())` if clean, else rendered diagnostics.

- [ ] **Step 1: Write the failing test**

Add to `src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_source_executes_hello_world() {
        let out = run_source(
            "h.lyra",
            "pub fn main() / {IO} {\n  io.println(\"Hello, Lyra!\")\n}\n",
        )
        .unwrap();
        assert_eq!(out, "Hello, Lyra!\n");
    }

    #[test]
    fn run_source_reports_diagnostics() {
        let err = run_source("b.lyra", "fn main() { x }\n").unwrap_err();
        assert!(err.contains("E0200"), "err: {err}");
    }

    #[test]
    fn check_source_is_ok_for_valid_program() {
        assert!(check_source("h.lyra", "fn main() { io.println(\"x\") }\n").is_ok());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib tests::run_source`
Expected: FAIL — `run_source` not found.

- [ ] **Step 3: Write the pipeline and CLI**

Add to `src/lib.rs`:
```rust
use crate::diag::{render, Diagnostic};
use crate::span::SourceMap;

/// Full pipeline: returns program output, or rendered diagnostics on failure.
pub fn run_source(name: &str, text: &str) -> Result<String, String> {
    let session = Session::new();
    let sm = SourceMap::new(name, text);
    let (module, mut diags) = parse::parse_module(&session, text);
    diags.extend(resolve::check(&session, &module));
    if let Some(rendered) = fail_if_errors(&diags, &sm) {
        return Err(rendered);
    }
    match eval::run_module(&module) {
        Ok(interp) => Ok(interp.output().to_string()),
        Err(e) => Err(render(&[e.diag], &sm)),
    }
}

/// Front-end only (no execution).
pub fn check_source(name: &str, text: &str) -> Result<(), String> {
    let session = Session::new();
    let sm = SourceMap::new(name, text);
    let (module, mut diags) = parse::parse_module(&session, text);
    diags.extend(resolve::check(&session, &module));
    match fail_if_errors(&diags, &sm) {
        Some(rendered) => Err(rendered),
        None => Ok(()),
    }
}

fn fail_if_errors(diags: &[Diagnostic], sm: &SourceMap) -> Option<String> {
    if diags.is_empty() {
        None
    } else {
        Some(render(diags, sm))
    }
}
```

Replace `src/main.rs`:
```rust
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => cmd(&args, true),
        Some("check") => cmd(&args, false),
        _ => {
            eprintln!("usage: lyra <run|check> <file.lyra>");
            ExitCode::from(2)
        }
    }
}

fn cmd(args: &[String], run: bool) -> ExitCode {
    let Some(path) = args.get(2) else {
        eprintln!("error: missing file path");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    if run {
        match lyra::run_source(path, &text) {
            Ok(out) => {
                print!("{out}");
                ExitCode::SUCCESS
            }
            Err(diags) => {
                eprint!("{diags}");
                ExitCode::FAILURE
            }
        }
    } else {
        match lyra::check_source(path, &text) {
            Ok(()) => {
                println!("ok");
                ExitCode::SUCCESS
            }
            Err(diags) => {
                eprint!("{diags}");
                ExitCode::FAILURE
            }
        }
    }
}
```

Create `examples/01_hello.lyra`:
```lyra
import lyra/io

pub fn main() / {IO} {
  io.println("Hello, Lyra!")
}
```

Create `examples/02_arith.lyra`:
```lyra
fn double(x) {
  x + x
}

pub fn main() / {IO} {
  let a = double(20)
  let b = a + 2
  if b == 42 {
    io.println("forty-two")
  } else {
    io.println("nope")
  }
}
```

- [ ] **Step 4: Run tests + a manual smoke run**

Run: `cargo test --lib tests`
Expected: PASS.
Run: `cargo run --quiet -- run examples/01_hello.lyra`
Expected output: `Hello, Lyra!`
Run: `cargo run --quiet -- run examples/02_arith.lyra`
Expected output: `forty-two`

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/main.rs examples/
git commit -m "feat: end-to-end pipeline, lyra run/check CLI, example programs"
```

---

## Task 13: golden example tests (`insta`)

**Files:**
- Create: `tests/examples.rs`

**Interfaces:**
- Consumes: `lyra::run_source` (public).

- [ ] **Step 1: Write the failing test**

`tests/examples.rs`:
```rust
//! Runs each example program and snapshots its output.

fn run_example(rel: &str) -> String {
    let path = format!("{}/examples/{rel}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).expect("read example");
    lyra::run_source(rel, &text).expect("example should run cleanly")
}

#[test]
fn hello() {
    insta::assert_snapshot!("01_hello", run_example("01_hello.lyra"));
}

#[test]
fn arith() {
    insta::assert_snapshot!("02_arith", run_example("02_arith.lyra"));
}
```

- [ ] **Step 2: Run test to verify it fails (pending snapshot)**

Run: `cargo test --test examples`
Expected: FAIL — snapshots not yet accepted (insta reports pending).

- [ ] **Step 3: Review and accept snapshots**

Run: `cargo insta review` (or `cargo insta accept` if `cargo-insta` is installed; otherwise set `INSTA_UPDATE=always cargo test --test examples` once and inspect the generated `.snap` files).
Confirm `01_hello` snapshot content is `Hello, Lyra!\n` and `02_arith` is `forty-two\n`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test examples`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tests/examples.rs tests/snapshots/
git commit -m "test: golden snapshot tests for example programs"
```

---

## Task 14: UI diagnostic harness (`//~ ERROR[Ennnn]`)

**Files:**
- Create: `tests/ui.rs`, `tests/ui/unresolved.lyra`, `tests/ui/bad_builtin.lyra`

**Interfaces:**
- The harness parses `//~ ERROR[Ennnn] <substring>` annotations from a fixture, runs `check_source`, and asserts (a) every expected code appears and (b) no rendered `%r`/`%e`/`%s` gibberish token appears (guards the future effect-diagnostics invariant from spec §9).

> **Note:** `check_source` returns *rendered* text. To assert on structured codes robustly, this harness checks that the rendered output contains each expected `Ennnn` code substring. Structured-field assertions on effect diagnostics arrive with the effect passes in Slice 3.

- [ ] **Step 1: Write the fixtures**

`tests/ui/unresolved.lyra`:
```lyra
fn main() {
  x
}
//~ ERROR[E0200] unresolved name
```

`tests/ui/bad_builtin.lyra`:
```lyra
fn main() {
  io.nope("x")
}
//~ ERROR[E0201] unknown builtin
```

- [ ] **Step 2: Write the failing harness test**

`tests/ui.rs`:
```rust
//! UI tests: fixtures annotated with `//~ ERROR[Ennnn] substring`.

use std::fs;

struct Expectation {
    code: String,
    substring: String,
}

fn parse_expectations(src: &str) -> Vec<Expectation> {
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix("//~ ERROR[") {
            let (code, tail) = rest.split_once(']').expect("malformed //~ ERROR");
            out.push(Expectation {
                code: code.to_string(),
                substring: tail.trim().to_string(),
            });
        }
    }
    out
}

fn check_fixture(name: &str) {
    let path = format!("{}/tests/ui/{name}", env!("CARGO_MANIFEST_DIR"));
    let src = fs::read_to_string(&path).expect("read fixture");
    let expects = parse_expectations(&src);
    assert!(!expects.is_empty(), "fixture has no expectations: {name}");
    let rendered = lyra::check_source(name, &src)
        .expect_err("fixture should produce diagnostics");
    for e in &expects {
        assert!(
            rendered.contains(&e.code),
            "missing code {} in output:\n{rendered}",
            e.code
        );
        assert!(
            rendered.contains(&e.substring),
            "missing text `{}` in output:\n{rendered}",
            e.substring
        );
    }
    // Invariant (spec §9): no raw inference tail-variable tokens leak.
    for bad in ["%r", "%e", "%s"] {
        assert!(
            !rendered.contains(bad),
            "diagnostic leaked internal token `{bad}`:\n{rendered}"
        );
    }
}

#[test]
fn unresolved() {
    check_fixture("unresolved.lyra");
}

#[test]
fn bad_builtin() {
    check_fixture("bad_builtin.lyra");
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --test ui`
Expected: PASS (2 tests). If a fixture's `//~` line itself triggers a lex error, confirm comments are skipped by the lexer (Task 4 skips `//...`).

- [ ] **Step 4: Commit**

```bash
git add tests/ui.rs tests/ui/
git commit -m "test: UI diagnostic harness with //~ ERROR fixtures and no-gibberish invariant"
```

---

## Task 15: module-layering DAG test

**Files:**
- Create: `tests/arch/layering.rs` registered via `tests/arch.rs`

**Interfaces:**
- Enforces the pinned layer map from Global Constraints by scanning `src/*.rs` for `crate::<module>` references and failing on any back-edge.

- [ ] **Step 1: Write the failing test**

Create `tests/arch.rs`:
```rust
mod arch {
    include!("arch/layering.rs");
}
```

Create `tests/arch/layering.rs`:
```rust
//! Enforces the compiler's internal module layering so a future workspace
//! split stays mechanical. A module may only reference strictly-lower layers
//! (plus equal-layer span/diag and lex/ast).

use std::collections::HashMap;
use std::fs;

fn layer(module: &str) -> Option<i32> {
    let map: HashMap<&str, i32> = HashMap::from([
        ("span", 0), ("diag", 0),
        ("lex", 1), ("ast", 1),
        ("parse", 2),
        ("resolve", 3),
        ("types", 4),
        ("core", 5),
        ("eval", 6),
        ("main", 7),
    ]);
    map.get(module).copied()
}

#[test]
fn no_upward_module_references() {
    let dir = format!("{}/src", env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    for entry in fs::read_dir(&dir).expect("read src") {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
        if stem == "lib" {
            continue; // lib.rs wires all modules together by design
        }
        let Some(this_layer) = layer(&stem) else { continue };
        let src = fs::read_to_string(&path).unwrap();
        for other in ["span", "diag", "lex", "ast", "parse", "resolve", "types", "core", "eval"] {
            if other == stem {
                continue;
            }
            let needle = format!("crate::{other}");
            if src.contains(&needle) {
                let other_layer = layer(other).unwrap();
                // Allowed: strictly lower, or equal-layer foundation pairs.
                let equal_ok = this_layer == other_layer
                    && matches!(
                        (stem.as_str(), other),
                        ("diag", "span") | ("ast", "lex") | ("lex", "ast") | ("parse", "ast")
                    );
                if other_layer >= this_layer && !equal_ok {
                    violations.push(format!(
                        "{stem} (layer {this_layer}) references {other} (layer {other_layer})"
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "module layering violations:\n{}",
        violations.join("\n")
    );
}
```

- [ ] **Step 2: Run test to verify it passes on the current layout**

Run: `cargo test --test arch`
Expected: PASS. If it FAILS, the failure names a real back-edge — fix the offending `use crate::…` (a lower-layer module must not depend on a higher-layer one). Note `parse` referencing `ast` and `lex` is allowed (both lower or equal-foundation).

- [ ] **Step 3: Add a self-check that the detector actually detects**

Add to `tests/arch/layering.rs`:
```rust
#[test]
fn detector_flags_a_synthetic_backedge() {
    // Sanity: the same predicate flags an upward edge.
    let this_layer = layer("span").unwrap(); // 0
    let other_layer = layer("eval").unwrap(); // 6
    assert!(other_layer >= this_layer, "predicate must flag span->eval");
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test arch`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add tests/arch.rs tests/arch/layering.rs
git commit -m "test(arch): enforce module-layer DAG so workspace split stays mechanical"
```

---

## Task 16: TCE depth-instrumentation "grow" control

**Files:**
- Create: `tests/tce.rs`
- Modify: `src/eval.rs` (expose a helper to run a module and read `max_depth`)

**Interfaces:**
- Consumes: `eval::{run_module, Interp}`.
- Produces: `pub fn run_module_with_stats(module: &Module) -> Result<Interp, RuntimeError>` is just `run_module` (already returns `Interp` exposing `max_depth()`), so no new function is required — the test uses `run_module` + `Interp::max_depth`.

> **Slice-1 scope:** the tree-walker recurses on the host stack, so it does **not** yet achieve bounded depth for tail recursion. This task establishes the *measurement machinery* and the "grow" control (deeper recursion ⇒ larger `max_depth`). The bounded/`assert_eq!` assertions from spec §11.4 land in Slice 2 with the CEK machine.

- [ ] **Step 1: Write the test**

`tests/tce.rs`:
```rust
//! Depth-instrumentation control. Bounded-depth (TCE) assertions arrive in
//! Slice 2 with the CEK machine; here we prove the measurement grows with
//! recursion, so the harness is real when the guarantee lands.

use lyra::ast::Module;
use lyra::parse::parse_module;
use lyra::eval::run_module;
use lyra::Session;

fn max_depth_for(src: &str) -> usize {
    let (m, d): (Module, _) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse diags: {d:?}");
    run_module(&m).unwrap().max_depth()
}

#[test]
fn recursion_depth_grows_with_input() {
    // A self-recursive countdown; deeper input => deeper host recursion (Slice 1).
    let prog = |n: i64| {
        format!(
            "fn go(n) {{ if n == 0 {{ 0 }} else {{ go(n - 1) }} }}\n\
             pub fn main() {{ let _ = go({n})\n io.println(\"done\") }}\n"
        )
    };
    let shallow = max_depth_for(&prog(5));
    let deep = max_depth_for(&prog(50));
    assert!(
        deep > shallow,
        "expected deeper recursion to grow max_depth: shallow={shallow}, deep={deep}"
    );
}
```

For this test to compile, `run_module`, `parse_module`, and modules must be public. Confirm `pub mod eval;`, `pub mod parse;`, `pub mod ast;` in `lib.rs` (they are) and that `run_module`, `parse_module`, `Interp::max_depth`, `Module` are `pub` (they are per Tasks 5/7/10/11).

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test --test tce`
Expected: PASS. (If `go` calling `go` fails to resolve, confirm Task 11 installs top-level fns into `globals_env` and that call environments are children of `globals_env`.)

- [ ] **Step 3: Commit**

```bash
git add tests/tce.rs
git commit -m "test(tce): depth-instrumentation grow-control (bounded asserts land Slice 2)"
```

---

## Task 17: local CI wiring + developer docs

**Files:**
- Create: `.git/hooks/pre-push` (documented; created by a setup step), `scripts/setup-hooks.sh`, `scripts/setup-hooks.ps1`
- Create: `README.md`

**Interfaces:**
- None (tooling/docs). Ensures the whole suite runs locally before any (future) push.

- [ ] **Step 1: Create the hook installer scripts**

`scripts/setup-hooks.sh`:
```sh
#!/usr/bin/env sh
set -e
hook=".git/hooks/pre-push"
cat > "$hook" <<'EOF'
#!/usr/bin/env sh
exec sh scripts/check.sh
EOF
chmod +x "$hook"
echo "installed pre-push hook -> scripts/check.sh"
```

`scripts/setup-hooks.ps1`:
```powershell
$hook = ".git/hooks/pre-push"
Set-Content -Path $hook -Value "#!/usr/bin/env sh`nexec sh scripts/check.sh" -Encoding utf8
Write-Output "installed pre-push hook -> scripts/check.sh"
```

- [ ] **Step 2: Write the README**

`README.md`:
```markdown
# Lyra

An effects-first, statically-typed, natively-compiled language (in progress).
This repository currently implements **Slice 1**: a tree-walking interpreter for
a small subset (literals, arithmetic, `let`, `if/else`, functions, `io.println`).

See the design spec: `docs/superpowers/specs/2026-08-05-lyra-language-design.md`.

## Build & run

```sh
cargo build
cargo run -- run examples/01_hello.lyra     # prints: Hello, Lyra!
cargo run -- check examples/02_arith.lyra    # front-end only
```

## Local CI

```sh
sh scripts/check.sh        # fmt + clippy + tests (POSIX)
pwsh scripts/check.ps1     # fmt + clippy + tests (PowerShell)
sh scripts/setup-hooks.sh  # install pre-push hook (optional; local only)
```

This project is **local-only**: no git remote is configured.
```

- [ ] **Step 3: Run the full local check**

Run: `sh scripts/check.sh` (or `pwsh scripts/check.ps1` on Windows).
Expected: `cargo fmt --check` clean, `cargo clippy -D warnings` clean, all tests PASS. Fix any `clippy`/`fmt` findings (e.g., run `cargo fmt --all`).

- [ ] **Step 4: Commit**

```bash
git add scripts/ README.md
git commit -m "chore: local CI check script, pre-push hook installer, README"
```

---

## Self-Review

**1. Spec coverage (design spec §§ mapped to tasks):**
- §3 syntax subset (brace-delimited, `<>`, `|>`, effect-row token, `if` expression) → Tasks 4, 6, 7. *(Full syntax — ADTs, traits, `case`, effects — is Slices 2–3, out of this plan by design.)*
- §4 EBNF (Slice-1 subset: import, fn, block, let, if, expressions) → Tasks 5, 6, 7.
- §5 example programs 01, 02(arith/if) runnable → Tasks 12, 13. *(03–10 require types/ADTs/effects → later slices.)*
- §7.3 TCE as tested guarantee → Task 16 establishes the measurement harness + grow control; bounded assertions deferred to Slice 2 (documented, per §12 sequencing).
- §9 diagnostics discipline + no-`%r/%e/%s` invariant → Task 14.
- §10.1 minimum crate layout → Task 1; §10.3 pure-pass signatures → all pass tasks; §10.4 lexer/parser/recovery → Tasks 4, 6, 7, 8; §10.5 name resolution → Task 9; §10.7 layering enforcement → Task 15.
- §11 testing: snapshots → Task 13; UI diagnostics → Task 14; oracle discipline → the tree-walker is the sole evaluator in Slice 1 (oracle cross-check begins Slice 2); layering → Task 15; TCE harness → Task 16; CI → Tasks 1, 17. *(Fuzzing/proptest deferred: not required to make Slice 1 a working deliverable; add in Slice 2 when the surface is larger.)*
- §12 Slice 1 exit criterion (`lyra run` on examples produces golden output; harnesses green) → Tasks 12–17.

**Gaps intentionally deferred (not Slice 1):** types/inference, CEK machine, ADTs/`case`, generics, traits, effects/handlers, bytecode/native, fuzzing. Each is a named later slice in spec §12–§13.

**2. Placeholder scan:** No `TBD`/`TODO`/"handle edge cases". Every code step contains runnable code. The one forward-reference note (Task 10's `Call` arm returns an explicit error superseded in Task 11) is intentional and labeled.

**3. Type consistency:** `Session`, `Span`, `Spanned`, `SourceMap`, `Diagnostic` (`error`/`with_label`/`with_help`), `TokenKind`, `Token`, `lex`, AST node names, `parse_expr_str`/`parse_module`, `check`/`builtins`, `Value`/`Env`/`Interp`/`eval_expr`/`eval_block`/`run_module`/`max_depth`, `run_source`/`check_source` are used identically across tasks. `pretty`/`pretty_expr_public` added in Tasks 5/6 and consumed in tests. Builtin name `io.println` is identical in `resolve::builtins`, `eval::call_builtin`, and examples.

---

## Execution Handoff

Plan complete. Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — execute tasks in this session with checkpoints for review.

Which approach?
