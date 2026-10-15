//! N7 part 1 (2026-10-09): convention specialization.
//!
//! A row-polymorphic function is compiled once, DIRECT (D16): its convention
//! must not depend on what its row variables are instantiated with. That made
//! every use of one at a user effect a named refusal ("effect-polymorphic
//! function used at a user effect") -- and that is exactly how a colourless
//! helper is used: `each(ws, f)` with a plain callback and with one that
//! yields (`examples/04_async.elya`).
//!
//! Instead, a use that instantiates some of a function's signature row
//! variables at a user effect gets a CLONE of the function, `name$cps<mask>`,
//! in which those variables are replaced by a marker label, [`POLY`]. To every
//! later pass the marker is just one more user effect: calls through it are
//! continuation sites, lambdas whose rows carry it take the CPS convention, and
//! the clone itself is CPS when its body calls through it. No performs or
//! handles ever name it, so it never reaches dispatch. Clones are made on
//! demand, to a fixpoint (a clone's body may itself use generic functions at
//! the marker), and at most once per (function, mask) -- a function with n
//! signature row variables has at most 2^n versions, and in practice one or two.
//!
//! The same machinery MONOMORPHIZES type variables (N7 part 2's first half,
//! the roadmap's "specialisation in core code"): a use at ground types gets a
//! clone with its signature type variables replaced by them, so `Ty::Var` --
//! "unrepresentable type" natively -- never reaches the back end for a
//! function that is only ever used at concrete types. HM has no polymorphic
//! recursion, so a clone's own recursive calls reuse its key: the fixpoint is
//! finite. A generic original nothing references any more is not emitted.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::rc::Rc;

use elya::core::{
    CoreArm, CoreClause, CoreCtor, CoreExpr, CoreFn, CoreHandle, CoreKind, CoreModule, CoreParam,
    CorePat, CorePerform, CoreReturn, CoreType,
};
use elya::types::{EffectLabel, EffectRow, RowTail, RowVar, Ty};

use crate::CodegenError;

/// The marker label a clone's specialized row variables become.
pub(crate) const POLY: &str = "$poly";

const BUILTIN_EFFECT: &str = "IO";

/// The row variables of a function's signature (parameter and result types),
/// in a fixed order: bit `i` of a mask is `sig_vars(f)[i]`.
pub(crate) fn sig_vars(f: &CoreFn) -> Vec<RowVar> {
    let mut out = Vec::new();
    for p in f.params.iter() {
        row_vars(&p.ty, &mut out);
    }
    row_vars(&f.body.ty, &mut out);
    out
}

/// The type variables of a function's signature, in a fixed order.
pub(crate) fn sig_tvars(f: &CoreFn) -> Vec<u32> {
    let mut out = Vec::new();
    for p in f.params.iter() {
        tvars(&p.ty, &mut out);
    }
    tvars(&f.body.ty, &mut out);
    out
}

fn tvars(t: &Ty, out: &mut Vec<u32>) {
    match t {
        Ty::Var(v) => {
            if !out.contains(v) {
                out.push(*v);
            }
        }
        Ty::Fn(ps, row, r) => {
            for p in ps {
                tvars(p, out);
            }
            for l in row.labels.values() {
                for a in &l.args {
                    tvars(a, out);
                }
            }
            tvars(r, out);
        }
        Ty::Con(_, args) | Ty::Tuple(args) => args.iter().for_each(|a| tvars(a, out)),
        _ => {}
    }
}

fn is_ground(t: &Ty) -> bool {
    let mut v = Vec::new();
    tvars(t, &mut v);
    v.is_empty()
}

fn row_vars(t: &Ty, out: &mut Vec<RowVar>) {
    match t {
        Ty::Fn(ps, row, r) => {
            for p in ps {
                row_vars(p, out);
            }
            for l in row.labels.values() {
                for a in &l.args {
                    row_vars(a, out);
                }
            }
            if let RowTail::Open(v) = row.tail {
                if !out.contains(&v) {
                    out.push(v);
                }
            }
            row_vars(r, out);
        }
        Ty::Con(_, args) | Ty::Tuple(args) => args.iter().for_each(|a| row_vars(a, out)),
        _ => {}
    }
}

/// Which of `f`'s signature row variables the use type `at` instantiates at a
/// USER effect (the remainder of the use's row beyond the generic row's labels
/// names something other than `IO`), as a mask over `sig_vars(f)`.
pub(crate) fn instantiation_mask(f: &CoreFn, at: &Ty) -> u64 {
    instantiation(f, at).0
}

/// The use's row mask and, for each signature type variable in order, the
/// type it is instantiated at (when the walk reached it).
fn instantiation(f: &CoreFn, at: &Ty) -> (u64, Vec<Option<Ty>>) {
    let generic = Ty::Fn(
        f.params.iter().map(|p| p.ty.clone()).collect(),
        EffectRow {
            labels: BTreeMap::new(),
            tail: RowTail::Closed,
        },
        Box::new(f.body.ty.clone()),
    );
    let mut eff: HashMap<RowVar, bool> = HashMap::new();
    let mut tys: HashMap<u32, Ty> = HashMap::new();
    walk(&generic, at, &mut eff, &mut tys);
    let vars = sig_vars(f);
    let mut mask = 0u64;
    for (i, v) in vars.iter().enumerate().take(64) {
        if eff.get(v).copied().unwrap_or(false) {
            mask |= 1 << i;
        }
    }
    let args = sig_tvars(f).iter().map(|v| tys.get(v).cloned()).collect();
    (mask, args)
}

fn walk(generic: &Ty, at: &Ty, eff: &mut HashMap<RowVar, bool>, tys: &mut HashMap<u32, Ty>) {
    match (generic, at) {
        (Ty::Var(v), t) => {
            tys.entry(*v).or_insert_with(|| t.clone());
        }
        (Ty::Fn(ps, row, r), Ty::Fn(ps2, row2, r2)) => {
            for (a, b) in ps.iter().zip(ps2.iter()) {
                walk(a, b, eff, tys);
            }
            walk(r, r2, eff, tys);
            if let RowTail::Open(v) = row.tail {
                let user = row2
                    .labels
                    .keys()
                    .any(|l| !row.labels.contains_key(l) && l != BUILTIN_EFFECT);
                *eff.entry(v).or_insert(false) |= user;
            }
        }
        (Ty::Con(_, a), Ty::Con(_, b)) | (Ty::Tuple(a), Ty::Tuple(b)) => {
            for (x, y) in a.iter().zip(b.iter()) {
                walk(x, y, eff, tys);
            }
        }
        _ => {}
    }
}

/// What a use needs: the original (`None`) or a clone keyed by its row mask
/// and its ground type arguments.
fn needed_clone(f: &CoreFn, at: &Ty) -> Option<(u64, Vec<Ty>)> {
    let (mask, args) = instantiation(f, at);
    let args: Option<Vec<Ty>> = args.into_iter().collect();
    match args {
        // Every signature type variable reached, and at a ground type.
        Some(args) if args.iter().all(is_ground) => {
            if mask == 0 && args.is_empty() {
                None
            } else {
                Some((mask, args))
            }
        }
        // Not monomorphizable here (the caller is itself generic): only the
        // convention can be specialized.
        _ if mask != 0 && sig_tvars(f).is_empty() => Some((mask, Vec::new())),
        _ => None,
    }
}

/// The most clones one module may need. HM has no polymorphic recursion, so
/// the worklist is finite; this is a backstop that turns a bug into a named
/// refusal instead of a build that never finishes.
const MAX_CLONES: usize = 4096;

/// The module with every needed clone added and every use at a user effect
/// redirected to its clone. A module with no such use comes back unchanged.
pub(crate) fn specialize(core: &CoreModule) -> Result<CoreModule, CodegenError> {
    let by_name: HashMap<String, &CoreFn> = core.fns.iter().map(|f| (f.name.clone(), f)).collect();
    let generic: HashSet<String> = core
        .fns
        .iter()
        .filter(|f| !sig_vars(f).is_empty() || !sig_tvars(f).is_empty())
        .map(|f| f.name.clone())
        .collect();
    // (function, mask, [`clone_key`] of the type arguments) -> the clone.
    let mut made: HashMap<(String, u64, String), String> = HashMap::new();
    let mut queue: VecDeque<CoreFn> = core.fns.iter().cloned().collect();
    let mut out: Vec<CoreFn> = Vec::new();
    while let Some(f) = queue.pop_front() {
        let mut needed: Vec<(String, u64, Vec<Ty>, String)> = Vec::new();
        let mut scope: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        let mut cx = Redirect {
            by_name: &by_name,
            generic: &generic,
            made: &mut made,
            needed: &mut needed,
        };
        let body = cx.expr(&f.body, &mut scope);
        if made.len() > MAX_CLONES {
            return Err(CodegenError::Unsupported("specialization did not converge"));
        }
        for (name, mask, args, clone) in needed {
            queue.push_back(make_clone(by_name[&name], mask, &args, clone));
        }
        out.push(CoreFn {
            name: f.name.clone(),
            params: f.params.clone(),
            body,
        });
    }
    // A generic original with type variables in its signature cannot be
    // represented natively; once no function reachable from `main` uses it
    // (every use went to a clone; its own recursive calls do not count), it
    // is not emitted.
    let refs_of: HashMap<String, Vec<String>> = out
        .iter()
        .map(|f| (f.name.clone(), refs(&f.body)))
        .collect();
    let mut reachable: HashSet<String> = HashSet::new();
    let mut stack = vec!["main".to_string()];
    while let Some(n) = stack.pop() {
        if !reachable.insert(n.clone()) {
            continue;
        }
        if let Some(rs) = refs_of.get(&n) {
            stack.extend(rs.iter().filter(|r| refs_of.contains_key(*r)).cloned());
        }
    }
    let out = out
        .into_iter()
        .filter(|f| sig_tvars(f).is_empty() || reachable.contains(&f.name))
        .collect();
    mono_types(out, &core.types)
}

/// A clone's identity: its type arguments with label spans dropped. Rows
/// stay (a clone's body is typed by them); a span only says where a label
/// was written, and keyed on it every use site got its own clone.
fn clone_key(args: &[Ty]) -> String {
    fn unspan(t: &Ty) -> Ty {
        match t {
            Ty::Fn(ps, row, r) => Ty::Fn(
                ps.iter().map(unspan).collect(),
                EffectRow {
                    labels: row
                        .labels
                        .iter()
                        .map(|(k, l)| {
                            (
                                k.clone(),
                                EffectLabel {
                                    args: l.args.iter().map(unspan).collect(),
                                    span: elya::span::Span::EMPTY,
                                },
                            )
                        })
                        .collect(),
                    tail: row.tail.clone(),
                },
                Box::new(unspan(r)),
            ),
            Ty::Con(n, a) => Ty::Con(n.clone(), a.iter().map(unspan).collect()),
            Ty::Tuple(a) => Ty::Tuple(a.iter().map(unspan).collect()),
            other => other.clone(),
        }
    }
    let unspanned: Vec<Ty> = args.iter().map(unspan).collect();
    format!("{unspanned:?}")
}

/// Every name a body mentions as a `Var` (an over-approximation of the
/// top-level functions it references: locals are included, harmlessly).
fn refs(e: &CoreExpr) -> Vec<String> {
    fn go(e: &CoreExpr, out: &mut Vec<String>) {
        match &e.kind {
            CoreKind::Var(n) => out.push(n.clone()),
            CoreKind::Lit(_) => {}
            CoreKind::App(f, a) => {
                go(f, out);
                a.iter().for_each(|x| go(x, out));
            }
            CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => {
                a.iter().for_each(|x| go(x, out))
            }
            CoreKind::Lambda(_, b) | CoreKind::Resume(b) => go(b, out),
            CoreKind::Let(_, v, b) => {
                go(v, out);
                go(b, out);
            }
            CoreKind::If(c, t, f) => {
                go(c, out);
                go(t, out);
                go(f, out);
            }
            CoreKind::Match(s, arms) => {
                go(s, out);
                arms.iter().for_each(|a| go(&a.body, out));
            }
            CoreKind::Handle(h) => {
                go(&h.body, out);
                h.clauses.iter().for_each(|c| go(&c.body, out));
                if let Some(r) = &h.ret {
                    go(&r.body, out);
                }
            }
            CoreKind::Perform(p) => p.args.iter().for_each(|x| go(x, out)),
        }
    }
    let mut out = Vec::new();
    go(e, &mut out);
    out
}

/// `f` with its masked signature row variables replaced by the marker and its
/// signature type variables by `args`.
fn make_clone(f: &CoreFn, mask: u64, args: &[Ty], name: String) -> CoreFn {
    let vars = sig_vars(f);
    let sub: HashSet<RowVar> = vars
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < 64 && mask & (1 << i) != 0)
        .map(|(_, v)| *v)
        .collect();
    let tsub: HashMap<u32, Ty> = sig_tvars(f).into_iter().zip(args.iter().cloned()).collect();
    let s = |t: &Ty| subst(t, &sub, &tsub);
    CoreFn {
        name,
        params: f
            .params
            .iter()
            .map(|p| CoreParam {
                name: p.name.clone(),
                ty: s(&p.ty),
            })
            .collect::<Vec<_>>()
            .into(),
        body: map_types(&f.body, &s),
    }
}

fn subst(t: &Ty, sub: &HashSet<RowVar>, tsub: &HashMap<u32, Ty>) -> Ty {
    match t {
        Ty::Var(v) => tsub.get(v).cloned().unwrap_or_else(|| t.clone()),
        Ty::Fn(ps, row, r) => {
            let mut labels: BTreeMap<String, EffectLabel> = row
                .labels
                .iter()
                .map(|(k, l)| {
                    (
                        k.clone(),
                        EffectLabel {
                            args: l.args.iter().map(|a| subst(a, sub, tsub)).collect(),
                            span: l.span,
                        },
                    )
                })
                .collect();
            let tail = match &row.tail {
                RowTail::Open(v) if sub.contains(v) => {
                    labels.insert(
                        POLY.to_string(),
                        EffectLabel {
                            args: Vec::new(),
                            span: elya::span::Span::EMPTY,
                        },
                    );
                    RowTail::Closed
                }
                other => other.clone(),
            };
            Ty::Fn(
                ps.iter().map(|p| subst(p, sub, tsub)).collect(),
                EffectRow { labels, tail },
                Box::new(subst(r, sub, tsub)),
            )
        }
        Ty::Con(n, args) => Ty::Con(
            n.clone(),
            args.iter().map(|a| subst(a, sub, tsub)).collect(),
        ),
        Ty::Tuple(args) => Ty::Tuple(args.iter().map(|a| subst(a, sub, tsub)).collect()),
        other => other.clone(),
    }
}

/// A deep copy of `e` with every type mapped through `s`.
fn map_types(e: &CoreExpr, s: &dyn Fn(&Ty) -> Ty) -> CoreExpr {
    let m = |c: &CoreExpr| map_types(c, s);
    let ms = |cs: &[CoreExpr]| cs.iter().map(m).collect::<Vec<_>>().into();
    let kind = match &e.kind {
        CoreKind::Lit(l) => CoreKind::Lit(l.clone()),
        CoreKind::Var(n) => CoreKind::Var(n.clone()),
        CoreKind::App(f, a) => CoreKind::App(Rc::new(m(f)), ms(a)),
        CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), ms(a)),
        CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), ms(a)),
        CoreKind::Prim(op, a) => CoreKind::Prim(*op, ms(a)),
        CoreKind::Lambda(ps, b) => CoreKind::Lambda(params(ps, s), Rc::new(m(b))),
        CoreKind::Let(x, v, b) => CoreKind::Let(x.clone(), Rc::new(m(v)), Rc::new(m(b))),
        CoreKind::If(c, t, f) => CoreKind::If(Rc::new(m(c)), Rc::new(m(t)), Rc::new(m(f))),
        CoreKind::Match(sc, arms) => CoreKind::Match(
            Rc::new(m(sc)),
            arms.iter()
                .map(|a| CoreArm {
                    pat: a.pat.clone(),
                    body: m(&a.body),
                })
                .collect::<Vec<_>>()
                .into(),
        ),
        CoreKind::Handle(h) => CoreKind::Handle(Rc::new(CoreHandle {
            body: Rc::new(m(&h.body)),
            clauses: h
                .clauses
                .iter()
                .map(|c| CoreClause {
                    effect: c.effect.clone(),
                    op: c.op.clone(),
                    params: params(&c.params, s),
                    body: Rc::new(m(&c.body)),
                })
                .collect::<Vec<_>>()
                .into(),
            ret: h.ret.as_ref().map(|r| {
                Rc::new(CoreReturn {
                    binder: r.binder.clone(),
                    body: Rc::new(m(&r.body)),
                })
            }),
            is_multi_declared: h.is_multi_declared,
        })),
        CoreKind::Resume(v) => CoreKind::Resume(Rc::new(m(v))),
        CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
            effect: p.effect.clone(),
            op: p.op.clone(),
            op_ty: s(&p.op_ty),
            args: ms(&p.args),
        })),
    };
    CoreExpr {
        span: e.span,
        ty: s(&e.ty),
        kind,
    }
}

fn params(ps: &[CoreParam], s: &dyn Fn(&Ty) -> Ty) -> Rc<[CoreParam]> {
    ps.iter()
        .map(|p| CoreParam {
            name: p.name.clone(),
            ty: s(&p.ty),
        })
        .collect::<Vec<_>>()
        .into()
}

/// The rewrite of one body: every reference to a generic top-level function
/// whose use needs a clone is renamed to it, and new clones are queued.
/// Locals shadow top-level names.
struct Redirect<'a, 'b> {
    by_name: &'a HashMap<String, &'a CoreFn>,
    generic: &'a HashSet<String>,
    made: &'b mut HashMap<(String, u64, String), String>,
    needed: &'b mut Vec<(String, u64, Vec<Ty>, String)>,
}

impl Redirect<'_, '_> {
    fn clone_for(&mut self, name: &str, at: &Ty) -> Option<String> {
        let f = self.by_name[name];
        let (mask, args) = needed_clone(f, at)?;
        let key = (name.to_string(), mask, clone_key(&args));
        if let Some(n) = self.made.get(&key) {
            return Some(n.clone());
        }
        let clone = format!("{name}$s{}", self.made.len());
        self.made.insert(key, clone.clone());
        self.needed
            .push((name.to_string(), mask, args, clone.clone()));
        Some(clone)
    }

    fn many(&mut self, cs: &[CoreExpr], scope: &mut Vec<String>) -> Rc<[CoreExpr]> {
        cs.iter()
            .map(|c| self.expr(c, scope))
            .collect::<Vec<_>>()
            .into()
    }

    fn expr(&mut self, e: &CoreExpr, scope: &mut Vec<String>) -> CoreExpr {
        let kind = match &e.kind {
            CoreKind::Var(n) => {
                if self.generic.contains(n) && !scope.iter().any(|s| s == n) {
                    if let Some(c) = self.clone_for(n, &e.ty) {
                        return CoreExpr {
                            span: e.span,
                            ty: e.ty.clone(),
                            kind: CoreKind::Var(c),
                        };
                    }
                }
                CoreKind::Var(n.clone())
            }
            CoreKind::Lit(l) => CoreKind::Lit(l.clone()),
            CoreKind::App(f, a) => {
                let f2 = self.expr(f, scope);
                CoreKind::App(Rc::new(f2), self.many(a, scope))
            }
            CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), self.many(a, scope)),
            CoreKind::Ctor(n, a) => CoreKind::Ctor(n.clone(), self.many(a, scope)),
            CoreKind::Prim(op, a) => CoreKind::Prim(*op, self.many(a, scope)),
            CoreKind::Lambda(ps, b) => {
                let depth = scope.len();
                scope.extend(ps.iter().map(|p| p.name.clone()));
                let b2 = self.expr(b, scope);
                scope.truncate(depth);
                CoreKind::Lambda(ps.clone(), Rc::new(b2))
            }
            CoreKind::Let(x, v, b) => {
                let v2 = self.expr(v, scope);
                scope.push(x.clone());
                let b2 = self.expr(b, scope);
                scope.pop();
                CoreKind::Let(x.clone(), Rc::new(v2), Rc::new(b2))
            }
            CoreKind::If(c, t, f) => {
                let c2 = self.expr(c, scope);
                let t2 = self.expr(t, scope);
                let f2 = self.expr(f, scope);
                CoreKind::If(Rc::new(c2), Rc::new(t2), Rc::new(f2))
            }
            CoreKind::Match(sc, arms) => {
                let sc2 = self.expr(sc, scope);
                let mut arms2 = Vec::with_capacity(arms.len());
                for a in arms.iter() {
                    let depth = scope.len();
                    crate::closure::pat_binders(&a.pat, scope);
                    let b2 = self.expr(&a.body, scope);
                    scope.truncate(depth);
                    arms2.push(CoreArm {
                        pat: a.pat.clone(),
                        body: b2,
                    });
                }
                CoreKind::Match(Rc::new(sc2), arms2.into())
            }
            CoreKind::Handle(h) => {
                let body = self.expr(&h.body, scope);
                let mut clauses = Vec::with_capacity(h.clauses.len());
                for c in h.clauses.iter() {
                    let depth = scope.len();
                    scope.extend(c.params.iter().map(|p| p.name.clone()));
                    scope.push(crate::closure::CONT.to_string());
                    let b2 = self.expr(&c.body, scope);
                    scope.truncate(depth);
                    clauses.push(CoreClause {
                        effect: c.effect.clone(),
                        op: c.op.clone(),
                        params: c.params.clone(),
                        body: Rc::new(b2),
                    });
                }
                let ret = h.ret.as_ref().map(|r| {
                    scope.push(r.binder.clone());
                    let b2 = self.expr(&r.body, scope);
                    scope.pop();
                    Rc::new(CoreReturn {
                        binder: r.binder.clone(),
                        body: Rc::new(b2),
                    })
                });
                CoreKind::Handle(Rc::new(CoreHandle {
                    body: Rc::new(body),
                    clauses: clauses.into(),
                    ret,
                    is_multi_declared: h.is_multi_declared,
                }))
            }
            CoreKind::Resume(v) => CoreKind::Resume(Rc::new(self.expr(v, scope))),
            CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
                effect: p.effect.clone(),
                op: p.op.clone(),
                op_ty: p.op_ty.clone(),
                args: self.many(&p.args, scope),
            })),
        };
        CoreExpr {
            span: e.span,
            ty: e.ty.clone(),
            kind,
        }
    }
}

// ------------------------------------------------- parametric ADTs (types) --

/// Parametric ADTs, monomorphized (2026-10-09). A GC row records which fields
/// are heap values, and `List(Int)` and `List(String)` disagree on that, so a
/// parametric type cannot have one set of constructor tags. Each ground
/// instantiation the module uses becomes a type of its own (`List$t0`, with
/// constructors `Nil$t0`, `Cons$t0`), every type that names it is rewritten,
/// and every constructor application and pattern on it is renamed. The
/// parametric originals are dropped: a use that is still not ground reaches
/// the back end's existing "parametric ADT" refusal. An instantiation is its
/// arguments up to effect rows ([`layout_key`]); a non-regular type, whose
/// instances never run out, is refused by name.
struct TypeMono<'a> {
    params_of: HashMap<String, &'a CoreType>,
    /// (type, [`layout_key`] of its ground args) -> instance index.
    index: HashMap<(String, String), usize>,
    instances: Vec<(String, Vec<Ty>)>,
    /// Set once the module's own uses are registered: the deepest argument a
    /// FIELD-derived instance may have. A regular type's fields only
    /// rearrange its arguments, so its instances stay within a few field
    /// depths of the uses; a non-regular one (`Nest(a)` holding a
    /// `Nest(Box(a))`) grows forever and crosses it.
    limit: Option<usize>,
    /// The limit was crossed: [`mono_types`] refuses the module by name.
    non_regular: bool,
}

/// The most instances one module may need before it is refused as
/// non-regular anyway (a backstop: depth catches every real case first).
const MAX_INSTANCES: usize = 4096;

/// What an instance's identity depends on: its arguments with every effect
/// row reduced to the calling convention it selects (review6's first
/// finding). Rows say nothing about layout, and a row variable's id, a
/// clone's `$poly` marker or a label's span differ between uses of ONE value
/// -- keyed on them, one instantiation became several types with different
/// constructor tags. The convention does matter (a binder's field type
/// decides how a closure taken out of it is called), and it is the same at
/// every use of one value: a clone's `$poly` stands for a user effect.
fn layout_key(args: &[Ty]) -> String {
    fn shape(t: &Ty) -> Ty {
        match t {
            Ty::Fn(ps, _, r) => {
                let mut labels = BTreeMap::new();
                if crate::cps::needs_cps(t) {
                    labels.insert(
                        "cps".to_string(),
                        EffectLabel {
                            args: Vec::new(),
                            span: elya::span::Span::EMPTY,
                        },
                    );
                }
                Ty::Fn(
                    ps.iter().map(shape).collect(),
                    EffectRow {
                        labels,
                        tail: RowTail::Closed,
                    },
                    Box::new(shape(r)),
                )
            }
            Ty::Con(n, a) => Ty::Con(n.clone(), a.iter().map(shape).collect()),
            Ty::Tuple(a) => Ty::Tuple(a.iter().map(shape).collect()),
            other => other.clone(),
        }
    }
    let shaped: Vec<Ty> = args.iter().map(shape).collect();
    format!("{shaped:?}")
}

/// The nesting depth of a type (a leaf is 1).
fn depth(t: &Ty) -> usize {
    match t {
        Ty::Fn(ps, row, r) => {
            let labels = row.labels.values().flat_map(|l| l.args.iter());
            1 + ps
                .iter()
                .chain(labels)
                .map(depth)
                .chain([depth(r)])
                .max()
                .unwrap_or(0)
        }
        Ty::Con(_, a) | Ty::Tuple(a) => 1 + a.iter().map(depth).max().unwrap_or(0),
        _ => 1,
    }
}

impl<'a> TypeMono<'a> {
    fn new(types: &'a [CoreType]) -> Self {
        TypeMono {
            params_of: types
                .iter()
                .filter(|t| !t.params.is_empty())
                .map(|t| (t.name.clone(), t))
                .collect(),
            index: HashMap::new(),
            instances: Vec::new(),
            limit: None,
            non_regular: false,
        }
    }

    /// The instance for a ground `name(args)`, registered on first sight.
    fn instance(&mut self, name: &str, args: &[Ty]) -> Option<usize> {
        if !self.params_of.contains_key(name) || !args.iter().all(is_ground) {
            return None;
        }
        let key = (name.to_string(), layout_key(args));
        if let Some(i) = self.index.get(&key) {
            return Some(*i);
        }
        if let Some(limit) = self.limit {
            let deepest = args.iter().map(depth).max().unwrap_or(0);
            if deepest > limit || self.instances.len() >= MAX_INSTANCES {
                self.non_regular = true;
                return None;
            }
        }
        let i = self.instances.len();
        self.index.insert(key, i);
        self.instances.push((name.to_string(), args.to_vec()));
        Some(i)
    }

    /// `t` with every ground parametric instance replaced by its own type.
    fn ty(&mut self, t: &Ty) -> Ty {
        match t {
            Ty::Con(n, args) => {
                let args2: Vec<Ty> = args.iter().map(|a| self.ty(a)).collect();
                match self.instance(n, args) {
                    Some(i) => Ty::Con(format!("{n}$t{i}"), Vec::new()),
                    None => Ty::Con(n.clone(), args2),
                }
            }
            Ty::Fn(ps, row, r) => Ty::Fn(
                ps.iter().map(|p| self.ty(p)).collect(),
                EffectRow {
                    labels: row
                        .labels
                        .iter()
                        .map(|(k, l)| {
                            (
                                k.clone(),
                                EffectLabel {
                                    args: l.args.iter().map(|a| self.ty(a)).collect(),
                                    span: l.span,
                                },
                            )
                        })
                        .collect(),
                    tail: row.tail.clone(),
                },
                Box::new(self.ty(r)),
            ),
            Ty::Tuple(args) => Ty::Tuple(args.iter().map(|a| self.ty(a)).collect()),
            other => other.clone(),
        }
    }

    /// The renamed constructor, when `ty` (pre-rewrite) is an instance.
    fn ctor(&mut self, name: &str, ty: &Ty) -> String {
        if let Ty::Con(n, args) = ty {
            if let Some(i) = self.instance(n, args) {
                return format!("{name}$t{i}");
            }
        }
        name.to_string()
    }

    fn param(&mut self, p: &CoreParam) -> CoreParam {
        CoreParam {
            name: p.name.clone(),
            ty: self.ty(&p.ty),
        }
    }

    fn many(&mut self, cs: &[CoreExpr]) -> Rc<[CoreExpr]> {
        cs.iter().map(|c| self.expr(c)).collect::<Vec<_>>().into()
    }

    fn expr(&mut self, e: &CoreExpr) -> CoreExpr {
        let kind = match &e.kind {
            CoreKind::Lit(l) => CoreKind::Lit(l.clone()),
            CoreKind::Var(n) => CoreKind::Var(n.clone()),
            CoreKind::App(f, a) => CoreKind::App(Rc::new(self.expr(f)), self.many(a)),
            CoreKind::Builtin(n, a) => CoreKind::Builtin(n.clone(), self.many(a)),
            CoreKind::Ctor(n, a) => {
                let n2 = self.ctor(n, &e.ty);
                CoreKind::Ctor(n2, self.many(a))
            }
            CoreKind::Prim(op, a) => CoreKind::Prim(*op, self.many(a)),
            CoreKind::Lambda(ps, b) => CoreKind::Lambda(
                ps.iter().map(|p| self.param(p)).collect::<Vec<_>>().into(),
                Rc::new(self.expr(b)),
            ),
            CoreKind::Let(x, v, b) => {
                CoreKind::Let(x.clone(), Rc::new(self.expr(v)), Rc::new(self.expr(b)))
            }
            CoreKind::If(c, t, f) => CoreKind::If(
                Rc::new(self.expr(c)),
                Rc::new(self.expr(t)),
                Rc::new(self.expr(f)),
            ),
            CoreKind::Match(sc, arms) => {
                let sty = sc.ty.clone();
                let sc2 = self.expr(sc);
                let arms2: Vec<CoreArm> = arms
                    .iter()
                    .map(|a| CoreArm {
                        pat: match &a.pat {
                            CorePat::Ctor(n, ps) => CorePat::Ctor(self.ctor(n, &sty), ps.clone()),
                            other => other.clone(),
                        },
                        body: self.expr(&a.body),
                    })
                    .collect();
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
                        params: c
                            .params
                            .iter()
                            .map(|p| self.param(p))
                            .collect::<Vec<_>>()
                            .into(),
                        body: Rc::new(self.expr(&c.body)),
                    })
                    .collect::<Vec<_>>()
                    .into(),
                ret: h.ret.as_ref().map(|r| {
                    Rc::new(CoreReturn {
                        binder: r.binder.clone(),
                        body: Rc::new(self.expr(&r.body)),
                    })
                }),
                is_multi_declared: h.is_multi_declared,
            })),
            CoreKind::Resume(v) => CoreKind::Resume(Rc::new(self.expr(v))),
            CoreKind::Perform(p) => CoreKind::Perform(Rc::new(CorePerform {
                effect: p.effect.clone(),
                op: p.op.clone(),
                op_ty: self.ty(&p.op_ty),
                args: self.many(&p.args),
            })),
        };
        CoreExpr {
            span: e.span,
            ty: self.ty(&e.ty),
            kind,
        }
    }

    /// The instance types, fields rewritten; instances their fields mention
    /// are registered (and built) too.
    fn instance_types(&mut self) -> Vec<CoreType> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.instances.len() && !self.non_regular {
            let (name, args) = self.instances[i].clone();
            let t = self.params_of[&name];
            let sub: HashMap<String, Ty> =
                t.params.iter().cloned().zip(args.iter().cloned()).collect();
            let ctors = t
                .ctors
                .iter()
                .map(|c| CoreCtor {
                    name: format!("{}$t{i}", c.name),
                    fields: c
                        .fields
                        .iter()
                        .map(|f| {
                            let g = subst_params(f, &sub);
                            self.ty(&g)
                        })
                        .collect(),
                })
                .collect();
            out.push(CoreType {
                name: format!("{name}$t{i}"),
                params: Vec::new(),
                ctors,
            });
            i += 1;
        }
        out
    }
}

/// A parametric type's field with its parameters (`Ty::Con(p, [])`) replaced.
fn subst_params(t: &Ty, sub: &HashMap<String, Ty>) -> Ty {
    match t {
        Ty::Con(n, args) if args.is_empty() && sub.contains_key(n) => sub[n].clone(),
        Ty::Con(n, args) => Ty::Con(
            n.clone(),
            args.iter().map(|a| subst_params(a, sub)).collect(),
        ),
        Ty::Fn(ps, row, r) => Ty::Fn(
            ps.iter().map(|p| subst_params(p, sub)).collect(),
            row.clone(),
            Box::new(subst_params(r, sub)),
        ),
        Ty::Tuple(args) => Ty::Tuple(args.iter().map(|a| subst_params(a, sub)).collect()),
        other => other.clone(),
    }
}

/// Monomorphize the module's parametric ADTs (see [`TypeMono`]).
fn mono_types(fns: Vec<CoreFn>, types: &[CoreType]) -> Result<CoreModule, CodegenError> {
    let mut m = TypeMono::new(types);
    let fns: Vec<CoreFn> = fns
        .iter()
        .map(|f| CoreFn {
            name: f.name.clone(),
            params: f
                .params
                .iter()
                .map(|p| m.param(p))
                .collect::<Vec<_>>()
                .into(),
            body: m.expr(&f.body),
        })
        .collect();
    // A plain type's fields name instances too (`type Q { Q(List(Int)) }`).
    let mut out_types: Vec<CoreType> = types
        .iter()
        .filter(|t| t.params.is_empty())
        .map(|t| CoreType {
            name: t.name.clone(),
            params: Vec::new(),
            ctors: t
                .ctors
                .iter()
                .map(|c| CoreCtor {
                    name: c.name.clone(),
                    fields: c.fields.iter().map(|f| m.ty(f)).collect(),
                })
                .collect(),
        })
        .collect();
    let used = m
        .instances
        .iter()
        .flat_map(|(_, args)| args.iter().map(depth))
        .max()
        .unwrap_or(0);
    let field = m
        .params_of
        .values()
        .flat_map(|t| t.ctors.iter().flat_map(|c| c.fields.iter().map(depth)))
        .max()
        .unwrap_or(0);
    m.limit = Some(used + (m.params_of.len() + 1) * (field + 1));
    out_types.extend(m.instance_types());
    if m.non_regular {
        return Err(CodegenError::Unsupported("non-regular parametric type"));
    }
    Ok(CoreModule {
        fns,
        types: out_types,
    })
}
