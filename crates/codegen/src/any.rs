//! `with any` natively: operation-generic handlers by expansion (spec
//! `docs/superpowers/specs/2026-10-17-elya-native-any-handlers-design.md`).
//!
//! A Core-to-Core pass, run after row specialization (`specialize::run`), when
//! the row a `with any` handler handles is a closed set of labels, and before
//! `mono::instantiate_types`, so declared types still carry their arguments.
//!
//! - **D1.** The handle becomes a plain one with a clause per operation of every
//!   handled label but `IO`; each clause is a copy of the generic clause with the
//!   rigid answer type replaced by the operation's.
//! - **D2.** The operation value is gone: in a copy the operation is known, so
//!   `op.*` and `==` on the binder are rewritten over the clause's parameters.
//!   Any other use of the binder is refused by name.
//! - **D7.** The expansion is one-shot; `is_multi_declared` is stamped from the
//!   handled operations' declarations.
//!
//! Deliberately LLVM-free: the unit tests below never construct a `Context`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use elya::ast::BinOp;
use elya::core::{
    CoreArm, CoreClause, CoreEffect, CoreExpr, CoreFn, CoreHandle, CoreKind, CoreLit, CoreModule,
    CoreOp, CoreParam, CorePat, CorePerform, CoreReturn,
};
use elya::span::Span;
use elya::types::{EffectLabel, EffectRow, RowTail, Ty, TyCon};

use crate::mono::{apply, map_expr, TySubst};
use crate::CodegenError;

type R<T> = Result<T, CodegenError>;

/// The builtin effect, never dispatched to a handler (the earlier spec's D2).
const BUILTIN_EFFECT: &str = "IO";

/// The refusal of D2, by name.
pub(crate) const OP_VALUE_REFUSAL: &str = "operation value used other than by op.* or ==";

/// Runs the pass over every function.
pub(crate) fn run(core: &CoreModule) -> R<CoreModule> {
    let mut cx = Cx {
        effects: core.effects.iter().map(|e| (e.name.clone(), e)).collect(),
        fresh: 0,
        rigid_fns: core
            .fns
            .iter()
            .map(|f| (f.name.clone(), fn_rigids(f)))
            .filter(|(_, r)| !r.is_empty())
            .collect(),
        clones: HashMap::new(),
        queue: Vec::new(),
    };
    let mut fns = Vec::with_capacity(core.fns.len());
    for f in &core.fns {
        fns.push(CoreFn {
            name: f.name.clone(),
            params: f.params.clone(),
            body: cx.expr(&f.body)?,
            row: f.row.clone(),
        });
    }
    // The review's R1: a generic function `mono::run` cloned at a rigid answer
    // type is cloned again per operation, at that operation's answer type, as
    // the clause is. Requests are served in order, so a clone's source exists.
    let mut done: Vec<CoreFn> = Vec::new();
    let mut i = 0;
    while i < cx.queue.len() {
        let (src, rigid, answer, name) = cx.queue[i].clone();
        i += 1;
        let f = fns
            .iter()
            .chain(done.iter())
            .find(|f| f.name == src)
            .cloned()
            .ok_or(CodegenError::Unsupported(
                "callee is not a top-level function",
            ))?;
        let ty = |t: &Ty| replace_con(t, &rigid, &answer);
        let row = match ty(&Ty::Fn(
            Vec::new(),
            f.row.clone(),
            Box::new(Ty::Base(TyCon::Unit)),
        )) {
            Ty::Fn(_, r, _) => r,
            _ => f.row.clone(),
        };
        let body = map_expr(&f.body, &ty);
        let body = cx.redirect(&body, &rigid, &answer)?;
        done.push(CoreFn {
            name,
            params: f
                .params
                .iter()
                .map(|p| CoreParam {
                    name: p.name.clone(),
                    ty: ty(&p.ty),
                })
                .collect(),
            body,
            row,
        });
    }
    if done.len() > crate::mono::MAX_TYPE_CLONES {
        return Err(CodegenError::Unsupported("too many function clones"));
    }
    fns.extend(done);
    // What is still at a rigid type is unreachable: every reference to it was
    // redirected (a rigid type never leaves its clause, E0435).
    fns.retain(|f| fn_rigids(f).is_empty());
    Ok(CoreModule {
        fns,
        types: core.types.clone(),
        generic_types: core.generic_types.clone(),
        effect_params_used: core.effect_params_used.clone(),
        effects: core.effects.clone(),
    })
}

/// The rigid answer types (`'tN`) a type mentions.
fn ty_rigids(t: &Ty, out: &mut BTreeSet<String>) {
    match t {
        Ty::Con(n, a) => {
            if a.is_empty() && n.starts_with('\'') {
                out.insert(n.clone());
            }
            a.iter().for_each(|x| ty_rigids(x, out));
        }
        Ty::Fn(ps, row, r) => {
            ps.iter().for_each(|p| ty_rigids(p, out));
            for l in row.labels.values() {
                l.args.iter().for_each(|a| ty_rigids(a, out));
            }
            ty_rigids(r, out);
        }
        Ty::Tuple(a) => a.iter().for_each(|x| ty_rigids(x, out)),
        _ => {}
    }
}

/// The rigid answer types a function's SIGNATURE mentions: a clone `mono::run`
/// made at one. (A function whose body holds a `with any` clause mentions its
/// rigid inside, never in its signature, E0435.)
fn fn_rigids(f: &CoreFn) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for p in f.params.iter() {
        ty_rigids(&p.ty, &mut out);
    }
    for l in f.row.labels.values() {
        l.args.iter().for_each(|a| ty_rigids(a, &mut out));
    }
    ty_rigids(&f.body.ty, &mut out);
    out
}

struct Cx<'a> {
    effects: HashMap<String, &'a CoreEffect>,
    /// Numbers the clause parameters the expansion introduces.
    fresh: usize,
    /// Functions at rigid answer types, and which (R1).
    rigid_fns: HashMap<String, BTreeSet<String>>,
    /// `(function, rigid, answer)` -> its clone's name (R1).
    clones: HashMap<(String, String, String), String>,
    /// Clones to build: `(source, rigid, answer, name)`.
    queue: Vec<(String, String, Ty, String)>,
}

/// One operation, as a clause copy knows it.
struct OpAt<'a> {
    /// `E.op`.
    named: String,
    effect: String,
    decl: &'a CoreOp,
    /// The clause's parameters: the operation's arguments.
    args: Vec<CoreParam>,
    /// The answer type at this instantiation.
    answer: Ty,
    /// The handled label (the effect's arguments), for the perform's type.
    label: EffectLabel,
}

impl<'a> Cx<'a> {
    /// Bottom-up: an `any` handle's children are expanded before it, so an inner
    /// handle's copies are copied and rewritten with the outer clause.
    fn expr(&mut self, e: &CoreExpr) -> R<CoreExpr> {
        let kind = match &e.kind {
            CoreKind::Lit(_) | CoreKind::Var(_) => e.kind.clone(),
            CoreKind::App(f, a) => CoreKind::App(Rc::new(self.expr(f)?), self.all(a)?),
            CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), self.all(a)?),
            CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), self.all(a)?),
            CoreKind::Prim(o, a) => CoreKind::Prim(*o, self.all(a)?),
            CoreKind::Lambda(ps, b) => CoreKind::Lambda(ps.clone(), Rc::new(self.expr(b)?)),
            CoreKind::Let(x, v, b) => {
                CoreKind::Let(x.clone(), Rc::new(self.expr(v)?), Rc::new(self.expr(b)?))
            }
            CoreKind::If(c, t, f) => CoreKind::If(
                Rc::new(self.expr(c)?),
                Rc::new(self.expr(t)?),
                Rc::new(self.expr(f)?),
            ),
            CoreKind::Match(s, arms) => {
                let mut out = Vec::with_capacity(arms.len());
                for a in arms.iter() {
                    out.push(CoreArm {
                        pat: a.pat.clone(),
                        body: self.expr(&a.body)?,
                    });
                }
                CoreKind::Match(Rc::new(self.expr(s)?), out.into())
            }
            CoreKind::Handle(h) => {
                let mut clauses = Vec::with_capacity(h.clauses.len());
                for c in h.clauses.iter() {
                    clauses.push(CoreClause {
                        effect: c.effect.clone(),
                        op: c.op.clone(),
                        params: c.params.clone(),
                        body: Rc::new(self.expr(&c.body)?),
                    });
                }
                let ret = match &h.ret {
                    Some(r) => Some(Rc::new(CoreReturn {
                        binder: r.binder.clone(),
                        body: Rc::new(self.expr(&r.body)?),
                    })),
                    None => None,
                };
                let inner = CoreHandle {
                    body: Rc::new(self.expr(&h.body)?),
                    clauses: clauses.into(),
                    ret,
                    is_multi_declared: h.is_multi_declared,
                    multi: h.multi,
                    any: h.any,
                };
                if inner.any {
                    CoreKind::Handle(Rc::new(self.expand(&inner)?))
                } else {
                    CoreKind::Handle(Rc::new(inner))
                }
            }
            CoreKind::Resume(v) => CoreKind::Resume(Rc::new(self.expr(v)?)),
            CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
                effect: p.effect.clone(),
                op: p.op.clone(),
                op_ty: p.op_ty.clone(),
                args: self.all(&p.args)?,
            })),
        };
        Ok(CoreExpr {
            span: e.span,
            ty: e.ty.clone(),
            kind,
        })
    }

    fn all(&mut self, cs: &[CoreExpr]) -> R<Rc<[CoreExpr]>> {
        let mut out = Vec::with_capacity(cs.len());
        for c in cs {
            out.push(self.expr(c)?);
        }
        Ok(out.into())
    }

    /// R1: every reference to a function at `rigid` goes to its clone at
    /// `answer`, requested on first use.
    fn redirect(&mut self, e: &CoreExpr, rigid: &str, answer: &Ty) -> R<CoreExpr> {
        let mut err = None;
        let out = map_vars_named(e, &mut |x: &str| {
            let rs = self.rigid_fns.get(x)?;
            if !rs.contains(rigid) {
                return None;
            }
            let key = (x.to_string(), rigid.to_string(), format!("{answer:?}"));
            if let Some(n) = self.clones.get(&key) {
                return Some(n.clone());
            }
            self.fresh += 1;
            let name = format!("{x}.any.{}", self.fresh);
            let mut left = rs.clone();
            left.remove(rigid);
            if !left.is_empty() {
                self.rigid_fns.insert(name.clone(), left);
            }
            self.clones.insert(key, name.clone());
            self.queue.push((
                x.to_string(),
                rigid.to_string(),
                answer.clone(),
                name.clone(),
            ));
            if self.queue.len() > crate::mono::MAX_TYPE_CLONES {
                err = Some(CodegenError::Unsupported("too many function clones"));
            }
            Some(name)
        });
        match err {
            Some(e) => Err(e),
            None => Ok(out),
        }
    }

    /// D1: the plain handle an `any` handle stands for.
    fn expand(&mut self, h: &CoreHandle) -> R<CoreHandle> {
        let [clause] = &h.clauses[..] else {
            return Err(CodegenError::Unsupported(
                "`with any` handler without exactly one operation clause",
            ));
        };
        let [binder] = &clause.params[..] else {
            return Err(CodegenError::Unsupported(
                "`with any` clause without exactly one binder",
            ));
        };
        let Some((rigid, handled)) = op_type(&binder.ty) else {
            return Err(CodegenError::Unsupported(
                "`with any` clause binder is not an operation",
            ));
        };
        let mut clauses = Vec::new();
        let mut is_multi_declared = false;
        for (label, lab) in &handled.labels {
            if label == BUILTIN_EFFECT {
                continue;
            }
            let eff = *self.effects.get(label).ok_or(CodegenError::Unsupported(
                "`with any` over an undeclared effect",
            ))?;
            is_multi_declared |= eff.multi;
            let s: TySubst = lab
                .args
                .iter()
                .enumerate()
                .map(|(i, a)| (i as u32, a.clone()))
                .collect();
            for decl in &eff.ops {
                let args: Vec<CoreParam> = decl
                    .params
                    .iter()
                    .map(|t| {
                        self.fresh += 1;
                        CoreParam {
                            name: format!("any.arg.{}", self.fresh),
                            ty: apply(t, &s),
                        }
                    })
                    .collect();
                let at = OpAt {
                    named: format!("{label}.{}", decl.name),
                    effect: label.clone(),
                    decl,
                    args: args.clone(),
                    answer: apply(&decl.ret, &s),
                    label: lab.clone(),
                };
                let typed = map_expr(&clause.body, &|t| replace_con(t, &rigid, &at.answer));
                let typed = self.redirect(&typed, &rigid, &at.answer)?;
                let body = rewrite(&typed, &binder.name, &at)?;
                clauses.push(CoreClause {
                    effect: label.clone(),
                    op: decl.name.clone(),
                    params: args.into(),
                    body: Rc::new(body),
                });
            }
        }
        Ok(CoreHandle {
            body: h.body.clone(),
            clauses: clauses.into(),
            ret: h.ret.clone(),
            is_multi_declared,
            multi: false,
            any: false,
        })
    }
}

/// `e` with each `Var(x)` renamed where `f(x)` says. Only top-level clone names
/// are renamed (they contain dots, which no local name can), so binders need no
/// tracking.
fn map_vars_named(e: &CoreExpr, f: &mut dyn FnMut(&str) -> Option<String>) -> CoreExpr {
    let go = |c: &CoreExpr, f: &mut dyn FnMut(&str) -> Option<String>| map_vars_named(c, f);
    let kind = match &e.kind {
        CoreKind::Var(x) => match f(x) {
            Some(n) => CoreKind::Var(n),
            None => e.kind.clone(),
        },
        CoreKind::Lit(_) => e.kind.clone(),
        CoreKind::App(g, a) => {
            CoreKind::App(Rc::new(go(g, f)), a.iter().map(|x| go(x, f)).collect())
        }
        CoreKind::Builtin(n, a) => {
            CoreKind::Builtin(n.clone(), a.iter().map(|x| go(x, f)).collect())
        }
        CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), a.iter().map(|x| go(x, f)).collect()),
        CoreKind::Prim(o, a) => CoreKind::Prim(*o, a.iter().map(|x| go(x, f)).collect()),
        CoreKind::Lambda(ps, b) => CoreKind::Lambda(ps.clone(), Rc::new(go(b, f))),
        CoreKind::Let(x, v, b) => CoreKind::Let(x.clone(), Rc::new(go(v, f)), Rc::new(go(b, f))),
        CoreKind::If(c, t, e2) => {
            CoreKind::If(Rc::new(go(c, f)), Rc::new(go(t, f)), Rc::new(go(e2, f)))
        }
        CoreKind::Match(s, arms) => CoreKind::Match(
            Rc::new(go(s, f)),
            arms.iter()
                .map(|a| CoreArm {
                    pat: a.pat.clone(),
                    body: go(&a.body, f),
                })
                .collect(),
        ),
        CoreKind::Handle(h) => CoreKind::Handle(Rc::new(CoreHandle {
            body: Rc::new(go(&h.body, f)),
            clauses: h
                .clauses
                .iter()
                .map(|c| CoreClause {
                    effect: c.effect.clone(),
                    op: c.op.clone(),
                    params: c.params.clone(),
                    body: Rc::new(go(&c.body, f)),
                })
                .collect(),
            ret: h.ret.as_ref().map(|r| {
                Rc::new(CoreReturn {
                    binder: r.binder.clone(),
                    body: Rc::new(go(&r.body, f)),
                })
            }),
            is_multi_declared: h.is_multi_declared,
            multi: h.multi,
            any: h.any,
        })),
        CoreKind::Resume(v) => CoreKind::Resume(Rc::new(go(v, f))),
        CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
            effect: p.effect.clone(),
            op: p.op.clone(),
            op_ty: p.op_ty.clone(),
            args: p.args.iter().map(|x| go(x, f)).collect(),
        })),
    };
    CoreExpr {
        span: e.span,
        ty: e.ty.clone(),
        kind,
    }
}

/// `$Op(t, fn() / H -> t)`: the rigid answer type's name and `H`.
fn op_type(t: &Ty) -> Option<(String, EffectRow)> {
    let Ty::Con(n, args) = t else { return None };
    if n != "$Op" {
        return None;
    }
    let [Ty::Con(rigid, none), Ty::Fn(_, row, _)] = &args[..] else {
        return None;
    };
    if !none.is_empty() {
        return None;
    }
    Some((rigid.clone(), row.clone()))
}

/// `t` with the rigid type `name` replaced by `with` (rows' arguments too).
fn replace_con(t: &Ty, name: &str, with: &Ty) -> Ty {
    match t {
        Ty::Con(n, a) if n == name && a.is_empty() => with.clone(),
        Ty::Con(n, a) => Ty::Con(
            n.clone(),
            a.iter().map(|x| replace_con(x, name, with)).collect(),
        ),
        Ty::Fn(ps, row, r) => Ty::Fn(
            ps.iter().map(|p| replace_con(p, name, with)).collect(),
            crate::mono::map_row(row, &|a| replace_con(a, name, with)),
            Box::new(replace_con(r, name, with)),
        ),
        Ty::Tuple(a) => Ty::Tuple(a.iter().map(|x| replace_con(x, name, with)).collect()),
        other => other.clone(),
    }
}

fn node(span: Span, ty: Ty, kind: CoreKind) -> CoreExpr {
    CoreExpr { span, ty, kind }
}

fn str_lit(span: Span, s: &str) -> CoreExpr {
    node(
        span,
        Ty::Base(TyCon::Str),
        CoreKind::Lit(CoreLit::Str(s.to_string())),
    )
}

fn var(span: Span, p: &CoreParam) -> CoreExpr {
    node(span, p.ty.clone(), CoreKind::Var(p.name.clone()))
}

/// `any.show(v, before, after)`: `v` in literal syntax; reaching a function
/// fails with `before <what> after`.
fn show(span: Span, v: CoreExpr, before: &str, after: &str) -> CoreExpr {
    node(
        span,
        Ty::Base(TyCon::Str),
        CoreKind::Builtin(
            "any.show".to_string(),
            Rc::from([v, str_lit(span, before), str_lit(span, after)]),
        ),
    )
}

fn concat(span: Span, a: CoreExpr, b: CoreExpr) -> CoreExpr {
    node(
        span,
        Ty::Base(TyCon::Str),
        CoreKind::Prim(BinOp::Concat, Rc::from([a, b])),
    )
}

/// D2: `e` with every use of the binder `o` rewritten for the operation `at`.
fn rewrite(e: &CoreExpr, o: &str, at: &OpAt) -> R<CoreExpr> {
    let sp = e.span;
    let go = |c: &CoreExpr| -> R<Rc<CoreExpr>> { Ok(Rc::new(rewrite(c, o, at)?)) };
    let all =
        |cs: &[CoreExpr]| -> R<Rc<[CoreExpr]>> { cs.iter().map(|c| rewrite(c, o, at)).collect() };
    let is_o = |c: &CoreExpr| matches!(&c.kind, CoreKind::Var(x) if x == o);
    let kind = match &e.kind {
        CoreKind::Var(x) if x == o => return Err(CodegenError::Unsupported(OP_VALUE_REFUSAL)),
        CoreKind::Lit(_) | CoreKind::Var(_) => e.kind.clone(),
        CoreKind::Builtin(n, a) if n.starts_with("op.") && a.first().is_some_and(is_o) => {
            let rest = all(&a[1..])?;
            return builtin(sp, &e.ty, n, &rest, at);
        }
        CoreKind::Prim(op @ (BinOp::Eq | BinOp::Ne), a)
            if a.len() == 2 && is_o(&a[0]) && is_o(&a[1]) =>
        {
            return Ok(op_eq(sp, *op, at));
        }
        CoreKind::App(f, a) => CoreKind::App(go(f)?, all(a)?),
        CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), all(a)?),
        CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), all(a)?),
        CoreKind::Prim(p, a) => CoreKind::Prim(*p, all(a)?),
        CoreKind::Lambda(ps, b) => {
            let body = if ps.iter().any(|p| p.name == o) {
                b.clone()
            } else {
                go(b)?
            };
            CoreKind::Lambda(ps.clone(), body)
        }
        CoreKind::Let(x, v, b) => {
            let body = if x == o { b.clone() } else { go(b)? };
            CoreKind::Let(x.clone(), go(v)?, body)
        }
        CoreKind::If(c, t, f) => CoreKind::If(go(c)?, go(t)?, go(f)?),
        CoreKind::Match(s, arms) => {
            let mut out = Vec::with_capacity(arms.len());
            for a in arms.iter() {
                let body = if binds(&a.pat, o) {
                    a.body.clone()
                } else {
                    rewrite(&a.body, o, at)?
                };
                out.push(CoreArm {
                    pat: a.pat.clone(),
                    body,
                });
            }
            CoreKind::Match(go(s)?, out.into())
        }
        CoreKind::Handle(h) => {
            let mut clauses = Vec::with_capacity(h.clauses.len());
            for c in h.clauses.iter() {
                let body = if c.params.iter().any(|p| p.name == o) {
                    c.body.clone()
                } else {
                    go(&c.body)?
                };
                clauses.push(CoreClause {
                    effect: c.effect.clone(),
                    op: c.op.clone(),
                    params: c.params.clone(),
                    body,
                });
            }
            let ret = match &h.ret {
                Some(r) => Some(Rc::new(CoreReturn {
                    binder: r.binder.clone(),
                    body: if r.binder == o {
                        r.body.clone()
                    } else {
                        go(&r.body)?
                    },
                })),
                None => None,
            };
            CoreKind::Handle(Rc::new(CoreHandle {
                body: go(&h.body)?,
                clauses: clauses.into(),
                ret,
                is_multi_declared: h.is_multi_declared,
                multi: h.multi,
                any: h.any,
            }))
        }
        CoreKind::Resume(v) => CoreKind::Resume(go(v)?),
        CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
            effect: p.effect.clone(),
            op: p.op.clone(),
            op_ty: p.op_ty.clone(),
            args: all(&p.args)?,
        })),
    };
    Ok(node(sp, e.ty.clone(), kind))
}

fn binds(p: &CorePat, o: &str) -> bool {
    match p {
        CorePat::Var(x) => x == o,
        CorePat::Ctor(_, subs) => subs.iter().any(|s| binds(s, o)),
        CorePat::Wild | CorePat::Lit(_) => false,
    }
}

/// `op.<n>(o, rest..)` for the operation `at`, at the node type `ty`.
fn builtin(sp: Span, ty: &Ty, n: &str, rest: &[CoreExpr], at: &OpAt) -> R<CoreExpr> {
    let named = &at.named;
    Ok(match (n, rest) {
        ("op.name", []) => str_lit(sp, named),
        ("op.show", []) => {
            let before = format!("op.show: an argument of `{named}` is ");
            let mut acc = str_lit(sp, &format!("{named}("));
            for (i, a) in at.args.iter().enumerate() {
                if i > 0 {
                    acc = concat(sp, acc, str_lit(sp, ", "));
                }
                acc = concat(sp, acc, show(sp, var(sp, a), &before, ", which is not data"));
            }
            concat(sp, acc, str_lit(sp, ")"))
        }
        ("op.perform", []) => {
            let labels: BTreeMap<String, EffectLabel> =
                [(at.effect.clone(), at.label.clone())].into_iter().collect();
            node(
                sp,
                at.answer.clone(),
                CoreKind::Perform(Rc::new(CorePerform {
                    effect: at.effect.clone(),
                    op: at.decl.name.clone(),
                    op_ty: Ty::Fn(
                        at.args.iter().map(|a| a.ty.clone()).collect(),
                        EffectRow {
                            labels,
                            tail: RowTail::Closed,
                        },
                        Box::new(at.answer.clone()),
                    ),
                    args: at.args.iter().map(|a| var(sp, a)).collect(),
                })),
            )
        }
        ("op.encode", [v]) => show(
            sp,
            v.clone(),
            &format!("op.encode: the answer of `{named}` is "),
            ", which is not data",
        ),
        ("op.decode", [s]) => match &at.decl.ret_mentions_param {
            // The evaluator's S2 refusal, at the same point (spec D4).
            Some(p) => node(
                sp,
                at.answer.clone(),
                CoreKind::Builtin(
                    "any.fail".to_string(),
                    Rc::from([
                        s.clone(),
                        str_lit(
                            sp,
                            &format!(
                                "op.decode: the answer type `{}` mentions the effect's parameter `{p}`, \
                                 which is not known at run time, for an answer of `{named}`",
                                at.decl.ret_display
                            ),
                        ),
                    ]),
                ),
            ),
            None => node(
                sp,
                at.answer.clone(),
                CoreKind::Builtin(
                    "any.decode".to_string(),
                    Rc::from([
                        s.clone(),
                        str_lit(sp, &at.decl.ret_display),
                        str_lit(sp, named),
                    ]),
                ),
            ),
        },
        _ => {
            let _ = ty;
            return Err(CodegenError::Unsupported("op builtin with the wrong arguments"));
        }
    })
}

/// `o == o` / `o != o` (the evaluator's N3: an operation equals itself when its
/// arguments do). An operation always equals itself, unless the comparison
/// reaches a function in an argument, which stops the evaluator's `==` by name
/// (the review's R2): each argument is checked for one at run time.
fn op_eq(sp: Span, op: BinOp, at: &OpAt) -> CoreExpr {
    let bool_ty = Ty::Base(TyCon::Bool);
    let mut acc = node(sp, bool_ty.clone(), CoreKind::Lit(CoreLit::Bool(true)));
    for a in &at.args {
        let check = node(
            sp,
            bool_ty.clone(),
            CoreKind::Builtin("any.check".to_string(), Rc::from([var(sp, a)])),
        );
        acc = node(
            sp,
            bool_ty.clone(),
            CoreKind::Prim(BinOp::And, Rc::from([acc, check])),
        );
    }
    match op {
        BinOp::Ne => node(
            sp,
            bool_ty.clone(),
            CoreKind::Prim(
                BinOp::Ne,
                Rc::from([acc, node(sp, bool_ty, CoreKind::Lit(CoreLit::Bool(true)))]),
            ),
        ),
        _ => acc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_of(src: &str) -> CoreModule {
        let session = elya::Session::new();
        let (m, pd) = elya::parse::parse_module(&session, src);
        assert!(pd.is_empty(), "{pd:?}");
        let (diags, table) = elya::types::infer_typed_table(&session, &m);
        assert!(diags.is_empty(), "{diags:?}");
        elya::core::lower_module(&m, &table).expect("lowers")
    }

    /// The pipeline up to and including this pass.
    fn expanded(src: &str) -> R<CoreModule> {
        let named = crate::anf::run(&core_of(src));
        let m = crate::mono::run(&named)?;
        let s = crate::specialize::run(&m)?;
        run(&s)
    }

    fn handles(e: &CoreExpr, out: &mut Vec<Rc<CoreHandle>>) {
        match &e.kind {
            CoreKind::Handle(h) => {
                out.push(h.clone());
                handles(&h.body, out);
                for c in h.clauses.iter() {
                    handles(&c.body, out);
                }
            }
            CoreKind::Let(_, v, b) => {
                handles(v, out);
                handles(b, out);
            }
            CoreKind::Lambda(_, b) => handles(b, out),
            CoreKind::App(f, a) => {
                handles(f, out);
                a.iter().for_each(|x| handles(x, out));
            }
            _ => {}
        }
    }

    const REC: &str = "effect W { fn roll(s: Int) -> Int  fn now() -> Int }\n\
        effect A { fn ask() -> Bool }\n\
        fn rec(body) { handle { body() } with any { op(o) -> { io.println(op.show(o))  resume(op.perform(o)) } } }\n";

    #[test]
    fn one_clause_per_handled_operation() {
        let src = format!(
            "{REC}pub fn main() -> Int {{\n\
               handle {{ handle {{ rec(fn() {{ if ask() {{ roll(6) }} else {{ now() }} }}) }} with {{\n\
                 W.roll(s) -> resume(s)  W.now() -> resume(1)\n\
               }} }} with {{ A.ask() -> resume(True) }}\n\
             }}\n"
        );
        let m = expanded(&src).expect("expands");
        let mut hs = Vec::new();
        for f in m.fns.iter().filter(|f| f.name.starts_with("rec")) {
            handles(&f.body, &mut hs);
        }
        assert!(!hs.is_empty());
        let clone = hs
            .iter()
            .find(|h| h.clauses.len() == 3)
            .expect("a clone handling W and A");
        let mut ops: Vec<String> = clone
            .clauses
            .iter()
            .map(|c| format!("{}.{}/{}", c.effect, c.op, c.params.len()))
            .collect();
        ops.sort();
        assert_eq!(ops, ["A.ask/0", "W.now/0", "W.roll/1"]);
        assert!(hs.iter().all(|h| !h.any));
    }

    #[test]
    fn io_is_never_handled() {
        let src = format!(
            "{REC}pub fn main() -> Int {{\n\
               handle {{ rec(fn() {{ io.println(\"x\")  roll(1) }}) }} with {{ W.roll(s) -> resume(s)  W.now() -> resume(1) }}\n\
             }}\n"
        );
        let m = expanded(&src).expect("expands");
        let mut hs = Vec::new();
        for f in &m.fns {
            handles(&f.body, &mut hs);
        }
        assert!(hs
            .iter()
            .all(|h| h.clauses.iter().all(|c| c.effect != BUILTIN_EFFECT)));
        assert!(hs
            .iter()
            .any(|h| h.clauses.len() == 2 && h.clauses.iter().all(|c| c.effect == "W")));
    }

    #[test]
    fn an_operation_value_used_as_a_value_is_refused() {
        for clause in [
            "op(o) -> { let p = o  resume(op.perform(p)) }",
            "op(o) -> { let f = fn(x: Int) { o }  io.println(op.show(f(1)))  resume(op.perform(o)) }",
        ] {
            let src = format!(
                "effect W {{ fn roll(s: Int) -> Int }}\n\
                 fn rec(body) {{ handle {{ body() }} with any {{ {clause} }} }}\n\
                 pub fn main() -> Int {{ handle {{ rec(fn() {{ roll(6) }}) }} with {{ W.roll(s) -> resume(s) }} }}\n"
            );
            match expanded(&src) {
                Err(CodegenError::Unsupported(m)) => assert_eq!(m, OP_VALUE_REFUSAL, "{clause}"),
                other => panic!("{clause}: {:?}", other.map(|_| ())),
            }
        }
    }
}
