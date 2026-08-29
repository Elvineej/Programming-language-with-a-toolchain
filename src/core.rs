//! Core IR (Slice 5a-2): a minimal, corpus-derived intermediate representation
//! with each node's zonked type carried inline (Shape C). Built by a type-directed
//! fold over the typed AST that clones types out of the frozen `Span → Ty` table —
//! it performs no inference. Core is not executed here (spec §6).

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::ast::{BinOp, Block, Decl, Expr, Module, PatLit, Pattern, Stmt};
use crate::span::Span;
use crate::types::Ty;
use crate::types::TyPrinter;

/// One Core expression: its source provenance, its inline type (Shape C), and shape.
/// Every child pointer is `Rc`-shared, so a subtree is a refcount clone, not a deep
/// copy — mirroring the AST; every later Core consumer inherits it.
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
    /// `if cond { .. } else { .. }` — always two-branch, because the AST's
    /// `else_block` is not optional. Both branches carry the same type (the
    /// checker unified them), which is what lets the back end join them with a
    /// single-typed `phi`.
    If(Rc<CoreExpr>, Rc<CoreExpr>, Rc<CoreExpr>),
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
pub fn lower_module(module: &Module, table: &BTreeMap<Span, Ty>) -> Result<CoreModule, LowerError> {
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
        .ok_or(LowerError::Unsupported("block without tail expression"))?;
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
    let ty = table.get(&span).cloned().ok_or(LowerError::Untyped(span))?;
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
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            let c = lower_expr(&cond.node, cond.span, table)?;
            let t = lower_block(&then_block.node, table)?;
            let e = lower_block(&else_block.node, table)?;
            CoreKind::If(Rc::new(c), Rc::new(t), Rc::new(e))
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
        CoreKind::If(cond, then_e, else_e) => {
            s.push_str("(if ");
            pretty_expr(cond, p, s);
            s.push(' ');
            pretty_expr(then_e, p, s);
            s.push(' ');
            pretty_expr(else_e, p, s);
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
