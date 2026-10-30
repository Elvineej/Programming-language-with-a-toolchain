//! N7 part 2 — type monomorphization (spec
//! `docs/superpowers/specs/2026-10-15-elya-n7b-type-monomorphization-design.md`).
//!
//! Two Core-to-Core passes around part 1's row specialization, so that every
//! later phase sees only ground types:
//!
//! 1. [`run`] (before part 1): a reference to a top-level function whose
//!    signature has a free type variable goes to a clone with the variables
//!    substituted by what the reference instantiates them at; a `let`-bound
//!    generic syntactic value (a lambda, a variable, a nullary constructor) is
//!    cloned per use in the scope its value sees and the original dropped.
//!    Variables left over are unconstrained and default to `Unit`, except in an
//!    effect position, which is refused by name.
//! 2. [`instantiate_types`] (after part 1): every applied parametric type
//!    becomes a ground nominal type (`List.mono.k`), built lazily -- only when
//!    a constructor or a pattern demands it -- and constructors and patterns
//!    are renamed per instance (`Cons.mono.k`).
//!
//! Deliberately LLVM-free, like `specialize.rs`: the unit tests below never
//! construct a `Context`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use elya::core::{
    let_spine, push_children, rebuild_spine, CoreArm, CoreClause, CoreCtor, CoreExpr, CoreFn,
    CoreGenericType, CoreHandle, CoreKind, CoreModule, CoreParam, CorePat, CorePerform, CoreReturn,
    CoreType,
};
use elya::span::Span;
use elya::types::{EffectLabel, EffectRow, RowTail, RowVar, Ty, TyCon};

use crate::CodegenError;

type R<T> = Result<T, CodegenError>;

/// The code-size budgets (spec §2). Termination does not depend on them.
pub(crate) const MAX_TYPE_CLONES: usize = 256;
pub(crate) const MAX_TYPE_INSTANCES: usize = 256;

/// A type-variable substitution.
pub(crate) type TySubst = BTreeMap<u32, Ty>;

// ------------------------------------------------------------- types --

/// Every type variable free in `t`, effect-label arguments included.
fn free_tvs(t: &Ty, out: &mut BTreeSet<u32>) {
    match t {
        Ty::Var(v) => {
            out.insert(*v);
        }
        Ty::Fn(ps, row, r) => {
            for p in ps {
                free_tvs(p, out);
            }
            for lab in row.labels.values() {
                for a in &lab.args {
                    free_tvs(a, out);
                }
            }
            free_tvs(r, out);
        }
        Ty::Con(_, a) | Ty::Tuple(a) => {
            for x in a {
                free_tvs(x, out);
            }
        }
        Ty::Base(_) | Ty::Error => {}
    }
}

fn has_tvs(t: &Ty) -> bool {
    let mut s = BTreeSet::new();
    free_tvs(t, &mut s);
    !s.is_empty()
}

/// Does a type variable occur in an effect label's arguments anywhere in `t`?
/// Only the arguments at parameters some operation of the effect uses count
/// (`used`, from the declarations): a phantom parameter carries no value, so
/// defaulting it is sound (the review's F2). An effect missing from `used`
/// counts every argument.
fn tv_in_label_args(t: &Ty, used: &BTreeMap<String, Vec<bool>>) -> bool {
    match t {
        Ty::Fn(ps, row, r) => {
            row.labels.iter().any(|(name, l)| {
                l.args.iter().enumerate().any(|(i, a)| {
                    has_tvs(a)
                        && used
                            .get(name)
                            .and_then(|u| u.get(i).copied())
                            .unwrap_or(true)
                })
            }) || ps.iter().any(|p| tv_in_label_args(p, used))
                || tv_in_label_args(r, used)
        }
        Ty::Con(_, a) | Ty::Tuple(a) => a.iter().any(|x| tv_in_label_args(x, used)),
        _ => false,
    }
}

/// A perform's type carries values through its parameters and result, and
/// its row through `tv_in_label_args`.
fn tv_in_op(t: &Ty, used: &BTreeMap<String, Vec<bool>>) -> bool {
    match t {
        Ty::Fn(ps, _, r) => ps.iter().any(has_tvs) || has_tvs(r) || tv_in_label_args(t, used),
        other => has_tvs(other),
    }
}

/// Matches a generic type against its instance, recording each type variable
/// of the generic. Shapes that do not line up are skipped.
pub(crate) fn match_tys(g: &Ty, i: &Ty, out: &mut TySubst) {
    match (g, i) {
        (Ty::Var(v), _) => {
            out.entry(*v).or_insert_with(|| i.clone());
        }
        (Ty::Fn(gp, gr, gt), Ty::Fn(ip, ir, it)) => {
            if gp.len() == ip.len() {
                for (a, b) in gp.iter().zip(ip.iter()) {
                    match_tys(a, b, out);
                }
            }
            for (name, gl) in &gr.labels {
                if let Some(il) = ir.labels.get(name) {
                    if gl.args.len() == il.args.len() {
                        for (a, b) in gl.args.iter().zip(il.args.iter()) {
                            match_tys(a, b, out);
                        }
                    }
                }
            }
            match_tys(gt, it, out);
        }
        (Ty::Con(gn, ga), Ty::Con(in_, ia)) if gn == in_ && ga.len() == ia.len() => {
            for (a, b) in ga.iter().zip(ia.iter()) {
                match_tys(a, b, out);
            }
        }
        (Ty::Tuple(ga), Ty::Tuple(ia)) if ga.len() == ia.len() => {
            for (a, b) in ga.iter().zip(ia.iter()) {
                match_tys(a, b, out);
            }
        }
        _ => {}
    }
}

/// `t` with every type variable mapped by `f` (rows' label arguments too).
pub(crate) fn map_vars(t: &Ty, f: &dyn Fn(u32) -> Option<Ty>) -> Ty {
    match t {
        Ty::Var(v) => f(*v).unwrap_or_else(|| t.clone()),
        Ty::Fn(ps, row, r) => Ty::Fn(
            ps.iter().map(|p| map_vars(p, f)).collect(),
            map_row(row, &|a| map_vars(a, f)),
            Box::new(map_vars(r, f)),
        ),
        Ty::Con(n, a) => Ty::Con(n.clone(), a.iter().map(|x| map_vars(x, f)).collect()),
        Ty::Tuple(a) => Ty::Tuple(a.iter().map(|x| map_vars(x, f)).collect()),
        other => other.clone(),
    }
}

pub(crate) fn map_row(row: &EffectRow, f: &dyn Fn(&Ty) -> Ty) -> EffectRow {
    EffectRow {
        labels: row
            .labels
            .iter()
            .map(|(l, lab)| {
                (
                    l.clone(),
                    EffectLabel {
                        args: lab.args.iter().map(f).collect(),
                        span: lab.span,
                    },
                )
            })
            .collect(),
        tail: row.tail.clone(),
    }
}

pub(crate) fn apply(t: &Ty, s: &TySubst) -> Ty {
    map_vars(t, &|v| s.get(&v).cloned())
}

/// The ambiguity default (spec §2): an unconstrained variable is `Unit`.
fn default_unit(t: &Ty) -> Ty {
    map_vars(t, &|_| Some(Ty::Base(TyCon::Unit)))
}

/// Every type in `e` (node types, parameters, perform and clause types) mapped.
pub(crate) fn map_expr(e: &CoreExpr, f: &dyn Fn(&Ty) -> Ty) -> CoreExpr {
    let go = |c: &CoreExpr| Rc::new(map_expr(c, f));
    let all = |cs: &[CoreExpr]| -> Rc<[CoreExpr]> { cs.iter().map(|c| map_expr(c, f)).collect() };
    let params = |ps: &[CoreParam]| -> Rc<[CoreParam]> {
        ps.iter()
            .map(|p| CoreParam {
                name: p.name.clone(),
                ty: f(&p.ty),
            })
            .collect()
    };
    let kind = match &e.kind {
        CoreKind::Lit(l) => CoreKind::Lit(l.clone()),
        CoreKind::Var(x) => CoreKind::Var(x.clone()),
        CoreKind::App(g, a) => CoreKind::App(go(g), all(a)),
        CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), all(a)),
        CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), all(a)),
        CoreKind::Prim(o, a) => CoreKind::Prim(*o, all(a)),
        CoreKind::Lambda(ps, b) => CoreKind::Lambda(params(ps), go(b)),
        // A block in a loop (spec 2026-10-30-elya-long-blocks D1).
        CoreKind::Let(..) => {
            let (spine, body) = let_spine(e);
            let lets = spine
                .iter()
                .map(|l| {
                    (
                        l.node.span,
                        f(&l.node.ty),
                        l.name.to_string(),
                        map_expr(l.value, f),
                    )
                })
                .collect();
            return rebuild_spine(lets, map_expr(body, f));
        }
        CoreKind::If(c, t, e2) => CoreKind::If(go(c), go(t), go(e2)),
        CoreKind::Match(s, arms) => CoreKind::Match(
            go(s),
            arms.iter()
                .map(|a| CoreArm {
                    pat: a.pat.clone(),
                    body: map_expr(&a.body, f),
                })
                .collect(),
        ),
        CoreKind::Handle(h) => CoreKind::Handle(Rc::new(CoreHandle {
            body: go(&h.body),
            clauses: h
                .clauses
                .iter()
                .map(|c| CoreClause {
                    effect: c.effect.clone(),
                    op: c.op.clone(),
                    params: params(&c.params),
                    body: go(&c.body),
                })
                .collect(),
            ret: h.ret.as_ref().map(|r| {
                Rc::new(CoreReturn {
                    binder: r.binder.clone(),
                    body: go(&r.body),
                })
            }),
            is_multi_declared: h.is_multi_declared,
            multi: h.multi,
            any: h.any,
        })),
        CoreKind::Resume(v) => CoreKind::Resume(go(v)),
        CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
            effect: p.effect.clone(),
            op: p.op.clone(),
            op_ty: f(&p.op_ty),
            args: all(&p.args),
        })),
    };
    CoreExpr {
        span: e.span,
        ty: f(&e.ty),
        kind,
    }
}

/// A canonical rendering of a type for clone and instance keys: spans are
/// ignored and row variables are numbered by first appearance, so two
/// references differing only in fresh row variables share a key. With
/// `rows == None` row tails are left out entirely: an INSTANCE key (the
/// review's F1 -- `Option(fn(Int) -> Int)` built where
/// `Option(fn(Int) / {|r} -> Int)` is matched must be ONE type with one tag
/// set). A tail never changes a field's representation or convention, and
/// neither do labels as such: an instance key renders a function row as its
/// convention alone (`cps::needs_cps`), since `IO` is a label and direct (f3).
fn canon(t: &Ty, rows: &mut Option<HashMap<RowVar, usize>>, out: &mut String) {
    match t {
        Ty::Var(v) => out.push_str(&format!("'{v}")),
        Ty::Base(b) => out.push_str(&format!("{b:?}")),
        Ty::Error => out.push('!'),
        Ty::Fn(ps, row, r) => {
            out.push_str("fn(");
            for p in ps {
                canon(p, rows, out);
                out.push(',');
            }
            out.push_str(")/{");
            if rows.is_none() {
                // An INSTANCE key: a row contributes its convention and
                // nothing else. Labels do not decide it either -- `IO` is a
                // label and direct (`cps::needs_cps`) -- so `Box(fn() / {IO}
                // -> Int)` built in `main` and `Box(fn() / {|r} -> Int)`
                // matched in a generic `call` were two instances, two tag
                // sets: "match failed" natively (f3). One value has one
                // convention at every use, so binder field types stay right.
                if crate::cps::needs_cps(t) {
                    out.push_str("cps");
                }
            } else {
                for (l, lab) in &row.labels {
                    out.push_str(l);
                    out.push('(');
                    for a in &lab.args {
                        canon(a, rows, out);
                        out.push(',');
                    }
                    out.push_str("),");
                }
            }
            match (&row.tail, rows.as_mut()) {
                (_, None) => {}
                (RowTail::Open(v), Some(rows)) => {
                    let n = rows.len();
                    let k = *rows.entry(*v).or_insert(n);
                    out.push_str(&format!("|r{k}"));
                }
                (t, Some(_)) => out.push_str(&format!("|{t:?}")),
            }
            out.push_str("}->");
            canon(r, rows, out);
        }
        Ty::Con(n, a) => {
            out.push_str(n);
            out.push('(');
            for x in a {
                canon(x, rows, out);
                out.push(',');
            }
            out.push(')');
        }
        Ty::Tuple(a) => {
            out.push('(');
            for x in a {
                canon(x, rows, out);
                out.push(',');
            }
            out.push(')');
        }
    }
}

/// A generic constructor's field types at `args`: the row parameters first
/// (spec 2026-10-30-elya-row-variables D6: a row parameter's argument is a
/// phantom carrying its row, and the only open tails in a declaration's field
/// are its row parameters'), then the type parameters.
fn ctor_fields_at(c: &CoreCtor, args: &[Ty]) -> Vec<Ty> {
    let s: TySubst = args
        .iter()
        .enumerate()
        .map(|(i, a)| (i as u32, a.clone()))
        .collect();
    let rows: HashMap<u32, EffectRow> = args
        .iter()
        .enumerate()
        .filter_map(|(i, a)| elya::types::phantom_row(a).map(|r| (i as u32, r.clone())))
        .collect();
    c.fields
        .iter()
        .map(|f| apply(&subst_row_params(f, &rows), &s))
        .collect()
}

/// A generic declaration's field type with its row parameters substituted (spec
/// 2026-10-30-elya-row-variables D6): row parameter `i`'s tail, `Open(i)`,
/// becomes the row its argument's phantom carries, labels joined.
fn subst_row_params(t: &Ty, rows: &HashMap<u32, EffectRow>) -> Ty {
    if rows.is_empty() {
        return t.clone();
    }
    match t {
        Ty::Fn(ps, row, r) => {
            let mut labels: BTreeMap<String, EffectLabel> = row
                .labels
                .iter()
                .map(|(k, l)| {
                    (
                        k.clone(),
                        EffectLabel {
                            args: l.args.iter().map(|a| subst_row_params(a, rows)).collect(),
                            span: l.span,
                        },
                    )
                })
                .collect();
            let tail = match &row.tail {
                RowTail::Open(v) => match rows.get(v) {
                    Some(sub) => {
                        for (k, l) in &sub.labels {
                            labels.entry(k.clone()).or_insert_with(|| l.clone());
                        }
                        sub.tail.clone()
                    }
                    None => RowTail::Open(*v),
                },
                other => other.clone(),
            };
            Ty::Fn(
                ps.iter().map(|p| subst_row_params(p, rows)).collect(),
                EffectRow { labels, tail },
                Box::new(subst_row_params(r, rows)),
            )
        }
        Ty::Con(n, a) => Ty::Con(
            n.clone(),
            a.iter().map(|x| subst_row_params(x, rows)).collect(),
        ),
        Ty::Tuple(a) => Ty::Tuple(a.iter().map(|x| subst_row_params(x, rows)).collect()),
        Ty::Var(_) | Ty::Base(_) | Ty::Error => t.clone(),
    }
}

fn key_of(s: &TySubst) -> String {
    let mut rows = Some(HashMap::new());
    let mut k = String::new();
    for (v, t) in s {
        k.push_str(&format!("{v}="));
        canon(t, &mut rows, &mut k);
        k.push(';');
    }
    k
}

/// The substitution a reference at `at` needs from a generic `g`: every free
/// variable of `g`, matched, with ambiguous variables defaulted to `Unit` (so a
/// key never carries a caller's unconstrained variable).
fn inst_subst(g: &Ty, at: &Ty) -> TySubst {
    let mut s = TySubst::new();
    match_tys(g, at, &mut s);
    let mut vs = BTreeSet::new();
    free_tvs(g, &mut vs);
    vs.into_iter()
        .map(|v| {
            let t = s.get(&v).map(default_unit).unwrap_or(Ty::Base(TyCon::Unit));
            (v, t)
        })
        .collect()
}

// ------------------------------------------------- 1. function clones --

/// A function's whole generic type, its own row included (spec 2026-10-16): a
/// type variable only in the row's label arguments is still generic, and a
/// reference's type shows what it is instantiated at.
fn signature(f: &CoreFn) -> Ty {
    Ty::Fn(
        f.params.iter().map(|p| p.ty.clone()).collect(),
        f.row.clone(),
        Box::new(f.body.ty.clone()),
    )
}

/// The value restriction's syntactic values, on Core (inference generalizes a
/// `let` only over these).
fn is_syntactic_value(e: &CoreExpr) -> bool {
    match &e.kind {
        CoreKind::Lambda(..) | CoreKind::Var(_) | CoreKind::Lit(_) => true,
        CoreKind::Ctor(_, a) => a.is_empty(),
        _ => false,
    }
}

struct Mono<'a> {
    used: &'a BTreeMap<String, Vec<bool>>,
    originals: HashMap<String, &'a CoreFn>,
    /// Top-level functions with a free type variable in their signature.
    generic: HashMap<String, Ty>,
    /// `(function, key)` -> clone name.
    top: HashMap<(String, String), String>,
    queue: Vec<(String, String, TySubst)>,
    clones: usize,
}

struct Local {
    name: String,
    generic: Option<LocalGeneric>,
}

struct LocalGeneric {
    ty: Ty,
    /// key -> (clone binder name, substitution), in creation order.
    clones: Vec<(String, String, TySubst)>,
}

impl Mono<'_> {
    fn fresh(&mut self, base: &str) -> R<String> {
        self.clones += 1;
        if self.clones > MAX_TYPE_CLONES {
            return Err(CodegenError::Unsupported("too many type specializations"));
        }
        Ok(format!("{base}.mono.{}", self.clones))
    }

    fn resolve(&mut self, x: &str, at: &Ty, locals: &mut [Local]) -> R<String> {
        if let Some(i) = locals.iter().rposition(|l| l.name == x) {
            let Some(g) = locals[i].generic.as_ref() else {
                return Ok(x.to_string());
            };
            let s = inst_subst(&g.ty, at);
            let key = key_of(&s);
            if let Some((_, n, _)) = g.clones.iter().find(|(k, _, _)| *k == key) {
                return Ok(n.clone());
            }
            let name = self.fresh(x)?;
            locals[i]
                .generic
                .as_mut()
                .expect("checked above")
                .clones
                .push((key, name.clone(), s));
            return Ok(name);
        }
        let Some(sig) = self.generic.get(x) else {
            return Ok(x.to_string());
        };
        let s = inst_subst(sig, at);
        let key = (x.to_string(), key_of(&s));
        if let Some(n) = self.top.get(&key) {
            return Ok(n.clone());
        }
        let name = self.fresh(x)?;
        self.top.insert(key, name.clone());
        self.queue.push((name.clone(), x.to_string(), s));
        Ok(name)
    }

    fn rewrite(&mut self, e: &CoreExpr, locals: &mut Vec<Local>) -> R<CoreExpr> {
        let plain = |name: &str| Local {
            name: name.to_string(),
            generic: None,
        };
        let kind = match &e.kind {
            CoreKind::Lit(_) => return Ok(e.clone()),
            CoreKind::Var(x) => CoreKind::Var(self.resolve(x, &e.ty, locals)?),
            CoreKind::App(f, a) => {
                let f = Rc::new(self.rewrite(f, locals)?);
                CoreKind::App(f, self.rewrite_all(a, locals)?)
            }
            CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), self.rewrite_all(a, locals)?),
            CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), self.rewrite_all(a, locals)?),
            CoreKind::Prim(o, a) => CoreKind::Prim(*o, self.rewrite_all(a, locals)?),
            CoreKind::Lambda(ps, b) => {
                let depth = locals.len();
                locals.extend(ps.iter().map(|p| plain(&p.name)));
                let b = self.rewrite(b, locals);
                locals.truncate(depth);
                CoreKind::Lambda(ps.clone(), Rc::new(b?))
            }
            CoreKind::Let(..) => return self.rewrite_block(e, locals),
            CoreKind::If(c, t, f) => CoreKind::If(
                Rc::new(self.rewrite(c, locals)?),
                Rc::new(self.rewrite(t, locals)?),
                Rc::new(self.rewrite(f, locals)?),
            ),
            CoreKind::Match(s, arms) => {
                let s = Rc::new(self.rewrite(s, locals)?);
                let mut out = Vec::with_capacity(arms.len());
                for a in arms.iter() {
                    let mut names = Vec::new();
                    crate::closure::pat_binders(&a.pat, &mut names);
                    let depth = locals.len();
                    locals.extend(names.iter().map(|n| plain(n)));
                    let body = self.rewrite(&a.body, locals);
                    locals.truncate(depth);
                    out.push(CoreArm {
                        pat: a.pat.clone(),
                        body: body?,
                    });
                }
                CoreKind::Match(s, out.into())
            }
            CoreKind::Handle(h) => {
                let body = Rc::new(self.rewrite(&h.body, locals)?);
                let mut clauses = Vec::with_capacity(h.clauses.len());
                for c in h.clauses.iter() {
                    let depth = locals.len();
                    locals.extend(c.params.iter().map(|p| plain(&p.name)));
                    locals.push(plain(crate::closure::CONT));
                    let cb = self.rewrite(&c.body, locals);
                    locals.truncate(depth);
                    clauses.push(CoreClause {
                        effect: c.effect.clone(),
                        op: c.op.clone(),
                        params: c.params.clone(),
                        body: Rc::new(cb?),
                    });
                }
                let ret = match &h.ret {
                    Some(r) => {
                        locals.push(plain(&r.binder));
                        let rb = self.rewrite(&r.body, locals);
                        locals.pop();
                        Some(Rc::new(CoreReturn {
                            binder: r.binder.clone(),
                            body: Rc::new(rb?),
                        }))
                    }
                    None => None,
                };
                CoreKind::Handle(Rc::new(CoreHandle {
                    body,
                    clauses: clauses.into(),
                    ret,
                    is_multi_declared: h.is_multi_declared,
                    multi: h.multi,
                    any: h.any,
                }))
            }
            CoreKind::Resume(v) => CoreKind::Resume(Rc::new(self.rewrite(v, locals)?)),
            CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
                effect: p.effect.clone(),
                op: p.op.clone(),
                op_ty: p.op_ty.clone(),
                args: self.rewrite_all(&p.args, locals)?,
            })),
        };
        Ok(CoreExpr {
            span: e.span,
            ty: e.ty.clone(),
            kind,
        })
    }

    fn rewrite_all(&mut self, cs: &[CoreExpr], locals: &mut Vec<Local>) -> R<Rc<[CoreExpr]>> {
        let mut out = Vec::with_capacity(cs.len());
        for c in cs {
            out.push(self.rewrite(c, locals)?);
        }
        Ok(out.into())
    }

    /// A block, in a loop over its `let`s (spec 2026-10-30-elya-long-blocks D1).
    /// A `let` of a generic syntactic value: the rest of the block is rewritten
    /// first (its uses record the instantiations they need), then one
    /// substituted clone of the value per instantiation is bound where the
    /// original was -- in the scope the original's value sees -- and the
    /// original is dropped: every use now names a clone. Any other `let`'s value
    /// is rewritten where it stands.
    fn rewrite_block(&mut self, e: &CoreExpr, locals: &mut Vec<Local>) -> R<CoreExpr> {
        // The scope is restored on every path, an error's too, as the
        // recursion restored it (the review's F6).
        let depth = locals.len();
        let out = self.rewrite_spine(e, locals);
        locals.truncate(depth);
        out
    }

    fn rewrite_spine(&mut self, e: &CoreExpr, locals: &mut Vec<Local>) -> R<CoreExpr> {
        enum Bound<'e> {
            Plain(Span, Ty, String, CoreExpr),
            Generic(Span, Ty, &'e CoreExpr),
        }
        let (spine, body) = let_spine(e);
        let mut bound = Vec::with_capacity(spine.len());
        for l in &spine {
            let v = l.value;
            if !(is_syntactic_value(v) && has_tvs(&v.ty)) {
                let v2 = self.rewrite(v, locals)?;
                locals.push(Local {
                    name: l.name.to_string(),
                    generic: None,
                });
                bound.push(Bound::Plain(
                    l.node.span,
                    l.node.ty.clone(),
                    l.name.to_string(),
                    v2,
                ));
            } else {
                locals.push(Local {
                    name: l.name.to_string(),
                    generic: Some(LocalGeneric {
                        ty: v.ty.clone(),
                        clones: Vec::new(),
                    }),
                });
                bound.push(Bound::Generic(l.node.span, l.node.ty.clone(), v));
            }
        }
        let mut out = self.rewrite(body, locals)?;
        for b in bound.into_iter().rev() {
            let me = locals.pop().expect("pushed above");
            match b {
                Bound::Plain(span, ty, x, v2) => {
                    out = CoreExpr {
                        span,
                        ty,
                        kind: CoreKind::Let(x, Rc::new(v2), Rc::new(out)),
                    };
                }
                Bound::Generic(span, ty, v) => {
                    let g = me.generic.expect("pushed generic");
                    for (_, name, s) in g.clones.into_iter().rev() {
                        let clone = self.rewrite(&map_expr(v, &|t| apply(t, &s)), locals)?;
                        out = CoreExpr {
                            span,
                            ty: ty.clone(),
                            kind: CoreKind::Let(name, Rc::new(clone), Rc::new(out)),
                        };
                    }
                }
            }
        }
        Ok(out)
    }

    /// One emitted function: substituted, references rewritten, then the
    /// effect-position check and the ambiguity default.
    fn emit(&mut self, name: String, f: &CoreFn, s: &TySubst) -> R<CoreFn> {
        let params: Rc<[CoreParam]> = f
            .params
            .iter()
            .map(|p| CoreParam {
                name: p.name.clone(),
                ty: apply(&p.ty, s),
            })
            .collect();
        let body = map_expr(&f.body, &|t| apply(t, s));
        let mut locals: Vec<Local> = params
            .iter()
            .map(|p| Local {
                name: p.name.clone(),
                generic: None,
            })
            .collect();
        let body = self.rewrite(&body, &mut locals)?;
        let row = map_row(&f.row, &|t| apply(t, s));
        let row_ty = Ty::Fn(Vec::new(), row.clone(), Box::new(Ty::Base(TyCon::Unit)));
        if params.iter().any(|p| tv_in_label_args(&p.ty, self.used))
            || tv_in_label_args(&row_ty, self.used)
            || effect_var(&body, self.used)
        {
            return Err(CodegenError::Unsupported(
                "type variable only in an effect row",
            ));
        }
        Ok(CoreFn {
            name,
            params: params
                .iter()
                .map(|p| CoreParam {
                    name: p.name.clone(),
                    ty: default_unit(&p.ty),
                })
                .collect(),
            body: map_expr(&body, &default_unit),
            row: map_row(&row, &default_unit),
        })
    }
}

/// A type variable left in an effect position: a perform's or a clause
/// parameter's type, or a used effect parameter's argument in any type (spec
/// §2).
fn effect_var(e: &CoreExpr, used: &BTreeMap<String, Vec<bool>>) -> bool {
    let mut found = false;
    visit(e, &mut |n| {
        if tv_in_label_args(&n.ty, used) {
            found = true;
        }
        match &n.kind {
            CoreKind::Perform(p) if tv_in_op(&p.op_ty, used) => found = true,
            CoreKind::Lambda(ps, _) if ps.iter().any(|p| tv_in_label_args(&p.ty, used)) => {
                found = true
            }
            CoreKind::Handle(h)
                if h.clauses
                    .iter()
                    .any(|c| c.params.iter().any(|p| has_tvs(&p.ty))) =>
            {
                found = true
            }
            _ => {}
        }
    });
    found
}

/// Pre-order visit of every node, left to right, with a heap stack (spec
/// 2026-10-30-elya-long-blocks D1).
fn visit(e: &CoreExpr, f: &mut dyn FnMut(&CoreExpr)) {
    let mut stack = vec![e];
    let mut children = Vec::new();
    while let Some(e) = stack.pop() {
        f(e);
        push_children(e, &mut children);
        stack.extend(children.drain(..).rev());
    }
}

/// Step 1: clone generic functions and locals per instantiation, from every
/// non-generic function, to a fixpoint (bounded by `MAX_TYPE_CLONES`). Generic
/// originals are not emitted.
pub(crate) fn run(core: &CoreModule) -> R<CoreModule> {
    let mut m = Mono {
        used: &core.effect_params_used,
        originals: core.fns.iter().map(|f| (f.name.clone(), f)).collect(),
        generic: core
            .fns
            .iter()
            .filter_map(|f| {
                let sig = signature(f);
                has_tvs(&sig).then(|| (f.name.clone(), sig))
            })
            .collect(),
        top: HashMap::new(),
        queue: Vec::new(),
        clones: 0,
    };
    let mut fns = Vec::with_capacity(core.fns.len());
    for f in &core.fns {
        if !m.generic.contains_key(&f.name) {
            fns.push(m.emit(f.name.clone(), f, &TySubst::new())?);
        } else if f.name == "main" {
            // The entry point is used once, by the shim, at its own type with
            // every variable at the ambiguity default (spec
            // 2026-10-30-elya-main-values D3): `pub fn main() { None }` is
            // `Option(Unit)`. Before, nothing used it, so it was never cloned
            // ("no `main`").
            let mut tvs = BTreeSet::new();
            free_tvs(&m.generic["main"], &mut tvs);
            let s: TySubst = tvs
                .into_iter()
                .map(|v| (v, Ty::Base(TyCon::Unit)))
                .collect();
            // Registered, so `main`'s own calls at that type are to itself (the
            // review's L3: they cloned a second, identical `main`).
            m.top.insert((f.name.clone(), key_of(&s)), f.name.clone());
            fns.push(m.emit(f.name.clone(), f, &s)?);
        }
    }
    while let Some((name, orig, s)) = m.queue.pop() {
        let f = m.originals[orig.as_str()];
        fns.push(m.emit(name, f, &s)?);
    }
    Ok(CoreModule {
        fns,
        types: core.types.clone(),
        generic_types: core.generic_types.clone(),
        effect_params_used: core.effect_params_used.clone(),
        effects: core.effects.clone(),
    })
}

// ----------------------------------------------- 2. type instances --

struct Instances<'a> {
    generic: HashMap<String, &'a CoreGenericType>,
    /// canonical `(T, args)` -> instance number.
    named: HashMap<String, usize>,
    /// instance number -> (T, args as written in Core).
    args_of: Vec<(String, Vec<Ty>)>,
    built: BTreeSet<usize>,
    out: Vec<CoreType>,
}

impl Instances<'_> {
    fn number(&mut self, t: &str, args: &[Ty]) -> usize {
        let mut key = String::new();
        canon(&Ty::Con(t.to_string(), args.to_vec()), &mut None, &mut key);
        if let Some(k) = self.named.get(&key) {
            return *k;
        }
        let k = self.args_of.len() + 1;
        self.named.insert(key, k);
        self.args_of.push((t.to_string(), args.to_vec()));
        k
    }

    fn rename_ty(&mut self, t: &Ty) -> Ty {
        match t {
            Ty::Con(n, a) if self.generic.contains_key(n) => {
                let k = self.number(n, a);
                Ty::Con(format!("{n}.mono.{k}"), Vec::new())
            }
            Ty::Con(n, a) => Ty::Con(n.clone(), a.iter().map(|x| self.rename_ty(x)).collect()),
            Ty::Fn(ps, row, r) => {
                let ps = ps.iter().map(|p| self.rename_ty(p)).collect();
                let labels = row
                    .labels
                    .iter()
                    .map(|(l, lab)| {
                        (
                            l.clone(),
                            EffectLabel {
                                args: lab.args.iter().map(|a| self.rename_ty(a)).collect(),
                                span: lab.span,
                            },
                        )
                    })
                    .collect();
                Ty::Fn(
                    ps,
                    EffectRow {
                        labels,
                        tail: row.tail.clone(),
                    },
                    Box::new(self.rename_ty(r)),
                )
            }
            Ty::Tuple(a) => Ty::Tuple(a.iter().map(|x| self.rename_ty(x)).collect()),
            other => other.clone(),
        }
    }

    /// The generic declaration's field types at these arguments (not renamed).
    fn fields_at(&self, t: &str, args: &[Ty], ctor: &str) -> Option<Vec<Ty>> {
        let g = self.generic.get(t)?;
        let c = g.ctors.iter().find(|c| c.name == ctor)?;
        Some(ctor_fields_at(c, args))
    }

    /// Builds instance `k` if it is not built yet: its constructors, renamed,
    /// with their fields substituted and renamed (which names, but does not
    /// build, the field types).
    fn demand(&mut self, k: usize) -> R<()> {
        if !self.built.insert(k) {
            return Ok(());
        }
        if self.built.len() > MAX_TYPE_INSTANCES {
            return Err(CodegenError::Unsupported("too many type instances"));
        }
        let (t, args) = self.args_of[k - 1].clone();
        let g = self.generic[t.as_str()];
        let mut ctors = Vec::with_capacity(g.ctors.len());
        for c in g.ctors.clone() {
            let fields = self
                .fields_at(&t, &args, &c.name)
                .expect("the constructor is the declaration's");
            ctors.push(CoreCtor {
                name: format!("{}.mono.{k}", c.name),
                fields: fields.iter().map(|f| self.rename_ty(f)).collect(),
            });
        }
        self.out.push(CoreType {
            name: format!("{t}.mono.{k}"),
            ctors,
        });
        Ok(())
    }

    /// Builds every instance reachable from `t` through constructor fields
    /// (spec 2026-10-17-elya-native-any-handlers D5). A non-regular type
    /// reaches unboundedly many and stops at the instance budget, by name.
    fn demand_closure(&mut self, t: &Ty, ground: &[CoreType]) -> R<()> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut work = vec![t.clone()];
        while let Some(t) = work.pop() {
            let Ty::Con(n, a) = &t else { continue };
            let mut key = String::new();
            canon(&t, &mut None, &mut key);
            if !seen.insert(key) {
                continue;
            }
            if self.generic.contains_key(n) {
                let k = self.number(n, a);
                self.demand(k)?;
                let g = self.generic[n.as_str()];
                for c in g.ctors.clone() {
                    if let Some(fields) = self.fields_at(n, a, &c.name) {
                        work.extend(fields);
                    }
                }
            } else if let Some(gt) = ground.iter().find(|g| g.name == *n) {
                for c in &gt.ctors {
                    work.extend(c.fields.iter().cloned());
                }
            }
        }
        Ok(())
    }

    /// A constructor's name at the (unrenamed) type `ty`, demanding the
    /// instance; a ground type's constructors keep their names.
    fn ctor_at(&mut self, ty: &Ty, ctor: &str) -> R<String> {
        match ty {
            Ty::Con(n, a) if self.generic.contains_key(n) => {
                let k = self.number(n, a);
                self.demand(k)?;
                Ok(format!("{ctor}.mono.{k}"))
            }
            _ => Ok(ctor.to_string()),
        }
    }

    fn pat(&mut self, p: &CorePat, scrut: &Ty, ground: &[CoreType]) -> R<CorePat> {
        Ok(match p {
            CorePat::Wild | CorePat::Var(_) | CorePat::Lit(_) => p.clone(),
            CorePat::Ctor(name, subs) => {
                let fields = match scrut {
                    Ty::Con(n, a) if self.generic.contains_key(n) => self.fields_at(n, a, name),
                    Ty::Con(n, _) => ground
                        .iter()
                        .filter(|t| t.name == *n)
                        .flat_map(|t| t.ctors.iter())
                        .find(|c| c.name == *name)
                        .map(|c| c.fields.clone()),
                    _ => None,
                };
                let new_name = self.ctor_at(scrut, name)?;
                let mut out = Vec::with_capacity(subs.len());
                for (i, sp) in subs.iter().enumerate() {
                    let ft = fields
                        .as_ref()
                        .and_then(|f| f.get(i).cloned())
                        .unwrap_or(Ty::Error);
                    out.push(self.pat(sp, &ft, ground)?);
                }
                CorePat::Ctor(new_name, out.into())
            }
        })
    }

    fn expr(&mut self, e: &CoreExpr, ground: &[CoreType]) -> R<CoreExpr> {
        let go =
            |c: &CoreExpr, me: &mut Self| -> R<Rc<CoreExpr>> { Ok(Rc::new(me.expr(c, ground)?)) };
        let kind = match &e.kind {
            CoreKind::Lit(l) => CoreKind::Lit(l.clone()),
            CoreKind::Var(x) => CoreKind::Var(x.clone()),
            CoreKind::App(f, a) => CoreKind::App(go(f, self)?, self.all(a, ground)?),
            CoreKind::Builtin(n, a) => {
                // A decode can build a value of any type its target reaches
                // (spec 2026-10-17-elya-native-any-handlers D5).
                if n == "any.decode" {
                    self.demand_closure(&e.ty, ground)?;
                }
                CoreKind::Builtin(n.clone(), self.all(a, ground)?)
            }
            CoreKind::Ctor(n, a) => {
                let n2 = self.ctor_at(&e.ty, n)?;
                CoreKind::Ctor(n2, self.all(a, ground)?)
            }
            CoreKind::Prim(o, a) => CoreKind::Prim(*o, self.all(a, ground)?),
            CoreKind::Lambda(ps, b) => CoreKind::Lambda(self.params(ps), go(b, self)?),
            // A block in a loop (spec 2026-10-30-elya-long-blocks D1), in the
            // recursion's order: the values, the body, then each `let`'s type
            // from the innermost out (instances are numbered as they are met).
            CoreKind::Let(..) => {
                let (spine, body) = let_spine(e);
                let mut values = Vec::with_capacity(spine.len());
                for l in &spine {
                    values.push(self.expr(l.value, ground)?);
                }
                let mut out = self.expr(body, ground)?;
                for (l, v) in spine.iter().zip(values).rev() {
                    out = CoreExpr {
                        span: l.node.span,
                        ty: self.rename_ty(&l.node.ty),
                        kind: CoreKind::Let(l.name.to_string(), Rc::new(v), Rc::new(out)),
                    };
                }
                return Ok(out);
            }
            CoreKind::If(c, t, f) => CoreKind::If(go(c, self)?, go(t, self)?, go(f, self)?),
            CoreKind::Match(s, arms) => {
                let s2 = go(s, self)?;
                let mut out = Vec::with_capacity(arms.len());
                for a in arms.iter() {
                    out.push(CoreArm {
                        pat: self.pat(&a.pat, &s.ty, ground)?,
                        body: self.expr(&a.body, ground)?,
                    });
                }
                CoreKind::Match(s2, out.into())
            }
            CoreKind::Handle(h) => {
                let body = go(&h.body, self)?;
                let mut clauses = Vec::with_capacity(h.clauses.len());
                for c in h.clauses.iter() {
                    clauses.push(CoreClause {
                        effect: c.effect.clone(),
                        op: c.op.clone(),
                        params: self.params(&c.params),
                        body: go(&c.body, self)?,
                    });
                }
                let ret = match &h.ret {
                    Some(r) => Some(Rc::new(CoreReturn {
                        binder: r.binder.clone(),
                        body: go(&r.body, self)?,
                    })),
                    None => None,
                };
                CoreKind::Handle(Rc::new(CoreHandle {
                    body,
                    clauses: clauses.into(),
                    ret,
                    is_multi_declared: h.is_multi_declared,
                    multi: h.multi,
                    any: h.any,
                }))
            }
            CoreKind::Resume(v) => CoreKind::Resume(go(v, self)?),
            CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
                effect: p.effect.clone(),
                op: p.op.clone(),
                op_ty: self.rename_ty(&p.op_ty),
                args: self.all(&p.args, ground)?,
            })),
        };
        Ok(CoreExpr {
            span: e.span,
            ty: self.rename_ty(&e.ty),
            kind,
        })
    }

    fn all(&mut self, cs: &[CoreExpr], ground: &[CoreType]) -> R<Rc<[CoreExpr]>> {
        let mut out = Vec::with_capacity(cs.len());
        for c in cs {
            out.push(self.expr(c, ground)?);
        }
        Ok(out.into())
    }

    fn params(&mut self, ps: &[CoreParam]) -> Rc<[CoreParam]> {
        ps.iter()
            .map(|p| CoreParam {
                name: p.name.clone(),
                ty: self.rename_ty(&p.ty),
            })
            .collect()
    }
}

/// Step 3: applied parametric types become ground nominal instances, built on
/// demand; constructors and patterns are renamed per instance.
pub(crate) fn instantiate_types(core: &CoreModule) -> R<CoreModule> {
    let mut ins = Instances {
        generic: core
            .generic_types
            .iter()
            .map(|g| (g.name.clone(), g))
            .collect(),
        named: HashMap::new(),
        args_of: Vec::new(),
        built: BTreeSet::new(),
        out: Vec::new(),
    };
    let mut types: Vec<CoreType> = core
        .types
        .iter()
        .map(|t| CoreType {
            name: t.name.clone(),
            ctors: t
                .ctors
                .iter()
                .map(|c| CoreCtor {
                    name: c.name.clone(),
                    fields: c.fields.iter().map(|f| ins.rename_ty(f)).collect(),
                })
                .collect(),
        })
        .collect();
    let mut fns = Vec::with_capacity(core.fns.len());
    for f in &core.fns {
        let row_ty = ins.rename_ty(&Ty::Fn(
            Vec::new(),
            f.row.clone(),
            Box::new(Ty::Base(TyCon::Unit)),
        ));
        let Ty::Fn(_, row, _) = row_ty else {
            unreachable!("rename_ty keeps a function type a function type")
        };
        fns.push(CoreFn {
            name: f.name.clone(),
            params: ins.params(&f.params),
            body: ins.expr(&f.body, &core.types)?,
            row,
        });
    }
    types.extend(ins.out);
    Ok(CoreModule {
        fns,
        types,
        generic_types: Vec::new(),
        effect_params_used: core.effect_params_used.clone(),
        effects: core.effects.clone(),
    })
}

/// A parametric constructor's field types at `scrut`'s arguments, for part 1's
/// pattern-binder tracking (which runs before the instances exist).
pub(crate) fn generic_fields(
    generic: &[CoreGenericType],
    scrut: &Ty,
    ctor: &str,
) -> Option<Vec<Ty>> {
    let Ty::Con(n, args) = scrut else {
        return None;
    };
    let g = generic.iter().find(|g| g.name == *n)?;
    let c = g.ctors.iter().find(|c| c.name == ctor)?;
    Some(ctor_fields_at(c, args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use elya::core::lower_module;
    use elya::parse::parse_module;
    use elya::Session;

    fn core_of(src: &str) -> CoreModule {
        let s = Session::new();
        let (m, pd) = parse_module(&s, src);
        assert!(pd.is_empty(), "{pd:?}");
        let (d, table) = elya::types::infer_typed_table(&s, &m);
        assert!(d.is_empty(), "{d:?}");
        lower_module(&m, &table).expect("lowers")
    }

    /// The whole N7 type pipeline, as `build_module` runs it.
    fn full(src: &str) -> R<CoreModule> {
        let m = run(&core_of(src))?;
        let m = crate::specialize::run(&m)?;
        instantiate_types(&m)
    }

    fn all_types(m: &CoreModule) -> Vec<Ty> {
        let mut out = Vec::new();
        for f in &m.fns {
            out.extend(f.params.iter().map(|p| p.ty.clone()));
            visit(&f.body, &mut |n| {
                out.push(n.ty.clone());
                if let CoreKind::Perform(p) = &n.kind {
                    out.push(p.op_ty.clone());
                }
            });
        }
        for t in &m.types {
            for c in &t.ctors {
                out.extend(c.fields.iter().cloned());
            }
        }
        out
    }

    fn names(m: &CoreModule) -> Vec<String> {
        let mut v: Vec<String> = m.fns.iter().map(|f| f.name.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn no_type_variable_survives() {
        for src in [
            "fn twice(f, x) { f(f(x)) }\n\
             pub fn main() -> Int { twice(fn(n) { n * 3 + 1 }, 5) }\n",
            "type List(a) { Nil, Cons(a, List(a)) }\n\
             fn map(f, l) { match l { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(f, t)) } }\n\
             fn range(a, b) { if a >= b { Nil } else { Cons(a, range(a + 1, b)) } }\n\
             pub fn main() -> Int { let xs = map(fn(n) { Cons(n, Nil) }, range(0, 3))  0 }\n",
            "fn mk(x) { let m = fn(y) { x }  m }\n\
             fn pair(a, b) { fn(k) { k(a, b) } }\n\
             pub fn main() -> Int { let p = pair(3, True)  p(fn(a, b) { if b { a } else { 0 } }) + mk(5)(True) }\n",
        ] {
            let m = full(src).expect("monomorphizes");
            for t in all_types(&m) {
                assert!(!has_tvs(&t), "a type variable survived: {t:?} in {src}");
                if let Ty::Con(_, a) = &t {
                    assert!(a.is_empty(), "an applied type survived: {t:?}");
                }
            }
        }
    }

    #[test]
    fn one_clone_per_instantiation_shared_across_sites() {
        let m = run(&core_of(
            "fn id(x) { x }\n\
             fn a() -> Int { id(1) }\n\
             fn b() -> Int { id(2) }\n\
             pub fn main() -> Int { if id(True) { a() + b() } else { 0 } }\n",
        ))
        .expect("monomorphizes");
        let ids: Vec<String> = names(&m)
            .into_iter()
            .filter(|n| n.starts_with("id"))
            .collect();
        assert_eq!(
            ids.len(),
            2,
            "one clone at Int, one at Bool, no original: {ids:?}"
        );
        assert!(ids.iter().all(|n| n.starts_with("id.mono.")), "{ids:?}");
    }

    #[test]
    fn a_generic_local_nullary_ctor_is_cloned_per_use() {
        let m = run(&core_of(
            "type List(a) { Nil, Cons(a, List(a)) }\n\
             fn len(l) { match l { Nil -> 0  Cons(_, t) -> 1 + len(t) } }\n\
             pub fn main() -> Int { let n = Nil  len(Cons(1, n)) * 10 + len(Cons(True, Cons(False, n))) }\n",
        ))
        .expect("monomorphizes");
        let main = m.fns.iter().find(|f| f.name == "main").expect("main");
        let mut lets = Vec::new();
        visit(&main.body, &mut |n| {
            if let CoreKind::Let(x, v, _) = &n.kind {
                lets.push((x.clone(), v.ty.clone()));
            }
        });
        assert_eq!(
            lets.len(),
            2,
            "two clones of `n`, the original dropped: {lets:?}"
        );
        assert!(
            lets.iter().all(|(x, _)| x.starts_with("n.mono.")),
            "{lets:?}"
        );
        assert_ne!(lets[0].1, lets[1].1, "at two different types: {lets:?}");
    }

    #[test]
    fn ambiguous_variables_default_to_unit_once() {
        let m = run(&core_of(
            "type List(a) { Nil, Cons(a, List(a)) }\n\
             fn seven(l) { 7 }\n\
             pub fn main() -> Int { seven(Nil) + seven(Nil) }\n",
        ))
        .expect("monomorphizes");
        let sevens: Vec<&CoreFn> = m
            .fns
            .iter()
            .filter(|f| f.name.starts_with("seven"))
            .collect();
        assert_eq!(sevens.len(), 1, "one clone for both call sites");
        assert_eq!(
            sevens[0].params[0].ty,
            Ty::Con("List".into(), vec![Ty::Base(TyCon::Unit)])
        );
    }

    /// The clone a module makes of `name`, with its own row.
    fn only_clone<'m>(m: &'m CoreModule, name: &str) -> &'m CoreFn {
        let cs: Vec<&CoreFn> = m
            .fns
            .iter()
            .filter(|f| f.name.starts_with(&format!("{name}.mono.")))
            .collect();
        assert_eq!(cs.len(), 1, "{:?}", names(m));
        assert!(
            !m.fns.iter().any(|f| f.name == name),
            "the original is not emitted"
        );
        cs[0]
    }

    #[test]
    fn a_row_only_type_variable_is_instantiated_from_the_reference() {
        // Was `a_row_only_type_variable_is_refused_by_name` (N7 part 2): `s`
        // occurs only in `dup`'s own row, which Core did not carry. Since
        // `CoreFn::row` (spec 2026-10-16) the reference at
        // `fn() / {St(String)} -> Int` instantiates it.
        let m = run(&core_of(
            "effect St(s) { fn get() -> s  fn put(v: s) -> Int }\n\
             fn dup() { put(get()) }\n\
             pub fn main() -> Int {\n\
             \x20 handle { dup()  5 } with { St.get() -> resume(\"x\")  St.put(v) -> resume(0)  return(r) -> r }\n\
             }\n",
        ))
        .expect("monomorphizes");
        let dup = only_clone(&m, "dup");
        assert_eq!(dup.row.labels["St"].args, vec![Ty::Base(TyCon::Str)]);
    }

    #[test]
    fn a_row_only_type_variable_through_generic_callees_is_instantiated() {
        // Was `a_row_only_type_variable_through_a_generic_callee_is_refused`:
        // the `String` crosses `dup3` only through its callees' rows; `dup3`'s
        // own row now says `St(s)`, so it is generic and cloned at `String`,
        // and so are `getter` and `putter` from inside the clone.
        let m = run(&core_of(
            "effect St(s) { fn get() -> s  fn put(v: s) -> Int }\n\
             fn getter() { get() }\n\
             fn putter(v) { put(v) }\n\
             fn dup3() { let v = getter()  putter(v) }\n\
             pub fn main() -> Int {\n\
             \x20 handle { dup3() } with { St.get() -> resume(\"x\")  St.put(v) -> resume(0)  return(r) -> r }\n\
             }\n",
        ))
        .expect("monomorphizes");
        for n in ["dup3", "getter", "putter"] {
            assert_eq!(
                only_clone(&m, n).row.labels["St"].args,
                vec![Ty::Base(TyCon::Str)],
                "{n}"
            );
        }
    }

    #[test]
    fn type_clone_budget() {
        // 300 generic functions, each used once at `Int`: 300 clones.
        let mut src = String::new();
        for i in 0..300 {
            src.push_str(&format!(
                "fn id{i}(x) {{ x }}\nfn u{i}() -> Int {{ id{i}(1) }}\n"
            ));
        }
        src.push_str("pub fn main() -> Int { 0 }\n");
        let err = run(&core_of(&src)).unwrap_err();
        assert!(
            matches!(
                err,
                CodegenError::Unsupported("too many type specializations")
            ),
            "{err:?}"
        );
    }

    #[test]
    fn a_recursive_generic_main_calls_itself() {
        // The main-values review's L3: the default instance was not registered,
        // so `main`'s own call cloned a second, identical `main`.
        let m = run(&core_of(
            "type Opt(a) { No, Yes(a) }\n\
             fn depth() -> Int { 3 }\n\
             pub fn main() { if depth() > 5 { main() } else { No } }\n",
        ))
        .expect("monomorphizes");
        let names: Vec<&str> = m.fns.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["depth", "main"]);
    }

    #[test]
    fn instances_are_lazy_for_a_non_regular_type() {
        let m = full(
            "type List(a) { Nil, Cons(a, List(a)) }\n\
             type Nest(a) { NNil, NCons(a, Nest(List(a))) }\n\
             fn depth(n) { match n { NNil -> 0  NCons(_, _) -> 1 } }\n\
             pub fn main() -> Int { depth(NCons(1, NCons(Cons(2, Nil), NNil))) }\n",
        )
        .expect("monomorphizes");
        let mut built: Vec<&str> = m.types.iter().map(|t| t.name.as_str()).collect();
        built.sort();
        // Built: Nest(Int), Nest(List(Int)), List(Int), and
        // Nest(List(List(Int))) -- the innermost `NNil` is BUILT at it (the plan
        // predicted 3 and missed that constructor). Named and not built:
        // List(List(Int)) and Nest(List(List(List(Int)))), the field types of
        // the deepest instance; eager instantiation would never stop.
        assert_eq!(built.len(), 4, "{built:?}");
        assert!(built.iter().all(|n| n.contains(".mono.")), "{built:?}");
    }

    #[test]
    fn type_instance_budget() {
        // 300 parametric types, each instance demanded by a constructor.
        let mut src = String::new();
        for i in 0..300 {
            src.push_str(&format!(
                "type T{i}(a) {{ C{i}(a) }}\nfn k{i}() -> Int {{ let v = C{i}(1)  0 }}\n"
            ));
        }
        src.push_str("pub fn main() -> Int { 0 }\n");
        let err = full(&src).unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("too many type instances")),
            "{err:?}"
        );
    }
}
