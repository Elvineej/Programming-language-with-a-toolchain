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
//!    specializes it like any other (types and rows).
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
    Ty::Fn(ups.clone(), own, uret.clone())
}

impl Anf<'_> {
    fn fresh(&mut self, what: &str) -> String {
        self.counter += 1;
        format!("${what}.{}", self.counter)
    }

    /// The eta-expansion of top-level `f` at type `ty`, or `None` when `ty` is
    /// not a function type (left to the emitters' own refusal).
    fn wrap(&mut self, e: &CoreExpr, f: &str) -> Option<CoreExpr> {
        let Ty::Fn(ps, _, ret) = &e.ty else {
            return None;
        };
        let inner_ty = inner_type(self.top[f], &e.ty);
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
            kind: CoreKind::App(
                Rc::new(CoreExpr {
                    span: e.span,
                    ty: inner_ty,
                    kind: CoreKind::Var(f.to_string()),
                }),
                args,
            ),
        };
        Some(CoreExpr {
            span: e.span,
            ty: e.ty.clone(),
            kind: CoreKind::Lambda(params, Rc::new(call)),
        })
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
            CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), self.all(a, locals)),
            CoreKind::Prim(o, a) => CoreKind::Prim(*o, self.all(a, locals)),
            CoreKind::Lambda(ps, b) => {
                let depth = locals.len();
                locals.extend(ps.iter().map(|p| p.name.clone()));
                let b = self.expr(b, locals);
                locals.truncate(depth);
                CoreKind::Lambda(ps.clone(), Rc::new(b))
            }
            CoreKind::Let(x, v, b) => {
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

/// Names every computed callee and wraps every top-level function used as a
/// value.
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
    }
}
