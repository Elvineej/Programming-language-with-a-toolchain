# Elya Slice 2 — Types & CEK Machine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Slice 1's exact language surface **typed** (Hindley–Milner, Algorithm J) and **running on a CEK abstract machine**, and turn tail-call elimination into a **measured bounded-depth guarantee** — all without adding any new value-level construct.

**Architecture:** Add a `types` module (HM inference: unification, generalization, SCC-grouped top-level inference, a zonking type reporter) wired into the pipeline before evaluation. Refactor `eval` into a shared value/environment layer plus a retained tree-walker (`tree`, the oracle) and a new CEK machine (`cek`); the environment becomes a persistent `Rc`-scope so tail recursion stays bounded. The pipeline becomes `parse → resolve → infer → cek`.

**Tech Stack:** Rust (edition 2021); no new dependencies (`std::rc::Rc` only). Existing deps unchanged (`logos`, `ariadne`, `insta`).

**Spec:** `docs/superpowers/specs/2026-08-06-elya-slice-2-types-and-cek.md` (approved). Section refs "spec §X" point there; "design spec §X" points at `2026-08-05-elya-language-design.md`.

## Global Constraints

- **Rust edition 2021**, toolchain `stable` (pinned), MSRV 1.75. No new dependencies.
- **Surface is frozen to Slice 1's** — no new AST nodes, no `case`, no lambdas, no ADTs (spec §1.2).
- **Pass signature rule** (unchanged): every pass is `fn(&Session, In) -> (Out, Vec<Diagnostic>)` or `-> Vec<Diagnostic>`. No globals/`thread_local`.
- **Module layer map (unchanged, pinned in `tests/arch/layering.rs`):** `span=0, diag=0, lex=1, ast=1, parse=2, resolve=3, types=4, core=5, eval=6, main=7`. Slice 2 fills `types` (4) and extends `eval` (6). `types` may reference only `ast`/`span`/`diag` (all lower); `eval` may reference only `ast`/`span`/`diag` (it must **not** reference `types` — types are erased before evaluation).
- **Diagnostic code scheme (extended):** `E00xx` lex, `E01xx` parse, `E02xx` resolve, `E03xx` runtime, **`E0400` type mismatch, `E0401` infinite type, `E0402` arity mismatch, `E0403` non-`Bool` condition** (E0404–E0419 reserved for future type errors; **E0420–E0429 reserved for effects — do not use in Slice 2**).
- **Diagnostic discipline:** type errors **zonk before printing** and name free type variables `a`, `b`, `c`, … — never an internal `%t`/`%v` token (spec §2.8). UI fixtures assert this.
- **No behavior regression:** every Slice-1 test stays green except tests that fed deliberately ill-typed input to the evaluator, which move to compile-time type-error assertions.
- **Gate:** `scripts/check.sh` (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all`) must be green at the end of every task.
- **Git:** strictly local, **no remote**. Commit after each task's final step. Work on a branch off `main` (the executing skill creates it).

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/types.rs` | HM inference: `Ty`/`Scheme`, unification, generalization/instantiation, SCC-grouped module inference, type reporter | **new** (layer 4) |
| `src/eval.rs` | Shared `Value`/`Env`/`RuntimeError`/`apply_binop`; `tree` submodule (oracle); `cek` submodule (machine); TCE instrumentation | **restructured** (layer 6) |
| `src/lib.rs` | `pub mod types;` + pipeline `parse→resolve→infer→cek` | modify |
| `src/ast.rs`, `src/lex.rs`, `src/parse.rs`, `src/resolve.rs`, `src/span.rs`, `src/diag.rs`, `src/main.rs` | unchanged (surface frozen) | none |
| `tests/types.rs` | inference unit/integration tests (schemes, poly, errors) | new |
| `tests/ui/*.elya` | type-error fixtures (E0400–E0403) | add |
| `tests/crosscheck.rs` | `cek_output == tree_output` on examples + corpus | new |
| `tests/tce.rs` | upgrade from grow-control to bounded-depth assertions | modify |
| `tests/snapshots/` | inferred-scheme snapshots | add |
| `README.md` | status line: Slice 2 done | modify |

---

## Task 1: `types.rs` — type representation and unification

**Files:**
- Create: `src/types.rs`
- Modify: `src/lib.rs` (add `pub mod types;`)
- Test: inline `#[cfg(test)]` in `src/types.rs`

**Interfaces:**
- Produces:
  - `pub enum TyCon { Int, Float, Bool, Str, Unit }`
  - `pub enum Ty { Var(u32), Base(TyCon), Fn(Vec<Ty>, Box<Ty>), Tuple(Vec<Ty>), Error }`
  - `pub struct Scheme { pub vars: Vec<u32>, pub ty: Ty }`
  - `pub struct Infer { subst: Vec<Option<Ty>>, pub diags: Vec<Diagnostic> }` with `Infer::new()`, `fresh(&mut) -> Ty`, `resolve(&self, &Ty) -> Ty` (deep top-follow), `unify(&mut self, &Ty, &Ty, Span)`, and helpers `int()/float()/bool()/str()/unit()`.

- [ ] **Step 1: Add the module declaration**

In `src/lib.rs`, add `pub mod types;` in the module list (after `pub mod span;`, alphabetical is fine).

- [ ] **Step 2: Write the failing test**

Create `src/types.rs` with the test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    #[test]
    fn unifies_equal_bases() {
        let mut inf = Infer::new();
        inf.unify(&Ty::int(), &Ty::int(), Span::EMPTY);
        assert!(inf.diags.is_empty());
    }

    #[test]
    fn mismatched_bases_are_e0400() {
        let mut inf = Infer::new();
        inf.unify(&Ty::int(), &Ty::str(), Span::EMPTY);
        assert_eq!(inf.diags.len(), 1);
        assert_eq!(inf.diags[0].code, "E0400");
    }

    #[test]
    fn binds_and_resolves_a_var() {
        let mut inf = Infer::new();
        let v = inf.fresh();
        inf.unify(&v, &Ty::bool(), Span::EMPTY);
        assert!(inf.diags.is_empty());
        assert_eq!(inf.resolve(&v), Ty::bool());
    }

    #[test]
    fn occurs_check_is_e0401() {
        let mut inf = Infer::new();
        let v = inf.fresh();
        // unify a  with  (a) -> Int  => infinite type
        let f = Ty::Fn(vec![v.clone()], Box::new(Ty::int()));
        inf.unify(&v, &f, Span::EMPTY);
        assert_eq!(inf.diags.len(), 1);
        assert_eq!(inf.diags[0].code, "E0401");
    }

    #[test]
    fn function_arity_mismatch_is_e0402() {
        let mut inf = Infer::new();
        let a = Ty::Fn(vec![Ty::int()], Box::new(Ty::unit()));
        let b = Ty::Fn(vec![Ty::int(), Ty::int()], Box::new(Ty::unit()));
        inf.unify(&a, &b, Span::EMPTY);
        assert_eq!(inf.diags.len(), 1);
        assert_eq!(inf.diags[0].code, "E0402");
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --lib types`
Expected: FAIL — `Ty`/`Infer` not found.

- [ ] **Step 4: Write the implementation**

Prepend to `src/types.rs`:
```rust
//! Hindley–Milner type inference (Algorithm J).

use crate::diag::Diagnostic;
use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TyCon {
    Int,
    Float,
    Bool,
    Str,
    Unit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Var(u32),
    Base(TyCon),
    Fn(Vec<Ty>, Box<Ty>),
    Tuple(Vec<Ty>),
    /// Poison value that unifies with anything; suppresses cascade errors.
    Error,
}

impl Ty {
    pub fn int() -> Ty {
        Ty::Base(TyCon::Int)
    }
    pub fn float() -> Ty {
        Ty::Base(TyCon::Float)
    }
    pub fn bool() -> Ty {
        Ty::Base(TyCon::Bool)
    }
    pub fn str() -> Ty {
        Ty::Base(TyCon::Str)
    }
    pub fn unit() -> Ty {
        Ty::Base(TyCon::Unit)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scheme {
    pub vars: Vec<u32>,
    pub ty: Ty,
}

pub struct Infer {
    subst: Vec<Option<Ty>>,
    pub diags: Vec<Diagnostic>,
}

impl Infer {
    pub fn new() -> Infer {
        Infer {
            subst: Vec::new(),
            diags: Vec::new(),
        }
    }

    pub fn fresh(&mut self) -> Ty {
        let id = self.subst.len() as u32;
        self.subst.push(None);
        Ty::Var(id)
    }

    /// Follow bound variables to a representative, recursively (deep).
    pub fn resolve(&self, t: &Ty) -> Ty {
        match t {
            Ty::Var(v) => match &self.subst[*v as usize] {
                Some(bound) => self.resolve(bound),
                None => Ty::Var(*v),
            },
            Ty::Base(c) => Ty::Base(*c),
            Ty::Fn(ps, r) => Ty::Fn(
                ps.iter().map(|p| self.resolve(p)).collect(),
                Box::new(self.resolve(r)),
            ),
            Ty::Tuple(xs) => Ty::Tuple(xs.iter().map(|x| self.resolve(x)).collect()),
            Ty::Error => Ty::Error,
        }
    }

    fn occurs(&self, v: u32, t: &Ty) -> bool {
        match self.resolve(t) {
            Ty::Var(u) => u == v,
            Ty::Base(_) | Ty::Error => false,
            Ty::Fn(ps, r) => ps.iter().any(|p| self.occurs(v, p)) || self.occurs(v, &r),
            Ty::Tuple(xs) => xs.iter().any(|x| self.occurs(v, x)),
        }
    }

    fn bind(&mut self, v: u32, t: &Ty, span: Span) {
        if let Ty::Var(u) = t {
            if *u == v {
                return;
            }
        }
        if self.occurs(v, t) {
            self.diags.push(
                Diagnostic::error("E0401", "infinite type")
                    .with_label(span, "a type would have to contain itself"),
            );
            self.subst[v as usize] = Some(Ty::Error);
            return;
        }
        self.subst[v as usize] = Some(t.clone());
    }

    pub fn unify(&mut self, a: &Ty, b: &Ty, span: Span) {
        let a = self.resolve(a);
        let b = self.resolve(b);
        match (a, b) {
            (Ty::Error, _) | (_, Ty::Error) => {}
            (Ty::Var(x), Ty::Var(y)) if x == y => {}
            (Ty::Var(x), t) | (t, Ty::Var(x)) => self.bind(x, &t, span),
            (Ty::Base(x), Ty::Base(y)) if x == y => {}
            (Ty::Fn(p1, r1), Ty::Fn(p2, r2)) => {
                if p1.len() != p2.len() {
                    self.diags.push(
                        Diagnostic::error("E0402", "wrong number of arguments").with_label(
                            span,
                            format!("expected {} argument(s), found {}", p1.len(), p2.len()),
                        ),
                    );
                } else {
                    for (x, y) in p1.iter().zip(&p2) {
                        self.unify(x, y, span);
                    }
                    self.unify(&r1, &r2, span);
                }
            }
            (Ty::Tuple(x), Ty::Tuple(y)) if x.len() == y.len() => {
                for (p, q) in x.iter().zip(&y) {
                    self.unify(p, q, span);
                }
            }
            (x, y) => {
                self.diags.push(
                    Diagnostic::error("E0400", "type mismatch").with_label(
                        span,
                        format!("expected `{}`, found `{}`", show(&x), show(&y)),
                    ),
                );
            }
        }
    }
}

impl Default for Infer {
    fn default() -> Self {
        Infer::new()
    }
}

/// Minimal non-zonking type printer used inside `unify`'s messages. The
/// user-facing zonked/letter-named printer is added in Task 2.
fn show(t: &Ty) -> String {
    match t {
        Ty::Var(v) => format!("t{v}"),
        Ty::Base(TyCon::Int) => "Int".into(),
        Ty::Base(TyCon::Float) => "Float".into(),
        Ty::Base(TyCon::Bool) => "Bool".into(),
        Ty::Base(TyCon::Str) => "String".into(),
        Ty::Base(TyCon::Unit) => "Unit".into(),
        Ty::Fn(ps, r) => format!(
            "fn({}) -> {}",
            ps.iter().map(show).collect::<Vec<_>>().join(", "),
            show(r)
        ),
        Ty::Tuple(xs) => format!("({})", xs.iter().map(show).collect::<Vec<_>>().join(", ")),
        Ty::Error => "<error>".into(),
    }
}
```

> Note: `show` here can emit `t{v}` for *unresolved* variables, which would violate the no-`%t` rule if it reached a user. It does not: `unify` always `resolve`s both sides first, so a printed `Var` is genuinely unbound at that point — and Task 2 replaces user-facing rendering with the zonking/letter-naming printer. `show`'s `t{v}` is used only for the internal mismatch of two *base/shape* types, where neither side is a bare var. We tighten this in Task 2.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib types`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add src/types.rs src/lib.rs
git commit -m "feat(types): Ty/Scheme representation + unification with occurs-check (E0400/E0401/E0402)"
```

---

## Task 2: `types.rs` — the zonking, letter-naming type printer

**Files:**
- Modify: `src/types.rs`
- Test: inline tests

**Interfaces:**
- Produces:
  - `pub fn display_ty(inf: &Infer, t: &Ty) -> String` — resolves `t` and prints free vars as `a, b, c, …` (per call).
  - `pub fn display_scheme(inf: &Infer, s: &Scheme) -> String` — prints `forall a b. <ty>` (or just the type when there are no quantifiers).
  - Both guarantee: **no `%t`/`t<number>`/`%v` token in the output** — free variables become letters.

- [ ] **Step 1: Write the failing test**

Add to `src/types.rs` tests:
```rust
    #[test]
    fn printer_names_free_vars_with_letters() {
        let mut inf = Infer::new();
        let a = inf.fresh();
        let b = inf.fresh();
        let t = Ty::Fn(vec![a.clone()], Box::new(b.clone()));
        let out = display_ty(&inf, &t);
        assert_eq!(out, "fn(a) -> b");
        assert!(!out.contains('%'), "no internal token: {out}");
    }

    #[test]
    fn printer_reuses_same_letter_for_same_var() {
        let mut inf = Infer::new();
        let a = inf.fresh();
        let t = Ty::Fn(vec![a.clone()], Box::new(a.clone()));
        assert_eq!(display_ty(&inf, &t), "fn(a) -> a");
    }

    #[test]
    fn printer_resolves_bound_vars() {
        let mut inf = Infer::new();
        let a = inf.fresh();
        inf.unify(&a, &Ty::int(), Span::EMPTY);
        assert_eq!(display_ty(&inf, &a), "Int");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib types::tests::printer`
Expected: FAIL — `display_ty` not found.

- [ ] **Step 3: Write the implementation**

Add to `src/types.rs`:
```rust
use std::collections::HashMap;

pub fn display_ty(inf: &Infer, t: &Ty) -> String {
    let mut names: HashMap<u32, String> = HashMap::new();
    let resolved = inf.resolve(t);
    let mut out = String::new();
    write_ty(&resolved, &mut names, &mut out);
    out
}

pub fn display_scheme(inf: &Infer, s: &Scheme) -> String {
    let mut names: HashMap<u32, String> = HashMap::new();
    // Assign letters to the quantified vars first, in order, for stable output.
    for v in &s.vars {
        let n = letter(names.len());
        names.insert(*v, n);
    }
    let resolved = inf.resolve(&s.ty);
    let mut body = String::new();
    write_ty(&resolved, &mut names, &mut body);
    if s.vars.is_empty() {
        body
    } else {
        let quant: Vec<String> = s.vars.iter().map(|v| names[v].clone()).collect();
        format!("forall {}. {}", quant.join(" "), body)
    }
}

fn letter(i: usize) -> String {
    // 0->a, 25->z, 26->a1, ...
    let c = (b'a' + (i % 26) as u8) as char;
    if i < 26 {
        c.to_string()
    } else {
        format!("{c}{}", i / 26)
    }
}

fn write_ty(t: &Ty, names: &mut HashMap<u32, String>, out: &mut String) {
    match t {
        Ty::Var(v) => {
            let next = names.len();
            let name = names.entry(*v).or_insert_with(|| letter(next)).clone();
            out.push_str(&name);
        }
        Ty::Base(TyCon::Int) => out.push_str("Int"),
        Ty::Base(TyCon::Float) => out.push_str("Float"),
        Ty::Base(TyCon::Bool) => out.push_str("Bool"),
        Ty::Base(TyCon::Str) => out.push_str("String"),
        Ty::Base(TyCon::Unit) => out.push_str("Unit"),
        Ty::Fn(ps, r) => {
            out.push_str("fn(");
            for (i, p) in ps.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_ty(p, names, out);
            }
            out.push_str(") -> ");
            write_ty(r, names, out);
        }
        Ty::Tuple(xs) => {
            out.push('(');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_ty(x, names, out);
            }
            out.push(')');
        }
        Ty::Error => out.push_str("<error>"),
    }
}
```

Then **replace the mismatch arm** of `unify` (the `(x, y) => …` arm from Task 1) so the user-facing message uses the zonking printer and never emits `t<number>`:
```rust
            (x, y) => {
                let msg = format!("expected `{}`, found `{}`", display_ty(self, &x), display_ty(self, &y));
                self.diags
                    .push(Diagnostic::error("E0400", "type mismatch").with_label(span, msg));
            }
```
Delete the old private `fn show(...)` (now unused).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib types`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add src/types.rs
git commit -m "feat(types): zonking letter-naming type printer; mismatch messages use it (no %t)"
```

---

## Task 3: `types.rs` — expression inference (monomorphic)

**Files:**
- Modify: `src/types.rs`
- Test: inline tests

**Interfaces:**
- Produces:
  - `pub struct TyEnv` — a scope stack `Vec<HashMap<String, Scheme>>` with `new()`, `push()/pop()`, `insert(&mut, name, Scheme)`, `lookup(&self, name) -> Option<&Scheme>`.
  - `impl Infer { fn instantiate(&mut self, &Scheme) -> Ty; fn infer_expr(&mut self, &Spanned<Expr>, &mut TyEnv) -> Ty; fn infer_block(&mut self, &Block, &mut TyEnv) -> Ty; }`
  - `fn operator_type(op: BinOp) -> (Ty, Ty, Ty)` and unary equivalent (operand/result monotypes; §2.6).

- [ ] **Step 1: Write the failing test**

Add to `src/types.rs` tests:
```rust
    use crate::ast::*;
    use crate::parse::parse_expr_str;
    use crate::Session;

    fn infer_expr_str(src: &str) -> (String, usize) {
        let (e, d) = parse_expr_str(&Session::new(), src);
        assert!(d.is_empty(), "parse: {d:?}");
        let e = e.unwrap();
        let mut inf = Infer::new();
        let mut env = TyEnv::new();
        let t = inf.infer_expr(&e, &mut env);
        (display_ty(&inf, &t), inf.diags.len())
    }

    #[test]
    fn infers_arithmetic_and_comparison() {
        assert_eq!(infer_expr_str("1 + 2"), ("Int".into(), 0));
        assert_eq!(infer_expr_str("1.0 +. 2.0"), ("Float".into(), 0));
        assert_eq!(infer_expr_str("1 < 2"), ("Bool".into(), 0));
        assert_eq!(infer_expr_str(r#""a" <> "b""#), ("String".into(), 0));
    }

    #[test]
    fn mismatch_in_operator_is_e0400() {
        let (_t, n) = infer_expr_str(r#"1 + "a""#);
        assert_eq!(n, 1);
    }

    #[test]
    fn if_branches_must_agree_and_cond_is_bool() {
        assert_eq!(infer_expr_str("if 1 < 2 { 10 } else { 20 }"), ("Int".into(), 0));
        let (_t, n) = infer_expr_str("if 1 { 10 } else { 20 }"); // non-Bool cond
        assert_eq!(n, 1);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib types::tests::infers`
Expected: FAIL — `TyEnv`/`infer_expr` not found.

- [ ] **Step 3: Write the implementation**

Add to `src/types.rs`:
```rust
use crate::ast::{BinOp, Block, Expr, Stmt, UnOp};
use crate::span::Spanned;

#[derive(Default)]
pub struct TyEnv {
    scopes: Vec<HashMap<String, Scheme>>,
}

impl TyEnv {
    pub fn new() -> TyEnv {
        TyEnv {
            scopes: vec![HashMap::new()],
        }
    }
    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }
    fn pop(&mut self) {
        self.scopes.pop();
    }
    pub fn insert(&mut self, name: &str, s: Scheme) {
        self.scopes.last_mut().unwrap().insert(name.to_string(), s);
    }
    pub fn lookup(&self, name: &str) -> Option<&Scheme> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }
}

fn binop_type(op: BinOp) -> (Ty, Ty, Ty) {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Div | Rem => (Ty::int(), Ty::int(), Ty::int()),
        AddF | SubF | MulF | DivF => (Ty::float(), Ty::float(), Ty::float()),
        Lt | Le | Gt | Ge => (Ty::int(), Ty::int(), Ty::bool()),
        Concat => (Ty::str(), Ty::str(), Ty::str()),
        And | Or => (Ty::bool(), Ty::bool(), Ty::bool()),
        // Eq/Ne handled specially in infer_expr (both operands share a fresh var).
        Eq | Ne => (Ty::Error, Ty::Error, Ty::bool()),
    }
}

impl Infer {
    pub fn instantiate(&mut self, s: &Scheme) -> Ty {
        if s.vars.is_empty() {
            return s.ty.clone();
        }
        let mapping: HashMap<u32, Ty> = s.vars.iter().map(|v| (*v, self.fresh())).collect();
        subst_vars(&s.ty, &mapping)
    }

    pub fn infer_block(&mut self, b: &Block, env: &mut TyEnv) -> Ty {
        env.push();
        for st in &b.stmts {
            match &st.node {
                Stmt::Let { name, value } => {
                    let t = self.infer_expr(value, env);
                    // Local let-generalization (§2.4). Env free vars computed lazily.
                    let scheme = self.generalize(&t, env);
                    env.insert(name, scheme);
                }
                Stmt::Expr(e) => {
                    self.infer_expr(e, env);
                }
            }
        }
        let result = match &b.tail {
            Some(tail) => self.infer_expr(tail, env),
            None => Ty::unit(),
        };
        env.pop();
        result
    }

    pub fn infer_expr(&mut self, e: &Spanned<Expr>, env: &mut TyEnv) -> Ty {
        let span = e.span;
        match &e.node {
            Expr::Int(_) => Ty::int(),
            Expr::Float(_) => Ty::float(),
            Expr::Str(_) => Ty::str(),
            Expr::Bool(_) => Ty::bool(),
            Expr::Unit => Ty::unit(),
            Expr::Var(name) => match env.lookup(name) {
                Some(s) => {
                    let s = s.clone();
                    self.instantiate(&s)
                }
                None => Ty::Error, // unresolved names are E0200 from resolution; avoid cascade
            },
            Expr::Qualified { module, name } => {
                // Builtins are only meaningful as call callees; typed at the Call site.
                let _ = (module, name);
                Ty::Error
            }
            Expr::Unary { op, expr } => {
                let t = self.infer_expr(expr, env);
                let (operand, result) = match op {
                    UnOp::Neg => (Ty::int(), Ty::int()),
                    UnOp::Not => (Ty::bool(), Ty::bool()),
                };
                self.unify(&t, &operand, span);
                result
            }
            Expr::Binary { op, lhs, rhs } => {
                let lt = self.infer_expr(lhs, env);
                let rt = self.infer_expr(rhs, env);
                if matches!(op, BinOp::Eq | BinOp::Ne) {
                    self.unify(&lt, &rt, span);
                    Ty::bool()
                } else {
                    let (l, r, res) = binop_type(*op);
                    self.unify(&lt, &l, span);
                    self.unify(&rt, &r, span);
                    res
                }
            }
            Expr::If { cond, then_block, else_block } => {
                let ct = self.infer_expr(cond, env);
                self.unify_cond(&ct, cond.span);
                let tt = self.infer_block(&then_block.node, env);
                let et = self.infer_block(&else_block.node, env);
                self.unify(&tt, &et, span);
                tt
            }
            Expr::Block(b) => self.infer_block(b, env),
            Expr::Call { callee, args } => self.infer_call(callee, args, span, env),
        }
    }

    fn unify_cond(&mut self, t: &Ty, span: Span) {
        // Dedicated E0403 rather than a generic mismatch when a condition isn't Bool.
        let r = self.resolve(t);
        if r != Ty::bool() && r != Ty::Error && !matches!(r, Ty::Var(_)) {
            self.diags.push(
                Diagnostic::error("E0403", "condition must be `Bool`")
                    .with_label(span, format!("this is `{}`", display_ty(self, &r))),
            );
        } else {
            self.unify(t, &Ty::bool(), span);
        }
    }

    fn infer_call(
        &mut self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
        span: Span,
        env: &mut TyEnv,
    ) -> Ty {
        // Builtin callee: io.println : (String) -> Unit
        if let Expr::Qualified { module, name } = &callee.node {
            if module == "io" && name == "println" {
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env)).collect();
                let want = Ty::Fn(vec![Ty::str()], Box::new(Ty::unit()));
                let got = Ty::Fn(arg_ts, Box::new(Ty::unit()));
                self.unify(&want, &got, span);
                return Ty::unit();
            }
            return Ty::Error; // unknown builtin is E0201 from resolution
        }
        let f = self.infer_expr(callee, env);
        let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env)).collect();
        let result = self.fresh();
        let expected = Ty::Fn(arg_ts, Box::new(result.clone()));
        self.unify(&f, &expected, span);
        result
    }

    /// Placeholder generalization; the real one lands in Task 5.
    fn generalize(&mut self, t: &Ty, _env: &TyEnv) -> Scheme {
        Scheme {
            vars: Vec::new(),
            ty: self.resolve(t),
        }
    }
}

fn subst_vars(t: &Ty, m: &HashMap<u32, Ty>) -> Ty {
    match t {
        Ty::Var(v) => m.get(v).cloned().unwrap_or(Ty::Var(*v)),
        Ty::Base(c) => Ty::Base(*c),
        Ty::Fn(ps, r) => Ty::Fn(
            ps.iter().map(|p| subst_vars(p, m)).collect(),
            Box::new(subst_vars(r, m)),
        ),
        Ty::Tuple(xs) => Ty::Tuple(xs.iter().map(|x| subst_vars(x, m)).collect()),
        Ty::Error => Ty::Error,
    }
}
```

> The `generalize` here is a monomorphic placeholder (`vars: []`); Task 5 replaces it with the real free-vars computation. Keeping the signature stable now avoids churn.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib types`
Expected: PASS (all types tests).

- [ ] **Step 5: Commit**

```bash
git add src/types.rs
git commit -m "feat(types): monomorphic expression inference (operators, if/E0403, calls, builtins)"
```

---

## Task 4: `types.rs` — module inference entry, wired into `check_source`

**Files:**
- Modify: `src/types.rs`, `src/lib.rs`
- Test: inline tests + `tests/types.rs` (new)

**Interfaces:**
- Produces: `pub fn infer(session: &Session, module: &Module) -> Vec<Diagnostic>` — types the whole module as a **single monomorphic group** for now (Task 5 upgrades to SCC/polymorphism). Also `pub fn infer_schemes(session, module) -> (Vec<(String, String)>, Vec<Diagnostic>)` returning each top-level function's pretty-printed scheme (for tests/snapshots).
- Modifies `lib.rs::check_source` to run `types::infer` after resolution.

- [ ] **Step 1: Write the failing test**

Create `tests/types.rs`:
```rust
use elya::Session;

fn type_diags(src: &str) -> Vec<String> {
    let (m, d) = elya::parse::parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    elya::types::infer(&Session::new(), &m)
        .into_iter()
        .map(|x| x.code)
        .collect()
}

#[test]
fn well_typed_module_has_no_type_errors() {
    let src = "fn double(x) { x + x }\npub fn main() { let _ = double(21)\n io.println(\"ok\") }\n";
    assert!(type_diags(src).is_empty(), "{:?}", type_diags(src));
}

#[test]
fn ill_typed_operator_is_e0400() {
    let src = "pub fn main() { let _ = 1 + \"a\"\n io.println(\"x\") }\n";
    assert_eq!(type_diags(src), vec!["E0400".to_string()]);
}
```

Add to `src/types.rs` tests a direct check that `infer` types `main`:
```rust
    #[test]
    fn module_infer_types_a_function() {
        let (m, d) = crate::parse::parse_module(&Session::new(), "fn f(x) { x + 1 }\n");
        assert!(d.is_empty());
        let diags = infer(&Session::new(), &m);
        assert!(diags.is_empty(), "{diags:?}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib types::tests::module_infer` then `cargo test --test types`
Expected: FAIL — `infer` not found.

- [ ] **Step 3: Write the implementation**

Add to `src/types.rs`:
```rust
use crate::ast::{Decl, Module};
use crate::Session;

pub fn infer(session: &Session, module: &Module) -> Vec<Diagnostic> {
    let (_schemes, diags) = infer_schemes(session, module);
    diags
}

pub fn infer_schemes(_session: &Session, module: &Module) -> (Vec<(String, String)>, Vec<Diagnostic>) {
    let mut inf = Infer::new();
    let mut env = TyEnv::new();

    // Single monomorphic group for now (Task 5 replaces with SCC ordering):
    // assign each top-level function a fresh Fn type with fresh params + result.
    let mut fn_tys: Vec<(String, Vec<Ty>, Ty)> = Vec::new();
    for d in &module.decls {
        let Decl::Fn(f) = &d.node;
        let params: Vec<Ty> = f.params.iter().map(|_| inf.fresh()).collect();
        let result = inf.fresh();
        env.insert(
            &f.name,
            Scheme {
                vars: Vec::new(),
                ty: Ty::Fn(params.clone(), Box::new(result.clone())),
            },
        );
        fn_tys.push((f.name.clone(), params, result));
    }

    // Infer each body under the params, unifying the inferred body type with the result var.
    for (d, (_, params, result)) in module.decls.iter().zip(&fn_tys) {
        let Decl::Fn(f) = &d.node;
        env.push();
        for (p, pty) in f.params.iter().zip(params) {
            env.insert(
                &p.node.name,
                Scheme {
                    vars: Vec::new(),
                    ty: pty.clone(),
                },
            );
        }
        let body_ty = inf.infer_block(&f.body.node, &mut env);
        inf.unify(&body_ty, result, f.body.span);
        env.pop();
    }

    let schemes = fn_tys
        .iter()
        .map(|(name, params, result)| {
            let fnty = Ty::Fn(params.clone(), Box::new(result.clone()));
            let s = Scheme {
                vars: Vec::new(),
                ty: inf.resolve(&fnty),
            };
            (name.clone(), display_scheme(&inf, &s))
        })
        .collect();

    (schemes, inf.diags)
}
```

In `src/lib.rs`, change `check_source` (and, later, `run_source`) to call inference after resolution:
```rust
pub fn check_source(name: &str, text: &str) -> Result<(), String> {
    let session = Session::new();
    let sm = SourceMap::new(name, text);
    let (module, mut diags) = parse::parse_module(&session, text);
    diags.extend(resolve::check(&session, &module));
    if diags.is_empty() {
        diags.extend(types::infer(&session, &module));
    }
    match fail_if_errors(&diags, &sm) {
        Some(rendered) => Err(rendered),
        None => Ok(()),
    }
}
```

> `run_source` is switched to `infer` + CEK in Task 11; leave it on the tree-walker for now so the crate stays green between tasks.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib types` and `cargo test --test types`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/types.rs src/lib.rs tests/types.rs
git commit -m "feat(types): module inference (monomorphic group) wired into check_source"
```

---

## Task 5: `types.rs` — generalization, instantiation, and SCC-grouped polymorphism

**Files:**
- Modify: `src/types.rs`
- Test: inline tests + `tests/types.rs`

**Interfaces:**
- Produces: real `generalize` (free-vars of `t` minus free-vars of the environment); `infer_schemes` reworked to process top-level functions in **SCC dependency order** (Tarjan), generalizing each group before later groups use it. Helper `free_vars(&Infer, &Ty, &mut Vec<u32>)` and `env_free_vars(&Infer, &TyEnv, &mut Vec<u32>)`.

- [ ] **Step 1: Write the failing test**

Add to `tests/types.rs`:
```rust
fn schemes(src: &str) -> std::collections::HashMap<String, String> {
    let (m, d) = elya::parse::parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let (ss, diags) = elya::types::infer_schemes(&Session::new(), &m);
    assert!(diags.is_empty(), "type: {diags:?}");
    ss.into_iter().collect()
}

#[test]
fn identity_generalizes() {
    let s = schemes("fn id(x) { x }\n");
    assert_eq!(s["id"], "forall a. fn(a) -> a");
}

#[test]
fn const_generalizes_two_vars() {
    let s = schemes("fn first(x, y) { x }\n");
    assert_eq!(s["first"], "forall a b. fn(a, b) -> a");
}

#[test]
fn identity_used_at_two_types_typechecks() {
    // `id` in its own SCC generalizes, so main may use it at Int and String.
    let src = "fn id(x) { x }\npub fn main() { let _ = id(1)\n io.println(id(\"hi\")) }\n";
    let (m, d) = elya::parse::parse_module(&Session::new(), src);
    assert!(d.is_empty());
    let diags = elya::types::infer(&Session::new(), &m);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn mutual_recursion_typechecks() {
    let src = "fn even(n) { if n == 0 { True } else { odd(n - 1) } }\n\
               fn odd(n) { if n == 0 { False } else { even(n - 1) } }\n";
    let (m, d) = elya::parse::parse_module(&Session::new(), src);
    assert!(d.is_empty());
    assert!(elya::types::infer(&Session::new(), &m).is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test types`
Expected: FAIL — `id` infers a monomorphic type (placeholder generalize), so `identity_used_at_two_types` errors and `identity_generalizes` mismatches.

- [ ] **Step 3: Replace `generalize` and rework `infer_schemes`**

Replace the placeholder `generalize` (from Task 3) with:
```rust
    fn generalize(&mut self, t: &Ty, env: &TyEnv) -> Scheme {
        let resolved = self.resolve(t);
        let mut in_ty = Vec::new();
        free_vars(self, &resolved, &mut in_ty);
        let mut in_env = Vec::new();
        env_free_vars(self, env, &mut in_env);
        let vars: Vec<u32> = in_ty
            .into_iter()
            .filter(|v| !in_env.contains(v))
            .collect();
        Scheme { ty: resolved, vars }
    }
```

Add free-var helpers:
```rust
fn free_vars(inf: &Infer, t: &Ty, acc: &mut Vec<u32>) {
    match inf.resolve(t) {
        Ty::Var(v) => {
            if !acc.contains(&v) {
                acc.push(v);
            }
        }
        Ty::Base(_) | Ty::Error => {}
        Ty::Fn(ps, r) => {
            for p in &ps {
                free_vars(inf, p, acc);
            }
            free_vars(inf, &r, acc);
        }
        Ty::Tuple(xs) => {
            for x in &xs {
                free_vars(inf, x, acc);
            }
        }
    }
}

fn env_free_vars(inf: &Infer, env: &TyEnv, acc: &mut Vec<u32>) {
    for scope in &env.scopes {
        for scheme in scope.values() {
            let mut fv = Vec::new();
            free_vars(inf, &scheme.ty, &mut fv);
            for v in fv {
                if !scheme.vars.contains(&v) && !acc.contains(&v) {
                    acc.push(v);
                }
            }
        }
    }
}
```
(Make `TyEnv::scopes` visible to these module-level helpers — they are in the same module, so no change is needed beyond the field already being private-to-module.)

Rework `infer_schemes` to process SCCs in dependency order:
```rust
pub fn infer_schemes(_session: &Session, module: &Module) -> (Vec<(String, String)>, Vec<Diagnostic>) {
    let mut inf = Infer::new();
    let mut env = TyEnv::new();

    // Index the functions and build the call graph over top-level names.
    let fns: Vec<&crate::ast::FnDecl> = module
        .decls
        .iter()
        .map(|d| {
            let Decl::Fn(f) = &d.node;
            f
        })
        .collect();
    let name_idx: HashMap<&str, usize> = fns
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.as_str(), i))
        .collect();
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); fns.len()];
    for (i, f) in fns.iter().enumerate() {
        let mut refs = Vec::new();
        collect_refs(&f.body.node, &mut refs);
        for r in refs {
            if let Some(&j) = name_idx.get(r.as_str()) {
                if !edges[i].contains(&j) {
                    edges[i].push(j);
                }
            }
        }
    }
    let groups = tarjan_scc(&edges); // Vec<Vec<usize>> in reverse-topological (leaves first) order

    let mut schemes_out: Vec<(String, String)> = Vec::new();

    for group in &groups {
        // 1. fresh monotype per member, in scope for the whole group (monomorphic recursion)
        let mut member_ty: HashMap<usize, (Vec<Ty>, Ty)> = HashMap::new();
        for &i in group {
            let f = fns[i];
            let params: Vec<Ty> = f.params.iter().map(|_| inf.fresh()).collect();
            let result = inf.fresh();
            env.insert(
                &f.name,
                Scheme {
                    vars: Vec::new(),
                    ty: Ty::Fn(params.clone(), Box::new(result.clone())),
                },
            );
            member_ty.insert(i, (params, result));
        }
        // 2. infer each body under its params
        for &i in group {
            let f = fns[i];
            let (params, result) = &member_ty[&i];
            env.push();
            for (p, pty) in f.params.iter().zip(params) {
                env.insert(
                    &p.node.name,
                    Scheme {
                        vars: Vec::new(),
                        ty: pty.clone(),
                    },
                );
            }
            let body_ty = inf.infer_block(&f.body.node, &mut env);
            inf.unify(&body_ty, result, f.body.span);
            env.pop();
        }
        // 3. generalize each member and re-insert its polytype for later groups
        for &i in group {
            let f = fns[i];
            let (params, result) = &member_ty[&i];
            let fnty = Ty::Fn(params.clone(), Box::new(result.clone()));
            // generalize against the environment MINUS this group's own monotypes:
            // pop the group's monotypes first so they are not treated as env-bound.
            let scheme = generalize_toplevel(&mut inf, &fnty, &env, group, &fns);
            env.insert(&f.name, scheme.clone());
            schemes_out.push((f.name.clone(), display_scheme(&inf, &scheme)));
        }
    }

    (schemes_out, inf.diags)
}
```

Add the helpers `collect_refs`, `tarjan_scc`, and `generalize_toplevel`:
```rust
fn collect_refs(b: &Block, acc: &mut Vec<String>) {
    for st in &b.stmts {
        match &st.node {
            Stmt::Let { value, .. } => collect_refs_expr(&value.node, acc),
            Stmt::Expr(e) => collect_refs_expr(&e.node, acc),
        }
    }
    if let Some(t) = &b.tail {
        collect_refs_expr(&t.node, acc);
    }
}

fn collect_refs_expr(e: &Expr, acc: &mut Vec<String>) {
    match e {
        Expr::Var(n) => acc.push(n.clone()),
        Expr::Call { callee, args } => {
            collect_refs_expr(&callee.node, acc);
            for a in args {
                collect_refs_expr(&a.node, acc);
            }
        }
        Expr::Unary { expr, .. } => collect_refs_expr(&expr.node, acc),
        Expr::Binary { lhs, rhs, .. } => {
            collect_refs_expr(&lhs.node, acc);
            collect_refs_expr(&rhs.node, acc);
        }
        Expr::If { cond, then_block, else_block } => {
            collect_refs_expr(&cond.node, acc);
            collect_refs(&then_block.node, acc);
            collect_refs(&else_block.node, acc);
        }
        Expr::Block(b) => collect_refs(b, acc),
        _ => {}
    }
}

/// Tarjan's SCC. Returns components; the natural output order (components
/// finalized as recursion unwinds) is reverse-topological — callees before
/// callers — which is exactly the order we want to generalize in.
fn tarjan_scc(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = edges.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut counter = 0usize;
    let mut out: Vec<Vec<usize>> = Vec::new();

    // Iterative Tarjan to avoid host recursion on large modules.
    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&(v, mut pi)) = call.last() {
            if pi == 0 {
                index[v] = counter;
                low[v] = counter;
                counter += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            let mut recursed = false;
            while pi < edges[v].len() {
                let w = edges[v][pi];
                pi += 1;
                if index[w] == usize::MAX {
                    call.last_mut().unwrap().1 = pi;
                    call.push((w, 0));
                    recursed = true;
                    break;
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            }
            if recursed {
                continue;
            }
            call.last_mut().unwrap().1 = pi;
            if low[v] == index[v] {
                let mut comp = Vec::new();
                loop {
                    let w = stack.pop().unwrap();
                    on_stack[w] = false;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                out.push(comp);
            }
            call.pop();
            if let Some(&(parent, _)) = call.last() {
                low[parent] = low[parent].min(low[v]);
            }
        }
    }
    out
}

fn generalize_toplevel(
    inf: &mut Infer,
    fnty: &Ty,
    env: &TyEnv,
    group: &[usize],
    fns: &[&crate::ast::FnDecl],
) -> Scheme {
    // Environment free vars excluding this group's own bindings (their monotypes
    // must be generalizable, not treated as env-fixed).
    let group_names: Vec<&str> = group.iter().map(|&i| fns[i].name.as_str()).collect();
    let resolved = inf.resolve(fnty);
    let mut in_ty = Vec::new();
    free_vars(inf, &resolved, &mut in_ty);
    let mut in_env = Vec::new();
    for scope in &env.scopes {
        for (n, scheme) in scope {
            if group_names.contains(&n.as_str()) {
                continue;
            }
            let mut fv = Vec::new();
            free_vars(inf, &scheme.ty, &mut fv);
            for v in fv {
                if !scheme.vars.contains(&v) && !in_env.contains(&v) {
                    in_env.push(v);
                }
            }
        }
    }
    let vars: Vec<u32> = in_ty.into_iter().filter(|v| !in_env.contains(v)).collect();
    Scheme { ty: resolved, vars }
}
```

> **Correctness note for the implementer:** the SCC output order from this iterative Tarjan is callees-before-callers (reverse topological), which is the order generalization requires — a function's SCC is generalized before any caller's SCC uses it. The `identity_used_at_two_types` test is the guard: if the ordering or generalization is wrong, `id` stays monomorphic and that test fails.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib types` and `cargo test --test types`
Expected: PASS (including `identity_generalizes`, `const_generalizes_two_vars`, `identity_used_at_two_types_typechecks`, `mutual_recursion_typechecks`).

- [ ] **Step 5: Commit**

```bash
git add src/types.rs tests/types.rs
git commit -m "feat(types): generalization + instantiation + SCC-ordered top-level polymorphism"
```

---

## Task 6: type-error UI fixtures + inferred-scheme snapshots

**Files:**
- Create: `tests/ui/type_mismatch.elya`, `tests/ui/occurs_check.elya`, `tests/ui/bad_arity.elya`, `tests/ui/non_bool_cond.elya`
- Create: `tests/type_schemes.rs`
- Test: extends the existing `tests/ui.rs` harness (already checks `//~ ERROR[…]` + the no-`%r/%e/%s` invariant); add a no-`%t` assertion there.

**Interfaces:**
- Consumes: `elya::check_source` (routes through `types::infer`), `elya::types::infer_schemes`.

- [ ] **Step 1: Write the fixtures**

`tests/ui/type_mismatch.elya`:
```elya
pub fn main() {
  let _ = 1 + "a"
  io.println("x")
}
//~ ERROR[E0400] type mismatch
```
`tests/ui/occurs_check.elya`:
```elya
fn f(x) {
  x(x)
}
//~ ERROR[E0401] infinite type
```
`tests/ui/bad_arity.elya`:
```elya
fn add(a, b) {
  a + b
}
pub fn main() {
  let _ = add(1)
  io.println("x")
}
//~ ERROR[E0402] wrong number of arguments
```
`tests/ui/non_bool_cond.elya`:
```elya
pub fn main() {
  let _ = if 1 { 2 } else { 3 }
  io.println("x")
}
//~ ERROR[E0403] condition must be
```

- [ ] **Step 2: Extend the UI harness with a no-`%t` check**

In `tests/ui.rs`, add `%t` and `%v` to the banned-token loop:
```rust
    for bad in ["%r", "%e", "%s", "%t", "%v"] {
```
Then register the four new fixtures:
```rust
#[test]
fn type_mismatch() { check_fixture("type_mismatch.elya"); }
#[test]
fn occurs_check() { check_fixture("occurs_check.elya"); }
#[test]
fn bad_arity() { check_fixture("bad_arity.elya"); }
#[test]
fn non_bool_cond() { check_fixture("non_bool_cond.elya"); }
```

- [ ] **Step 3: Write the scheme-snapshot test**

`tests/type_schemes.rs`:
```rust
use elya::Session;

fn schemes_text(src: &str) -> String {
    let (m, d) = elya::parse::parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let (ss, diags) = elya::types::infer_schemes(&Session::new(), &m);
    assert!(diags.is_empty(), "type: {diags:?}");
    let mut lines: Vec<String> = ss.into_iter().map(|(n, s)| format!("{n} : {s}")).collect();
    lines.sort();
    lines.join("\n")
}

#[test]
fn polymorphic_schemes() {
    insta::assert_snapshot!(
        "poly_schemes",
        schemes_text("fn id(x) { x }\nfn first(x, y) { x }\nfn twice(n) { n + n }\n")
    );
}
```

- [ ] **Step 4: Run tests, review snapshot, verify pass**

Run: `cargo test --test ui`
Expected: PASS (8 UI tests: 4 prior + 4 new).
Run: `INSTA_UPDATE=always cargo test --test type_schemes` then inspect `tests/snapshots/type_schemes__poly_schemes.snap` — it must read:
```
first : forall a b. fn(a, b) -> a
id : forall a. fn(a) -> a
twice : fn(Int) -> Int
```
Then `cargo test --test type_schemes` → PASS.

- [ ] **Step 5: Commit**

```bash
git add tests/ui.rs tests/ui/type_mismatch.elya tests/ui/occurs_check.elya tests/ui/bad_arity.elya tests/ui/non_bool_cond.elya tests/type_schemes.rs tests/snapshots
git commit -m "test(types): E0400-E0403 UI fixtures (no-%t invariant) + inferred-scheme snapshots"
```

---

## Task 7: `eval.rs` — persistent `Rc`-scope environment + closures (refactor, behavior-preserving)

**Files:**
- Modify: `src/eval.rs`
- Test: existing `src/eval.rs` tests must stay green

**Interfaces:**
- Produces:
  - `pub struct Env(Option<Rc<Scope>>)` with `Env::new()`, `extend(&self, &[(String, Value)]) -> Env`, `get(&self, &str) -> Option<Value>`.
  - `Value::Closure(Rc<FnDecl>, Env)` replaces `Value::Func(Rc<FnDecl>)`.
  - The Slice-1 tree-walker is moved under `pub mod tree` with `pub fn run_module(&Module) -> Result<Interp, RuntimeError>` reading/writing through the new `Env`. `Interp`, `Value`, `RuntimeError`, `apply_binop`, `apply_unop` become shared items at the module top.

- [ ] **Step 1: Introduce the persistent `Env` and adjust `Value` (compile-first)**

Replace the current `Env` definition and `Value::Func` with:
```rust
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Unit,
    Closure(Rc<FnDecl>, Env),
}

#[derive(Clone, Debug, PartialEq)]
struct Scope {
    vars: HashMap<String, Value>,
    parent: Option<Rc<Scope>>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Env(Option<Rc<Scope>>);

impl Env {
    pub fn new() -> Env {
        Env(None)
    }

    /// Return a new environment with one added scope holding `bindings`.
    pub fn extend(&self, bindings: &[(String, Value)]) -> Env {
        let mut vars = HashMap::with_capacity(bindings.len());
        for (k, v) in bindings {
            vars.insert(k.clone(), v.clone());
        }
        Env(Some(Rc::new(Scope {
            vars,
            parent: self.0.clone(),
        })))
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        let mut cur = self.0.as_deref();
        while let Some(scope) = cur {
            if let Some(v) = scope.vars.get(name) {
                return Some(v.clone());
            }
            cur = scope.parent.as_deref();
        }
        None
    }
}
```

- [ ] **Step 2: Move the tree-walker into `mod tree` and adapt it to `Env`**

Wrap the existing `Interp` walker methods in `pub mod tree { ... }`, replacing every `env.child()`+`define` sequence with a single `env.extend(&[...])`, and every closure creation with `Value::Closure(fn, defining_env)`. The shared items (`Value`, `Env`, `RuntimeError`, `rt`, `apply_binop`, `apply_unop`) stay at the module top; only the walker moves into `tree`. Public entry: `eval::tree::run_module`. Rename the current binop function to `apply_binop` and add `apply_unop`:
```rust
pub(crate) fn apply_binop(op: BinOp, l: Value, r: Value, span: Span) -> Result<Value, RuntimeError> {
    /* body identical to Slice 1's eval_binop */
}
pub(crate) fn apply_unop(op: UnOp, v: Value, span: Span) -> Result<Value, RuntimeError> {
    match (op, v) {
        (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(-n)),
        (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
        (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
        _ => Err(rt(span, "type error in unary operator")),
    }
}
```
In the tree-walker, application becomes:
```rust
// closure call:
let mut bindings = Vec::with_capacity(func.params.len());
for (p, a) in func.params.iter().zip(args) {
    let v = self.eval_expr(&a.node, a.span, env)?;
    bindings.push((p.node.name.clone(), v));
}
let call_env = closure_env.extend(&bindings);
self.eval_block(&func.body.node, &call_env)
```
where `closure_env` comes from the matched `Value::Closure(func, closure_env)`. Top-level functions are installed as `Value::Closure(Rc::new(f.clone()), globals_env.clone())`.

- [ ] **Step 3: Point the existing tests at `tree::run_module`**

The `src/eval.rs` test helper `run` becomes `eval::tree::run_module`. Keep `eval::run_module` temporarily aliased to `tree::run_module` (so `lib.rs` still compiles) — Task 8 repoints it to the CEK machine:
```rust
pub fn run_module(module: &Module) -> Result<Interp, RuntimeError> {
    tree::run_module(module)
}
pub fn run_module_tree(module: &Module) -> Result<Interp, RuntimeError> {
    tree::run_module(module)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib eval` then `cargo test --all`
Expected: PASS — all Slice-1 eval tests and example/tce/ui tests still green (behavior preserved; only `Env` representation changed).

- [ ] **Step 5: Commit**

```bash
git add src/eval.rs
git commit -m "refactor(eval): persistent Rc-scope Env + closures; tree-walker retained as eval::tree"
```

---

## Task 8: `eval.rs` — the CEK machine

**Files:**
- Modify: `src/eval.rs`
- Test: inline tests in the new `cek` submodule

**Interfaces:**
- Produces: `pub mod cek` with `pub fn run_module(&Module) -> Result<Interp, RuntimeError>` and internal `State`/`Frame`/`Kont`/`step`. `eval::run_module` repointed to `cek::run_module`. `Interp` gains `pub fn peak_kont_depth(&self) -> usize` (populated by the CEK machine; the tree-walker leaves it 0).

> **Implementer note (lifetimes):** the CEK machine holds **borrowed** AST references (`&'a`) into the `Module` for the duration of `run_module`; `Kont` is a persistent `Rc`-cons list of frames. This is efficient and gives the persistence Slice 3 needs *structurally*. Capturing a continuation *into a heap `Value`* (Slice 3 `resume`) will additionally require the referenced AST to be `'static`; Slice 3 achieves that by sharing the AST via `Rc` — a mechanical change, not a machine rewrite. Not needed in Slice 2.

- [ ] **Step 1: Write the failing test**

Add at the bottom of `src/eval.rs`:
```rust
#[cfg(test)]
mod cek_tests {
    use super::*;
    use crate::parse::parse_module;
    use crate::Session;

    fn run(src: &str) -> String {
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "parse: {d:?}");
        cek::run_module(&m).unwrap().output().to_string()
    }

    #[test]
    fn hello_world_on_cek() {
        assert_eq!(
            run("pub fn main() { io.println(\"Hello, Elya!\") }\n"),
            "Hello, Elya!\n"
        );
    }

    #[test]
    fn arithmetic_functions_if_on_cek() {
        let src = "fn double(x) { x + x }\npub fn main() { \
            let a = double(20)\n let b = a + 2\n \
            if b == 42 { io.println(\"forty-two\") } else { io.println(\"nope\") } }\n";
        assert_eq!(run(src), "forty-two\n");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib cek_tests`
Expected: FAIL — `cek::run_module` not found.

- [ ] **Step 3: Write the CEK machine**

Add to `src/eval.rs`:
```rust
pub mod cek {
    use super::{apply_binop, apply_unop, rt, Interp, RuntimeError, Value};
    use crate::ast::*;
    use crate::span::{Span, Spanned};
    use std::rc::Rc;

    type Env = super::Env;

    enum Frame<'a> {
        BinRight { op: BinOp, rhs: &'a Spanned<Expr>, env: Env, span: Span },
        BinApply { op: BinOp, lval: Value, span: Span },
        UnApply { op: UnOp, span: Span },
        IfBranch { then_blk: &'a Block, else_blk: &'a Block, env: Env, span: Span },
        LetCont { name: &'a str, rest: &'a [Spanned<Stmt>], tail: Option<&'a Spanned<Expr>>, env: Env },
        SeqDrop { rest: &'a [Spanned<Stmt>], tail: Option<&'a Spanned<Expr>>, env: Env },
        CallArgs {
            callee: Option<Value>,
            done: Vec<Value>,
            pending: &'a [Spanned<Expr>],
            env: Env,
            span: Span,
        },
    }

    type Kont<'a> = Option<Rc<(Frame<'a>, Kont<'a>)>>;

    fn push<'a>(f: Frame<'a>, k: Kont<'a>) -> Kont<'a> {
        Some(Rc::new((f, k)))
    }

    enum State<'a> {
        Eval(&'a Spanned<Expr>, Env, Kont<'a>),
        Return(Value, Kont<'a>),
    }

    pub fn run_module(module: &Module) -> Result<Interp, RuntimeError> {
        let mut interp = Interp::new();
        // Install top-level functions as closures over the globals env.
        let mut bindings = Vec::new();
        for d in &module.decls {
            let Decl::Fn(f) = &d.node;
            bindings.push((f.name.clone(), Value::Closure(Rc::new(f.clone()), Env::new())));
        }
        let globals = Env::new().extend(&bindings);
        // Re-close each function over `globals` so recursion resolves.
        let globals = reclose(&globals, module);

        let main = module.decls.iter().find_map(|d| {
            let Decl::Fn(f) = &d.node;
            (f.name == "main").then(|| f.clone())
        });
        let Some(main) = main else {
            return Err(rt(Span::EMPTY, "no `main` function found"));
        };
        let body = &main.body.node;
        let start = eval_block(body, &globals, None);
        run_loop(&mut interp, start)?;
        Ok(interp)
    }

    // Rebind each top-level closure so its captured env is the fully-populated globals.
    fn reclose(globals: &Env, module: &Module) -> Env {
        let mut bindings = Vec::new();
        for d in &module.decls {
            let Decl::Fn(f) = &d.node;
            bindings.push((f.name.clone(), Value::Closure(Rc::new(f.clone()), globals.clone())));
        }
        globals.extend(&bindings)
    }

    // A block's tail is in tail position: evaluating it does not add a frame.
    fn eval_block<'a>(b: &'a Block, env: &Env, k: Kont<'a>) -> State<'a> {
        step_block(&b.stmts, b.tail.as_deref(), env.clone(), k)
    }

    fn step_block<'a>(
        stmts: &'a [Spanned<Stmt>],
        tail: Option<&'a Spanned<Expr>>,
        env: Env,
        k: Kont<'a>,
    ) -> State<'a> {
        match stmts.split_first() {
            None => match tail {
                Some(t) => State::Eval(t, env, k), // tail position — no frame
                None => State::Return(Value::Unit, k),
            },
            Some((st, rest)) => match &st.node {
                Stmt::Let { name, value } => State::Eval(
                    value,
                    env.clone(),
                    push(Frame::LetCont { name, rest, tail, env }, k),
                ),
                Stmt::Expr(e) => {
                    State::Eval(e, env.clone(), push(Frame::SeqDrop { rest, tail, env }, k))
                }
            },
        }
    }

    fn run_loop(interp: &mut Interp, mut st: State) -> Result<(), RuntimeError> {
        loop {
            interp.note_kont(&kont_of(&st));
            st = match step(interp, st)? {
                Some(next) => next,
                None => return Ok(()),
            };
        }
    }

    fn kont_len(k: &Kont) -> usize {
        let mut n = 0;
        let mut cur = k;
        while let Some(node) = cur {
            n += 1;
            cur = &node.1;
        }
        n
    }

    fn kont_of<'a, 'b>(st: &'b State<'a>) -> &'b Kont<'a> {
        match st {
            State::Eval(_, _, k) => k,
            State::Return(_, k) => k,
        }
    }

    fn step<'a>(interp: &mut Interp, st: State<'a>) -> Result<Option<State<'a>>, RuntimeError> {
        Ok(match st {
            State::Eval(e, env, k) => Some(eval(e, env, k)?),
            State::Return(v, k) => ret(interp, v, k)?,
        })
    }

    fn eval<'a>(e: &'a Spanned<Expr>, env: Env, k: Kont<'a>) -> Result<State<'a>, RuntimeError> {
        let span = e.span;
        Ok(match &e.node {
            Expr::Int(n) => State::Return(Value::Int(*n), k),
            Expr::Float(x) => State::Return(Value::Float(*x), k),
            Expr::Str(s) => State::Return(Value::Str(s.clone()), k),
            Expr::Bool(b) => State::Return(Value::Bool(*b), k),
            Expr::Unit => State::Return(Value::Unit, k),
            Expr::Var(name) => {
                let v = env.get(name).ok_or_else(|| rt(span, format!("unbound variable `{name}`")))?;
                State::Return(v, k)
            }
            Expr::Qualified { module, name } => {
                return Err(rt(span, format!("`{module}.{name}` must be called")))
            }
            Expr::Unary { op, expr } => {
                State::Eval(expr, env, push(Frame::UnApply { op: *op, span }, k))
            }
            Expr::Binary { op, lhs, rhs } => State::Eval(
                lhs,
                env.clone(),
                push(Frame::BinRight { op: *op, rhs, env, span }, k),
            ),
            Expr::If { cond, then_block, else_block } => State::Eval(
                cond,
                env.clone(),
                push(
                    Frame::IfBranch { then_blk: &then_block.node, else_blk: &else_block.node, env, span },
                    k,
                ),
            ),
            Expr::Block(b) => step_block(&b.stmts, b.tail.as_deref(), env, k),
            Expr::Call { callee, args } => State::Eval(
                callee,
                env.clone(),
                push(
                    Frame::CallArgs { callee: None, done: Vec::new(), pending: args, env, span },
                    k,
                ),
            ),
        })
    }

    fn ret<'a>(
        interp: &mut Interp,
        v: Value,
        k: Kont<'a>,
    ) -> Result<Option<State<'a>>, RuntimeError> {
        let Some(node) = k else {
            interp.set_result(v);
            return Ok(None);
        };
        let (frame, rest) = Rc::try_unwrap(node)
            .unwrap_or_else(|rc| (*rc).clone_frame_pair());
        Ok(Some(match frame {
            Frame::BinRight { op, rhs, env, span } => {
                State::Eval(rhs, env, push(Frame::BinApply { op, lval: v, span }, rest))
            }
            Frame::BinApply { op, lval, span } => {
                State::Return(apply_binop(op, lval, v, span)?, rest)
            }
            Frame::UnApply { op, span } => State::Return(apply_unop(op, v, span)?, rest),
            Frame::IfBranch { then_blk, else_blk, env, span } => match v {
                Value::Bool(true) => eval_block_ref(then_blk, env, rest),
                Value::Bool(false) => eval_block_ref(else_blk, env, rest),
                _ => return Err(rt(span, "if condition must be a Bool")),
            },
            Frame::LetCont { name, rest: stmts, tail, env } => {
                let env2 = env.extend(&[(name.to_string(), v)]);
                step_block(stmts, tail, env2, rest)
            }
            Frame::SeqDrop { rest: stmts, tail, env } => step_block(stmts, tail, env, rest),
            Frame::CallArgs { callee, done, pending, env, span } => {
                advance_call(interp, v, callee, done, pending, env, span, rest)?
            }
        }))
    }

    fn eval_block_ref<'a>(b: &'a Block, env: Env, k: Kont<'a>) -> State<'a> {
        step_block(&b.stmts, b.tail.as_deref(), env, k)
    }

    #[allow(clippy::too_many_arguments)]
    fn advance_call<'a>(
        interp: &mut Interp,
        v: Value,
        callee: Option<Value>,
        mut done: Vec<Value>,
        pending: &'a [Spanned<Expr>],
        env: Env,
        span: Span,
        rest: Kont<'a>,
    ) -> Result<State<'a>, RuntimeError> {
        // `v` is the just-evaluated callee (if callee is None) or an argument.
        let callee_val = match callee {
            None => v, // v was the callee
            Some(c) => {
                done.push(v);
                c
            }
        };
        if let Some((next, more)) = pending.split_first() {
            return Ok(State::Eval(
                next,
                env.clone(),
                push(
                    Frame::CallArgs { callee: Some(callee_val), done, pending: more, env, span },
                    rest,
                ),
            ));
        }
        // All args evaluated — apply. NO frame is pushed here (the TCE lever).
        apply(interp, callee_val, done, span, rest)
    }

    fn apply<'a>(
        interp: &mut Interp,
        callee: Value,
        args: Vec<Value>,
        span: Span,
        k: Kont<'a>,
    ) -> Result<State<'a>, RuntimeError> {
        match callee {
            Value::Closure(func, cenv) => {
                if func.params.len() != args.len() {
                    return Err(rt(
                        span,
                        format!("`{}` expects {} argument(s), got {}", func.name, func.params.len(), args.len()),
                    ));
                }
                let bindings: Vec<(String, Value)> = func
                    .params
                    .iter()
                    .map(|p| p.node.name.clone())
                    .zip(args)
                    .collect();
                let call_env = cenv.extend(&bindings);
                // SAFETY of lifetimes: `func` is Rc<FnDecl> owned here; but the
                // machine borrows `&'a` AST. To evaluate the *borrowed* body we
                // need the module's FnDecl, not this clone. See note below.
                let _ = call_env;
                Err(rt(span, "internal: closure application over borrowed AST — see Task 8 note"))
            }
            _ => Err(rt(span, "value is not callable")),
        }
    }
}
```

> **Resolve the borrow/ownership tension before finishing Step 3.** The `apply` stub above surfaces the one real design decision in this task: frames hold `&'a` borrows into the `Module`, but closures were stored as `Value::Closure(Rc<FnDecl>, Env)` (owned). To evaluate a closure body as a **borrowed** `&'a Block`, the machine must reach the *borrowed* `FnDecl` in the module, not an owned clone. **Fix:** index the module's functions by name once at startup into `HashMap<String, &'a FnDecl>`, store closures as `Value::Closure(Rc<String /*fn name*/>, Env)` **or** carry the function index, and in `apply` look up the borrowed `&'a FnDecl` by name/index to get `&'a Block`. Implement that lookup (a `fns: &'a HashMap<String, &'a FnDecl>` threaded through `run_loop`/`step`/`ret`/`apply`, or a `Vec<&'a FnDecl>` with closures holding the index). Then `apply` ends with `Ok(eval_block_ref(&fdecl.body.node, call_env, k))` — **no frame pushed**. Complete this so the two Step-1 tests pass; the crosscheck in Task 9 and the TCE test in Task 10 depend on it.

Also add to the shared `Interp` (module top) the methods the machine calls:
```rust
impl Interp {
    fn note_kont(&mut self, k: &usize_placeholder) { /* replaced below */ }
}
```
Concretely, add a `peak_kont: usize` field to `Interp`, a setter for the final result value (unused for output but keeps the loop uniform), and:
```rust
    pub fn peak_kont_depth(&self) -> usize { self.peak_kont }
    fn note_kont_depth(&mut self, depth: usize) { self.peak_kont = self.peak_kont.max(depth); }
    fn set_result(&mut self, _v: Value) { /* output already captured via io.println */ }
```
and change `run_loop` to call `interp.note_kont_depth(kont_len(kont_of(&st)))`. (`io.println` output is written when the builtin is applied — implement the builtin branch inside `apply` for a `Value::Closure` whose name is a builtin, or special-case `Expr::Qualified` callees in `eval` for `io.println` exactly as the type checker does. Simplest: in `advance_call`, when the *callee expression* was `io.println`, route to the builtin; carry a small `Callee` enum `{ Builtin(&'static str), Value(Value) }` instead of `Option<Value>` so builtins don't need a `Value` representation.)

> This task is the single largest in the plan. The Step-1 tests (`hello_world_on_cek`, `arithmetic_functions_if_on_cek`) are the acceptance gate: implement the borrowed-AST closure application and the `io.println` builtin routing until both pass.

- [ ] **Step 4: Repoint `eval::run_module` to the CEK machine**

```rust
pub fn run_module(module: &Module) -> Result<Interp, RuntimeError> {
    cek::run_module(module)
}
```
(keep `run_module_tree` → `tree::run_module`).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib cek_tests` then `cargo test --all`
Expected: PASS — the two CEK tests plus every existing test (examples now run via the CEK machine).

- [ ] **Step 6: Commit**

```bash
git add src/eval.rs
git commit -m "feat(eval): CEK abstract machine (persistent Rc-frame Kont, no-push application)"
```

---

## Task 9: cross-check — CEK output equals tree-walker output

**Files:**
- Create: `tests/crosscheck.rs`

**Interfaces:**
- Consumes: `elya::eval::{run_module, run_module_tree}`, `elya::parse::parse_module`.

- [ ] **Step 1: Write the test**

`tests/crosscheck.rs`:
```rust
use elya::parse::parse_module;
use elya::Session;

fn both(src: &str) -> (String, String) {
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let cek = elya::eval::run_module(&m).unwrap().output().to_string();
    let tree = elya::eval::run_module_tree(&m).unwrap().output().to_string();
    (cek, tree)
}

fn corpus() -> Vec<String> {
    let dir = format!("{}/examples", env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) == Some("elya") {
            out.push(std::fs::read_to_string(p).unwrap());
        }
    }
    out.push("fn f(x){ x + 1 }\npub fn main(){ let a = f(f(1))\n io.println(\"ok\") }\n".into());
    out.push("pub fn main(){ if 1 < 2 { io.println(\"a\") } else { io.println(\"b\") } }\n".into());
    out
}

#[test]
fn cek_matches_tree_walker() {
    for src in corpus() {
        let (cek, tree) = both(&src);
        assert_eq!(cek, tree, "divergence on:\n{src}");
    }
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test --test crosscheck`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/crosscheck.rs
git commit -m "test(eval): CEK output cross-checked against the tree-walker oracle"
```

---

## Task 10: TCE — bounded-depth assertions

**Files:**
- Modify: `tests/tce.rs`

**Interfaces:**
- Consumes: `elya::eval::run_module` (CEK) + `Interp::peak_kont_depth`.

- [ ] **Step 1: Replace the grow-only test with bounded + grow assertions**

Rewrite `tests/tce.rs`:
```rust
//! TCE is a measured guarantee: deep tail recursion runs in bounded Kont depth.

use elya::parse::parse_module;
use elya::Session;

fn peak(src: &str) -> usize {
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    elya::eval::run_module(&m).unwrap().peak_kont_depth()
}

// Pin K_MAX to the measured peak of a bounded tail loop. Any per-iteration leak
// pushes the measured peak toward the iteration count and fails the equality.
const K_MAX: usize = 8; // adjust to the first measured value; MUST be a small constant

#[test]
fn self_tail_recursion_is_bounded() {
    let src = "fn down(n) { if n == 0 { 0 } else { down(n - 1) } }\n\
               pub fn main() { let _ = down(1000000)\n io.println(\"done\") }\n";
    assert!(peak(src) <= K_MAX, "peak={} exceeds K_MAX={}", peak(src), K_MAX);
}

#[test]
fn mutual_tail_recursion_is_bounded() {
    let src = "fn ev(n) { if n == 0 { True } else { od(n - 1) } }\n\
               fn od(n) { if n == 0 { False } else { ev(n - 1) } }\n\
               pub fn main() { let _ = ev(1000000)\n io.println(\"done\") }\n";
    assert!(peak(src) <= K_MAX, "peak={} exceeds K_MAX={}", peak(src), K_MAX);
}

#[test]
fn non_tail_recursion_grows_with_depth() {
    let prog = |n: i64| {
        format!(
            "fn sum(n) {{ if n == 0 {{ 0 }} else {{ n + sum(n - 1) }} }}\n\
             pub fn main() {{ let _ = sum({n})\n io.println(\"done\") }}\n"
        )
    };
    let shallow = peak(&prog(5));
    let deep = peak(&prog(50));
    assert!(deep > shallow, "non-tail must grow: shallow={shallow}, deep={deep}");
    assert!(deep >= 45, "expected depth ~proportional to n=50, got {deep}");
}
```

- [ ] **Step 2: Measure and pin `K_MAX`**

Run: `cargo test --test tce -- --nocapture` once. If `self_tail_recursion_is_bounded` fails with `peak=<value>`, set `const K_MAX` to that measured `<value>` (it must be a small single/low-double-digit constant, independent of the 1,000,000 iteration count). If instead the measured peak is near 1,000,000, that is a **real TCE bug** in Task 8's `apply` (a frame is being pushed on tail calls) — fix the machine, do not raise `K_MAX`.

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --test tce`
Expected: PASS (3 tests): both bounded assertions hold at the pinned small `K_MAX`; the non-tail control grows.

- [ ] **Step 4: Commit**

```bash
git add tests/tce.rs
git commit -m "test(tce): bounded-depth TCE guarantee (self + mutual tail recursion) with grow control"
```

---

## Task 11: pipeline switch, regression sweep, and docs

**Files:**
- Modify: `src/lib.rs`, `src/eval.rs` (defensive internal errors), `README.md`
- Test: existing suites

**Interfaces:**
- `run_source` becomes `parse → resolve → types::infer → eval::run_module (CEK)`; type errors stop before evaluation.

- [ ] **Step 1: Switch `run_source` to infer-then-CEK**

In `src/lib.rs`:
```rust
pub fn run_source(name: &str, text: &str) -> Result<String, String> {
    let session = Session::new();
    let sm = SourceMap::new(name, text);
    let (module, mut diags) = parse::parse_module(&session, text);
    diags.extend(resolve::check(&session, &module));
    if diags.is_empty() {
        diags.extend(types::infer(&session, &module));
    }
    if let Some(rendered) = fail_if_errors(&diags, &sm) {
        return Err(rendered);
    }
    match eval::run_module(&module) {
        Ok(interp) => Ok(interp.output().to_string()),
        Err(e) => Err(render(&[e.diag], &sm)),
    }
}
```

- [ ] **Step 2: Add a type-error → run_source test**

Add to `src/lib.rs` tests:
```rust
    #[test]
    fn run_source_rejects_ill_typed_at_compile_time() {
        let err = run_source("t.elya", "pub fn main() { let _ = 1 + \"a\"\n io.println(\"x\") }\n")
            .unwrap_err();
        assert!(err.contains("E0400"), "{err}");
    }
```

- [ ] **Step 3: Run the full suite and fix regressions**

Run: `cargo test --all`
Expected: PASS. If a Slice-1 evaluator unit test fed deliberately ill-typed input and now the pipeline path rejects it earlier, that test targeted the *evaluator directly* (not `run_source`) and is unaffected; only `run_source`/`check_source`-level tests see the new compile-time rejection. Adjust any that assumed runtime type errors from `run_source`.

- [ ] **Step 4: Update the README status**

In `README.md`, change the status paragraph to note Slice 2 is complete: the interpreter is now typed (Hindley–Milner) and runs on a CEK machine with a measured TCE guarantee; Slice 3 (algebraic effects & handlers) is next.

- [ ] **Step 5: Run the full local gate**

Run: `sh scripts/check.sh` (or `pwsh scripts/check.ps1`).
Expected: `cargo fmt --check` clean, `cargo clippy --all-targets -- -D warnings` clean, all tests PASS. Fix any fmt/clippy findings (`cargo fmt --all`; resolve clippy lints as in Slice 1 — prefer real fixes over `#[allow]`).

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/eval.rs README.md
git commit -m "feat: pipeline is parse->resolve->infer->cek; type errors are compile-time; Slice 2 complete"
```

---

## Self-Review

**1. Spec coverage (spec §§ → tasks):**
- §2.1–2.2 types + unification + occurs-check → Task 1.
- §2.8 zonking/letter-naming reporter + no-`%t` → Tasks 2, 6.
- §2.3, §2.6 inference rules + operator/builtin monotypes → Task 3.
- §2.4 instantiation/let-generalization → Tasks 3, 5.
- §2.5 SCC recursive groups + polymorphism → Task 5.
- §3.2 persistent `Env`/`Kont` → Tasks 7 (Env, required for TCE) and 8 (Kont).
- §3.3–3.6 frame set, step function, no-push application, builtins → Task 8.
- §3.7 oracle cross-check → Tasks 7 (retain tree) and 9 (cross-check).
- §4 TCE bounded-depth assertions (self + mutual + existence + grow) → Task 10.
- §5 pipeline change, compile-time type errors, layer map → Tasks 4, 11.
- §6 testing (inference units, UI fixtures, snapshots, crosscheck, TCE, regression, gate) → Tasks 3–6, 9, 10, 11.
- §2.7 `case` deferred → not built (surface frozen); nothing to do.

**Deferrals honored (spec §9):** effect rows unparsed-into-types (Task 3 ignores the row), no ADTs/traits/lambdas/annotations/Core IR, polymorphic recursion unsupported (Task 5 monomorphic-within-SCC), multi-file/cross-module TCE not attempted (Task 10 is intra-file). The borrowed-AST-vs-capture nuance is flagged in Task 8 as a mechanical Slice-3 step.

**2. Placeholder scan:** No `TODO`/`TBD`. Two deliberate, labeled forward-references: Task 3's placeholder `generalize` (replaced in Task 5, signature stable) and Task 8's `apply` stub whose resolution is spelled out in the same step. Task 10's `K_MAX` is a measured-then-pinned constant with an explicit "don't raise it to hide a bug" instruction.

**3. Type/name consistency:** `Ty`, `TyCon`, `Scheme`, `Infer` (`fresh`/`resolve`/`unify`/`instantiate`/`generalize`), `TyEnv`, `display_ty`/`display_scheme`, `infer`/`infer_schemes` are used identically across Tasks 1–6. `Value::Closure`, `Env` (`new`/`extend`/`get`), `apply_binop`/`apply_unop`, `eval::{run_module, run_module_tree}`, `Interp::{output, peak_kont_depth}` are consistent across Tasks 7–11. Diagnostic codes E0400–E0403 match the spec and Global Constraints.

---

## Execution Handoff

Plan complete. Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks.
2. **Inline Execution** — execute tasks in this session with checkpoints.

Which approach?
