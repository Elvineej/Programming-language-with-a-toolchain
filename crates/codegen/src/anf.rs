//! Computed callees and functions as values (spec
//! `docs/superpowers/specs/2026-10-16-elya-callees-and-function-values-design.md`).
//!
//! A Core-to-Core pass, run first in `build_module`, so every later phase sees
//! only the two call shapes the emitters compile -- a call of a top-level
//! function by name, and a call of a local closure:
//!
//! 1. **A computed callee is named.** `App(e, args)` whose callee is not a
//!    variable becomes `let $callee.k = e in $callee.k(args)`. The callee is
//!    evaluated before the arguments, as in the evaluator.
//! 2. **A top-level function used as a value is wrapped.** A `Var(f)` naming a
//!    top-level function (not shadowed by a local) outside callee position
//!    becomes `fn(a1, .., an) { f(a1, .., an) }` at the variable's type: a
//!    closure with no captures. Its inner call is an ordinary reference, so N7
//!    specializes it like any other (types and rows). When `f`'s own result
//!    differs from the use's (an upcast), it is
//!    `fn(a1, .., an) { let r = f(a1, .., an)  r }`, so `specialize` coerces `r`
//!    (spec 2026-10-23-elya-scope-captures-and-wrapper-results D2).
//! 3. **A constructor used as a value is wrapped** (spec
//!    2026-10-22-elya-one-shot-words-and-sweep-gaps D2). Core lowers a bare
//!    constructor name to a field-less `Ctor`; one with fields, typed as a
//!    function, becomes `fn(a1, .., an) { C(a1, .., an) }`. The construction
//!    inside is saturated, so `mono` instantiates a parametric one like any other.
//! 4. **An alias of a row-polymorphic local is substituted** (the same spec,
//!    D3). `let x = y`, `y` a local whose type has an open row, becomes its body
//!    with `x` renamed `y`. Each occurrence keeps its own type, so N7 sees `y`
//!    referenced at the instantiation each use needs. The `let` is kept when
//!    renaming would capture, when a top-level function shares `y`'s name, or
//!    when `y`'s row is closed (`Anf::substitutes` says why each).
//!
//! LLVM-free, like `specialize.rs`; the unit tests never build a `Context`.

use std::collections::HashMap;
use std::rc::Rc;

use elya::core::{
    CoreArm, CoreClause, CoreExpr, CoreFn, CoreHandle, CoreKind, CoreModule, CoreParam,
    CorePerform, CoreReturn,
};
use elya::types::{RowTail, Ty, TyCon};

struct Anf<'a> {
    top: &'a HashMap<String, &'a CoreFn>,
    counter: usize,
}

/// The type the wrapper's inner call of `f` has. When `f`'s own row is CLOSED
/// (it performs a fixed set of effects: pure, `{L}`), that row at this
/// instantiation -- not the use's row, which sub-effecting may have widened (a
/// pure `inc` passed where `fn(Int) / {E} -> Int` is expected: the wrapper is
/// typed `/ {E}`, the call of `inc` inside it is still pure; the review's
/// finding 1). An OPEN own row relays a parameter's row, and the reference's
/// own type already says what that relay is at this use (part 1 clones `f`
/// from it), so the use's type is kept.
///
/// The RESULT, too, is `f`'s own at this instantiation when it is ground
/// (spec 2026-10-23-elya-scope-captures-and-wrapper-results D2): the use's
/// result may be an upcast (`mk` returns `fn() -> Int`, used where
/// `fn() -> fn() / {E} -> Int` is expected), and a call typed with it jumped
/// into a direct closure with the effectful convention (5 for 15).
fn inner_type(f: &CoreFn, use_ty: &Ty) -> Ty {
    let Ty::Fn(ups, _, uret) = use_ty else {
        return use_ty.clone();
    };
    if f.row.tail != RowTail::Closed {
        return use_ty.clone();
    }
    let sig = Ty::Fn(
        f.params.iter().map(|p| p.ty.clone()).collect(),
        f.row.clone(),
        Box::new(f.body.ty.clone()),
    );
    // Type variables of the own row's label arguments: matched over the whole
    // type (the use's widened row contains the own labels).
    let mut tys = crate::mono::TySubst::new();
    crate::mono::match_tys(&sig, use_ty, &mut tys);
    let own = match crate::mono::apply(
        &Ty::Fn(Vec::new(), f.row.clone(), Box::new(Ty::Base(TyCon::Unit))),
        &tys,
    ) {
        Ty::Fn(_, r, _) => r,
        _ => f.row.clone(),
    };
    let own_ret = crate::mono::apply(&f.body.ty, &tys);
    // Spec 2026-10-23-elya-wrapper-parameter-rows D1: a row variable a
    // parameter mentions is instantiated at the use's PARAMETER at the same
    // position (sub-effecting widens results and data, never a parameter), so
    // it is bound from there first, through every position of the parameter
    // types -- the instantiation `specialize` clones `f` for.
    let mut own_row_vars = Vec::new();
    row_vars(
        &Ty::Fn(Vec::new(), f.row.clone(), Box::new(Ty::Base(TyCon::Unit))),
        &mut own_row_vars,
    );
    let mut binds = crate::specialize::RowSubst::new();
    if f.params.len() == ups.len() {
        for (p, u) in f.params.iter().zip(ups.iter()) {
            bind_tails(
                &crate::mono::apply(&p.ty, &tys),
                u,
                &own_row_vars,
                true,
                &mut binds,
            );
        }
    }
    // D1 (spec 2026-10-23-elya-row-tails-in-data): a row variable of the own
    // result that no parameter and not the own row mentions is instantiated by
    // this use alone, so it takes the use's row at the same position (a
    // closed one; `specialize` clones `f` for it). A CLOSED row of the own
    // result stays: that layer is a real upcast, which the coercion reaches.
    // A parameter's variable the pass above left unbound stays unbound.
    // The result's own parameter positions are walked too, before its results
    // (the parameter-rows spec D2, the review's finding): a free variable
    // there is this use's alone; bound from the use's parameter, that
    // parameter agrees with the use exactly (the adapter cannot coerce a
    // parameter), where binding it from a widened result first would not.
    let mut fixed = own_row_vars;
    for p in f.params.iter() {
        row_vars(&p.ty, &mut fixed);
    }
    bind_tails(&own_ret, uret, &fixed, true, &mut binds);
    let own_ret = crate::specialize::subst_ty(&own_ret, &binds);
    let ret = if is_ground(&own_ret) {
        own_ret
    } else {
        (**uret).clone()
    };
    Ty::Fn(ups.clone(), own, Box::new(ret))
}

/// Every open row variable in `t`.
fn row_vars(t: &Ty, out: &mut Vec<elya::types::RowVar>) {
    match t {
        Ty::Fn(ps, row, r) => {
            if let RowTail::Open(v) = row.tail {
                out.push(v);
            }
            for l in row.labels.values() {
                l.args.iter().for_each(|a| row_vars(a, out));
            }
            ps.iter().for_each(|p| row_vars(p, out));
            row_vars(r, out);
        }
        Ty::Con(_, a) | Ty::Tuple(a) => a.iter().for_each(|x| row_vars(x, out)),
        _ => {}
    }
}

/// D1: walks `own` and the use's `used` together through results and data
/// arguments, and through parameters too when `params` (a parameter's type,
/// matched against the use's parameter); an open tail of `own` not in `fixed`
/// and not bound yet, where `used` has a CLOSED row, is bound to the use's
/// labels beyond its own. An open row in the use (inside row-polymorphic code)
/// binds nothing, so the result stays unground and the use's type is kept.
fn bind_tails(
    own: &Ty,
    used: &Ty,
    fixed: &[elya::types::RowVar],
    params: bool,
    out: &mut crate::specialize::RowSubst,
) {
    match (own, used) {
        (Ty::Fn(op, orow, or), Ty::Fn(up, urow, ur)) if op.len() == up.len() => {
            if let RowTail::Open(v) = orow.tail {
                if !fixed.contains(&v) && urow.tail == RowTail::Closed && !out.contains_key(&v) {
                    let extra = urow
                        .labels
                        .iter()
                        .filter(|(l, _)| !orow.labels.contains_key(*l))
                        .map(|(l, lab)| (l.clone(), lab.clone()))
                        .collect();
                    out.insert(v, extra);
                }
            }
            if params {
                for (o, u) in op.iter().zip(up.iter()) {
                    bind_tails(o, u, fixed, params, out);
                }
            }
            bind_tails(or, ur, fixed, params, out);
        }
        (Ty::Con(on, oa), Ty::Con(un, ua)) if on == un && oa.len() == ua.len() => {
            for (o, u) in oa.iter().zip(ua.iter()) {
                bind_tails(o, u, fixed, params, out);
            }
        }
        (Ty::Tuple(oa), Ty::Tuple(ua)) if oa.len() == ua.len() => {
            for (o, u) in oa.iter().zip(ua.iter()) {
                bind_tails(o, u, fixed, params, out);
            }
        }
        _ => {}
    }
}

/// No type variable and no open row anywhere in `t`.
fn is_ground(t: &Ty) -> bool {
    match t {
        Ty::Var(_) | Ty::Error => false,
        Ty::Base(_) => true,
        Ty::Fn(ps, row, r) => {
            row.tail == RowTail::Closed
                && row.labels.values().all(|l| l.args.iter().all(is_ground))
                && ps.iter().all(is_ground)
                && is_ground(r)
        }
        Ty::Con(_, a) | Ty::Tuple(a) => a.iter().all(is_ground),
    }
}

impl Anf<'_> {
    fn fresh(&mut self, what: &str) -> String {
        self.counter += 1;
        format!("${what}.{}", self.counter)
    }

    /// The eta-expansion of top-level `f` at type `ty`, or `None` when `ty` is
    /// not a function type (left to the emitters' own refusal).
    fn wrap(&mut self, e: &CoreExpr, f: &str) -> Option<CoreExpr> {
        if !matches!(e.ty, Ty::Fn(..)) {
            return None;
        }
        let inner = inner_type(self.top[f], &e.ty);
        let (Ty::Fn(_, _, own_ret), Ty::Fn(_, _, use_ret)) = (&inner, &e.ty) else {
            return None;
        };
        let (own_ret, use_ret) = ((**own_ret).clone(), (**use_ret).clone());
        let callee = CoreExpr {
            span: e.span,
            ty: inner.clone(),
            kind: CoreKind::Var(f.to_string()),
        };
        // Equal types, not equal conventions: inside a row-polymorphic
        // function the use's result can still have an open row here (counted
        // as direct), and `specialize` closes it later (the review's u7: 5 for
        // 15). A `let` that turns out not to be needed coerces nothing.
        if own_ret == use_ret {
            return self.eta(e, |args| CoreKind::App(Rc::new(callee), args));
        }
        // D2: the call at `f`'s own result, bound, and the binder used at the
        // use's result: a value use `specialize` coerces (its upcast adapter).
        let r = self.fresh("eta.result");
        let span = e.span;
        self.eta(e, |args| {
            CoreKind::Let(
                r.clone(),
                Rc::new(CoreExpr {
                    span,
                    ty: own_ret,
                    kind: CoreKind::App(Rc::new(callee), args),
                }),
                Rc::new(CoreExpr {
                    span,
                    ty: use_ret,
                    kind: CoreKind::Var(r),
                }),
            )
        })
    }

    /// Rule 3: constructor `c` used as a value at `e`'s function type.
    fn wrap_ctor(&mut self, e: &CoreExpr, c: &str) -> Option<CoreExpr> {
        self.eta(e, |args| CoreKind::Ctor(c.to_string(), args))
    }

    /// `fn(k.0, .., k.n) { body(k.0, .., k.n) }` at `e`'s function type; the
    /// body is typed as the function's result.
    fn eta(
        &mut self,
        e: &CoreExpr,
        body: impl FnOnce(Rc<[CoreExpr]>) -> CoreKind,
    ) -> Option<CoreExpr> {
        let Ty::Fn(ps, _, ret) = &e.ty else {
            return None;
        };
        let k = self.fresh("eta");
        let params: Rc<[CoreParam]> = ps
            .iter()
            .enumerate()
            .map(|(i, t)| CoreParam {
                name: format!("{k}.{i}"),
                ty: t.clone(),
            })
            .collect();
        let args: Rc<[CoreExpr]> = params
            .iter()
            .map(|p| CoreExpr {
                span: e.span,
                ty: p.ty.clone(),
                kind: CoreKind::Var(p.name.clone()),
            })
            .collect();
        let call = CoreExpr {
            span: e.span,
            ty: (**ret).clone(),
            kind: body(args),
        };
        Some(CoreExpr {
            span: e.span,
            ty: e.ty.clone(),
            kind: CoreKind::Lambda(params, Rc::new(call)),
        })
    }

    /// Rule 4's conditions for `let x = y in b` (the review of 2026-10-23 found
    /// each one by a wrong value):
    /// - `y` is a LOCAL. An alias of a top-level function stays a local, so
    ///   rule 2 wraps the function once, at the binding, and the upcast adapter
    ///   still sees a local binder at every later upcast (5 for 15 otherwise);
    /// - `y`'s type at the binding has an open row (or is an operation): the
    ///   row-polymorphic alias the rule is for. A closed row may be an
    ///   annotated upcast, which `specialize` coerces at the binding but not at
    ///   a callee;
    ///
    /// Since 2026-10-24 (spec 2026-10-24-elya-alias-renaming) a top-level
    /// function sharing `y`'s name no longer blocks it (D2: captures are decided
    /// by scope since 2026-10-23), and a body that binds `y` again has those
    /// binders renamed first (D1: capture-avoiding substitution).
    fn substitutes(&self, x: &str, v: &CoreExpr, y: &str, locals: &[String]) -> bool {
        x != y && locals.iter().any(|l| l == y) && (has_open_row(&v.ty) || is_operation(&v.ty))
    }

    /// D1: `e` with every binder of `name` in it (a `let`, a lambda's or a
    /// clause's parameter, a pattern's variable, a `return` binder) renamed to a
    /// fresh name no source can write, together with the uses it scopes.
    fn fresh_binders(&mut self, e: &CoreExpr, name: &str) -> CoreExpr {
        let kind = match &e.kind {
            CoreKind::Lit(_) | CoreKind::Var(_) => return e.clone(),
            CoreKind::App(f, a) => CoreKind::App(
                Rc::new(self.fresh_binders(f, name)),
                self.fresh_all(a, name),
            ),
            CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), self.fresh_all(a, name)),
            CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), self.fresh_all(a, name)),
            CoreKind::Prim(o, a) => CoreKind::Prim(*o, self.fresh_all(a, name)),
            CoreKind::Lambda(ps, b) => {
                let mut b = (**b).clone();
                let ps: Rc<[CoreParam]> = ps
                    .iter()
                    .map(|p| {
                        if p.name == name {
                            let n = self.fresh(&format!("{name}$alias"));
                            b = rename(&b, name, &n);
                            CoreParam {
                                name: n,
                                ty: p.ty.clone(),
                            }
                        } else {
                            p.clone()
                        }
                    })
                    .collect();
                CoreKind::Lambda(ps, Rc::new(self.fresh_binders(&b, name)))
            }
            CoreKind::Let(x, v, b) => {
                let v = self.fresh_binders(v, name);
                if x == name {
                    let n = self.fresh(&format!("{name}$alias"));
                    let b = rename(b, name, &n);
                    CoreKind::Let(n, Rc::new(v), Rc::new(self.fresh_binders(&b, name)))
                } else {
                    CoreKind::Let(x.clone(), Rc::new(v), Rc::new(self.fresh_binders(b, name)))
                }
            }
            CoreKind::If(c, t, f) => CoreKind::If(
                Rc::new(self.fresh_binders(c, name)),
                Rc::new(self.fresh_binders(t, name)),
                Rc::new(self.fresh_binders(f, name)),
            ),
            CoreKind::Match(sc, arms) => {
                let sc = self.fresh_binders(sc, name);
                let mut out = Vec::with_capacity(arms.len());
                for a in arms.iter() {
                    let mut names = Vec::new();
                    crate::closure::pat_binders(&a.pat, &mut names);
                    let (pat, body) = if names.iter().any(|n| n == name) {
                        let n = self.fresh(&format!("{name}$alias"));
                        (rename_pat(&a.pat, name, &n), rename(&a.body, name, &n))
                    } else {
                        (a.pat.clone(), a.body.clone())
                    };
                    out.push(CoreArm {
                        pat,
                        body: self.fresh_binders(&body, name),
                    });
                }
                CoreKind::Match(Rc::new(sc), out.into())
            }
            CoreKind::Handle(h) => {
                let body = Rc::new(self.fresh_binders(&h.body, name));
                let mut clauses = Vec::with_capacity(h.clauses.len());
                for c in h.clauses.iter() {
                    let mut cb = (*c.body).clone();
                    let params: Rc<[CoreParam]> = c
                        .params
                        .iter()
                        .map(|p| {
                            if p.name == name {
                                let n = self.fresh(&format!("{name}$alias"));
                                cb = rename(&cb, name, &n);
                                CoreParam {
                                    name: n,
                                    ty: p.ty.clone(),
                                }
                            } else {
                                p.clone()
                            }
                        })
                        .collect();
                    clauses.push(CoreClause {
                        effect: c.effect.clone(),
                        op: c.op.clone(),
                        params,
                        body: Rc::new(self.fresh_binders(&cb, name)),
                    });
                }
                let ret = h.ret.as_ref().map(|r| {
                    let (binder, rb) = if r.binder == name {
                        let n = self.fresh(&format!("{name}$alias"));
                        let rb = rename(&r.body, name, &n);
                        (n, rb)
                    } else {
                        (r.binder.clone(), (*r.body).clone())
                    };
                    Rc::new(CoreReturn {
                        binder,
                        body: Rc::new(self.fresh_binders(&rb, name)),
                    })
                });
                CoreKind::Handle(Rc::new(CoreHandle {
                    body,
                    clauses: clauses.into(),
                    ret,
                    is_multi_declared: h.is_multi_declared,
                    multi: h.multi,
                    any: h.any,
                }))
            }
            CoreKind::Resume(v) => CoreKind::Resume(Rc::new(self.fresh_binders(v, name))),
            CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
                effect: p.effect.clone(),
                op: p.op.clone(),
                op_ty: p.op_ty.clone(),
                args: self.fresh_all(&p.args, name),
            })),
        };
        CoreExpr {
            span: e.span,
            ty: e.ty.clone(),
            kind,
        }
    }

    fn fresh_all(&mut self, cs: &[CoreExpr], name: &str) -> Rc<[CoreExpr]> {
        cs.iter().map(|c| self.fresh_binders(c, name)).collect()
    }

    fn expr(&mut self, e: &CoreExpr, locals: &mut Vec<String>) -> CoreExpr {
        let kind = match &e.kind {
            CoreKind::Lit(_) => return e.clone(),
            CoreKind::Var(x) => {
                if self.top.contains_key(x) && !locals.iter().any(|l| l == x) {
                    if let Some(w) = self.wrap(e, x) {
                        return w;
                    }
                }
                return e.clone();
            }
            CoreKind::App(f, a) => {
                let args = self.all(a, locals);
                match &f.kind {
                    // A callee by name stays a call by name (a local closure or
                    // a top-level function); it is not a value use.
                    CoreKind::Var(_) => CoreKind::App(f.clone(), args),
                    _ => {
                        let callee = self.expr(f, locals);
                        let k = self.fresh("callee");
                        let call = CoreExpr {
                            span: e.span,
                            ty: e.ty.clone(),
                            kind: CoreKind::App(
                                Rc::new(CoreExpr {
                                    span: f.span,
                                    ty: f.ty.clone(),
                                    kind: CoreKind::Var(k.clone()),
                                }),
                                args,
                            ),
                        };
                        CoreKind::Let(k, Rc::new(callee), Rc::new(call))
                    }
                }
            }
            CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), self.all(a, locals)),
            CoreKind::Ctor(n, a) => {
                if a.is_empty() {
                    if let Some(w) = self.wrap_ctor(e, n) {
                        return w;
                    }
                }
                CoreKind::Ctor(n.clone(), self.all(a, locals))
            }
            CoreKind::Prim(o, a) => CoreKind::Prim(*o, self.all(a, locals)),
            CoreKind::Lambda(ps, b) => {
                let depth = locals.len();
                locals.extend(ps.iter().map(|p| p.name.clone()));
                let b = self.expr(b, locals);
                locals.truncate(depth);
                CoreKind::Lambda(ps.clone(), Rc::new(b))
            }
            CoreKind::Let(x, v, b) => {
                if let CoreKind::Var(y) = &v.kind {
                    if self.substitutes(x, v, y, locals) {
                        // D1: the body's own binders of `y` are renamed first,
                        // so substituting `y` for `x` captures nothing.
                        let b = if binds(b, y) {
                            self.fresh_binders(b, y)
                        } else {
                            (**b).clone()
                        };
                        return self.expr(&rename(&b, x, y), locals);
                    }
                }
                let v = self.expr(v, locals);
                locals.push(x.clone());
                let b = self.expr(b, locals);
                locals.pop();
                CoreKind::Let(x.clone(), Rc::new(v), Rc::new(b))
            }
            CoreKind::If(c, t, f) => CoreKind::If(
                Rc::new(self.expr(c, locals)),
                Rc::new(self.expr(t, locals)),
                Rc::new(self.expr(f, locals)),
            ),
            CoreKind::Match(s, arms) => {
                let s = Rc::new(self.expr(s, locals));
                let arms = arms
                    .iter()
                    .map(|a| {
                        let mut names = Vec::new();
                        crate::closure::pat_binders(&a.pat, &mut names);
                        let depth = locals.len();
                        locals.extend(names);
                        let body = self.expr(&a.body, locals);
                        locals.truncate(depth);
                        CoreArm {
                            pat: a.pat.clone(),
                            body,
                        }
                    })
                    .collect();
                CoreKind::Match(s, arms)
            }
            CoreKind::Handle(h) => {
                let body = Rc::new(self.expr(&h.body, locals));
                let clauses = h
                    .clauses
                    .iter()
                    .map(|c| {
                        let depth = locals.len();
                        locals.extend(c.params.iter().map(|p| p.name.clone()));
                        locals.push(crate::closure::CONT.to_string());
                        let body = Rc::new(self.expr(&c.body, locals));
                        locals.truncate(depth);
                        CoreClause {
                            effect: c.effect.clone(),
                            op: c.op.clone(),
                            params: c.params.clone(),
                            body,
                        }
                    })
                    .collect();
                let ret = h.ret.as_ref().map(|r| {
                    locals.push(r.binder.clone());
                    let body = Rc::new(self.expr(&r.body, locals));
                    locals.pop();
                    Rc::new(CoreReturn {
                        binder: r.binder.clone(),
                        body,
                    })
                });
                CoreKind::Handle(Rc::new(CoreHandle {
                    body,
                    clauses,
                    ret,
                    is_multi_declared: h.is_multi_declared,
                    multi: h.multi,
                    any: h.any,
                }))
            }
            CoreKind::Resume(v) => CoreKind::Resume(Rc::new(self.expr(v, locals))),
            CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
                effect: p.effect.clone(),
                op: p.op.clone(),
                op_ty: p.op_ty.clone(),
                args: self.all(&p.args, locals),
            })),
        };
        CoreExpr {
            span: e.span,
            ty: e.ty.clone(),
            kind,
        }
    }

    fn all(&mut self, cs: &[CoreExpr], locals: &mut Vec<String>) -> Rc<[CoreExpr]> {
        cs.iter().map(|c| self.expr(c, locals)).collect()
    }
}

/// Whether a type mentions an open effect row anywhere.
fn has_open_row(t: &Ty) -> bool {
    match t {
        Ty::Fn(ps, row, r) => {
            row.tail != RowTail::Closed
                || row.labels.values().any(|l| l.args.iter().any(has_open_row))
                || ps.iter().any(has_open_row)
                || has_open_row(r)
        }
        Ty::Con(_, a) | Ty::Tuple(a) => a.iter().any(has_open_row),
        _ => false,
    }
}

/// `$Op(t, fn() / H -> t)`, a `with any` clause's operation.
fn is_operation(t: &Ty) -> bool {
    matches!(t, Ty::Con(n, _) if n == "$Op")
}

/// Whether `e` binds `name` anywhere: a `let`, a parameter, a pattern, a
/// clause's parameter or a return binder (rule 4's capture check).
fn binds(e: &CoreExpr, name: &str) -> bool {
    let any = |cs: &[CoreExpr]| cs.iter().any(|c| binds(c, name));
    match &e.kind {
        CoreKind::Lit(_) | CoreKind::Var(_) => false,
        CoreKind::App(f, a) => binds(f, name) || any(a),
        CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => any(a),
        CoreKind::Lambda(ps, b) => ps.iter().any(|p| p.name == name) || binds(b, name),
        CoreKind::Let(x, v, b) => x == name || binds(v, name) || binds(b, name),
        CoreKind::If(c, t, f) => binds(c, name) || binds(t, name) || binds(f, name),
        CoreKind::Match(s, arms) => {
            binds(s, name)
                || arms.iter().any(|a| {
                    let mut names = Vec::new();
                    crate::closure::pat_binders(&a.pat, &mut names);
                    names.iter().any(|n| n == name) || binds(&a.body, name)
                })
        }
        CoreKind::Handle(h) => {
            binds(&h.body, name)
                || h.clauses
                    .iter()
                    .any(|c| c.params.iter().any(|p| p.name == name) || binds(&c.body, name))
                || h.ret
                    .as_ref()
                    .is_some_and(|r| r.binder == name || binds(&r.body, name))
        }
        CoreKind::Resume(v) => binds(v, name),
        CoreKind::Perform(p) => any(&p.args),
    }
}

/// `e` with every free `from` renamed `to`, each occurrence keeping its type.
/// A binder of `from` ends the renaming beneath it; the caller has checked
/// that nothing in `e` binds `to` (`binds`).
/// A pattern with its variable `from` renamed `to`.
fn rename_pat(p: &elya::core::CorePat, from: &str, to: &str) -> elya::core::CorePat {
    use elya::core::CorePat;
    match p {
        CorePat::Var(v) if v == from => CorePat::Var(to.to_string()),
        CorePat::Ctor(c, subs) => CorePat::Ctor(
            c.clone(),
            subs.iter().map(|s| rename_pat(s, from, to)).collect(),
        ),
        other => other.clone(),
    }
}

fn rename(e: &CoreExpr, from: &str, to: &str) -> CoreExpr {
    let go = |c: &CoreExpr| rename(c, from, to);
    let all = |cs: &[CoreExpr]| cs.iter().map(go).collect::<Rc<[CoreExpr]>>();
    let kind = match &e.kind {
        CoreKind::Lit(_) => return e.clone(),
        CoreKind::Var(v) if v == from => CoreKind::Var(to.to_string()),
        CoreKind::Var(_) => return e.clone(),
        CoreKind::App(f, a) => CoreKind::App(Rc::new(go(f)), all(a)),
        CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), all(a)),
        CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), all(a)),
        CoreKind::Prim(o, a) => CoreKind::Prim(*o, all(a)),
        CoreKind::Lambda(ps, b) => {
            if ps.iter().any(|p| p.name == from) {
                return e.clone();
            }
            CoreKind::Lambda(ps.clone(), Rc::new(go(b)))
        }
        CoreKind::Let(x, v, b) => {
            let b = if x == from { b.clone() } else { Rc::new(go(b)) };
            CoreKind::Let(x.clone(), Rc::new(go(v)), b)
        }
        CoreKind::If(c, t, f) => CoreKind::If(Rc::new(go(c)), Rc::new(go(t)), Rc::new(go(f))),
        CoreKind::Match(s, arms) => CoreKind::Match(
            Rc::new(go(s)),
            arms.iter()
                .map(|a| {
                    let mut names = Vec::new();
                    crate::closure::pat_binders(&a.pat, &mut names);
                    CoreArm {
                        pat: a.pat.clone(),
                        body: if names.iter().any(|n| n == from) {
                            a.body.clone()
                        } else {
                            go(&a.body)
                        },
                    }
                })
                .collect(),
        ),
        CoreKind::Handle(h) => CoreKind::Handle(Rc::new(CoreHandle {
            body: Rc::new(go(&h.body)),
            clauses: h
                .clauses
                .iter()
                .map(|c| CoreClause {
                    effect: c.effect.clone(),
                    op: c.op.clone(),
                    params: c.params.clone(),
                    body: if c.params.iter().any(|p| p.name == from) {
                        c.body.clone()
                    } else {
                        Rc::new(go(&c.body))
                    },
                })
                .collect(),
            ret: h.ret.as_ref().map(|r| {
                Rc::new(CoreReturn {
                    binder: r.binder.clone(),
                    body: if r.binder == from {
                        r.body.clone()
                    } else {
                        Rc::new(go(&r.body))
                    },
                })
            }),
            is_multi_declared: h.is_multi_declared,
            multi: h.multi,
            any: h.any,
        })),
        CoreKind::Resume(v) => CoreKind::Resume(Rc::new(go(v))),
        CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
            effect: p.effect.clone(),
            op: p.op.clone(),
            op_ty: p.op_ty.clone(),
            args: all(&p.args),
        })),
    };
    CoreExpr {
        span: e.span,
        ty: e.ty.clone(),
        kind,
    }
}

/// Names every computed callee, wraps every top-level function and constructor
/// used as a value, and substitutes every alias of a variable.
pub(crate) fn run(core: &CoreModule) -> CoreModule {
    let top: HashMap<String, &CoreFn> = core.fns.iter().map(|f| (f.name.clone(), f)).collect();
    let mut a = Anf {
        top: &top,
        counter: 0,
    };
    let fns = core
        .fns
        .iter()
        .map(|f| {
            let mut locals: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
            CoreFn {
                name: f.name.clone(),
                params: f.params.clone(),
                body: a.expr(&f.body, &mut locals),
                row: f.row.clone(),
            }
        })
        .collect();
    CoreModule {
        fns,
        types: core.types.clone(),
        generic_types: core.generic_types.clone(),
        effect_params_used: core.effect_params_used.clone(),
        effects: core.effects.clone(),
    }
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

    fn walk<'a>(e: &'a CoreExpr, out: &mut Vec<&'a CoreExpr>) {
        out.push(e);
        match &e.kind {
            CoreKind::Lit(_) | CoreKind::Var(_) => {}
            CoreKind::App(f, a) => {
                walk(f, out);
                a.iter().for_each(|c| walk(c, out));
            }
            CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => {
                a.iter().for_each(|c| walk(c, out))
            }
            CoreKind::Lambda(_, b) | CoreKind::Resume(b) => walk(b, out),
            CoreKind::Let(_, v, b) => {
                walk(v, out);
                walk(b, out);
            }
            CoreKind::If(c, t, f) => {
                walk(c, out);
                walk(t, out);
                walk(f, out);
            }
            CoreKind::Match(s, arms) => {
                walk(s, out);
                arms.iter().for_each(|a| walk(&a.body, out));
            }
            CoreKind::Handle(h) => {
                walk(&h.body, out);
                h.clauses.iter().for_each(|c| walk(&c.body, out));
                if let Some(r) = &h.ret {
                    walk(&r.body, out);
                }
            }
            CoreKind::Perform(p) => p.args.iter().for_each(|c| walk(c, out)),
        }
    }

    fn main_nodes(core: &CoreModule) -> Vec<CoreExpr> {
        let main = core.fns.iter().find(|f| f.name == "main").expect("main");
        let mut out = Vec::new();
        walk(&main.body, &mut out);
        out.into_iter().cloned().collect()
    }

    /// Spec 2026-10-22 D2: a constructor with fields used as a value was a
    /// NULLARY construction typed as a function (a call of it jumped through
    /// its tag word: SIGSEGV). It is a closure over a saturated construction.
    #[test]
    fn a_constructor_used_as_a_value_is_wrapped() {
        let out = run(&core_of(
            "type W { Wrap(Int) }\n\
             fn ap(f, x) { f(x) }\n\
             pub fn main() -> Int { match ap(Wrap, 5) { Wrap(h) -> h } }\n",
        ));
        let nodes = main_nodes(&out);
        assert!(
            !nodes
                .iter()
                .any(|n| matches!(&n.kind, CoreKind::Ctor(c, a) if c == "Wrap" && a.is_empty())),
            "a field-less construction of `Wrap` is left"
        );
        let wrapped = nodes.iter().any(|n| match &n.kind {
            CoreKind::Lambda(ps, b) => {
                matches!(n.ty, Ty::Fn(..))
                    && ps.len() == 1
                    && matches!(&b.kind, CoreKind::Ctor(c, a)
                        if c == "Wrap" && a.len() == 1
                            && matches!(&a[0].kind, CoreKind::Var(v) if *v == ps[0].name))
            }
            _ => false,
        });
        assert!(wrapped, "no `fn(x) {{ Wrap(x) }}` in main");
    }

    /// Spec 2026-10-22 D3: `let x = y` is substituted away. When its body binds
    /// `y` again, that binder is renamed first (spec 2026-10-24-elya-alias-renaming
    /// D1; it used to keep the alias -- CHANGED EXPECTATION by that decision).
    #[test]
    fn an_alias_whose_target_is_rebound_is_kept() {
        let alias_let = |core: &CoreModule| {
            main_nodes(core).iter().any(
                |n| matches!(&n.kind, CoreKind::Let(x, v, _) if x == "ap2" && matches!(&v.kind, CoreKind::Var(y) if y == "ap")),
            )
        };
        let plain = run(&core_of(
            "pub fn main() -> Int { let ap = fn(g) { g() + 1 }  let ap2 = ap  ap2(fn() { 5 }) }\n",
        ));
        assert!(!alias_let(&plain), "the alias was not substituted");
        let rebound = run(&core_of(
            "pub fn main() -> Int {\n\
             \x20 let ap = fn(g) { g() + 1 }\n\
             \x20 let ap2 = ap\n\
             \x20 let r = { let ap = 5  ap2(fn() { ap }) }\n\
             \x20 r\n\
             }\n",
        ));
        assert!(
            !alias_let(&rebound),
            "the alias was kept under a rebinding of its target"
        );
        // ... and the inner `let ap = 5` is renamed, with its use.
        let renamed = main_nodes(&rebound).iter().any(|n| {
            matches!(&n.kind, CoreKind::Let(x, _, b)
                if x.starts_with("$ap$alias.")
                    && matches!(&b.kind, CoreKind::App(_, a)
                        if a.iter().any(|l| matches!(&l.kind, CoreKind::Lambda(_, body)
                            if matches!(&body.kind, CoreKind::Var(v) if v == x)))))
        });
        assert!(
            renamed,
            "the inner binder of `ap` was not renamed with its use"
        );
    }

    /// Spec 2026-10-23 D2: `mk` returns a DIRECT closure, used where
    /// `fn() -> fn() / {E} -> Int` is expected. The wrapper's inner call is typed
    /// at `mk`'s own result, and a `let` makes it a binder `specialize` coerces.
    #[test]
    fn a_wrappers_inner_call_has_the_functions_own_result() {
        let out = run(&core_of(
            "effect E { fn e() -> Int }\n\
             fn mk() -> fn() -> Int { fn() { 5 } }\n\
             fn ap2(f: fn() -> fn() / {E} -> Int) -> Int { let r = f()  r() + e() }\n\
             pub fn main() -> Int { handle { ap2(mk) } with { E.e() -> resume(10) } }\n",
        ));
        let pure_result = |t: &Ty| matches!(t, Ty::Fn(_, row, _) if row.labels.is_empty());
        let bound = main_nodes(&out).iter().any(|n| match &n.kind {
            CoreKind::Lambda(_, b) => match &b.kind {
                CoreKind::Let(x, v, body) => {
                    matches!(&v.kind, CoreKind::App(f, _) if matches!(&f.kind, CoreKind::Var(m) if m == "mk"))
                        && pure_result(&v.ty)
                        && matches!(&body.kind, CoreKind::Var(y) if y == x)
                        && !pure_result(&body.ty)
                }
                _ => false,
            },
            _ => false,
        });
        assert!(bound, "no wrapper binding `mk()` at its own result");
    }
}
