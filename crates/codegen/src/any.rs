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
//! - **D2, revised 2026-10-23** (spec
//!   `2026-10-23-elya-operation-values-natively-design.md`): in a copy the
//!   operation is known, so `o` is a value of a synthesized one-constructor type
//!   per operation instance, holding its arguments. Functions over it are cloned
//!   per operation, and `op.*` and `==` on any value of such a type are rewritten
//!   over its fields, wherever they occur.
//! - **D7.** The expansion is one-shot; `is_multi_declared` is stamped from the
//!   handled operations' declarations.
//!
//! Deliberately LLVM-free: the unit tests below never construct a `Context`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use elya::ast::BinOp;
use elya::core::{
    CoreArm, CoreClause, CoreCtor, CoreEffect, CoreExpr, CoreFn, CoreHandle, CoreKind, CoreLit,
    CoreModule, CoreOp, CoreParam, CorePat, CorePerform, CoreReturn, CoreType,
};
use elya::span::Span;
use elya::types::{EffectLabel, EffectRow, RowTail, Ty, TyCon};

use crate::mono::{apply, map_expr, TySubst};
use crate::CodegenError;

type R<T> = Result<T, CodegenError>;

/// The builtin effect, never dispatched to a handler (the earlier spec's D2).
const BUILTIN_EFFECT: &str = "IO";

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
        op_fns: core
            .fns
            .iter()
            .flat_map(|f| fn_op_rigids(f).into_iter().map(|r| (f.name.clone(), r)))
            .collect(),
        clones: HashMap::new(),
        queue: Vec::new(),
        op_names: HashMap::new(),
        op_info: HashMap::new(),
        op_types: Vec::new(),
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
        let (src, rigid, answer, op_ty, name) = cx.queue[i].clone();
        i += 1;
        let f = fns
            .iter()
            .chain(done.iter())
            .find(|f| f.name == src)
            .cloned()
            .ok_or(CodegenError::Unsupported(
                "callee is not a top-level function",
            ))?;
        // The review's F3: an operation type made inside `f` (a `with any` in it,
        // already expanded) that mentions `rigid` is made again at `answer`.
        let renamed = cx.reinstance(&rigid, &op_ty, &answer)?;
        let ty = |t: &Ty| rename_tys(&subst_op(t, &rigid, &op_ty, &answer), &renamed);
        let row = match ty(&Ty::Fn(
            Vec::new(),
            f.row.clone(),
            Box::new(Ty::Base(TyCon::Unit)),
        )) {
            Ty::Fn(_, r, _) => r,
            _ => f.row.clone(),
        };
        let body = map_expr(&f.body, &ty);
        let body = map_vars_named(&body, &mut |x: &str| renamed.get(x).cloned());
        let body = cx.redirect(&body, &rigid, &op_ty, &answer)?;
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
    // D3 (2026-10-23): `op.*` and `==` on an operation value, in a clause copy or
    // in a clone, are rewritten over the value's fields.
    let mut out = Vec::with_capacity(fns.len());
    for f in &fns {
        out.push(CoreFn {
            name: f.name.clone(),
            params: f.params.clone(),
            body: cx.ops(&f.body)?,
            row: f.row.clone(),
        });
    }
    let fns = out;
    let mut types = core.types.clone();
    // An operation type at a rigid answer type belonged to a dropped function.
    types.extend(
        cx.op_types
            .iter()
            .filter(|t| {
                let mut r = BTreeSet::new();
                t.ctors[0].fields.iter().for_each(|f| ty_rigids(f, &mut r));
                if let Some(i) = cx.op_info.get(&t.name) {
                    ty_rigids(&i.answer, &mut r);
                    i.label.args.iter().for_each(|a| ty_rigids(a, &mut r));
                }
                r.is_empty()
            })
            .cloned(),
    );
    Ok(CoreModule {
        fns,
        types,
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

/// The rigid answer types whose OPERATION (`$Op(rigid, _)`) a function's
/// signature mentions: such a function is cloned per operation (spec D2); one
/// that sees only the answer type is cloned per answer type (the review's F2).
fn fn_op_rigids(f: &CoreFn) -> BTreeSet<String> {
    fn go(t: &Ty, out: &mut BTreeSet<String>) {
        match t {
            Ty::Con(n, a) => {
                if n == "$Op" {
                    if let Some(Ty::Con(r, none)) = a.first() {
                        if none.is_empty() {
                            out.insert(r.clone());
                        }
                    }
                }
                a.iter().for_each(|x| go(x, out));
            }
            Ty::Fn(ps, row, r) => {
                ps.iter().for_each(|p| go(p, out));
                for l in row.labels.values() {
                    l.args.iter().for_each(|a| go(a, out));
                }
                go(r, out);
            }
            Ty::Tuple(a) => a.iter().for_each(|x| go(x, out)),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    f.params.iter().for_each(|p| go(&p.ty, &mut out));
    go(&f.body.ty, &mut out);
    out
}

/// `t` with each operation type named in `map` renamed.
fn rename_tys(t: &Ty, map: &HashMap<String, String>) -> Ty {
    match t {
        Ty::Con(n, a) if a.is_empty() => match map.get(n) {
            Some(m) => Ty::Con(m.clone(), Vec::new()),
            None => t.clone(),
        },
        Ty::Con(n, a) => Ty::Con(n.clone(), a.iter().map(|x| rename_tys(x, map)).collect()),
        Ty::Fn(ps, row, r) => Ty::Fn(
            ps.iter().map(|p| rename_tys(p, map)).collect(),
            crate::mono::map_row(row, &|a| rename_tys(a, map)),
            Box::new(rename_tys(r, map)),
        ),
        Ty::Tuple(a) => Ty::Tuple(a.iter().map(|x| rename_tys(x, map)).collect()),
        other => other.clone(),
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
    /// `(function, rigid)` pairs whose signature takes the rigid's operation.
    op_fns: BTreeSet<(String, String)>,
    /// `(function, rigid, operation type)` -> its clone's name (R1; keyed by the
    /// operation since 2026-10-23, D2).
    clones: HashMap<(String, String, String), String>,
    /// Clones to build: `(source, rigid, answer, operation type, name)`.
    queue: Vec<(String, String, Ty, Ty, String)>,
    /// `(E.op, the label instance)` -> its operation type's name (D1).
    op_names: HashMap<(String, String), String>,
    /// An operation type's name -> the operation it holds.
    op_info: HashMap<String, OpInfo>,
    /// The synthesized operation types, in creation order.
    op_types: Vec<CoreType>,
}

/// What an operation type (D1) stands for.
#[derive(Clone)]
struct OpInfo {
    effect: String,
    /// The operation's index in its effect's declaration.
    index: usize,
    label: EffectLabel,
    answer: Ty,
    /// The operation's parameter types at the instance: the type's fields.
    fields: Vec<Ty>,
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

    /// R1: every reference to a function at `rigid` goes to its clone for the
    /// operation `op_ty` (answering `answer`), requested on first use.
    fn redirect(&mut self, e: &CoreExpr, rigid: &str, op_ty: &Ty, answer: &Ty) -> R<CoreExpr> {
        let mut err = None;
        let out = map_vars_named(e, &mut |x: &str| {
            let rs = self.rigid_fns.get(x)?;
            if !rs.contains(rigid) {
                return None;
            }
            // D2, narrowed by the review's F2: per operation only when the
            // function takes it.
            let key = if self.op_fns.contains(&(x.to_string(), rigid.to_string())) {
                (x.to_string(), rigid.to_string(), format!("{op_ty:?}"))
            } else {
                (x.to_string(), rigid.to_string(), format!("{answer:?}"))
            };
            if let Some(n) = self.clones.get(&key) {
                return Some(n.clone());
            }
            self.fresh += 1;
            let name = format!("{x}.any.{}", self.fresh);
            let mut left = rs.clone();
            left.remove(rigid);
            for r in &left {
                if self.op_fns.contains(&(x.to_string(), r.clone())) {
                    self.op_fns.insert((name.clone(), r.clone()));
                }
            }
            if !left.is_empty() {
                self.rigid_fns.insert(name.clone(), left);
            }
            self.clones.insert(key, name.clone());
            self.queue.push((
                x.to_string(),
                rigid.to_string(),
                answer.clone(),
                op_ty.clone(),
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
            for (index, decl) in eff.ops.iter().enumerate() {
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
                let answer = apply(&decl.ret, &s);
                let op_ty = self.op_type(
                    label,
                    lab,
                    index,
                    decl,
                    args.iter().map(|a| a.ty.clone()).collect(),
                    &answer,
                )?;
                let typed = map_expr(&clause.body, &|t| subst_op(t, &rigid, &op_ty, &answer));
                let typed = self.redirect(&typed, &rigid, &op_ty, &answer)?;
                // D1: the copy opens by making its operation a value.
                let Ty::Con(ctor, _) = &op_ty else {
                    unreachable!("an operation type is a constructor")
                };
                let sp = typed.span;
                let made = node(
                    sp,
                    op_ty.clone(),
                    CoreKind::Ctor(ctor.clone(), args.iter().map(|a| var(sp, a)).collect()),
                );
                let body = node(
                    sp,
                    typed.ty.clone(),
                    CoreKind::Let(binder.name.clone(), Rc::new(made), Rc::new(typed)),
                );
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

impl<'a> Cx<'a> {
    /// D1: the operation type of `E.op` at the label instance `lab`, made on
    /// first use.
    fn op_type(
        &mut self,
        label: &str,
        lab: &EffectLabel,
        index: usize,
        decl: &CoreOp,
        fields: Vec<Ty>,
        answer: &Ty,
    ) -> R<Ty> {
        let named = format!("{label}.{}", decl.name);
        let key = (named.clone(), format!("{lab:?}"));
        if let Some(n) = self.op_names.get(&key) {
            return Ok(Ty::Con(n.clone(), Vec::new()));
        }
        let name = format!("$Op.{named}.{}", self.op_types.len());
        self.op_types.push(CoreType {
            name: name.clone(),
            ctors: vec![CoreCtor {
                name: name.clone(),
                fields: fields.clone(),
            }],
        });
        self.op_info.insert(
            name.clone(),
            OpInfo {
                effect: label.to_string(),
                index,
                label: lab.clone(),
                answer: answer.clone(),
                fields,
            },
        );
        self.op_names.insert(key, name.clone());
        Ok(Ty::Con(name, Vec::new()))
    }

    /// F3: every operation type mentioning `rigid`, made again with `rigid` at
    /// `answer` (and its operation at `op_ty`): old name -> new name.
    fn reinstance(&mut self, rigid: &str, op_ty: &Ty, answer: &Ty) -> R<HashMap<String, String>> {
        let mut map: HashMap<String, String> = HashMap::new();
        let names: Vec<String> = self.op_types.iter().map(|t| t.name.clone()).collect();
        for n in names {
            let info = self.op_info[&n].clone();
            let mut r = BTreeSet::new();
            info.fields.iter().for_each(|f| ty_rigids(f, &mut r));
            ty_rigids(&info.answer, &mut r);
            info.label.args.iter().for_each(|a| ty_rigids(a, &mut r));
            if !r.contains(rigid) {
                continue;
            }
            let ty = |t: &Ty| rename_tys(&subst_op(t, rigid, op_ty, answer), &map);
            let label = EffectLabel {
                args: info.label.args.iter().map(ty).collect(),
                ..info.label.clone()
            };
            let eff = *self
                .effects
                .get(&info.effect)
                .ok_or(CodegenError::Unsupported(
                    "`with any` over an undeclared effect",
                ))?;
            let decl = &eff.ops[info.index];
            let effect = info.effect.clone();
            let made = self.op_type(
                &effect,
                &label,
                info.index,
                decl,
                info.fields.iter().map(ty).collect(),
                &ty(&info.answer),
            )?;
            if let Ty::Con(m, _) = made {
                map.insert(n, m);
            }
        }
        Ok(map)
    }

    /// The operation an operation-typed expression holds, if it is one.
    fn info_of(&self, e: &CoreExpr) -> Option<(String, OpInfo)> {
        let Ty::Con(n, a) = &e.ty else { return None };
        if !a.is_empty() {
            return None;
        }
        self.op_info.get(n).map(|i| (n.clone(), i.clone()))
    }

    /// `match v { C(f1, .., fn) -> k(OpAt over f1..fn) }`.
    fn open(
        &mut self,
        v: CoreExpr,
        name: &str,
        info: &OpInfo,
        ty: &Ty,
        k: &mut dyn FnMut(&mut Self, &OpAt) -> R<CoreExpr>,
    ) -> R<CoreExpr> {
        let sp = v.span;
        let eff = *self
            .effects
            .get(&info.effect)
            .ok_or(CodegenError::Unsupported(
                "`with any` over an undeclared effect",
            ))?;
        let decl = &eff.ops[info.index];
        let args: Vec<CoreParam> = info
            .fields
            .iter()
            .map(|t| {
                self.fresh += 1;
                CoreParam {
                    name: format!("any.field.{}", self.fresh),
                    ty: t.clone(),
                }
            })
            .collect();
        let at = OpAt {
            named: format!("{}.{}", info.effect, decl.name),
            effect: info.effect.clone(),
            decl,
            args: args.clone(),
            answer: info.answer.clone(),
            label: info.label.clone(),
        };
        let body = k(self, &at)?;
        let pat = CorePat::Ctor(
            name.to_string(),
            args.iter().map(|a| CorePat::Var(a.name.clone())).collect(),
        );
        Ok(node(
            sp,
            ty.clone(),
            CoreKind::Match(Rc::new(v), Rc::from([CoreArm { pat, body }])),
        ))
    }

    /// D3: `e` with every `op.*` and `==`/`!=` on an operation value rewritten
    /// over the value's fields.
    fn ops(&mut self, e: &CoreExpr) -> R<CoreExpr> {
        let sp = e.span;
        let kind = match &e.kind {
            CoreKind::Builtin(n, a) if n.starts_with("op.") && !a.is_empty() => {
                let Some((name, info)) = self.info_of(&a[0]) else {
                    return Err(CodegenError::Unsupported(
                        "op builtin on a value that is not an operation",
                    ));
                };
                let subject = self.ops(&a[0])?;
                let rest = self.all_ops(&a[1..])?;
                return self.open(subject, &name, &info, &e.ty, &mut |_, at| {
                    builtin(sp, &e.ty, n, &rest, at)
                });
            }
            CoreKind::Prim(op @ (BinOp::Eq | BinOp::Ne), a)
                if a.len() == 2 && self.info_of(&a[0]).is_some() =>
            {
                let (name, info) = self.info_of(&a[0]).expect("checked");
                let l = self.ops(&a[0])?;
                let r = self.ops(&a[1])?;
                let op = *op;
                let ty = e.ty.clone();
                // Both operands are opened, in order; they hold one operation
                // with one set of arguments (spec D3), so the left one's are
                // walked.
                return self.open(l, &name, &info, &ty, &mut |cx, at| {
                    let cmp = op_eq(sp, op, at);
                    let info2 = info.clone();
                    let r2 = r.clone();
                    let ty2 = ty.clone();
                    cx.open(r2, &name, &info2, &ty2, &mut |_, _| Ok(cmp.clone()))
                });
            }
            CoreKind::Lit(_) | CoreKind::Var(_) => e.kind.clone(),
            CoreKind::App(f, a) => CoreKind::App(Rc::new(self.ops(f)?), self.all_ops(a)?),
            CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), self.all_ops(a)?),
            CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), self.all_ops(a)?),
            CoreKind::Prim(p, a) => CoreKind::Prim(*p, self.all_ops(a)?),
            CoreKind::Lambda(ps, b) => CoreKind::Lambda(ps.clone(), Rc::new(self.ops(b)?)),
            CoreKind::Let(x, v, b) => {
                CoreKind::Let(x.clone(), Rc::new(self.ops(v)?), Rc::new(self.ops(b)?))
            }
            CoreKind::If(c, t, f) => CoreKind::If(
                Rc::new(self.ops(c)?),
                Rc::new(self.ops(t)?),
                Rc::new(self.ops(f)?),
            ),
            CoreKind::Match(s, arms) => {
                let mut out = Vec::with_capacity(arms.len());
                for a in arms.iter() {
                    out.push(CoreArm {
                        pat: a.pat.clone(),
                        body: self.ops(&a.body)?,
                    });
                }
                CoreKind::Match(Rc::new(self.ops(s)?), out.into())
            }
            CoreKind::Handle(h) => {
                let mut clauses = Vec::with_capacity(h.clauses.len());
                for c in h.clauses.iter() {
                    clauses.push(CoreClause {
                        effect: c.effect.clone(),
                        op: c.op.clone(),
                        params: c.params.clone(),
                        body: Rc::new(self.ops(&c.body)?),
                    });
                }
                let ret = match &h.ret {
                    Some(r) => Some(Rc::new(CoreReturn {
                        binder: r.binder.clone(),
                        body: Rc::new(self.ops(&r.body)?),
                    })),
                    None => None,
                };
                CoreKind::Handle(Rc::new(CoreHandle {
                    body: Rc::new(self.ops(&h.body)?),
                    clauses: clauses.into(),
                    ret,
                    is_multi_declared: h.is_multi_declared,
                    multi: h.multi,
                    any: h.any,
                }))
            }
            CoreKind::Resume(v) => CoreKind::Resume(Rc::new(self.ops(v)?)),
            CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
                effect: p.effect.clone(),
                op: p.op.clone(),
                op_ty: p.op_ty.clone(),
                args: self.all_ops(&p.args)?,
            })),
        };
        Ok(node(sp, e.ty.clone(), kind))
    }

    fn all_ops(&mut self, cs: &[CoreExpr]) -> R<Rc<[CoreExpr]>> {
        let mut out = Vec::with_capacity(cs.len());
        for c in cs {
            out.push(self.ops(c)?);
        }
        Ok(out.into())
    }
}

/// `e` with each `Var(x)` and constructor renamed where `f(x)` says. Only
/// top-level clone names and operation types' constructors are renamed (they
/// contain dots, which no local name can), so binders need no tracking.
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
        CoreKind::Ctor(n, a) => CoreKind::Ctor(
            f(n).unwrap_or_else(|| n.clone()),
            a.iter().map(|x| go(x, f)).collect(),
        ),
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

/// `t` in a clause copy (D1): the clause's operation type `$Op(rigid, _)` is
/// `op_ty`, then `rigid` is `answer`.
fn subst_op(t: &Ty, rigid: &str, op_ty: &Ty, answer: &Ty) -> Ty {
    match t {
        Ty::Con(n, a) if n == "$Op" => match &a[..] {
            [Ty::Con(r, none), _] if r == rigid && none.is_empty() => op_ty.clone(),
            _ => Ty::Con(
                n.clone(),
                a.iter()
                    .map(|x| subst_op(x, rigid, op_ty, answer))
                    .collect(),
            ),
        },
        Ty::Con(n, a) if n == rigid && a.is_empty() => answer.clone(),
        Ty::Con(n, a) => Ty::Con(
            n.clone(),
            a.iter()
                .map(|x| subst_op(x, rigid, op_ty, answer))
                .collect(),
        ),
        Ty::Fn(ps, row, r) => Ty::Fn(
            ps.iter()
                .map(|p| subst_op(p, rigid, op_ty, answer))
                .collect(),
            crate::mono::map_row(row, &|a| subst_op(a, rigid, op_ty, answer)),
            Box::new(subst_op(r, rigid, op_ty, answer)),
        ),
        Ty::Tuple(a) => Ty::Tuple(
            a.iter()
                .map(|x| subst_op(x, rigid, op_ty, answer))
                .collect(),
        ),
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
    fn an_operation_value_used_as_a_value_expands() {
        // Refused by name ("operation value used other than by op.* or ==")
        // until 2026-10-23: `o` is now a value of an operation type per
        // operation (spec 2026-10-23-elya-operation-values-natively D1, D3).
        for clause in [
            "op(o) -> { let p = if True { o } else { o }  resume(op.perform(p)) }",
            "op(o) -> { let f = fn(x: Int) { o }  io.println(op.show(f(1)))  resume(op.perform(o)) }",
        ] {
            let src = format!(
                "effect W {{ fn roll(s: Int) -> Int  fn now() -> Int }}\n\
                 fn rec(body) {{ handle {{ body() }} with any {{ {clause} }} }}\n\
                 pub fn main() -> Int {{ handle {{ rec(fn() {{ roll(6) + now() }}) }} with {{ W.roll(s) -> resume(s)  W.now() -> resume(1) }} }}\n"
            );
            let m = expanded(&src).unwrap_or_else(|e| panic!("{clause}: {e:?}"));
            let mut ops: Vec<String> = m
                .types
                .iter()
                .filter(|t| t.name.starts_with("$Op."))
                .map(|t| format!("{}/{}", t.name, t.ctors[0].fields.len()))
                .collect();
            ops.sort();
            assert_eq!(ops, ["$Op.W.now.1/0", "$Op.W.roll.0/1"], "{clause}");
        }
    }
}
