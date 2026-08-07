//! Hindley–Milner type inference (Algorithm J).

use crate::ast::{BinOp, Block, Decl, Expr, FnDecl, Module, Stmt, UnOp};
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
    pub ty: Ty,
}

pub struct Infer {
    subst: Vec<Option<Ty>>,
    row_subst: Vec<Option<EffectRow>>,
    pub diags: Vec<Diagnostic>,
}

impl Infer {
    pub fn new() -> Infer {
        Infer {
            subst: Vec::new(),
            row_subst: Vec::new(),
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
            // Effect rows are ignored here until Task 4 threads real rows; every
            // arrow is pure at this stage, so the rows are trivially equal.
            (Ty::Fn(p1, _row1, r1), Ty::Fn(p2, _row2, r2)) => {
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

    /// Fold every label of `eff` into `amb` (pour a callee's row into ambient).
    pub fn add_row(&mut self, amb: RowVar, eff: &EffectRow, span: Span) -> Result<(), RowConflict> {
        self.unify_row(&EffectRow::open(amb), eff, span)
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
    // Seed quantified type variables first so they take the leading letters,
    // in declaration order.
    let quant: Vec<String> = s.vars.iter().map(|v| names.ty_name(*v)).collect();
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
        if s.vars.is_empty() {
            return s.ty.clone();
        }
        let mapping: HashMap<u32, Ty> = s.vars.iter().map(|v| (*v, self.fresh())).collect();
        subst_vars(&s.ty, &mapping)
    }

    pub fn infer_block(&mut self, b: &Block, env: &mut TyEnv) -> Ty {
        env.push();
        for st in b.stmts.iter() {
            match &st.node {
                Stmt::Let { name, value } => {
                    let t = self.infer_expr(value, env);
                    let scheme = self.generalize(&t, env);
                    env.insert(name, scheme);
                }
                Stmt::Expr(e) => {
                    self.infer_expr(e, env);
                }
            }
        }
        let result = match &b.tail {
            Some(tail) => self.infer_expr(tail, env),
            None => Ty::unit(),
        };
        env.pop();
        result
    }

    pub fn infer_expr(&mut self, e: &Spanned<Expr>, env: &mut TyEnv) -> Ty {
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
                let t = self.infer_expr(expr, env);
                let (operand, result) = match op {
                    UnOp::Neg => (Ty::int(), Ty::int()),
                    UnOp::Not => (Ty::bool(), Ty::bool()),
                };
                self.unify(&t, &operand, span);
                result
            }
            Expr::Binary { op, lhs, rhs } => {
                let lt = self.infer_expr(lhs, env);
                let rt = self.infer_expr(rhs, env);
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
                let ct = self.infer_expr(cond, env);
                self.unify_cond(&ct, cond.span);
                let tt = self.infer_block(&then_block.node, env);
                let et = self.infer_block(&else_block.node, env);
                self.unify(&tt, &et, span);
                tt
            }
            Expr::Block(b) => self.infer_block(b, env),
            Expr::Call { callee, args } => self.infer_call(callee, args, span, env),
            Expr::Handle { .. } | Expr::Resume { .. } => {
                self.diags.push(
                    Diagnostic::error("E0499", "effects are not type-checked yet (Slice 3b)")
                        .with_label(span, "unsupported here"),
                );
                Ty::Error
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
    ) -> Ty {
        if let Expr::Qualified { module, name } = &callee.node {
            if module == "io" && name == "println" {
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env)).collect();
                let want = Ty::Fn(vec![Ty::str()], EffectRow::pure(), Box::new(Ty::unit()));
                let got = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(Ty::unit()));
                self.unify(&want, &got, span);
                return Ty::unit();
            }
            return Ty::Error; // unknown builtin is E0201 from resolution
        }
        let f = self.infer_expr(callee, env);
        let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env)).collect();
        let result = self.fresh();
        let expected = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(result.clone()));
        self.unify(&f, &expected, span);
        result
    }

    /// Generalize `t` under `env`: quantify the vars free in `t` but not in the
    /// environment. (The level/rank optimization is a valid future speedup;
    /// this free-vars version produces identical schemes.)
    fn generalize(&mut self, t: &Ty, env: &TyEnv) -> Scheme {
        let resolved = self.resolve(t);
        let mut in_ty = Vec::new();
        free_vars(self, &resolved, &mut in_ty);
        let mut in_env = Vec::new();
        env_free_vars(self, env, &mut in_env);
        let vars: Vec<u32> = in_ty.into_iter().filter(|v| !in_env.contains(v)).collect();
        Scheme { ty: resolved, vars }
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

fn subst_vars(t: &Ty, m: &HashMap<u32, Ty>) -> Ty {
    match t {
        Ty::Var(v) => m.get(v).cloned().unwrap_or(Ty::Var(*v)),
        Ty::Base(c) => Ty::Base(*c),
        Ty::Fn(ps, row, r) => Ty::Fn(
            ps.iter().map(|p| subst_vars(p, m)).collect(),
            row.clone(), // row instantiation is Task 4
            Box::new(subst_vars(r, m)),
        ),
        Ty::Tuple(xs) => Ty::Tuple(xs.iter().map(|x| subst_vars(x, m)).collect()),
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
        // 1. fresh monotype per member, in scope for the whole group.
        let mut member_ty: HashMap<usize, (Vec<Ty>, Ty)> = HashMap::new();
        for &i in group {
            let f = fns[i];
            let params: Vec<Ty> = f.params.iter().map(|_| inf.fresh()).collect();
            let result = inf.fresh();
            env.insert(
                &f.name,
                Scheme {
                    vars: Vec::new(),
                    ty: Ty::Fn(params.clone(), EffectRow::pure(), Box::new(result.clone())),
                },
            );
            member_ty.insert(i, (params, result));
        }
        // 2. infer each body under its params (monomorphic within the group).
        for &i in group {
            let f = fns[i];
            let (params, result) = &member_ty[&i];
            env.push();
            for (p, pty) in f.params.iter().zip(params) {
                env.insert(
                    &p.node.name,
                    Scheme {
                        vars: Vec::new(),
                        ty: pty.clone(),
                    },
                );
            }
            let body_ty = inf.infer_block(&f.body.node, &mut env);
            inf.unify(&body_ty, result, f.body.span);
            env.pop();
        }
        // 3. generalize each member and re-insert its polytype for later groups.
        for &i in group {
            let f = fns[i];
            let (params, result) = &member_ty[&i];
            let fnty = Ty::Fn(params.clone(), EffectRow::pure(), Box::new(result.clone()));
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
    let mut in_env = Vec::new();
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
        }
    }
    let vars: Vec<u32> = in_ty.into_iter().filter(|v| !in_env.contains(v)).collect();
    Scheme { ty: resolved, vars }
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
        let t = inf.infer_expr(&e, &mut env);
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
