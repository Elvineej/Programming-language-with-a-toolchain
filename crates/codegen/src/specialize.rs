//! N7 part 1 — convention specialization and the upcast adapter (spec
//! `docs/superpowers/specs/2026-10-09-elya-n7a-convention-specialization-design.md`).
//!
//! A Core-to-Core pass, run first in `build_module`, so that every later phase
//! (the CPS partition, sites, frames, descriptors, emission) sees ordinary Core
//! whose types already say the right convention:
//!
//! 1. **Rows are monomorphized.** A reference to a row-polymorphic function
//!    (top-level, or a `let`-bound lambda) whose open row variables are
//!    instantiated at rows naming a user effect goes to a CLONE in which each
//!    such variable is substituted by the labels it was instantiated with
//!    (closed). One clone per distinct substitution; clones are scanned in turn,
//!    so recursion refers to its own clone and generic callees of a clone are
//!    specialized too.
//! 2. **Upcasts are eta-expanded.** A use of a binder at a type whose convention
//!    differs from the binder's at some covariant function layer (sub-effecting:
//!    a direct closure used where an effectful one is expected) becomes a lambda
//!    of the use type that calls the binder at its own type. That lambda is the
//!    adapter: its type names the effect, so it compiles CPS, and it calls the
//!    direct code directly.
//!
//! Deliberately LLVM-free, like `cps.rs` and `closure.rs`: the unit tests below
//! never construct a `Context`.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use elya::core::{
    CoreClause, CoreExpr, CoreFn, CoreHandle, CoreKind, CoreModule, CoreParam, CorePerform,
    CoreReturn,
};
use elya::types::{EffectLabel, EffectRow, RowTail, RowVar, Ty};

use crate::cps::needs_cps;
use crate::CodegenError;

type R<T> = Result<T, CodegenError>;

/// The code-size budget: clones per module (spec §2). Termination does not need
/// it (label sets are finite); a stated bound does.
pub(crate) const MAX_CLONES: usize = 256;

/// The builtin effect: its calls are direct, so an instantiation at it alone
/// needs no clone. Mirrors `cps::BUILTIN_EFFECT`.
const BUILTIN_EFFECT: &str = "IO";

/// A row substitution: each open row variable to the labels it is instantiated
/// with. Applied with the tail CLOSED (a convention depends only on labels).
pub(crate) type RowSubst = BTreeMap<RowVar, BTreeMap<String, EffectLabel>>;

/// Runs both halves of the pass.
pub(crate) fn run(core: &CoreModule) -> R<CoreModule> {
    let rows = specialize_rows(core)?;
    adapt_upcasts(&rows)
}

// ------------------------------------------------------------ matching --

/// Matches a generic type against its instance, recording what each open row
/// variable of the generic stands for: the instance's labels minus the
/// generic's own. Shapes that do not line up (a type variable, poison) are
/// skipped -- they carry no row.
fn match_rows(generic: &Ty, inst: &Ty, out: &mut RowSubst) {
    match (generic, inst) {
        (Ty::Fn(gp, gr, gt), Ty::Fn(ip, ir, it)) => {
            if gp.len() == ip.len() {
                for (g, i) in gp.iter().zip(ip.iter()) {
                    match_rows(g, i, out);
                }
            }
            match_row(gr, ir, out);
            match_rows(gt, it, out);
        }
        (Ty::Con(gn, ga), Ty::Con(in_, ia)) if gn == in_ && ga.len() == ia.len() => {
            for (g, i) in ga.iter().zip(ia.iter()) {
                match_rows(g, i, out);
            }
        }
        (Ty::Tuple(ga), Ty::Tuple(ia)) if ga.len() == ia.len() => {
            for (g, i) in ga.iter().zip(ia.iter()) {
                match_rows(g, i, out);
            }
        }
        _ => {}
    }
}

fn match_row(generic: &EffectRow, inst: &EffectRow, out: &mut RowSubst) {
    if let RowTail::Open(v) = generic.tail {
        let extra: BTreeMap<String, EffectLabel> = inst
            .labels
            .iter()
            .filter(|(l, _)| !generic.labels.contains_key(*l))
            .map(|(l, lab)| (l.clone(), lab.clone()))
            .collect();
        out.entry(v).or_default().extend(extra);
    }
}

/// The part of the generic-to-instance substitution that changes a
/// convention: the variables instantiated at a row naming a user effect. A
/// variable instantiated at nothing new, at `{IO}`, or at another open row
/// needs no clone -- the direct original is right for it.
pub(crate) fn user_subst(generic: &Ty, inst: &Ty) -> RowSubst {
    let mut all = RowSubst::new();
    match_rows(generic, inst, &mut all);
    all.into_iter()
        .filter(|(_, labels)| labels.keys().any(|l| l != BUILTIN_EFFECT))
        .collect()
}

/// A canonical key for a substitution: the variables and their label NAMES
/// with type arguments, in order.
fn key_of(s: &RowSubst) -> String {
    let mut k = String::new();
    for (v, labels) in s {
        k.push_str(&format!("r{v}:"));
        for (name, lab) in labels {
            k.push_str(&format!("{name}{:?},", lab.args));
        }
        k.push(';');
    }
    k
}

// -------------------------------------------------------- substitution --

pub(crate) fn subst_ty(t: &Ty, s: &RowSubst) -> Ty {
    match t {
        Ty::Fn(ps, row, ret) => Ty::Fn(
            ps.iter().map(|p| subst_ty(p, s)).collect(),
            subst_row(row, s),
            Box::new(subst_ty(ret, s)),
        ),
        Ty::Con(n, a) => Ty::Con(n.clone(), a.iter().map(|x| subst_ty(x, s)).collect()),
        Ty::Tuple(a) => Ty::Tuple(a.iter().map(|x| subst_ty(x, s)).collect()),
        other => other.clone(),
    }
}

fn subst_row(row: &EffectRow, s: &RowSubst) -> EffectRow {
    let mut labels: BTreeMap<String, EffectLabel> = row
        .labels
        .iter()
        .map(|(l, lab)| {
            (
                l.clone(),
                EffectLabel {
                    args: lab.args.iter().map(|a| subst_ty(a, s)).collect(),
                    span: lab.span,
                },
            )
        })
        .collect();
    match row.tail {
        RowTail::Open(v) => match s.get(&v) {
            Some(extra) => {
                for (l, lab) in extra {
                    labels.entry(l.clone()).or_insert_with(|| lab.clone());
                }
                EffectRow {
                    labels,
                    tail: RowTail::Closed,
                }
            }
            None => EffectRow {
                labels,
                tail: RowTail::Open(v),
            },
        },
        ref t => EffectRow {
            labels,
            tail: t.clone(),
        },
    }
}

fn subst_params(ps: &[CoreParam], s: &RowSubst) -> Rc<[CoreParam]> {
    ps.iter()
        .map(|p| CoreParam {
            name: p.name.clone(),
            ty: subst_ty(&p.ty, s),
        })
        .collect()
}

/// Every type in `e` (node types, parameters, perform types) substituted.
pub(crate) fn subst_expr(e: &CoreExpr, s: &RowSubst) -> CoreExpr {
    let go = |c: &CoreExpr| Rc::new(subst_expr(c, s));
    let all = |cs: &[CoreExpr]| -> Rc<[CoreExpr]> { cs.iter().map(|c| subst_expr(c, s)).collect() };
    let kind = match &e.kind {
        CoreKind::Lit(l) => CoreKind::Lit(l.clone()),
        CoreKind::Var(x) => CoreKind::Var(x.clone()),
        CoreKind::App(f, a) => CoreKind::App(go(f), all(a)),
        CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), all(a)),
        CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), all(a)),
        CoreKind::Prim(o, a) => CoreKind::Prim(*o, all(a)),
        CoreKind::Lambda(ps, b) => CoreKind::Lambda(subst_params(ps, s), go(b)),
        CoreKind::Let(x, v, b) => CoreKind::Let(x.clone(), go(v), go(b)),
        CoreKind::If(c, t, f) => CoreKind::If(go(c), go(t), go(f)),
        CoreKind::Match(sc, arms) => CoreKind::Match(
            go(sc),
            arms.iter()
                .map(|a| elya::core::CoreArm {
                    pat: a.pat.clone(),
                    body: subst_expr(&a.body, s),
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
                    params: subst_params(&c.params, s),
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
        })),
        CoreKind::Resume(v) => CoreKind::Resume(go(v)),
        CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
            effect: p.effect.clone(),
            op: p.op.clone(),
            op_ty: subst_ty(&p.op_ty, s),
            args: all(&p.args),
        })),
    };
    CoreExpr {
        span: e.span,
        ty: subst_ty(&e.ty, s),
        kind,
    }
}

// ----------------------------------------------------- 1. specialization --

/// A top-level function's generic signature, as a function type whose row is
/// left closed (a function's own row is not on `CoreFn`; only rows visible in
/// its parameters and result can be matched -- spec §4).
fn signature(f: &CoreFn) -> Ty {
    Ty::Fn(
        f.params.iter().map(|p| p.ty.clone()).collect(),
        EffectRow::pure(),
        Box::new(f.body.ty.clone()),
    )
}

/// The generic signature, if the function is row-polymorphic at all.
fn generic_signature(f: &CoreFn) -> Option<Ty> {
    let sig = signature(f);
    if crate::cps::ty_has_open_row(&sig) {
        Some(sig)
    } else {
        None
    }
}

/// D16's precise condition, shared with the prepass guard: does this reference
/// instantiate one of the target's open row variables at a user effect?
pub(crate) fn instantiates_user_effect(target: &CoreFn, at: &Ty) -> bool {
    match generic_signature(target) {
        Some(sig) => !user_subst(&sig, at).is_empty(),
        None => false,
    }
}

struct Specializer<'a> {
    originals: HashMap<String, &'a CoreFn>,
    /// `(function, key)` -> clone name.
    top: HashMap<(String, String), String>,
    /// Clones still to be made: (clone name, original, substitution).
    queue: Vec<(String, String, RowSubst)>,
    clones: usize,
    counter: usize,
}

/// One binder in scope during the rewrite: a `let`-bound row-polymorphic
/// lambda carries its generic type and the clones made of it so far.
struct Local {
    name: String,
    generic: Option<LocalGeneric>,
}

struct LocalGeneric {
    ty: Ty,
    /// key -> (clone binder name, substitution), in creation order.
    clones: Vec<(String, String, RowSubst)>,
}

impl<'a> Specializer<'a> {
    fn fresh(&mut self, base: &str) -> R<String> {
        self.clones += 1;
        if self.clones > MAX_CLONES {
            return Err(CodegenError::Unsupported(
                "too many convention specializations",
            ));
        }
        self.counter += 1;
        Ok(format!("{base}.spec.{}", self.counter))
    }

    fn top_clone(&mut self, g: &str, s: RowSubst) -> R<String> {
        let key = (g.to_string(), key_of(&s));
        if let Some(n) = self.top.get(&key) {
            return Ok(n.clone());
        }
        let name = self.fresh(g)?;
        self.top.insert(key, name.clone());
        self.queue.push((name.clone(), g.to_string(), s));
        Ok(name)
    }

    fn rewrite(&mut self, e: &CoreExpr, locals: &mut Vec<Local>) -> R<CoreExpr> {
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
                locals.extend(ps.iter().map(|p| Local {
                    name: p.name.clone(),
                    generic: None,
                }));
                let b = self.rewrite(b, locals);
                locals.truncate(depth);
                CoreKind::Lambda(ps.clone(), Rc::new(b?))
            }
            CoreKind::Let(x, v, b) => return self.rewrite_let(e, x, v, b, locals),
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
                    locals.extend(names.into_iter().map(|name| Local {
                        name,
                        generic: None,
                    }));
                    let body = self.rewrite(&a.body, locals);
                    locals.truncate(depth);
                    out.push(elya::core::CoreArm {
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
                    locals.extend(c.params.iter().map(|p| Local {
                        name: p.name.clone(),
                        generic: None,
                    }));
                    locals.push(Local {
                        name: crate::closure::CONT.to_string(),
                        generic: None,
                    });
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
                        locals.push(Local {
                            name: r.binder.clone(),
                            generic: None,
                        });
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

    /// The name a reference should use: a clone's when the reference
    /// instantiates a row variable at a user effect, else its own.
    fn resolve(&mut self, x: &str, at: &Ty, locals: &mut [Local]) -> R<String> {
        if let Some(i) = locals.iter().rposition(|l| l.name == x) {
            let Some(g) = locals[i].generic.as_ref() else {
                return Ok(x.to_string());
            };
            let s = user_subst(&g.ty, at);
            if s.is_empty() {
                return Ok(x.to_string());
            }
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
        let Some(target) = self.originals.get(x).copied() else {
            return Ok(x.to_string());
        };
        let Some(sig) = generic_signature(target) else {
            return Ok(x.to_string());
        };
        let s = user_subst(&sig, at);
        if s.is_empty() {
            return Ok(x.to_string());
        }
        self.top_clone(x, s)
    }

    /// A `let` of a row-polymorphic lambda: rewrite the body first (which
    /// records the instantiations its uses need), then bind one clone per
    /// instantiation right after the original, in the same scope.
    fn rewrite_let(
        &mut self,
        e: &CoreExpr,
        x: &str,
        v: &CoreExpr,
        b: &CoreExpr,
        locals: &mut Vec<Local>,
    ) -> R<CoreExpr> {
        let v2 = self.rewrite(v, locals)?;
        let generic = match &v.kind {
            CoreKind::Lambda(..) if crate::cps::ty_has_open_row(&v.ty) => Some(LocalGeneric {
                ty: v.ty.clone(),
                clones: Vec::new(),
            }),
            _ => None,
        };
        locals.push(Local {
            name: x.to_string(),
            generic,
        });
        let b2 = self.rewrite(b, locals);
        let me = locals.pop().expect("pushed above");
        let mut out = CoreExpr {
            span: e.span,
            ty: e.ty.clone(),
            kind: CoreKind::Let(x.to_string(), Rc::new(v2), Rc::new(b2?)),
        };
        if let Some(g) = me.generic {
            // The clones are bound BEFORE the original, in the scope the
            // original's value sees: a free `x` inside the lambda means an
            // OUTER binding of `x`, which a clone bound under `let x` would
            // capture instead (the review's finding 1: 1007 for 12, panics).
            // Each is a substituted copy of the ORIGINAL value, rewritten in
            // that same scope (it may itself call generics).
            for (_, name, s) in g.clones.into_iter().rev() {
                let clone = self.rewrite(&subst_expr(v, &s), locals)?;
                out = CoreExpr {
                    span: e.span,
                    ty: e.ty.clone(),
                    kind: CoreKind::Let(name, Rc::new(clone), Rc::new(out)),
                };
            }
        }
        Ok(out)
    }
}

/// Step 1: rewrite every function, make the clones the references need, and
/// rewrite those in turn, to a fixpoint (bounded by `MAX_CLONES`).
pub(crate) fn specialize_rows(core: &CoreModule) -> R<CoreModule> {
    let mut sp = Specializer {
        originals: core.fns.iter().map(|f| (f.name.clone(), f)).collect(),
        top: HashMap::new(),
        queue: Vec::new(),
        clones: 0,
        counter: 0,
    };
    let mut fns = Vec::with_capacity(core.fns.len());
    for f in &core.fns {
        fns.push(rewrite_fn(&mut sp, f.name.clone(), f, &RowSubst::new())?);
    }
    while let Some((name, orig, s)) = sp.queue.pop() {
        let f = sp.originals[orig.as_str()];
        fns.push(rewrite_fn(&mut sp, name, f, &s)?);
    }
    Ok(CoreModule {
        fns,
        types: core.types.clone(),
    })
}

fn rewrite_fn(sp: &mut Specializer<'_>, name: String, f: &CoreFn, s: &RowSubst) -> R<CoreFn> {
    let params = subst_params(&f.params, s);
    let body = subst_expr(&f.body, s);
    let mut locals: Vec<Local> = params
        .iter()
        .map(|p| Local {
            name: p.name.clone(),
            generic: None,
        })
        .collect();
    let body = sp.rewrite(&body, &mut locals)?;
    Ok(CoreFn { name, params, body })
}

// ------------------------------------------------------------ 2. adapter --

/// Do two types agree on every calling convention a caller can observe: the
/// function's own, and those of its parameters and result, recursively?
pub(crate) fn conv_eq(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Fn(pa, _, ra), Ty::Fn(pb, _, rb)) => {
            needs_cps(a) == needs_cps(b)
                && pa.len() == pb.len()
                && pa.iter().zip(pb.iter()).all(|(x, y)| conv_eq(x, y))
                && conv_eq(ra, rb)
        }
        (Ty::Con(_, xa), Ty::Con(_, xb)) | (Ty::Tuple(xa), Ty::Tuple(xb)) => {
            xa.len() == xb.len() && xa.iter().zip(xb.iter()).all(|(x, y)| conv_eq(x, y))
        }
        _ => true,
    }
}

/// Can `coerce` turn a value of type `from` into one of type `to`: is every
/// disagreement in a covariant layer, never in a parameter or under a data
/// constructor? A disagreement that is an INSTANTIATION of an open row (an
/// alias of a generic local) is left to the guard, which names it.
fn coercible(from: &Ty, to: &Ty) -> bool {
    if conv_eq(from, to) {
        return true;
    }
    if !user_subst(from, to).is_empty() {
        return false;
    }
    match (from, to) {
        (Ty::Fn(fp, _, fr), Ty::Fn(tp, _, tr)) => {
            fp.len() == tp.len()
                && fp.iter().zip(tp.iter()).all(|(x, y)| conv_eq(x, y))
                && coercible(fr, tr)
        }
        _ => false,
    }
}

struct Adapter {
    counter: usize,
}

impl Adapter {
    fn fresh(&mut self, what: &str) -> String {
        self.counter += 1;
        format!("$adapt.{what}.{}", self.counter)
    }

    /// `e` (of type `from`) as a value of type `to`. `e` is a variable or an
    /// application; anything that is not a variable is bound once first.
    fn coerce(&mut self, e: CoreExpr, from: &Ty, to: &Ty) -> R<CoreExpr> {
        if conv_eq(from, to) {
            return Ok(e);
        }
        let (Ty::Fn(fp, _, fr), Ty::Fn(tp, _, tr)) = (from, to) else {
            return Err(CodegenError::Unsupported(
                "upcast through a data constructor",
            ));
        };
        if fp.len() != tp.len() || fp.iter().zip(tp.iter()).any(|(x, y)| !conv_eq(x, y)) {
            return Err(CodegenError::Unsupported("upcast in a parameter position"));
        }
        let span = e.span;
        if !matches!(e.kind, CoreKind::Var(_)) {
            let t = self.fresh("val");
            let var = CoreExpr {
                span,
                ty: from.clone(),
                kind: CoreKind::Var(t.clone()),
            };
            let inner = self.coerce(var, from, to)?;
            return Ok(CoreExpr {
                span,
                ty: to.clone(),
                kind: CoreKind::Let(t, Rc::new(e), Rc::new(inner)),
            });
        }
        let callee = CoreExpr {
            span,
            ty: from.clone(),
            kind: e.kind,
        };
        let params: Vec<CoreParam> = tp
            .iter()
            .map(|t| CoreParam {
                name: self.fresh("arg"),
                ty: t.clone(),
            })
            .collect();
        let args: Rc<[CoreExpr]> = params
            .iter()
            .map(|p| CoreExpr {
                span,
                ty: p.ty.clone(),
                kind: CoreKind::Var(p.name.clone()),
            })
            .collect();
        let call = CoreExpr {
            span,
            ty: (**fr).clone(),
            kind: CoreKind::App(Rc::new(callee), args),
        };
        let body = self.coerce(call, fr, tr)?;
        Ok(CoreExpr {
            span,
            ty: to.clone(),
            kind: CoreKind::Lambda(params.into(), Rc::new(body)),
        })
    }

    /// Walks with every tracked binder's type (the binders
    /// `check_local_conventions` tracks); a VALUE use whose type disagrees in
    /// convention with its binder's is coerced. A callee is not a value use.
    fn walk(&mut self, e: &CoreExpr, locals: &mut Vec<(String, Option<Ty>)>) -> R<CoreExpr> {
        let kind = match &e.kind {
            CoreKind::Lit(_) => return Ok(e.clone()),
            CoreKind::Var(x) => {
                if let Some((_, Some(bound))) = locals.iter().rev().find(|(n, _)| n == x) {
                    if !conv_eq(bound, &e.ty) {
                        let bound = bound.clone();
                        let var = CoreExpr {
                            span: e.span,
                            ty: bound.clone(),
                            kind: CoreKind::Var(x.clone()),
                        };
                        return self.coerce(var, &bound, &e.ty);
                    }
                }
                return Ok(e.clone());
            }
            CoreKind::App(f, a) => {
                let args = self.walk_all(a, locals)?;
                if let CoreKind::Var(x) = &f.kind {
                    // A callee is not a value use, but inference records an
                    // upcast of its RESULT on it (`if c { f() } else { .. }`;
                    // the review's finding 2: 11 for 1511). The call keeps the
                    // binder's own convention and its result is coerced.
                    if let Some((_, Some(bound))) = locals.iter().rev().find(|(n, _)| n == x) {
                        if let (Ty::Fn(bp, _, br), Ty::Fn(up, _, ur)) = (bound, &f.ty) {
                            if !conv_eq(bound, &f.ty)
                                && coercible(bound, &f.ty)
                                && needs_cps(bound) == needs_cps(&f.ty)
                                && bp.len() == up.len()
                                && bp.iter().zip(up.iter()).all(|(p, q)| conv_eq(p, q))
                            {
                                let (bound, br, ur) =
                                    (bound.clone(), (**br).clone(), (**ur).clone());
                                let callee = CoreExpr {
                                    span: f.span,
                                    ty: bound,
                                    kind: CoreKind::Var(x.clone()),
                                };
                                let call = CoreExpr {
                                    span: e.span,
                                    ty: br.clone(),
                                    kind: CoreKind::App(Rc::new(callee), args),
                                };
                                return self.coerce(call, &br, &ur);
                            }
                        }
                    }
                    CoreKind::App(f.clone(), args)
                } else {
                    CoreKind::App(Rc::new(self.walk(f, locals)?), args)
                }
            }
            CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), self.walk_all(a, locals)?),
            CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), self.walk_all(a, locals)?),
            CoreKind::Prim(o, a) => CoreKind::Prim(*o, self.walk_all(a, locals)?),
            CoreKind::Lambda(ps, b) => {
                let depth = locals.len();
                locals.extend(ps.iter().map(|p| (p.name.clone(), Some(p.ty.clone()))));
                let b = self.walk(b, locals);
                locals.truncate(depth);
                CoreKind::Lambda(ps.clone(), Rc::new(b?))
            }
            CoreKind::Let(x, v, b) => {
                let v2 = self.walk(v, locals)?;
                locals.push((x.clone(), Some(v.ty.clone())));
                let b2 = self.walk(b, locals);
                locals.pop();
                CoreKind::Let(x.clone(), Rc::new(v2), Rc::new(b2?))
            }
            CoreKind::If(c, t, f) => CoreKind::If(
                Rc::new(self.walk(c, locals)?),
                Rc::new(self.walk(t, locals)?),
                Rc::new(self.walk(f, locals)?),
            ),
            CoreKind::Match(s, arms) => {
                let s = Rc::new(self.walk(s, locals)?);
                let mut out = Vec::with_capacity(arms.len());
                for a in arms.iter() {
                    let mut names = Vec::new();
                    crate::closure::pat_binders(&a.pat, &mut names);
                    let depth = locals.len();
                    locals.extend(names.into_iter().map(|n| (n, None)));
                    let body = self.walk(&a.body, locals);
                    locals.truncate(depth);
                    out.push(elya::core::CoreArm {
                        pat: a.pat.clone(),
                        body: body?,
                    });
                }
                CoreKind::Match(s, out.into())
            }
            CoreKind::Handle(h) => {
                let body = Rc::new(self.walk(&h.body, locals)?);
                let mut clauses = Vec::with_capacity(h.clauses.len());
                for c in h.clauses.iter() {
                    let depth = locals.len();
                    locals.extend(
                        c.params
                            .iter()
                            .map(|p| (p.name.clone(), Some(p.ty.clone()))),
                    );
                    locals.push((crate::closure::CONT.to_string(), None));
                    let cb = self.walk(&c.body, locals);
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
                        locals.push((r.binder.clone(), Some(h.body.ty.clone())));
                        let rb = self.walk(&r.body, locals);
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
                }))
            }
            CoreKind::Resume(v) => CoreKind::Resume(Rc::new(self.walk(v, locals)?)),
            CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
                effect: p.effect.clone(),
                op: p.op.clone(),
                op_ty: p.op_ty.clone(),
                args: self.walk_all(&p.args, locals)?,
            })),
        };
        Ok(CoreExpr {
            span: e.span,
            ty: e.ty.clone(),
            kind,
        })
    }

    fn walk_all(
        &mut self,
        cs: &[CoreExpr],
        locals: &mut Vec<(String, Option<Ty>)>,
    ) -> R<Rc<[CoreExpr]>> {
        let mut out = Vec::with_capacity(cs.len());
        for c in cs {
            out.push(self.walk(c, locals)?);
        }
        Ok(out.into())
    }
}

/// Step 2: eta-expand every upcast use of a tracked binder.
pub(crate) fn adapt_upcasts(core: &CoreModule) -> R<CoreModule> {
    let mut ad = Adapter { counter: 0 };
    let mut fns = Vec::with_capacity(core.fns.len());
    for f in &core.fns {
        let mut locals: Vec<(String, Option<Ty>)> = f
            .params
            .iter()
            .map(|p| (p.name.clone(), Some(p.ty.clone())))
            .collect();
        let body = ad.walk(&f.body, &mut locals)?;
        fns.push(CoreFn {
            name: f.name.clone(),
            params: f.params.clone(),
            body,
        });
    }
    Ok(CoreModule {
        fns,
        types: core.types.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_of(src: &str) -> CoreModule {
        let session = elya::Session::new();
        let (m, pd) = elya::parse::parse_module(&session, src);
        assert!(pd.is_empty(), "parse: {pd:?}");
        let (diags, table) = elya::types::infer_typed_table(&session, &m);
        assert!(diags.is_empty(), "type errors: {diags:?}");
        elya::core::lower_module(&m, &table).expect("lowers")
    }

    fn clones_of(m: &CoreModule, base: &str) -> Vec<String> {
        let prefix = format!("{base}.spec.");
        m.fns
            .iter()
            .filter(|f| f.name.starts_with(&prefix))
            .map(|f| f.name.clone())
            .collect()
    }

    /// Every name a `Var` node carries, in pre-order.
    fn vars(e: &CoreExpr, out: &mut Vec<String>) {
        if let CoreKind::Var(x) = &e.kind {
            out.push(x.clone());
        }
        match &e.kind {
            CoreKind::Lit(_) | CoreKind::Var(_) => {}
            CoreKind::App(f, a) => {
                vars(f, out);
                a.iter().for_each(|x| vars(x, out));
            }
            CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => {
                a.iter().for_each(|x| vars(x, out))
            }
            CoreKind::Perform(p) => p.args.iter().for_each(|x| vars(x, out)),
            CoreKind::Lambda(_, b) | CoreKind::Resume(b) => vars(b, out),
            CoreKind::Let(_, v, b) => {
                vars(v, out);
                vars(b, out);
            }
            CoreKind::If(c, t, f) => {
                vars(c, out);
                vars(t, out);
                vars(f, out);
            }
            CoreKind::Match(s, arms) => {
                vars(s, out);
                arms.iter().for_each(|a| vars(&a.body, out));
            }
            CoreKind::Handle(h) => {
                vars(&h.body, out);
                h.clauses.iter().for_each(|c| vars(&c.body, out));
                if let Some(r) = &h.ret {
                    vars(&r.body, out);
                }
            }
        }
    }

    fn fn_vars(m: &CoreModule, name: &str) -> Vec<String> {
        let f = m.fns.iter().find(|f| f.name == name).expect("function");
        let mut out = Vec::new();
        vars(&f.body, &mut out);
        out
    }

    const L: &str = "effect L { fn lg(x: Int) -> Int }\n";

    #[test]
    fn a_pure_instantiation_makes_no_clone() {
        let m = specialize_rows(&core_of(&format!(
            "{L}fn apply(f) {{ f(1) + 1 }}\n\
             pub fn main() -> Int {{ apply(fn(x) {{ x * 7 }}) }}\n"
        )))
        .unwrap();
        assert_eq!(clones_of(&m, "apply"), Vec::<String>::new());
        assert_eq!(m.fns.len(), 2);
    }

    #[test]
    fn one_clone_per_distinct_instantiation() {
        // `apply` at {L}, at {M}, at {L, M}, and at {L} again: three clones.
        let m = specialize_rows(&core_of(&format!(
            "{L}effect M {{ fn mm(x: Int) -> Int }}\n\
             fn apply(f: fn(Int) -> Int) -> Int {{ f(2) + 1 }}\n\
             pub fn main() -> Int {{\n\
             \x20 let a = handle {{ apply(fn(x) {{ lg(x) }}) + apply(fn(x) {{ lg(x) + 1 }}) }} with {{ L.lg(x) -> resume(x)  return(r) -> r }}\n\
             \x20 let b = handle {{ apply(fn(x) {{ mm(x) }}) }} with {{ M.mm(x) -> resume(x)  return(r) -> r }}\n\
             \x20 let c = handle {{ handle {{ apply(fn(x) {{ mm(x) + lg(x) }}) }} with {{ M.mm(x) -> resume(x)  return(r) -> r }} }} with {{ L.lg(x) -> resume(x)  return(r) -> r }}\n\
             \x20 a + b + c\n\
             }}\n"
        )))
        .unwrap();
        let clones = clones_of(&m, "apply");
        assert_eq!(clones.len(), 3, "{clones:?}");
        let refs: Vec<String> = fn_vars(&m, "main")
            .into_iter()
            .filter(|v| v.starts_with("apply"))
            .collect();
        // The two {L} uses share one clone; nothing still names the original.
        assert_eq!(refs.len(), 4, "{refs:?}");
        assert_eq!(refs[0], refs[1], "{refs:?}");
        assert!(refs.iter().all(|r| r != "apply"), "{refs:?}");
    }

    #[test]
    fn recursion_reuses_its_own_clone() {
        let m = specialize_rows(&core_of(&format!(
            "{L}fn mapl(f, n) {{ if n == 0 {{ 0 }} else {{ f(n) + mapl(f, n - 1) }} }}\n\
             pub fn main() -> Int {{\n\
             \x20 handle {{ mapl(fn(x) {{ lg(x) }}, 10) }} with {{ L.lg(x) -> resume(x)  return(r) -> r }}\n\
             }}\n"
        )))
        .unwrap();
        let clones = clones_of(&m, "mapl");
        assert_eq!(clones.len(), 1, "{clones:?}");
        let inner: Vec<String> = fn_vars(&m, &clones[0])
            .into_iter()
            .filter(|v| v.starts_with("mapl"))
            .collect();
        assert_eq!(inner, vec![clones[0].clone()]);
    }

    #[test]
    fn a_generic_callee_of_a_clone_is_specialized_too() {
        let m = specialize_rows(&core_of(&format!(
            "{L}fn apply(f: fn(Int) -> Int) -> Int {{ f(1) }}\n\
             fn apply2(f: fn(Int) -> Int) -> Int {{ apply(f) + apply(f) * 10 }}\n\
             pub fn main() -> Int {{\n\
             \x20 handle {{ apply2(fn(x) {{ lg(x) }}) }} with {{ L.lg(x) -> resume(x)  return(r) -> r }}\n\
             }}\n"
        )))
        .unwrap();
        assert_eq!(clones_of(&m, "apply2").len(), 1);
        let apply_clones = clones_of(&m, "apply");
        // `apply.spec.` does not match `apply2.spec.`, so this is `apply`'s.
        assert_eq!(apply_clones.len(), 1, "{apply_clones:?}");
        let inner: Vec<String> = fn_vars(&m, &clones_of(&m, "apply2")[0])
            .into_iter()
            .filter(|v| v.starts_with("apply"))
            .collect();
        assert_eq!(
            inner,
            vec![apply_clones[0].clone(), apply_clones[0].clone()]
        );
        // The ORIGINAL `apply2` (unused at pure here) still calls the original.
        assert!(fn_vars(&m, "apply2").iter().all(|v| v != &apply_clones[0]));
    }

    #[test]
    fn a_local_generic_lambda_gets_a_local_clone() {
        let m = specialize_rows(&core_of(
            "effect S { fn get() -> Int }\n\
             pub fn main() -> Int {\n\
             \x20 let app = fn(g) { g() + 1 }\n\
             \x20 let p = app(fn() { 3 })\n\
             \x20 p + handle { app(fn() { get() }) } with { S.get() -> resume(7)  return(r) -> r }\n\
             }\n",
        ))
        .unwrap();
        let names = fn_vars(&m, "main");
        let apps: Vec<&String> = names.iter().filter(|v| v.starts_with("app")).collect();
        // The pure use keeps `app`; the {S} use names the clone.
        assert_eq!(apps.len(), 2, "{apps:?}");
        assert_eq!(apps[0], "app", "{apps:?}");
        assert!(apps[1].starts_with("app.spec."), "{apps:?}");
        // ... and the clone is bound by a `let` (somewhere in main).
        fn binds(e: &CoreExpr, x: &str) -> bool {
            match &e.kind {
                CoreKind::Let(n, v, b) => n == x || binds(v, x) || binds(b, x),
                CoreKind::Handle(h) => binds(&h.body, x),
                CoreKind::App(f, a) => binds(f, x) || a.iter().any(|c| binds(c, x)),
                CoreKind::Prim(_, a) => a.iter().any(|c| binds(c, x)),
                _ => false,
            }
        }
        let main = m.fns.iter().find(|f| f.name == "main").unwrap();
        assert!(binds(&main.body, apps[1]));
    }

    #[test]
    fn an_upcast_becomes_an_eta_expansion() {
        let m = run(&core_of(
            "effect T { fn t() -> Int }\n\
             fn pick(c) {\n\
             \x20 let f = fn(x) { x + 1 }\n\
             \x20 let g = if c { f } else { fn(x) { t() + x } }\n\
             \x20 g(10)\n\
             }\n\
             pub fn main() -> Int { handle { pick(True) } with { T.t() -> resume(2)  return(r) -> r } }\n",
        ))
        .unwrap();
        let pick = m.fns.iter().find(|f| f.name == "pick").unwrap();
        // let f = ..; let g = if c { <here> } else { .. }; ..
        let CoreKind::Let(_, _, rest) = &pick.body.kind else {
            panic!()
        };
        let CoreKind::Let(_, g, _) = &rest.kind else {
            panic!()
        };
        let CoreKind::If(_, then, _) = &g.kind else {
            panic!()
        };
        assert!(needs_cps(&then.ty));
        let CoreKind::Lambda(ps, body) = &then.kind else {
            panic!("not eta-expanded: {:?}", then.kind)
        };
        assert_eq!(ps.len(), 1);
        let CoreKind::App(callee, _) = &body.kind else {
            panic!()
        };
        assert!(matches!(&callee.kind, CoreKind::Var(x) if x == "f"));
        assert!(!needs_cps(&callee.ty), "the adapter calls `f` direct");
    }

    #[test]
    fn the_guard_compares_every_covariant_layer() {
        // The prepass ALONE (no adapter) on the result-layer upcast: refused by
        // name, where the outermost-only guard let it through (2 for 3306).
        let core = core_of(
            "effect T { fn t() -> Int }\n\
             fn h(c) {\n\
             \x20 let f = fn() { fn(x) { x + 1 } }\n\
             \x20 let g = if c { f } else { fn() { fn(x) { t() + x } } }\n\
             \x20 let k = g()\n\
             \x20 k(1)\n\
             }\n\
             pub fn main() -> Int { handle { h(True) } with { T.t() -> resume(10)  return(r) -> r } }\n",
        );
        let fx = crate::cps::effect_facts(&core);
        let cps_fns = crate::cps_emit::cps_functions(&core, &fx);
        let err = crate::cps_emit::prepass(&core, &cps_fns).unwrap_err();
        assert!(
            matches!(
                err,
                CodegenError::Unsupported(
                    "direct function used where an effectful one is expected"
                )
            ),
            "{err:?}"
        );
        // ... and through the pass, the prepass accepts the module.
        let adapted = run(&core).unwrap();
        let fx = crate::cps::effect_facts(&adapted);
        let cps_fns = crate::cps_emit::cps_functions(&adapted, &fx);
        crate::cps_emit::prepass(&adapted, &cps_fns).expect("adapted");
    }

    #[test]
    fn the_clone_budget_is_refused_by_name() {
        // A 257-term program: the front end recurses on the host stack, so it
        // runs on a big one (as `crosscheck` runs the examples).
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(clone_budget)
            .expect("spawn")
            .join()
            .expect("the budget test passed");
    }

    fn clone_budget() {
        // `apply` instantiated at MAX_CLONES + 1 distinct effects.
        let n = MAX_CLONES + 1;
        let mut src = String::new();
        for i in 0..n {
            src.push_str(&format!("effect E{i} {{ fn op{i}() -> Int }}\n"));
        }
        src.push_str("fn apply(f: fn() -> Int) -> Int { f() }\npub fn main() -> Int {\n  0");
        for i in 0..n {
            src.push_str(&format!(
                " + handle {{ apply(fn() {{ op{i}() }}) }} with {{ E{i}.op{i}() -> resume({i})  return(r) -> r }}"
            ));
        }
        src.push_str("\n}\n");
        let err = specialize_rows(&core_of(&src)).unwrap_err();
        assert!(
            matches!(
                err,
                CodegenError::Unsupported("too many convention specializations")
            ),
            "{err:?}"
        );
        // One fewer instantiation fits the budget.
        let ok = src.replace(
            &format!(
                " + handle {{ apply(fn() {{ op{i}() }}) }} with {{ E{i}.op{i}() -> resume({i})  return(r) -> r }}",
                i = n - 1
            ),
            "",
        );
        assert_eq!(
            clones_of(&specialize_rows(&core_of(&ok)).unwrap(), "apply").len(),
            MAX_CLONES
        );
    }
}
