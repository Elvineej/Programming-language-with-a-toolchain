//! Argument records past three parameters (spec
//! `docs/superpowers/specs/2026-10-16-elya-argument-records-design.md`).
//!
//! A Core-to-Core pass, run after pattern compilation and before the CPS
//! analyses: every function, lambda, function type, call, operation and clause
//! with more than [`MAX_SOURCE_PARAMS`] parameters keeps its first two and
//! carries the rest in ONE argument record -- an ordinary constructor of a
//! synthesized ground type, one per distinct list of spilled types. The record
//! is built by an ordinary `Ctor` at the call and taken apart by an ordinary
//! one-arm `match` at the callee's entry, so the emitters, the runtime and the
//! collector are unchanged; after the pass no kind exceeds its LLVM cap.
//!
//! LLVM-free; the unit tests never build a `Context`.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use elya::core::{
    CoreArm, CoreClause, CoreCtor, CoreExpr, CoreFn, CoreHandle, CoreKind, CoreModule, CoreParam,
    CorePat, CorePerform, CoreReturn, CoreType,
};
use elya::span::Span;
use elya::types::{EffectLabel, EffectRow, Ty};

/// The most parameters any function travels with as themselves: the smallest
/// cap of any kind (an effectful lambda and an operation: closure or frame,
/// three, and the continuation within `MAX_PARAMS`).
pub(crate) const MAX_SOURCE_PARAMS: usize = 3;

/// Parameters kept in place when a list is spilled (the record is the third).
const KEPT: usize = MAX_SOURCE_PARAMS - 1;

struct Spill {
    /// canonical spilled-type list -> record number.
    records: HashMap<String, usize>,
    /// record number -> its field types (already rewritten), in creation order.
    fields: Vec<Vec<Ty>>,
    counter: usize,
}

fn type_name(k: usize) -> String {
    format!("$Args.{k}")
}

fn ctor_name(k: usize) -> String {
    format!("$args.{k}")
}

/// A record key's rendering of a type: what its value looks like in a field --
/// a function type by its calling convention (and its parameters' and result's,
/// recursively), never by its row's labels or tail. A call's argument and the
/// callee's parameter may differ there (a closed row against an open one, the
/// review's finding) and must still name ONE record.
fn key(t: &Ty, out: &mut String) {
    match t {
        Ty::Fn(ps, _, r) => {
            out.push_str("fn(");
            for p in ps {
                key(p, out);
                out.push(',');
            }
            out.push_str(if crate::cps::needs_cps(t) {
                ")cps->"
            } else {
                ")->"
            });
            key(r, out);
        }
        Ty::Con(n, a) => {
            out.push_str(n);
            out.push('(');
            for x in a {
                key(x, out);
                out.push(',');
            }
            out.push(')');
        }
        Ty::Tuple(a) => {
            out.push('(');
            for x in a {
                key(x, out);
                out.push(',');
            }
            out.push(')');
        }
        other => out.push_str(&format!("{other:?}")),
    }
}

impl Spill {
    /// The record for these (rewritten) spilled types.
    fn record(&mut self, tys: &[Ty]) -> usize {
        let mut k_str = String::new();
        for t in tys {
            key(t, &mut k_str);
            k_str.push(';');
        }
        if let Some(k) = self.records.get(&k_str) {
            return *k;
        }
        let k = self.fields.len();
        self.records.insert(k_str, k);
        self.fields.push(tys.to_vec());
        k
    }

    /// A parameter list, rewritten and spilled when it is too long: the kept
    /// types and, past the cap, the record's type and number.
    fn params(&mut self, ps: &[Ty]) -> (Vec<Ty>, Option<usize>) {
        let ps: Vec<Ty> = ps.iter().map(|p| self.ty(p)).collect();
        if ps.len() <= MAX_SOURCE_PARAMS {
            return (ps, None);
        }
        let k = self.record(&ps[KEPT..]);
        let mut kept = ps[..KEPT].to_vec();
        kept.push(Ty::Con(type_name(k), Vec::new()));
        (kept, Some(k))
    }

    fn ty(&mut self, t: &Ty) -> Ty {
        match t {
            Ty::Fn(ps, row, r) => {
                let (ps, _) = self.params(ps);
                let row = EffectRow {
                    labels: row
                        .labels
                        .iter()
                        .map(|(l, lab)| {
                            (
                                l.clone(),
                                EffectLabel {
                                    args: lab.args.iter().map(|a| self.ty(a)).collect(),
                                    span: lab.span,
                                },
                            )
                        })
                        .collect(),
                    tail: row.tail.clone(),
                };
                Ty::Fn(ps, row, Box::new(self.ty(r)))
            }
            Ty::Con(n, a) => Ty::Con(n.clone(), a.iter().map(|x| self.ty(x)).collect()),
            Ty::Tuple(a) => Ty::Tuple(a.iter().map(|x| self.ty(x)).collect()),
            other => other.clone(),
        }
    }

    fn fresh(&mut self) -> String {
        self.counter += 1;
        format!("$rest.{}", self.counter)
    }

    /// Spilled parameters: the kept ones, the record parameter, and the body
    /// wrapped in the entry `match` that names the spilled ones again.
    fn bind_params(&mut self, ps: &[CoreParam], body: CoreExpr) -> (Rc<[CoreParam]>, CoreExpr) {
        let tys: Vec<Ty> = ps.iter().map(|p| p.ty.clone()).collect();
        let (kept, rec) = self.params(&tys);
        let Some(k) = rec else {
            let ps: Rc<[CoreParam]> = ps
                .iter()
                .zip(kept)
                .map(|(p, ty)| CoreParam {
                    name: p.name.clone(),
                    ty,
                })
                .collect();
            return (ps, body);
        };
        let rest = self.fresh();
        let rec_ty = Ty::Con(type_name(k), Vec::new());
        let mut out: Vec<CoreParam> = ps[..KEPT]
            .iter()
            .zip(kept.iter())
            .map(|(p, ty)| CoreParam {
                name: p.name.clone(),
                ty: ty.clone(),
            })
            .collect();
        out.push(CoreParam {
            name: rest.clone(),
            ty: rec_ty.clone(),
        });
        let pat = CorePat::Ctor(
            ctor_name(k),
            ps[KEPT..]
                .iter()
                .map(|p| CorePat::Var(p.name.clone()))
                .collect(),
        );
        let span = body.span;
        let ty = body.ty.clone();
        let entry = CoreExpr {
            span,
            ty,
            kind: CoreKind::Match(
                Rc::new(CoreExpr {
                    span,
                    ty: rec_ty,
                    kind: CoreKind::Var(rest),
                }),
                Rc::from([CoreArm { pat, body }]),
            ),
        };
        (out.into(), entry)
    }

    /// Arguments for a callee whose ORIGINAL parameter count is `n`: packed
    /// into the record past the cap (left to right, as written).
    /// The record is the one the CALLEE's (rewritten) parameter types name --
    /// the same `bind_params` unpacks -- never derived from the arguments'
    /// own types.
    fn pack(&mut self, params: &[Ty], args: Vec<CoreExpr>, span: Span) -> Rc<[CoreExpr]> {
        let n = params.len();
        if n <= MAX_SOURCE_PARAMS || args.len() != n {
            return args.into();
        }
        let (_, rec) = self.params(params);
        let Some(k) = rec else {
            return args.into();
        };
        let mut args = args;
        let spilled: Vec<CoreExpr> = args.split_off(KEPT);
        args.push(CoreExpr {
            span,
            ty: Ty::Con(type_name(k), Vec::new()),
            kind: CoreKind::Ctor(ctor_name(k), spilled.into()),
        });
        args.into()
    }

    fn expr(&mut self, e: &CoreExpr) -> CoreExpr {
        let kind = match &e.kind {
            CoreKind::Lit(l) => CoreKind::Lit(l.clone()),
            CoreKind::Var(x) => CoreKind::Var(x.clone()),
            CoreKind::App(f, a) => {
                let ps = match &f.ty {
                    Ty::Fn(ps, _, _) => ps.clone(),
                    _ => Vec::new(),
                };
                let f2 = Rc::new(self.expr(f));
                let args = a.iter().map(|x| self.expr(x)).collect();
                CoreKind::App(f2, self.pack(&ps, args, e.span))
            }
            CoreKind::Builtin(n, a) => {
                CoreKind::Builtin(n.clone(), a.iter().map(|x| self.expr(x)).collect())
            }
            CoreKind::Ctor(n, a) => {
                CoreKind::Ctor(n.clone(), a.iter().map(|x| self.expr(x)).collect())
            }
            CoreKind::Prim(o, a) => CoreKind::Prim(*o, a.iter().map(|x| self.expr(x)).collect()),
            CoreKind::Lambda(ps, b) => {
                let body = self.expr(b);
                let (ps, body) = self.bind_params(ps, body);
                CoreKind::Lambda(ps, Rc::new(body))
            }
            CoreKind::Let(x, v, b) => {
                CoreKind::Let(x.clone(), Rc::new(self.expr(v)), Rc::new(self.expr(b)))
            }
            CoreKind::If(c, t, f) => CoreKind::If(
                Rc::new(self.expr(c)),
                Rc::new(self.expr(t)),
                Rc::new(self.expr(f)),
            ),
            CoreKind::Match(s, arms) => CoreKind::Match(
                Rc::new(self.expr(s)),
                arms.iter()
                    .map(|a| CoreArm {
                        pat: a.pat.clone(),
                        body: self.expr(&a.body),
                    })
                    .collect(),
            ),
            CoreKind::Handle(h) => {
                let body = Rc::new(self.expr(&h.body));
                let clauses = h
                    .clauses
                    .iter()
                    .map(|c| {
                        let cb = self.expr(&c.body);
                        let (params, cb) = self.bind_params(&c.params, cb);
                        CoreClause {
                            effect: c.effect.clone(),
                            op: c.op.clone(),
                            params,
                            body: Rc::new(cb),
                        }
                    })
                    .collect();
                let ret = h.ret.as_ref().map(|r| {
                    Rc::new(CoreReturn {
                        binder: r.binder.clone(),
                        body: Rc::new(self.expr(&r.body)),
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
            CoreKind::Resume(v) => CoreKind::Resume(Rc::new(self.expr(v))),
            CoreKind::Perform(p) => {
                let ps = match &p.op_ty {
                    Ty::Fn(ps, _, _) => ps.clone(),
                    _ => Vec::new(),
                };
                let args = p.args.iter().map(|x| self.expr(x)).collect();
                CoreKind::Perform(Rc::new(CorePerform {
                    effect: p.effect.clone(),
                    op: p.op.clone(),
                    op_ty: self.ty(&p.op_ty),
                    args: self.pack(&ps, args, e.span),
                }))
            }
        };
        CoreExpr {
            span: e.span,
            ty: self.ty(&e.ty),
            kind,
        }
    }
}

/// Spills every parameter list longer than [`MAX_SOURCE_PARAMS`].
pub(crate) fn run(core: &CoreModule) -> CoreModule {
    let mut sp = Spill {
        records: HashMap::new(),
        fields: Vec::new(),
        counter: 0,
    };
    let fns: Vec<CoreFn> = core
        .fns
        .iter()
        .map(|f| {
            let body = sp.expr(&f.body);
            let (params, body) = sp.bind_params(&f.params, body);
            let row = match sp.ty(&Ty::Fn(
                Vec::new(),
                f.row.clone(),
                Box::new(Ty::Base(elya::types::TyCon::Unit)),
            )) {
                Ty::Fn(_, r, _) => r,
                _ => f.row.clone(),
            };
            CoreFn {
                name: f.name.clone(),
                params,
                body,
                row,
            }
        })
        .collect();
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
                    fields: c.fields.iter().map(|x| sp.ty(x)).collect(),
                })
                .collect(),
        })
        .collect();
    // Record types last (their numbers are their order); a record's fields may
    // have created further records, so the list is read only now.
    let ordered: BTreeMap<usize, Vec<Ty>> = sp.fields.iter().cloned().enumerate().collect();
    for (k, fields) in ordered {
        types.push(CoreType {
            name: type_name(k),
            ctors: vec![CoreCtor {
                name: ctor_name(k),
                fields,
            }],
        });
    }
    CoreModule {
        fns,
        types,
        generic_types: core.generic_types.clone(),
        effect_params_used: core.effect_params_used.clone(),
        effects: core.effects.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_parameter_list_keeps_two_and_one_record() {
        let mut sp = Spill {
            records: HashMap::new(),
            fields: Vec::new(),
            counter: 0,
        };
        let int = Ty::Base(elya::types::TyCon::Int);
        let five = Ty::Fn(
            vec![int.clone(); 5],
            EffectRow::pure(),
            Box::new(int.clone()),
        );
        let Ty::Fn(ps, _, _) = sp.ty(&five) else {
            panic!()
        };
        assert_eq!(ps.len(), MAX_SOURCE_PARAMS);
        assert_eq!(sp.fields, vec![vec![int.clone(); 3]]);
        // Three stay as they are.
        let three = Ty::Fn(vec![int.clone(); 3], EffectRow::pure(), Box::new(int));
        assert_eq!(sp.ty(&three), three);
    }
}
