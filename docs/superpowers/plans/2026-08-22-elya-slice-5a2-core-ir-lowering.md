# Elya — Slice 5a-2 Implementation Plan: Core IR Datatype & Type-Directed Lowering

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce the first real Core IR datatype (`src/core.rs`) and a type-directed lowering that consumes 5a-1's frozen `Span → Ty` table and produces a Core tree with the type inline on each node (Shape C), proven by a typed-Core snapshot over the reused 5a-1 corpus.

**Architecture:** A minimal, corpus-derived Core datatype whose child pointers are all `Rc`-shared (mirroring the AST); a structural fold `lower_module` that copies each node's already-zonked `Ty` out of the frozen table and places it inline (never touching the solver); and a typed S-expression snapshot rendered through one shared name table, plus per-surface teeth and two origin-specific proofs (lookup cross-check for direct nodes, derivation check for synthesized `Let` nodes). Core is built and snapshotted but **not executed** and **not wired into `front_end`**.

**Tech Stack:** Rust 2021 (MSRV 1.75); `insta` (dev-dependency) for snapshots; no new dependencies. The Core type reuses `types::Ty` and `ast::BinOp` verbatim.

**Spec:** [docs/superpowers/specs/2026-08-21-elya-slice-5a2-core-ir-lowering-design.md](../specs/2026-08-21-elya-slice-5a2-core-ir-lowering-design.md) — the plan argues from the spec; executors read both.

## Global Constraints

Every task's requirements implicitly include this section. Values copied verbatim from project conventions and the spec's non-negotiable boundary (spec §1, §6).

- **Rust 2021, MSRV 1.75.** No new dependencies. `insta = "1"` is already a dev-dependency; `logos`/`ariadne` are the only runtime deps.
- **Command prelude.** Prefix every cargo invocation in this environment with:
  `export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0` — cargo/rustc are not on the default Bash PATH here, and the incremental cache hangs on this machine.
- **The gate is `sh scripts/check.sh`**, which runs, in order: `cargo fmt --all -- --check`, then `cargo clippy --all-targets -- -D warnings`, then `cargo test --all`. **Run `cargo fmt --all` (write mode) before the gate** — the gate fmt-*checks* and fails hard on any drift.
- **Clippy is `-D warnings`:** committed code must be clippy-clean. Prefer `ok_or_else(|| …)` over `ok_or(…)`; match the codebase's `s.push_str(&format!(…))` idiom (ast.rs uses it, clippy passes).
- **Atomic commits, never red.** Structure each commit as `if sh scripts/check.sh; then git commit …; fi` — never commit a red tree.
- **Explicit paths only.** Stage with `git add <explicit paths>`; **never** `git add -A` / `git add .`.
- **Commit trailer** (last line of every commit message):
  `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`
- **Push `origin/main` after each task commit.**
- **Non-negotiable boundary (spec §1, §6):** Core is built and snapshotted but **not executed** (no evaluator, no CEK re-point); lowering is **not** wired into `front_end`; exhaustiveness and `affine` are **not** re-platformed onto Core; lowering **never** calls `resolve`/`unify`/`unify_row`/`bind`/`instantiate`/`generalize` (the table is frozen and zonked — it only *clones* `Ty`); `eval.rs`, the CEK machine, and `infer_with_types` are byte-for-byte untouched; the arch layering test passes **unmodified** (`core` is pre-reserved at layer 5, and `src/core.rs` references only `ast`/`types`/`span`).

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `src/types.rs` (modify) | Add `infer_typed_table` (raw `Span → Ty` accessor, sibling of `infer_with_types`) and, later, the public `TyPrinter` render facade that wraps the private `Names`/`write_ty` path. | 1, 3 |
| `tests/typed_table.rs` (create) | Parity tests: the raw accessor types the same spans `infer_with_types` renders, carries structured `Ty` (not strings), and preserves the polymorphic `Ty::Var`. | 1 |
| `src/lib.rs` (modify) | Add `pub mod core;`. | 2 |
| `src/core.rs` (create) | The minimal Core datatype (Shape C, `Rc`-shared), `LowerError`, `lower_module`/`lower_*` (the structural fold), a focused `fn id` unit test, and — in Task 3 — `pretty_typed` (the typed S-expression renderer). References only `ast`/`types`/`span`. | 2, 3 |
| `tests/core_lowering.rs` (create) | The deliverable: reused 5a-1 corpus lowered to Core, per-surface snapshots + teeth, the two origin proofs, and the coherence / no-token-leak invariants. | 3 |
| `tests/snapshots/core_lowering__*.snap` (generated) | The auditable typed-Core snapshots. | 3 |

**Note on the spec's `pretty_typed(&CoreModule, &mut Names)` sketch (§7):** `Names` and `write_ty` are **private** to `types.rs` (verified: `struct Names` at types.rs:587, `fn write_ty` at types.rs:647). Rust module privacy means `core.rs` cannot name `types::Names`. This plan therefore exposes a focused public facade `TyPrinter` (wrapping the private `Names`) instead of leaking `Names`/`write_ty` — preserving the spec's intent ("rendered through the *same* `write_ty`/`Names` path 5a-1 uses", §5) with the minimum public surface. `pretty_typed` takes `&mut TyPrinter`.

---

### Task 1: The raw `Span → Ty` accessor

**Files:**
- Modify: `src/types.rs` (add `infer_typed_table` immediately after `infer_with_types`, which ends at types.rs:1668)
- Test: `tests/typed_table.rs` (create)

**Interfaces:**
- Consumes: `infer_all(module, want_types: bool) -> (Vec<(String,String)>, Vec<Diagnostic>, HashSet<Span>, BTreeMap<Span, Ty>)` (existing, types.rs:1684); `infer_with_types(&Session, &Module) -> (Vec<Diagnostic>, BTreeMap<Span, String>)` (existing, types.rs:1655).
- Produces: `pub fn infer_typed_table(session: &Session, module: &Module) -> (Vec<Diagnostic>, BTreeMap<Span, Ty>)` in `crate::types` — the raw, zonked, structured type table that Task 2's `lower_module` consumes.

- [ ] **Step 1: Write the failing parity test**

Create `tests/typed_table.rs`:

```rust
//! Slice 5a-2 Task 1 — the raw `Span → Ty` accessor. Proves it feeds the same
//! zonked types `infer_with_types` renders, but structured (not stringified), and
//! that a polymorphic node's `Ty::Var` rides through unmonomorphized.

use std::collections::BTreeSet;

use elya::parse::parse_module;
use elya::span::Span;
use elya::types::{infer_typed_table, infer_with_types, Ty, TyCon};
use elya::Session;

/// A rendered type is a bare type variable iff it is `[a-z][0-9]*`.
fn is_var(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_lowercase()) && cs.all(|c| c.is_ascii_digit())
}

#[test]
fn raw_table_shares_spans_with_rendered_and_carries_structured_types() {
    let src = "fn add1(n) { n + 1 }\n";
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");

    let (d_raw, raw) = infer_typed_table(&Session::new(), &m);
    let (d_str, rendered) = infer_with_types(&Session::new(), &m);
    assert!(d_raw.is_empty(), "raw diags: {d_raw:?}");
    assert!(d_str.is_empty(), "str diags: {d_str:?}");

    // Both accessors type exactly the same set of spans.
    let keys_raw: BTreeSet<Span> = raw.keys().copied().collect();
    let keys_str: BTreeSet<Span> = rendered.keys().copied().collect();
    assert_eq!(keys_raw, keys_str, "raw/rendered span sets differ");

    // The raw table carries structured `Ty` (not strings): a monomorphic program
    // has at least one Int node, and its span renders "Int" in the string table.
    let mut saw_int = false;
    for (span, t) in &raw {
        if matches!(t, Ty::Base(TyCon::Int)) {
            saw_int = true;
            assert_eq!(rendered.get(span).map(String::as_str), Some("Int"));
        }
    }
    assert!(saw_int, "expected an Int-typed node: {raw:?}");
}

#[test]
fn raw_table_preserves_the_polymorphic_var() {
    // The load-bearing case: a polymorphic body zonks to a bare `Ty::Var`; the raw
    // table must carry it as a variable (not a ground type), matching the rendered
    // table's bare-letter entry (stay-polymorphic, 5a-1 §4).
    let src = "fn id(x) { x }\n";
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");

    let (_d, raw) = infer_typed_table(&Session::new(), &m);
    let (_d2, rendered) = infer_with_types(&Session::new(), &m);

    assert!(
        raw.values().any(|t| matches!(t, Ty::Var(_))),
        "polymorphic id should carry a Ty::Var node: {raw:?}"
    );
    for (span, t) in &raw {
        if matches!(t, Ty::Var(_)) {
            let r = rendered.get(span).expect("var span rendered");
            assert!(is_var(r), "raw Var did not render as a bare var: {r}");
        }
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0; cargo test --test typed_table`
Expected: FAIL to compile — `infer_typed_table` is not found in `elya::types`.

- [ ] **Step 3: Add the accessor**

In `src/types.rs`, immediately after `infer_with_types` (the function ending at line 1668), add:

```rust
/// Inference plus the raw per-node type table (Slice 5a-2): the same zonked
/// `Span → Ty` table `infer_with_types` renders, returned as structured `Ty`
/// values (not strings) so the Core IR lowering can carry them inline. A sibling
/// of `infer_with_types`; both run `infer_all(module, true)`.
pub fn infer_typed_table(
    _session: &Session,
    module: &Module,
) -> (Vec<Diagnostic>, BTreeMap<Span, Ty>) {
    let (_schemes, diags, _sites, typed) = infer_all(module, true);
    (diags, typed)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0; cargo test --test typed_table`
Expected: PASS (both tests).

- [ ] **Step 5: Run the full gate — confirm no existing snapshot or caller churns**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0; cargo fmt --all && sh scripts/check.sh`
Expected: PASS. `infer_with_types` and the two `want_types = false` callers (`infer_schemes`, `infer_with_sites`) are unchanged; `tests/typed_inference.rs` and all existing `.snap` files are byte-for-byte identical.

- [ ] **Step 6: Commit and push**

```bash
export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0
if sh scripts/check.sh; then
  git add src/types.rs tests/typed_table.rs
  git commit -m "$(cat <<'EOF'
feat(core): infer_typed_table — raw Span→Ty accessor (5a-2 Task 1)

The Core IR lowering feeder: a sibling of infer_with_types that returns the
frozen, zonked BTreeMap<Span, Ty> structured (not stringified). Parity tests
pin the shared span set, structured Int nodes, and the preserved polymorphic
Ty::Var. infer_with_types and existing snapshots are untouched.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
EOF
)"
  git push origin main
fi
```

---

### Task 2: The Core datatype + type-directed lowering

**Files:**
- Modify: `src/lib.rs` (add `pub mod core;` in the alphabetical module block, lib.rs:3-12 — between `pub mod ast;` and `pub mod diag;`)
- Create: `src/core.rs`
- Test: `src/core.rs` `#[cfg(test)] mod tests` (the focused `fn id` headline tooth)

**Interfaces:**
- Consumes: `infer_typed_table(&Session, &Module) -> (Vec<Diagnostic>, BTreeMap<Span, Ty>)` (Task 1); AST types from `crate::ast` — `Module`, `Decl::Fn(FnDecl)`, `FnDecl{name, params: Vec<Spanned<Param>>, body: Rc<Spanned<Block>>}`, `Block{stmts: Rc<[Spanned<Stmt>]>, tail: Option<Rc<Spanned<Expr>>>}`, `Stmt::{Let{name, value}, Expr}`, `Expr::{Int, Bool, Str, Unit, Var, Call{callee, args}, Binary{op, lhs, rhs}, Lambda{params, body}, Match{scrutinee, arms}, Float, Qualified, Unary, If, Block, Handle, Resume}`, `MatchArm{pat, body}`, `Pattern::{Wild, Var, Ctor{name, args}, Lit}`, `PatLit::{Int, Bool, Str, Unit}`, `BinOp`; `crate::span::Span`; `crate::types::Ty`.
- Produces (in `crate::core`):
  - `pub struct CoreExpr { pub span: Span, pub ty: Ty, pub kind: CoreKind }`
  - `pub enum CoreKind { Lit(CoreLit), Var(String), App(Rc<CoreExpr>, Rc<[CoreExpr]>), Prim(BinOp, Rc<[CoreExpr]>), Lambda(Rc<[String]>, Rc<CoreExpr>), Let(String, Rc<CoreExpr>, Rc<CoreExpr>), Match(Rc<CoreExpr>, Rc<[CoreArm]>) }`
  - `pub struct CoreArm { pub pat: CorePat, pub body: CoreExpr }`
  - `pub enum CorePat { Wild, Var(String), Ctor(String, Rc<[CorePat]>), Lit(CoreLit) }`
  - `pub enum CoreLit { Int(i64), Bool(bool), Str(String), Unit }`
  - `pub struct CoreFn { pub name: String, pub params: Rc<[String]>, pub body: CoreExpr }`
  - `pub struct CoreModule { pub fns: Vec<CoreFn> }`
  - `pub enum LowerError { Unsupported(&'static str), Untyped(Span) }`
  - `pub fn lower_module(module: &Module, table: &BTreeMap<Span, Ty>) -> Result<CoreModule, LowerError>`

- [ ] **Step 1: Declare the module**

In `src/lib.rs`, add `pub mod core;` immediately after `pub mod ast;` (line 4), keeping the block alphabetical:

```rust
pub mod affine;
pub mod ast;
pub mod core;
pub mod diag;
```

- [ ] **Step 2: Write the datatype + lowering (`src/core.rs`)**

Create `src/core.rs`:

```rust
//! Core IR (Slice 5a-2): a minimal, corpus-derived intermediate representation
//! with each node's zonked type carried inline (Shape C). Built by a type-directed
//! fold over the typed AST that clones types out of the frozen `Span → Ty` table —
//! it performs no inference. Core is not executed here (spec §6).

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::ast::{BinOp, Block, Decl, Expr, Module, PatLit, Pattern, Stmt};
use crate::span::Span;
use crate::types::Ty;

/// One Core expression: its source provenance, its inline type (Shape C), and shape.
/// Every child pointer is `Rc`-shared, so a subtree is a refcount clone, not a deep
/// copy — mirroring the AST (ast.rs:167-202); every later Core consumer inherits it.
#[derive(Clone, Debug)]
pub struct CoreExpr {
    /// Provenance (diagnostics + the §5 lookup cross-check); never a type key.
    pub span: Span,
    /// The type materialized in 5a-1, now carried inline (may be `Ty::Var(_)`).
    pub ty: Ty,
    pub kind: CoreKind,
}

#[derive(Clone, Debug)]
pub enum CoreKind {
    Lit(CoreLit),
    /// local, top-level fn name, or nullary ctor (e.g. `Tok`).
    Var(String),
    /// callee + args (function / ctor / effect-op application).
    App(Rc<CoreExpr>, Rc<[CoreExpr]>),
    /// `+` etc. — the primitive operator set is reused verbatim from the AST.
    Prim(BinOp, Rc<[CoreExpr]>),
    /// uncurried params; the block body is flattened into a single expression.
    Lambda(Rc<[String]>, Rc<CoreExpr>),
    /// binding spine synthesized from block-flattening: `Let(name, value, body)`.
    Let(String, Rc<CoreExpr>, Rc<CoreExpr>),
    Match(Rc<CoreExpr>, Rc<[CoreArm]>),
}

#[derive(Clone, Debug)]
pub struct CoreArm {
    pub pat: CorePat,
    pub body: CoreExpr,
}

#[derive(Clone, Debug)]
pub enum CorePat {
    Wild,
    Var(String),
    Ctor(String, Rc<[CorePat]>),
    Lit(CoreLit),
}

#[derive(Clone, Debug)]
pub enum CoreLit {
    Int(i64),
    Bool(bool),
    Str(String),
    Unit,
}

#[derive(Clone, Debug)]
pub struct CoreFn {
    pub name: String,
    pub params: Rc<[String]>,
    pub body: CoreExpr,
}

#[derive(Clone, Debug)]
pub struct CoreModule {
    pub fns: Vec<CoreFn>,
}

/// Lowering failure. The deferred AST surface is a *typed boundary*, not a panic:
/// an out-of-subset AST node is `Unsupported`, a node absent from the frozen table
/// is `Untyped`. Neither fires over the fixed corpus (spec §4).
#[derive(Clone, Debug, PartialEq)]
pub enum LowerError {
    Unsupported(&'static str),
    Untyped(Span),
}

/// Lower a whole module: each `Decl::Fn` becomes a `CoreFn`; `Decl::Type` and
/// `Decl::Effect` are not re-homed (spec §2 — exhaustiveness-on-Core is out of scope).
pub fn lower_module(
    module: &Module,
    table: &BTreeMap<Span, Ty>,
) -> Result<CoreModule, LowerError> {
    let mut fns = Vec::new();
    for d in &module.decls {
        if let Decl::Fn(f) = &d.node {
            let params: Vec<String> = f.params.iter().map(|p| p.node.name.clone()).collect();
            let body = lower_block(&f.body.node, table)?;
            fns.push(CoreFn {
                name: f.name.clone(),
                params: params.into(),
                body,
            });
        }
    }
    Ok(CoreModule { fns })
}

/// Flatten a block into a right-nested `Let` spine terminating in the lowered tail.
/// The `Let` nodes are the only *synthesized* Core nodes: their type is derived by
/// propagation (`ty = body.ty`), their span is the originating statement's span.
fn lower_block(block: &Block, table: &BTreeMap<Span, Ty>) -> Result<CoreExpr, LowerError> {
    let tail = block
        .tail
        .as_ref()
        .ok_or_else(|| LowerError::Unsupported("block without tail expression"))?;
    let mut acc = lower_expr(&tail.node, tail.span, table)?;
    for stmt in block.stmts.iter().rev() {
        let (name, value) = match &stmt.node {
            Stmt::Let { name, value } => {
                (name.clone(), lower_expr(&value.node, value.span, table)?)
            }
            // A non-tail expression statement: a discarded binding — no separate
            // sequencing node is needed for the corpus.
            Stmt::Expr(e) => ("_".to_string(), lower_expr(&e.node, e.span, table)?),
        };
        let ty = acc.ty.clone();
        acc = CoreExpr {
            span: stmt.span,
            ty,
            kind: CoreKind::Let(name, Rc::new(value), Rc::new(acc)),
        };
    }
    Ok(acc)
}

/// Lower one expression node. `ty` is copied from the frozen table (`table[span]`);
/// the map is never consulted again after the tree is built. No solver primitive is
/// ever called (spec §4, §6).
fn lower_expr(e: &Expr, span: Span, table: &BTreeMap<Span, Ty>) -> Result<CoreExpr, LowerError> {
    let ty = table
        .get(&span)
        .cloned()
        .ok_or_else(|| LowerError::Untyped(span))?;
    let kind = match e {
        Expr::Int(n) => CoreKind::Lit(CoreLit::Int(*n)),
        Expr::Bool(b) => CoreKind::Lit(CoreLit::Bool(*b)),
        Expr::Str(v) => CoreKind::Lit(CoreLit::Str(v.clone())),
        Expr::Unit => CoreKind::Lit(CoreLit::Unit),
        Expr::Var(x) => CoreKind::Var(x.clone()),
        Expr::Call { callee, args } => {
            let f = lower_expr(&callee.node, callee.span, table)?;
            let mut lowered = Vec::with_capacity(args.len());
            for a in args.iter() {
                lowered.push(lower_expr(&a.node, a.span, table)?);
            }
            CoreKind::App(Rc::new(f), lowered.into())
        }
        Expr::Binary { op, lhs, rhs } => {
            let l = lower_expr(&lhs.node, lhs.span, table)?;
            let r = lower_expr(&rhs.node, rhs.span, table)?;
            CoreKind::Prim(*op, vec![l, r].into())
        }
        Expr::Lambda { params, body } => {
            let names: Vec<String> = params.iter().map(|p| p.node.name.clone()).collect();
            let b = lower_block(&body.node, table)?;
            CoreKind::Lambda(names.into(), Rc::new(b))
        }
        Expr::Match { scrutinee, arms } => {
            let s = lower_expr(&scrutinee.node, scrutinee.span, table)?;
            let mut lowered = Vec::with_capacity(arms.len());
            for arm in arms.iter() {
                let body = lower_expr(&arm.node.body.node, arm.node.body.span, table)?;
                lowered.push(CoreArm {
                    pat: lower_pat(&arm.node.pat.node),
                    body,
                });
            }
            CoreKind::Match(Rc::new(s), lowered.into())
        }
        // The deferred surface (spec §4, §11): a typed boundary, not a panic. None
        // of these occur in the 5a-2 corpus.
        Expr::Float(_) => return Err(LowerError::Unsupported("Float")),
        Expr::Qualified { .. } => return Err(LowerError::Unsupported("Qualified")),
        Expr::Unary { .. } => return Err(LowerError::Unsupported("Unary")),
        Expr::If { .. } => return Err(LowerError::Unsupported("If")),
        Expr::Block(_) => return Err(LowerError::Unsupported("Block")),
        Expr::Handle { .. } => return Err(LowerError::Unsupported("Handle")),
        Expr::Resume { .. } => return Err(LowerError::Unsupported("Resume")),
    };
    Ok(CoreExpr { span, ty, kind })
}

/// Patterns carry no inline type (the corpus asserts on expression-node types only;
/// binder types are not in the table — 5a-1 §3). Total over the corpus pattern set.
fn lower_pat(p: &Pattern) -> CorePat {
    match p {
        Pattern::Wild => CorePat::Wild,
        Pattern::Var(x) => CorePat::Var(x.clone()),
        Pattern::Ctor { name, args } => {
            let lowered: Vec<CorePat> = args.iter().map(|a| lower_pat(&a.node)).collect();
            CorePat::Ctor(name.clone(), lowered.into())
        }
        Pattern::Lit(l) => CorePat::Lit(lower_pat_lit(l)),
    }
}

fn lower_pat_lit(l: &PatLit) -> CoreLit {
    match l {
        PatLit::Int(n) => CoreLit::Int(*n),
        PatLit::Bool(b) => CoreLit::Bool(*b),
        PatLit::Str(v) => CoreLit::Str(v.clone()),
        PatLit::Unit => CoreLit::Unit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_module;
    use crate::types::infer_typed_table;
    use crate::Session;

    #[test]
    fn lowers_polymorphic_id_body_to_a_var_node() {
        // The one seam with real risk (spec §3, §5): a node inside a polymorphic body
        // has type `Ty::Var(_)`. An assume-ground-types bug would monomorphize it to a
        // base type or poison it to `Ty::Error`. This tooth fails loudly on both.
        let src = "fn id(x) { x }\n";
        let (m, pd) = parse_module(&Session::new(), src);
        assert!(pd.is_empty(), "parse: {pd:?}");
        let (diags, table) = infer_typed_table(&Session::new(), &m);
        assert!(diags.is_empty(), "type errors: {diags:?}");

        let core = lower_module(&m, &table).expect("lowering fn id should succeed");
        let body = &core.fns[0].body;
        assert!(
            matches!(body.kind, CoreKind::Var(ref x) if x == "x"),
            "expected Var(\"x\") body, got {:?}",
            body.kind
        );
        assert!(
            matches!(body.ty, Ty::Var(_)),
            "lowering monomorphized or errored a polymorphic node: {:?}",
            body.ty
        );
    }
}
```

- [ ] **Step 3: Run the headline tooth — verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0; cargo test --lib core::tests::lowers_polymorphic_id_body_to_a_var_node`
Expected: PASS.

- [ ] **Step 4: Confirm the layering test still passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0; cargo test --test arch`
Expected: PASS (`no_upward_module_references`). `src/core.rs` references only `crate::ast` (layer 1), `crate::types` (layer 4), `crate::span` (layer 0) in production code, and `crate::parse` (layer 2) only in `#[cfg(test)]` — all strictly lower than `core` (layer 5), so no edit to `tests/arch/layering.rs` is needed.

- [ ] **Step 5: Run the full gate**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0; cargo fmt --all && sh scripts/check.sh`
Expected: PASS (fmt clean, clippy clean, all tests green).

- [ ] **Step 6: Commit and push**

```bash
export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0
if sh scripts/check.sh; then
  git add src/lib.rs src/core.rs
  git commit -m "$(cat <<'EOF'
feat(core): Core IR datatype + type-directed lowering (5a-2 Task 2)

Minimal, corpus-derived Core (Shape C, Rc-shared child pointers) reusing
types::Ty and ast::BinOp verbatim. lower_module folds the typed AST into Core,
copying each node's zonked Ty out of the frozen table inline — no solver reach.
Block-flattening synthesizes the Let spine (type derived = body.ty). The
deferred AST surface is a typed LowerError, not a panic. Headline tooth:
fn id(x){x} lowers to a Core body node whose ty is Ty::Var(_) — carried, not
monomorphized. Layering test passes unmodified (core → ast/types/span only).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
EOF
)"
  git push origin main
fi
```

---

### Task 3: The typed-Core snapshot corpus

**Files:**
- Modify: `src/types.rs` (add the public `TyPrinter` facade after `write_row`, which ends at types.rs:740)
- Modify: `src/core.rs` (add `pretty_typed` + private render helpers; add `use crate::types::TyPrinter;`)
- Create: `tests/core_lowering.rs`
- Generated: `tests/snapshots/core_lowering__*.snap`

**Interfaces:**
- Consumes: `lower_module` (Task 2); `infer_typed_table` (Task 1); the Core datatype (Task 2); `crate::types::Ty`.
- Produces:
  - `pub struct TyPrinter` with `pub fn new() -> TyPrinter` and `pub fn render(&mut self, t: &Ty) -> String` in `crate::types` — a stateful renderer assigning `a, b, c, …` to free variables in first-appearance order *across calls* (cross-node coherence), wrapping the private `Names`/`write_ty` path.
  - `pub fn pretty_typed(m: &CoreModule, p: &mut TyPrinter) -> String` in `crate::core` — the typed S-expression renderer for the snapshot.

- [ ] **Step 1: Add the `TyPrinter` facade to `src/types.rs`**

In `src/types.rs`, immediately after `write_row` (which ends at line 740, before `pub struct TyEnv`), add:

```rust
/// A stateful renderer that assigns `a, b, c, …` to free type/row variables in
/// first-appearance order **across `render` calls**, so a variable shared between
/// two nodes renders with the same letter (cross-node coherence — 5a-1 §5). It
/// wraps the same private `Names`/`write_ty` path `infer_with_types` uses, so a
/// Core node's inline type (Slice 5a-2) renders identically to the 5a-1 table line
/// for the AST node it came from. Assumes already-zonked types (the frozen table).
#[derive(Default)]
pub struct TyPrinter {
    names: Names,
}

impl TyPrinter {
    pub fn new() -> TyPrinter {
        TyPrinter::default()
    }

    /// Render one already-zonked type, extending the shared name assignment.
    pub fn render(&mut self, t: &Ty) -> String {
        let mut out = String::new();
        write_ty(t, &mut self.names, &mut out);
        out
    }
}
```

- [ ] **Step 2: Add the Core renderer to `src/core.rs`**

In `src/core.rs`, add `use crate::types::TyPrinter;` to the import block (below `use crate::types::Ty;`), then append the renderer (before the `#[cfg(test)] mod tests`):

```rust
/// Render a Core module as a typed S-expression: each expression node is annotated
/// with its inline type (`… : <ty>`), rendered through ONE shared `TyPrinter` so a
/// variable shared across nodes renders with one coherent letter. Patterns carry no
/// annotation (spec §4, §5). This string is the snapshot deliverable.
pub fn pretty_typed(m: &CoreModule, p: &mut TyPrinter) -> String {
    let mut s = String::new();
    for f in &m.fns {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str("(fn ");
        s.push_str(&f.name);
        s.push_str(" (");
        for (i, param) in f.params.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            s.push_str(param);
        }
        s.push_str(") ");
        pretty_expr(&f.body, p, &mut s);
        s.push(')');
    }
    s
}

fn pretty_expr(e: &CoreExpr, p: &mut TyPrinter, s: &mut String) {
    match &e.kind {
        CoreKind::Lit(l) => {
            s.push_str("(lit ");
            push_lit(l, s);
        }
        CoreKind::Var(x) => {
            s.push_str("(var ");
            s.push_str(x);
        }
        CoreKind::App(f, args) => {
            s.push_str("(app ");
            pretty_expr(f, p, s);
            for a in args.iter() {
                s.push(' ');
                pretty_expr(a, p, s);
            }
        }
        CoreKind::Prim(op, args) => {
            s.push_str(&format!("(prim {op:?}"));
            for a in args.iter() {
                s.push(' ');
                pretty_expr(a, p, s);
            }
        }
        CoreKind::Lambda(params, body) => {
            s.push_str("(fn (");
            for (i, param) in params.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                s.push_str(param);
            }
            s.push_str(") ");
            pretty_expr(body, p, s);
        }
        CoreKind::Let(name, value, body) => {
            s.push_str("(let ");
            s.push_str(name);
            s.push(' ');
            pretty_expr(value, p, s);
            s.push(' ');
            pretty_expr(body, p, s);
        }
        CoreKind::Match(scrut, arms) => {
            s.push_str("(match ");
            pretty_expr(scrut, p, s);
            for arm in arms.iter() {
                s.push_str(" (");
                pretty_pat(&arm.pat, s);
                s.push(' ');
                pretty_expr(&arm.body, p, s);
                s.push(')');
            }
        }
    }
    // Every expression node is annotated with its inline type.
    s.push_str(" : ");
    s.push_str(&p.render(&e.ty));
    s.push(')');
}

fn pretty_pat(p: &CorePat, s: &mut String) {
    match p {
        CorePat::Wild => s.push('_'),
        CorePat::Var(x) => s.push_str(x),
        CorePat::Ctor(name, args) => {
            s.push_str(name);
            if !args.is_empty() {
                s.push_str(" (");
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        s.push(' ');
                    }
                    pretty_pat(a, s);
                }
                s.push(')');
            }
        }
        CorePat::Lit(l) => push_lit(l, s),
    }
}

fn push_lit(l: &CoreLit, s: &mut String) {
    match l {
        CoreLit::Int(n) => s.push_str(&n.to_string()),
        CoreLit::Bool(b) => s.push_str(if *b { "True" } else { "False" }),
        CoreLit::Str(v) => s.push_str(&format!("{v:?}")),
        CoreLit::Unit => s.push_str("Unit"),
    }
}
```

- [ ] **Step 3: Write the snapshot corpus test (`tests/core_lowering.rs`)**

Create `tests/core_lowering.rs`:

```rust
//! Slice 5a-2 Task 3 — the typed-Core snapshot corpus (spec §5), the deliverable.
//! Reuses the 5a-1 programs (tests/typed_inference.rs), lowers each to Core, and
//! pins the result with an insta snapshot PLUS per-surface teeth and two origin
//! proofs: the lookup cross-check (direct nodes) and the derivation check
//! (synthesized Let nodes). The snapshot IS the proof of lowering-preserves-types.

use std::collections::{BTreeMap, HashSet};

use elya::core::{lower_module, pretty_typed, CoreExpr, CoreKind, CoreModule};
use elya::parse::parse_module;
use elya::span::Span;
use elya::types::{infer_typed_table, Ty, TyPrinter};
use elya::Session;

/// A rendered type is a bare type variable iff it is `[a-z][0-9]*`.
fn is_var(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_lowercase()) && cs.all(|c| c.is_ascii_digit())
}

/// Parse → infer (raw table) → lower. Panics on any parse/type/lowering error.
fn lower_src(src: &str) -> (CoreModule, BTreeMap<Span, Ty>) {
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (diags, table) = infer_typed_table(&Session::new(), &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    let core = lower_module(&m, &table).expect("lowering the corpus subset should succeed");
    (core, table)
}

/// Pre-order collection of every `CoreExpr` node in a module.
fn nodes(core: &CoreModule) -> Vec<&CoreExpr> {
    fn walk<'a>(e: &'a CoreExpr, out: &mut Vec<&'a CoreExpr>) {
        out.push(e);
        match &e.kind {
            CoreKind::Lit(_) | CoreKind::Var(_) => {}
            CoreKind::App(f, args) => {
                walk(f, out);
                for a in args.iter() {
                    walk(a, out);
                }
            }
            CoreKind::Prim(_, args) => {
                for a in args.iter() {
                    walk(a, out);
                }
            }
            CoreKind::Lambda(_, body) => walk(body, out),
            CoreKind::Let(_, value, body) => {
                walk(value, out);
                walk(body, out);
            }
            CoreKind::Match(scrut, arms) => {
                walk(scrut, out);
                for arm in arms.iter() {
                    walk(&arm.body, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for f in &core.fns {
        walk(&f.body, &mut out);
    }
    out
}

/// Render one type in isolation (ground types need no shared context).
fn render1(t: &Ty) -> String {
    TyPrinter::new().render(t)
}

/// The two origin proofs (spec §5): every direct-origin node's inline type equals
/// `node_types[span]` (lookup cross-check); every synthesized `Let` node's type
/// equals its body's (derivation check). Synthesized nodes are verified, not exempted.
fn both_origin_checks(core: &CoreModule, table: &BTreeMap<Span, Ty>) {
    for n in nodes(core) {
        match &n.kind {
            CoreKind::Let(_, _, body) => {
                assert_eq!(
                    n.ty, body.ty,
                    "synthesized Let node type must equal its body's (derivation check)"
                );
            }
            _ => {
                assert_eq!(
                    Some(&n.ty),
                    table.get(&n.span),
                    "direct Core node type must equal node_types[span] (lookup cross-check)"
                );
            }
        }
    }
}

const CORPUS: &[&str] = &[
    "fn add1(n) { n + 1 }\n",
    "fn id(x) { x }\n",
    "fn id(x) { x }\nfn u() { let a = id(5)  let b = id(True)  0 }\n",
    "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
     fn worker() { set(1) }\n\
     fn use_it() { let g = worker  g() }\n",
    "fn demo() { let f = fn(x) { x }  let a = f(1)  let b = f(True)  0 }\n",
    "type Option(a) { None, Some(a) }\n\
     fn m(o) { match o { None -> 0  Some(x) -> x } }\n",
    "linear type Tok { Tok }\nfn lin() { let t = Tok\n t }\n",
    "fn two(x, y) { let p = x  let q = y  0 }\n",
];

#[test]
fn both_origin_proofs_and_no_token_leak_across_corpus() {
    for src in CORPUS {
        let (core, table) = lower_src(src);
        both_origin_checks(&core, &table);
        let rendered = pretty_typed(&core, &mut TyPrinter::new());
        for bad in ["%r", "%e", "%s", "%t", "%v", "%row"] {
            assert!(
                !rendered.contains(bad),
                "leaked internal token {bad} in:\n{rendered}"
            );
        }
    }
}

#[test]
fn ty_printer_shares_letters_across_calls() {
    let mut p = TyPrinter::new();
    let a1 = p.render(&Ty::Var(7));
    let a2 = p.render(&Ty::Var(7));
    assert_eq!(a1, a2, "shared var must render identically across calls");
    let b = p.render(&Ty::Var(8));
    assert_ne!(a1, b, "distinct vars must render as distinct letters");
}

// --- Surface 1: monomorphic fn — no Core node carries a variable ---------------
#[test]
fn surface1_add1_fully_monomorphic() {
    let (core, _t) = lower_src("fn add1(n) { n + 1 }\n");
    assert!(
        nodes(&core).iter().all(|n| !matches!(n.ty, Ty::Var(_))),
        "monomorphic program has a var-typed Core node"
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 2: polymorphic fn — the body node stays a variable ----------------
#[test]
fn surface2_id_body_stays_polymorphic() {
    let (core, _t) = lower_src("fn id(x) { x }\n");
    let body = &core.fns[0].body;
    assert!(
        matches!(body.kind, CoreKind::Var(ref x) if x == "x"),
        "id body should be Var(\"x\"), got {:?}",
        body.kind
    );
    assert!(
        matches!(body.ty, Ty::Var(_)),
        "lowering monomorphized or errored a polymorphic node: {:?}",
        body.ty
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 3: use-site instantiation — two distinct concrete instances -------
#[test]
fn surface3_use_site_two_instances() {
    let (core, _t) =
        lower_src("fn id(x) { x }\nfn u() { let a = id(5)  let b = id(True)  0 }\n");
    let rendered: Vec<String> = nodes(&core).iter().map(|n| render1(&n.ty)).collect();
    assert!(
        rendered.iter().any(|r| r == "fn(Int) -> Int"),
        "missing Int instance callee: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|r| r == "fn(Bool) -> Bool"),
        "missing Bool instance callee: {rendered:?}"
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 4: effectful arrow — row rides inline through Ty::Fn's EffectRow ---
#[test]
fn surface4_effect_row_rides_inline() {
    let (core, _t) = lower_src(
        "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
         fn worker() { set(1) }\n\
         fn use_it() { let g = worker  g() }\n",
    );
    let rendered: Vec<String> = nodes(&core).iter().map(|n| render1(&n.ty)).collect();
    assert!(
        rendered.iter().any(|r| r.contains("State(Int)")),
        "row not materialized inline on any Core node: {rendered:?}"
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 5: let-generalized lambda (value restriction) ---------------------
#[test]
fn surface5_lambda_body_var_and_instances() {
    let (core, _t) =
        lower_src("fn demo() { let f = fn(x) { x }  let a = f(1)  let b = f(True)  0 }\n");
    let lambda_body_is_var = nodes(&core)
        .iter()
        .any(|n| matches!(&n.kind, CoreKind::Lambda(_, body) if matches!(body.ty, Ty::Var(_))));
    assert!(lambda_body_is_var, "lambda body should be a Ty::Var node");
    let rendered: Vec<String> = nodes(&core).iter().map(|n| render1(&n.ty)).collect();
    assert!(
        rendered.iter().any(|r| r == "fn(Int) -> Int"),
        "no Int instance: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|r| r == "fn(Bool) -> Bool"),
        "no Bool instance: {rendered:?}"
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 6: match — the Match node is Int ----------------------------------
#[test]
fn surface6_match_node_is_int() {
    let (core, _t) = lower_src(
        "type Option(a) { None, Some(a) }\n\
         fn m(o) { match o { None -> 0  Some(x) -> x } }\n",
    );
    let match_ty = nodes(&core)
        .iter()
        .find(|n| matches!(n.kind, CoreKind::Match(..)))
        .map(|n| render1(&n.ty));
    assert_eq!(match_ty.as_deref(), Some("Int"), "match result should be Int");
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 7: nullary ctor — the Var("Tok") node carries Con("Tok", []) ------
#[test]
fn surface7_nullary_ctor_carries_con() {
    let (core, _t) = lower_src("linear type Tok { Tok }\nfn lin() { let t = Tok\n t }\n");
    let tok_ty = nodes(&core)
        .iter()
        .find(|n| matches!(&n.kind, CoreKind::Var(x) if x == "Tok"))
        .map(|n| n.ty.clone());
    assert!(
        matches!(tok_ty, Some(Ty::Con(ref n, ref a)) if n == "Tok" && a.is_empty()),
        "Tok node should carry Con(\"Tok\", []), got {tok_ty:?}"
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Invariant: two distinct variables render as two distinct letters ----------
#[test]
fn coherence_two_distinct_vars_render_distinct_letters() {
    let (core, _t) = lower_src("fn two(x, y) { let p = x  let q = y  0 }\n");
    // Render the whole tree through ONE shared printer, then count distinct
    // single-letter var annotations — two distinct params must not collapse to one.
    let mut printer = TyPrinter::new();
    let mut letters = HashSet::new();
    for n in nodes(&core) {
        let r = printer.render(&n.ty);
        if is_var(&r) {
            letters.insert(r);
        }
    }
    assert!(
        letters.len() >= 2,
        "shared TyPrinter should give >=2 distinct var letters: {letters:?}"
    );
}
```

- [ ] **Step 4: Materialize the snapshots**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0; INSTA_UPDATE=always cargo test --test core_lowering`
Expected: PASS — `insta` writes the eight `tests/snapshots/core_lowering__surface*.snap` files (surface 1-7 plus none for the non-snapshot tests) and the assertions pass. (`INSTA_UPDATE=always` accepts new snapshots inline; no `cargo-insta` binary is required.)

- [ ] **Step 5: Review the generated snapshots — the deliverable**

Read each new `tests/snapshots/core_lowering__*.snap`. Confirm by eye (these ARE the proof):
- `surface2_id_body_stays_polymorphic`: the body renders `(var x : a)` — a bare variable, not a base type.
- `surface4_effect_row_rides_inline`: some node's annotation contains `State(Int)` — the effect row rode inline through `Ty::Fn`'s `EffectRow`.
- `surface3` / `surface5`: two distinct applications annotated `fn(Int) -> Int` and `fn(Bool) -> Bool`.
- `surface7_nullary_ctor_carries_con`: the `Tok` node renders `(var Tok : Tok)`.
- No annotation contains a `%`-prefixed internal token anywhere.

If any snapshot looks wrong, fix `pretty_typed`/lowering, delete the stale `.snap`, and re-run Step 4 — do not accept a wrong snapshot.

- [ ] **Step 6: Run the full gate**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0; cargo fmt --all && sh scripts/check.sh`
Expected: PASS (fmt clean, clippy clean, `cargo test --all` green with the now-committed snapshots). The layering test still passes unmodified — `TyPrinter` is added to `types.rs` (no new cross-module reference) and `pretty_typed` to `core.rs` (adds only `crate::types`, already present).

- [ ] **Step 7: Commit and push**

```bash
export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_INCREMENTAL=0
if sh scripts/check.sh; then
  git add src/types.rs src/core.rs tests/core_lowering.rs tests/snapshots/core_lowering__*.snap
  git commit -m "$(cat <<'EOF'
test(core): typed-Core snapshot corpus — lowering preserves types (5a-2 Task 3)

The deliverable: the reused 5a-1 corpus lowered to Core and pinned by a typed
S-expression snapshot rendered through one shared TyPrinter, plus per-surface
teeth and both origin proofs — the lookup cross-check (direct nodes:
ty == node_types[span]) and the derivation check (synthesized Let nodes:
ty == body.ty). Surface 4's row shows State(Int) inline; surface 2's body stays
Ty::Var; no rendered type leaks an internal token. TyPrinter exposes the shared
Names/write_ty render path publicly without leaking Names.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
EOF
)"
  git push origin main
fi
```

---

## Self-Review

**1. Spec coverage** (each spec section → task):

| Spec section | Covered by |
|---|---|
| §2 minimal Core datatype (Shape C, `Rc`-shared, `span` provenance, no ctor/effect-control nodes) | Task 2 Step 2 — the datatype block |
| §3 stay-polymorphic type-var form; no canonicalization; render through shared `Names` | Task 2 (headline tooth) + Task 3 (`TyPrinter`, coherence test) |
| §4 type-directed `lower` (direct rules, pattern lowering, block-flattening, typed `LowerError`) | Task 2 Step 2 — `lower_module`/`lower_block`/`lower_expr`/`lower_pat` |
| §5 typed-Core snapshot; headline tooth; lookup cross-check; derivation check; per-surface teeth; invariants | Task 2 (headline tooth) + Task 3 (all) |
| §6 non-interference (no exec, no `front_end`, no solver reach, exhaust/affine/layering untouched) | Global Constraints + Task 2 Step 4 + Task 3 Step 6 |
| §7 `src/core.rs` + `pub mod core;`; `infer_typed_table`; `pretty_typed`; no `front_end`/CLI change | Task 1 (accessor) + Task 2 (module) + Task 3 (renderer) |
| §8 testing strategy (snapshot corpus, behavior-preservation, determinism) | Task 1 parity + Task 3 corpus + gate at every task |
| §9 build order (3 tasks) | Tasks 1-3 map 1:1 |
| §12 milestone checklist (both origin proofs, layering unmodified, honest exit criteria) | `both_origin_checks` + layering step + no execution claim |

No spec requirement is left without a task. Exhaustiveness-on-Core (the decoy, §0/§11) is correctly **absent** — deferred, not implemented.

**2. Placeholder scan:** No "TBD"/"implement later"/"handle edge cases"/"similar to Task N". Every code step contains complete, compilable code. The deferred AST surface is concrete (seven explicit `Unsupported` arms), not a stub.

**3. Type consistency:** `infer_typed_table` signature is identical in Task 1 (produced), Task 2 (consumed), Task 3 (consumed). `lower_module(&Module, &BTreeMap<Span, Ty>) -> Result<CoreModule, LowerError>` identical across Task 2 (produced) and Task 3 (consumed). `TyPrinter::new()`/`render(&mut self, &Ty) -> String` and `pretty_typed(&CoreModule, &mut TyPrinter) -> String` identical between production (Task 3 Steps 1-2) and consumption (Task 3 Step 3). `CoreKind` variant shapes used in the test walker (`App(f, args)`, `Prim(_, args)`, `Lambda(_, body)`, `Let(_, value, body)`, `Match(scrut, arms)`) match the datatype declaration exactly. Verified against the real code: `Ty: Clone + Debug + PartialEq` (types.rs:20), `Span: Copy + Ord + Eq + Hash` (span.rs:3), AST field names (`Call{callee,args}`, `Binary{op,lhs,rhs}`, `Lambda{params,body}`, `Match{scrutinee,arms}`, `Stmt::Let{name,value}`, `Block{stmts,tail}`) all confirmed.

**Boundary note:** the lookup cross-check holds by construction (lowering copies `table[span]`), so it functions as a *regression pin* — a future `lower` that recomputes types instead of copying them drifts and fails here — exactly as the spec (§2, §5) frames it. It is complemented by three teeth that catch real current bugs: the headline var tooth (monomorphization/poisoning), the derivation check (Let propagation), and the snapshots (rendering).
