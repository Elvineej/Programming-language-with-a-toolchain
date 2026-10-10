//! Native pattern compilation (spec
//! `docs/superpowers/specs/2026-10-10-elya-native-pattern-compilation-design.md`).
//!
//! A Core-to-Core pass, run right after `specialize`: every `match` whose arms
//! are not all FLAT -- an ADT scrutinee, constructor patterns whose arguments
//! are variables or wildcards, a final variable or wildcard -- is rewritten into
//! flat matches, `if`s and `let`s, the only shapes both emitters (`lower_match`
//! and the CPS `match_dispatch`) support. Nothing downstream changes.
//!
//! Semantics are the evaluator's: arms are tried top to bottom and the first
//! that matches wins. A non-flat arm becomes `if <test> { <bind; body> } else
//! { <the arms below> }` (D2): the test is a pure `Bool` expression over fresh
//! `$pat.N` names that answers `False` wherever it can fail, so the arms below
//! appear exactly once and the output grows linearly with the match. User
//! variables are bound only after the test has passed (D3), so a failed arm
//! never shadows a name a later arm reads. The scrutinee is bound once.
//!
//! Deliberately LLVM-free, like `specialize.rs`.

use std::rc::Rc;

use elya::ast::BinOp;
use elya::core::{
    CoreArm, CoreClause, CoreExpr, CoreFn, CoreHandle, CoreKind, CoreLit, CoreModule, CorePat,
    CorePerform, CoreReturn, CoreType,
};
use elya::span::Span;
use elya::types::{Ty, TyCon};

/// Compile every non-flat match in the module. A function with none is kept
/// as it is (not rebuilt).
pub(crate) fn run(core: &CoreModule) -> CoreModule {
    let mut c = Compiler {
        types: &core.types,
        counter: 0,
    };
    let fns = core
        .fns
        .iter()
        .map(|f| {
            if needs(&f.body) {
                CoreFn {
                    name: f.name.clone(),
                    params: f.params.clone(),
                    body: c.expr(&f.body),
                }
            } else {
                f.clone()
            }
        })
        .collect();
    CoreModule {
        fns,
        types: core.types.clone(),
    }
}

fn is_binder(p: &CorePat) -> bool {
    matches!(p, CorePat::Wild | CorePat::Var(_))
}

fn is_flat_ctor(p: &CorePat) -> bool {
    matches!(p, CorePat::Ctor(_, subs) if subs.iter().all(is_binder))
}

fn is_flat_match(scrut: &Ty, arms: &[CoreArm]) -> bool {
    matches!(scrut, Ty::Con(..))
        && arms
            .iter()
            .all(|a| is_binder(&a.pat) || is_flat_ctor(&a.pat))
}

/// Does `e` contain a match this pass rewrites?
fn needs(e: &CoreExpr) -> bool {
    let any = |cs: &[CoreExpr]| cs.iter().any(needs);
    match &e.kind {
        CoreKind::Lit(_) | CoreKind::Var(_) => false,
        CoreKind::App(f, a) => needs(f) || any(a),
        CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => any(a),
        CoreKind::Lambda(_, b) => needs(b),
        CoreKind::Let(_, v, b) => needs(v) || needs(b),
        CoreKind::If(c, t, f) => needs(c) || needs(t) || needs(f),
        CoreKind::Match(sc, arms) => {
            !is_flat_match(&sc.ty, arms) || needs(sc) || arms.iter().any(|a| needs(&a.body))
        }
        CoreKind::Handle(h) => {
            needs(&h.body)
                || h.clauses.iter().any(|c| needs(&c.body))
                || h.ret.as_ref().is_some_and(|r| needs(&r.body))
        }
        CoreKind::Resume(v) => needs(v),
        CoreKind::Perform(p) => any(&p.args),
    }
}

fn node(span: Span, ty: Ty, kind: CoreKind) -> CoreExpr {
    CoreExpr { span, ty, kind }
}

fn var(span: Span, name: &str, ty: &Ty) -> CoreExpr {
    node(span, ty.clone(), CoreKind::Var(name.to_string()))
}

fn let_(span: Span, name: &str, value: CoreExpr, body: CoreExpr) -> CoreExpr {
    let ty = body.ty.clone();
    node(
        span,
        ty,
        CoreKind::Let(name.to_string(), Rc::new(value), Rc::new(body)),
    )
}

type Arm = (CorePat, CoreExpr);

struct Compiler<'t> {
    types: &'t [CoreType],
    counter: usize,
}

impl Compiler<'_> {
    fn fresh(&mut self) -> String {
        self.counter += 1;
        format!("$pat.{}", self.counter)
    }

    /// The declared field types of constructor `name` of the (non-parametric)
    /// ADT `scrut`. A parametric type's fields mention its parameters: `None`,
    /// and the match is left alone (natively refused as before).
    fn field_types(&self, scrut: &Ty, name: &str) -> Option<Vec<Ty>> {
        self.types
            .iter()
            .filter(|t| matches!(scrut, Ty::Con(n, args) if *n == t.name && args.is_empty()))
            .flat_map(|t| t.ctors.iter())
            .find(|c| c.name == name)
            .map(|c| c.fields.clone())
    }

    /// Rebuild `e`, compiling every non-flat match bottom-up.
    fn expr(&mut self, e: &CoreExpr) -> CoreExpr {
        let kind = match &e.kind {
            CoreKind::Lit(_) | CoreKind::Var(_) => return e.clone(),
            CoreKind::App(f, a) => CoreKind::App(Rc::new(self.expr(f)), self.all(a)),
            CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), self.all(a)),
            CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), self.all(a)),
            CoreKind::Prim(o, a) => CoreKind::Prim(*o, self.all(a)),
            CoreKind::Lambda(ps, b) => CoreKind::Lambda(ps.clone(), Rc::new(self.expr(b))),
            CoreKind::Let(x, v, b) => {
                CoreKind::Let(x.clone(), Rc::new(self.expr(v)), Rc::new(self.expr(b)))
            }
            CoreKind::If(c, t, f) => CoreKind::If(
                Rc::new(self.expr(c)),
                Rc::new(self.expr(t)),
                Rc::new(self.expr(f)),
            ),
            CoreKind::Match(sc, arms) => {
                let sc2 = self.expr(sc);
                let arms2: Vec<CoreArm> = arms
                    .iter()
                    .map(|a| CoreArm {
                        pat: a.pat.clone(),
                        body: self.expr(&a.body),
                    })
                    .collect();
                if !is_flat_match(&sc2.ty, &arms2) {
                    if let Some(compiled) = self.compile(e.span, &e.ty, sc2.clone(), &arms2) {
                        return compiled;
                    }
                }
                CoreKind::Match(Rc::new(sc2), arms2.into())
            }
            CoreKind::Handle(h) => CoreKind::Handle(Rc::new(CoreHandle {
                body: Rc::new(self.expr(&h.body)),
                clauses: h
                    .clauses
                    .iter()
                    .map(|c| CoreClause {
                        effect: c.effect.clone(),
                        op: c.op.clone(),
                        params: c.params.clone(),
                        body: Rc::new(self.expr(&c.body)),
                    })
                    .collect(),
                ret: h.ret.as_ref().map(|r| {
                    Rc::new(CoreReturn {
                        binder: r.binder.clone(),
                        body: Rc::new(self.expr(&r.body)),
                    })
                }),
                is_multi_declared: h.is_multi_declared,
                multi: h.multi,
            })),
            CoreKind::Resume(v) => CoreKind::Resume(Rc::new(self.expr(v))),
            CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
                effect: p.effect.clone(),
                op: p.op.clone(),
                op_ty: p.op_ty.clone(),
                args: self.all(&p.args),
            })),
        };
        node(e.span, e.ty.clone(), kind)
    }

    fn all(&mut self, cs: &[CoreExpr]) -> Rc<[CoreExpr]> {
        cs.iter().map(|c| self.expr(c)).collect()
    }

    /// `match scrut { arms }` (bodies already compiled) as flat Core, or
    /// `None` when a field type is unknown (left as it was).
    fn compile(
        &mut self,
        span: Span,
        ty: &Ty,
        scrut: CoreExpr,
        arms: &[CoreArm],
    ) -> Option<CoreExpr> {
        let arms: Vec<Arm> = arms
            .iter()
            .map(|a| (a.pat.clone(), a.body.clone()))
            .collect();
        // The scrutinee is evaluated once: a variable is used as it is, any
        // other expression is bound to a fresh name first.
        if let CoreKind::Var(s) = &scrut.kind {
            return self.expand(span, ty, s, &scrut.ty, &arms);
        }
        let s = self.fresh();
        let body = self.expand(span, ty, &s, &scrut.ty, &arms)?;
        Some(let_(span, &s, scrut, body))
    }

    /// Try `arms` against the variable `s` (of type `s_ty`) in order; when none
    /// matches, the emitter's named match trap (an ADT) or, for a non-ADT value,
    /// nothing (D4). Every arm and the expansion below it appear exactly once.
    fn expand(
        &mut self,
        span: Span,
        ty: &Ty,
        s: &str,
        s_ty: &Ty,
        arms: &[Arm],
    ) -> Option<CoreExpr> {
        let Some((pat, body)) = arms.first() else {
            // An ADT match with no arms: its fall-through is `elya_match_fail`.
            // A non-ADT value has no trap block (and D4 never asks for one).
            return matches!(s_ty, Ty::Con(..)).then(|| {
                node(
                    span,
                    ty.clone(),
                    CoreKind::Match(Rc::new(var(span, s, s_ty)), Rc::from(Vec::new())),
                )
            });
        };
        match pat {
            CorePat::Wild | CorePat::Lit(CoreLit::Unit) => Some(body.clone()),
            CorePat::Var(x) => Some(let_(span, x, var(span, s, s_ty), body.clone())),
            CorePat::Ctor(..) if is_flat_ctor(pat) => {
                // A run of flat constructor arms is one flat match.
                let run = arms.iter().take_while(|(p, _)| is_flat_ctor(p)).count();
                let mut flat: Vec<CoreArm> = arms[..run]
                    .iter()
                    .map(|(p, b)| CoreArm {
                        pat: p.clone(),
                        body: b.clone(),
                    })
                    .collect();
                if run < arms.len() {
                    let rest = self.expand(span, ty, s, s_ty, &arms[run..])?;
                    flat.push(CoreArm {
                        pat: CorePat::Wild,
                        body: rest,
                    });
                }
                Some(node(
                    span,
                    ty.clone(),
                    CoreKind::Match(Rc::new(var(span, s, s_ty)), flat.into()),
                ))
            }
            // A literal, or a constructor with nested sub-patterns.
            _ => {
                // D4: the last arm with nothing below it, on a non-ADT value,
                // is taken unconditionally -- the checker proved the match
                // exhaustive (a `Bool` match covers both values; an `Int` one
                // needs a catch-all, so its last arm is never a literal).
                if arms.len() == 1 && !matches!(s_ty, Ty::Con(..)) {
                    return self.bind(span, s, s_ty, pat, body.clone());
                }
                // D2: `if <the arm matches> { <bind it; body> } else { rest }`.
                // The test is pure and returns `False` at every point where it
                // can fail, so `rest` appears once, whatever the arm's shape.
                let test = self.test(span, s, s_ty, pat)?;
                let success = self.bind(span, s, s_ty, pat, body.clone())?;
                let rest = self.expand(span, ty, s, s_ty, &arms[1..])?;
                Some(node(
                    span,
                    ty.clone(),
                    CoreKind::If(Rc::new(test), Rc::new(success), Rc::new(rest)),
                ))
            }
        }
    }

    /// A pure `Bool` expression: does `p` match the variable `v` (of type `t`)?
    /// It binds only fresh names and has no effects.
    fn test(&mut self, span: Span, v: &str, t: &Ty, p: &CorePat) -> Option<CoreExpr> {
        let boolean = Ty::Base(TyCon::Bool);
        Some(match p {
            CorePat::Wild | CorePat::Var(_) | CorePat::Lit(CoreLit::Unit) => {
                node(span, boolean, CoreKind::Lit(CoreLit::Bool(true)))
            }
            CorePat::Lit(l) => node(
                span,
                boolean,
                CoreKind::Prim(
                    BinOp::Eq,
                    Rc::from(vec![
                        var(span, v, t),
                        node(span, t.clone(), CoreKind::Lit(l.clone())),
                    ]),
                ),
            ),
            CorePat::Ctor(name, subs) => {
                let fields = self.field_types(t, name)?;
                if fields.len() != subs.len() {
                    return None;
                }
                // Inside the constructor's arm: every sub-test in turn, as
                // nested `if`s (each failure is the literal `False`).
                let mut flat_subs = Vec::with_capacity(subs.len());
                let mut inner: Vec<(String, Ty, CorePat)> = Vec::new();
                for (sp, ft) in subs.iter().zip(fields.iter()) {
                    if is_binder(sp) || matches!(sp, CorePat::Lit(CoreLit::Unit)) {
                        flat_subs.push(CorePat::Wild);
                    } else {
                        let f = self.fresh();
                        flat_subs.push(CorePat::Var(f.clone()));
                        inner.push((f, ft.clone(), sp.clone()));
                    }
                }
                let mut all = node(span, boolean.clone(), CoreKind::Lit(CoreLit::Bool(true)));
                for (f, ft, sp) in inner.iter().rev() {
                    let here = self.test(span, f, ft, sp)?;
                    all = node(
                        span,
                        boolean.clone(),
                        CoreKind::If(
                            Rc::new(here),
                            Rc::new(all),
                            Rc::new(node(
                                span,
                                boolean.clone(),
                                CoreKind::Lit(CoreLit::Bool(false)),
                            )),
                        ),
                    );
                }
                node(
                    span,
                    boolean.clone(),
                    CoreKind::Match(
                        Rc::new(var(span, v, t)),
                        Rc::from(vec![
                            CoreArm {
                                pat: CorePat::Ctor(name.clone(), flat_subs.into()),
                                body: all,
                            },
                            CoreArm {
                                pat: CorePat::Wild,
                                body: node(span, boolean, CoreKind::Lit(CoreLit::Bool(false))),
                            },
                        ]),
                    ),
                )
            }
        })
    }

    /// `body` with `p`'s user variables bound from `v`. Runs only once the
    /// test has passed (D3), so every match here has the one arm that matches;
    /// its fall-through (the named trap) is unreachable.
    fn bind(
        &mut self,
        span: Span,
        v: &str,
        t: &Ty,
        p: &CorePat,
        body: CoreExpr,
    ) -> Option<CoreExpr> {
        Some(match p {
            CorePat::Wild | CorePat::Lit(_) => body,
            CorePat::Var(x) => let_(span, x, var(span, v, t), body),
            CorePat::Ctor(name, subs) => {
                if !has_binder(p) {
                    return Some(body);
                }
                let fields = self.field_types(t, name)?;
                if fields.len() != subs.len() {
                    return None;
                }
                let mut flat_subs = Vec::with_capacity(subs.len());
                let mut inner: Vec<(String, Ty, CorePat)> = Vec::new();
                for (sp, ft) in subs.iter().zip(fields.iter()) {
                    match sp {
                        CorePat::Var(x) => flat_subs.push(CorePat::Var(x.clone())),
                        CorePat::Ctor(..) if has_binder(sp) => {
                            let f = self.fresh();
                            flat_subs.push(CorePat::Var(f.clone()));
                            inner.push((f, ft.clone(), sp.clone()));
                        }
                        _ => flat_subs.push(CorePat::Wild),
                    }
                }
                let mut out = body;
                for (f, ft, sp) in inner.iter().rev() {
                    out = self.bind(span, f, ft, sp, out)?;
                }
                let ty = out.ty.clone();
                node(
                    span,
                    ty,
                    CoreKind::Match(
                        Rc::new(var(span, v, t)),
                        Rc::from(vec![CoreArm {
                            pat: CorePat::Ctor(name.clone(), flat_subs.into()),
                            body: out,
                        }]),
                    ),
                )
            }
        })
    }
}

/// Does `p` bind a user variable anywhere?
fn has_binder(p: &CorePat) -> bool {
    match p {
        CorePat::Var(_) => true,
        CorePat::Wild | CorePat::Lit(_) => false,
        CorePat::Ctor(_, subs) => subs.iter().any(has_binder),
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

    fn size(e: &CoreExpr) -> usize {
        let all = |cs: &[CoreExpr]| cs.iter().map(size).sum::<usize>();
        1 + match &e.kind {
            CoreKind::Lit(_) | CoreKind::Var(_) => 0,
            CoreKind::App(f, a) => size(f) + all(a),
            CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => all(a),
            CoreKind::Lambda(_, b) => size(b),
            CoreKind::Let(_, v, b) => size(v) + size(b),
            CoreKind::If(c, t, f) => size(c) + size(t) + size(f),
            CoreKind::Match(sc, arms) => {
                size(sc) + arms.iter().map(|a| size(&a.body)).sum::<usize>()
            }
            CoreKind::Handle(h) => {
                size(&h.body)
                    + h.clauses.iter().map(|c| size(&c.body)).sum::<usize>()
                    + h.ret.as_ref().map_or(0, |r| size(&r.body))
            }
            CoreKind::Resume(v) => size(v),
            CoreKind::Perform(p) => all(&p.args),
        }
    }

    /// `n` arms, each with three nested constructor tests and three literal
    /// tests that can fail (the review's shape, 2026-10-10).
    fn arms(n: usize) -> String {
        let mut s = String::from("type L { Nil, Cons(Int, L) }\nfn f(l) { match l {\n");
        for i in 0..n {
            s.push_str(&format!(
                "  Cons({i}, Cons({}, Cons({}, Nil))) -> {}\n",
                i + 1,
                i + 2,
                i + 1
            ));
        }
        s.push_str("  _ -> 0\n} }\npub fn main() -> Int { f(Nil) }\n");
        s
    }

    fn compiled_size(n: usize) -> usize {
        let out = run(&core_of(&arms(n)));
        out.fns.iter().map(|f| size(&f.body)).sum()
    }

    /// Spec D2 (as revised after the review): the compiled match grows
    /// linearly with its arms. Backtracking by copying the failure
    /// continuation grew it by a factor per arm (6 MB of code for 6 arms).
    #[test]
    fn a_compiled_match_grows_linearly_with_its_arms() {
        let (a, b) = (compiled_size(8), compiled_size(16));
        assert!(b <= 2 * a + 16, "size(8) = {a}, size(16) = {b}");
    }

    /// No node appears twice in the output (later phases key on addresses).
    #[test]
    fn no_node_is_shared_in_the_output() {
        fn walk(e: &CoreExpr, seen: &mut std::collections::HashSet<usize>) {
            assert!(seen.insert(e as *const CoreExpr as usize), "shared node");
            let all = |cs: &[CoreExpr], seen: &mut std::collections::HashSet<usize>| {
                for c in cs {
                    walk(c, seen)
                }
            };
            match &e.kind {
                CoreKind::Lit(_) | CoreKind::Var(_) => {}
                CoreKind::App(f, a) => {
                    walk(f, seen);
                    all(a, seen)
                }
                CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => {
                    all(a, seen)
                }
                CoreKind::Lambda(_, b) => walk(b, seen),
                CoreKind::Let(_, v, b) => {
                    walk(v, seen);
                    walk(b, seen)
                }
                CoreKind::If(c, t, f) => {
                    walk(c, seen);
                    walk(t, seen);
                    walk(f, seen)
                }
                CoreKind::Match(sc, arms) => {
                    walk(sc, seen);
                    for a in arms.iter() {
                        walk(&a.body, seen)
                    }
                }
                CoreKind::Handle(h) => {
                    walk(&h.body, seen);
                    for c in h.clauses.iter() {
                        walk(&c.body, seen)
                    }
                    if let Some(r) = &h.ret {
                        walk(&r.body, seen)
                    }
                }
                CoreKind::Resume(v) => walk(v, seen),
                CoreKind::Perform(p) => all(&p.args, seen),
            }
        }
        let out = run(&core_of(&arms(6)));
        let mut seen = std::collections::HashSet::new();
        for f in &out.fns {
            walk(&f.body, &mut seen);
        }
    }
}
