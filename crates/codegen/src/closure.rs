//! Slice 5b-6 §6 — closure conversion's ANALYSIS half: free variables and the
//! lambda-site table.
//!
//! Deliberately LLVM-free. Every function here is a pure fold over Core, so the
//! capture rules — which the collector's correctness rests on — are provable by
//! unit tests that never construct an LLVM `Context`. The emission half lives in
//! `lib.rs`, which reads the table this module builds.

use std::collections::BTreeMap;
use std::rc::Rc;

use elya::core::{CoreExpr, CoreKind, CoreModule, CoreParam, CorePat};
use elya::types::Ty;

/// The synthetic binder for a handler clause's continuation (5b-8 D12). A
/// `resume` makes it free; the clause that owns the continuation binds it. So a
/// lambda that resumes -- and may run after its `handle` has returned --
/// captures the continuation by the ordinary free-variable rule. `$` cannot
/// begin a source identifier, so no user name collides with it (the house
/// style of D7's `$k`, which is a different binder: the RESULT of a resume).
pub const CONT: &str = "$cont";

/// The continuation's type, as `fv_walk` records it at a `resume(v)` whose
/// result has type `result`: a function from the resumed value to the handle's
/// answer. Only its heap-ness is read today -- `is_heap_ty` must answer true,
/// because the continuation is a pointer to the frame chain (D12).
pub fn cont_ty(arg: &Ty, result: &Ty) -> Ty {
    Ty::Fn(
        vec![arg.clone()],
        elya::types::EffectRow::pure(),
        Box::new(result.clone()),
    )
}

/// One lambda in the module, in a fixed pre-order.
pub struct LambdaSite {
    /// Identity: the ADDRESS of the `CoreExpr` node this site was built from.
    ///
    /// Span is forbidden as a key (`core.rs:20`: provenance, never a type key),
    /// and a pre-order index would rest on an unchecked walk-order invariant
    /// shared between two separate walks. The address is checkable and exact.
    /// It is stable because `collect_lambdas` and the emitter both run under the
    /// SAME immutable `&CoreModule` borrow, so nothing can move or reallocate a
    /// node in between; and every child node is behind an `Rc`, so cloning the
    /// handles below preserves addresses exactly.
    pub key: usize,
    /// The lifted function's Elya-level name, `<enclosing_fn>.lambda.<n>`, later
    /// mangled to `elya_<enclosing_fn>.lambda.<n>`. `.` is legal in LLVM
    /// identifiers and in COFF symbols, and it is Elya's access operator, so no
    /// source identifier can collide with one of these.
    pub symbol: String,
    /// The SYNTHETIC constructor tag for this closure's descriptor row. Assigned
    /// in the same pre-order walk as `symbol`, so the symbol and its descriptor
    /// row cannot drift apart.
    pub tag: usize,
    /// Captures in NAME order (`BTreeMap` iteration order), which is therefore
    /// the slot order in the heap block and the bit order in the pointer mask.
    pub captures: Vec<(String, Ty)>,
    pub params: Rc<[CoreParam]>,
    pub body: Rc<CoreExpr>,
    /// The lifted function's return type — the body's own type, never a second
    /// source of truth (the same rule `CoreFn` states at `core.rs:87`).
    pub ret: Ty,
}

/// Free variables of `e`, each mapped to the type carried by its first free
/// occurrence.
///
/// Types come from occurrences because pattern binders carry none: `CorePat::Ctor`
/// holds only sub-patterns, so no type environment can be threaded through a pure
/// Core walk. All occurrences of a monomorphic local agree, so "first occurrence
/// wins" is not a choice between different answers.
pub fn free_vars(e: &CoreExpr) -> BTreeMap<String, Ty> {
    free_vars_under(e, &[])
}

/// `free_vars`, with an initial set of already-bound names — used to bind the
/// enclosing function's parameters when computing which names are module-level.
pub fn free_vars_under(e: &CoreExpr, bound: &[String]) -> BTreeMap<String, Ty> {
    let mut scope: Vec<String> = bound.to_vec();
    let mut out = BTreeMap::new();
    fv_walk(e, &mut scope, &mut out);
    out
}

/// The match is EXHAUSTIVE with no catch-all, on purpose. Under-capture is a
/// wrong answer that the collector turns into a use-after-free; over-capture is
/// benign retention. So a new `CoreKind` variant must fail the build here rather
/// than fall into a `_ => {}` and go silently uncaptured.
fn fv_walk(e: &CoreExpr, scope: &mut Vec<String>, out: &mut BTreeMap<String, Ty>) {
    match &e.kind {
        CoreKind::Lit(_) => {}
        CoreKind::Var(x) => {
            if !scope.iter().any(|b| b == x) {
                out.entry(x.clone()).or_insert_with(|| e.ty.clone());
            }
        }
        CoreKind::App(f, args) => {
            fv_walk(f, scope, out);
            for a in args.iter() {
                fv_walk(a, scope, out);
            }
        }
        CoreKind::Builtin(_, args) => {
            for a in args.iter() {
                fv_walk(a, scope, out);
            }
        }
        CoreKind::Ctor(_, fields) => {
            for f in fields.iter() {
                fv_walk(f, scope, out);
            }
        }
        CoreKind::Prim(_, args) => {
            for a in args.iter() {
                fv_walk(a, scope, out);
            }
        }
        CoreKind::Lambda(params, body) => {
            let depth = scope.len();
            for p in params.iter() {
                scope.push(p.name.clone());
            }
            fv_walk(body, scope, out);
            scope.truncate(depth);
        }
        CoreKind::Let(name, value, body) => {
            // The VALUE is outside the binding; the BODY is inside it.
            fv_walk(value, scope, out);
            let depth = scope.len();
            scope.push(name.clone());
            fv_walk(body, scope, out);
            scope.truncate(depth);
        }
        CoreKind::If(c, t, f) => {
            fv_walk(c, scope, out);
            fv_walk(t, scope, out);
            fv_walk(f, scope, out);
        }
        CoreKind::Match(scrutinee, arms) => {
            fv_walk(scrutinee, scope, out);
            for arm in arms.iter() {
                let depth = scope.len();
                pat_binders(&arm.pat, scope);
                fv_walk(&arm.body, scope, out);
                scope.truncate(depth);
            }
        }
        // The handled body is outside every binder; each clause's params bind in
        // that clause's body only; the return binder binds in the return body only.
        CoreKind::Handle(h) => {
            fv_walk(&h.body, scope, out);
            for c in h.clauses.iter() {
                let depth = scope.len();
                for p in c.params.iter() {
                    scope.push(p.name.clone());
                }
                // D12: the clause owns its continuation.
                scope.push(CONT.to_string());
                fv_walk(&c.body, scope, out);
                scope.truncate(depth);
            }
            if let Some(r) = &h.ret {
                let depth = scope.len();
                scope.push(r.binder.clone());
                fv_walk(&r.body, scope, out);
                scope.truncate(depth);
            }
        }
        // D12: a resume uses the enclosing clause's continuation, so the
        // continuation is free here exactly like a variable occurrence.
        CoreKind::Resume(v) => {
            if !scope.iter().any(|b| b == CONT) {
                out.entry(CONT.to_string())
                    .or_insert_with(|| cont_ty(&v.ty, &e.ty));
            }
            fv_walk(v, scope, out)
        }
        // The op name is not a variable (D11); only the arguments are walked.
        CoreKind::Perform(p) => {
            for a in p.args.iter() {
                fv_walk(a, scope, out);
            }
        }
    }
}

pub(crate) fn pat_binders(p: &CorePat, scope: &mut Vec<String>) {
    match p {
        CorePat::Wild | CorePat::Lit(_) => {}
        CorePat::Var(x) => scope.push(x.clone()),
        CorePat::Ctor(_, subs) => {
            for s in subs.iter() {
                pat_binders(s, scope);
            }
        }
    }
}

/// Every lambda in the module, in one fixed pre-order, with tags running
/// consecutively from `first_tag` (which the caller sets to the number of real
/// constructor descriptor rows, so closure tags continue the same numbering).
pub fn collect_lambdas(core: &CoreModule, first_tag: usize) -> Vec<LambdaSite> {
    let mut out = Vec::new();
    for f in &core.fns {
        // A name free in the WHOLE enclosing function body (with that function's
        // parameters bound) is module-level, so it is not a capture. A name bound
        // by an enclosing `let`, parameter, or pattern IS. This is structural: it
        // needs no symbol table, and it gets a local that shadows a top-level name
        // right, where a by-name filter would drop the capture and then silently
        // call the global instead.
        let param_names: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        let module_level = free_vars_under(&f.body, &param_names);
        let mut n = 0usize;
        collect_in(&f.body, &f.name, &mut n, &module_level, first_tag, &mut out);
    }
    out
}

fn collect_in(
    e: &CoreExpr,
    enclosing: &str,
    n: &mut usize,
    module_level: &BTreeMap<String, Ty>,
    first_tag: usize,
    out: &mut Vec<LambdaSite>,
) {
    match &e.kind {
        CoreKind::Lambda(params, body) => {
            let captures: Vec<(String, Ty)> = free_vars(e)
                .into_iter()
                .filter(|(name, _)| !module_level.contains_key(name))
                .collect();
            let symbol = format!("{enclosing}.lambda.{n}");
            *n += 1;
            out.push(LambdaSite {
                key: e as *const CoreExpr as usize,
                symbol,
                tag: first_tag + out.len(),
                captures,
                params: Rc::clone(params),
                body: Rc::clone(body),
                ret: body.ty.clone(),
            });
            collect_in(body, enclosing, n, module_level, first_tag, out);
        }
        CoreKind::Lit(_) | CoreKind::Var(_) => {}
        CoreKind::App(f, args) => {
            collect_in(f, enclosing, n, module_level, first_tag, out);
            for a in args.iter() {
                collect_in(a, enclosing, n, module_level, first_tag, out);
            }
        }
        CoreKind::Builtin(_, args) => {
            for a in args.iter() {
                collect_in(a, enclosing, n, module_level, first_tag, out);
            }
        }
        CoreKind::Ctor(_, fields) => {
            for f in fields.iter() {
                collect_in(f, enclosing, n, module_level, first_tag, out);
            }
        }
        CoreKind::Prim(_, args) => {
            for a in args.iter() {
                collect_in(a, enclosing, n, module_level, first_tag, out);
            }
        }
        CoreKind::Let(_, value, body) => {
            collect_in(value, enclosing, n, module_level, first_tag, out);
            collect_in(body, enclosing, n, module_level, first_tag, out);
        }
        CoreKind::If(c, t, f) => {
            collect_in(c, enclosing, n, module_level, first_tag, out);
            collect_in(t, enclosing, n, module_level, first_tag, out);
            collect_in(f, enclosing, n, module_level, first_tag, out);
        }
        CoreKind::Match(scrutinee, arms) => {
            collect_in(scrutinee, enclosing, n, module_level, first_tag, out);
            for arm in arms.iter() {
                collect_in(&arm.body, enclosing, n, module_level, first_tag, out);
            }
        }
        CoreKind::Handle(h) => {
            collect_in(&h.body, enclosing, n, module_level, first_tag, out);
            for c in h.clauses.iter() {
                collect_in(&c.body, enclosing, n, module_level, first_tag, out);
            }
            if let Some(r) = &h.ret {
                collect_in(&r.body, enclosing, n, module_level, first_tag, out);
            }
        }
        CoreKind::Resume(v) => collect_in(v, enclosing, n, module_level, first_tag, out),
        CoreKind::Perform(p) => {
            for a in p.args.iter() {
                collect_in(a, enclosing, n, module_level, first_tag, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use elya::core::{
        CoreArm, CoreClause, CoreFn, CoreHandle, CoreLit, CorePerform, CoreReturn, CoreType,
    };
    use elya::span::Span;
    use elya::types::{EffectRow, TyCon};

    fn e(ty: Ty, kind: CoreKind) -> CoreExpr {
        CoreExpr {
            span: Span::EMPTY,
            ty,
            kind,
        }
    }
    fn int() -> Ty {
        Ty::Base(TyCon::Int)
    }
    fn var(name: &str) -> CoreExpr {
        e(int(), CoreKind::Var(name.to_string()))
    }
    fn lit(n: i64) -> CoreExpr {
        e(int(), CoreKind::Lit(CoreLit::Int(n)))
    }
    fn param(name: &str) -> CoreParam {
        CoreParam {
            name: name.to_string(),
            ty: int(),
        }
    }

    /// `let x = a  x` — `a` is free (it is in the VALUE, outside the binding),
    /// `x` is not (it is in the BODY, inside it). Getting this asymmetry backwards
    /// is the classic capture bug: it silently under-captures `a`.
    #[test]
    fn let_binds_its_body_but_not_its_value() {
        let expr = e(
            int(),
            CoreKind::Let("x".to_string(), Rc::new(var("a")), Rc::new(var("x"))),
        );
        let fv = free_vars(&expr);
        assert!(fv.contains_key("a"), "the let VALUE is outside the binding");
        assert!(!fv.contains_key("x"), "the let BODY is inside the binding");
    }

    /// An inner lambda's parameter shadows an outer free name of the same spelling.
    #[test]
    fn inner_lambda_parameter_shadows() {
        let inner = e(
            Ty::Fn(vec![int()], EffectRow::pure(), Box::new(int())),
            CoreKind::Lambda(Rc::from([param("k")]), Rc::new(var("k"))),
        );
        assert!(
            free_vars(&inner).is_empty(),
            "`k` is bound by the lambda's own parameter"
        );
    }

    /// Match-arm binders are binders. `Cons(h, t) -> h` must not report `h` free.
    #[test]
    fn match_arm_binders_bind() {
        let arm = CoreArm {
            pat: CorePat::Ctor(
                "Cons".to_string(),
                Rc::from([CorePat::Var("h".to_string()), CorePat::Wild]),
            ),
            body: var("h"),
        };
        let expr = e(int(), CoreKind::Match(Rc::new(var("xs")), Rc::from([arm])));
        let fv = free_vars(&expr);
        assert!(fv.contains_key("xs"), "the scrutinee is outside the arm");
        assert!(!fv.contains_key("h"), "a pattern binder binds in its arm");
    }

    /// A free `Var` carries the type of its own occurrence — that is where a
    /// capture's type comes from, since pattern binders carry none.
    #[test]
    fn a_free_var_carries_its_own_type() {
        let fv = free_vars(&var("a"));
        assert_eq!(fv.get("a"), Some(&int()));
    }

    #[test]
    fn builtin_arguments_contribute_free_vars_and_nested_lambdas() {
        let nested = e(
            Ty::Fn(vec![int()], EffectRow::pure(), Box::new(int())),
            CoreKind::Lambda(Rc::from([param("x")]), Rc::new(var("captured"))),
        );
        let expr = e(
            int(),
            CoreKind::Builtin("io.println".to_string(), Rc::from([var("message"), nested])),
        );
        let fv = free_vars(&expr);
        assert_eq!(
            fv.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["captured", "message"],
            "the builtin name is not a variable, but every argument is walked"
        );
        let core = CoreModule {
            fns: vec![CoreFn {
                name: "main".to_string(),
                params: Rc::from([]),
                body: expr,
            }],
            types: Vec::new(),
        };
        assert_eq!(
            collect_lambdas(&core, 0).len(),
            1,
            "collect_in must descend into builtin arguments"
        );
    }

    /// `fn wrap(k) { fn(x) { x + k } }` — `k` is captured; a module-level name
    /// used inside the lambda is NOT, because it is free in the enclosing function
    /// too. Tags are assigned from `first_tag` in the collection order.
    #[test]
    fn collect_captures_locals_and_excludes_globals() {
        let lam = e(
            Ty::Fn(vec![int()], EffectRow::pure(), Box::new(int())),
            CoreKind::Lambda(
                Rc::from([param("x")]),
                Rc::new(e(
                    int(),
                    CoreKind::App(Rc::new(var("helper")), Rc::from([var("x"), var("k")])),
                )),
            ),
        );
        let core = CoreModule {
            fns: vec![CoreFn {
                name: "wrap".to_string(),
                params: Rc::from([param("k")]),
                body: lam,
            }],
            types: Vec::<CoreType>::new(),
        };
        let sites = collect_lambdas(&core, 7);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].symbol, "wrap.lambda.0");
        assert_eq!(sites[0].tag, 7, "tags start at `first_tag`");
        let names: Vec<&str> = sites[0].captures.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec!["k"],
            "`k` is a parameter of the enclosing fn (a capture); \
             `helper` is free in the enclosing fn too (a global)"
        );
    }

    /// The four fields the emitter reads. `key` identifies the site's Core node
    /// by ADDRESS — Task 3 matches sites back to nodes with it, so a wrong answer
    /// there mis-associates lambdas silently rather than failing. `params`, `body`
    /// and `ret` are the lifted function's ingredients, and `ret` is the BODY's
    /// own type rather than a second source of truth (`core.rs:87`'s rule).
    #[test]
    fn a_site_carries_the_lifted_functions_ingredients() {
        let lam = e(
            Ty::Fn(vec![int()], EffectRow::pure(), Box::new(int())),
            CoreKind::Lambda(Rc::from([param("x")]), Rc::new(var("x"))),
        );
        let core = CoreModule {
            fns: vec![CoreFn {
                name: "id".to_string(),
                params: Rc::from([]),
                body: lam,
            }],
            types: Vec::<CoreType>::new(),
        };
        let sites = collect_lambdas(&core, 0);
        assert_eq!(sites.len(), 1);
        assert_eq!(
            sites[0].key, &core.fns[0].body as *const CoreExpr as usize,
            "`key` is the ADDRESS of the lambda's own Core node"
        );
        let names: Vec<&str> = sites[0].params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["x"], "the parameters travel with the site");
        assert!(
            matches!(sites[0].body.kind, CoreKind::Var(_)),
            "so does the body"
        );
        assert_eq!(sites[0].ret, int(), "`ret` IS the body's type");
    }

    /// Two sibling lambdas: order is fixed pre-order, symbols number within the
    /// enclosing function, tags number consecutively from `first_tag`. Task 3
    /// hard-checks `tag == first_tag + i` when it appends descriptor rows, so
    /// this is load-bearing, not cosmetic.
    #[test]
    fn sites_are_ordered_and_tags_are_consecutive() {
        let mk = |p: &str| {
            e(
                Ty::Fn(vec![int()], EffectRow::pure(), Box::new(int())),
                CoreKind::Lambda(Rc::from([param(p)]), Rc::new(lit(0))),
            )
        };
        let body = e(
            int(),
            CoreKind::Let(
                "a".to_string(),
                Rc::new(mk("u")),
                Rc::new(e(
                    int(),
                    CoreKind::Let("b".to_string(), Rc::new(mk("v")), Rc::new(lit(1))),
                )),
            ),
        );
        let core = CoreModule {
            fns: vec![CoreFn {
                name: "two".to_string(),
                params: Rc::from([]),
                body,
            }],
            types: Vec::<CoreType>::new(),
        };
        let sites = collect_lambdas(&core, 0);
        let syms: Vec<&str> = sites.iter().map(|s| s.symbol.as_str()).collect();
        assert_eq!(syms, vec!["two.lambda.0", "two.lambda.1"]);
        for (i, s) in sites.iter().enumerate() {
            assert_eq!(s.tag, i, "tag {i} must be `first_tag + i`");
        }
    }

    /// `f(a)` with both names as `Var`s — a two-name use with no new binders.
    fn uses(f: &str, a: &str) -> CoreExpr {
        e(int(), CoreKind::App(Rc::new(var(f)), Rc::from([var(a)])))
    }

    fn handle(body: CoreExpr, clauses: Vec<CoreClause>, ret: Option<(&str, CoreExpr)>) -> CoreExpr {
        let ret = ret.map(|(binder, body)| {
            Rc::new(CoreReturn {
                binder: binder.to_string(),
                body: Rc::new(body),
            })
        });
        e(
            int(),
            CoreKind::Handle(Rc::new(CoreHandle {
                body: Rc::new(body),
                clauses: clauses.into(),
                ret,
                is_multi_declared: false,
            })),
        )
    }

    fn clause(params: &[&str], body: CoreExpr) -> CoreClause {
        CoreClause {
            effect: "E".to_string(),
            op: "op".to_string(),
            params: params.iter().map(|p| param(p)).collect::<Vec<_>>().into(),
            body: Rc::new(body),
        }
    }

    /// A handler's binders (5b-8 Task 5). Each clause's params bind in THAT
    /// clause's body only, the return binder binds in the return body only, and
    /// the handled body is outside both. Under-capture here is the
    /// use-after-free the match's doc comment warns of, so both directions are
    /// pinned: a binder that fails to bind, and one that leaks past its clause.
    #[test]
    fn handler_clause_params_and_the_return_binder_bind_only_their_own_body() {
        // Binding: `k` and `r` are bound; `a`, `b` and the body's `x` are free.
        let bound = handle(
            var("x"),
            vec![clause(
                &["k"],
                e(int(), CoreKind::Resume(Rc::new(uses("k", "a")))),
            )],
            Some(("r", uses("r", "b"))),
        );
        let fv = free_vars(&bound);
        assert_eq!(
            fv.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["a", "b", "x"],
            "a clause param and the return binder bind in their own bodies"
        );

        // Scoping: clause 1's `k` must not leak into clause 2 or the return
        // clause, both of which use `k` free.
        let scoped = handle(
            var("x"),
            vec![clause(&["k"], var("a")), clause(&[], var("k"))],
            Some(("r", var("k"))),
        );
        assert!(
            free_vars(&scoped).contains_key("k"),
            "a clause param leaked past its own clause"
        );
    }

    /// `collect_in` descends into every handler subtree — the handled body, each
    /// clause body (through a `resume`), and the return body — so a lambda in
    /// any of them gets a site and a descriptor row.
    #[test]
    fn collect_descends_into_every_handler_subtree() {
        let lam = |p: &str| {
            e(
                Ty::Fn(vec![int()], EffectRow::pure(), Box::new(int())),
                CoreKind::Lambda(Rc::from([param(p)]), Rc::new(lit(0))),
            )
        };
        let body = handle(
            lam("u"),
            vec![clause(&[], e(int(), CoreKind::Resume(Rc::new(lam("v")))))],
            Some(("r", lam("w"))),
        );
        let core = CoreModule {
            fns: vec![CoreFn {
                name: "h".to_string(),
                params: Rc::from([]),
                body,
            }],
            types: Vec::<CoreType>::new(),
        };
        let sites = collect_lambdas(&core, 0);
        let params: Vec<&str> = sites.iter().map(|s| s.params[0].name.as_str()).collect();
        assert_eq!(
            params,
            vec!["u", "v", "w"],
            "body, clause (under resume) and return lambdas, in pre-order"
        );
    }

    fn perform(op: &str, args: Vec<CoreExpr>) -> CoreExpr {
        e(
            int(),
            CoreKind::Perform(Rc::new(CorePerform {
                effect: "E".to_string(),
                op: op.to_string(),
                op_ty: int(),
                args: args.into(),
            })),
        )
    }

    /// D11: a perform's arguments are expressions and are walked, but its op name
    /// is not a variable. Treating it as one would make `ping` look free -- and,
    /// in a lambda, look like a capture of whatever the name happens to bind.
    #[test]
    fn a_perform_walks_its_arguments_but_its_op_name_is_not_a_variable() {
        let fv = free_vars(&perform("ping", vec![var("a")]));
        assert_eq!(
            fv.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["a"],
            "only the argument is free"
        );
    }

    #[test]
    fn collect_descends_into_perform_arguments() {
        let lam = e(
            Ty::Fn(vec![int()], EffectRow::pure(), Box::new(int())),
            CoreKind::Lambda(Rc::from([param("x")]), Rc::new(var("x"))),
        );
        let core = CoreModule {
            fns: vec![CoreFn {
                name: "p".to_string(),
                params: Rc::from([]),
                body: perform("ping", vec![lam]),
            }],
            types: Vec::<CoreType>::new(),
        };
        assert_eq!(
            collect_lambdas(&core, 0).len(),
            1,
            "a lambda passed to an op gets its site and descriptor row"
        );
    }
}
