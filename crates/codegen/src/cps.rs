//! The selective-CPS partition (spec §8.1-8.2).
//!
//! Deliberately LLVM-free: this is a pure question about a `Ty`, so it needs no
//! `Context` and its tests are unit tests.

use std::collections::{BTreeSet, HashMap, HashSet};

use elya::core::{CoreExpr, CoreKind, CoreModule};
use elya::types::{EffectRow, RowTail, Ty};

use crate::closure::{cont_ty, free_vars_under, pat_binders, CONT};

/// The one effect label the back end treats as builtin. Sound as a *name* test
/// only because lowering refuses a user-declared `effect IO` (D1, Task 2) --
/// `CoreModule` carries no effect declarations, so the name is all codegen has.
/// Mirrors `src/types.rs`'s `OBSERVABLE_EFFECTS`.
const BUILTIN_EFFECT: &str = "IO";

/// True iff a call to a value of this type may need its continuation captured.
///
/// A non-function type is not a call site and answers `false`.
///
/// Called by `collect_sites` (7b-2) to recognise an effectful call.
pub fn needs_cps(ty: &Ty) -> bool {
    match ty {
        Ty::Fn(_, row, _) => row_needs_cps(row),
        _ => false,
    }
}

fn row_needs_cps(row: &EffectRow) -> bool {
    if row.labels.keys().any(|l| l != BUILTIN_EFFECT) {
        return true;
    }
    match row.tail {
        // D16: an open tail ALONE is direct. A row-polymorphic function is
        // compiled once, so the convention must not depend on what its row
        // variable is later instantiated with; an instantiation at a user
        // effect is refused by name instead (`ty_names_user_effect` +
        // `ty_has_open_row`, checked at every reference). 7a answered `true`
        // here "conservatively", but over-CPS is not safe across a convention
        // boundary: `apply(fn(x) { x * 10 })` would call a direct lambda with
        // the CPS convention (measured: it compiles and prints 11 at 831b023).
        RowTail::Closed | RowTail::Open(_) => false,
        // Poison: never survives a clean front end. Kept conservative.
        RowTail::ErrorRow => true,
    }
}

/// One value a suspended computation still needs (D10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Saved {
    /// A local binding, identified by BINDING, not by name: `binding` is its
    /// index in the scope stack at the site, so two live bindings that share a
    /// name (`let x = 1  (let x = 2  get() + x) + x`) get two slots.
    Var { name: String, binding: usize },
    /// An operand already evaluated before the site, keyed by its node's
    /// address (closure.rs's `LambdaSite::key` rule). Its VALUE is saved:
    /// recomputing it after resumption would repeat whatever it did.
    Temp(usize),
}

/// One ancestor step from a site's region root down to the site (7b-3 reads
/// it to emit the site's resumption function from the hole upward).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathStep {
    /// The ancestor node's address.
    pub key: usize,
    /// Which child of it the walk descended into.
    pub slot: usize,
    /// Scope depth AT the ancestor: the bindings its later children see.
    pub depth: usize,
}

/// One continuation site (D10): a call that may capture its continuation,
/// in a position where something still runs after it returns.
pub struct ContSite {
    /// The address of the site's `CoreExpr` (see `Saved::Temp`).
    pub key: usize,
    /// The top-level function whose body (or nested region) holds the site.
    pub owner: String,
    /// Region root (outermost) to the site's parent (innermost).
    pub path: Vec<PathStep>,
    /// Slot order in the frame after `[tag][code_ptr][next]`: temporaries in
    /// path order (outermost ancestor first, left to right), then bindings in
    /// binding order (outermost first). Deterministic, and the bit order of the
    /// frame row's mask.
    pub saved: Vec<(Saved, Ty)>,
}

/// Every continuation site in the module, in a fixed pre-order: the functions
/// in order, each walked through all of its regions -- the body, every lambda
/// body, every handled body, every clause body and every return body.
///
/// A site is a `Perform`, or an `App` whose callee's type `needs_cps`, that is
/// NOT in tail position of its region: a tail call passes the current
/// continuation straight on (D14), so it needs no frame. A direct handle and a
/// direct resume are nesting calls (D14), so they are not sites; a CPS handle
/// and a CPS resume are (5b-10, `Fx`). Each of a handle's bodies is a region
/// of its own.
pub fn collect_sites(core: &CoreModule) -> Vec<ContSite> {
    collect(core).sites
}

/// One `handle` node (7b-3): what its handler frame must carry.
pub struct HandlerSite {
    /// The `Handle` node's address.
    pub key: usize,
    /// The top-level function that contains it.
    pub owner: String,
    /// Scope depth at the handle: the binding indices its body continues from.
    pub depth: usize,
    /// Locals in scope at the handle that the handled body, any clause body
    /// (its params and the continuation bound) or the return body (its binder
    /// bound) uses -- in binding order. The body runs in its own function and
    /// the return clause in another, so these travel in the handler frame,
    /// after `[tag][code_ptr][next]`, exactly as a site's saved values do.
    pub saved: Vec<(Saved, Ty)>,
}

/// Every `handle` in the module, in the same pre-order as `collect_sites`.
pub fn collect_handlers(core: &CoreModule) -> Vec<HandlerSite> {
    collect(core).handlers
}

/// One lambda's region (slice 5b-9b): the scope at the lambda, BEFORE its
/// parameters. A lambda body continues its enclosing scope's binding indices,
/// so an effectful lifted body must place each capture at the index of the
/// innermost binding of that name here -- the index every site inside the
/// body saved it under -- and bind its parameters from `scope.len()` on.
pub struct LambdaRegion {
    pub scope: Vec<String>,
}

impl LambdaRegion {
    /// The binding index of `name` as seen from inside the lambda.
    pub fn binding_of(&self, name: &str) -> Option<usize> {
        self.scope.iter().rposition(|n| n == name)
    }
}

/// Every lambda's region, by the lambda node's address (`LambdaSite::key`).
pub fn collect_lambda_regions(core: &CoreModule) -> HashMap<usize, LambdaRegion> {
    collect(core).lambdas
}

struct Out {
    fx: Fx,
    sites: Vec<ContSite>,
    handlers: Vec<HandlerSite>,
    lambdas: HashMap<usize, LambdaRegion>,
}

fn collect(core: &CoreModule) -> Out {
    let mut out = Out {
        fx: effect_facts(core),
        sites: Vec::new(),
        handlers: Vec::new(),
        lambdas: HashMap::new(),
    };
    for f in &core.fns {
        let mut scope: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        let mut path = Vec::new();
        walk(&f.body, &f.name, &mut scope, &mut path, &mut out);
    }
    out
}

/// The locals a handler frame carries: free in the body, the clause bodies
/// (params and `$cont` bound) or the return body (binder bound), and bound in
/// `scope` -- keyed by binding, outermost first.
fn handler_saved(h: &elya::core::CoreHandle, scope: &[String]) -> Vec<(Saved, Ty)> {
    let mut vars: std::collections::BTreeMap<usize, (String, Ty)> =
        std::collections::BTreeMap::new();
    let mut need = |fv: std::collections::BTreeMap<String, Ty>| {
        for (n, t) in fv {
            if let Some(b) = scope.iter().rposition(|x| *x == n) {
                vars.entry(b).or_insert((n, t));
            }
        }
    };
    need(free_vars_under(&h.body, &[]));
    for c in h.clauses.iter() {
        let mut bound: Vec<String> = c.params.iter().map(|p| p.name.clone()).collect();
        bound.push(CONT.to_string());
        need(free_vars_under(&c.body, &bound));
    }
    if let Some(r) = &h.ret {
        need(free_vars_under(&r.body, std::slice::from_ref(&r.binder)));
    }
    vars.into_iter()
        .map(|(binding, (name, ty))| (Saved::Var { name, binding }, ty))
        .collect()
}

/// Slice 5b-10: which `handle`s LEAK a user effect -- their clauses, return
/// clause or body perform something the handle does not handle -- and which
/// `resume`s belong to such a handle, by node address. A leaking handle is a
/// CPS handle (a continuation site of its region, its clauses and return
/// clause CPS regions); every other handle is a direct nesting call, as in
/// 5b-8. Which handle a resume belongs to is lexical: the innermost enclosing
/// clause binds `$cont`.
#[derive(Default, Debug)]
pub struct Fx {
    pub leaking: HashSet<usize>,
    pub cps_resumes: HashSet<usize>,
}

/// Computes `Fx` structurally, to a fixpoint: a region's user effects are what
/// it performs and what the rows of its callees name; a handle subtracts the
/// effect its clauses name (E0423: one per handle) -- if they cover every op of
/// it the module performs -- from its body's and adds its
/// clauses' and return clause's; a resume contributes its own handle's leaks.
/// The convention checks (D16) refuse, by name, any disagreement with the types.
pub fn effect_facts(core: &CoreModule) -> Fx {
    let mut owner: HashMap<usize, usize> = HashMap::new();
    for f in &core.fns {
        resume_owners(&f.body, None, &mut owner);
    }
    // Every op the module performs, by effect: a handle HANDLES an effect only
    // if it has a clause for each of them. The front end accepts a handle with
    // clauses for some of an effect's ops and types it as discharging all of
    // it, while at run time the other ops go to an outer handler -- so such a
    // handle LEAKS that effect (found by the 5b-10 review: compiled direct,
    // the outer clause's continuation ended at the inner frame -- 2220 where
    // the evaluator printed 1220).
    let mut performed: HashMap<String, BTreeSet<String>> = HashMap::new();
    for f in &core.fns {
        performed_ops(&f.body, &mut performed);
    }
    let mut leaks: HashMap<usize, BTreeSet<String>> = HashMap::new();
    loop {
        let mut changed = false;
        for f in &core.fns {
            region_effects(&f.body, &owner, &performed, &mut leaks, &mut changed);
        }
        if !changed {
            break;
        }
    }
    let leaking: HashSet<usize> = leaks
        .iter()
        .filter(|(_, l)| !l.is_empty())
        .map(|(k, _)| *k)
        .collect();
    let cps_resumes = owner
        .iter()
        .filter(|(_, h)| leaking.contains(h))
        .map(|(r, _)| *r)
        .collect();
    Fx {
        leaking,
        cps_resumes,
    }
}

fn key(e: &CoreExpr) -> usize {
    e as *const CoreExpr as usize
}

/// Every `(effect, op)` performed anywhere in `e`, lambda bodies and handle
/// regions included.
fn performed_ops(e: &CoreExpr, out: &mut HashMap<String, BTreeSet<String>>) {
    if let CoreKind::Perform(p) = &e.kind {
        out.entry(p.effect.clone())
            .or_default()
            .insert(p.op.clone());
    }
    let mut go = |c: &CoreExpr| performed_ops(c, out);
    match &e.kind {
        CoreKind::Lit(_) | CoreKind::Var(_) => {}
        CoreKind::App(f, args) => {
            go(f);
            args.iter().for_each(go);
        }
        CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => {
            a.iter().for_each(go)
        }
        CoreKind::Perform(p) => p.args.iter().for_each(go),
        CoreKind::Lambda(_, body) | CoreKind::Resume(body) => go(body),
        CoreKind::Let(_, v, b) => {
            go(v);
            go(b);
        }
        CoreKind::If(c, t, f) => {
            go(c);
            go(t);
            go(f);
        }
        CoreKind::Match(s, arms) => {
            go(s);
            arms.iter().for_each(|a| go(&a.body));
        }
        CoreKind::Handle(h) => {
            go(&h.body);
            h.clauses.iter().for_each(|c| go(&c.body));
            if let Some(r) = &h.ret {
                go(&r.body);
            }
        }
    }
}

/// Each `resume` node -> the `handle` whose clause encloses it.
fn resume_owners(e: &CoreExpr, clause_of: Option<usize>, out: &mut HashMap<usize, usize>) {
    if let (CoreKind::Resume(_), Some(h)) = (&e.kind, clause_of) {
        out.insert(key(e), h);
    }
    let mut go = |c: &CoreExpr| resume_owners(c, clause_of, out);
    match &e.kind {
        CoreKind::Lit(_) | CoreKind::Var(_) => {}
        CoreKind::App(f, args) => {
            go(f);
            args.iter().for_each(go);
        }
        CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => {
            a.iter().for_each(go)
        }
        CoreKind::Perform(p) => p.args.iter().for_each(go),
        CoreKind::Lambda(_, body) => go(body),
        CoreKind::Resume(v) => go(v),
        CoreKind::Let(_, v, b) => {
            go(v);
            go(b);
        }
        CoreKind::If(c, t, f) => {
            go(c);
            go(t);
            go(f);
        }
        CoreKind::Match(s, arms) => {
            go(s);
            arms.iter().for_each(|a| go(&a.body));
        }
        CoreKind::Handle(h) => {
            go(&h.body);
            if let Some(r) = &h.ret {
                go(&r.body);
            }
        }
    }
    if let CoreKind::Handle(h) = &e.kind {
        for c in h.clauses.iter() {
            resume_owners(&c.body, Some(key(e)), out);
        }
    }
}

/// The user effects `e`'s own region may perform (lambda bodies are regions of
/// their own: visited for their handles, contributing nothing here). Records
/// every handle's leaks in `leaks`, setting `changed` when one grows.
fn region_effects(
    e: &CoreExpr,
    owner: &HashMap<usize, usize>,
    performed: &HashMap<String, BTreeSet<String>>,
    leaks: &mut HashMap<usize, BTreeSet<String>>,
    changed: &mut bool,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    {
        let mut go = |c: &CoreExpr, out: &mut BTreeSet<String>| {
            out.extend(region_effects(c, owner, performed, leaks, changed));
        };
        match &e.kind {
            CoreKind::Lit(_) | CoreKind::Var(_) => {}
            CoreKind::App(f, args) => {
                if let Ty::Fn(_, row, _) = &f.ty {
                    out.extend(row.labels.keys().filter(|l| *l != BUILTIN_EFFECT).cloned());
                }
                go(f, &mut out);
                args.iter().for_each(|a| go(a, &mut out));
            }
            CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => {
                a.iter().for_each(|x| go(x, &mut out))
            }
            CoreKind::Perform(p) => {
                if p.effect != BUILTIN_EFFECT {
                    out.insert(p.effect.clone());
                }
                p.args.iter().for_each(|a| go(a, &mut out));
            }
            CoreKind::Lambda(_, body) => {
                let _ = region_effects(body, owner, performed, leaks, changed);
            }
            CoreKind::Resume(v) => {
                go(v, &mut out);
                if let Some(h) = owner.get(&key(e)) {
                    if let Some(l) = leaks.get(h) {
                        out.extend(l.iter().cloned());
                    }
                }
            }
            CoreKind::Let(_, v, b) => {
                go(v, &mut out);
                go(b, &mut out);
            }
            CoreKind::If(c, t, f) => {
                go(c, &mut out);
                go(t, &mut out);
                go(f, &mut out);
            }
            CoreKind::Match(s, arms) => {
                go(s, &mut out);
                arms.iter().for_each(|a| go(&a.body, &mut out));
            }
            CoreKind::Handle(_) => {}
        }
    }
    if let CoreKind::Handle(h) = &e.kind {
        let covers = |eff: &String| {
            performed.get(eff).map_or(true, |ops| {
                ops.iter()
                    .all(|op| h.clauses.iter().any(|c| &c.effect == eff && &c.op == op))
            })
        };
        let handled: BTreeSet<&String> = h
            .clauses
            .iter()
            .map(|c| &c.effect)
            .filter(|eff| covers(eff))
            .collect();
        let body = region_effects(&h.body, owner, performed, leaks, changed);
        out.extend(body.into_iter().filter(|l| !handled.contains(l)));
        for c in h.clauses.iter() {
            out.extend(region_effects(&c.body, owner, performed, leaks, changed));
        }
        if let Some(r) = &h.ret {
            out.extend(region_effects(&r.body, owner, performed, leaks, changed));
        }
        let entry = leaks.entry(key(e)).or_default();
        for l in &out {
            if entry.insert(l.clone()) {
                *changed = true;
            }
        }
    }
    out
}

/// True iff `e` needs the CPS machinery in its OWN region: it contains a
/// `Perform`, a call whose callee type `needs_cps`, a LEAKING `handle` or a
/// resume of one (5b-10), outside any lambda body and outside any handle's
/// own regions. D16: a top-level function is CPS iff its body answers true.
pub fn contains_effect(e: &CoreExpr, fx: &Fx) -> bool {
    if is_call_site(e, fx) {
        return true;
    }
    let go = |c: &CoreExpr| contains_effect(c, fx);
    match &e.kind {
        CoreKind::Lit(_) | CoreKind::Var(_) | CoreKind::Lambda(..) | CoreKind::Handle(_) => false,
        CoreKind::App(f, args) => go(f) || args.iter().any(go),
        CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => a.iter().any(go),
        CoreKind::Perform(p) => p.args.iter().any(go),
        CoreKind::Let(_, v, b) => go(v) || go(b),
        CoreKind::If(c, t, f) => go(c) || go(t) || go(f),
        CoreKind::Match(s, arms) => go(s) || arms.iter().any(|a| go(&a.body)),
        CoreKind::Resume(v) => go(v),
    }
}

/// Does `ty` NAME a user-declared effect anywhere -- in its own row, or in a
/// row inside a parameter, result or constructor argument? (D16's refusal.)
pub fn ty_names_user_effect(ty: &Ty) -> bool {
    match ty {
        Ty::Fn(ps, row, r) => {
            row.labels.keys().any(|l| l != BUILTIN_EFFECT)
                || ps.iter().any(ty_names_user_effect)
                || ty_names_user_effect(r)
        }
        Ty::Con(_, args) => args.iter().any(ty_names_user_effect),
        _ => false,
    }
}

/// Does `ty` contain an open row tail anywhere -- is it effect-polymorphic?
pub fn ty_has_open_row(ty: &Ty) -> bool {
    match ty {
        Ty::Fn(ps, row, r) => {
            matches!(row.tail, RowTail::Open(_))
                || ps.iter().any(ty_has_open_row)
                || ty_has_open_row(r)
        }
        Ty::Con(_, args) => args.iter().any(ty_has_open_row),
        _ => false,
    }
}

/// Is child `slot` of `e` in tail position of `e` (its value is `e`'s value,
/// with nothing left to do)? The one rule `in_tail` and 7b-3's emitter share.
pub fn is_tail_slot(e: &CoreExpr, slot: usize) -> bool {
    match &e.kind {
        CoreKind::Let(..) => slot == 1,
        CoreKind::If(..) | CoreKind::Match(..) => slot >= 1,
        _ => false,
    }
}

/// An ancestor of the node being visited, inside the current region: the
/// node, which child the walk descended into, and the scope depth AT the
/// ancestor (the bindings its later children can see).
struct Step<'a> {
    node: &'a CoreExpr,
    slot: usize,
    depth: usize,
}

fn is_call_site(e: &CoreExpr, fx: &Fx) -> bool {
    match &e.kind {
        CoreKind::Perform(_) => true,
        CoreKind::App(f, _) => needs_cps(&f.ty),
        // 5b-10: a CPS handle and a CPS resume suspend their region: the rest
        // of it is the handle's (or the resume's) continuation.
        CoreKind::Handle(_) => fx.leaking.contains(&key(e)),
        CoreKind::Resume(_) => fx.cps_resumes.contains(&key(e)),
        _ => false,
    }
}

/// Tail slots: a `Let` body, an `If` branch, a `Match` arm body. Nothing else
/// returns its child's value as its own without further work.
fn in_tail(path: &[Step]) -> bool {
    path.iter().all(|s| is_tail_slot(s.node, s.slot))
}

/// The match is EXHAUSTIVE with no catch-all, for the reason `fv_walk` gives:
/// a missed region is a missed site, and a missed site is a frame that does not
/// exist when the continuation is captured.
fn walk<'a>(
    e: &'a CoreExpr,
    owner: &str,
    scope: &mut Vec<String>,
    path: &mut Vec<Step<'a>>,
    out: &mut Out,
) {
    if is_call_site(e, &out.fx) && !in_tail(path) {
        out.sites.push(ContSite {
            key: e as *const CoreExpr as usize,
            owner: owner.to_string(),
            path: path
                .iter()
                .map(|s| PathStep {
                    key: s.node as *const CoreExpr as usize,
                    slot: s.slot,
                    depth: s.depth,
                })
                .collect(),
            saved: saved_at(path, scope),
        });
    }
    let visit = |child: &'a CoreExpr,
                 slot: usize,
                 scope: &mut Vec<String>,
                 path: &mut Vec<Step<'a>>,
                 out: &mut Out| {
        path.push(Step {
            node: e,
            slot,
            depth: scope.len(),
        });
        walk(child, owner, scope, path, out);
        path.pop();
    };
    match &e.kind {
        CoreKind::Lit(_) | CoreKind::Var(_) => {}
        CoreKind::App(f, args) => {
            visit(f, 0, scope, path, out);
            for (i, a) in args.iter().enumerate() {
                visit(a, i + 1, scope, path, out);
            }
        }
        CoreKind::Builtin(_, args) | CoreKind::Ctor(_, args) | CoreKind::Prim(_, args) => {
            for (i, a) in args.iter().enumerate() {
                visit(a, i, scope, path, out);
            }
        }
        CoreKind::Perform(p) => {
            for (i, a) in p.args.iter().enumerate() {
                visit(a, i, scope, path, out);
            }
        }
        CoreKind::Let(name, value, body) => {
            visit(value, 0, scope, path, out);
            let depth = scope.len();
            scope.push(name.clone());
            visit(body, 1, scope, path, out);
            scope.truncate(depth);
        }
        CoreKind::If(c, t, f) => {
            visit(c, 0, scope, path, out);
            visit(t, 1, scope, path, out);
            visit(f, 2, scope, path, out);
        }
        CoreKind::Match(scrutinee, arms) => {
            visit(scrutinee, 0, scope, path, out);
            for (i, arm) in arms.iter().enumerate() {
                let depth = scope.len();
                pat_binders(&arm.pat, scope);
                visit(&arm.body, i + 1, scope, path, out);
                scope.truncate(depth);
            }
        }
        CoreKind::Resume(v) => visit(v, 0, scope, path, out),
        // New regions: each starts with an empty path and the scope it sees.
        CoreKind::Lambda(params, body) => {
            out.lambdas.insert(
                e as *const CoreExpr as usize,
                LambdaRegion {
                    scope: scope.clone(),
                },
            );
            let depth = scope.len();
            for p in params.iter() {
                scope.push(p.name.clone());
            }
            walk(body, owner, scope, &mut Vec::new(), out);
            scope.truncate(depth);
        }
        CoreKind::Handle(h) => {
            out.handlers.push(HandlerSite {
                key: e as *const CoreExpr as usize,
                owner: owner.to_string(),
                depth: scope.len(),
                saved: handler_saved(h, scope),
            });
            walk(&h.body, owner, scope, &mut Vec::new(), out);
            for c in h.clauses.iter() {
                let depth = scope.len();
                for p in c.params.iter() {
                    scope.push(p.name.clone());
                }
                scope.push(CONT.to_string());
                walk(&c.body, owner, scope, &mut Vec::new(), out);
                scope.truncate(depth);
            }
            if let Some(r) = &h.ret {
                let depth = scope.len();
                scope.push(r.binder.clone());
                walk(&r.body, owner, scope, &mut Vec::new(), out);
                scope.truncate(depth);
            }
        }
    }
}

/// What the rest of the region needs once the site's call returns.
fn saved_at(path: &[Step], scope: &[String]) -> Vec<(Saved, Ty)> {
    let mut temps: Vec<(Saved, Ty)> = Vec::new();
    let mut vars: std::collections::BTreeMap<usize, (String, Ty)> =
        std::collections::BTreeMap::new();
    // A name free in something an ancestor still has to run refers to the
    // binding visible AT that ancestor -- the innermost one below its depth.
    let mut need = |name: &str, ty: &Ty, depth: usize| {
        if let Some(b) = scope[..depth].iter().rposition(|n| n == name) {
            vars.entry(b)
                .or_insert_with(|| (name.to_string(), ty.clone()));
        }
    };
    for step in path {
        let operands: Vec<&CoreExpr> = match &step.node.kind {
            CoreKind::App(f, args) => std::iter::once(&**f).chain(args.iter()).collect(),
            CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => {
                a.iter().collect()
            }
            CoreKind::Perform(p) => p.args.iter().collect(),
            CoreKind::Let(name, _, body) => {
                if step.slot == 0 {
                    for (n, t) in free_vars_under(body, std::slice::from_ref(name)) {
                        need(&n, &t, step.depth);
                    }
                }
                continue;
            }
            CoreKind::If(_, t, f) => {
                if step.slot == 0 {
                    for b in [t, f] {
                        for (n, ty) in free_vars_under(b, &[]) {
                            need(&n, &ty, step.depth);
                        }
                    }
                }
                continue;
            }
            CoreKind::Match(_, arms) => {
                if step.slot == 0 {
                    for arm in arms.iter() {
                        let mut binders = Vec::new();
                        pat_binders(&arm.pat, &mut binders);
                        for (n, ty) in free_vars_under(&arm.body, &binders) {
                            need(&n, &ty, step.depth);
                        }
                    }
                }
                continue;
            }
            // The resume still has to RUN after the site returns, and it needs
            // the clause's continuation -- whose native stack is gone once the
            // site suspends. (Found by independent review: before this arm, a
            // site inside `resume(..)`'s argument saved nothing.)
            CoreKind::Resume(v) => {
                need(CONT, &cont_ty(&v.ty, &step.node.ty), step.depth);
                continue;
            }
            // Lit, Var, Lambda and Handle are never on a path (they are leaves
            // or region boundaries).
            CoreKind::Lit(_) | CoreKind::Var(_) | CoreKind::Lambda(..) | CoreKind::Handle(_) => {
                continue
            }
        };
        for (i, op) in operands.iter().enumerate() {
            if i < step.slot {
                // Evaluated before the site; its value is used after it.
                match &op.kind {
                    CoreKind::Lit(_) => {}
                    CoreKind::Var(x) => need(x, &op.ty, step.depth),
                    _ => temps.push((Saved::Temp(*op as *const CoreExpr as usize), op.ty.clone())),
                }
            } else if i > step.slot {
                for (n, ty) in free_vars_under(op, &[]) {
                    need(&n, &ty, step.depth);
                }
            }
        }
    }
    temps
        .into_iter()
        .chain(
            vars.into_iter()
                .map(|(binding, (name, ty))| (Saved::Var { name, binding }, ty)),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use elya::span::Span;
    use elya::types::{EffectLabel, EffectRow, RowTail, Ty, TyCon};
    use std::collections::BTreeMap;

    fn row(labels: &[&str], tail: RowTail) -> EffectRow {
        let mut m = BTreeMap::new();
        for l in labels {
            m.insert(
                (*l).to_string(),
                EffectLabel {
                    args: Vec::new(),
                    span: Span::EMPTY,
                },
            );
        }
        EffectRow { labels: m, tail }
    }

    fn func(r: EffectRow) -> Ty {
        Ty::Fn(
            vec![Ty::Base(TyCon::Int)],
            r,
            Box::new(Ty::Base(TyCon::Int)),
        )
    }

    #[test]
    fn a_pure_function_is_a_direct_call() {
        assert!(!needs_cps(&func(EffectRow::pure())));
    }

    #[test]
    fn a_printing_function_is_still_a_direct_call() {
        // The whole point of 8.2: `{IO}` must NOT select CPS, or every
        // function that prints pays for machinery it cannot use.
        assert!(!needs_cps(&func(row(&["IO"], RowTail::Closed))));
    }

    #[test]
    fn a_user_declared_effect_selects_cps() {
        assert!(needs_cps(&func(row(&["State"], RowTail::Closed))));
    }

    #[test]
    fn a_user_effect_alongside_io_still_selects_cps() {
        assert!(needs_cps(&func(row(&["IO", "State"], RowTail::Closed))));
    }

    #[test]
    fn an_unresolved_tail_selects_cps_conservatively() {
        // Over-CPS costs speed. Under-CPS is a wrong answer. Same asymmetry
        // `fv_walk` cites for over- vs under-capture (closure.rs). This one is
        // the poison tail; the open row variable has its own test below, so
        // each unresolved tail fails on its own.
        assert!(needs_cps(&func(row(&[], RowTail::ErrorRow))));
    }

    #[test]
    fn an_open_row_tail_alone_is_a_direct_call() {
        // D16 (approved expected-value change; 7a asserted `true`). A
        // row-polymorphic function is compiled once, direct; instantiating it
        // at a user effect is refused by name instead (N7).
        assert!(!needs_cps(&func(row(&[], RowTail::Open(0)))));
    }

    #[test]
    fn a_non_function_type_is_not_a_call_site_at_all() {
        assert!(!needs_cps(&Ty::Base(TyCon::Int)));
    }

    // ---- 7b-2: continuation sites (D10) and the implicit capture (D12) ----

    fn core_of(src: &str) -> elya::core::CoreModule {
        let session = elya::Session::new();
        let (m, pd) = elya::parse::parse_module(&session, src);
        assert!(pd.is_empty(), "parse: {pd:?}");
        let (diags, table) = elya::types::infer_typed_table(&session, &m);
        assert!(diags.is_empty(), "type errors: {diags:?}");
        elya::core::lower_module(&m, &table).expect("lowers")
    }

    const S: &str = "effect S { fn get() -> Int }\n";

    fn sites_of(src: &str) -> (elya::core::CoreModule, Vec<ContSite>) {
        let core = core_of(&format!("{S}{src}"));
        let sites = collect_sites(&core);
        (core, sites)
    }

    /// The table `build_module` emits, for the same module: real ctors, then
    /// lambdas, then the string row, the frame row, then one row per site.
    fn rows_of(core: &elya::core::CoreModule, sites: &[ContSite]) -> crate::Descriptors {
        let n_real_ctors: usize = core.types.iter().map(|t| t.ctors.len()).sum();
        let lambdas = crate::closure::collect_lambdas(core, n_real_ctors);
        let string_tag = n_real_ctors + lambdas.len();
        crate::descriptor_rows(
            core,
            &lambdas,
            sites,
            &collect_handlers(core),
            n_real_ctors,
            string_tag,
        )
        .expect("rows")
    }

    fn row_at(d: &crate::Descriptors, tag: usize) -> [u64; 2] {
        [d.rows[2 * tag], d.rows[2 * tag + 1]]
    }

    fn names(site: &ContSite) -> Vec<String> {
        site.saved
            .iter()
            .map(|(s, _)| match s {
                Saved::Var { name, .. } => name.clone(),
                Saved::Temp(_) => "<temp>".to_string(),
            })
            .collect()
    }

    #[test]
    fn a_tail_perform_needs_no_frame() {
        // The current continuation is passed straight through (D14).
        let (_c, sites) = sites_of("fn tail() -> Int { get() }\n");
        assert!(
            sites.is_empty(),
            "a tail perform is not a site: {}",
            sites.len()
        );
    }

    #[test]
    fn a_non_tail_perform_is_a_site_and_a_literal_sibling_is_not_saved() {
        let (core, sites) = sites_of("fn nontail() -> Int { get() + 1 }\n");
        assert_eq!(sites.len(), 1);
        assert!(names(&sites[0]).is_empty(), "{:?}", names(&sites[0]));
        let d = rows_of(&core, &sites);
        assert_eq!(
            d.site_tags[&sites[0].key], d.frame_tag,
            "no saved values -> Task 6's row"
        );
    }

    #[test]
    fn a_live_local_is_saved_by_the_site() {
        let (_c, sites) = sites_of("fn g(x) { x + get() }\nfn u() -> Int { g(1) }\n");
        assert_eq!(sites.len(), 1, "g's perform only; u's call is a tail call");
        assert_eq!(names(&sites[0]), vec!["x"]);
    }

    #[test]
    fn an_already_evaluated_operand_is_saved_as_a_temporary_not_recomputed() {
        // `x * 2` ran before the perform; re-running it after resumption would
        // repeat effects in general, so its VALUE is saved, and `x` itself is
        // dead after it.
        let (_c, sites) = sites_of("fn h(x) { (x * 2) + get() }\nfn u() -> Int { h(1) }\n");
        assert_eq!(sites.len(), 1);
        assert_eq!(names(&sites[0]), vec!["<temp>"]);
    }

    #[test]
    fn a_heap_saved_value_is_listed_with_its_type() {
        let (_c, sites) = sites_of("fn g() -> Str { let s = \"a\"  let n = get()  s }\n");
        assert_eq!(sites.len(), 1);
        assert_eq!(names(&sites[0]), vec!["s"]);
    }

    #[test]
    fn a_site_row_sets_bit_j_plus_2_for_a_heap_saved_value() {
        // D10: [tag][code_ptr][next][saved_0..] -- bit 0 clear (code), bit 1
        // set (next), bit j+2 set iff saved j is heap.
        let (core, sites) = sites_of("fn g() -> Str { let s = \"a\"  let n = get()  s }\n");
        let d = rows_of(&core, &sites);
        let tag = d.site_tags[&sites[0].key];
        assert_eq!(row_at(&d, tag), [3, 0b110]);
    }

    #[test]
    fn a_site_whose_remainder_needs_nothing_uses_the_frame_row() {
        let (core, sites) = sites_of("fn g() -> Int { let n = get()  n + 1 }\n");
        assert_eq!(sites.len(), 1);
        let d = rows_of(&core, &sites);
        assert_eq!(d.site_tags[&sites[0].key], d.frame_tag);
    }

    #[test]
    fn a_non_tail_call_of_an_effectful_function_is_a_site() {
        let (_c, sites) =
            sites_of("fn w() -> Int { get() }\nfn u(x) { w() + x }\nfn v() -> Int { u(1) }\n");
        assert_eq!(sites.len(), 1, "only u's `w()`");
        assert_eq!(names(&sites[0]), vec!["x"]);
    }

    #[test]
    fn a_site_in_a_handled_body_saves_the_enclosing_local() {
        let (_c, sites) = sites_of(
            "pub fn main() -> Int {\n  let y = 5\n  handle { y + get() } with {\n    \
             S.get() -> resume(1)\n    return(r) -> r\n  }\n}\n",
        );
        assert_eq!(sites.len(), 1);
        assert_eq!(names(&sites[0]), vec!["y"]);
    }

    const RESUME_IN_LAMBDA: &str =
        "pub fn main() -> Int {\n  let f = handle { get() } with {\n    \
         S.get() -> fn(s) { (resume(s))(s) }\n    return(x) -> fn(s) { x }\n  }\n  f(1)\n}\n";

    #[test]
    fn a_lambda_that_resumes_captures_the_continuation() {
        // D12: the lambda runs after the handle returned, so it must carry the
        // clause's continuation.
        let core = core_of(&format!("{S}{RESUME_IN_LAMBDA}"));
        let lambdas = crate::closure::collect_lambdas(&core, 0);
        assert!(
            lambdas
                .iter()
                .any(|l| l.captures.iter().any(|(n, _)| n == crate::closure::CONT)),
            "no lambda captures {}",
            crate::closure::CONT
        );
    }

    #[test]
    fn the_captured_continuation_is_traced_by_its_lambda_row() {
        // D12: a clear bit leaves the frame chain untraced, and a collection
        // between the handle returning and the lambda resuming frees it.
        let core = core_of(&format!("{S}{RESUME_IN_LAMBDA}"));
        let n_real_ctors: usize = core.types.iter().map(|t| t.ctors.len()).sum();
        let lambdas = crate::closure::collect_lambdas(&core, n_real_ctors);
        let (site, j) = lambdas
            .iter()
            .find_map(|l| {
                l.captures
                    .iter()
                    .position(|(n, _)| n == crate::closure::CONT)
                    .map(|j| (l, j))
            })
            .expect("a lambda captures the continuation");
        let d = rows_of(&core, &collect_sites(&core));
        let mask = d.rows[2 * site.tag + 1];
        assert_ne!(
            mask & (1 << (j + 1)),
            0,
            "continuation capture bit clear: {mask:#b}"
        );
    }

    #[test]
    fn two_live_bindings_sharing_a_name_get_two_slots() {
        // Both `x`s are live after the perform: the inner one in `get() + x`,
        // the outer one in `r + x`. The evaluator answers 10 + 2 + 1 = 13 with
        // `resume(10)`; saving by NAME would keep one `x` and compute 14 or 12.
        let (_c, sites) = sites_of(
            "fn g() -> Int {\n  let x = 1\n  let r = { let x = 2  get() + x }\n  r + x\n}\n",
        );
        assert_eq!(sites.len(), 1);
        assert_eq!(names(&sites[0]), vec!["x", "x"]);
    }

    #[test]
    fn a_site_inside_a_resume_argument_saves_the_continuation() {
        // Found by independent review. After `t()` returns, the remainder must
        // still RESUME, and the clause's native stack is gone -- so the clause's
        // continuation must be in the frame. (`{ let v = t()  resume(v) }`
        // already saved it, through the Let step; the direct-ancestor case did not.)
        let (_c, sites) = sites_of(
            "effect T { fn t() -> Int }\n\
             pub fn main() -> Int {\n  handle {\n    handle { get() + 1 } with {\n      \
             S.get() -> resume(t())\n      return(r) -> r\n    }\n  } with {\n    \
             T.t() -> resume(10)\n    return(r) -> r\n  }\n}\n",
        );
        assert_eq!(sites.len(), 2, "get() + 1 and t()");
        assert!(
            sites.iter().any(|s| names(s) == vec![crate::closure::CONT]),
            "the t() site must save the continuation: {:?}",
            sites.iter().map(names).collect::<Vec<_>>()
        );
    }
}
