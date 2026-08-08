//! Hindley–Milner type inference (Algorithm J).

use crate::ast::{BinOp, Block, Decl, Expr, FnDecl, Handler, Module, Stmt, TypeAnn, UnOp};
use crate::diag::Diagnostic;
use crate::span::{Span, Spanned};
use crate::Session;
use std::collections::{BTreeMap, HashMap};

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

/// An effect row: a set of effect labels (each with the span that introduced
/// it, for provenance) and a tail. Labels are kept sorted (`BTreeMap`) so the
/// printed order is stable and each label appears at most once (idempotent).
#[derive(Clone, Debug, PartialEq)]
pub struct EffectRow {
    pub labels: BTreeMap<String, Span>,
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
    params: Vec<Ty>,
    ret: Ty,
}

pub struct Infer {
    subst: Vec<Option<Ty>>,
    row_subst: Vec<Option<EffectRow>>,
    /// Operation name -> its elaborated signature. Populated from `effect`
    /// declarations before inference; a call to one of these is a *perform*.
    ops: HashMap<String, OpInfo>,
    /// Stack of `(B, R)` for the handler clause currently being typed: `resume`
    /// takes the operation's result type `B` and yields the handle's result `R`.
    resume_stack: Vec<(Ty, Ty)>,
    pub diags: Vec<Diagnostic>,
}

impl Infer {
    pub fn new() -> Infer {
        Infer {
            subst: Vec::new(),
            row_subst: Vec::new(),
            ops: HashMap::new(),
            resume_stack: Vec::new(),
            diags: Vec::new(),
        }
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
                        for (k, sp) in &bound.labels {
                            labels.entry(k.clone()).or_insert(*sp);
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
        // Matching labels are compatible (idempotent; monomorphic ops carry no
        // payloads to reconcile in Slice 3b). Split out each side's extras.
        let only1: Vec<(String, Span)> = r1
            .labels
            .iter()
            .filter(|(k, _)| !r2.labels.contains_key(*k))
            .map(|(k, s)| (k.clone(), *s))
            .collect();
        let only2: Vec<(String, Span)> = r2
            .labels
            .iter()
            .filter(|(k, _)| !r1.labels.contains_key(*k))
            .map(|(k, s)| (k.clone(), *s))
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
        extra: &[(String, Span)],
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
                let labels: BTreeMap<String, Span> = extra.iter().cloned().collect();
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
                bucket.extend(extra.iter().cloned());
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
        self.row_subst[v as usize] = Some(resolved);
    }

    /// Force `op ∈ amb`: unify the ambient with `{op@span} | Open(fresh)`, which
    /// rewrites `amb`'s tail to expose `op`. Never fails when `amb` is open.
    pub fn add_effect(&mut self, amb: RowVar, op: &str, span: Span) -> Result<(), RowConflict> {
        let fresh = self.fresh_row();
        let mut labels = BTreeMap::new();
        labels.insert(op.to_string(), span);
        let target = EffectRow {
            labels,
            tail: RowTail::Open(fresh),
        };
        self.unify_row(&EffectRow::open(amb), &target, span)
    }

    /// Pour a callee's latent row into the ambient: `amb ⊇ eff`. Adds each of
    /// `eff`'s labels (keeping `amb` open, so a *closed* callee row never forces
    /// the ambient closed), and relays a polymorphic tail into the ambient.
    pub fn add_row(&mut self, amb: RowVar, eff: &EffectRow, span: Span) -> Result<(), RowConflict> {
        let eff = self.resolve_row(eff);
        for (label, sp) in &eff.labels {
            self.add_effect(amb, label, *sp)?;
        }
        if let RowTail::Open(rho) = eff.tail {
            // The callee's polymorphic effects flow into the ambient.
            self.unify_row(&EffectRow::open(amb), &EffectRow::open(rho), span)?;
        }
        Ok(())
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
        Ty::Error => out.push_str("<error>"),
    }
}

/// Render an effect row as `{A, B}` (labels sorted, from the `BTreeMap`), with a
/// ` | e` residual when the tail is a genuinely-polymorphic row variable. Assumes
/// the row is already zonked (callers resolve the enclosing type first). Never
/// emits a raw row-variable token — the effect-diagnostic no-`%r` discipline.
fn write_row(row: &EffectRow, names: &mut Names, out: &mut String) {
    out.push('{');
    for (i, label) in row.labels.keys().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(label);
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
        let row_mapping: HashMap<RowVar, RowVar> =
            s.row_vars.iter().map(|v| (*v, self.fresh_row())).collect();
        subst_vars(&s.ty, &mapping, &row_mapping)
    }

    /// Infer a block, threading the ambient effect row `amb` through every
    /// sub-expression — sequencing unions effects into the same ambient.
    pub fn infer_block(&mut self, b: &Block, env: &mut TyEnv, amb: RowVar) -> Ty {
        env.push();
        for st in b.stmts.iter() {
            match &st.node {
                Stmt::Let { name, value } => {
                    let t = self.infer_expr(value, env, amb);
                    let scheme = self.generalize(&t, env);
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

    pub fn infer_expr(&mut self, e: &Spanned<Expr>, env: &mut TyEnv, amb: RowVar) -> Ty {
        let span = e.span;
        match &e.node {
            Expr::Int(_) => Ty::int(),
            Expr::Float(_) => Ty::float(),
            Expr::Str(_) => Ty::str(),
            Expr::Bool(_) => Ty::bool(),
            Expr::Unit => Ty::unit(),
            Expr::Var(name) => match env.lookup(name) {
                Some(s) => {
                    let s = s.clone();
                    self.instantiate(&s)
                }
                None => Ty::Error, // unresolved names are E0200 from resolution
            },
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
                    Some((b, r)) => {
                        // resume : (B) -> R — takes the operation's result, yields
                        // the handle's result. Its latent effect is the clause's
                        // ambient, already threaded, so nothing new is added here.
                        self.unify(&arg_ty, &b, span);
                        r
                    }
                    // resume outside a handler is E0210 at resolve time.
                    None => Ty::Error,
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
                let _ = self.add_effect(amb, "IO", span); // io.println performs {IO}
                return Ty::unit();
            }
            return Ty::Error; // unknown builtin is E0201 from resolution
        }
        // An operation call is a *perform*: it adds its effect to the ambient
        // and yields the operation's declared result type.
        if let Expr::Var(name) = &callee.node {
            if let Some(op) = self.ops.get(name).cloned() {
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
                let want = Ty::Fn(
                    op.params.clone(),
                    EffectRow::pure(),
                    Box::new(op.ret.clone()),
                );
                let got = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(op.ret.clone()));
                self.unify(&want, &got, span);
                let _ = self.add_effect(amb, &op.effect, span);
                return op.ret;
            }
        }
        // Ordinary function call: unify the arrow's param/result types, then pour
        // the callee's latent effect row into the ambient (`amb ⊇ ε_f`).
        let f = self.infer_expr(callee, env, amb);
        let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
        let result = self.fresh();
        let call_row = self.fresh_row();
        let expected = Ty::Fn(arg_ts, EffectRow::open(call_row), Box::new(result.clone()));
        self.unify(&f, &expected, span);
        let eff = self.resolve_row(&EffectRow::open(call_row));
        let _ = self.add_row(amb, &eff, span);
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
        // `e` may perform E plus a polymorphic remainder.
        let amb_in = self.fresh_row();
        if let Some(e) = &effect {
            let _ = self.add_effect(amb_in, e, span);
        }
        let body_ty = self.infer_expr(body, env, amb_in);

        // R — the handle's result type, shared by every clause and `return`.
        let result = self.fresh();
        for c in &handler.clauses {
            let clause = &c.node;
            let (params, b) = match self.ops.get(&clause.op) {
                Some(op) => (op.params.clone(), op.ret.clone()),
                None => (Vec::new(), Ty::Error),
            };
            env.push();
            for (idx, p) in clause.params.iter().enumerate() {
                let pty = params.get(idx).cloned().unwrap_or_else(|| self.fresh());
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
            self.resume_stack.push((b, result.clone()));
            let clause_ty = self.infer_expr(&clause.body, env, amb);
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
        let _ = self.add_row(amb, &residual, span);
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
                    .copied()
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
            .map(|(k, s)| (k.clone(), *s))
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
        for (label, sp) in &performed.labels {
            if label != "IO" {
                self.diags.push(
                    Diagnostic::error("E0420", format!("effect `{label}` is never handled"))
                        .with_label(*sp, format!("`{label}` is performed here"))
                        .with_help(format!(
                            "`main` may perform only {{IO}}; handle it with `handle … with {{ {label}.op(..) -> … }}`"
                        )),
                );
            }
        }
    }
}

/// Elaborate an operation-signature type annotation into a `Ty`. Slice 3b
/// operations are monomorphic over base types; anything else (a generic type,
/// a type variable, an unknown name) is `E0404` + `Ty::Error`.
fn elaborate_ty(inf: &mut Infer, ann: &Spanned<TypeAnn>) -> Ty {
    let t = &ann.node;
    if !t.args.is_empty() {
        inf.diags.push(
            Diagnostic::error("E0404", "unsupported type in effect operation").with_label(
                ann.span,
                "generic effect operations are not supported yet (Slice 3b)",
            ),
        );
        return Ty::Error;
    }
    match t.name.as_str() {
        "Int" => Ty::int(),
        "Float" => Ty::float(),
        "Bool" => Ty::bool(),
        "String" => Ty::str(),
        "Unit" => Ty::unit(),
        other => {
            inf.diags.push(
                Diagnostic::error(
                    "E0404",
                    format!("unknown type `{other}` in effect operation"),
                )
                .with_label(
                    ann.span,
                    "effect operations use base types: Int, Float, Bool, String, Unit",
                ),
            );
            Ty::Error
        }
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
        Ty::Fn(ps, _row, r) => {
            // Row variables are generalized separately (Task 4); this collects
            // only free *type* variables.
            for p in &ps {
                free_vars(inf, p, acc);
            }
            free_vars(inf, &r, acc);
        }
        Ty::Tuple(xs) => {
            for x in &xs {
                free_vars(inf, x, acc);
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
            free_row_vars(inf, &r, acc);
        }
        Ty::Tuple(xs) => {
            for x in &xs {
                free_row_vars(inf, x, acc);
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
            Ty::Fn(
                ps.iter().map(|p| subst_vars(p, m, rm)).collect(),
                EffectRow {
                    labels: row.labels.clone(),
                    tail,
                },
                Box::new(subst_vars(r, m, rm)),
            )
        }
        Ty::Tuple(xs) => Ty::Tuple(xs.iter().map(|x| subst_vars(x, m, rm)).collect()),
        Ty::Error => Ty::Error,
    }
}

pub fn infer(session: &Session, module: &Module) -> Vec<Diagnostic> {
    let (_schemes, diags) = infer_schemes(session, module);
    diags
}

/// Type every top-level function, processing mutually-recursive groups (SCCs of
/// the call graph) in dependency order so each group is generalized before later
/// groups use it — giving proper let-polymorphism across the top level.
pub fn infer_schemes(
    _session: &Session,
    module: &Module,
) -> (Vec<(String, String)>, Vec<Diagnostic>) {
    let mut inf = Infer::new();
    let mut env = TyEnv::new();

    // Elaborate effect declarations into the operation-signature table, so a
    // call to an operation is recognised as a perform during inference.
    for d in &module.decls {
        if let Decl::Effect(e) = &d.node {
            for op in &e.ops {
                let sig = &op.node;
                let params: Vec<Ty> = sig
                    .param_tys
                    .iter()
                    .map(|t| elaborate_ty(&mut inf, t))
                    .collect();
                let ret = elaborate_ty(&mut inf, &sig.ret);
                inf.ops.insert(
                    sig.name.clone(),
                    OpInfo {
                        effect: e.name.clone(),
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
            member_ty.insert(i, (params, amb_f, result));
        }
        // 2. infer each body under its params + ambient (monomorphic in-group).
        for &i in group {
            let f = fns[i];
            let (params, amb_f, result) = member_ty[&i].clone();
            env.push();
            for (p, pty) in f.params.iter().zip(&params) {
                env.insert(
                    &p.node.name,
                    Scheme {
                        vars: Vec::new(),
                        row_vars: Vec::new(),
                        ty: pty.clone(),
                    },
                );
            }
            let body_ty = inf.infer_block(&f.body.node, &mut env, amb_f);
            inf.unify(&body_ty, &result, f.body.span);
            env.pop();
        }
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

    (schemes_out, inf.diags)
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
        _ => {}
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
        labels.insert("Log".to_string(), Span::EMPTY);
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
            m.insert((*l).to_string(), Span::EMPTY);
        }
        EffectRow { labels: m, tail }
    }

    #[test]
    fn add_effect_extends_open_row() {
        let mut inf = Infer::new();
        let amb = inf.fresh_row();
        assert!(inf.add_effect(amb, "Log", Span::EMPTY).is_ok());
        let r = inf.resolve_row(&EffectRow::open(amb));
        assert!(r.labels.contains_key("Log"), "amb should now contain Log");
        assert!(matches!(r.tail, RowTail::Open(_)), "still open/polymorphic");
        assert!(inf.diags.is_empty());
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
    fn unknown_op_type_is_e0404() {
        let (m, _) =
            crate::parse::parse_module(&Session::new(), "effect E { fn op(x: Foo) -> Unit }\n");
        let (_s, d) = infer_schemes(&Session::new(), &m);
        assert!(d.iter().any(|x| x.code == "E0404"), "expected E0404: {d:?}");
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
