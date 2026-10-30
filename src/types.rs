//! Hindley–Milner type inference (Algorithm J).

use crate::ast::{
    BinOp, Block, Decl, Expr, FnDecl, Handler, Module, PatLit, Pattern, Stmt, TypeAnn, UnOp,
};
use crate::diag::Diagnostic;
use crate::span::{Span, Spanned};
use crate::Session;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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

/// A function's annotation names: its type variables and its row variables.
type AnnNames = (HashMap<String, Ty>, HashMap<String, RowVar>);

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
    /// Per effect parameter, its row variable when it is a row parameter (spec
    /// 2026-10-30-elya-row-variables D3, D4): `params`/`ret` mention it, and the
    /// label's argument there is its phantom.
    row_params: Vec<Option<RowVar>>,
    params: Vec<Ty>,
    ret: Ty,
}

/// Every declaration's parameters, each with its kind: `true` for a row
/// parameter (spec 2026-10-30-elya-row-variables D3). Types and effects by name.
#[derive(Clone, Debug, Default)]
pub struct Kinds {
    pub types: HashMap<String, Vec<(String, bool)>>,
    pub effects: HashMap<String, Vec<(String, bool)>>,
}

impl Kinds {
    /// Whether type `name`'s parameter `i` is a row parameter.
    pub fn type_row(&self, name: &str, i: usize) -> bool {
        self.types
            .get(name)
            .and_then(|k| k.get(i))
            .is_some_and(|k| k.1)
    }
    /// Whether effect `name`'s parameter `i` is a row parameter.
    pub fn effect_row(&self, name: &str, i: usize) -> bool {
        self.effects
            .get(name)
            .and_then(|k| k.get(i))
            .is_some_and(|k| k.1)
    }
    fn param_name<'a>(
        map: &'a HashMap<String, Vec<(String, bool)>>,
        name: &str,
        i: usize,
    ) -> &'a str {
        map.get(name)
            .and_then(|k| k.get(i))
            .map(|k| k.0.as_str())
            .unwrap_or("?")
    }
}

/// Each declaration's parameters' kinds (spec 2026-10-30-elya-row-variables D3):
/// a parameter is a row parameter when its declaration uses it where a row goes
/// -- a row's tail, or the argument at another declaration's row parameter. A
/// fixpoint over every declaration: a parameter only turns from type to row.
pub fn decl_kinds(module: &Module) -> Kinds {
    let mut k = Kinds::default();
    for d in &module.decls {
        match &d.node {
            Decl::Type(t) => {
                let ps = t.params.iter().map(|p| (p.clone(), false)).collect();
                k.types.insert(t.name.clone(), ps);
            }
            Decl::Effect(e) => {
                let ps = e.params.iter().map(|p| (p.clone(), false)).collect();
                k.effects.insert(e.name.clone(), ps);
            }
            Decl::Fn(_) => {}
        }
    }
    loop {
        let mut changed = false;
        for d in &module.decls {
            let mut rows = HashSet::new();
            match &d.node {
                Decl::Type(t) => {
                    for v in &t.variants {
                        for f in &v.node.fields {
                            row_uses(&f.node, &t.params, &k, &mut rows);
                        }
                    }
                    let ps = k.types.get_mut(&t.name).expect("registered above");
                    changed |= mark_rows(ps, &rows);
                }
                Decl::Effect(e) => {
                    for op in &e.ops {
                        for a in op
                            .node
                            .param_tys
                            .iter()
                            .chain(std::iter::once(&op.node.ret))
                        {
                            row_uses(&a.node, &e.params, &k, &mut rows);
                        }
                    }
                    let ps = k.effects.get_mut(&e.name).expect("registered above");
                    changed |= mark_rows(ps, &rows);
                }
                Decl::Fn(_) => {}
            }
        }
        if !changed {
            return k;
        }
    }
}

fn mark_rows(ps: &mut [(String, bool)], rows: &HashSet<String>) -> bool {
    let mut changed = false;
    for (p, is_row) in ps.iter_mut() {
        if !*is_row && rows.contains(p.as_str()) {
            *is_row = true;
            changed = true;
        }
    }
    changed
}

/// The parameters (of `params`) that `a` uses where a row goes.
fn row_uses(a: &TypeAnn, params: &[String], k: &Kinds, out: &mut HashSet<String>) {
    let bare = |x: &TypeAnn, out: &mut HashSet<String>| {
        if x.args.is_empty() && x.row.is_none() && params.contains(&x.name) {
            out.insert(x.name.clone());
        }
    };
    if let Some(row) = &a.row {
        if let Some(t) = &row.tail {
            if params.contains(&t.node) {
                out.insert(t.node.clone());
            }
        }
        for l in &row.labels {
            for (j, x) in l.args.iter().flatten().enumerate() {
                if k.effect_row(&l.name.node, j) {
                    bare(&x.node, out);
                }
                row_uses(&x.node, params, k, out);
            }
        }
    }
    for (j, x) in a.args.iter().enumerate() {
        if a.name != "fn" && k.type_row(&a.name, j) {
            bare(&x.node, out);
        }
        row_uses(&x.node, params, k, out);
    }
}

/// A row argument as a type (spec 2026-10-30-elya-row-variables D4): the
/// phantom function type `fn() / R -> $Row`.
pub fn phantom(row: EffectRow) -> Ty {
    Ty::Fn(
        Vec::new(),
        row,
        Box::new(Ty::Con(crate::ast::ROW_RESULT.to_string(), Vec::new())),
    )
}

/// The row a row argument's phantom carries.
pub fn phantom_row(t: &Ty) -> Option<&EffectRow> {
    match t {
        Ty::Fn(ps, row, r)
            if ps.is_empty()
                && matches!(&**r, Ty::Con(n, a) if n == crate::ast::ROW_RESULT && a.is_empty()) =>
        {
            Some(row)
        }
        _ => None,
    }
}

/// A resolved type's head, for `unify_effect_args`: its constructor and arity.
fn head(t: &Ty) -> (u8, String, usize) {
    match t {
        Ty::Var(_) => (0, String::new(), 0),
        Ty::Base(b) => (1, format!("{b:?}"), 0),
        Ty::Fn(ps, _, _) => (2, String::new(), ps.len()),
        Ty::Tuple(xs) => (3, String::new(), xs.len()),
        Ty::Con(n, a) => (4, n.clone(), a.len()),
        Ty::Error => (5, String::new(), 0),
    }
}

/// Whether a resolved type has no variable in it (type or row).
fn ground(t: &Ty) -> bool {
    match t {
        Ty::Var(_) | Ty::Error => false,
        Ty::Base(_) => true,
        Ty::Fn(ps, row, r) => {
            matches!(row.tail, RowTail::Closed)
                && row.labels.values().all(|l| l.args.iter().all(ground))
                && ps.iter().all(ground)
                && ground(r)
        }
        Ty::Tuple(xs) | Ty::Con(_, xs) => xs.iter().all(ground),
    }
}

/// D5's messages, shared by both elaborators.
fn wants_row(decl: &str, param: &str) -> String {
    format!("`{decl}`'s parameter `{param}` is a row: write one, `{{Log}}`")
}
fn wants_type(decl: &str, param: &str) -> String {
    format!("`{decl}`'s parameter `{param}` is a type, not a row")
}
fn row_not_here() -> &'static str {
    "a row is written only where a type or an effect takes one"
}

/// What a `resume` inside a clause needs (resume-row fix, 2026-10-08, and
/// its relay and return-clause half, 2026-10-09).
#[derive(Clone)]
struct ResumeCx {
    /// The operation's result type: `resume`'s argument.
    arg: Ty,
    /// The handle's result type: `resume`'s result.
    result: Ty,
    /// The handled body's ambient. A resume runs the rest of the body, so its
    /// row includes the body's effects except the handled one.
    body_amb: RowVar,
    /// The return clause's ambient: the rest of the body ends in the return
    /// clause, so a resume performs its effects too.
    ret_amb: RowVar,
    /// The handled effect, which the resumed handler handles again.
    handled: Option<String>,
    /// A `with any` handle (2026-10-16): the resumed handler handles every
    /// effect of the body but `IO`, its row's tail included.
    handles_all: bool,
    /// The lambda depth at the clause: a resume at a greater depth is inside a
    /// lambda, which may run after the clause has returned (it ESCAPES).
    lambda_depth: usize,
    /// The handle's "every clause" row: an escaped resume's ambient takes it as
    /// its tail, and `infer_handle` fills it with every clause's effects once
    /// all clauses are typed (the resumed body may re-enter any clause).
    clauses_row: RowVar,
}

/// A row a `with any` handler handles, queued for E0434 (2026-10-16, D5).
struct GenericSite {
    /// The labels the handled body performs, as known at the handle (empty for
    /// an instantiation): always checked.
    labels: BTreeMap<String, EffectLabel>,
    /// The handled row's tail (or an instantiated copy of a generalized one).
    tail: EffectRow,
    /// Whether the handler performs its operations again (`op.perform`): its
    /// tail is then its function's own row, which a call unifies with the
    /// caller's ambient, so labels arriving through it are not the body's and
    /// are left to the run-time check.
    forwarded: bool,
    span: Span,
}

/// A declared type's parameter variables and each constructor's field types.
type AdtFields = (Vec<u32>, Vec<Vec<Ty>>);

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
    /// Recursion through a handle body (2026-10-15): `(member ambient, handle
    /// body ambient)` -- a call of a function of the SCC group being inferred,
    /// made directly in a handle's body, performs the member's row, so the body
    /// must INCLUDE its labels; unifying the two rows (as a call otherwise
    /// does) made the member's row EQUAL to the body's, which holds the handled
    /// effect, and the handled effect leaked into the function's type.
    /// Flushed with `pending_incl`, labels only.
    pending_member_incl: Vec<(RowVar, RowVar, Span)>,
    /// The ambients of the SCC group's members while the group is inferred.
    group_ambs: Vec<RowVar>,
    /// The parameter types of the group's members.
    group_params: Vec<Ty>,
    /// The regions being inferred, innermost last: a handle body or a lambda
    /// body (`true`: a group member called directly in it is INCLUDED, not
    /// unified), or a clause or return-clause body (`false`).
    regions: Vec<(RowVar, bool)>,
    /// Lambda ambients that include a group member's row (recursion through a
    /// lambda, 2026-10-15): their labels are complete only when the group is,
    /// so they are neither closed at the lambda's end nor generalized, and are
    /// closed with the group.
    deferred_rows: Vec<(RowVar, Vec<Ty>)>,
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
    resume_stack: Vec<ResumeCx>,
    /// The `clauses_row` of every handle whose clauses are being typed. Such a
    /// row is filled only when its handle is done, so until then it is neither
    /// closed (`close_unrelayed_residual`) nor generalized (`generalize`).
    pending_clause_rows: Vec<RowVar>,
    /// How many lambdas enclose the expression being typed.
    lambda_depth: usize,
    /// Constructor name -> arity. An n-ary constructor used unapplied or
    /// partially applied is `E0433` (unapplied constructors need 4b's closures).
    ctor_arity: HashMap<String, usize>,
    /// Effect name -> whether it is declared `multi` (Slice 4d-1). Read by the
    /// `with multi` conformance rule; a declaration fact, NOT a row attribute.
    effect_multi: HashMap<String, bool>,
    /// Names of `linear`-declared types (Slice 4d-2) — their values are affine.
    linear_types: HashSet<String>,
    /// Names of declared types with no constructors (2026-10-16): the only
    /// scrutinee types an empty `match` may have.
    empty_types: HashSet<String>,
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
    /// The current top-level function's named row variables (spec
    /// 2026-10-30-elya-row-variables D2): `e` in `/ {e}` or `/ {Log | e}`, one
    /// row for the whole function, as a lowercase type name is one type.
    ann_rows: HashMap<String, RowVar>,
    /// The answer types of `with any` clauses (2026-10-16): rigid, one per
    /// clause, as `Ty::Con` names that no declaration can spell. Each with its
    /// clause's span, for E0435 when one escapes.
    skolems: Vec<(String, Span)>,
    /// E0434 (2026-10-16): row variables handled by a `with any` handler. A
    /// scheme generalizing one queues every instantiation's copy for the data
    /// check (which marks the copy's own tail in turn, if it is still open).
    generic_rows: HashSet<RowVar>,
    /// Rows a `with any` handler handles, with the span to blame: checked for
    /// data (`check_generic_rows`) when their binding group is solved.
    pending_generic: Vec<GenericSite>,
    /// The span of the expression being inferred (an instantiation's site).
    cur_span: Span,
    /// Declared type -> its parameters' variables and each constructor's
    /// field types, for the data check.
    adt_fields: HashMap<String, AdtFields>,
    /// Equality type variables (structural equality, 2026-10-16, D1): each
    /// must become data, because a `==` compares values of it. Binding one
    /// checks what it is bound to (marking variables in it); instantiating a
    /// scheme that quantifies one marks the fresh copy.
    /// Each with the span of the `==` that marked it, named in the error.
    eq_vars: HashMap<u32, Span>,
    /// The `==` sites already reported, so a site is blamed once.
    eq_reported: HashSet<Span>,
    /// Every declaration's parameters and their kinds (spec
    /// 2026-10-30-elya-row-variables D3).
    kinds: Kinds,
    pub diags: Vec<Diagnostic>,
}

impl Infer {
    pub fn new() -> Infer {
        Infer {
            subst: Vec::new(),
            row_subst: Vec::new(),
            ops: HashMap::new(),
            resume_stack: Vec::new(),
            pending_clause_rows: Vec::new(),
            lambda_depth: 0,
            phantom: HashSet::new(),
            pending_incl: Vec::new(),
            pending_member_incl: Vec::new(),
            group_ambs: Vec::new(),
            group_params: Vec::new(),
            regions: Vec::new(),
            deferred_rows: Vec::new(),
            param_tys: Vec::new(),
            ctor_arity: HashMap::new(),
            effect_multi: HashMap::new(),
            linear_types: HashSet::new(),
            empty_types: HashSet::new(),
            affine_sites: HashSet::new(),
            node_types: HashMap::new(),
            known_types: HashMap::new(),
            ann_vars: HashMap::new(),
            ann_rows: HashMap::new(),
            skolems: Vec::new(),
            generic_rows: HashSet::new(),
            pending_generic: Vec::new(),
            cur_span: Span::EMPTY,
            adt_fields: HashMap::new(),
            eq_vars: HashMap::new(),
            eq_reported: HashSet::new(),
            kinds: Kinds::default(),
            diags: Vec::new(),
        }
    }

    /// The function's row variable named `name` (D2): the same one at every
    /// mention, a meaningful (not phantom) variable.
    fn ann_row(&mut self, name: &str, span: Span) -> RowVar {
        if let Some(v) = self.ann_rows.get(name) {
            return *v;
        }
        // Checked where a name first appears (the review's F5): an effect's
        // name in lowercase is a typo, not a variable; and one name is a type or
        // a row, not both.
        let mut cap = name.to_string();
        if let Some(c) = cap.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        if self.effect_multi.contains_key(&cap) || cap == "IO" {
            self.diags.push(
                Diagnostic::error(
                    "E0432",
                    format!("row variable `{name}` is spelled like an effect"),
                )
                .with_label(span, "a row variable")
                .with_help(format!(
                    "write `{cap}` for the effect, or give the row variable another name"
                )),
            );
        } else if self.ann_vars.contains_key(name) {
            self.diags.push(
                Diagnostic::error("E0400", format!("`{name}` names a type and a row"))
                    .with_label(span, "a row variable here")
                    .with_help(
                        "a lowercase name in a row is a row; elsewhere it is a type: use two names",
                    ),
            );
        }
        let v = self.fresh_row();
        self.ann_rows.insert(name.to_string(), v);
        v
    }

    /// A type annotation as a type (2026-10-09; annotations used to be parsed
    /// and discarded). Base and declared names as in an ADT field; a lowercase
    /// name is the function's shared variable of that name -- FLEXIBLE: it
    /// says "the same type here and there", not "for every type" (no rigid
    /// skolems); `fn(A) / {E} -> R` is a function type whose row, when none is
    /// written, is open (any effects), and when written is exactly those.
    fn elaborate_ann(&mut self, ann: &Spanned<TypeAnn>, covariant: bool) -> Ty {
        let t = &ann.node;
        if t.as_row_arg().is_some() {
            // A row where a type goes (a type's or an effect's row argument is
            // read by `ann_row_arg`).
            self.diags.push(
                Diagnostic::error("E0400", row_not_here()).with_label(ann.span, "a row here"),
            );
            return Ty::Error;
        }
        self.elaborate_ann_inner(ann, covariant)
    }

    fn elaborate_ann_inner(&mut self, ann: &Spanned<TypeAnn>, covariant: bool) -> Ty {
        let t = &ann.node;
        if t.name == crate::ast::ROW_RESULT && t.args.is_empty() {
            return Ty::Con(t.name.clone(), Vec::new());
        }
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
                Some(row) => {
                    let labels = self.ann_labels(row);
                    EffectRow {
                        labels,
                        tail: match &row.tail {
                            Some(v) => RowTail::Open(self.ann_row(&v.node, v.span)),
                            None => RowTail::Closed,
                        },
                    }
                }
            };
            return Ty::Fn(params, row, Box::new(ret));
        }
        if t.args.is_empty() && t.name.starts_with(|c: char| c.is_ascii_lowercase()) {
            if let Some(v) = self.ann_vars.get(&t.name) {
                return v.clone();
            }
            if self.ann_rows.contains_key(&t.name) {
                self.diags.push(
                    Diagnostic::error("E0400", format!("`{}` names a type and a row", t.name))
                        .with_label(ann.span, "a type here")
                        .with_help("a lowercase name in a row is a row; elsewhere it is a type: use two names"),
                );
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
                    // Type arguments are invariant: no upcast tails inside. A
                    // row parameter's argument is a row (D5).
                    let mut args = Vec::with_capacity(t.args.len());
                    for (j, a) in t.args.iter().enumerate() {
                        let param = Kinds::param_name(&self.kinds.types, other, j).to_string();
                        args.push(if self.kinds.type_row(other, j) {
                            self.ann_row_arg(a, other, &param)
                        } else {
                            self.ann_type_arg(a, other, &param)
                        });
                    }
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

    /// A row's labels in a function's annotations, each with its arguments: the
    /// written ones elaborated (D5: they used to be dropped), unwritten ones
    /// fresh.
    fn ann_labels(&mut self, row: &crate::ast::RowAnn) -> BTreeMap<String, EffectLabel> {
        let mut m = BTreeMap::new();
        for l in &row.labels {
            let name = &l.name.node;
            if !self.effect_multi.contains_key(name) && name != "IO" {
                self.diags.push(
                    Diagnostic::error("E0432", format!("unknown effect `{name}`"))
                        .with_label(l.name.span, "no such effect"),
                );
                continue;
            }
            let arity = self.kinds.effects.get(name).map(|k| k.len()).unwrap_or(0);
            let args = match &l.args {
                None => (0..arity).map(|_| self.fresh()).collect(),
                Some(xs) if xs.len() != arity => {
                    self.diags.push(
                        Diagnostic::error(
                            "E0400",
                            format!(
                                "effect `{name}` takes {arity} argument(s), found {}",
                                xs.len()
                            ),
                        )
                        .with_label(l.name.span, "wrong number of arguments"),
                    );
                    continue;
                }
                Some(xs) => {
                    let mut args = Vec::with_capacity(xs.len());
                    for (j, x) in xs.iter().enumerate() {
                        let param = Kinds::param_name(&self.kinds.effects, name, j).to_string();
                        args.push(if self.kinds.effect_row(name, j) {
                            self.ann_row_arg(x, name, &param)
                        } else {
                            self.ann_type_arg(x, name, &param)
                        });
                    }
                    args
                }
            };
            m.insert(
                name.clone(),
                EffectLabel {
                    args,
                    span: l.name.span,
                },
            );
        }
        m
    }

    /// A row parameter's argument in a function's annotations (D5): a row
    /// (`{Log | e}`, its phantom), or a row variable alone (`e`).
    fn ann_row_arg(&mut self, a: &Spanned<TypeAnn>, decl: &str, param: &str) -> Ty {
        let t = &a.node;
        if t.as_row_arg().is_some() {
            return self.elaborate_ann_inner(a, false);
        }
        if t.args.is_empty()
            && t.row.is_none()
            && t.name.starts_with(|c: char| c.is_ascii_lowercase())
        {
            let v = self.ann_row(&t.name, a.span);
            return phantom(EffectRow::open(v));
        }
        self.diags.push(
            Diagnostic::error("E0400", wants_row(decl, param)).with_label(a.span, "a type here"),
        );
        Ty::Error
    }

    /// A type parameter's argument in a function's annotations: a type, not a
    /// row (D5).
    fn ann_type_arg(&mut self, a: &Spanned<TypeAnn>, decl: &str, param: &str) -> Ty {
        if a.node.as_row_arg().is_some() {
            self.diags.push(
                Diagnostic::error("E0400", wants_type(decl, param))
                    .with_label(a.span, "a row here"),
            );
            return Ty::Error;
        }
        self.elaborate_ann(a, false)
    }

    /// A function's declared row's written label arguments (spec
    /// 2026-10-30-elya-row-variables D5), elaborated where its annotation names
    /// are live; unified with what the body performs before anything closes
    /// (2.47). An unknown effect is reported by the row's own checks.
    fn declared_label_args(&mut self, row: &crate::ast::RowAnn) -> Vec<(String, Vec<Ty>, Span)> {
        let written = crate::ast::RowAnn {
            labels: row
                .labels
                .iter()
                .filter(|l| l.args.is_some() && self.kinds.effects.contains_key(&l.name.node))
                .cloned()
                .collect(),
            tail: None,
        };
        self.ann_labels(&written)
            .into_iter()
            .map(|(n, l)| (n, l.args, l.span))
            .collect()
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

    /// One effect met twice along a row's chain (a label, and the same label
    /// inside a binding of its tail): `resolve_row` keeps the first, so their
    /// arguments are unified here, where rows meet. A label kept out of a
    /// row variable its arguments mention (`add_row`, spec
    /// 2026-10-30-elya-row-variables, the part 2 review's F3) can reach that
    /// variable later by another path: dropped, `E(Int)` beside
    /// `E(fn() -> Int)` checked clean.
    fn settle_duplicates(&mut self, r: &EffectRow, span: Span) {
        let mut seen: BTreeMap<String, Vec<Ty>> = r
            .labels
            .iter()
            .map(|(k, l)| (k.clone(), l.args.clone()))
            .collect();
        let mut pairs: Vec<(String, Vec<Ty>, Vec<Ty>)> = Vec::new();
        let mut tail = r.tail.clone();
        while let RowTail::Open(v) = tail {
            let Some(bound) = self.row_subst[v as usize].clone() else {
                break;
            };
            for (k, l) in &bound.labels {
                match seen.get(k) {
                    Some(first) => {
                        let differ = first.len() == l.args.len()
                            && first
                                .iter()
                                .zip(&l.args)
                                .any(|(a, b)| self.resolve(a) != self.resolve(b));
                        if differ {
                            pairs.push((k.clone(), first.clone(), l.args.clone()));
                        }
                    }
                    None => {
                        seen.insert(k.clone(), l.args.clone());
                    }
                }
            }
            tail = bound.tail;
        }
        for (k, a, b) in pairs {
            self.unify_effect_args(&k, &a, &b, span);
        }
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
        // live in a separate union-find with its own occurs-check (Task 3). A
        // row's labels carry types, though: `t := fn() / {E(t)} -> Int` contains
        // itself through one (spec 2026-10-30-elya-occurs-in-rows D1; it was
        // bound, and `resolve` recursed until the stack ran out).
        // `resolve` is deep (rows and their labels' arguments included), so
        // one call and a plain walk of its result decide it: resolving again
        // at every level was polynomial in a type's depth (the review's F3:
        // 8.4 s became 43.8 s on a hundred nested performs).
        fn walk(v: u32, t: &Ty) -> bool {
            match t {
                Ty::Var(u) => *u == v,
                Ty::Base(_) | Ty::Error => false,
                Ty::Fn(ps, row, r) => {
                    ps.iter().any(|p| walk(v, p))
                        || walk(v, r)
                        || row
                            .labels
                            .values()
                            .any(|l| l.args.iter().any(|a| walk(v, a)))
                }
                Ty::Tuple(xs) | Ty::Con(_, xs) => xs.iter().any(|x| walk(v, x)),
            }
        }
        walk(v, &self.resolve(t))
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
        if let Some(origin) = self.eq_vars.get(&v).copied() {
            self.require_data(t, span, Some(origin));
        }
    }

    /// E0436 (structural equality, D1): `t` is compared with `==`, so it must be
    /// data. Checked eagerly: a variable in it is MARKED (`eq_vars`) and checked
    /// again when it is bound, and wherever a scheme quantifying it is
    /// instantiated, so a generic comparison is decided at its uses.
    /// `origin`: the `==` that marked the variable being bound, or `None` at
    /// the `==` itself.
    fn require_data(&mut self, t: &Ty, span: Span, origin: Option<Span>) {
        let marker = origin.unwrap_or(span);
        if let Some(why) = self.eq_not_data(t, marker) {
            if self.eq_reported.insert(span) {
                let mut d =
                    Diagnostic::error("E0436", "only data can be compared with `==` or `!=`");
                d = match origin {
                    None => d.with_label(span, format!("this compares {why}")),
                    Some(o) => d
                        .with_label(span, format!("this makes a comparison compare {why}"))
                        .with_label(o, "the comparison it reaches"),
                };
                self.diags.push(
                    d
                        .with_help("data is `Int`, `Float`, `Bool`, `String`, `Unit` and declared types whose fields are data; a function has no equality (two functions that agree on every argument cannot be told apart by running them)"),
                );
            }
        }
    }

    /// Why `t` is not data, or `None`; marks the variables in it (D1) with
    /// the comparison `origin`.
    fn eq_not_data(&mut self, t: &Ty, origin: Span) -> Option<String> {
        match self.resolve(t) {
            Ty::Base(_) | Ty::Error => None,
            Ty::Var(v) => {
                self.eq_vars.entry(v).or_insert(origin);
                None
            }
            Ty::Fn(..) => Some("functions".to_string()),
            Ty::Tuple(xs) => xs.iter().find_map(|x| self.eq_not_data(x, origin)),
            // An operation caught by `with any` is data: E0434 requires every
            // operation such a handler handles to take data. Its answer-type
            // argument is an index, not a field (N3: two operations compare by
            // what they are).
            Ty::Con(n, _) if n == "$Op" => None,
            Ty::Con(n, args) => {
                let mut why = None;
                for a in &args {
                    if let Some(w) = self.eq_not_data(a, origin) {
                        why.get_or_insert(w);
                    }
                }
                why.or_else(|| {
                    self.decl_not_data(&n, &mut Vec::new())
                        .map(|w| format!("values of `{n}`, which holds {w}"))
                })
            }
        }
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
        let (given1, given2) = (r1, r2);
        self.settle_duplicates(given1, span);
        self.settle_duplicates(given2, span);
        let mut r1 = self.resolve_row(given1);
        let mut r2 = self.resolve_row(given2);
        // A label present on BOTH sides must agree on its type arguments (Slice
        // 4c-2, design B). Monomorphic effects have empty args, so this is a no-op
        // — exactly the Slice-3 behavior. A conflicting instantiation is `E0423`.
        // Unifying them can bind either row's tail (a row argument's phantom
        // binds the other row's own tail, spec 2026-10-30-elya-row-variables,
        // the part 2 review's F1), so both rows are read again until no label
        // is newly shared: split as they were, a label was lost (`St(String)`
        // beside `St(Int)`) or never matched.
        let mut unified: BTreeSet<String> = BTreeSet::new();
        loop {
            // A poison tail on either side absorbs everything — no cascade.
            if r1.tail == RowTail::ErrorRow || r2.tail == RowTail::ErrorRow {
                return Ok(());
            }
            let shared: Vec<(String, Vec<Ty>, Vec<Ty>)> = r1
                .labels
                .iter()
                .filter(|(k, _)| !unified.contains(*k))
                .filter_map(|(k, l1)| {
                    r2.labels
                        .get(k)
                        .map(|l2| (k.clone(), l1.args.clone(), l2.args.clone()))
                })
                .collect();
            if shared.is_empty() {
                break;
            }
            for (effect, a1, a2) in shared {
                self.unify_effect_args(&effect, &a1, &a2, span);
                unified.insert(effect);
            }
            r1 = self.resolve_row(given1);
            r2 = self.resolve_row(given2);
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
        // Each tail read again after the other side's absorb: when both rows end
        // in one variable (`{Log | e}` against `{e}`), the first absorb binds it,
        // and the stale tail was then rebound, dropping the label -- a program
        // that checked clean failed at run time (spec
        // 2026-10-30-elya-row-variables, the review's F1; older, but row
        // variables made it writable). Rows are sets, so `e := {Log | f}` solves it.
        let r1_tail = self.tail_now(&r1.tail);
        let t1 = self.absorb(&only2, &r1_tail, span, &mut conflict.only2);
        if !conflict.only1.is_empty() || !conflict.only2.is_empty() {
            return Err(conflict);
        }
        // The two residual tails must be equal.
        let t2 = self.tail_now(&t2);
        self.unify_tails(&t1, &t2, span);
        Ok(())
    }

    /// `tail` as it stands now: through any binding made since it was read.
    fn tail_now(&self, tail: &RowTail) -> RowTail {
        self.resolve_row(&EffectRow {
            labels: BTreeMap::new(),
            tail: tail.clone(),
        })
        .tail
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

    /// Whether `t` mentions row variable `v`: in a function type's row tail, or
    /// in a row's labels' arguments, at any depth.
    fn mentions_row(&self, v: RowVar, t: &Ty) -> bool {
        // One deep `resolve`, then a plain walk (as in `occurs`).
        fn walk(v: RowVar, t: &Ty) -> bool {
            match t {
                Ty::Var(_) | Ty::Base(_) | Ty::Error => false,
                Ty::Fn(ps, row, r) => {
                    row.tail == RowTail::Open(v)
                        || row
                            .labels
                            .values()
                            .any(|l| l.args.iter().any(|a| walk(v, a)))
                        || ps.iter().any(|p| walk(v, p))
                        || walk(v, r)
                }
                Ty::Tuple(xs) | Ty::Con(_, xs) => xs.iter().any(|x| walk(v, x)),
            }
        }
        walk(v, &self.resolve(t))
    }

    /// Bind row variable `v := r`, with an occurs-check (`E0424`): `v` may not
    /// appear in `r`'s tail, or in its labels' arguments, or the row would
    /// contain itself.
    fn bind_row(&mut self, v: RowVar, r: &EffectRow, span: Span) {
        // A bound variable is never overwritten (that drops what it holds):
        // unify with what it holds instead (the row-variables review's F1).
        if self.row_subst[v as usize].is_some() {
            let r2 = self.unify_row(&EffectRow::open(v), r, span);
            self.surface_conflict(r2);
            return;
        }
        let resolved = self.resolve_row(r);
        // Also through a label's arguments: `v := {E(fn() / {.. | v} -> ..)}`
        // (spec 2026-10-30-elya-occurs-in-rows D2).
        if resolved.tail == RowTail::Open(v)
            || resolved
                .labels
                .values()
                .any(|l| l.args.iter().any(|a| self.mentions_row(v, a)))
        {
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
                // A poisoned row (a reported conflict, E0424) absorbs every
                // label: nothing to add, so it is settled. Adding to it
                // "succeeded" every pass, and the fixpoint never ended (spec
                // 2026-10-30-elya-occurs-in-rows, the review's F1).
                if have.tail == RowTail::ErrorRow {
                    continue;
                }
                let want = self.resolve_row(&EffectRow::open(src));
                // A recorded tail that has since become MEANINGFUL (it aliased
                // a real row variable, e.g. a parameter's) is a relay after
                // all: the ambient must take whatever that row takes later,
                // which labels alone cannot say (the sub-effecting review's
                // F1: `f` was typed pure though it calls its parameter `k`).
                if let RowTail::Open(w) = want.tail {
                    if !self.phantom.contains(&w) && have.tail != RowTail::Open(w) {
                        // Through `add_row`: an including row whose labels
                        // mention `w` keeps them its own (the part 2 review's
                        // F2: a state holding a function, `St(fn(Int) -> Int)`,
                        // called; equated with `w`, `w` held `St(.. w ..)`).
                        let r = self.add_row(dst, &EffectRow::open(w), span);
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
            let mut relayed = Vec::new();
            for p in self.group_params.clone() {
                free_row_vars(self, &p, &mut relayed);
            }
            for (src, dst, span) in self.pending_member_incl.clone() {
                let have = self.resolve_row(&EffectRow::open(dst));
                // Settled when poisoned, as above.
                if have.tail == RowTail::ErrorRow {
                    continue;
                }
                let want = self.resolve_row(&EffectRow::open(src));
                // A member's tail that has since become a RELAY of a member's
                // parameter is whatever the arguments perform: the including
                // row must take it too, as a relay is taken anywhere (the
                // lambda review's F1: a lambda closed pure while the member
                // relayed `k`, and an escaped resume missed it, F1b).
                if let RowTail::Open(w) = want.tail {
                    if relayed.contains(&w) && have.tail != RowTail::Open(w) {
                        let r = self.unify_row(&EffectRow::open(dst), &EffectRow::open(w), span);
                        if r.is_ok() {
                            changed = true;
                        } else {
                            self.pending_member_incl.retain(|p| *p != (src, dst, span));
                        }
                        self.surface_conflict(r);
                    }
                }
                for (label, l) in want.labels.iter() {
                    if let Some(h) = have.labels.get(label) {
                        // The same effect on both sides: its type arguments
                        // must agree (the review's F1 -- skipping this let a
                        // member's `St(String)` hide under the handled
                        // `St(Bool)`). A conflict is reported once.
                        let before = self.diags.len();
                        self.unify_effect_args(label, &h.args.clone(), &l.args.clone(), span);
                        if self.diags.len() > before {
                            self.pending_member_incl.retain(|p| *p != (src, dst, span));
                        }
                        continue;
                    }
                    let r = self.add_effect(dst, label, l.args.clone(), span);
                    if r.is_ok() {
                        changed = true;
                    } else {
                        self.pending_member_incl.retain(|p| *p != (src, dst, span));
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
        let (args, m, rm) = self.fresh_effect_args(op);
        let params = op.params.iter().map(|t| subst_vars(t, &m, &rm)).collect();
        let ret = subst_vars(&op.ret, &m, &rm);
        (args, params, ret)
    }

    /// Fresh arguments for an operation's effect: a type variable per type
    /// parameter, a fresh row's phantom per row parameter (spec
    /// 2026-10-30-elya-row-variables D4), and the substitutions that take the
    /// declaration's variables to them.
    fn fresh_effect_args(
        &mut self,
        op: &OpInfo,
    ) -> (Vec<Ty>, HashMap<u32, Ty>, HashMap<RowVar, RowVar>) {
        let mut m: HashMap<u32, Ty> = HashMap::new();
        let mut rm: HashMap<RowVar, RowVar> = HashMap::new();
        let mut args = Vec::with_capacity(op.effect_params.len());
        for (i, &p) in op.effect_params.iter().enumerate() {
            let a = match op.row_params.get(i).copied().flatten() {
                Some(rv) => {
                    let fresh = self.fresh_row();
                    rm.insert(rv, fresh);
                    phantom(EffectRow::open(fresh))
                }
                None => self.fresh(),
            };
            m.insert(p, a.clone());
            args.push(a);
        }
        (args, m, rm)
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
            // A conflict is two arguments that cannot be one type: different
            // heads, or two ground types that differ. Two that inference only
            // partly knows (`Box(a)`, `Box(Int)`; a row argument's phantom,
            // concrete from the start) are unified, and a conflict inside them
            // reported there (spec 2026-10-30-elya-row-variables, the part 2
            // review's F2: written label arguments are now checked).
            let conflict = head(&rx) != head(&ry) || (ground(&rx) && ground(&ry));
            if both_concrete && rx != ry && conflict {
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
        self.settle_duplicates(eff, span);
        let eff = self.resolve_row(eff);
        if let RowTail::Open(rho) = eff.tail {
            // A label whose arguments mention the row's own tail (a task's row,
            // `{Async(e) | e}`, spec 2026-10-30-elya-row-variables D4) cannot be
            // inside that tail. Such a row is unified whole: `Async` meets the
            // ambient's `Async`, and `e` the rest; adding the labels and then
            // equating the ambient with `e` would put `Async(e)` inside `e`
            // (E0424). The same holds for such a label already in the ambient
            // (a handle's residual `e` reaching an ambient where a clause ran a
            // task: the part 2 review's F3): it stays the ambient's own.
            let mentions =
                |me: &Self, l: &EffectLabel| l.args.iter().any(|a| me.mentions_row(rho, a));
            let own = eff.labels.values().any(|l| mentions(self, l));
            let amb_now = self.resolve_row(&EffectRow::open(amb));
            let theirs: Vec<(String, EffectLabel)> = amb_now
                .labels
                .iter()
                .filter(|(k, l)| !eff.labels.contains_key(*k) && mentions(self, l))
                .map(|(k, l)| (k.clone(), l.clone()))
                .collect();
            if own || !theirs.is_empty() {
                let mut whole = eff.clone();
                whole.labels.extend(theirs);
                return self.unify_row(&EffectRow::open(amb), &whole, span);
            }
        }
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
        // A row argument (spec 2026-10-30-elya-row-variables D4) is its row:
        // `Task({Log | a})`, not its phantom function type.
        t if phantom_row(t).is_some() => {
            write_row(phantom_row(t).expect("a phantom"), names, out);
        }
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
        // An operation of a `with any` clause: `Op(t / {E})`, its answer type
        // and the row performing it performs.
        Ty::Con(n, args) if n == "$Op" && args.len() == 2 => {
            out.push_str("Op(");
            write_ty(&args[0], names, out);
            if let Ty::Fn(_, row, _) = &args[1] {
                out.push_str(" / ");
                write_row(row, names, out);
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
        LtF | LeF | GtF | GeF => (Ty::float(), Ty::float(), Ty::bool()),
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
        for (v, fresh) in &mapping {
            if let (Some(origin), Ty::Var(f)) = (self.eq_vars.get(v).copied(), fresh) {
                self.eq_vars.insert(*f, origin);
            }
        }
        let mut row_mapping: HashMap<RowVar, RowVar> = HashMap::new();
        for v in &s.row_vars {
            let fresh = if self.phantom.contains(v) {
                self.fresh_phantom_row()
            } else {
                self.fresh_row()
            };
            // A handled tail that is also the function's own row (a forwarding
            // handler) stands for the caller's ambient after the call: not
            // checked statically (D5).
            let latent = matches!(&s.ty, Ty::Fn(_, r, _) if r.tail == RowTail::Open(*v));
            if self.generic_rows.contains(v) && !latent {
                self.pending_generic.push(GenericSite {
                    labels: BTreeMap::new(),
                    tail: EffectRow::open(fresh),
                    forwarded: false,
                    span: self.cur_span,
                });
            }
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
        self.cur_span = e.span;
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
            // Poison: only in a module with a parse error (never inferred).
            Expr::Error => Ty::Error,
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
                    UnOp::NegF => (Ty::float(), Ty::float()),
                };
                self.unify(&t, &operand, span);
                result
            }
            Expr::Binary { op, lhs, rhs } => {
                let lt = self.infer_expr(lhs, env, amb);
                let rt = self.infer_expr(rhs, env, amb);
                if matches!(op, BinOp::Eq | BinOp::Ne) {
                    self.unify(&lt, &rt, span);
                    self.require_data(&lt, span, None);
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
                    Some(cx) => {
                        // resume : (B) / ε -> R — takes the operation's result,
                        // yields the handle's result, and runs the rest of the
                        // handled body AND then the return clause, which may
                        // still perform ε. Directly in the clause ε is already in
                        // the ambient; inside a LAMBDA in the clause it is not,
                        // and leaving it out typed such a lambda pure (found by
                        // slice 5b-10, and again by the async-step-1 review for
                        // relayed and return-clause effects).
                        //
                        // LABELS, never the bare tail: unifying with the body's
                        // tail made this site's ambient EQUAL to it and merged
                        // rows related only by inclusion (the first resume-row
                        // review). Both rows are complete here: the body was
                        // inferred and its inclusions flushed, the return clause
                        // was inferred first and its residual closed. A tail
                        // that is a RELAY -- a row variable of a parameter in
                        // scope, which a caller instantiates -- IS unified,
                        // exactly as a call through that parameter would be.
                        self.unify(&arg_ty, &cx.arg, span);
                        if self.lambda_depth > cx.lambda_depth {
                            // Escaped: the rest of the body may re-enter any
                            // clause, whose effects are not all known yet.
                            let all = EffectRow::open(cx.clauses_row);
                            let r = self.add_row(amb, &all, span);
                            self.surface_conflict(r);
                            // ... and it re-runs the body's recursive calls,
                            // whose member rows may still grow: include them
                            // here too, so a later label reaches this lambda or
                            // is reported against its closed row (the
                            // handle-recursion review's F2).
                            let body_tail = self.resolve_row(&EffectRow::open(cx.body_amb)).tail;
                            for (m, dst, sp) in self.pending_member_incl.clone() {
                                if self.resolve_row(&EffectRow::open(dst)).tail == body_tail {
                                    self.pending_member_incl.push((m, amb, sp));
                                }
                            }
                        }
                        let mut in_params = Vec::new();
                        for p in self.param_tys.clone() {
                            free_row_vars(self, &p, &mut in_params);
                        }
                        for (src, except, all) in [
                            (cx.body_amb, cx.handled.clone(), cx.handles_all),
                            (cx.ret_amb, None, false),
                        ] {
                            let now = self.resolve_row(&EffectRow::open(src));
                            for (label, l) in now.labels.iter() {
                                if except.as_deref() == Some(label.as_str())
                                    || (all && label != "IO")
                                {
                                    continue;
                                }
                                let r = self.add_effect(amb, label, l.args.clone(), l.span);
                                self.surface_conflict(r);
                            }
                            if let RowTail::Open(w) = now.tail {
                                if in_params.contains(&w) && !all {
                                    let relay = EffectRow {
                                        labels: BTreeMap::new(),
                                        tail: RowTail::Open(w),
                                    };
                                    let r = self.add_row(amb, &relay, span);
                                    self.surface_conflict(r);
                                }
                            }
                        }
                        cx.result
                    }
                    // resume outside a handler is E0210 at resolve time.
                    None => Ty::Error,
                }
            }
            Expr::Match { scrutinee, arms } => {
                let s = self.infer_expr(scrutinee, env, amb);
                let result = self.fresh();
                // An empty match is exhaustive exactly when the scrutinee's type
                // has no constructors; its result then has any type (spec
                // 2026-10-16). `exhaust.rs` is type-free and leaves it here.
                if arms.is_empty() {
                    let st = self.resolve(&s);
                    let empty = matches!(&st, Ty::Con(n, _) if self.empty_types.contains(n));
                    if !empty && st != Ty::Error {
                        self.diags.push(
                            Diagnostic::error(
                                "E0430",
                                format!(
                                    "a match with no arms needs a scrutinee whose type has no constructors; this is `{}`",
                                    display_ty(self, &st)
                                ),
                            )
                            .with_label(scrutinee.span, "a value of this type may exist")
                            .with_help("add arms, or match a value of an empty type such as `type Never { }`"),
                        );
                    }
                }
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
                self.lambda_depth += 1;
                self.regions.push((lam_amb, true));
                let body_ty = self.infer_block(&body.node, env, lam_amb);
                self.regions.pop();
                self.lambda_depth -= 1;
                self.param_tys.truncate(mark);
                env.pop();
                // Fork A (4b-2 §2): close the lambda's residual tail unless it is
                // relayed through a parameter — the same discipline top-level fns
                // use. A concrete-effect lambda gets a minimal closed row (`{Log}`);
                // a relay lambda keeps its open, row-polymorphic tail.
                // Pending sub-effecting inclusions reach this ambient before it
                // closes (a label arriving later is a surfaced conflict).
                // A relay of an ENCLOSING function's (or lambda's) parameter
                // keeps the tail open too (2026-10-09): `fn wrap(f) { fn(x) {
                // f(x) } }` relays `f`'s row, and closing it forced `f` pure
                // (PARKED by sub-effecting until native code could compile the
                // effect-polymorphic result -- N7 part 1).
                self.flush_inclusions();
                let mut scope_params = self.param_tys.clone();
                scope_params.extend(param_tys.iter().cloned());
                // Recursion through a lambda (2026-10-15): a lambda that
                // includes a group member's row cannot close yet -- the member
                // may perform more later in its body. Closed with the group.
                let lam_tail = self.resolve_row(&EffectRow::open(lam_amb)).tail;
                let includes_member = self
                    .pending_member_incl
                    .clone()
                    .into_iter()
                    .any(|(_, dst, _)| self.resolve_row(&EffectRow::open(dst)).tail == lam_tail);
                if includes_member && matches!(lam_tail, RowTail::Open(_)) {
                    self.deferred_rows.push((lam_amb, scope_params));
                } else {
                    self.close_unrelayed_residual(lam_amb, &scope_params);
                }
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
            // `time.now_ms : fn() -> Int` and `time.sleep_ms : fn(Int) -> Unit`,
            // both performing `IO` (spec 2026-10-15-elya-async-step2-timers D3).
            if module == "time" && (name == "now_ms" || name == "sleep_ms") {
                let (params, ret) = if name == "now_ms" {
                    (Vec::new(), Ty::int())
                } else {
                    (vec![Ty::int()], Ty::unit())
                };
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
                let want = Ty::Fn(params.clone(), EffectRow::pure(), Box::new(ret.clone()));
                let got = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(ret.clone()));
                self.unify(&want, &got, span);
                {
                    let r = self.add_effect(amb, "IO", Vec::new(), span);
                    self.surface_conflict(r);
                }
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
                        params,
                        EffectRow {
                            labels: io_labels,
                            tail: RowTail::Closed,
                        },
                        Box::new(ret.clone()),
                    ),
                );
                return ret;
            }
            // The builtins on an operation of a `with any` clause (2026-10-16,
            // D4). `o : Op(t, fn() / H -> t)`: `op.perform` performs `H`.
            if module == "op"
                && matches!(
                    name.as_str(),
                    "name" | "show" | "perform" | "encode" | "decode"
                )
            {
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
                let t = self.fresh();
                let h = self.fresh_row();
                let op_ty = Ty::Con(
                    "$Op".to_string(),
                    vec![
                        t.clone(),
                        Ty::Fn(Vec::new(), EffectRow::open(h), Box::new(t.clone())),
                    ],
                );
                let (params, ret) = match name.as_str() {
                    "name" | "show" => (vec![op_ty], Ty::str()),
                    "perform" => (vec![op_ty], t.clone()),
                    "encode" => (vec![op_ty, t.clone()], Ty::str()),
                    _ => (vec![op_ty, Ty::str()], t.clone()),
                };
                let row = if name == "perform" {
                    EffectRow::open(h)
                } else {
                    EffectRow::pure()
                };
                let want = Ty::Fn(params.clone(), EffectRow::pure(), Box::new(ret.clone()));
                let got = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(ret.clone()));
                self.unify(&want, &got, span);
                if name == "perform" {
                    let performed = self.resolve_row(&EffectRow::open(h));
                    let r = self.add_row(amb, &performed, span);
                    self.surface_conflict(r);
                }
                self.node_types
                    .insert(callee.span, Ty::Fn(params, row, Box::new(ret.clone())));
                return ret;
            }
            // `int.to_string : fn(Int) -> String`, pure (spec
            // 2026-10-15-elya-native-strings D1).
            // `float.to_string : fn(Float) -> String`, pure (spec
            // 2026-10-23-elya-float-natively D5).
            if (module == "int" || module == "float") && name == "to_string" {
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
                let num = if module == "int" {
                    Ty::int()
                } else {
                    Ty::float()
                };
                let want = Ty::Fn(vec![num], EffectRow::pure(), Box::new(Ty::str()));
                let got = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(Ty::str()));
                self.unify(&want, &got, span);
                self.node_types.insert(callee.span, want);
                return Ty::str();
            }
            // `float.of_int : fn(Int) -> Float`, `float.truncate`/`round :
            // fn(Float) -> Int`, pure (spec 2026-10-24-elya-float-part-2 D3, D4;
            // a failure is not an effect, as with checked `Int` arithmetic).
            if module == "float" && crate::resolve::FLOAT_CONVERSIONS.contains(&name.as_str()) {
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
                let (from, to) = if name == "of_int" {
                    (Ty::int(), Ty::float())
                } else {
                    (Ty::float(), Ty::int())
                };
                let want = Ty::Fn(vec![from], EffectRow::pure(), Box::new(to.clone()));
                let got = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(to.clone()));
                self.unify(&want, &got, span);
                self.node_types.insert(callee.span, want);
                return to;
            }
            // A string by byte (spec 2026-10-24-elya-trace-text D1), pure: a
            // failure is not an effect.
            if module == "string" && crate::resolve::STRING_BUILTINS.contains(&name.as_str()) {
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
                let (params, ret) = match name.as_str() {
                    "length" => (vec![Ty::str()], Ty::int()),
                    "byte_at" => (vec![Ty::str(), Ty::int()], Ty::int()),
                    _ => (vec![Ty::str(), Ty::int(), Ty::int()], Ty::str()),
                };
                let want = Ty::Fn(params, EffectRow::pure(), Box::new(ret.clone()));
                let got = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(ret.clone()));
                self.unify(&want, &got, span);
                self.node_types.insert(callee.span, want);
                return ret;
            }
            // Wrapping and bit arithmetic: `fn(Int, Int) -> Int`, pure (spec
            // 2026-10-17-elya-unary-and-wrapping D2).
            if module == "int" && crate::resolve::INT_BINARY_BUILTINS.contains(&name.as_str()) {
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
                let want = Ty::Fn(
                    vec![Ty::int(), Ty::int()],
                    EffectRow::pure(),
                    Box::new(Ty::int()),
                );
                let got = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(Ty::int()));
                self.unify(&want, &got, span);
                self.node_types.insert(callee.span, want);
                return Ty::int();
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
            } else if !in_params.contains(&v) {
                if let Some(m) = self.member_in_handle_body(v, amb) {
                    self.pending_member_incl.push((m, amb, span));
                    eff.tail = RowTail::Closed;
                }
            }
        }
        {
            let r = self.add_row(amb, &eff, span);
            self.surface_conflict(r);
        }
        result
    }

    /// Is a callee row ending in `v` a member of the group being inferred,
    /// called directly in the body of a handle or a lambda (not in a clause,
    /// nor in a region nested inside) whose ambient `amb` is not already that
    /// member's row? Then the member to include (recursion through a handle
    /// body or a lambda, 2026-10-15).
    fn member_in_handle_body(&mut self, v: RowVar, amb: RowVar) -> Option<RowVar> {
        let &(region, includes) = self.regions.last()?;
        if !includes {
            return None;
        }
        let amb_tail = self.resolve_row(&EffectRow::open(amb)).tail;
        if amb_tail != self.resolve_row(&EffectRow::open(region)).tail
            || amb_tail == RowTail::Open(v)
        {
            return None;
        }
        // A tail shared with a member's parameter is a RELAY (it becomes what
        // the arguments perform): it must be unified, as any relay is.
        let mut relayed = Vec::new();
        for p in self.group_params.clone() {
            free_row_vars(self, &p, &mut relayed);
        }
        if relayed.contains(&v) {
            return None;
        }
        self.group_ambs
            .clone()
            .into_iter()
            .find(|&m| self.resolve_row(&EffectRow::open(m)).tail == RowTail::Open(v))
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
        if handler.any {
            return self.infer_any_handle(body, handler, span, env, amb);
        }
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
        // A row parameter is instantiated as a fresh row, shared the same way
        // (spec 2026-10-30-elya-row-variables D4).
        let (eff_args, eff_rows): (Vec<Ty>, HashMap<RowVar, RowVar>) = match effect
            .as_ref()
            .and_then(|e| self.ops.values().find(|o| &o.effect == e).cloned())
        {
            Some(op) => {
                let (args, _, rm) = self.fresh_effect_args(&op);
                (args, rm)
            }
            None => (Vec::new(), HashMap::new()),
        };
        // `e` may perform E plus a polymorphic remainder.
        let amb_in = self.fresh_row();
        if let Some(e) = &effect {
            {
                let r = self.add_effect(amb_in, e, eff_args.clone(), span);
                self.surface_conflict(r);
            }
        }
        self.regions.push((amb_in, true));
        let body_ty = self.infer_expr(body, env, amb_in);
        self.regions.pop();

        // R — the handle's result type, shared by every clause and `return`.
        let result = self.fresh();
        // The return clause FIRST (2026-10-09): a resume runs the rest of the
        // body and then this clause, so a lambda that resumes must carry its
        // effects -- and a lambda's row closes when the lambda is done, which
        // for a lambda in a clause is before a later return clause would be
        // inferred. Its own ambient is a region like a lambda body: inclusions
        // flushed, its residual closed unless relayed through a parameter,
        // then poured into the handle's ambient.
        let ret_amb = self.fresh_row();
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
                self.regions.push((ret_amb, false));
                let ret_ty = self.infer_expr(&ret.body, env, ret_amb);
                self.regions.pop();
                self.unify(&ret_ty, &result, ret.body.span);
                env.pop();
            }
            // No return clause ⇒ identity: R = type of `e`.
            None => self.unify(&result, &body_ty, span),
        }
        self.flush_inclusions();
        let scope_params = self.param_tys.clone();
        self.close_unrelayed_residual(ret_amb, &scope_params);
        {
            let ret_row = self.resolve_row(&EffectRow::open(ret_amb));
            let r = self.add_row(amb, &ret_row, span);
            self.surface_conflict(r);
        }
        let mut clause_ambs: Vec<RowVar> = Vec::new();
        let clauses_row = self.fresh_row();
        self.pending_clause_rows.push(clauses_row);
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
                    (
                        op.params
                            .iter()
                            .map(|t| subst_vars(t, &m, &eff_rows))
                            .collect::<Vec<_>>(),
                        subst_vars(&op.ret, &m, &eff_rows),
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
            // Each clause is a region of its own (2026-10-09), so its effects
            // are known apart: an escaped resume may re-enter it.
            let clause_amb = self.fresh_row();
            clause_ambs.push(clause_amb);
            self.resume_stack.push(ResumeCx {
                arg: b,
                result: result.clone(),
                body_amb: amb_in,
                ret_amb,
                handled: effect.clone(),
                handles_all: false,
                lambda_depth: self.lambda_depth,
                clauses_row,
            });
            let mark = self.param_tys.len();
            self.param_tys.extend(params.iter().cloned());
            self.regions.push((clause_amb, false));
            let clause_ty = self.infer_expr(&clause.body, env, clause_amb);
            self.regions.pop();
            self.flush_inclusions();
            let scope_params = self.param_tys.clone();
            self.close_unrelayed_residual(clause_amb, &scope_params);
            self.param_tys.truncate(mark);
            {
                let row = self.resolve_row(&EffectRow::open(clause_amb));
                let r = self.add_row(amb, &row, clause.body.span);
                self.surface_conflict(r);
            }
            self.unify(&clause_ty, &result, clause.body.span);
            self.resume_stack.pop();
            env.pop();
        }
        // Escaped resumes: a lambda that resumes runs the rest of the body,
        // which may re-enter any clause -- so the handle's clauses row takes
        // every clause's effects (but the handled one, which the resumed
        // handler handles), and only then closes. A clauses row that has
        // become the enclosing ambient's own tail (the lambda was called in a
        // clause) is left to that ambient.
        self.pending_clause_rows.pop();
        for &ca in &clause_ambs {
            let row = self.resolve_row(&EffectRow::open(ca));
            for (label, l) in row.labels.iter() {
                if effect.as_deref() == Some(label.as_str()) {
                    continue;
                }
                let r = self.add_effect(clauses_row, label, l.args.clone(), l.span);
                self.surface_conflict(r);
            }
            // A clause's open tail (a parameter's row it relays) is performed
            // again by an escaped resume that re-enters the clause (found by
            // the `with any` review, 2026-10-16: the lambda was typed pure).
            if matches!(row.tail, RowTail::Open(_)) {
                let tail = EffectRow {
                    labels: BTreeMap::new(),
                    tail: row.tail.clone(),
                };
                let r = self.add_row(clauses_row, &tail, span);
                self.surface_conflict(r);
            }
        }
        self.flush_inclusions();
        let own_tail = self.resolve_row(&EffectRow::open(amb)).tail;
        if self.resolve_row(&EffectRow::open(clauses_row)).tail != own_tail {
            let scope_params = self.param_tys.clone();
            self.close_unrelayed_residual(clauses_row, &scope_params);
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

    /// `handle e with any { op(o) -> c  return(x) -> r }` (2026-10-16, spec
    /// `2026-10-16-elya-any-handlers-design.md`). Handles everything `e` performs
    /// but `IO` (D2): the residual is `IO` alone, and the rest -- the body row's
    /// labels and its tail -- is handled. `o : Op(t, fn() / H -> t)`, where `t` is
    /// a rigid type of this clause (D3) and `H` the handled row: the function
    /// type is how the row rides in the operation's type, so `op.perform(o)`
    /// performs it wherever `o` goes (a function given `o` relays it).
    fn infer_any_handle(
        &mut self,
        body: &Spanned<Expr>,
        handler: &Handler,
        span: Span,
        env: &mut TyEnv,
        amb: RowVar,
    ) -> Ty {
        let amb_in = self.fresh_row();
        self.regions.push((amb_in, true));
        let body_ty = self.infer_expr(body, env, amb_in);
        self.regions.pop();

        let result = self.fresh();
        let ret_amb = self.fresh_row();
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
                self.regions.push((ret_amb, false));
                let ret_ty = self.infer_expr(&ret.body, env, ret_amb);
                self.regions.pop();
                self.unify(&ret_ty, &result, ret.body.span);
                env.pop();
            }
            None => self.unify(&result, &body_ty, span),
        }
        self.flush_inclusions();
        let scope_params = self.param_tys.clone();
        self.close_unrelayed_residual(ret_amb, &scope_params);
        {
            let ret_row = self.resolve_row(&EffectRow::open(ret_amb));
            let r = self.add_row(amb, &ret_row, span);
            self.surface_conflict(r);
        }
        // The handled row: the body's, but `IO`. Its labels and tail must be
        // data (E0434), checked when the group is solved.
        let mut handled = self.resolve_row(&EffectRow::open(amb_in));
        handled.labels.remove("IO");
        let snapshot = handled.labels.clone();

        let mut clause_ambs: Vec<RowVar> = Vec::new();
        let clauses_row = self.fresh_row();
        self.pending_clause_rows.push(clauses_row);
        let mut new_skolems: Vec<(String, Span)> = Vec::new();
        for c in &handler.clauses {
            let clause = &c.node;
            let sk_name = format!("'t{}", self.skolems.len() + 1);
            self.skolems.push((sk_name.clone(), c.span));
            new_skolems.push((sk_name.clone(), c.span));
            let t = Ty::Con(sk_name, Vec::new());
            let op_ty = Ty::Con(
                "$Op".to_string(),
                vec![
                    t.clone(),
                    Ty::Fn(Vec::new(), handled.clone(), Box::new(t.clone())),
                ],
            );
            env.push();
            for p in clause.params.iter() {
                self.node_types.insert(p.span, op_ty.clone());
                env.insert(
                    &p.node.name,
                    Scheme {
                        vars: Vec::new(),
                        row_vars: Vec::new(),
                        ty: op_ty.clone(),
                    },
                );
            }
            let clause_amb = self.fresh_row();
            clause_ambs.push(clause_amb);
            self.resume_stack.push(ResumeCx {
                arg: t,
                result: result.clone(),
                body_amb: amb_in,
                ret_amb,
                handled: None,
                handles_all: true,
                lambda_depth: self.lambda_depth,
                clauses_row,
            });
            let mark = self.param_tys.len();
            self.param_tys.push(op_ty.clone());
            self.regions.push((clause_amb, false));
            let clause_ty = self.infer_expr(&clause.body, env, clause_amb);
            self.regions.pop();
            self.flush_inclusions();
            let scope_params = self.param_tys.clone();
            self.close_unrelayed_residual(clause_amb, &scope_params);
            self.param_tys.truncate(mark);
            {
                let row = self.resolve_row(&EffectRow::open(clause_amb));
                let r = self.add_row(amb, &row, clause.body.span);
                self.surface_conflict(r);
            }
            self.unify(&clause_ty, &result, clause.body.span);
            self.resume_stack.pop();
            env.pop();
        }
        // Escaped resumes may re-enter the clause, whose effects -- the
        // operations it performs again included -- happen outside the handler.
        self.pending_clause_rows.pop();
        for &ca in &clause_ambs {
            let row = self.resolve_row(&EffectRow::open(ca));
            for (label, l) in row.labels.iter() {
                let r = self.add_effect(clauses_row, label, l.args.clone(), l.span);
                self.surface_conflict(r);
            }
            // A clause's open tail -- the handled one when it forwards, or a
            // parameter's it relays -- is performed again by an escaped resume
            // that re-enters it (the review's S1 and F1: such a lambda was
            // typed pure, and its performs reached no handler).
            if matches!(row.tail, RowTail::Open(_)) {
                let tail = EffectRow {
                    labels: BTreeMap::new(),
                    tail: row.tail.clone(),
                };
                let r = self.add_row(clauses_row, &tail, span);
                self.surface_conflict(r);
            }
        }
        self.flush_inclusions();
        let own_tail = self.resolve_row(&EffectRow::open(amb)).tail;
        if self.resolve_row(&EffectRow::open(clauses_row)).tail != own_tail {
            let scope_params = self.param_tys.clone();
            self.close_unrelayed_residual(clauses_row, &scope_params);
        }
        // The residual: `IO`, never dispatched to a handler, if the body has it.
        let body_row = self.resolve_row(&EffectRow::open(amb_in));
        if let Some(io) = body_row.labels.get("IO") {
            let r = self.add_effect(amb, "IO", io.args.clone(), io.span);
            self.surface_conflict(r);
        }
        // Forwarding: a clause that performs its operation again poured the
        // handled tail into its ambient (`add_row` unifies tails).
        let handled_tail = self.resolve_row(&handled).tail;
        let forwarded = matches!(handled_tail, RowTail::Open(_))
            && clause_ambs
                .iter()
                .any(|&ca| self.resolve_row(&EffectRow::open(ca)).tail == handled_tail);
        self.pending_generic.push(GenericSite {
            labels: snapshot,
            tail: EffectRow {
                labels: BTreeMap::new(),
                tail: handled.tail.clone(),
            },
            forwarded,
            span,
        });
        // E0435: an answer type stays in its clause.
        for (sk, sp) in new_skolems {
            self.check_skolem_escape(&sk, sp, &result, env, amb);
        }
        result
    }

    /// E0435 (2026-10-16): the rigid answer type `sk` of a `with any` clause in
    /// the handle's result, in a binding outside the clause, or in the ambient
    /// row's effect arguments. One run of the clause would meet another's.
    fn check_skolem_escape(&mut self, sk: &str, span: Span, result: &Ty, env: &TyEnv, amb: RowVar) {
        let mut where_: Option<String> = None;
        if self.mentions_con(result, sk) {
            where_ = Some("the handle's result type".to_string());
        }
        if where_.is_none() {
            'outer: for scope in &env.scopes {
                for (name, scheme) in scope {
                    if self.mentions_con(&scheme.ty, sk) {
                        where_ = Some(format!("`{name}`, bound outside the clause"));
                        break 'outer;
                    }
                }
            }
        }
        if where_.is_none() {
            let row = self.resolve_row(&EffectRow::open(amb));
            if row
                .labels
                .values()
                .any(|l| l.args.iter().any(|a| self.mentions_con(a, sk)))
            {
                where_ = Some("an effect's type argument".to_string());
            }
        }
        if let Some(w) = where_ {
            self.diags.push(
                Diagnostic::error("E0435", "an operation's answer type escapes its clause")
                    .with_label(span, format!("the answer type of this clause reaches {w}"))
                    .with_help(
                        "a `with any` clause runs once per operation, at a different answer type each time; give an answer back with `resume`, or turn it into data with `op.encode`",
                    ),
            );
        }
    }

    /// Whether the resolved `t` mentions the nominal type `name`, rows included.
    fn mentions_con(&self, t: &Ty, name: &str) -> bool {
        match self.resolve(t) {
            Ty::Con(n, args) => n == name || args.iter().any(|a| self.mentions_con(a, name)),
            Ty::Fn(ps, row, r) => {
                ps.iter().any(|p| self.mentions_con(p, name))
                    || self.mentions_con(&r, name)
                    || row
                        .labels
                        .values()
                        .any(|l| l.args.iter().any(|a| self.mentions_con(a, name)))
            }
            Ty::Tuple(xs) => xs.iter().any(|x| self.mentions_con(x, name)),
            Ty::Var(_) | Ty::Base(_) | Ty::Error => false,
        }
    }

    /// E0434 (2026-10-16, D5): every row a `with any` handler handles, queued
    /// at the handle and at each instantiation of a scheme generalizing one, is
    /// data: each label a user effect (never `IO`) whose operations take and
    /// answer data at the label's arguments. A tail still open becomes a
    /// generically handled variable, checked again wherever it is instantiated.
    fn check_generic_rows(&mut self) {
        for site in std::mem::take(&mut self.pending_generic) {
            let mut labels = site.labels.clone();
            let row = self.resolve_row(&site.tail);
            if !site.forwarded {
                for (k, l) in &row.labels {
                    labels.entry(k.clone()).or_insert_with(|| l.clone());
                }
            }
            for (label, l) in &labels {
                if label == "IO" {
                    self.diags.push(
                        Diagnostic::error("E0434", "`IO` cannot be handled by `with any`")
                            .with_label(site.span, "the computation given to a `with any` handler performs `IO`")
                            .with_help("`IO` is never dispatched to a handler, so a generic handler that does not perform its operations again would hide it; handle a user effect instead"),
                    );
                    continue;
                }
                let mut ops: Vec<(String, OpInfo)> = self
                    .ops
                    .iter()
                    .filter(|(_, o)| &o.effect == label)
                    .map(|(n, o)| (n.clone(), o.clone()))
                    .collect();
                ops.sort_by(|a, b| a.0.cmp(&b.0));
                for (name, op) in ops {
                    let m: HashMap<u32, Ty> = op
                        .effect_params
                        .iter()
                        .cloned()
                        .zip(l.args.iter().cloned())
                        .collect();
                    let rm = HashMap::new();
                    let tys: Vec<Ty> = op
                        .params
                        .iter()
                        .chain(std::iter::once(&op.ret))
                        .map(|t| subst_vars(t, &m, &rm))
                        .collect();
                    if let Some(why) = tys.iter().find_map(|t| self.not_data(t, &mut Vec::new())) {
                        self.diags.push(
                            Diagnostic::error("E0434", "an effect handled by `with any` must be data")
                                .with_label(site.span, format!("`{label}.{name}` is handled here generically"))
                                .with_help(format!(
                                    "a `with any` handler shows, encodes and decodes every operation it handles, so their argument and answer types must be data (no functions); `{label}.{name}` has {why}"
                                )),
                        );
                    }
                }
            }
            if let RowTail::Open(v) = row.tail {
                self.generic_rows.insert(v);
            }
        }
    }

    /// Why `t` (resolved) is not data, or `None` if it is: a declared type is
    /// data when its arguments are and its declaration is (`decl_not_data`).
    /// Checking the two apart terminates on non-regular types, whose arguments
    /// grow under recursion (`Nest(a) { S(Nest(Opt(a))) }`; the review's B1).
    fn not_data(&self, t: &Ty, seen: &mut Vec<String>) -> Option<String> {
        match self.resolve(t) {
            Ty::Base(_) | Ty::Error => None,
            Ty::Fn(..) => Some("a function type".to_string()),
            Ty::Var(_) => Some("a type not known to be data".to_string()),
            Ty::Tuple(xs) => xs.iter().find_map(|x| self.not_data(x, seen)),
            Ty::Con(n, args) => args
                .iter()
                .find_map(|a| self.not_data(a, seen))
                .or_else(|| self.decl_not_data(&n, seen)),
        }
    }

    /// Whether a declared type's fields are data, its parameters taken as data
    /// (its arguments are checked where it is used).
    fn decl_not_data(&self, name: &str, seen: &mut Vec<String>) -> Option<String> {
        if seen.iter().any(|n| n == name) {
            return None;
        }
        seen.push(name.to_string());
        let Some((params, ctors)) = self.adt_fields.get(name).cloned() else {
            return Some(format!("the type `{name}`"));
        };
        ctors
            .iter()
            .flatten()
            .find_map(|f| self.shape_not_data(f, &params, seen))
    }

    /// `not_data` over a declaration's field type, where its parameters are data.
    fn shape_not_data(&self, t: &Ty, params: &[u32], seen: &mut Vec<String>) -> Option<String> {
        match t {
            Ty::Base(_) | Ty::Error => None,
            Ty::Fn(..) => Some("a function type".to_string()),
            Ty::Var(v) if params.contains(v) => None,
            Ty::Var(_) => Some("a type not known to be data".to_string()),
            Ty::Tuple(xs) => xs.iter().find_map(|x| self.shape_not_data(x, params, seen)),
            Ty::Con(n, args) => args
                .iter()
                .find_map(|a| self.shape_not_data(a, params, seen))
                .or_else(|| self.decl_not_data(n, seen)),
        }
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
        // The function's named rows are one row for the whole function (D2), as
        // its annotation type variables are (above).
        for v in self.ann_rows.values().copied().collect::<Vec<_>>() {
            if let RowTail::Open(w) = self.resolve_row(&EffectRow::open(v)).tail {
                row_in_env.push(w);
            }
        }
        // A handle's clauses row still being filled is not quantified: its
        // labels arrive when the handle is done (the resume-row review's F1).
        for c in self.pending_clause_rows.clone().into_iter().chain(
            self.deferred_rows
                .iter()
                .map(|(r, _)| *r)
                .collect::<Vec<_>>(),
        ) {
            if let RowTail::Open(v) = self.resolve_row(&EffectRow::open(c)).tail {
                row_in_env.push(v);
            }
        }
        // A row a `with any` handler handles, not yet checked for data (E0434
        // runs when the top-level group is solved), stays monomorphic in a
        // local `let`: generalized, its later instantiations were never
        // checked (the review's S3: `IO` hidden from a pure function).
        for site in self.pending_generic.iter().filter(|s| !s.forwarded) {
            if let RowTail::Open(v) = self.resolve_row(&site.tail).tail {
                row_in_env.push(v);
            }
        }
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
            // A handle's clauses row still being filled stays open.
            for c in self.pending_clause_rows.clone() {
                if self.resolve_row(&EffectRow::open(c)).tail == RowTail::Open(rho) {
                    return;
                }
            }
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
    fn check_exact_row(&mut self, amb: RowVar, declared: &crate::ast::RowAnn) {
        // A named row's check is done earlier, before the ambient closes
        // (2.48).
        if declared.tail.is_some() {
            return;
        }
        let declared: Vec<&Spanned<String>> = declared.labels.iter().map(|l| &l.name).collect();
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

    /// A declared row that ends in a named variable, `/ {Log | e}` (spec
    /// 2026-10-30-elya-row-variables D2): each declared label must be performed
    /// (E0423, as for a closed row), and the rest of what the body performs is
    /// `e`'s: the ambient is unified with `{labels | e}`.
    fn check_named_row(&mut self, amb: RowVar, labels: &[Spanned<String>], name: &Spanned<String>) {
        let performed = self.resolve_row(&EffectRow::open(amb));
        let unused: Vec<(String, Span)> = labels
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
            return;
        }
        let tail = self.ann_row(&name.node, name.span);
        // A performed label whose arguments mention the named row (a task's own
        // `Async(e)` under `/ {e}`) cannot be in it: the body performs an effect
        // the row leaves out, by name (the part 2 review's F4: unified, it was
        // "cyclic effect row" and named nothing). The name's variable as it
        // stands now: a relay may have bound it to a fresh tail.
        let now = match self.tail_now(&RowTail::Open(tail)) {
            RowTail::Open(v) => v,
            _ => tail,
        };
        let left_out: Vec<(String, Span)> = performed
            .labels
            .iter()
            .filter(|(k, l)| {
                !labels.iter().any(|d| &d.node == *k)
                    && l.args.iter().any(|a| self.mentions_row(now, a))
            })
            .map(|(k, l)| (k.clone(), l.span))
            .collect();
        if !left_out.is_empty() {
            let span = left_out[0].1;
            self.emit_row_mismatch(
                &left_out,
                span,
                "the body performs an effect the declared row does not",
            );
            return;
        }
        let mut m = BTreeMap::new();
        for l in labels {
            if let Some(have) = performed.labels.get(&l.node) {
                m.insert(l.node.clone(), have.clone());
            }
        }
        let declared = EffectRow {
            labels: m,
            tail: RowTail::Open(tail),
        };
        let span = labels.first().map(|l| l.span).unwrap_or(Span::EMPTY);
        let r = self.unify_row(&EffectRow::open(amb), &declared, span);
        self.surface_conflict(r);
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

/// What a declaration's annotations are elaborated under (spec
/// 2026-10-30-elya-row-variables D3).
struct DeclCx<'a> {
    /// The declaration's name, for messages.
    decl: &'a str,
    /// Its type parameters' variables.
    param_env: &'a HashMap<String, Ty>,
    /// Its row parameters' row variables.
    row_env: &'a HashMap<String, RowVar>,
    known: &'a HashMap<String, usize>,
    /// Every declared effect and its arity.
    effects: &'a HashMap<String, usize>,
    kinds: &'a Kinds,
    /// The effect being declared and its own arguments: its bare name in its
    /// own declaration is itself at its own parameters (D3).
    own: Option<(&'a str, &'a [Ty])>,
}

/// Elaborate an ADT field/argument type annotation under a type-parameter
/// environment: a bare param name → its type variable; a base name → its
/// `Base`; an `Upper(args)` → a `Ty::Con` (arity-checked against `known`).
///
/// A function type `fn(A) / {E} -> R` (2026-10-09, async step 1): a
/// declaration's rows are exactly what is written -- no `/ {..}` is the empty
/// row (pure), not "any effects" as in a function's own annotations. A row may
/// name `IO` and any declared effect, with its arguments written (spec
/// 2026-10-30-elya-row-variables D5), and end in one of the declaration's row
/// parameters (D3).
fn elaborate_adt_ty(inf: &mut Infer, ann: &Spanned<TypeAnn>, cx: &DeclCx) -> Ty {
    if ann.node.as_row_arg().is_some() {
        inf.diags
            .push(Diagnostic::error("E0400", row_not_here()).with_label(ann.span, "a row here"));
        return Ty::Error;
    }
    elaborate_adt_inner(inf, ann, cx)
}

fn elaborate_adt_inner(inf: &mut Infer, ann: &Spanned<TypeAnn>, cx: &DeclCx) -> Ty {
    let t = &ann.node;
    if t.name == crate::ast::ROW_RESULT && t.args.is_empty() {
        return Ty::Con(t.name.clone(), Vec::new());
    }
    if t.name == "fn" && !t.args.is_empty() {
        let n = t.args.len();
        let params: Vec<Ty> = t.args[..n - 1]
            .iter()
            .map(|a| elaborate_adt_ty(inf, a, cx))
            .collect();
        let ret = elaborate_adt_ty(inf, &t.args[n - 1], cx);
        let mut labels = BTreeMap::new();
        let empty = crate::ast::RowAnn::default();
        let row = t.row.as_ref().unwrap_or(&empty);
        for l in &row.labels {
            let name = &l.name.node;
            if name == "IO" {
                labels.insert(
                    name.clone(),
                    EffectLabel {
                        args: Vec::new(),
                        span: l.name.span,
                    },
                );
                continue;
            }
            let Some(&arity) = cx.effects.get(name) else {
                inf.diags.push(
                    Diagnostic::error("E0432", format!("unknown effect `{name}`"))
                        .with_label(l.name.span, "no such effect"),
                );
                continue;
            };
            let args = match &l.args {
                None if arity == 0 => Vec::new(),
                None if cx.own.is_some_and(|o| o.0 == name) => {
                    cx.own.map(|o| o.1.to_vec()).unwrap_or_default()
                }
                None => {
                    let mut d = Diagnostic::error(
                        "E0432",
                        format!("effect `{name}` takes type arguments: write them, `{name}(..)`"),
                    )
                    .with_label(l.name.span, "a parameterized effect")
                    .with_help("a declaration's rows are exactly what is written");
                    if (0..arity).any(|j| cx.kinds.effect_row(name, j)) {
                        d = d.with_help(format!(
                            "its argument is a row: `{name}({{Log}})`, or a row parameter, `{name}(e)`"
                        ));
                    }
                    inf.diags.push(d);
                    continue;
                }
                Some(xs) if xs.len() != arity => {
                    inf.diags.push(
                        Diagnostic::error(
                            "E0400",
                            format!(
                                "effect `{name}` takes {arity} argument(s), found {}",
                                xs.len()
                            ),
                        )
                        .with_label(l.name.span, "wrong number of arguments"),
                    );
                    continue;
                }
                Some(xs) => {
                    let mut args = Vec::with_capacity(xs.len());
                    for (j, x) in xs.iter().enumerate() {
                        let param = Kinds::param_name(&cx.kinds.effects, name, j);
                        args.push(if cx.kinds.effect_row(name, j) {
                            decl_row_arg(inf, x, cx, name, param)
                        } else {
                            decl_type_arg(inf, x, cx, name, param)
                        });
                    }
                    args
                }
            };
            labels.insert(
                name.clone(),
                EffectLabel {
                    args,
                    span: l.name.span,
                },
            );
        }
        let tail = match &row.tail {
            None => RowTail::Closed,
            Some(v) => match cx.row_env.get(&v.node) {
                Some(&rv) => RowTail::Open(rv),
                None => {
                    inf.diags.push(undeclared_row_var(&v.node, v.span, cx.decl));
                    RowTail::Closed
                }
            },
        };
        return Ty::Fn(params, EffectRow { labels, tail }, Box::new(ret));
    }
    if t.args.is_empty() {
        if let Some(ty) = cx.param_env.get(&t.name) {
            return ty.clone();
        }
        if cx.row_env.contains_key(&t.name) {
            inf.diags.push(
                Diagnostic::error(
                    "E0400",
                    format!(
                        "`{}` is a row parameter of `{}`: a type goes here",
                        t.name, cx.decl
                    ),
                )
                .with_label(ann.span, "a type here")
                .with_help(
                    "a parameter is a type or a row: the declaration uses this one as a row",
                ),
            );
            return Ty::Error;
        }
    }
    match t.name.as_str() {
        "Int" => Ty::int(),
        "Float" => Ty::float(),
        "Bool" => Ty::bool(),
        "String" => Ty::str(),
        "Unit" => Ty::unit(),
        other => match cx.known.get(other) {
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
                for (j, a) in t.args.iter().enumerate() {
                    let param = Kinds::param_name(&cx.kinds.types, other, j);
                    args.push(if cx.kinds.type_row(other, j) {
                        decl_row_arg(inf, a, cx, other, param)
                    } else {
                        decl_type_arg(inf, a, cx, other, param)
                    });
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

/// E0432 for a row variable a declaration does not declare (part 1 refused
/// every one; D3 makes a declared one a row parameter).
fn undeclared_row_var(v: &str, span: Span, decl: &str) -> Diagnostic {
    Diagnostic::error(
        "E0432",
        format!("a declaration's row cannot name a row variable (`{v}`) it does not declare"),
    )
    .with_label(span, "a row variable in a declaration")
    .with_help(format!(
        "declare it as a parameter (`{decl}({v})`), or write the row's effects: a declaration's unwritten row is pure"
    ))
}

/// A row parameter's argument in a declaration (D5): a row, or one of the
/// declaration's row parameters alone.
fn decl_row_arg(inf: &mut Infer, a: &Spanned<TypeAnn>, cx: &DeclCx, decl: &str, param: &str) -> Ty {
    let t = &a.node;
    if t.as_row_arg().is_some() {
        return elaborate_adt_inner(inf, a, cx);
    }
    if t.args.is_empty() && t.row.is_none() && t.name.starts_with(|c: char| c.is_ascii_lowercase())
    {
        if let Some(&rv) = cx.row_env.get(&t.name) {
            return phantom(EffectRow::open(rv));
        }
        if !cx.param_env.contains_key(&t.name) {
            inf.diags.push(undeclared_row_var(&t.name, a.span, cx.decl));
            return Ty::Error;
        }
    }
    inf.diags
        .push(Diagnostic::error("E0400", wants_row(decl, param)).with_label(a.span, "a type here"));
    Ty::Error
}

/// A type parameter's argument in a declaration: a type, not a row (D5).
fn decl_type_arg(
    inf: &mut Infer,
    a: &Spanned<TypeAnn>,
    cx: &DeclCx,
    decl: &str,
    param: &str,
) -> Ty {
    if a.node.as_row_arg().is_some() {
        inf.diags.push(
            Diagnostic::error("E0400", wants_type(decl, param)).with_label(a.span, "a row here"),
        );
        return Ty::Error;
    }
    elaborate_adt_ty(inf, a, cx)
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
    let (schemes, diags, _sites, _typed, _fns) = infer_all(module, false);
    (schemes, diags)
}

/// Inference plus the affine binding-site set (Slice 4d-2): which let-bindings /
/// params have a `linear`-declared type. The one bounded reach the `affine::check`
/// pass consumes; the affine *logic* lives entirely in that pass.
pub fn infer_with_sites(_session: &Session, module: &Module) -> (Vec<Diagnostic>, HashSet<Span>) {
    let (_schemes, diags, sites, _typed, _fns) = infer_all(module, false);
    (diags, sites)
}

/// [`infer_with_sites`] plus every top-level function's generalized type, by
/// name: a module's interface (spec 2026-10-24-elya-modules-part-2 D1).
pub fn infer_with_interfaces(
    _session: &Session,
    module: &Module,
) -> (Vec<Diagnostic>, HashSet<Span>, HashMap<String, Ty>) {
    let (_schemes, diags, sites, _typed, fns) = infer_all(module, false);
    (diags, sites, fns)
}

/// Inference plus the per-node type table (Slice 5a-1): every expression node's
/// zonked type, rendered span-sorted through one shared `Names` so a variable
/// shared across nodes renders identically. The Core IR arc's feeder; 5a-2's
/// lowering consumes the underlying `node_types` directly.
pub fn infer_with_types(
    _session: &Session,
    module: &Module,
) -> (Vec<Diagnostic>, BTreeMap<Span, String>) {
    let (_schemes, diags, _sites, typed, _fns) = infer_all(module, true);
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
    let (_schemes, diags, _sites, typed, _fns) = infer_all(module, true);
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
    HashMap<String, Ty>,
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
            if t.variants.is_empty() {
                inf.empty_types.insert(t.name.clone());
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
    // Each declaration's parameters' kinds (spec 2026-10-30-elya-row-variables
    // D3), before any declaration is elaborated.
    let kinds = decl_kinds(module);
    inf.kinds = kinds.clone();
    // A declaration's parameters: a type parameter is a type variable; a row
    // parameter is a row variable, its argument the variable's phantom (D4).
    struct DeclParams {
        args: Vec<Ty>,
        var_ids: Vec<u32>,
        row_vars: Vec<Option<RowVar>>,
        param_env: HashMap<String, Ty>,
        row_env: HashMap<String, RowVar>,
    }
    let decl_params = |inf: &mut Infer, ps: &[(String, bool)]| -> DeclParams {
        let mut out = DeclParams {
            args: Vec::new(),
            var_ids: Vec::new(),
            row_vars: Vec::new(),
            param_env: HashMap::new(),
            row_env: HashMap::new(),
        };
        for (p, is_row) in ps {
            let v = inf.fresh();
            let Ty::Var(id) = v else { unreachable!() };
            out.var_ids.push(id);
            if *is_row {
                let rv = inf.fresh_row();
                out.row_env.insert(p.clone(), rv);
                out.row_vars.push(Some(rv));
                out.args.push(phantom(EffectRow::open(rv)));
            } else {
                out.param_env.insert(p.clone(), v.clone());
                out.row_vars.push(None);
                out.args.push(v);
            }
        }
        out
    };
    for d in &module.decls {
        if let Decl::Type(t) = &d.node {
            let dp = decl_params(&mut inf, &kinds.types[&t.name]);
            let var_ids: Vec<u32> = dp
                .var_ids
                .iter()
                .zip(&dp.row_vars)
                .filter(|(_, r)| r.is_none())
                .map(|(v, _)| *v)
                .collect();
            let row_vars: Vec<RowVar> = dp.row_vars.iter().flatten().copied().collect();
            let cx = DeclCx {
                decl: &t.name,
                param_env: &dp.param_env,
                row_env: &dp.row_env,
                known: &known_types,
                effects: &effect_arity,
                kinds: &kinds,
                own: None,
            };
            let result = Ty::Con(t.name.clone(), dp.args.clone());
            let mut ctors: Vec<Vec<Ty>> = Vec::new();
            for v in &t.variants {
                let vd = &v.node;
                inf.ctor_arity.insert(vd.name.clone(), vd.fields.len());
                let mut field_tys = Vec::new();
                for f in &vd.fields {
                    field_tys.push(elaborate_adt_ty(&mut inf, f, &cx));
                }
                ctors.push(field_tys.clone());
                let ty = if field_tys.is_empty() {
                    result.clone()
                } else {
                    Ty::Fn(field_tys, EffectRow::pure(), Box::new(result.clone()))
                };
                env.insert(
                    &vd.name,
                    Scheme {
                        vars: var_ids.clone(),
                        row_vars: row_vars.clone(),
                        ty,
                    },
                );
            }
            inf.adt_fields
                .insert(t.name.clone(), (var_ids.clone(), ctors));
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
            // A row parameter is a row variable, and the effect's own label in
            // its declaration carries its phantom (D3, D4).
            let dp = decl_params(&mut inf, &kinds.effects[&e.name]);
            let cx = DeclCx {
                decl: &e.name,
                param_env: &dp.param_env,
                row_env: &dp.row_env,
                known: &known_types,
                effects: &effect_arity,
                kinds: &kinds,
                own: Some((&e.name, &dp.args)),
            };
            for op in &e.ops {
                let sig = &op.node;
                let params: Vec<Ty> = sig
                    .param_tys
                    .iter()
                    .map(|t| elaborate_adt_ty(&mut inf, t, &cx))
                    .collect();
                let ret = elaborate_adt_ty(&mut inf, &sig.ret, &cx);
                inf.ops.insert(
                    sig.name.clone(),
                    OpInfo {
                        effect: e.name.clone(),
                        effect_params: dp.var_ids.clone(),
                        row_params: dp.row_vars.clone(),
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
    // Each function's declaration span, in `fns` order: its whole type (its own
    // row included) is recorded there for Core (spec 2026-10-16).
    let decl_spans: Vec<Span> = module
        .decls
        .iter()
        .filter(|d| matches!(d.node, Decl::Fn(_)))
        .map(|d| d.span)
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
    let mut fn_types: Vec<(String, Ty)> = Vec::new();

    for group in &groups {
        // 1. fresh monotype per member (with a fresh open ambient row), in scope
        //    for the whole group.
        let mut member_ty: HashMap<usize, (Vec<Ty>, RowVar, Ty)> = HashMap::new();
        let mut ann_vars_of: HashMap<usize, AnnNames> = HashMap::new();
        let mut declared_args_of: HashMap<usize, Vec<(String, Vec<Ty>, Span)>> = HashMap::new();
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
            inf.ann_rows = HashMap::new();
            for (p, pty) in f.params.iter().zip(&params) {
                inf.check_ann(&p.node.ann, pty, false);
            }
            inf.check_ann(&f.ret_ann, &result, true);
            if let Some(row) = &f.effect_row {
                let args = inf.declared_label_args(row);
                declared_args_of.insert(i, args);
            }
            ann_vars_of.insert(
                i,
                (
                    std::mem::take(&mut inf.ann_vars),
                    std::mem::take(&mut inf.ann_rows),
                ),
            );
            // A function's own row is in no expression's type: recorded at its
            // declaration's span, pre-zonk like every entry (the single zonk at
            // the end resolves it), for `CoreFn::row` (spec 2026-10-16).
            inf.node_types.insert(
                decl_spans[i],
                Ty::Fn(
                    params.clone(),
                    EffectRow::open(amb_f),
                    Box::new(result.clone()),
                ),
            );
            member_ty.insert(i, (params, amb_f, result));
        }
        inf.group_ambs = group.iter().map(|i| member_ty[i].1).collect();
        inf.group_params = group
            .iter()
            .flat_map(|i| member_ty[i].0.iter().cloned())
            .collect();
        // 2. infer each body under its params + ambient (monomorphic in-group).
        let mut named_of: HashMap<usize, HashMap<String, RowVar>> = HashMap::new();
        for &i in group {
            let f = fns[i];
            let (params, amb_f, result) = member_ty[&i].clone();
            let (vars, rows) = ann_vars_of.remove(&i).unwrap_or_default();
            inf.ann_vars = vars;
            inf.ann_rows = rows;
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
            // This member's row names, its body's included, for its declared
            // row (2.48; the review's F3: the last member's were read).
            named_of.insert(i, std::mem::take(&mut inf.ann_rows));
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
        // 2.45. Deferred lambda rows (recursion through a lambda): the group's
        //       labels are all in, so they close now, unless relayed through
        //       a parameter in the lambda's scope or a member's.
        let group_params = inf.group_params.clone();
        for (l, mut params) in std::mem::take(&mut inf.deferred_rows) {
            params.extend(group_params.iter().cloned());
            inf.close_unrelayed_residual(l, &params);
        }
        // 2.47. A declared row's written label arguments (spec
        //       2026-10-30-elya-row-variables D5): what the body performs is at
        //       them. Before anything closes, like 2.48.
        for &i in group {
            let amb_f = member_ty[&i].1;
            for (name, want, span) in declared_args_of.remove(&i).unwrap_or_default() {
                let performed = inf.resolve_row(&EffectRow::open(amb_f));
                if let Some(have) = performed.labels.get(&name) {
                    let have = have.args.clone();
                    inf.unify_effect_args(&name, &have, &want, span);
                }
            }
        }
        // 2.48. A declared row that ends in a named variable (spec
        //       2026-10-30-elya-row-variables D2), with its own member's names,
        //       BEFORE anything closes: closing first forced the name pure
        //       wherever else it is written (the review's F2).
        for &i in group {
            let f = fns[i];
            let (_params, amb_f, _result) = member_ty[&i].clone();
            if let Some(declared) = &f.effect_row {
                if let Some(v) = &declared.tail {
                    let labels: Vec<Spanned<String>> =
                        declared.labels.iter().map(|l| l.name.clone()).collect();
                    inf.ann_rows = named_of.remove(&i).unwrap_or_default();
                    inf.check_named_row(amb_f, &labels, v);
                }
            }
        }
        inf.ann_rows.clear();
        // 2.5. close each member's residual ambient (once all in-group effects
        //      are accumulated), unless it is relayed through a parameter.
        for &i in group {
            let (params, amb_f, _result) = member_ty[&i].clone();
            inf.close_unrelayed_residual(amb_f, &params);
        }
        // 2.6. enforce annotated rows (exact match) and main's discharge set.
        //      A member's row with one effect met twice along it has its
        //      arguments unified first (`settle_duplicates`).
        for &i in group {
            let amb_f = member_ty[&i].1;
            let span = fns[i].body.span;
            inf.settle_duplicates(&EffectRow::open(amb_f), span);
        }
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
        inf.pending_member_incl.clear();
        inf.group_ambs.clear();
        inf.group_params.clear();
        let pending: Vec<RowVar> = inf.phantom.iter().copied().collect();
        for v in pending {
            if inf.row_subst[v as usize].is_none() {
                inf.bind_row(v, &EffectRow::pure(), Span::EMPTY);
            }
        }
        inf.phantom.clear();
        // 2.8. E0434: what `with any` handlers handle is data. A tail still
        //      open here is generalized: its instantiations are checked.
        inf.check_generic_rows();
        // 3. generalize each member and re-insert its polytype for later groups.
        for &i in group {
            let f = fns[i];
            let (params, amb_f, result) = member_ty[&i].clone();
            let fnty = Ty::Fn(params, EffectRow::open(amb_f), Box::new(result));
            let scheme = generalize_toplevel(&mut inf, &fnty, &env, group, &fns);
            env.insert(&f.name, scheme.clone());
            schemes_out.push((f.name.clone(), display_scheme(&inf, &scheme)));
            fn_types.push((f.name.clone(), scheme.ty.clone()));
        }
    }
    let fn_types: HashMap<String, Ty> = fn_types
        .into_iter()
        .map(|(n, t)| (n, inf.resolve(&t)))
        .collect();

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
    (
        schemes_out,
        inf.diags,
        inf.affine_sites,
        node_types,
        fn_types,
    )
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
        | Expr::Error
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
        // second recording path appeared. It moved 5 -> 6 on 2026-10-16 (spec
        // `2026-10-16-elya-own-row-in-core-design.md`): the declaration's span
        // now records the function's whole type, its own row included.
        assert_eq!(
            table.len(),
            6,
            "wrapper must record every node once: {table:?}"
        );
        // Ordering: all five node entries zonked to Int (a record-time table
        // would show vars), and the declaration's entry to `fn(Int) -> Int`.
        // The parameter entry is included, which is the whole reason §3.2 could
        // record it pre-zonk: the single zonk pass maps over the WHOLE table.
        assert_eq!(
            table.values().filter(|v| *v == "Int").count(),
            5,
            "record-then-zonk violated — a node kept its pre-zonk var: {table:?}"
        );
        assert!(
            table.values().any(|v| v == "fn(Int) -> Int"),
            "the declaration's entry: {table:?}"
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
