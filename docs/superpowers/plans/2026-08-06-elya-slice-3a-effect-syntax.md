# Elya Sub-Slice 3a — AST `Box`→`Rc` + Effect Syntax Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the mechanical, behavior-preserving groundwork for Slice 3 — convert the AST's recursive positions from `Box` to `Rc` (so the CEK machine can later capture continuations into first-class `resume` values), and **parse + resolve** the effect syntax (`effect` declarations, `handle … with [multi] { … }`, `resume`) — **with no type-checking or evaluation semantics**.

**Architecture:** Two isolated changes behind the full existing suite. (1) `Box`→`Rc` in `Expr`/`Block`/`FnDecl` recursive positions — deref-only downstream, no behavior change. (2) New AST nodes + parser + resolver for effects; the type checker and evaluator get placeholder arms so the crate compiles and effect programs parse+resolve but are rejected downstream with a "not yet implemented (Slice 3b/3c)" diagnostic.

**Tech Stack:** Rust 2021; existing deps only (`logos`, `ariadne`, `insta`). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-06-elya-slice-3-effects.md` (approved). "spec §X" refers there.

## Global Constraints

- **Rust edition 2021**, toolchain `stable`, MSRV 1.75. No new dependencies.
- **Pass-signature rule** (unchanged): every pass is `fn(&Session, In) -> (Out, Vec<Diagnostic>)` / `-> Vec<Diagnostic>`; no globals/`thread_local`.
- **Module layer map (pinned, unchanged):** `span/diag=0, lex=1, ast=1, parse=2, resolve=3, types=4, core=5, eval=6, main=7`. `tests/arch/layering.rs` enforces it.
- **Diagnostic codes:** `E00xx` lex, `E01xx` parse, `E02xx` resolve, `E03xx` runtime, `E04xx` types (`E042x` reserved for effects — the effect *checks* land in 3b, not here). 3a introduces **no new `E042x`**; the "effects not yet implemented" placeholder uses a temporary **`E0499`** (general "unsupported construct") in the type pass.
- **Diagnostic discipline:** any user-facing message zonks/uses readable names; UI fixtures forbid `%`-tokens.
- **3a EXIT GATE (spec §9, review-confirmed):** the **entire existing test suite stays green** after 3a. Effect syntax parses + resolves; no effect *semantics* land. `Box`→`Rc` is behavior-preserving.
- **`cargo fmt --all` before every commit; `scripts/check.sh` (fmt + clippy `-D warnings` + `cargo test --all`) green at each task's end.** Toolchain PATH: prepend `~/.cargo/bin` (see project memory). Push to `origin/main` after each commit (standing rule).
- **Resume representation (plan decision, resolving the spec §7.1 open item):** `resume` is a **dedicated AST node** `Expr::Resume { arg }`, and `resume` becomes a **contextually reserved word** — `resume(EXPR)` always parses to `Expr::Resume`. This is cleaner for the machine (a distinct node) than overloading a call.

---

## File Structure

| File | Responsibility | 3a change |
|---|---|---|
| `src/ast.rs` | AST node types + pretty-printer | `Box`→`Rc`; add `TypeAnn`, `EffectDecl`, `OpSig`, `Decl::Effect`, `Handler`, `OpClause`, `Expr::Handle`, `Expr::Resume`; pretty arms |
| `src/parse.rs` | parser | `Box::new`→`Rc::new`, `Vec`→`Rc<[_]>`; parse `effect`, `handle`/`with`/`multi`, `resume` |
| `src/resolve.rs` | name resolution | register effects; resolve operation calls + `resume`; handle `Decl::Effect`; iterate `Rc<[_]>` |
| `src/types.rs` | HM inference | placeholder arms: skip `Decl::Effect`; `Expr::Handle`/`Resume` → `E0499` + `Ty::Error`; iterate `Rc<[_]>` |
| `src/eval.rs` | evaluators (tree + cek) | placeholder arms: skip `Decl::Effect`; `Handle`/`Resume` → runtime error; iterate `Rc<[_]>` |
| `tests/effect_syntax.rs` | new — parse+resolve effect programs | new |
| `tests/ui/*.elya` | resolver-error fixtures | add `resume_outside_handler.elya` |

No lexer change: `effect`/`handle`/`with`/`multi`/`return` tokens already exist (`src/lex.rs`); `resume` lexes as `Lower("resume")` and is handled in the parser.

---

## Task 1: AST `Box` → `Rc` (mechanical, behavior-preserving)

**Files:**
- Modify: `src/ast.rs` (field types + constructors in `pretty`), `src/parse.rs`, `src/resolve.rs`, `src/types.rs`, `src/eval.rs`
- Test: the existing suite (no new test — this task's gate is "everything still green")

**Interfaces:**
- Produces (new field types — every later task and file relies on these):
  - `Block { stmts: Rc<[Spanned<Stmt>]>, tail: Option<Rc<Spanned<Expr>>> }`
  - `Expr::Call { callee: Rc<Spanned<Expr>>, args: Rc<[Spanned<Expr>]> }`
  - `Expr::Unary { op: UnOp, expr: Rc<Spanned<Expr>> }`
  - `Expr::Binary { op: BinOp, lhs: Rc<Spanned<Expr>>, rhs: Rc<Spanned<Expr>> }`
  - `Expr::If { cond: Rc<Spanned<Expr>>, then_block: Rc<Spanned<Block>>, else_block: Rc<Spanned<Block>> }`
  - `FnDecl { is_pub, name, params, effect_row: Vec<String>, body: Rc<Spanned<Block>> }`
- Every downstream read is deref-only: `Rc<T>` derefs to `T`; `Rc<[T]>` derefs to `[T]` (iterate with `.iter()`).

- [ ] **Step 1: Change the field types in `src/ast.rs`**

Add `use std::rc::Rc;` at the top. Change the six structs/variants above from `Box`/`Vec`/`Spanned<Block>` to the `Rc` forms. The three transforms are: `Box<X>` → `Rc<X>`; `Vec<X>` (for `stmts`, `args`) → `Rc<[X]>`; `Spanned<Block>` (for `then_block`, `else_block`, `FnDecl.body`) → `Rc<Spanned<Block>>`. Leave `Vec<Spanned<Param>>` (params), `Vec<Spanned<Import>>`, `Vec<Spanned<Decl>>` as `Vec` (not machine-captured).

- [ ] **Step 2: Fix `src/ast.rs`'s own pretty-printer**

The pretty-printer iterates `b.stmts` and derefs boxed children. Change `for st in &b.stmts` → `for st in b.stmts.iter()`; boxed-child derefs (`&callee.node`, `&lhs.node`, etc.) are unchanged (`Rc` derefs the same). In the `tests` module's builder helper, replace `Box::new(...)` with `Rc::new(...)` and any `vec![...]` used as `stmts`/`args` with `Rc::from(vec![...])` (or `[...].into()`), and `then_block`/`else_block`/`body` `Spanned<Block>` values with `Rc::new(<that Spanned<Block>>)`.

- [ ] **Step 3: Fix `src/parse.rs` constructors**

Every `Box::new(e)` for a captured child becomes `Rc::new(e)`. For `Expr::Call { callee, args }`: `callee: Rc::new(callee)`, and build `args: Vec<Spanned<Expr>>` then pass `args: args.into()` (`Vec<T> → Rc<[T]>`). For `block()`: it returns `Spanned<Block>`; where a `Block`'s `stmts` is built as a `Vec`, convert with `.into()` for the `Rc<[_]>` field, and `tail: Some(Rc::new(e))`. For `if_expr`: `cond: Rc::new(cond)`, `then_block: Rc::new(then_block)`, `else_block: Rc::new(else_block)` (each `self.block()?` result wrapped). For `fn_decl`: `body: Rc::new(body)`.

Concretely, `block()`'s return changes from `Block { stmts, tail }` to:
```rust
Some(spanned(
    Block {
        stmts: stmts.into(),               // Vec<Spanned<Stmt>> -> Rc<[_]>
        tail: tail.map(Rc::new),           // Option<Spanned<Expr>> -> Option<Rc<Spanned<Expr>>>
    },
    start.merge(end),
))
```
(where `tail` is accumulated as `Option<Spanned<Expr>>`, not `Option<Box<...>>`), and the tail-vs-stmt logic sets `tail = Some(e)` (a `Spanned<Expr>`, no `Box`).

- [ ] **Step 4: Fix the consumers (`resolve.rs`, `types.rs`, `eval.rs`)**

The compiler lists every site; each is exactly one of two mechanical fixes:
- **Iterate an `Rc<[_]>`:** `for st in &b.stmts` → `for st in b.stmts.iter()`; `for a in args` → `for a in args.iter()` (where `args` is now `Rc<[_]>`). Zips like `params.iter().zip(args)` become `.zip(args.iter())`.
- **Deref stays the same:** `&callee.node`, `&lhs.node`, `&cond.node`, `&then_block.node`, `&fdecl.body.node` all still work (`Rc` auto-derefs).

Known sites to change to `.iter()`: `resolve::check_block`/`check_expr` (stmts, call args); `types::infer_block` (stmts), `types::collect_refs`/`collect_refs_expr` (stmts, args), `types::infer_call` (args map), `types::infer_schemes` (nothing — decls stay `Vec`); `eval::tree::eval_block`/`eval_expr` (stmts, args); `eval::cek` frames already take `&[Spanned<...>]` — for **borrowed** slices from `Rc<[_]>`, use `&stmts[..]` / `&args[..]` (deref to slice). (The `cek` machine is untouched semantically here; it still borrows — the `Rc` fields deref to slices/nodes exactly as the old `Box`/`Vec` did.)

- [ ] **Step 5: Run the full suite — the gate**

Run: `cargo fmt --all && sh scripts/check.sh` (with `~/.cargo/bin` on PATH).
Expected: **all existing tests pass** (43 lib + arch + crosscheck + examples + tce + type_schemes + types + ui), fmt clean, clippy `-D warnings` clean. Behavior is unchanged — this is the pre-flagged behavior-preserving change. If anything fails, it is a mechanical miss (a `Box::new` left, or a `&rc_slice` needing `.iter()`); fix it.

- [ ] **Step 6: Commit + push**

```bash
git add -A
git commit -m "refactor(ast): Box -> Rc in recursive positions (prep for Slice-3 continuation capture); behavior-preserving"
git push origin main
```

---

## Task 2: New effect AST nodes + placeholder downstream arms

**Files:**
- Modify: `src/ast.rs` (new nodes + pretty), `src/resolve.rs`, `src/types.rs`, `src/eval.rs` (placeholder arms so the crate compiles + existing tests stay green)
- Test: inline `#[cfg(test)]` in `src/ast.rs`

**Interfaces:**
- Produces:
  - `pub struct TypeAnn { pub name: String, pub args: Vec<Spanned<TypeAnn>> }` — a minimal surface type (base names like `Int`, `String`, `Unit`; args for future generic types).
  - `pub struct OpSig { pub name: String, pub params: Vec<Spanned<Param>>, pub param_tys: Vec<Spanned<TypeAnn>>, pub ret: Spanned<TypeAnn> }`
  - `pub struct EffectDecl { pub name: String, pub ops: Vec<Spanned<OpSig>> }`
  - `Decl::Effect(EffectDecl)` (new variant alongside `Decl::Fn`)
  - `pub struct OpClause { pub effect: Option<String>, pub op: String, pub params: Vec<Spanned<Param>>, pub body: Rc<Spanned<Expr>> }` (`effect` is the `Effect` in `Effect.op(..)`; `None` if written unqualified)
  - `pub struct ReturnClause { pub binder: String, pub body: Rc<Spanned<Expr>> }`
  - `pub struct Handler { pub multi: bool, pub clauses: Vec<Spanned<OpClause>>, pub ret: Option<ReturnClause> }`
  - `Expr::Handle { body: Rc<Spanned<Expr>>, handler: Rc<Handler> }`
  - `Expr::Resume { arg: Rc<Spanned<Expr>> }`
  - `pub fn pretty` handles the new nodes.

- [ ] **Step 1: Write the failing test**

Add to `src/ast.rs` tests:
```rust
    #[test]
    fn pretty_prints_effect_decl_and_handle() {
        // effect Log { fn log(msg: String) -> Unit }
        let eff = Decl::Effect(EffectDecl {
            name: "Log".into(),
            ops: vec![sp(OpSig {
                name: "log".into(),
                params: vec![sp(Param { name: "msg".into() })],
                param_tys: vec![sp(TypeAnn { name: "String".into(), args: vec![] })],
                ret: sp(TypeAnn { name: "Unit".into(), args: vec![] }),
            })],
        });
        // handle x with { Log.log(m) -> resume(m) return(r) -> r }
        let handler = Handler {
            multi: false,
            clauses: vec![sp(OpClause {
                effect: Some("Log".into()),
                op: "log".into(),
                params: vec![sp(Param { name: "m".into() })],
                body: Rc::new(sp(Expr::Resume { arg: Rc::new(sp(Expr::Var("m".into()))) })),
            })],
            ret: Some(ReturnClause { binder: "r".into(), body: Rc::new(sp(Expr::Var("r".into()))) }),
        };
        let handle = Expr::Handle {
            body: Rc::new(sp(Expr::Var("x".into()))),
            handler: Rc::new(handler),
        };
        let m = Module {
            imports: vec![],
            decls: vec![sp(eff), sp(Decl::Fn(FnDecl {
                is_pub: false, name: "f".into(), params: vec![], effect_row: vec![],
                body: Rc::new(sp(Block { stmts: Rc::from(vec![]), tail: Some(Rc::new(sp(handle))) })),
            }))],
        };
        assert_eq!(
            pretty(&m),
            "(module (effect Log (log)) (fn f () (block (handle x (Log.log (m) (resume m)) (return r r)))))"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib ast::tests::pretty_prints_effect_decl_and_handle`
Expected: FAIL — types not found.

- [ ] **Step 3: Add the AST nodes**

In `src/ast.rs`, add the structs from Interfaces (all `#[derive(Clone, Debug, PartialEq)]`), add `Effect(EffectDecl)` to `enum Decl`, add `Handle { … }` and `Resume { … }` to `enum Expr`. Extend the pretty-printer:
```rust
// in pretty_decl:
        Decl::Effect(e) => {
            s.push_str(&format!("(effect {}", e.name));
            for op in &e.ops {
                s.push_str(&format!(" ({})", op.node.name));
            }
            s.push(')');
        }
// in pretty_expr, new arms:
        Expr::Handle { body, handler } => {
            s.push_str("(handle ");
            pretty_expr(&body.node, s);
            for c in &handler.clauses {
                let c = &c.node;
                let eff = c.effect.clone().unwrap_or_default();
                s.push_str(&format!(" ({}.{} (", eff, c.op));
                for (i, p) in c.params.iter().enumerate() {
                    if i > 0 { s.push(' '); }
                    s.push_str(&p.node.name);
                }
                s.push_str(") ");
                pretty_expr(&c.body.node, s);
                s.push(')');
            }
            if let Some(r) = &handler.ret {
                s.push_str(&format!(" (return {} ", r.binder));
                pretty_expr(&r.body.node, s);
                s.push(')');
            }
            s.push(')');
        }
        Expr::Resume { arg } => {
            s.push_str("(resume ");
            pretty_expr(&arg.node, s);
            s.push(')');
        }
```

- [ ] **Step 4: Add placeholder arms downstream so the crate compiles + existing tests stay green**

`Decl` now has two variants, so every `let Decl::Fn(f) = &d.node` becomes refutable. Fix each iteration to **skip effects**:
- `resolve::check`: replace `let Decl::Fn(f) = &d.node;` loops with `if let Decl::Fn(f) = &d.node { … }` (build `fn_names` from `Fn` only; skip `Effect`). Add `Expr` arms: `Expr::Handle { body, handler } => { self.check_expr(&body.node, body.span, scope); /* clauses/resume resolved in Task 5 */ }` and `Expr::Resume { arg } => self.check_expr(&arg.node, arg.span, scope)` — *provisional* (Task 5 tightens).
- `types::infer_schemes` / `collect_refs`: iterate only `Decl::Fn` (skip `Effect`). Add `infer_expr` arms:
  ```rust
  Expr::Handle { .. } | Expr::Resume { .. } => {
      self.diags.push(Diagnostic::error(
          "E0499",
          "effects are not type-checked yet (Slice 3b)",
      ).with_label(span, "unsupported here"));
      Ty::Error
  }
  ```
- `eval::tree::eval_expr` and `eval::cek::eval`: add arms for `Handle`/`Resume` returning `Err(rt(span, "effects are not evaluated yet (Slice 3c)"))`. `fn_table` and `run_module` iterate only `Decl::Fn` (skip `Effect`).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib ast` then `cargo fmt --all && sh scripts/check.sh`.
Expected: the new AST test passes; **all existing tests still green** (no existing program uses the new nodes, so the placeholder arms are never hit).

- [ ] **Step 6: Commit + push**

```bash
git add -A
git commit -m "feat(ast): effect/handle/resume nodes + TypeAnn; placeholder downstream arms (E0499/runtime), suite green"
git push origin main
```

---

## Task 3: Parse `effect` declarations

**Files:**
- Modify: `src/parse.rs`
- Test: inline `#[cfg(test)]` in `src/parse.rs`

**Interfaces:**
- Consumes: `TypeAnn`, `OpSig`, `EffectDecl`, `Decl::Effect` (Task 2).
- Produces: `module()` recognizes `effect NAME { fn op(p: T, …) -> T … }` and pushes `Decl::Effect`. Internal `fn effect_decl(&mut self)`, `fn op_sig(&mut self)`, `fn type_ann(&mut self)`.

- [ ] **Step 1: Write the failing test**

Add to `src/parse.rs` tests:
```rust
    #[test]
    fn parses_effect_declaration() {
        let src = "effect Log {\n  fn log(msg: String) -> Unit\n}\n";
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "diags: {d:?}");
        assert_eq!(crate::ast::pretty(&m), "(module (effect Log (log)))");
        let Decl::Effect(e) = &m.decls[0].node else { panic!("expected effect") };
        assert_eq!(e.name, "Log");
        assert_eq!(e.ops[0].node.name, "log");
        assert_eq!(e.ops[0].node.param_tys[0].node.name, "String");
        assert_eq!(e.ops[0].node.ret.node.name, "Unit");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib parse::tests::parses_effect_declaration`
Expected: FAIL — `module()` errors on `effect` (unexpected decl keyword).

- [ ] **Step 3: Implement the parser methods**

In `module()`'s match, add a branch for `Some(TokenKind::KwEffect)` → `self.effect_decl()`. Add:
```rust
    fn effect_decl(&mut self) -> Option<Spanned<Decl>> {
        let start = self.peek_span();
        self.bump(); // effect
        let name = match self.peek()?.clone() {
            TokenKind::Upper(n) => { self.bump(); n }
            _ => { self.error(self.peek_span(), "expected effect name (uppercase)"); return None; }
        };
        if !self.eat(&TokenKind::LBrace) {
            self.error(self.peek_span(), "expected `{`"); return None;
        }
        let mut ops = Vec::new();
        while self.peek() == Some(&TokenKind::KwFn) {
            let op = self.op_sig()?;
            ops.push(op);
        }
        let end = self.peek_span();
        self.eat(&TokenKind::RBrace);
        Some(spanned(Decl::Effect(EffectDecl { name, ops }), start.merge(end)))
    }

    fn op_sig(&mut self) -> Option<Spanned<OpSig>> {
        let start = self.peek_span();
        self.bump(); // fn
        let name = match self.peek()?.clone() {
            TokenKind::Lower(n) => { self.bump(); n }
            _ => { self.error(self.peek_span(), "expected operation name"); return None; }
        };
        if !self.eat(&TokenKind::LParen) { self.error(self.peek_span(), "expected `(`"); return None; }
        let mut params = Vec::new();
        let mut param_tys = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                let pspan = self.peek_span();
                let pname = match self.peek()?.clone() {
                    TokenKind::Lower(n) => { self.bump(); n }
                    _ => { self.error(pspan, "expected parameter name"); return None; }
                };
                if !self.eat(&TokenKind::Colon) { self.error(self.peek_span(), "operation params need a type"); return None; }
                let ty = self.type_ann()?;
                params.push(spanned(Param { name: pname }, pspan));
                param_tys.push(ty);
                if !self.eat(&TokenKind::Comma) { break; }
            }
        }
        if !self.eat(&TokenKind::RParen) { self.error(self.peek_span(), "expected `)`"); return None; }
        if !self.eat(&TokenKind::Arrow) { self.error(self.peek_span(), "operation needs a return type `-> T`"); return None; }
        let ret = self.type_ann()?;
        let end = ret.span;
        Some(spanned(OpSig { name, params, param_tys, ret }, start.merge(end)))
    }

    fn type_ann(&mut self) -> Option<Spanned<TypeAnn>> {
        let span = self.peek_span();
        let name = match self.peek()?.clone() {
            TokenKind::Upper(n) => { self.bump(); n }
            TokenKind::Unit => { self.bump(); "Unit".to_string() }
            TokenKind::Lower(n) => { self.bump(); n } // type variable
            _ => { self.error(span, "expected a type"); return None; }
        };
        let mut args = Vec::new();
        if self.eat(&TokenKind::LParen) {
            if self.peek() != Some(&TokenKind::RParen) {
                loop {
                    args.push(self.type_ann()?);
                    if !self.eat(&TokenKind::Comma) { break; }
                }
            }
            self.eat(&TokenKind::RParen);
        }
        Some(spanned(TypeAnn { name, args }, span))
    }
```
Add `use crate::ast::{EffectDecl, OpSig, TypeAnn};` (or extend the existing `use crate::ast::*;`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib parse` then `sh scripts/check.sh`.
Expected: PASS (all parser tests incl. the new one; suite green).

- [ ] **Step 5: Commit + push**

```bash
git add -A
git commit -m "feat(parse): effect declarations (effect NAME { fn op(p: T) -> T }) with minimal TypeAnn"
git push origin main
```

---

## Task 4: Parse `handle` / `with` / `multi` / `resume`

**Files:**
- Modify: `src/parse.rs`
- Test: inline `#[cfg(test)]` in `src/parse.rs`

**Interfaces:**
- Consumes: `Handler`, `OpClause`, `ReturnClause`, `Expr::Handle`, `Expr::Resume` (Task 2).
- Produces: `atom()` recognizes `handle` and a `resume(EXPR)` call. Internal `fn handle_expr(&mut self)`, `fn op_clause(&mut self)`.

- [ ] **Step 1: Write the failing test**

Add to `src/parse.rs` tests:
```rust
    #[test]
    fn parses_handle_with_resume_and_return() {
        let src = "fn f() {\n  handle g() with {\n    Log.log(m) -> resume(m)\n    return(r) -> r\n  }\n}\n";
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "diags: {d:?}");
        assert_eq!(
            crate::ast::pretty(&m),
            "(module (fn f () (block (handle (call g) (Log.log (m) (resume m)) (return r r)))))"
        );
    }

    #[test]
    fn parses_multi_handler() {
        let src = "fn f() {\n  handle g() with multi {\n    Flip.flip() -> resume(True)\n  }\n}\n";
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "diags: {d:?}");
        let Decl::Fn(f) = &m.decls[0].node else { panic!() };
        // dig out the Handle node's multi flag
        let tail = f.body.node.tail.as_ref().unwrap();
        let Expr::Handle { handler, .. } = &tail.node else { panic!("expected handle") };
        assert!(handler.multi);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib parse::tests::parses_handle`
Expected: FAIL — `handle`/`resume` not parsed.

- [ ] **Step 3: Implement in the parser**

In `atom()`, add two arms at the top of the `match self.peek()?.clone()`:
```rust
            TokenKind::KwHandle => self.handle_expr(),
            TokenKind::Lower(ref n) if n == "resume" => {
                self.bump(); // resume
                let s = span;
                if !self.eat(&TokenKind::LParen) { self.error(self.peek_span(), "expected `(` after `resume`"); return None; }
                let arg = self.expr(0)?;
                let end = self.peek_span();
                if !self.eat(&TokenKind::RParen) { self.error(end, "expected `)`"); return None; }
                Some(spanned(Expr::Resume { arg: Rc::new(arg) }, s.merge(end)))
            }
```
(`resume` is now contextually reserved — Global Constraints.) Add:
```rust
    fn handle_expr(&mut self) -> Option<Spanned<Expr>> {
        let start = self.peek_span();
        self.bump(); // handle
        let body = self.expr(0)?;
        if !self.eat(&TokenKind::KwWith) { self.error(self.peek_span(), "expected `with`"); return None; }
        let multi = self.eat(&TokenKind::KwMulti);
        if !self.eat(&TokenKind::LBrace) { self.error(self.peek_span(), "expected `{`"); return None; }
        let mut clauses = Vec::new();
        let mut ret = None;
        while self.peek().is_some() && self.peek() != Some(&TokenKind::RBrace) {
            if self.peek() == Some(&TokenKind::KwReturn) {
                let start_r = self.peek_span();
                self.bump(); // return
                if !self.eat(&TokenKind::LParen) { self.error(self.peek_span(), "expected `(`"); return None; }
                let binder = match self.peek()?.clone() {
                    TokenKind::Lower(n) => { self.bump(); n }
                    _ => { self.error(self.peek_span(), "expected binder"); return None; }
                };
                if !self.eat(&TokenKind::RParen) { self.error(self.peek_span(), "expected `)`"); return None; }
                if !self.eat(&TokenKind::Arrow) { self.error(self.peek_span(), "expected `->`"); return None; }
                let body_r = self.expr(0)?;
                let _ = start_r;
                ret = Some(ReturnClause { binder, body: Rc::new(body_r) });
            } else {
                let c = self.op_clause()?;
                clauses.push(c);
            }
        }
        let end = self.peek_span();
        self.eat(&TokenKind::RBrace);
        Some(spanned(
            Expr::Handle { body: Rc::new(body), handler: Rc::new(Handler { multi, clauses, ret }) },
            start.merge(end),
        ))
    }

    fn op_clause(&mut self) -> Option<Spanned<OpClause>> {
        let start = self.peek_span();
        // Effect.op(params)  (Effect optional: `op(params)`)
        let (effect, op) = match self.peek()?.clone() {
            TokenKind::Upper(eff) => {
                self.bump();
                if !self.eat(&TokenKind::Dot) { self.error(self.peek_span(), "expected `.` after effect name"); return None; }
                let op = match self.peek()?.clone() {
                    TokenKind::Lower(o) => { self.bump(); o }
                    _ => { self.error(self.peek_span(), "expected operation name"); return None; }
                };
                (Some(eff), op)
            }
            TokenKind::Lower(o) => { self.bump(); (None, o) }
            _ => { self.error(self.peek_span(), "expected `Effect.op` clause"); return None; }
        };
        if !self.eat(&TokenKind::LParen) { self.error(self.peek_span(), "expected `(`"); return None; }
        let mut params = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                let pspan = self.peek_span();
                match self.peek()?.clone() {
                    TokenKind::Lower(n) => { self.bump(); params.push(spanned(Param { name: n }, pspan)); }
                    _ => { self.error(pspan, "expected parameter name"); return None; }
                }
                if !self.eat(&TokenKind::Comma) { break; }
            }
        }
        if !self.eat(&TokenKind::RParen) { self.error(self.peek_span(), "expected `)`"); return None; }
        if !self.eat(&TokenKind::Arrow) { self.error(self.peek_span(), "expected `->`"); return None; }
        let body = self.expr(0)?;
        let end = body.span;
        Some(spanned(OpClause { effect, op, params, body: Rc::new(body) }, start.merge(end)))
    }
```
Add `use crate::ast::{Handler, OpClause, ReturnClause};` (or the existing glob import already covers them).

> Note the `resume(m)` inside a clause parses via the `atom()` `resume` arm; `Log.log(m)` as a *clause head* is parsed by `op_clause` (not as a qualified expression) because `handle_expr` calls `op_clause` for each clause. The **body** `resume(m)` is an ordinary expression.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib parse` then `sh scripts/check.sh`.
Expected: PASS (handle + multi tests; suite green).

- [ ] **Step 5: Commit + push**

```bash
git add -A
git commit -m "feat(parse): handle/with/multi + resume (resume contextually reserved)"
git push origin main
```

---

## Task 5: Resolver — effect registration, operation calls, `resume`

**Files:**
- Modify: `src/resolve.rs`
- Create: `tests/ui/resume_outside_handler.elya`
- Test: inline `#[cfg(test)]` in `src/resolve.rs`; register the UI fixture in `tests/ui.rs`

**Interfaces:**
- Produces: `check` registers effect operations as callable names; operation calls resolve; `resume` resolves **only inside a handler clause body** (else `E0210`). New diagnostic **`E0210` — `resume` outside a handler**.

- [ ] **Step 1: Write the failing tests**

Add to `src/resolve.rs` tests:
```rust
    #[test]
    fn operation_call_resolves_when_effect_in_scope() {
        let d = diags("effect Log { fn log(m: String) -> Unit }\nfn f() / {Log} { log(\"hi\") }\n");
        assert!(d.is_empty(), "unexpected: {d:?}");
    }

    #[test]
    fn resume_inside_handler_resolves() {
        let d = diags("effect Log { fn log(m: String) -> Unit }\n\
                       fn f() { handle f() with { Log.log(m) -> resume(Unit) } }\n");
        assert!(d.is_empty(), "unexpected: {d:?}");
    }

    #[test]
    fn resume_outside_handler_is_e0210() {
        let d = diags("fn f() { resume(1) }\n");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0210");
    }
```

`tests/ui/resume_outside_handler.elya`:
```elya
fn f() {
  resume(1)
}
//~ ERROR[E0210] resume
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib resolve::tests::resume_outside_handler_is_e0210`
Expected: FAIL — `resume` currently resolves its arg without complaint (Task 2 placeholder).

- [ ] **Step 3: Implement resolution**

Extend the resolver: collect effect operation names into the builtin-like set; thread an `in_handler: bool` (or a depth counter) through `check_expr`; register handler-clause params in scope for the clause body.
```rust
// In `check`, gather operation names into `fn_names`-adjacent set:
    let mut op_names: HashSet<String> = HashSet::new();
    for d in &module.decls {
        if let Decl::Effect(e) = &d.node {
            for op in &e.ops {
                op_names.insert(op.node.name.clone());
            }
        }
    }
    // pass op_names into Cx (new field); a bare Var/Call to an operation resolves.
```
Add an `in_handler` counter to `Cx`. In `check_expr`:
```rust
            Expr::Resume { arg } => {
                if self.in_handler == 0 {
                    self.diags.push(
                        Diagnostic::error("E0210", "`resume` used outside a handler")
                            .with_label(span, "only valid inside a handler clause"),
                    );
                }
                self.check_expr(&arg.node, arg.span, scope);
            }
            Expr::Handle { body, handler } => {
                self.check_expr(&body.node, body.span, scope);
                self.in_handler += 1;
                for c in &handler.clauses {
                    let c = &c.node;
                    scope.push(HashSet::new());
                    for p in &c.params { scope.last_mut().unwrap().insert(p.node.name.clone()); }
                    self.check_expr(&c.body.node, c.body.span, scope);
                    scope.pop();
                }
                if let Some(r) = &handler.ret {
                    scope.push(HashSet::new());
                    scope.last_mut().unwrap().insert(r.binder.clone());
                    self.check_expr(&r.body.node, r.body.span, scope);
                    scope.pop();
                }
                self.in_handler -= 1;
            }
```
In `resolves_var` (and the `Var`/`Call` operation path), treat an `op_names` member as resolved (an operation used as a bare name/callee). Skip `Decl::Effect` when building `fn_names` and when iterating function bodies (already done in Task 2's placeholder; keep it).

- [ ] **Step 4: Register the UI fixture**

In `tests/ui.rs`, add:
```rust
#[test]
fn resume_outside_handler() { check_fixture("resume_outside_handler.elya"); }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib resolve && cargo test --test ui` then `sh scripts/check.sh`.
Expected: PASS.

- [ ] **Step 6: Commit + push**

```bash
git add -A
git commit -m "feat(resolve): register effect operations; resolve operation calls; E0210 for resume outside a handler"
git push origin main
```

---

## Task 6: 3a integration test + exit gate

**Files:**
- Create: `tests/effect_syntax.rs`
- Test: parse+resolve a complete effect program end to end

**Interfaces:**
- Consumes: `elya::parse::parse_module`, `elya::resolve::check`, `elya::Session`.

- [ ] **Step 1: Write the test**

`tests/effect_syntax.rs`:
```rust
//! Sub-slice 3a: effect syntax parses and resolves. NO type-checking or
//! evaluation yet (3b/3c) — those are intentionally out of scope here.

use elya::Session;

fn parse_resolve(src: &str) -> Vec<String> {
    let (m, pd) = elya::parse::parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse diags: {pd:?}");
    elya::resolve::check(&Session::new(), &m)
        .into_iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn full_effect_program_parses_and_resolves_clean() {
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn greet(name) / {Log} { log(\"hi \" <> name) }\n\
               pub fn main() / {IO} {\n\
                 handle greet(\"ada\") with {\n\
                   Log.log(m) -> { io.println(m) resume(Unit) }\n\
                   return(x) -> x\n\
                 }\n\
               }\n";
    assert!(parse_resolve(src).is_empty(), "resolve diags: {:?}", parse_resolve(src));
}

#[test]
fn effects_are_not_type_checked_yet() {
    // check_source runs types::infer, which rejects Handle with E0499 in 3a.
    let src = "effect Log { fn log(m: String) -> Unit }\n\
               fn f() { handle f() with { Log.log(m) -> resume(Unit) } }\n";
    let err = elya::check_source("t.elya", src).unwrap_err();
    assert!(err.contains("E0499"), "expected E0499 placeholder, got: {err}");
}
```

- [ ] **Step 2: Run + the full exit gate**

Run: `cargo test --test effect_syntax` then `cargo fmt --all && sh scripts/check.sh`.
Expected: PASS. **The 3a exit gate:** the entire suite is green, effect syntax parses + resolves, and effect programs are cleanly rejected downstream with the `E0499` placeholder (no semantics). No behavior of any pre-3a program changed.

- [ ] **Step 3: Commit + push**

```bash
git add -A
git commit -m "test(effects): 3a integration — effect syntax parses+resolves; effects gated with E0499 (no semantics yet)"
git push origin main
```

---

## Self-Review

**1. Spec coverage (spec §9 sub-slice 3a → tasks):**
- `Box`→`Rc` (spec §4.1, §7.1) → Task 1, with the **behavior-preserving green gate** (spec §9 3a gate) enforced in Task 1 Step 5 and Task 6.
- Parse `effect`/rows/`handle`/`with`/`multi`/`resume` (spec §7.1) → Tasks 3, 4 (effect-row `/ {E}` on functions is already parsed since Slice 1 as `Vec<String>` — unchanged here; elaboration to a real `EffectRow` is 3b).
- Resolver: register effects, resolve operations + `resume` (spec §7.2) → Task 5.
- **No semantics** (spec §9 3a): types/eval get placeholder arms (`E0499` / runtime error) → Task 2; asserted in Task 6.
- Resume representation decided (spec §7.1 open item) → Global Constraints + Task 4 (`Expr::Resume`, contextually reserved).

**Deliberately out of 3a (later sub-slices, flagged):** effect-row *types* + row unification + ambient inference + discharge (`E0420`–`E0424`) → 3b; handler/`resume` *semantics* on the CEK machine + one-shot (`E0425`) → 3c; multi-shot + `E0426` → 3d; effect-TCE → 3e. `TypeAnn` retains op-signature types now so 3b has them; function param/return type annotations remain parse-and-discard (Slice 2 behavior) — HM infers them.

**2. Placeholder scan:** No `TODO`/`TBD`. The `E0499` "not implemented yet" arms are **intentional, tested placeholders** for a syntax-only slice, not gaps — they are asserted in Task 6 and replaced in 3b/3c. Task 1 Step 4's "compiler lists every site; each is one of two mechanical fixes" enumerates the known sites and gives the exact transform, not a vague "fix errors."

**3. Type/name consistency:** `TypeAnn`, `OpSig{name,params,param_tys,ret}`, `EffectDecl{name,ops}`, `Decl::Effect`, `OpClause{effect,op,params,body}`, `ReturnClause{binder,body}`, `Handler{multi,clauses,ret}`, `Expr::Handle{body,handler}`, `Expr::Resume{arg}` are used identically across Tasks 2–6. The `Rc`/`Rc<[_]>` field types from Task 1 are consumed unchanged. Diagnostic codes: `E0210` (resume-outside-handler, resolve), `E0499` (effects-not-yet-typed, temporary) — both new here and used consistently.

---

## Execution Handoff

Plan complete. Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks.
2. **Inline Execution** — execute tasks in this session with checkpoints.

Which approach?
