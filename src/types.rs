//! Hindley–Milner type inference (Algorithm J).

use crate::ast::{
    BinOp, Block, Decl, Expr, FnDecl, Handler, Module, PatLit, Pattern, Stmt, TypeAnn, UnOp,
};
use crate::diag::Diagnostic;
use crate::span::{Span, Spanned};
use crate::Session;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TyCon {
    Int,
    Float,
    Bool,
    Str,
    Unit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Var(u32),
    Base(TyCon),
    /// A function type carries its latent effect row between the params and the
    /// result: `fn(A) / {E} -> B`. A pure function's row is `EffectRow::pure()`.
    Fn(Vec<Ty>, EffectRow, Box<Ty>),
    Tuple(Vec<Ty>),
    /// An applied, nominal type constructor: `List(Int)` = `Con("List",[Int])`.
    Con(String, Vec<Ty>),
    /// Poison value that unifies with anything; suppresses cascade errors.
    Error,
}

/// Row variables index a second union-find (`row_subst`), distinct from the
/// type-variable space that indexes `subst`.
pub type RowVar = u32;

/// Effects whose duplication under a multi-shot handler is *observable*, and so
/// worth an `E0426` cleanup lint. A membership set, not a hardcoded label, so
/// new observable effects (`Net`, device I/O) join it instead of escaping the
/// lint (spec §11 tracked obligation).
const OBSERVABLE_EFFECTS: &[&str] = &["IO"];

#[derive(Clone, Debug, PartialEq)]
pub enum RowTail {
    /// Exactly the labels present — no more.
    Closed,
    /// The labels present *plus* whatever this row variable resolves to.
    Open(RowVar),
    /// Poison tail that absorbs any label; suppresses cascade row errors.
    ErrorRow,
}

/// One effect in a row: its type arguments (empty for a monomorphic effect —
/// the arity-0 case that reproduces Slice-3 behavior) and the span that
/// introduced it, for provenance.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectLabel {
    pub args: Vec<Ty>,
    pub span: Span,
}

/// An effect row: a set of effect labels (each with its type arguments and the
/// span that introduced it) and a tail. Labels are kept sorted (`BTreeMap`) so
/// the printed order is stable and each label appears at most once (idempotent);
/// a label present at two type argumentations is reconciled by `unify_row`.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectRow {
    pub labels: BTreeMap<String, EffectLabel>,
    pub tail: RowTail,
}

impl EffectRow {
    /// The empty, closed row — a pure computation performs nothing.
    pub fn pure() -> EffectRow {
        EffectRow {
            labels: BTreeMap::new(),
            tail: RowTail::Closed,
        }
    }
    /// An empty row open at `tail` — "these (none yet) plus whatever `tail` is".
    pub fn open(tail: RowVar) -> EffectRow {
        EffectRow {
            labels: BTreeMap::new(),
            tail: RowTail::Open(tail),
        }
    }
    /// Whether this is the empty, closed (pure) row.
    pub fn is_pure(&self) -> bool {
        self.labels.is_empty() && self.tail == RowTail::Closed
    }
}

/// The result of a failed row reconciliation. `unify_row` is a diagnostic-free
/// primitive: it *returns* the labels that could not be absorbed (because the
/// other side's tail was closed), and the caller — an inference site or the
/// discharge pass — decides whether that is `E0420`/`E0421`/`E0423` and attaches
/// provenance. (The row occurs-check `E0424` is the one exception, pushed inside
/// the primitive, mirroring how the type occurs-check `E0401` is pushed in `bind`.)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowConflict {
    /// Labels present in the first row that the second row (closed) cannot absorb.
    pub only1: Vec<(String, Span)>,
    /// Labels present in the second row that the first row (closed) cannot absorb.
    pub only2: Vec<(String, Span)>,
}

impl Ty {
    pub fn int() -> Ty {
        Ty::Base(TyCon::Int)
    }
    pub fn float() -> Ty {
        Ty::Base(TyCon::Float)
    }
    pub fn bool() -> Ty {
        Ty::Base(TyCon::Bool)
    }
    pub fn str() -> Ty {
        Ty::Base(TyCon::Str)
    }
    pub fn unit() -> Ty {
        Ty::Base(TyCon::Unit)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scheme {
    pub vars: Vec<u32>,
    /// Quantified effect-row variables (row polymorphism, spec §3.5).
    pub row_vars: Vec<RowVar>,
    pub ty: Ty,
}

/// The elaborated signature of an effect operation: which effect it belongs to,
/// its (monomorphic, base-typed) parameter types, and its result type.
#[derive(Clone)]
struct OpInfo {
    effect: String,
    /// The effect's type-parameter variables (Slice 4c-2): `params`/`ret` are
    /// expressed over these. Empty for a monomorphic effect (the arity-0 case).
    /// A perform instantiates them fresh; a handle fixes them for its scope.
    effect_params: Vec<u32>,
    params: Vec<Ty>,
    ret: Ty,
}

pub struct Infer {
    subst: Vec<Option<Ty>>,
    row_subst: Vec<Option<EffectRow>>,
    /// Sub-effecting (2026-10-08): the row variables that only say "this
    /// function VALUE may be used where more effects are allowed" -- the fresh
    /// tails `open_covariant` and lambda literals add. A call closes such a
    /// tail (the callee performs at most its labels); unified with a
    /// meaningful row variable, a tail stops being phantom; one still unbound
    /// when its SCC group is done is closed.
    phantom: HashSet<RowVar>,
    /// Sub-effecting: `(callee tail, ambient)` -- a call whose callee row ends
    /// in a phantom tail performs at most what that tail becomes, so the
    /// ambient must INCLUDE it. Recorded instead of unified (unifying made the
    /// callee's row equal to the caller's), and flushed -- labels only -- before
    /// a lambda's ambient closes and when the group is done.
    pending_incl: Vec<(RowVar, RowVar, Span)>,
    /// The parameter types in scope (function, lambda and clause parameters):
    /// a callee tail shared with one is a RELAY and is unified, never treated
    /// as an upcast -- generalization needs the shared variable.
    param_tys: Vec<Ty>,
    /// Operation name -> its elaborated signature. Populated from `effect`
    /// declarations before inference; a call to one of these is a *perform*.
    ops: HashMap<String, OpInfo>,
    /// Stack of `(B, R, ε)` for the handler clause currently being typed:
    /// `resume` takes the operation's result type `B`, yields the handle's
    /// result `R`, and performs `ε` -- the handled body's effects minus the
    /// handled one, which the resumed computation may still perform.
    resume_stack: Vec<(Ty, Ty, EffectRow, Option<String>)>,
    /// Constructor name -> arity. An n-ary constructor used unapplied or
    /// partially applied is `E0433` (unapplied constructors need 4b's closures).
    ctor_arity: HashMap<String, usize>,
    /// Effect name -> whether it is declared `multi` (Slice 4d-1). Read by the
    /// `with multi` conformance rule; a declaration fact, NOT a row attribute.
    effect_multi: HashMap<String, bool>,
    /// Names of `linear`-declared types (Slice 4d-2) — their values are affine.
    linear_types: HashSet<String>,
    /// Spans of bindings whose inferred type is linear (affine binding sites),
    /// exposed to the `affine::check` pass. The one bounded reach for 4d-2.
    affine_sites: HashSet<Span>,
    /// Every expression node's inferred type, zonked at the end of inference
    /// (Slice 5a-1). The Core IR arc's feeder; recorded unconditionally, exposed
    /// only through `infer_with_types`. Keyed by span (Shape A; the span-audit
    /// gate proves the key is unique).
    node_types: HashMap<Span, Ty>,
    /// Declared type names -> arity, for elaborating annotations (2026-10-09).
    known_types: HashMap<String, usize>,
    /// The current top-level function's annotation type variables: a lowercase
    /// name in any annotation of one function (its signature, its `let`s and
    /// lambdas) is one shared unification variable.
    ann_vars: HashMap<String, Ty>,
    pub diags: Vec<Diagnostic>,
}

impl Infer {
    pub fn new() -> Infer {
        Infer {
            subst: Vec::new(),
            row_subst: Vec::new(),
            ops: HashMap::new(),
            resume_stack: Vec::new(),
            phantom: HashSet::new(),
            pending_incl: Vec::new(),
            param_tys: Vec::new(),
            ctor_arity: HashMap::new(),
            effect_multi: HashMap::new(),
            linear_types: HashSet::new(),
            affine_sites: HashSet::new(),
            node_types: HashMap::new(),
            known_types: HashMap::new(),
            ann_vars: HashMap::new(),
            diags: Vec::new(),
        }
    }

    /// A type annotation as a type (2026-10-09; annotations used to be parsed
    /// and discarded). Base and declared names as in an ADT field; a lowercase
    /// name is the function's shared variable of that name -- FLEXIBLE: it
    /// says "the same type here and there", not "for every type" (no rigid
    /// skolems); `fn(A) / {E} -> R` is a function type whose row, when none is
    /// written, is open (any effects), and when written is exactly those.
    fn elaborate_ann(&mut self, ann: &Spanned<TypeAnn>, covariant: bool) -> Ty {
        let t = &ann.node;
        if t.name == "fn" && !t.args.is_empty() {
            let n = t.args.len();
            let params: Vec<Ty> = t.args[..n - 1]
                .iter()
                .map(|a| self.elaborate_ann(a, !covariant))
                .collect();
            let ret = self.elaborate_ann(&t.args[n - 1], covariant);
            let row = match &t.row {
                // An unwritten row says nothing about effects. Where the value
                // is PRODUCED (a covariant position: a result, a `let`) it is
                // an upcast tail, exactly what an unannotated lambda gets --
                // a meaningful variable there stopped sub-effecting (the
                // review's F2). Where it is CONSUMED (a parameter) it is a
                // real variable the body's calls relay.
                None if covariant => EffectRow::open(self.fresh_phantom_row()),
                None => EffectRow::open(self.fresh_row()),
                Some(labels) => {
                    let mut m = BTreeMap::new();
                    for l in labels {
                        if !self.effect_multi.contains_key(&l.node) && l.node != "IO" {
                            self.diags.push(
                                Diagnostic::error("E0432", format!("unknown effect `{}`", l.node))
                                    .with_label(l.span, "no such effect"),
                            );
                            continue;
                        }
                        let arity = self
                            .ops
                            .values()
                            .find(|o| o.effect == l.node)
                            .map(|o| o.effect_params.len())
                            .unwrap_or(0);
                        let args = (0..arity).map(|_| self.fresh()).collect();
                        m.insert(l.node.clone(), EffectLabel { args, span: l.span });
                    }
                    EffectRow {
                        labels: m,
                        tail: RowTail::Closed,
                    }
                }
            };
            return Ty::Fn(params, row, Box::new(ret));
        }
        if t.args.is_empty() && t.name.starts_with(|c: char| c.is_ascii_lowercase()) {
            if let Some(v) = self.ann_vars.get(&t.name) {
                return v.clone();
            }
            let v = self.fresh();
            self.ann_vars.insert(t.name.clone(), v.clone());
            return v;
        }
        if matches!(
            t.name.as_str(),
            "Int" | "Float" | "Bool" | "String" | "Unit"
        ) && !t.args.is_empty()
        {
            self.diags.push(
                Diagnostic::error("E0400", format!("`{}` takes no type arguments", t.name))
                    .with_label(ann.span, "remove the arguments"),
            );
            return Ty::Error;
        }
        match t.name.as_str() {
            "Int" => Ty::int(),
            "Float" => Ty::float(),
            "Bool" => Ty::bool(),
            "String" => Ty::str(),
            "Unit" => Ty::unit(),
            other => match self.known_types.get(other).copied() {
                Some(arity) => {
                    if t.args.len() != arity {
                        self.diags.push(
                            Diagnostic::error(
                                "E0400",
                                format!(
                                    "type `{other}` expects {arity} argument(s), found {}",
                                    t.args.len()
                                ),
                            )
                            .with_label(ann.span, "wrong number of type arguments"),
                        );
                        return Ty::Error;
                    }
                    // Type arguments are invariant: no upcast tails inside.
                    let args = t
                        .args
                        .iter()
                        .map(|a| self.elaborate_ann(a, false))
                        .collect();
                    Ty::Con(other.to_string(), args)
                }
                None => {
                    self.diags.push(
                        Diagnostic::error("E0432", format!("unknown type `{other}`"))
                            .with_label(ann.span, "no such type"),
                    );
                    Ty::Error
                }
            },
        }
    }

    /// Check an annotation against the type inference gave the annotated
    /// thing; a mismatch is reported at the annotation.
    fn check_ann(&mut self, ann: &Option<Spanned<TypeAnn>>, ty: &Ty, covariant: bool) {
        if let Some(a) = ann {
            let want = self.elaborate_ann(a, covariant);
            self.unify(&want, ty, a.span);
        }
    }

    /// Whether `t`'s head is a `linear`-declared type (Slice 4d-2): its values
    /// are affine. Resolves first so a bound variable is seen through.
    fn ty_is_linear(&self, t: &Ty) -> bool {
        matches!(self.resolve(t), Ty::Con(n, _) if self.linear_types.contains(&n))
    }

    pub fn fresh(&mut self) -> Ty {
        let id = self.subst.len() as u32;
        self.subst.push(None);
        Ty::Var(id)
    }

    /// A fresh, unbound row variable.
    pub fn fresh_row(&mut self) -> RowVar {
        let id = self.row_subst.len() as u32;
        self.row_subst.push(None);
        id
    }

    /// Deep-zonk a row: follow the tail through `row_subst`, accumulating the
    /// labels found along the chain (union; earliest provenance span wins). The
    /// result's tail is the first `Closed`/`ErrorRow`/unbound-`Open` reached.
    pub fn resolve_row(&self, r: &EffectRow) -> EffectRow {
        let mut labels = r.labels.clone();
        let mut tail = r.tail.clone();
        loop {
            // Borrow `tail` (don't move it) to find the next link, if any.
            let next = match &tail {
                RowTail::Open(v) => match &self.row_subst[*v as usize] {
                    Some(bound) => {
                        for (k, l) in &bound.labels {
                            labels.entry(k.clone()).or_insert_with(|| l.clone());
                        }
                        Some(bound.tail.clone())
                    }
                    None => None, // unbound: this open tail is the residual
                },
                RowTail::Closed | RowTail::ErrorRow => None,
            };
            match next {
                Some(t) => tail = t,
                None => break,
            }
        }
        // Resolve each label's type arguments (Slice 4c-2), so downstream
        // consumers (printing, arg reconciliation, generalization) see the
        // substituted types rather than raw variables.
        for l in labels.values_mut() {
            for a in l.args.iter_mut() {
                *a = self.resolve(a);
            }
        }
        EffectRow { labels, tail }
    }

    /// Follow bound variables to a representative, recursively (deep).
    pub fn resolve(&self, t: &Ty) -> Ty {
        match t {
            Ty::Var(v) => match &self.subst[*v as usize] {
                Some(bound) => self.resolve(bound),
                None => Ty::Var(*v),
            },
            Ty::Base(c) => Ty::Base(*c),
            Ty::Fn(ps, row, r) => Ty::Fn(
                ps.iter().map(|p| self.resolve(p)).collect(),
                self.resolve_row(row),
                Box::new(self.resolve(r)),
            ),
            Ty::Tuple(xs) => Ty::Tuple(xs.iter().map(|x| self.resolve(x)).collect()),
            Ty::Con(n, args) => Ty::Con(n.clone(), args.iter().map(|a| self.resolve(a)).collect()),
            Ty::Error => Ty::Error,
        }
    }

    fn occurs(&self, v: u32, t: &Ty) -> bool {
        // Type occurs-check only concerns the type-variable space; row variables
        // live in a separate union-find with its own occurs-check (Task 3).
        match self.resolve(t) {
            Ty::Var(u) => u == v,
            Ty::Base(_) | Ty::Error => false,
            Ty::Fn(ps, _row, r) => ps.iter().any(|p| self.occurs(v, p)) || self.occurs(v, &r),
            Ty::Tuple(xs) => xs.iter().any(|x| self.occurs(v, x)),
            Ty::Con(_, args) => args.iter().any(|a| self.occurs(v, a)),
        }
    }

    fn bind(&mut self, v: u32, t: &Ty, span: Span) {
        if let Ty::Var(u) = t {
            if *u == v {
                return;
            }
        }
        if self.occurs(v, t) {
            self.diags.push(
                Diagnostic::error("E0401", "infinite type")
                    .with_label(span, "a type would have to contain itself"),
            );
            self.subst[v as usize] = Some(Ty::Error);
            return;
        }
        self.subst[v as usize] = Some(t.clone());
    }

    pub fn unify(&mut self, a: &Ty, b: &Ty, span: Span) {
        let a = self.resolve(a);
        let b = self.resolve(b);
        match (a, b) {
            (Ty::Error, _) | (_, Ty::Error) => {}
            (Ty::Var(x), Ty::Var(y)) if x == y => {}
            (Ty::Var(x), t) | (t, Ty::Var(x)) => self.bind(x, &t, span),
            (Ty::Base(x), Ty::Base(y)) if x == y => {}
            (Ty::Fn(p1, row1, r1), Ty::Fn(p2, row2, r2)) => {
                if p1.len() != p2.len() {
                    self.diags.push(
                        Diagnostic::error("E0402", "wrong number of arguments").with_label(
                            span,
                            format!("expected {} argument(s), found {}", p1.len(), p2.len()),
                        ),
                    );
                } else {
                    for (x, y) in p1.iter().zip(&p2) {
                        self.unify(x, y, span);
                    }
                    self.unify(&r1, &r2, span);
                    // The two arrows' effect rows must reconcile. A closing
                    // conflict (both rows closed but differing) is E0423.
                    if let Err(c) = self.unify_row(&row1, &row2, span) {
                        let differ: Vec<(String, Span)> =
                            c.only1.iter().chain(c.only2.iter()).cloned().collect();
                        self.emit_row_mismatch(
                            &differ,
                            span,
                            "these function values perform different effects",
                        );
                    }
                }
            }
            (Ty::Tuple(x), Ty::Tuple(y)) if x.len() == y.len() => {
                for (p, q) in x.iter().zip(&y) {
                    self.unify(p, q, span);
                }
            }
            // Nominal: same constructor name + arity, then unify args pairwise.
            (Ty::Con(n1, a1), Ty::Con(n2, a2)) if n1 == n2 && a1.len() == a2.len() => {
                for (p, q) in a1.iter().zip(&a2) {
                    self.unify(p, q, span);
                }
            }
            (x, y) => {
                let msg = format!(
                    "expected `{}`, found `{}`",
                    display_ty(self, &x),
                    display_ty(self, &y)
                );
                self.diags
                    .push(Diagnostic::error("E0400", "type mismatch").with_label(span, msg));
            }
        }
    }

    // ---- Effect-row unification (second union-find) ----

    /// Rewriting unification of two simple effect rows (Rémy / Leijen; Koka).
    /// Diagnostic-free except the row occurs-check (`E0424`): label-absorption
    /// failures are *returned* as a `RowConflict` for the caller to classify.
    pub fn unify_row(
        &mut self,
        r1: &EffectRow,
        r2: &EffectRow,
        span: Span,
    ) -> Result<(), RowConflict> {
        let r1 = self.resolve_row(r1);
        let r2 = self.resolve_row(r2);
        // A poison tail on either side absorbs everything — no cascade.
        if r1.tail == RowTail::ErrorRow || r2.tail == RowTail::ErrorRow {
            return Ok(());
        }
        // A label present on BOTH sides must agree on its type arguments (Slice
        // 4c-2, design B). Monomorphic effects have empty args, so this is a no-op
        // — exactly the Slice-3 behavior. A conflicting instantiation is `E0423`.
        let shared: Vec<(String, Vec<Ty>, Vec<Ty>)> = r1
            .labels
            .iter()
            .filter_map(|(k, l1)| {
                r2.labels
                    .get(k)
                    .map(|l2| (k.clone(), l1.args.clone(), l2.args.clone()))
            })
            .collect();
        for (effect, a1, a2) in shared {
            self.unify_effect_args(&effect, &a1, &a2, span);
        }
        // Split out each side's labels absent from the other (their args ride
        // along, so growing an open tail preserves them).
        let only1: Vec<(String, EffectLabel)> = r1
            .labels
            .iter()
            .filter(|(k, _)| !r2.labels.contains_key(*k))
            .map(|(k, l)| (k.clone(), l.clone()))
            .collect();
        let only2: Vec<(String, EffectLabel)> = r2
            .labels
            .iter()
            .filter(|(k, _)| !r1.labels.contains_key(*k))
            .map(|(k, l)| (k.clone(), l.clone()))
            .collect();

        let mut conflict = RowConflict::default();
        // r2's tail must absorb only1; r1's tail must absorb only2. Each returns
        // the residual tail left after rewriting.
        let t2 = self.absorb(&only1, &r2.tail, span, &mut conflict.only1);
        let t1 = self.absorb(&only2, &r1.tail, span, &mut conflict.only2);
        if !conflict.only1.is_empty() || !conflict.only2.is_empty() {
            return Err(conflict);
        }
        // The two residual tails must be equal.
        self.unify_tails(&t1, &t2, span);
        Ok(())
    }

    /// Rewrite `tail` so it contains `extra`, returning the residual tail.
    /// `Open(v)` grows by binding `v := {extra} | Open(fresh)`; a `Closed` tail
    /// cannot absorb non-empty `extra`, so those labels go into `bucket`.
    fn absorb(
        &mut self,
        extra: &[(String, EffectLabel)],
        tail: &RowTail,
        span: Span,
        bucket: &mut Vec<(String, Span)>,
    ) -> RowTail {
        if extra.is_empty() {
            return tail.clone();
        }
        match tail {
            RowTail::Open(v) => {
                let fresh = self.fresh_row();
                // What remains of a phantom tail is still phantom.
                if self.phantom.contains(v) {
                    self.phantom.insert(fresh);
                }
                let labels: BTreeMap<String, EffectLabel> = extra.iter().cloned().collect();
                self.bind_row(
                    *v,
                    &EffectRow {
                        labels,
                        tail: RowTail::Open(fresh),
                    },
                    span,
                );
                RowTail::Open(fresh)
            }
            RowTail::Closed => {
                bucket.extend(extra.iter().map(|(k, l)| (k.clone(), l.span)));
                RowTail::Closed
            }
            RowTail::ErrorRow => RowTail::ErrorRow,
        }
    }

    /// Unify two residual row tails (no labels remain at this point).
    fn unify_tails(&mut self, t1: &RowTail, t2: &RowTail, span: Span) {
        match (t1, t2) {
            (RowTail::ErrorRow, _) | (_, RowTail::ErrorRow) => {}
            (RowTail::Open(a), RowTail::Open(b)) => {
                if a != b {
                    self.bind_row(*a, &EffectRow::open(*b), span);
                }
            }
            (RowTail::Open(a), RowTail::Closed) | (RowTail::Closed, RowTail::Open(a)) => {
                self.bind_row(*a, &EffectRow::pure(), span);
            }
            (RowTail::Closed, RowTail::Closed) => {}
        }
    }

    /// Bind row variable `v := r`, with an occurs-check (`E0424`): `v` may not
    /// appear in `r`'s tail, or the row would contain itself.
    fn bind_row(&mut self, v: RowVar, r: &EffectRow, span: Span) {
        let resolved = self.resolve_row(r);
        if resolved.tail == RowTail::Open(v) {
            self.diags.push(
                Diagnostic::error("E0424", "cyclic effect row")
                    .with_label(span, "an effect row would contain itself"),
            );
            self.row_subst[v as usize] = Some(EffectRow {
                labels: BTreeMap::new(),
                tail: RowTail::ErrorRow,
            });
            return;
        }
        // A meaningful variable bound to `{.. | w}` hands its meaning to `w`.
        if !self.phantom.contains(&v) {
            if let RowTail::Open(w) = resolved.tail {
                self.phantom.remove(&w);
            }
        }
        self.row_subst[v as usize] = Some(resolved);
    }

    /// Add each pending callee tail's CURRENT labels to its ambient, to a
    /// fixpoint (a label can travel through several inclusions). A label that
    /// meets an ambient already closed is a surfaced conflict, never dropped.
    fn flush_inclusions(&mut self) {
        loop {
            let mut changed = false;
            for (src, dst, span) in self.pending_incl.clone() {
                let have = self.resolve_row(&EffectRow::open(dst));
                let want = self.resolve_row(&EffectRow::open(src));
                // A recorded tail that has since become MEANINGFUL (it aliased
                // a real row variable, e.g. a parameter's) is a relay after
                // all: the ambient must take whatever that row takes later,
                // which labels alone cannot say (the sub-effecting review's
                // F1: `f` was typed pure though it calls its parameter `k`).
                if let RowTail::Open(w) = want.tail {
                    if !self.phantom.contains(&w) && have.tail != RowTail::Open(w) {
                        let r = self.unify_row(&EffectRow::open(dst), &EffectRow::open(w), span);
                        if r.is_ok() {
                            changed = true;
                        }
                        self.surface_conflict(r);
                    }
                }
                for (label, l) in want.labels.iter() {
                    if have.labels.contains_key(label) {
                        continue;
                    }
                    let r = self.add_effect(dst, label, l.args.clone(), span);
                    // Only a label that got in can carry further; a conflict
                    // is reported once and the pair dropped (no retry loop).
                    if r.is_ok() {
                        changed = true;
                    } else {
                        self.pending_incl.retain(|p| *p != (src, dst, span));
                    }
                    self.surface_conflict(r);
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn fresh_phantom_row(&mut self) -> RowVar {
        let v = self.fresh_row();
        self.phantom.insert(v);
        v
    }

    /// Force `op ∈ amb`: unify the ambient with `{op@span} | Open(fresh)`, which
    /// rewrites `amb`'s tail to expose `op`. Never fails when `amb` is open.
    pub fn add_effect(
        &mut self,
        amb: RowVar,
        op: &str,
        args: Vec<Ty>,
        span: Span,
    ) -> Result<(), RowConflict> {
        let fresh = self.fresh_row();
        let mut labels = BTreeMap::new();
        labels.insert(op.to_string(), EffectLabel { args, span });
        let target = EffectRow {
            labels,
            tail: RowTail::Open(fresh),
        };
        self.unify_row(&EffectRow::open(amb), &target, span)
    }

    /// Instantiate an operation's effect type parameters with fresh type
    /// variables (Slice 4c-2): returns the effect's fresh type arguments (for the
    /// row), the substituted operation parameters, and the substituted result. A
    /// monomorphic op returns empty args and its params/ret unchanged.
    fn instantiate_op(&mut self, op: &OpInfo) -> (Vec<Ty>, Vec<Ty>, Ty) {
        if op.effect_params.is_empty() {
            return (Vec::new(), op.params.clone(), op.ret.clone());
        }
        let mut m: HashMap<u32, Ty> = HashMap::new();
        let mut args = Vec::with_capacity(op.effect_params.len());
        for &p in &op.effect_params {
            let fresh = self.fresh();
            m.insert(p, fresh.clone());
            args.push(fresh);
        }
        let rm = HashMap::new();
        let params = op.params.iter().map(|t| subst_vars(t, &m, &rm)).collect();
        let ret = subst_vars(&op.ret, &m, &rm);
        (args, params, ret)
    }

    /// Reconcile the type arguments of one effect present in two rows (design B).
    /// A pair of concrete, unequal arguments is an effect used at conflicting
    /// types — reported as `E0423` (the row-mismatch code) and poisoned so no
    /// cascading `E0400` is also emitted.
    fn unify_effect_args(&mut self, effect: &str, a1: &[Ty], a2: &[Ty], span: Span) {
        for (x, y) in a1.iter().zip(a2) {
            let rx = self.resolve(x);
            let ry = self.resolve(y);
            let both_concrete =
                !matches!(rx, Ty::Var(_) | Ty::Error) && !matches!(ry, Ty::Var(_) | Ty::Error);
            if both_concrete && rx != ry {
                let (dx, dy) = (display_ty(self, &rx), display_ty(self, &ry));
                self.diags.push(
                    Diagnostic::error("E0423", "effect row mismatch")
                        .with_label(span, "the same effect is used at different types here")
                        .with_help(format!(
                            "effect `{effect}` is used at conflicting type arguments: `{dx}` vs `{dy}`"
                        )),
                );
                return;
            }
            self.unify(x, y, span);
        }
    }

    /// Pour a callee's latent row into the ambient: `amb ⊇ eff`. Adds each of
    /// `eff`'s labels (keeping `amb` open, so a *closed* callee row never forces
    /// the ambient closed), and relays a polymorphic tail into the ambient.
    pub fn add_row(&mut self, amb: RowVar, eff: &EffectRow, span: Span) -> Result<(), RowConflict> {
        let eff = self.resolve_row(eff);
        for (label, l) in &eff.labels {
            self.add_effect(amb, label, l.args.clone(), l.span)?;
        }
        if let RowTail::Open(rho) = eff.tail {
            // The callee's polymorphic effects flow into the ambient.
            self.unify_row(&EffectRow::open(amb), &EffectRow::open(rho), span)?;
        }
        Ok(())
    }

    /// Sub-effecting (2026-10-08): open every CLOSED row in a covariant
    /// position of `t` -- a function's own row, and rows in its result -- to a
    /// fresh tail. Parameter rows are contravariant and stay as they are
    /// (opening one would let an effectful argument into a pure parameter).
    fn open_covariant(&mut self, t: &Ty) -> Ty {
        match self.resolve(t) {
            Ty::Fn(ps, row, r) => {
                let mut row = self.resolve_row(&row);
                if row.tail == RowTail::Closed {
                    row.tail = RowTail::Open(self.fresh_phantom_row());
                }
                let r = self.open_covariant(&r);
                Ty::Fn(ps, row, Box::new(r))
            }
            other => other,
        }
    }

    /// A row conflict on the way INTO an ambient -- a perform, a call or a
    /// handle's residual meeting a row that was already closed -- used to be
    /// discarded, so the effect vanished from the type (found by the
    /// resume-row review). Report it instead.
    fn surface_conflict(&mut self, r: Result<(), RowConflict>) {
        if let Err(c) = r {
            let mut differ = c.only1.clone();
            differ.extend(c.only2.iter().cloned());
            let span = differ.first().map(|(_, s)| *s).unwrap_or(Span::EMPTY);
            self.emit_row_mismatch(
                &differ,
                span,
                "this effect reaches a row that was already closed",
            );
        }
    }

    /// Emit `E0423` naming the exact set of labels two rows differ by. Rows are
    /// rendered as named labels (never a `%r` token) — the effect-diagnostic
    /// discipline. Shared by the unifier and the exact-match pass (Task 6).
    fn emit_row_mismatch(&mut self, differ: &[(String, Span)], span: Span, note: &str) {
        let mut labels: Vec<String> = differ.iter().map(|(k, _)| k.clone()).collect();
        labels.sort();
        labels.dedup();
        let set = format!("{{{}}}", labels.join(", "));
        self.diags.push(
            Diagnostic::error("E0423", "effect row mismatch")
                .with_label(span, note.to_string())
                .with_help(format!("the rows differ by exactly: {set}")),
        );
    }
}

impl Default for Infer {
    fn default() -> Self {
        Infer::new()
    }
}

/// Letter names for free variables during printing. Type variables and row
/// variables occupy *separate* id-spaces, so they get separate maps — but draw
/// from a *shared* counter, so every distinct variable (of either kind) gets a
/// distinct letter in first-seen order (`a`, `b`, `c`, …). Never a `%`/`t<n>`.
#[derive(Default)]
struct Names {
    ty: HashMap<u32, String>,
    row: HashMap<u32, String>,
}

impl Names {
    fn ty_name(&mut self, v: u32) -> String {
        if let Some(n) = self.ty.get(&v) {
            return n.clone();
        }
        let n = letter(self.ty.len() + self.row.len());
        self.ty.insert(v, n.clone());
        n
    }
    fn row_name(&mut self, v: u32) -> String {
        if let Some(n) = self.row.get(&v) {
            return n.clone();
        }
        let n = letter(self.ty.len() + self.row.len());
        self.row.insert(v, n.clone());
        n
    }
}

/// Render a type for humans: resolve (zonk) it, then name remaining free
/// variables `a, b, c, …` per call — never an internal `t<number>` token.
pub fn display_ty(inf: &Infer, t: &Ty) -> String {
    let mut names = Names::default();
    let resolved = inf.resolve(t);
    let mut out = String::new();
    write_ty(&resolved, &mut names, &mut out);
    out
}

/// Render a scheme as `forall a b. <ty>` (or just the type when unquantified).
pub fn display_scheme(inf: &Infer, s: &Scheme) -> String {
    let mut names = Names::default();
    // Seed quantified type variables first, then row variables, so each takes a
    // distinct leading letter in declaration order (`forall a b. …`).
    let mut quant: Vec<String> = s.vars.iter().map(|v| names.ty_name(*v)).collect();
    quant.extend(s.row_vars.iter().map(|v| names.row_name(*v)));
    let resolved = inf.resolve(&s.ty);
    let mut body = String::new();
    write_ty(&resolved, &mut names, &mut body);
    if quant.is_empty() {
        body
    } else {
        format!("forall {}. {}", quant.join(" "), body)
    }
}

fn letter(i: usize) -> String {
    let c = (b'a' + (i % 26) as u8) as char;
    if i < 26 {
        c.to_string()
    } else {
        format!("{c}{}", i / 26)
    }
}

fn write_ty(t: &Ty, names: &mut Names, out: &mut String) {
    match t {
        Ty::Var(v) => {
            out.push_str(&names.ty_name(*v));
        }
        Ty::Base(TyCon::Int) => out.push_str("Int"),
        Ty::Base(TyCon::Float) => out.push_str("Float"),
        Ty::Base(TyCon::Bool) => out.push_str("Bool"),
        Ty::Base(TyCon::Str) => out.push_str("String"),
        Ty::Base(TyCon::Unit) => out.push_str("Unit"),
        Ty::Fn(ps, row, r) => {
            out.push_str("fn(");
            for (i, p) in ps.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_ty(p, names, out);
            }
            out.push(')');
            // A pure row is invisible: `fn(A) -> B`. A non-pure row prints as
            // ` / {E, …}` (with a ` | e` residual tail when polymorphic).
            if !row.is_pure() {
                out.push_str(" / ");
                write_row(row, names, out);
            }
            out.push_str(" -> ");
            write_ty(r, names, out);
        }
        Ty::Tuple(xs) => {
            out.push('(');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_ty(x, names, out);
            }
            out.push(')');
        }
        Ty::Con(n, args) => {
            out.push_str(n);
            if !args.is_empty() {
                out.push('(');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write_ty(a, names, out);
                }
                out.push(')');
            }
        }
        Ty::Error => out.push_str("<error>"),
    }
}

/// Render an effect row as `{A, B}` (labels sorted, from the `BTreeMap`), with a
/// ` | e` residual when the tail is a genuinely-polymorphic row variable. Assumes
/// the row is already zonked (callers resolve the enclosing type first). Never
/// emits a raw row-variable token — the effect-diagnostic no-`%r` discipline.
fn write_row(row: &EffectRow, names: &mut Names, out: &mut String) {
    out.push('{');
    for (i, (label, l)) in row.labels.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(label);
        // A parametric effect prints its arguments (`State(Int)`); a monomorphic
        // effect has empty args and prints bare (`Log`), unchanged from Slice 3.
        if !l.args.is_empty() {
            out.push('(');
            for (j, a) in l.args.iter().enumerate() {
                if j > 0 {
                    out.push_str(", ");
                }
                write_ty(a, names, out);
            }
            out.push(')');
        }
    }
    match &row.tail {
        RowTail::Open(v) => {
            let name = names.row_name(*v);
            if row.labels.is_empty() {
                out.push_str(&name);
            } else {
                out.push_str(" | ");
                out.push_str(&name);
            }
        }
        RowTail::ErrorRow => out.push_str(" | <error>"),
        RowTail::Closed => {}
    }
    out.push('}');
}

/// Public rendering facade over the private `Names` map and `write_ty` (spec §5, §7).
/// A single `TyPrinter` accumulates variable→letter assignments *across* `render`
/// calls, so two nodes that share a `Ty::Var(n)` render the same letter and two
/// distinct vars render distinct letters — the cross-node coherence tripwire. It
/// assumes the type is already zonked (Core carries zonked types inline), so unlike
/// `display_ty` it does not `resolve`; it never reaches a solver.
#[derive(Default)]
pub struct TyPrinter {
    names: Names,
}

impl TyPrinter {
    pub fn new() -> TyPrinter {
        TyPrinter::default()
    }

    pub fn render(&mut self, t: &Ty) -> String {
        let mut out = String::new();
        write_ty(t, &mut self.names, &mut out);
        out
    }
}

#[derive(Default)]
pub struct TyEnv {
    scopes: Vec<HashMap<String, Scheme>>,
}

impl TyEnv {
    pub fn new() -> TyEnv {
        TyEnv {
            scopes: vec![HashMap::new()],
        }
    }
    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }
    fn pop(&mut self) {
        self.scopes.pop();
    }
    pub fn insert(&mut self, name: &str, s: Scheme) {
        self.scopes.last_mut().unwrap().insert(name.to_string(), s);
    }
    pub fn lookup(&self, name: &str) -> Option<&Scheme> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }
}

fn binop_type(op: BinOp) -> (Ty, Ty, Ty) {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Div | Rem => (Ty::int(), Ty::int(), Ty::int()),
        AddF | SubF | MulF | DivF => (Ty::float(), Ty::float(), Ty::float()),
        Lt | Le | Gt | Ge => (Ty::int(), Ty::int(), Ty::bool()),
        Concat => (Ty::str(), Ty::str(), Ty::str()),
        And | Or => (Ty::bool(), Ty::bool(), Ty::bool()),
        // Eq/Ne handled specially in infer_expr (both operands share a fresh var).
        Eq | Ne => (Ty::Error, Ty::Error, Ty::bool()),
    }
}

impl Infer {
    pub fn instantiate(&mut self, s: &Scheme) -> Ty {
        if s.vars.is_empty() && s.row_vars.is_empty() {
            return s.ty.clone();
        }
        let mapping: HashMap<u32, Ty> = s.vars.iter().map(|v| (*v, self.fresh())).collect();
        let mut row_mapping: HashMap<RowVar, RowVar> = HashMap::new();
        for v in &s.row_vars {
            let fresh = if self.phantom.contains(v) {
                self.fresh_phantom_row()
            } else {
                self.fresh_row()
            };
            row_mapping.insert(*v, fresh);
        }
        subst_vars(&s.ty, &mapping, &row_mapping)
    }

    /// `E0433`: an n-ary constructor used unapplied or partially applied.
    /// A constructor applied to the wrong number of arguments (Slice 4b-1: a bare
    /// constructor is now a legal function value, so this fires only for genuine
    /// partial/over-application at a call site — `Cons(1)` where `Cons` needs 2).
    fn emit_unapplied_ctor(&mut self, name: &str, arity: usize, span: Span) {
        let plural = if arity == 1 { "" } else { "s" };
        self.diags.push(
            Diagnostic::error(
                "E0433",
                format!(
                    "constructor `{name}` takes {arity} argument{plural} but was applied to a different number"
                ),
            )
            .with_label(span, "wrong number of arguments for this constructor")
            .with_help(format!(
                "Elya constructors are not curried — apply all {arity} argument{plural}, or wrap it in a lambda, e.g. `fn(x) {{ {name}(x, …) }}`"
            )),
        );
    }

    /// Infer a block, threading the ambient effect row `amb` through every
    /// sub-expression — sequencing unions effects into the same ambient.
    pub fn infer_block(&mut self, b: &Block, env: &mut TyEnv, amb: RowVar) -> Ty {
        env.push();
        for st in b.stmts.iter() {
            match &st.node {
                Stmt::Let { name, ann, value } => {
                    let t = self.infer_expr(value, env, amb);
                    self.check_ann(ann, &t, true);
                    // Record an affine binding site (Slice 4d-2): a `let` whose
                    // value has a `linear`-declared type. Keyed by the value span,
                    // which the affine pass reads to recognise the binding.
                    if self.ty_is_linear(&t) {
                        self.affine_sites.insert(value.span);
                    }
                    // Value restriction: generalize only syntactic values (spec
                    // 4b-1 §2.4). A non-value binding keeps its monotype — sound in
                    // the presence of first-class functions/continuations.
                    let scheme = if is_syntactic_value(&value.node) {
                        self.generalize(&t, env)
                    } else {
                        Scheme {
                            vars: Vec::new(),
                            row_vars: Vec::new(),
                            ty: self.resolve(&t),
                        }
                    };
                    env.insert(name, scheme);
                }
                Stmt::Expr(e) => {
                    self.infer_expr(e, env, amb);
                }
            }
        }
        let result = match &b.tail {
            Some(tail) => self.infer_expr(tail, env, amb),
            None => Ty::unit(),
        };
        env.pop();
        result
    }

    /// Type an expression and record its (pre-zonk) type against its span. This
    /// thin wrapper is the single record point (Slice 5a-1): every path — the
    /// recursive calls inside `infer_expr_inner`, `infer_block`, `infer_call`,
    /// `infer_handle` — routes through here, so no node escapes recording. The
    /// final zonk pass runs once in `infer_all`, not at record-time.
    pub fn infer_expr(&mut self, e: &Spanned<Expr>, env: &mut TyEnv, amb: RowVar) -> Ty {
        let ty = self.infer_expr_inner(e, env, amb);
        self.node_types.insert(e.span, ty.clone());
        ty
    }

    fn infer_expr_inner(&mut self, e: &Spanned<Expr>, env: &mut TyEnv, amb: RowVar) -> Ty {
        let span = e.span;
        match &e.node {
            Expr::Int(_) => Ty::int(),
            Expr::Float(_) => Ty::float(),
            Expr::Str(_) => Ty::str(),
            Expr::Bool(_) => Ty::bool(),
            Expr::Unit => Ty::unit(),
            Expr::Var(name) => {
                // A bare constructor is a first-class function value (Slice 4b-1):
                // its scheme is already an arrow (`Some : ∀a. (a) -> Option(a)`), so
                // instantiate it like any other name. Partial application stays an
                // error, caught at the *call* site (E0433, `infer_call`).
                match env.lookup(name) {
                    Some(s) => {
                        let s = s.clone();
                        let t = self.instantiate(&s);
                        // Sub-effecting at a VALUE use (a call's callee is typed
                        // in `infer_call`, unopened): a closed row in a covariant
                        // position opens to a fresh tail.
                        self.open_covariant(&t)
                    }
                    None => Ty::Error, // unresolved names are E0200 from resolution
                }
            }
            Expr::Qualified { .. } => Ty::Error, // typed at the Call site (builtins)
            Expr::Unary { op, expr } => {
                let t = self.infer_expr(expr, env, amb);
                let (operand, result) = match op {
                    UnOp::Neg => (Ty::int(), Ty::int()),
                    UnOp::Not => (Ty::bool(), Ty::bool()),
                };
                self.unify(&t, &operand, span);
                result
            }
            Expr::Binary { op, lhs, rhs } => {
                let lt = self.infer_expr(lhs, env, amb);
                let rt = self.infer_expr(rhs, env, amb);
                if matches!(op, BinOp::Eq | BinOp::Ne) {
                    self.unify(&lt, &rt, span);
                    Ty::bool()
                } else {
                    let (l, r, res) = binop_type(*op);
                    self.unify(&lt, &l, span);
                    self.unify(&rt, &r, span);
                    res
                }
            }
            Expr::If {
                cond,
                then_block,
                else_block,
            } => {
                let ct = self.infer_expr(cond, env, amb);
                self.unify_cond(&ct, cond.span);
                let tt = self.infer_block(&then_block.node, env, amb);
                let et = self.infer_block(&else_block.node, env, amb);
                self.unify(&tt, &et, span);
                tt
            }
            Expr::Block(b) => self.infer_block(b, env, amb),
            Expr::Call { callee, args } => self.infer_call(callee, args, span, env, amb),
            Expr::Handle { body, handler } => self.infer_handle(body, handler, span, env, amb),
            Expr::Resume { arg } => {
                let arg_ty = self.infer_expr(arg, env, amb);
                match self.resume_stack.last().cloned() {
                    Some((b, r, eff, handled)) => {
                        // resume : (B) / ε -> R — takes the operation's result,
                        // yields the handle's result, and runs the rest of the
                        // handled body, which may still perform ε. Directly in
                        // the clause ε is already in the ambient (the handle
                        // discharges into it); inside a LAMBDA in the clause it
                        // is not, and leaving it out typed such a lambda pure
                        // (found by slice 5b-10: an escaped resume performed an
                        // effect nothing handled, on a program `check` passed).
                        //
                        // LABELS only, never the tail: `add_row` unifies tails,
                        // which made this site's ambient EQUAL to the body's tail
                        // and merged rows related only by inclusion (the review of
                        // the first version: a direct resume tied the enclosing
                        // function's row to a lambda that then closed it, and a
                        // later effect was dropped -- a new hole). The body's
                        // labels are re-read here, so ones it gained since the
                        // clauses began are included; an effect the body only
                        // RELAYS through an open row is not (PARKED).
                        self.unify(&arg_ty, &b, span);
                        let now = self.resolve_row(&eff);
                        for (label, l) in now.labels.iter() {
                            if handled.as_deref() == Some(label.as_str()) {
                                continue;
                            }
                            {
                                let r = self.add_effect(amb, label, l.args.clone(), l.span);
                                self.surface_conflict(r);
                            }
                        }
                        r
                    }
                    // resume outside a handler is E0210 at resolve time.
                    None => Ty::Error,
                }
            }
            Expr::Match { scrutinee, arms } => {
                let s = self.infer_expr(scrutinee, env, amb);
                let result = self.fresh();
                for arm in arms.iter() {
                    let mut bindings = Vec::new();
                    self.check_pattern(&arm.node.pat, &s, env, &mut bindings);
                    env.push();
                    for (name, ty) in bindings {
                        env.insert(
                            &name,
                            Scheme {
                                vars: Vec::new(),
                                row_vars: Vec::new(),
                                ty,
                            },
                        );
                    }
                    let body_ty = self.infer_expr(&arm.node.body, env, amb);
                    self.unify(&body_ty, &result, arm.node.body.span);
                    env.pop();
                }
                result
            }
            Expr::Lambda { params, body } => {
                // Each parameter gets a fresh monomorphic type variable.
                env.push();
                let mut param_tys = Vec::with_capacity(params.len());
                for p in params {
                    let pv = self.fresh();
                    env.insert(
                        &p.node.name,
                        Scheme {
                            vars: Vec::new(),
                            row_vars: Vec::new(),
                            ty: pv.clone(),
                        },
                    );
                    // 5b-6 §4 (obligation T3): record the parameter's type at its
                    // own span, exactly as a top-level fn parameter is recorded.
                    // The back end then READS a lambda parameter's type instead of
                    // reconstructing it from call-site context.
                    self.node_types.insert(p.span, pv.clone());
                    self.check_ann(&p.node.ann, &pv, false);
                    param_tys.push(pv);
                }
                // The lambda has its OWN latent effect row: infer the body under a
                // fresh ambient. Creating the closure performs nothing, so the row
                // is NOT added to the enclosing `amb`; only *calling* it pours the
                // row in (infer_call). This mirrors top-level fn typing (spec §2.1).
                let lam_amb = self.fresh_row();
                let mark = self.param_tys.len();
                self.param_tys.extend(param_tys.iter().cloned());
                let body_ty = self.infer_block(&body.node, env, lam_amb);
                self.param_tys.truncate(mark);
                env.pop();
                // Fork A (4b-2 §2): close the lambda's residual tail unless it is
                // relayed through a parameter — the same discipline top-level fns
                // use. A concrete-effect lambda gets a minimal closed row (`{Log}`);
                // a relay lambda keeps its open, row-polymorphic tail.
                // Pending sub-effecting inclusions reach this ambient before it
                // closes (a label arriving later is a surfaced conflict).
                self.flush_inclusions();
                self.close_unrelayed_residual(lam_amb, &param_tys);
                // Sub-effecting (the maintainer's decision, 2026-10-08): the
                // VALUE's row is open even when the body's is closed -- a
                // function that performs ε may stand where ε ∪ ρ is expected.
                // The body's own ambient stays closed (nothing leaks in); only
                // the type gains a fresh tail, the row-polymorphic upcast.
                let mut row = self.resolve_row(&EffectRow::open(lam_amb));
                if row.tail == RowTail::Closed {
                    row.tail = RowTail::Open(self.fresh_phantom_row());
                }
                Ty::Fn(param_tys, row, Box::new(body_ty))
            }
        }
    }

    /// Check a pattern against `expected`, collecting variable bindings. A
    /// constructor pattern instantiates the constructor's scheme, unifies its
    /// result with `expected`, and recurses into its field patterns.
    fn check_pattern(
        &mut self,
        pat: &Spanned<Pattern>,
        expected: &Ty,
        env: &TyEnv,
        bindings: &mut Vec<(String, Ty)>,
    ) {
        match &pat.node {
            Pattern::Wild => {}
            Pattern::Var(x) => bindings.push((x.clone(), expected.clone())),
            Pattern::Lit(l) => {
                let lit_ty = match l {
                    PatLit::Int(_) => Ty::int(),
                    PatLit::Bool(_) => Ty::bool(),
                    PatLit::Str(_) => Ty::str(),
                    PatLit::Unit => Ty::unit(),
                };
                self.unify(expected, &lit_ty, pat.span);
            }
            Pattern::Ctor { name, args } => {
                let Some(scheme) = env.lookup(name).cloned() else {
                    return; // unknown constructor is E0432 at resolve time
                };
                let ty = self.instantiate(&scheme);
                let (field_tys, result) = match ty {
                    Ty::Fn(fs, _row, r) => (fs, *r),
                    other => (Vec::new(), other), // nullary constructor
                };
                self.unify(&result, expected, pat.span);
                if args.len() != field_tys.len() {
                    self.diags.push(
                        Diagnostic::error(
                            "E0402",
                            format!(
                                "constructor `{name}` expects {} field(s), found {}",
                                field_tys.len(),
                                args.len()
                            ),
                        )
                        .with_label(pat.span, "wrong number of fields in pattern"),
                    );
                }
                for (a, ft) in args.iter().zip(&field_tys) {
                    self.check_pattern(a, ft, env, bindings);
                }
            }
        }
    }

    fn unify_cond(&mut self, t: &Ty, span: Span) {
        let r = self.resolve(t);
        if r != Ty::bool() && r != Ty::Error && !matches!(r, Ty::Var(_)) {
            self.diags.push(
                Diagnostic::error("E0403", "condition must be `Bool`")
                    .with_label(span, format!("this is `{}`", display_ty(self, &r))),
            );
        } else {
            self.unify(t, &Ty::bool(), span);
        }
    }

    fn infer_call(
        &mut self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
        span: Span,
        env: &mut TyEnv,
        amb: RowVar,
    ) -> Ty {
        // Builtins.
        if let Expr::Qualified { module, name } = &callee.node {
            if module == "io" && name == "println" {
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
                let want = Ty::Fn(vec![Ty::str()], EffectRow::pure(), Box::new(Ty::unit()));
                let got = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(Ty::unit()));
                self.unify(&want, &got, span);
                {
                    let r = self.add_effect(amb, "IO", Vec::new(), span);
                    self.surface_conflict(r);
                } // io.println performs {IO}
                let mut io_labels = BTreeMap::new();
                io_labels.insert(
                    "IO".to_string(),
                    EffectLabel {
                        args: Vec::new(),
                        span: callee.span,
                    },
                );
                self.node_types.insert(
                    callee.span,
                    Ty::Fn(
                        vec![Ty::str()],
                        EffectRow {
                            labels: io_labels,
                            tail: RowTail::Closed,
                        },
                        Box::new(Ty::unit()),
                    ),
                );
                return Ty::unit();
            }
            return Ty::Error; // unknown builtin is E0201 from resolution
        }
        // A constructor application: `Cons(h, t)`. Must be saturated (E0433).
        if let Expr::Var(name) = &callee.node {
            if let Some(&arity) = self.ctor_arity.get(name) {
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
                if args.len() != arity {
                    self.emit_unapplied_ctor(name, arity, span);
                    return Ty::Error;
                }
                let Some(scheme) = env.lookup(name).cloned() else {
                    return Ty::Error;
                };
                let (params, result) = match self.instantiate(&scheme) {
                    Ty::Fn(ps, _r, r) => (ps, *r),
                    other => (Vec::new(), other),
                };
                for (at, pt) in arg_ts.iter().zip(&params) {
                    self.unify(at, pt, span);
                }
                return result;
            }
        }
        // An operation call is a *perform*: it adds its effect to the ambient
        // and yields the operation's declared result type. Locals first (slice
        // 5c-2): a name bound in the environment is a call of that binding. Under
        // E0205 no top-level function shares an op's name, so the environment
        // can hold one only as a local, which shadows the op lexically.
        if let Expr::Var(name) = &callee.node {
            let op = if env.lookup(name).is_none() {
                self.ops.get(name).cloned()
            } else {
                None
            };
            if let Some(op) = op {
                // Instantiate the effect's type params fresh (Slice 4c-2); the
                // fresh args ride in the row, so a second perform of the same
                // effect reconciles against them via `unify_row`.
                let (eff_args, op_params, op_ret) = self.instantiate_op(&op);
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
                // Record the operation-reference callee node against its span
                // (Slice 5a-1's recorder invariant). Only the args route through
                // `infer_expr`, so without this the callee `Var` span is absent
                // from `node_types` and Core lowering of the perform cannot type
                // it. The callee denotes the operation as an arrow that performs
                // its own effect: `fn(op_params) / {Effect(eff_args)} -> op_ret`.
                let mut op_labels = BTreeMap::new();
                op_labels.insert(
                    op.effect.clone(),
                    EffectLabel {
                        args: eff_args.clone(),
                        span: callee.span,
                    },
                );
                let callee_ty = Ty::Fn(
                    op_params.clone(),
                    EffectRow {
                        labels: op_labels,
                        tail: RowTail::Closed,
                    },
                    Box::new(op_ret.clone()),
                );
                self.node_types.insert(callee.span, callee_ty);
                let want = Ty::Fn(op_params, EffectRow::pure(), Box::new(op_ret.clone()));
                let got = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(op_ret.clone()));
                self.unify(&want, &got, span);
                {
                    let r = self.add_effect(amb, &op.effect, eff_args, span);
                    self.surface_conflict(r);
                }
                return op_ret;
            }
        }
        // Ordinary function call: unify the arrow's param/result types, then pour
        // the callee's latent effect row into the ambient (`amb ⊇ ε_f`).
        let f = match &callee.node {
            // A callee is not a value use: no sub-effecting opening (the call
            // pours exactly the callee's row into the ambient).
            Expr::Var(name) => match env.lookup(name) {
                Some(s) => {
                    let s = s.clone();
                    let t = self.instantiate(&s);
                    self.node_types.insert(callee.span, t.clone());
                    t
                }
                None => Ty::Error,
            },
            _ => self.infer_expr(callee, env, amb),
        };
        let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
        let result = self.fresh();
        // Phantom until the callee gives it meaning: unified with a meaningful
        // row (a relay, an in-progress row) it stops being phantom
        // (`bind_row`); unified only with the callee's upcast tail, it stays
        // phantom and is closed below.
        let call_row = self.fresh_phantom_row();
        let expected = Ty::Fn(arg_ts, EffectRow::open(call_row), Box::new(result.clone()));
        self.unify(&f, &expected, span);
        let mut eff = self.resolve_row(&EffectRow::open(call_row));
        // Sub-effecting: a PHANTOM callee tail (an upcast, see `phantom`) that
        // nothing in the environment shares is not unified with the ambient --
        // that would make this use's row EQUAL to the caller's and merge rows
        // related only by inclusion. The ambient must include whatever the tail
        // becomes: recorded in `pending_incl`, flushed later. A meaningful tail
        // (a RELAY through a parameter, an in-progress row) is unified as before.
        if let RowTail::Open(v) = eff.tail {
            let mut in_params = Vec::new();
            for p in self.param_tys.clone() {
                free_row_vars(self, &p, &mut in_params);
            }
            if self.phantom.contains(&v) && !in_params.contains(&v) {
                self.pending_incl.push((v, amb, span));
                eff.tail = RowTail::Closed;
            }
        }
        {
            let r = self.add_row(amb, &eff, span);
            self.surface_conflict(r);
        }
        result
    }

    /// Type `handle e with H`: infer `e` under a fresh inner ambient seeded with
    /// the handled effect `E`; type each clause (binding params + `resume : (B)
    /// -> R`) and the return clause; then *discharge* `E` so the residual effects
    /// of `e` pass through into the enclosing ambient (spec §3.4).
    fn infer_handle(
        &mut self,
        body: &Spanned<Expr>,
        handler: &Handler,
        span: Span,
        env: &mut TyEnv,
        amb: RowVar,
    ) -> Ty {
        let effect = self.handler_effect(handler);
        // Conformance (Slice 4d-1): `with multi` is legal only over a `multi`-
        // declared effect. Multi-resuming a one-shot effect is E0427. `is_multi`
        // is looked up by name — no row involvement.
        if handler.multi {
            if let Some(e) = &effect {
                if !self.effect_multi.get(e).copied().unwrap_or(false) {
                    self.diags.push(
                        Diagnostic::error(
                            "E0427",
                            "a one-shot effect cannot be handled with `multi`".to_string(),
                        )
                        .with_label(span, format!("`{e}` is handled `with multi` here"))
                        .with_help(format!(
                            "effect `{e}` is one-shot (its continuation resumes at most once); declare it `effect multi {e} {{ … }}` to allow multi-shot resumption, or drop `multi`"
                        )),
                    );
                }
            }
        }
        // One instantiation of the handled effect's type params (Slice 4c-2),
        // shared by the seeded ambient AND every clause — so the body's performs
        // (which unify against the seed via the row) and the clauses agree on the
        // effect's type arguments. Empty for a monomorphic effect.
        let effect_param_ids: Vec<u32> = match &effect {
            Some(e) => self
                .ops
                .values()
                .find(|o| &o.effect == e)
                .map(|o| o.effect_params.clone())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let eff_args: Vec<Ty> = effect_param_ids.iter().map(|_| self.fresh()).collect();
        // `e` may perform E plus a polymorphic remainder.
        let amb_in = self.fresh_row();
        if let Some(e) = &effect {
            {
                let r = self.add_effect(amb_in, e, eff_args.clone(), span);
                self.surface_conflict(r);
            }
        }
        let body_ty = self.infer_expr(body, env, amb_in);

        // R — the handle's result type, shared by every clause and `return`.
        let result = self.fresh();
        // ε — what a `resume` in a clause may still perform: the body's effects
        // minus the handled one (the resumed handler handles that one again).
        // Its tail is the body's row variable, so effects the body's row gains
        // later reach every resume too.
        let mut resume_eff = self.resolve_row(&EffectRow::open(amb_in));
        if let Some(e) = &effect {
            resume_eff.labels.remove(e);
        }
        for c in &handler.clauses {
            let clause = &c.node;
            let (params, b) = match self.ops.get(&clause.op).cloned() {
                Some(op) => {
                    // Substitute the effect's type params with this handler's
                    // shared instantiation, so clause params/`resume` agree with
                    // the body's performs on the effect's type arguments.
                    let m: HashMap<u32, Ty> = op
                        .effect_params
                        .iter()
                        .cloned()
                        .zip(eff_args.iter().cloned())
                        .collect();
                    let rm = HashMap::new();
                    (
                        op.params
                            .iter()
                            .map(|t| subst_vars(t, &m, &rm))
                            .collect::<Vec<_>>(),
                        subst_vars(&op.ret, &m, &rm),
                    )
                }
                None => (Vec::new(), Ty::Error),
            };
            env.push();
            for (idx, p) in clause.params.iter().enumerate() {
                let pty = params.get(idx).cloned().unwrap_or_else(|| self.fresh());
                // Slice 5b-8 Task 5: record the clause parameter's type at its own
                // span, exactly as a lambda parameter is recorded (obligation T3).
                // Core lowering then READS it out of the frozen table, the same
                // lookup it makes for a lambda parameter, instead of guessing it.
                self.node_types.insert(p.span, pty.clone());
                env.insert(
                    &p.node.name,
                    Scheme {
                        vars: Vec::new(),
                        row_vars: Vec::new(),
                        ty: pty,
                    },
                );
            }
            // The clause body runs at the handler's *outer* ambient; `resume`
            // there takes B and yields R.
            self.resume_stack
                .push((b, result.clone(), resume_eff.clone(), effect.clone()));
            let mark = self.param_tys.len();
            self.param_tys.extend(params.iter().cloned());
            let clause_ty = self.infer_expr(&clause.body, env, amb);
            self.param_tys.truncate(mark);
            self.unify(&clause_ty, &result, clause.body.span);
            self.resume_stack.pop();
            env.pop();
        }
        match &handler.ret {
            Some(ret) => {
                env.push();
                env.insert(
                    &ret.binder,
                    Scheme {
                        vars: Vec::new(),
                        row_vars: Vec::new(),
                        ty: body_ty.clone(),
                    },
                );
                let ret_ty = self.infer_expr(&ret.body, env, amb);
                self.unify(&ret_ty, &result, ret.body.span);
                env.pop();
            }
            // No return clause ⇒ identity: R = type of `e`.
            None => self.unify(&result, &body_ty, span),
        }

        // Cleanup lint (E0426): a `with multi` handler may re-run its captured
        // continuation, repeating any observably-duplicable effect the body
        // performs. Best-effort (spec §8.6) — a full guarantee awaits linear types.
        if handler.multi {
            let body_row = self.resolve_row(&EffectRow::open(amb_in));
            let dup: Vec<&str> = OBSERVABLE_EFFECTS
                .iter()
                .copied()
                .filter(|e| body_row.labels.contains_key(*e))
                .collect();
            if !dup.is_empty() {
                let set = format!("{{{}}}", dup.join(", "));
                self.diags.push(
                    Diagnostic::warning(
                        "E0426",
                        "a multi-shot handler may run its continuation's effects more than once",
                    )
                    .with_label(
                        span,
                        format!(
                            "the handled computation performs {set}; a multi-shot resume repeats those effects"
                        ),
                    )
                    .with_help("best-effort lint — a full guarantee awaits linear/affine types"),
                );
            }
        }

        // Discharge E: everything `e` performed *except* E flows into the ambient.
        let mut residual = self.resolve_row(&EffectRow::open(amb_in));
        if let Some(e) = &effect {
            residual.labels.remove(e);
        }
        {
            let r = self.add_row(amb, &residual, span);
            self.surface_conflict(r);
        }
        result
    }

    /// The single effect a handler covers — all clauses must agree. Clauses that
    /// span more than one effect are `E0423` (a well-formedness condition on the
    /// handler's typed meaning, so it lives with the type info; plan 5a).
    fn handler_effect(&mut self, handler: &Handler) -> Option<String> {
        let mut found: Option<String> = None;
        for c in &handler.clauses {
            let clause = &c.node;
            let eff = clause
                .effect
                .clone()
                .or_else(|| self.ops.get(&clause.op).map(|o| o.effect.clone()));
            match (&found, eff) {
                (None, Some(e)) => found = Some(e),
                (Some(f), Some(e)) if *f != e => {
                    self.diags.push(
                        Diagnostic::error("E0423", "a handler must cover a single effect")
                            .with_label(
                                c.span,
                                format!(
                                "this clause handles `{e}`, but the handler already handles `{f}`"
                            ),
                            ),
                    );
                }
                _ => {}
            }
        }
        found
    }

    /// Generalize `t` under `env`: quantify the type *and* row variables free in
    /// `t` but not in the environment (row polymorphism, spec §3.5).
    fn generalize(&mut self, t: &Ty, env: &TyEnv) -> Scheme {
        let resolved = self.resolve(t);
        let mut in_ty = Vec::new();
        free_vars(self, &resolved, &mut in_ty);
        let mut in_env = Vec::new();
        env_free_vars(self, env, &mut in_env);
        // The function's annotation variables are part of the environment: a
        // lowercase name is ONE type for the whole function, so an inner `let`
        // must not quantify it (the annotations review's F1: a later binding
        // of it rewrote the generalized nodes, and natively a captured ADT was
        // treated as an untraced Int).
        for t in self.ann_vars.values().cloned().collect::<Vec<_>>() {
            free_vars(self, &t, &mut in_env);
        }
        let vars: Vec<u32> = in_ty.into_iter().filter(|v| !in_env.contains(v)).collect();

        let mut row_in_ty = Vec::new();
        free_row_vars(self, &resolved, &mut row_in_ty);
        let mut row_in_env = Vec::new();
        env_free_row_vars(self, env, &mut row_in_env);
        let row_vars: Vec<RowVar> = row_in_ty
            .into_iter()
            .filter(|v| !row_in_env.contains(v))
            .collect();
        Scheme {
            ty: resolved,
            vars,
            row_vars,
        }
    }

    /// Close a function's ambient residual tail *unless* that tail is relayed
    /// through a parameter's effect row. A function that performs a concrete set
    /// of effects (e.g. `{Log}`) thus gets a closed row; only one that relays a
    /// higher-order argument's effects stays row-polymorphic (spec §3.5).
    ///
    /// Known limitation (flagged): a function that BOTH relays a parameter and
    /// performs its own concrete effect keeps an open tail and may print a more
    /// general row than minimal. Not exercised by the 3b corpus (no lambdas;
    /// row-poly is demonstrated via the pure relay `run_it`).
    fn close_unrelayed_residual(&mut self, amb: RowVar, params: &[Ty]) {
        let resolved = self.resolve_row(&EffectRow::open(amb));
        if let RowTail::Open(rho) = resolved.tail {
            let mut in_params = Vec::new();
            for p in params {
                free_row_vars(self, p, &mut in_params);
            }
            if !in_params.contains(&rho) {
                self.bind_row(rho, &EffectRow::pure(), Span::EMPTY);
            }
        }
    }

    /// Enforce that an *annotated* function performs exactly its declared row
    /// (spec §3.6, strict default — no sub-effecting). Declaring pure (`/ {}`)
    /// but performing an effect is `E0421`; any set difference (in either
    /// direction) is `E0423`, naming the exact labels it differs by.
    fn check_exact_row(&mut self, amb: RowVar, declared: &[Spanned<String>]) {
        let performed = self.resolve_row(&EffectRow::open(amb));
        if declared.is_empty() {
            // Declared pure `/ {}`: any performed effect is a purity violation.
            if !performed.labels.is_empty() {
                let names: Vec<String> = performed.labels.keys().cloned().collect();
                let span = performed
                    .labels
                    .values()
                    .next()
                    .map(|l| l.span)
                    .unwrap_or(Span::EMPTY);
                self.diags.push(
                    Diagnostic::error(
                        "E0421",
                        format!(
                            "this function is declared pure but performs `{}`",
                            names.join(", ")
                        ),
                    )
                    .with_label(span, "performed here")
                    .with_help(format!(
                        "the declared row is {{}} (pure); the body performs {{{}}}",
                        names.join(", ")
                    )),
                );
            }
            return;
        }
        let declared_names: Vec<&str> = declared.iter().map(|l| l.node.as_str()).collect();
        // performed \ declared — the body does more than it declares.
        let extra: Vec<(String, Span)> = performed
            .labels
            .iter()
            .filter(|(k, _)| !declared_names.contains(&k.as_str()))
            .map(|(k, l)| (k.clone(), l.span))
            .collect();
        if !extra.is_empty() {
            let span = extra[0].1;
            self.emit_row_mismatch(
                &extra,
                span,
                "the body performs an effect the declared row does not",
            );
        }
        // declared \ performed — the body declares more than it does.
        let unused: Vec<(String, Span)> = declared
            .iter()
            .filter(|l| !performed.labels.contains_key(&l.node))
            .map(|l| (l.node.clone(), l.span))
            .collect();
        if !unused.is_empty() {
            let span = unused[0].1;
            self.emit_row_mismatch(
                &unused,
                span,
                "the declared row includes an effect the body never performs",
            );
        }
    }

    /// The discharge pass at `main`: `main` may perform only `{IO}` (natively
    /// discharged by the runtime). Any user effect that survives to `main` is
    /// unhandled — `E0420`, naming the perform site from provenance.
    fn check_main_discharge(&mut self, amb: RowVar) {
        let performed = self.resolve_row(&EffectRow::open(amb));
        for (label, l) in &performed.labels {
            if label != "IO" {
                self.diags.push(
                    Diagnostic::error("E0420", format!("effect `{label}` is never handled"))
                        .with_label(l.span, format!("`{label}` is performed here"))
                        .with_help(format!(
                            "`main` may perform only {{IO}}; handle it with `handle … with {{ {label}.op(..) -> … }}`"
                        )),
                );
            }
        }
    }
}

/// Elaborate an ADT field/argument type annotation under a type-parameter
/// environment: a bare param name → its type variable; a base name → its
/// `Base`; an `Upper(args)` → a `Ty::Con` (arity-checked against `known`).
///
/// A function type `fn(A) / {E} -> R` (2026-10-09, async step 1): a
/// declaration has no row variables, so its rows are exactly what is written
/// -- no `/ {..}` is the empty row (pure), not "any effects" as in a
/// function's own annotations. A row may name `IO` and any declared effect
/// without type parameters (`effects` maps every declared effect to its
/// arity); an effect's arguments cannot be written in a row yet.
fn elaborate_adt_ty(
    inf: &mut Infer,
    ann: &Spanned<TypeAnn>,
    param_env: &HashMap<String, Ty>,
    known: &HashMap<String, usize>,
    effects: &HashMap<String, usize>,
) -> Ty {
    let t = &ann.node;
    if t.name == "fn" && !t.args.is_empty() {
        let n = t.args.len();
        let params: Vec<Ty> = t.args[..n - 1]
            .iter()
            .map(|a| elaborate_adt_ty(inf, a, param_env, known, effects))
            .collect();
        let ret = elaborate_adt_ty(inf, &t.args[n - 1], param_env, known, effects);
        let mut labels = BTreeMap::new();
        for l in t.row.iter().flatten() {
            match effects.get(&l.node) {
                _ if l.node == "IO" => {}
                Some(0) => {}
                Some(_) => {
                    inf.diags.push(
                        Diagnostic::error(
                            "E0432",
                            format!(
                                "effect `{}` takes type arguments, which a declaration row cannot write yet",
                                l.node
                            ),
                        )
                        .with_label(l.span, "a parameterized effect"),
                    );
                    continue;
                }
                None => {
                    inf.diags.push(
                        Diagnostic::error("E0432", format!("unknown effect `{}`", l.node))
                            .with_label(l.span, "no such effect"),
                    );
                    continue;
                }
            }
            labels.insert(
                l.node.clone(),
                EffectLabel {
                    args: Vec::new(),
                    span: l.span,
                },
            );
        }
        return Ty::Fn(
            params,
            EffectRow {
                labels,
                tail: RowTail::Closed,
            },
            Box::new(ret),
        );
    }
    if t.args.is_empty() {
        if let Some(ty) = param_env.get(&t.name) {
            return ty.clone();
        }
    }
    match t.name.as_str() {
        "Int" => Ty::int(),
        "Float" => Ty::float(),
        "Bool" => Ty::bool(),
        "String" => Ty::str(),
        "Unit" => Ty::unit(),
        other => match known.get(other) {
            Some(&arity) => {
                if t.args.len() != arity {
                    inf.diags.push(
                        Diagnostic::error(
                            "E0400",
                            format!(
                                "type `{other}` expects {arity} argument(s), found {}",
                                t.args.len()
                            ),
                        )
                        .with_label(ann.span, "wrong number of type arguments"),
                    );
                }
                let mut args = Vec::new();
                for a in &t.args {
                    args.push(elaborate_adt_ty(inf, a, param_env, known, effects));
                }
                Ty::Con(other.to_string(), args)
            }
            None => {
                inf.diags.push(
                    Diagnostic::error("E0432", format!("unknown type `{other}`"))
                        .with_label(ann.span, "no such type"),
                );
                Ty::Error
            }
        },
    }
}

fn free_vars(inf: &Infer, t: &Ty, acc: &mut Vec<u32>) {
    match inf.resolve(t) {
        Ty::Var(v) => {
            if !acc.contains(&v) {
                acc.push(v);
            }
        }
        Ty::Base(_) | Ty::Error => {}
        Ty::Fn(ps, row, r) => {
            // Row variables are generalized separately; this collects free *type*
            // variables — including any that appear ONLY inside an effect argument
            // (e.g. the `s` of `{State(s)}`), which must still be generalized.
            for p in &ps {
                free_vars(inf, p, acc);
            }
            for l in row.labels.values() {
                for a in &l.args {
                    free_vars(inf, a, acc);
                }
            }
            free_vars(inf, &r, acc);
        }
        Ty::Tuple(xs) => {
            for x in &xs {
                free_vars(inf, x, acc);
            }
        }
        Ty::Con(_, args) => {
            for a in &args {
                free_vars(inf, a, acc);
            }
        }
    }
}

fn env_free_vars(inf: &Infer, env: &TyEnv, acc: &mut Vec<u32>) {
    for scope in &env.scopes {
        for scheme in scope.values() {
            let mut fv = Vec::new();
            free_vars(inf, &scheme.ty, &mut fv);
            for v in fv {
                if !scheme.vars.contains(&v) && !acc.contains(&v) {
                    acc.push(v);
                }
            }
        }
    }
}

/// Collect the free effect-row variables of a type (the open tails of its
/// arrows). The type is resolved first, so tails are their residual variables.
fn free_row_vars(inf: &Infer, t: &Ty, acc: &mut Vec<RowVar>) {
    match inf.resolve(t) {
        Ty::Fn(ps, row, r) => {
            for p in &ps {
                free_row_vars(inf, p, acc);
            }
            if let RowTail::Open(v) = row.tail {
                if !acc.contains(&v) {
                    acc.push(v);
                }
            }
            // An effect argument may itself be a function type carrying a row.
            for l in row.labels.values() {
                for a in &l.args {
                    free_row_vars(inf, a, acc);
                }
            }
            free_row_vars(inf, &r, acc);
        }
        Ty::Tuple(xs) => {
            for x in &xs {
                free_row_vars(inf, x, acc);
            }
        }
        Ty::Con(_, args) => {
            for a in &args {
                free_row_vars(inf, a, acc);
            }
        }
        _ => {}
    }
}

fn env_free_row_vars(inf: &Infer, env: &TyEnv, acc: &mut Vec<RowVar>) {
    for scope in &env.scopes {
        for scheme in scope.values() {
            let mut fv = Vec::new();
            free_row_vars(inf, &scheme.ty, &mut fv);
            for v in fv {
                if !scheme.row_vars.contains(&v) && !acc.contains(&v) {
                    acc.push(v);
                }
            }
        }
    }
}

fn subst_vars(t: &Ty, m: &HashMap<u32, Ty>, rm: &HashMap<RowVar, RowVar>) -> Ty {
    match t {
        Ty::Var(v) => m.get(v).cloned().unwrap_or(Ty::Var(*v)),
        Ty::Base(c) => Ty::Base(*c),
        Ty::Fn(ps, row, r) => {
            let tail = match &row.tail {
                RowTail::Open(v) => RowTail::Open(*rm.get(v).unwrap_or(v)),
                other => other.clone(),
            };
            let labels = row
                .labels
                .iter()
                .map(|(k, l)| {
                    (
                        k.clone(),
                        EffectLabel {
                            args: l.args.iter().map(|a| subst_vars(a, m, rm)).collect(),
                            span: l.span,
                        },
                    )
                })
                .collect();
            Ty::Fn(
                ps.iter().map(|p| subst_vars(p, m, rm)).collect(),
                EffectRow { labels, tail },
                Box::new(subst_vars(r, m, rm)),
            )
        }
        Ty::Tuple(xs) => Ty::Tuple(xs.iter().map(|x| subst_vars(x, m, rm)).collect()),
        Ty::Con(n, args) => Ty::Con(
            n.clone(),
            args.iter().map(|a| subst_vars(a, m, rm)).collect(),
        ),
        Ty::Error => Ty::Error,
    }
}

pub fn infer(session: &Session, module: &Module) -> Vec<Diagnostic> {
    let (_schemes, diags) = infer_schemes(session, module);
    diags
}

pub fn infer_schemes(
    _session: &Session,
    module: &Module,
) -> (Vec<(String, String)>, Vec<Diagnostic>) {
    let (schemes, diags, _sites, _typed) = infer_all(module, false);
    (schemes, diags)
}

/// Inference plus the affine binding-site set (Slice 4d-2): which let-bindings /
/// params have a `linear`-declared type. The one bounded reach the `affine::check`
/// pass consumes; the affine *logic* lives entirely in that pass.
pub fn infer_with_sites(_session: &Session, module: &Module) -> (Vec<Diagnostic>, HashSet<Span>) {
    let (_schemes, diags, sites, _typed) = infer_all(module, false);
    (diags, sites)
}

/// Inference plus the per-node type table (Slice 5a-1): every expression node's
/// zonked type, rendered span-sorted through one shared `Names` so a variable
/// shared across nodes renders identically. The Core IR arc's feeder; 5a-2's
/// lowering consumes the underlying `node_types` directly.
pub fn infer_with_types(
    _session: &Session,
    module: &Module,
) -> (Vec<Diagnostic>, BTreeMap<Span, String>) {
    let (_schemes, diags, _sites, typed) = infer_all(module, true);
    let mut names = Names::default();
    let mut rendered: BTreeMap<Span, String> = BTreeMap::new();
    for (span, ty) in &typed {
        let mut out = String::new();
        write_ty(ty, &mut names, &mut out);
        rendered.insert(*span, out);
    }
    (diags, rendered)
}

/// Inference plus the raw per-node type table (Slice 5a-2): the same zonked
/// `Span → Ty` table `infer_with_types` renders, returned as structured `Ty`
/// values (not strings) so the Core IR lowering can carry them inline. A sibling
/// of `infer_with_types`; both run `infer_all(module, true)`.
pub fn infer_typed_table(
    _session: &Session,
    module: &Module,
) -> (Vec<Diagnostic>, BTreeMap<Span, Ty>) {
    let (_schemes, diags, _sites, typed) = infer_all(module, true);
    (diags, typed)
}

/// Result of a full-module inference pass: generalized top-level schemes (name →
/// rendered scheme), diagnostics, affine binding sites, and — when requested —
/// the zonked per-node type table (Slice 5a-1). Empty `BTreeMap` when types
/// weren't requested.
type InferAllOut = (
    Vec<(String, String)>,
    Vec<Diagnostic>,
    HashSet<Span>,
    BTreeMap<Span, Ty>,
);

/// Type every top-level function, processing mutually-recursive groups (SCCs of
/// the call graph) in dependency order so each group is generalized before later
/// groups use it — giving proper let-polymorphism across the top level.
fn infer_all(module: &Module, want_types: bool) -> InferAllOut {
    let mut inf = Infer::new();
    // Names of `linear`-declared types — set before inference so `ty_is_linear`
    // sees them while recording affine binding sites (Slice 4d-2).
    for d in &module.decls {
        if let Decl::Type(t) = &d.node {
            if t.is_linear {
                inf.linear_types.insert(t.name.clone());
            }
        }
    }
    let mut env = TyEnv::new();

    // Register ADT constructors as polymorphic schemes (two-pass: type-name
    // arities first — so recursive/mutual types elaborate — then the schemes).
    let mut known_types: HashMap<String, usize> = HashMap::new();
    for d in &module.decls {
        if let Decl::Type(t) = &d.node {
            known_types.insert(t.name.clone(), t.params.len());
        }
    }
    inf.known_types = known_types.clone();
    // Every declared effect and its arity, before any declaration is
    // elaborated: a function type in a field or an operation may name an
    // effect declared later, or the effect being declared.
    let mut effect_arity: HashMap<String, usize> = HashMap::new();
    for d in &module.decls {
        if let Decl::Effect(e) = &d.node {
            effect_arity.insert(e.name.clone(), e.params.len());
        }
    }
    for d in &module.decls {
        if let Decl::Type(t) = &d.node {
            let param_vars: Vec<Ty> = t.params.iter().map(|_| inf.fresh()).collect();
            let var_ids: Vec<u32> = param_vars
                .iter()
                .map(|v| match v {
                    Ty::Var(id) => *id,
                    _ => unreachable!(),
                })
                .collect();
            let param_env: HashMap<String, Ty> = t
                .params
                .iter()
                .cloned()
                .zip(param_vars.iter().cloned())
                .collect();
            let result = Ty::Con(t.name.clone(), param_vars.clone());
            for v in &t.variants {
                let vd = &v.node;
                inf.ctor_arity.insert(vd.name.clone(), vd.fields.len());
                let mut field_tys = Vec::new();
                for f in &vd.fields {
                    field_tys.push(elaborate_adt_ty(
                        &mut inf,
                        f,
                        &param_env,
                        &known_types,
                        &effect_arity,
                    ));
                }
                let ty = if field_tys.is_empty() {
                    result.clone()
                } else {
                    Ty::Fn(field_tys, EffectRow::pure(), Box::new(result.clone()))
                };
                env.insert(
                    &vd.name,
                    Scheme {
                        vars: var_ids.clone(),
                        row_vars: Vec::new(),
                        ty,
                    },
                );
            }
        }
    }

    // Elaborate effect declarations into the operation-signature table, so a
    // call to an operation is recognised as a perform during inference.
    for d in &module.decls {
        if let Decl::Effect(e) = &d.node {
            // Record the effect's resumption discipline for the `with multi`
            // conformance rule (Slice 4d-1) — a declaration fact, read by name.
            inf.effect_multi.insert(e.name.clone(), e.is_multi);
            // The effect's type parameters -> fresh type variables, shared by all
            // its operation signatures (Slice 4c-2). Empty for a monomorphic
            // effect. Op signatures elaborate under this param-env (reusing the
            // ADT elaborator), which is what lifts the old E0404 generic-op reject.
            let param_vars: Vec<Ty> = e.params.iter().map(|_| inf.fresh()).collect();
            let effect_params: Vec<u32> = param_vars
                .iter()
                .map(|v| match v {
                    Ty::Var(id) => *id,
                    _ => unreachable!(),
                })
                .collect();
            let param_env: HashMap<String, Ty> = e
                .params
                .iter()
                .cloned()
                .zip(param_vars.iter().cloned())
                .collect();
            for op in &e.ops {
                let sig = &op.node;
                let params: Vec<Ty> = sig
                    .param_tys
                    .iter()
                    .map(|t| elaborate_adt_ty(&mut inf, t, &param_env, &known_types, &effect_arity))
                    .collect();
                let ret =
                    elaborate_adt_ty(&mut inf, &sig.ret, &param_env, &known_types, &effect_arity);
                inf.ops.insert(
                    sig.name.clone(),
                    OpInfo {
                        effect: e.name.clone(),
                        effect_params: effect_params.clone(),
                        params,
                        ret,
                    },
                );
            }
        }
    }

    let fns: Vec<&FnDecl> = module
        .decls
        .iter()
        .filter_map(|d| match &d.node {
            Decl::Fn(f) => Some(f),
            Decl::Effect(_) => None,
            Decl::Type(_) => None,
        })
        .collect();
    let name_idx: HashMap<&str, usize> = fns
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.as_str(), i))
        .collect();
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); fns.len()];
    for (i, f) in fns.iter().enumerate() {
        let mut refs = Vec::new();
        collect_refs(&f.body.node, &mut refs);
        for r in refs {
            if let Some(&j) = name_idx.get(r.as_str()) {
                if !edges[i].contains(&j) {
                    edges[i].push(j);
                }
            }
        }
    }
    let groups = tarjan_scc(&edges); // callees before callers (reverse topological)

    let mut schemes_out: Vec<(String, String)> = Vec::new();

    for group in &groups {
        // 1. fresh monotype per member (with a fresh open ambient row), in scope
        //    for the whole group.
        let mut member_ty: HashMap<usize, (Vec<Ty>, RowVar, Ty)> = HashMap::new();
        let mut ann_vars_of: HashMap<usize, HashMap<String, Ty>> = HashMap::new();
        for &i in group {
            let f = fns[i];
            let params: Vec<Ty> = f.params.iter().map(|_| inf.fresh()).collect();
            let amb_f = inf.fresh_row();
            let result = inf.fresh();
            env.insert(
                &f.name,
                Scheme {
                    vars: Vec::new(),
                    row_vars: Vec::new(),
                    ty: Ty::Fn(
                        params.clone(),
                        EffectRow::open(amb_f),
                        Box::new(result.clone()),
                    ),
                },
            );
            // Annotations (2026-10-09): each parameter's and the result's,
            // against the member's monotype, before any body is inferred --
            // so a call earlier in the group already sees them. The function's
            // annotation variables live on into its body (step 2).
            inf.ann_vars = HashMap::new();
            for (p, pty) in f.params.iter().zip(&params) {
                inf.check_ann(&p.node.ann, pty, false);
            }
            inf.check_ann(&f.ret_ann, &result, true);
            ann_vars_of.insert(i, std::mem::take(&mut inf.ann_vars));
            member_ty.insert(i, (params, amb_f, result));
        }
        // 2. infer each body under its params + ambient (monomorphic in-group).
        for &i in group {
            let f = fns[i];
            let (params, amb_f, result) = member_ty[&i].clone();
            inf.ann_vars = ann_vars_of.remove(&i).unwrap_or_default();
            env.push();
            for (p, pty) in f.params.iter().zip(&params) {
                // Slice 5b-3 §3.2: record the parameter's type at its OWN span,
                // so `lower_module` can look it up the same way it looks up an
                // expression's. The precedent for recording a non-expression
                // span is the callee-span insert in the `Call` arm. Recorded
                // pre-zonk like every other entry — the single zonk pass at the
                // end of this function maps over the whole table, so these
                // resolve for free.
                inf.node_types.insert(p.span, pty.clone());
                env.insert(
                    &p.node.name,
                    Scheme {
                        vars: Vec::new(),
                        row_vars: Vec::new(),
                        ty: pty.clone(),
                    },
                );
            }
            inf.param_tys = params.clone();
            let body_ty = inf.infer_block(&f.body.node, &mut env, amb_f);
            inf.param_tys.clear();
            inf.unify(&result, &body_ty, f.body.span);
            env.pop();
        }
        // 2.4. Sub-effecting: every pending inclusion reaches its ambient
        //      before anything closes. A recorded tail that is now free in a
        //      member's PARAMETER types is a relay: it stops being phantom (the
        //      review's F2 -- closing it at 2.7 forced every `k` pure).
        {
            let mut in_params = Vec::new();
            for &i in group {
                for p in &member_ty[&i].0 {
                    free_row_vars(&inf, p, &mut in_params);
                }
            }
            for (src, _, _) in inf.pending_incl.clone() {
                if let RowTail::Open(w) = inf.resolve_row(&EffectRow::open(src)).tail {
                    if in_params.contains(&w) {
                        inf.phantom.remove(&w);
                    }
                }
            }
        }
        inf.flush_inclusions();
        // 2.5. close each member's residual ambient (once all in-group effects
        //      are accumulated), unless it is relayed through a parameter.
        for &i in group {
            let (params, amb_f, _result) = member_ty[&i].clone();
            inf.close_unrelayed_residual(amb_f, &params);
        }
        // 2.6. enforce annotated rows (exact match) and main's discharge set.
        for &i in group {
            let f = fns[i];
            let (_params, amb_f, _result) = member_ty[&i].clone();
            if let Some(declared) = &f.effect_row {
                inf.check_exact_row(amb_f, declared);
            }
            if f.name == "main" {
                inf.check_main_discharge(amb_f);
            }
        }
        // 2.7. Sub-effecting: the group is solved; an upcast tail nothing bound
        //      is closed (Koka's close-at-generalization; value uses re-open).
        inf.flush_inclusions();
        inf.pending_incl.clear();
        let pending: Vec<RowVar> = inf.phantom.iter().copied().collect();
        for v in pending {
            if inf.row_subst[v as usize].is_none() {
                inf.bind_row(v, &EffectRow::pure(), Span::EMPTY);
            }
        }
        inf.phantom.clear();
        // 3. generalize each member and re-insert its polytype for later groups.
        for &i in group {
            let f = fns[i];
            let (params, amb_f, result) = member_ty[&i].clone();
            let fnty = Ty::Fn(params, EffectRow::open(amb_f), Box::new(result));
            let scheme = generalize_toplevel(&mut inf, &fnty, &env, group, &fns);
            env.insert(&f.name, scheme.clone());
            schemes_out.push((f.name.clone(), display_scheme(&inf, &scheme)));
        }
    }

    // Record-then-zonk (spec §3): resolve every recorded node type ONCE here,
    // after all SCC groups are solved — never at record-time. Skipped unless a
    // caller wants the table, so the hot inference paths pay nothing.
    let node_types: BTreeMap<Span, Ty> = if want_types {
        inf.node_types
            .iter()
            .map(|(s, t)| (*s, inf.resolve(t)))
            .collect()
    } else {
        BTreeMap::new()
    };
    (schemes_out, inf.diags, inf.affine_sites, node_types)
}

fn collect_refs(b: &Block, acc: &mut Vec<String>) {
    for st in b.stmts.iter() {
        match &st.node {
            Stmt::Let { value, .. } => collect_refs_expr(&value.node, acc),
            Stmt::Expr(e) => collect_refs_expr(&e.node, acc),
        }
    }
    if let Some(t) = &b.tail {
        collect_refs_expr(&t.node, acc);
    }
}

fn collect_refs_expr(e: &Expr, acc: &mut Vec<String>) {
    match e {
        Expr::Var(n) => acc.push(n.clone()),
        Expr::Call { callee, args } => {
            collect_refs_expr(&callee.node, acc);
            for a in args.iter() {
                collect_refs_expr(&a.node, acc);
            }
        }
        Expr::Unary { expr, .. } => collect_refs_expr(&expr.node, acc),
        Expr::Binary { lhs, rhs, .. } => {
            collect_refs_expr(&lhs.node, acc);
            collect_refs_expr(&rhs.node, acc);
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_refs_expr(&cond.node, acc);
            collect_refs(&then_block.node, acc);
            collect_refs(&else_block.node, acc);
        }
        Expr::Block(b) => collect_refs(b, acc),
        // Exhaustive, with no catch-all (fixed 2026-10-05): a call hidden in a
        // match arm, a lambda or a handler used to be no edge at all, so a
        // caller could be generalised before its callee was typed -- `forall a`
        // results, and ill-typed programs that checked clean. Over-approximation
        // (a local sharing a function's name) only merges SCCs, which is safe.
        Expr::Match { scrutinee, arms } => {
            collect_refs_expr(&scrutinee.node, acc);
            for arm in arms.iter() {
                collect_refs_expr(&arm.node.body.node, acc);
            }
        }
        Expr::Lambda { body, .. } => collect_refs(&body.node, acc),
        Expr::Handle { body, handler } => {
            collect_refs_expr(&body.node, acc);
            for c in &handler.clauses {
                collect_refs_expr(&c.node.body.node, acc);
            }
            if let Some(r) = &handler.ret {
                collect_refs_expr(&r.body.node, acc);
            }
        }
        Expr::Resume { arg } => collect_refs_expr(&arg.node, acc),
        Expr::Int(_)
        | Expr::Float(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Unit
        | Expr::Qualified { .. } => {}
    }
}

/// Iterative Tarjan's SCC. Components are emitted callees-before-callers
/// (reverse topological), which is the order generalization needs.
fn tarjan_scc(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = edges.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut counter = 0usize;
    let mut out: Vec<Vec<usize>> = Vec::new();

    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&(v, mut pi)) = call.last() {
            if pi == 0 {
                index[v] = counter;
                low[v] = counter;
                counter += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            let mut recursed = false;
            while pi < edges[v].len() {
                let w = edges[v][pi];
                pi += 1;
                if index[w] == usize::MAX {
                    call.last_mut().unwrap().1 = pi;
                    call.push((w, 0));
                    recursed = true;
                    break;
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            }
            if recursed {
                continue;
            }
            call.last_mut().unwrap().1 = pi;
            if low[v] == index[v] {
                let mut comp = Vec::new();
                loop {
                    let w = stack.pop().unwrap();
                    on_stack[w] = false;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                out.push(comp);
            }
            call.pop();
            if let Some(&(parent, _)) = call.last() {
                low[parent] = low[parent].min(low[v]);
            }
        }
    }
    out
}

/// The value restriction: only *syntactic values* may have their `let`-bound
/// type generalized. Generalizing a non-value (an application, `match`, `if`, …)
/// is unsound once first-class functions or continuations exist — a lambda-bound
/// or continuation-captured cell could escape its monomorphic use. Names, literals,
/// and lambdas are values; everything that *computes* is not. (Slice 4b-1 §2.4.)
fn is_syntactic_value(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Var(_)
            | Expr::Qualified { .. }
            | Expr::Int(_)
            | Expr::Float(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Unit
            | Expr::Lambda { .. }
    )
}

fn generalize_toplevel(
    inf: &mut Infer,
    fnty: &Ty,
    env: &TyEnv,
    group: &[usize],
    fns: &[&FnDecl],
) -> Scheme {
    let group_names: Vec<&str> = group.iter().map(|&i| fns[i].name.as_str()).collect();
    let resolved = inf.resolve(fnty);
    let mut in_ty = Vec::new();
    free_vars(inf, &resolved, &mut in_ty);
    let mut row_in_ty = Vec::new();
    free_row_vars(inf, &resolved, &mut row_in_ty);
    let mut in_env = Vec::new();
    let mut row_in_env = Vec::new();
    for scope in &env.scopes {
        for (n, scheme) in scope {
            if group_names.contains(&n.as_str()) {
                continue;
            }
            let mut fv = Vec::new();
            free_vars(inf, &scheme.ty, &mut fv);
            for v in fv {
                if !scheme.vars.contains(&v) && !in_env.contains(&v) {
                    in_env.push(v);
                }
            }
            let mut rfv = Vec::new();
            free_row_vars(inf, &scheme.ty, &mut rfv);
            for v in rfv {
                if !scheme.row_vars.contains(&v) && !row_in_env.contains(&v) {
                    row_in_env.push(v);
                }
            }
        }
    }
    let vars: Vec<u32> = in_ty.into_iter().filter(|v| !in_env.contains(v)).collect();
    let row_vars: Vec<RowVar> = row_in_ty
        .into_iter()
        .filter(|v| !row_in_env.contains(v))
        .collect();
    Scheme {
        ty: resolved,
        vars,
        row_vars,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_expr_str;
    use crate::span::Span;
    use crate::Session;

    fn infer_expr_str(src: &str) -> (String, usize) {
        let (e, d) = parse_expr_str(&Session::new(), src);
        assert!(d.is_empty(), "parse: {d:?}");
        let e = e.unwrap();
        let mut inf = Infer::new();
        let mut env = TyEnv::new();
        let amb = inf.fresh_row();
        let t = inf.infer_expr(&e, &mut env, amb);
        (display_ty(&inf, &t), inf.diags.len())
    }

    #[test]
    fn infer_flags_linear_let_binding_as_affine() {
        // Slice 4d-2: `infer_with_sites` records exactly the `let t = Tok`
        // binding as affine (its type is a `linear` type), not `let n = 1`.
        let src = "linear type Tok { Tok }\n\
                   pub fn main() {\n\
                     let t = Tok\n\
                     let n = 1\n\
                     io.println(\"x\")\n\
                   }\n";
        let (m, pd) = crate::parse::parse_module(&Session::new(), src);
        assert!(pd.is_empty(), "parse: {pd:?}");
        let (d, sites) = infer_with_sites(&Session::new(), &m);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(
            sites.len(),
            1,
            "exactly the `let t = Tok` binding is affine, not `let n = 1`"
        );
    }

    #[test]
    fn infer_with_types_records_every_node_and_zonks_after_inference() {
        // `x` and `y` are FRESH VARS when first recorded; `y + 1` forces them to
        // Int only later. If the final table shows them as `Int` (not variables),
        // the zonk ran AFTER the SCC loop — proving record-then-zonk ordering, not
        // a record-time resolve. The node count proves the wrapper records every
        // node exactly once (no path bypasses it).
        let (m, pd) =
            crate::parse::parse_module(&Session::new(), "fn ord(x) { let y = x  y + 1 }\n");
        assert!(pd.is_empty(), "{pd:?}");
        let (diags, table) = infer_with_types(&Session::new(), &m);
        assert!(diags.is_empty(), "{diags:?}");
        // Coverage: the four expression nodes `x`, `y`, `1`, `y + 1`, plus the
        // PARAMETER span `x` at 7..8 that the SCC loop records (Slice 5b-3
        // §3.2) — each once. The count is the tripwire: it moved 4 → 5 exactly
        // when parameters started being recorded, and it would move again if a
        // second recording path appeared.
        assert_eq!(
            table.len(),
            5,
            "wrapper must record every node once: {table:?}"
        );
        // Ordering: all five zonked to Int (a record-time table would show vars).
        // The parameter entry is included, which is the whole reason §3.2 could
        // record it pre-zonk: the single zonk pass maps over the WHOLE table.
        assert!(
            table.values().all(|v| v == "Int"),
            "record-then-zonk violated — a node kept its pre-zonk var: {table:?}"
        );
    }

    #[test]
    fn value_restriction_keeps_values_polymorphic() {
        // `Nil` is a syntactic value (a Var / nullary ctor), so a let-bound `Nil`
        // stays polymorphic and unifies at two distinct element types.
        let src = "type List(a) { Nil, Cons(a, List(a)) }\n\
                   pub fn main() {\n\
                     let e = Nil\n\
                     let _ = Cons(1, e)\n\
                     let _ = Cons(\"a\", e)\n\
                     io.println(\"ok\")\n\
                   }\n";
        assert!(
            crate::check_source("t.elya", src).is_ok(),
            "{:?}",
            crate::check_source("t.elya", src)
        );
    }

    #[test]
    fn infers_arithmetic_and_comparison() {
        assert_eq!(infer_expr_str("1 + 2"), ("Int".into(), 0));
        assert_eq!(infer_expr_str("1.0 +. 2.0"), ("Float".into(), 0));
        assert_eq!(infer_expr_str("1 < 2"), ("Bool".into(), 0));
        assert_eq!(infer_expr_str(r#""a" <> "b""#), ("String".into(), 0));
    }

    #[test]
    fn mismatch_in_operator_is_e0400() {
        let (_t, n) = infer_expr_str(r#"1 + "a""#);
        assert_eq!(n, 1);
    }

    #[test]
    fn if_branches_must_agree_and_cond_is_bool() {
        assert_eq!(
            infer_expr_str("if 1 < 2 { 10 } else { 20 }"),
            ("Int".into(), 0)
        );
        let (_t, n) = infer_expr_str("if 1 { 10 } else { 20 }");
        assert_eq!(n, 1);
    }

    #[test]
    fn module_infer_types_a_function() {
        let (m, d) = crate::parse::parse_module(&Session::new(), "fn f(x) { x + 1 }\n");
        assert!(d.is_empty());
        let diags = infer(&Session::new(), &m);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn unifies_equal_bases() {
        let mut inf = Infer::new();
        inf.unify(&Ty::int(), &Ty::int(), Span::EMPTY);
        assert!(inf.diags.is_empty());
    }

    #[test]
    fn mismatched_bases_are_e0400() {
        let mut inf = Infer::new();
        inf.unify(&Ty::int(), &Ty::str(), Span::EMPTY);
        assert_eq!(inf.diags.len(), 1);
        assert_eq!(inf.diags[0].code, "E0400");
    }

    #[test]
    fn binds_and_resolves_a_var() {
        let mut inf = Infer::new();
        let v = inf.fresh();
        inf.unify(&v, &Ty::bool(), Span::EMPTY);
        assert!(inf.diags.is_empty());
        assert_eq!(inf.resolve(&v), Ty::bool());
    }

    #[test]
    fn occurs_check_is_e0401() {
        let mut inf = Infer::new();
        let v = inf.fresh();
        let f = Ty::Fn(vec![v.clone()], EffectRow::pure(), Box::new(Ty::int()));
        inf.unify(&v, &f, Span::EMPTY);
        assert_eq!(inf.diags.len(), 1);
        assert_eq!(inf.diags[0].code, "E0401");
    }

    #[test]
    fn function_arity_mismatch_is_e0402() {
        let mut inf = Infer::new();
        let a = Ty::Fn(vec![Ty::int()], EffectRow::pure(), Box::new(Ty::unit()));
        let b = Ty::Fn(
            vec![Ty::int(), Ty::int()],
            EffectRow::pure(),
            Box::new(Ty::unit()),
        );
        inf.unify(&a, &b, Span::EMPTY);
        assert_eq!(inf.diags.len(), 1);
        assert_eq!(inf.diags[0].code, "E0402");
    }

    #[test]
    fn printer_names_free_vars_with_letters() {
        let mut inf = Infer::new();
        let a = inf.fresh();
        let b = inf.fresh();
        let t = Ty::Fn(vec![a.clone()], EffectRow::pure(), Box::new(b.clone()));
        let out = display_ty(&inf, &t);
        assert_eq!(out, "fn(a) -> b");
        assert!(!out.contains('%'), "no internal token: {out}");
        assert!(
            !out.contains("t0") && !out.contains("t1"),
            "no raw var id: {out}"
        );
    }

    #[test]
    fn pure_fn_prints_without_row() {
        let inf = Infer::new();
        let t = Ty::Fn(vec![Ty::int()], EffectRow::pure(), Box::new(Ty::unit()));
        assert_eq!(display_ty(&inf, &t), "fn(Int) -> Unit");
    }

    #[test]
    fn nonpure_fn_prints_row_and_open_tail() {
        let mut inf = Infer::new();
        let tail = inf.fresh_row();
        let mut labels = std::collections::BTreeMap::new();
        labels.insert(
            "Log".to_string(),
            EffectLabel {
                args: Vec::new(),
                span: Span::EMPTY,
            },
        );
        let row = EffectRow {
            labels,
            tail: RowTail::Open(tail),
        };
        let t = Ty::Fn(vec![], row, Box::new(Ty::unit()));
        // Named label + a polymorphic residual, no `%`/raw-var token.
        let out = display_ty(&inf, &t);
        assert_eq!(out, "fn() / {Log | a} -> Unit");
        assert!(!out.contains('%'), "no internal token: {out}");
    }

    #[test]
    fn printer_reuses_same_letter_for_same_var() {
        let mut inf = Infer::new();
        let a = inf.fresh();
        let t = Ty::Fn(vec![a.clone()], EffectRow::pure(), Box::new(a.clone()));
        assert_eq!(display_ty(&inf, &t), "fn(a) -> a");
    }

    #[test]
    fn printer_resolves_bound_vars() {
        let mut inf = Infer::new();
        let a = inf.fresh();
        inf.unify(&a, &Ty::int(), Span::EMPTY);
        assert_eq!(display_ty(&inf, &a), "Int");
    }

    #[test]
    fn scheme_prints_quantifiers() {
        let mut inf = Infer::new();
        let a = inf.fresh();
        let Ty::Var(av) = a else { unreachable!() };
        let s = Scheme {
            vars: vec![av],
            row_vars: vec![],
            ty: Ty::Fn(vec![a.clone()], EffectRow::pure(), Box::new(a.clone())),
        };
        assert_eq!(display_scheme(&inf, &s), "forall a. fn(a) -> a");
    }

    // ---- Effect-row unification (Task 3) ----

    fn row(labels: &[&str], tail: RowTail) -> EffectRow {
        let mut m = std::collections::BTreeMap::new();
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

    #[test]
    fn an_effect_reaching_a_closed_row_is_reported_not_dropped() {
        // The row-soundness sweep: every perform/call/discharge site used to
        // discard this conflict, and the effect vanished from the type. The
        // helper on its own; `tests/row_soundness.rs` has the programs.
        let mut inf = Infer::new();
        let amb = inf.fresh_row();
        inf.bind_row(amb, &EffectRow::pure(), Span::EMPTY);
        let r = inf.add_effect(amb, "Log", Vec::new(), Span::EMPTY);
        assert!(r.is_err(), "a closed row cannot absorb Log");
        inf.surface_conflict(r);
        assert!(
            inf.diags.iter().any(|d| d.code == "E0423"),
            "the conflict must be a diagnostic: {:?}",
            inf.diags
        );
    }

    #[test]
    fn add_effect_extends_open_row() {
        let mut inf = Infer::new();
        let amb = inf.fresh_row();
        assert!(inf.add_effect(amb, "Log", Vec::new(), Span::EMPTY).is_ok());
        let r = inf.resolve_row(&EffectRow::open(amb));
        assert!(r.labels.contains_key("Log"), "amb should now contain Log");
        assert!(matches!(r.tail, RowTail::Open(_)), "still open/polymorphic");
        assert!(inf.diags.is_empty());
    }

    // ---- Slice 4c-2: effect labels carry type arguments ----

    fn state_row(arg: Ty) -> EffectRow {
        let mut m = std::collections::BTreeMap::new();
        m.insert(
            "State".to_string(),
            EffectLabel {
                args: vec![arg],
                span: Span::EMPTY,
            },
        );
        EffectRow {
            labels: m,
            tail: RowTail::Closed,
        }
    }

    #[test]
    fn unify_row_reconciles_matching_effect_args() {
        // {State(Int)} unifies with {State(Int)} -> ok, no diagnostic.
        let mut inf = Infer::new();
        let a = state_row(Ty::int());
        let b = state_row(Ty::int());
        assert!(inf.unify_row(&a, &b, Span::EMPTY).is_ok());
        assert!(inf.diags.is_empty(), "matching args must not diagnose");
    }

    #[test]
    fn unify_row_rejects_conflicting_effect_args_e0423() {
        // {State(Int)} vs {State(String)} -> E0423 (same effect, different type).
        let mut inf = Infer::new();
        let a = state_row(Ty::int());
        let b = state_row(Ty::str());
        let _ = inf.unify_row(&a, &b, Span::EMPTY);
        assert!(
            inf.diags.iter().any(|d| d.code == "E0423"),
            "State(Int) vs State(String) must be E0423: {:?}",
            inf.diags
        );
    }

    #[test]
    fn two_open_rows_reconcile_by_mutual_extension() {
        let mut inf = Infer::new();
        let a = inf.fresh_row();
        let b = inf.fresh_row();
        let r1 = row(&["Log"], RowTail::Open(a));
        let r2 = row(&["Net"], RowTail::Open(b));
        assert!(inf.unify_row(&r1, &r2, Span::EMPTY).is_ok());
        let z1 = inf.resolve_row(&r1);
        let z2 = inf.resolve_row(&r2);
        // Both rows now carry both labels — no "neither is a subset" failure.
        for z in [&z1, &z2] {
            assert!(z.labels.contains_key("Log") && z.labels.contains_key("Net"));
        }
        assert!(inf.diags.is_empty());
    }

    #[test]
    fn closed_row_meeting_extra_is_conflict_not_diag() {
        let mut inf = Infer::new();
        let r1 = row(&["Log"], RowTail::Closed);
        let r2 = row(&["Log", "Net"], RowTail::Closed);
        let err = inf.unify_row(&r1, &r2, Span::EMPTY).unwrap_err();
        // `Net` is in r2 and cannot be absorbed by the closed r1.
        let only2: Vec<&str> = err.only2.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(only2, ["Net"]);
        assert!(err.only1.is_empty());
        assert!(
            inf.diags.is_empty(),
            "unify_row must not push diagnostics for label conflicts"
        );
    }

    #[test]
    fn unknown_op_type_is_e0432() {
        // An unknown type name in an operation signature is the unified E0432
        // "unknown type" (Slice 4c-2: operation signatures elaborate via the
        // general type elaborator; the old E0404 "generic effect ops not supported
        // yet" is obsolete now that parametric effects exist).
        let (m, _) =
            crate::parse::parse_module(&Session::new(), "effect E { fn op(x: Foo) -> Unit }\n");
        let (_s, d) = infer_schemes(&Session::new(), &m);
        assert!(d.iter().any(|x| x.code == "E0432"), "expected E0432: {d:?}");
    }

    #[test]
    fn cyclic_row_is_e0424() {
        // Directly exercise the occurs-check code path: bind σ := {A} | Open(σ),
        // a row that would contain itself. (Unconstructible from 3b surface
        // syntax, so the primitive's unit test is its coverage — plan 5b.)
        let mut inf = Infer::new();
        let s = inf.fresh_row();
        let cyclic = row(&["A"], RowTail::Open(s));
        inf.bind_row(s, &cyclic, Span::EMPTY);
        assert_eq!(inf.diags.len(), 1);
        assert_eq!(inf.diags[0].code, "E0424");
        // σ is poisoned to ErrorRow so no cascade follows.
        assert_eq!(inf.resolve_row(&EffectRow::open(s)).tail, RowTail::ErrorRow);
    }
}
