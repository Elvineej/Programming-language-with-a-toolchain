//! Core IR (Slice 5a-2): a minimal, corpus-derived intermediate representation
//! with each node's zonked type carried inline (Shape C). Built by a type-directed
//! fold over the typed AST that clones types out of the frozen `Span → Ty` table —
//! it performs no inference. Core is not executed here (spec §6).

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::rc::Rc;

use crate::ast::{BinOp, Block, Decl, Expr, Module, PatLit, Pattern, Stmt, TypeAnn};
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
    /// local or top-level fn name (a bare constructor is `Ctor`, not `Var`).
    Var(String),
    /// callee + args (function / effect-op application).
    App(Rc<CoreExpr>, Rc<[CoreExpr]>),
    /// A compiler builtin, syntactically distinct from `App` just as `Ctor` is.
    Builtin(String, Rc<[CoreExpr]>),
    /// ADT construction, syntactically distinct from application (5b-4 §3.2):
    /// `Ctor(name, fields)` — `name` is the constructor.
    Ctor(String, Rc<[CoreExpr]>),
    /// `+` etc. — the primitive operator set is reused verbatim from the AST.
    Prim(BinOp, Rc<[CoreExpr]>),
    /// uncurried params, each carrying the type recorded at its span (5b-6 §4,
    /// obligation T3); the block body is flattened into a single expression.
    Lambda(Rc<[CoreParam]>, Rc<CoreExpr>),
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

/// One function parameter, carrying the type inference recorded at its span
/// (Slice 5b-3 §3.3). A named struct rather than a `(String, Ty)` pair because
/// the affine work (4d-2) identified a parameter multiplicity annotation as a
/// plausible future field — a tuple would have to be rewritten to grow one.
#[derive(Clone, Debug)]
pub struct CoreParam {
    pub name: String,
    pub ty: Ty,
}

#[derive(Clone, Debug)]
pub struct CoreFn {
    pub name: String,
    pub params: Rc<[CoreParam]>,
    /// The body's root type IS the return type: `lower_block` propagates the
    /// block type onto the synthesized `Let` spine, so a separate `ret` field
    /// would be a second source of truth (Slice 5b-3 §3.3).
    pub body: CoreExpr,
}

#[derive(Clone, Debug)]
pub struct CoreType {
    pub name: String,
    /// Constructors in declaration order — the index IS the tag (5b-4 §2.2).
    pub ctors: Vec<CoreCtor>,
}

#[derive(Clone, Debug)]
pub struct CoreCtor {
    pub name: String,
    /// One type per field; len == arity.
    pub fields: Vec<Ty>,
}

#[derive(Clone, Debug)]
pub struct CoreModule {
    pub fns: Vec<CoreFn>,
    pub types: Vec<CoreType>,
}

/// Lowering failure. The deferred AST surface is a *typed boundary*, not a panic:
/// an out-of-subset AST node is `Unsupported`, a node absent from the frozen table
/// is `Untyped`. Neither fires over the fixed corpus (spec §4).
#[derive(Clone, Debug, PartialEq)]
pub enum LowerError {
    Unsupported(&'static str),
    Untyped(Span),
}

/// Lower a whole module in two passes. Pass 1 collects constructor names and the
/// ADT declarations (so a function body lowered in pass 2 sees every constructor
/// regardless of declaration order). Pass 2 lowers each `Decl::Fn` to a `CoreFn`.
/// `Decl::Effect` is not re-homed (spec §2 — exhaustiveness-on-Core is out of scope).
pub fn lower_module(module: &Module, table: &BTreeMap<Span, Ty>) -> Result<CoreModule, LowerError> {
    let mut ctor_names: HashSet<String> = HashSet::new();
    let mut types = Vec::new();
    for d in &module.decls {
        if let Decl::Type(t) = &d.node {
            // Collect every constructor name regardless of arity, so a constructor
            // application — even of a deferred parametric ADT — lowers to `Ctor`.
            for v in &t.variants {
                ctor_names.insert(v.node.name.clone());
            }
            // Only GROUND (monomorphic) ADTs are recorded in `types`: the back end
            // is monomorphic-only (5b-4 §6), and a parametric field type would need
            // a type-parameter substitution this slice deliberately defers. The field
            // elaboration below is therefore param-free and never sees a type param.
            if t.params.is_empty() {
                let mut ctors = Vec::with_capacity(t.variants.len());
                for v in &t.variants {
                    let vd = &v.node;
                    let fields = vd.fields.iter().map(|f| ann_to_ty(&f.node)).collect();
                    ctors.push(CoreCtor {
                        name: vd.name.clone(),
                        fields,
                    });
                }
                types.push(CoreType {
                    name: t.name.clone(),
                    ctors,
                });
            }
        }
        if let Decl::Effect(e) = &d.node {
            // D1 / spec §9.2: codegen decides "does this row mention a
            // user-declared effect?" by asking `label != "IO"`. That read is
            // sound only while `IO` cannot be user-declared, so it is refused
            // here — the last pass that still sees the declaration.
            if e.name == "IO" {
                return Err(LowerError::Unsupported("effect IO"));
            }
        }
    }

    let mut fns = Vec::new();
    for d in &module.decls {
        if let Decl::Fn(f) = &d.node {
            let mut params = Vec::with_capacity(f.params.len());
            for p in &f.params {
                let ty = table
                    .get(&p.span)
                    .cloned()
                    .ok_or(LowerError::Untyped(p.span))?;
                params.push(CoreParam {
                    name: p.node.name.clone(),
                    ty,
                });
            }
            let body = lower_block(&f.body.node, table, &ctor_names)?;
            fns.push(CoreFn {
                name: f.name.clone(),
                params: params.into(),
                body,
            });
        }
    }
    Ok(CoreModule { fns, types })
}

/// Elaborate a (monomorphic) ADT field type annotation to a `Ty`. Param-free by
/// design (5b-4 §6): base names map to their `Base`, any other name is an ADT
/// reference and maps to `Ty::Con` with its args elaborated. No type parameters
/// reach here, so there is no substitution.
fn ann_to_ty(a: &TypeAnn) -> Ty {
    match a.name.as_str() {
        "Int" => Ty::int(),
        "Float" => Ty::float(),
        "Bool" => Ty::bool(),
        "String" => Ty::str(),
        "Unit" => Ty::unit(),
        other => {
            let args = a.args.iter().map(|x| ann_to_ty(&x.node)).collect();
            Ty::Con(other.to_string(), args)
        }
    }
}

/// Flatten a block into a right-nested `Let` spine terminating in the lowered tail.
/// The `Let` nodes are the only *synthesized* Core nodes: their type is derived by
/// propagation (`ty = body.ty`), their span is the originating statement's span.
fn lower_block(
    block: &Block,
    table: &BTreeMap<Span, Ty>,
    ctors: &HashSet<String>,
) -> Result<CoreExpr, LowerError> {
    let tail = block
        .tail
        .as_ref()
        .ok_or(LowerError::Unsupported("block without tail expression"))?;
    let mut acc = lower_expr(&tail.node, tail.span, table, ctors)?;
    for stmt in block.stmts.iter().rev() {
        let (name, value) = match &stmt.node {
            Stmt::Let { name, value } => (
                name.clone(),
                lower_expr(&value.node, value.span, table, ctors)?,
            ),
            // A non-tail expression statement: a discarded binding — no separate
            // sequencing node is needed for the corpus.
            Stmt::Expr(e) => ("_".to_string(), lower_expr(&e.node, e.span, table, ctors)?),
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
fn lower_expr(
    e: &Expr,
    span: Span,
    table: &BTreeMap<Span, Ty>,
    ctors: &HashSet<String>,
) -> Result<CoreExpr, LowerError> {
    let ty = table.get(&span).cloned().ok_or(LowerError::Untyped(span))?;
    let kind = match e {
        Expr::Int(n) => CoreKind::Lit(CoreLit::Int(*n)),
        Expr::Bool(b) => CoreKind::Lit(CoreLit::Bool(*b)),
        Expr::Str(v) => CoreKind::Lit(CoreLit::Str(v.clone())),
        Expr::Unit => CoreKind::Lit(CoreLit::Unit),
        Expr::Var(x) => {
            // A bare constructor name is construction of a nullary ctor (5b-4
            // §3.2), distinct from a variable reference.
            if ctors.contains(x) {
                CoreKind::Ctor(x.clone(), Rc::from([]))
            } else {
                CoreKind::Var(x.clone())
            }
        }
        Expr::Call { callee, args } => {
            if let Expr::Qualified { module, name } = &callee.node {
                if module == "io" && name == "println" {
                    let mut lowered = Vec::with_capacity(args.len());
                    for a in args.iter() {
                        lowered.push(lower_expr(&a.node, a.span, table, ctors)?);
                    }
                    return Ok(CoreExpr {
                        span,
                        ty,
                        kind: CoreKind::Builtin("io.println".to_string(), lowered.into()),
                    });
                }
            }
            // A saturated constructor call lowers to a distinct Ctor node, not an
            // App. Check the callee name before lowering it as a variable.
            if let Expr::Var(name) = &callee.node {
                if ctors.contains(name) {
                    let mut lowered = Vec::with_capacity(args.len());
                    for a in args.iter() {
                        lowered.push(lower_expr(&a.node, a.span, table, ctors)?);
                    }
                    return Ok(CoreExpr {
                        span,
                        ty,
                        kind: CoreKind::Ctor(name.clone(), lowered.into()),
                    });
                }
            }
            let f = lower_expr(&callee.node, callee.span, table, ctors)?;
            let mut lowered = Vec::with_capacity(args.len());
            for a in args.iter() {
                lowered.push(lower_expr(&a.node, a.span, table, ctors)?);
            }
            CoreKind::App(Rc::new(f), lowered.into())
        }
        Expr::Binary { op, lhs, rhs } => {
            let l = lower_expr(&lhs.node, lhs.span, table, ctors)?;
            let r = lower_expr(&rhs.node, rhs.span, table, ctors)?;
            CoreKind::Prim(*op, vec![l, r].into())
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            let c = lower_expr(&cond.node, cond.span, table, ctors)?;
            let t = lower_block(&then_block.node, table, ctors)?;
            let e = lower_block(&else_block.node, table, ctors)?;
            CoreKind::If(Rc::new(c), Rc::new(t), Rc::new(e))
        }
        Expr::Lambda { params, body } => {
            // Same shape as the top-level fn parameter loop above: the type comes
            // out of the frozen table keyed by the parameter's own span. A lambda
            // parameter missing from the table is `Untyped`, not a guess.
            let mut ps = Vec::with_capacity(params.len());
            for p in params {
                let ty = table
                    .get(&p.span)
                    .cloned()
                    .ok_or(LowerError::Untyped(p.span))?;
                ps.push(CoreParam {
                    name: p.node.name.clone(),
                    ty,
                });
            }
            let b = lower_block(&body.node, table, ctors)?;
            CoreKind::Lambda(ps.into(), Rc::new(b))
        }
        Expr::Match { scrutinee, arms } => {
            let s = lower_expr(&scrutinee.node, scrutinee.span, table, ctors)?;
            let mut lowered = Vec::with_capacity(arms.len());
            for arm in arms.iter() {
                let body = lower_expr(&arm.node.body.node, arm.node.body.span, table, ctors)?;
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
    for t in &m.types {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str("(type ");
        s.push_str(&t.name);
        for c in &t.ctors {
            s.push_str(" (");
            s.push_str(&c.name);
            for f in &c.fields {
                s.push(' ');
                s.push_str(&p.render(f));
            }
            s.push(')');
        }
        s.push(')');
    }
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
            s.push_str(&param.name);
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
        CoreKind::Builtin(name, args) => {
            s.push_str("(builtin ");
            s.push_str(name);
            for a in args.iter() {
                s.push(' ');
                pretty_expr(a, p, s);
            }
        }
        CoreKind::Ctor(name, fields) => {
            s.push_str("(ctor ");
            s.push_str(name);
            for f in fields.iter() {
                s.push(' ');
                pretty_expr(f, p, s);
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
                s.push_str(&param.name);
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
