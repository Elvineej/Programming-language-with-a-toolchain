//! The affine pass (spec 2026-11-06-elya-affine-across-functions D2, D3, D7):
//! what inference needs to know about uses, read off the syntax before it runs.
//!
//! - Every local binding used *many* times (D2). Uses in sequence add; the arms
//!   of an `if` or a `match` take the larger count; a use inside a lambda counts
//!   as often as the lambda runs (a `let`-bound lambda as often as its name is
//!   called, its name used any other way counting many; a lambda called where
//!   it is written once; any other lambda many); a use inside an operation
//!   clause many (a clause runs once per perform), inside a return clause once.
//! - What each *point* holds (D3): a point is a call, a `resume` or a whole
//!   `handle`; it holds the bindings bound before it and used after it in the
//!   same body (a function's, a lambda's, a clause's; a `handle` body's
//!   continues into its return clause), each with its first use after the
//!   point. A use of a `let`-bound lambda is a use of what it captures (D4).
//!   Read only for a program that declares a `multi` effect.
//!
//! Inference imposes both (`types.rs`): a binding used many times, or held by a
//! point that may resume more than once, must have a duplicable type.
//!
//! A binding is keyed by its binder's span: a parameter's (of a function, a
//! lambda or a clause), a `let`'s value's, a pattern variable's, and a return
//! clause's body's (its binder has no span of its own).

use crate::ast::*;
use crate::span::{Span, Spanned};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Why a binding is used many times, for its diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// Two uses that can both run.
    Twice,
    /// A use in this lambda, which may run more than once.
    Closure(Span),
    /// A use in this operation clause, which runs once per perform.
    Clause(Span),
}

/// A binding used many times: the use that made it many, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Many {
    pub at: Span,
    pub why: Why,
}

#[derive(Default, Debug)]
pub struct Facts {
    /// The bindings used many times, by binder (D2).
    pub many: HashMap<Span, Many>,
    /// What each point holds, by the point's span: each binding with its first
    /// use after the point, in binder order (D3).
    pub held: HashMap<Span, Vec<(Span, Span)>>,
    /// Each `handle`'s `resume`s (in its clauses, the lambdas there included),
    /// by the `handle`'s span: points whose rows are the `handle`'s.
    pub resumes: HashMap<Span, Vec<Span>>,
}

pub fn analyze(module: &Module) -> Facts {
    let fns: Vec<&FnDecl> = module
        .decls
        .iter()
        .filter_map(|d| match &d.node {
            Decl::Fn(f) => Some(f),
            _ => None,
        })
        .collect();
    let mut r = Resolver::default();
    for f in &fns {
        r.function(f);
    }
    let mut facts = Facts::default();
    let mut c = Counter {
        occ: &r.occ,
        many: &mut facts.many,
        bodies: HashMap::new(),
    };
    for f in &fns {
        c.function(f);
    }
    let any_multi = module
        .decls
        .iter()
        .any(|d| matches!(&d.node, Decl::Effect(e) if e.is_multi));
    if any_multi {
        let ctors: HashSet<&str> = module
            .decls
            .iter()
            .filter_map(|d| match &d.node {
                Decl::Type(t) => Some(t),
                _ => None,
            })
            .flat_map(|t| t.variants.iter().map(|v| v.node.name.as_str()))
            .collect();
        let mut l = Liveness {
            occ: &r.occ,
            caps: &r.caps,
            ctors: &ctors,
            held: &mut facts.held,
            resumes: &mut facts.resumes,
            clauses: Vec::new(),
            pending: Vec::new(),
        };
        for f in &fns {
            l.function(f);
        }
    }
    facts
}

/// The variables a pattern binds, with their spans.
fn pattern_vars(p: &Spanned<Pattern>, out: &mut Vec<(String, Span)>) {
    match &p.node {
        Pattern::Var(x) => out.push((x.clone(), p.span)),
        Pattern::Ctor { args, .. } => {
            for a in args {
                pattern_vars(a, out);
            }
        }
        Pattern::Wild | Pattern::Lit(_) => {}
    }
}

fn param_names(ps: &[Spanned<Param>]) -> Vec<(String, Span)> {
    ps.iter().map(|p| (p.node.name.clone(), p.span)).collect()
}

/// Each use's binding, and what each lambda captures.
#[derive(Default)]
struct Resolver {
    /// The bindings in scope, innermost last.
    scope: Vec<Span>,
    /// Each name's positions in `scope`, innermost last.
    index: HashMap<String, Vec<usize>>,
    /// The lambdas open around the walk: `scope`'s length when each began,
    /// and the bindings from before it that it uses.
    frames: Vec<(usize, HashSet<Span>)>,
    /// A use (a `Var`'s span) -> its binding.
    occ: HashMap<Span, Span>,
    /// A lambda (its span: a `let` binding it is keyed by the same span) ->
    /// what it captures, through the `let`-bound lambdas it uses.
    caps: HashMap<Span, Vec<Span>>,
}

impl Resolver {
    fn bind_all(&mut self, names: &[(String, Span)]) {
        for (n, k) in names {
            self.index
                .entry(n.clone())
                .or_default()
                .push(self.scope.len());
            self.scope.push(*k);
        }
    }

    fn unbind_all(&mut self, names: &[(String, Span)]) {
        for (n, _) in names.iter().rev() {
            if let Some(v) = self.index.get_mut(n) {
                v.pop();
            }
            self.scope.pop();
        }
    }

    fn function(&mut self, f: &FnDecl) {
        let params = param_names(&f.params);
        self.bind_all(&params);
        self.block(&f.body.node);
        self.unbind_all(&params);
    }

    fn block(&mut self, b: &Block) {
        let mut names: Vec<(String, Span)> = Vec::new();
        for st in b.stmts.iter() {
            match &st.node {
                Stmt::Let { name, value, .. } => {
                    self.expr(value);
                    let n = vec![(name.clone(), value.span)];
                    self.bind_all(&n);
                    names.extend(n);
                }
                Stmt::Expr(e) => self.expr(e),
            }
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
        self.unbind_all(&names);
    }

    fn scoped(&mut self, names: &[(String, Span)], body: &Spanned<Expr>) {
        self.bind_all(names);
        self.expr(body);
        self.unbind_all(names);
    }

    fn var(&mut self, name: &str, span: Span) {
        let Some(&i) = self.index.get(name).and_then(|v| v.last()) else {
            return;
        };
        let b = self.scope[i];
        self.occ.insert(span, b);
        let extra = self.caps.get(&b).cloned().unwrap_or_default();
        for (start, free) in self.frames.iter_mut().rev() {
            if i >= *start {
                break;
            }
            free.insert(b);
            free.extend(extra.iter().copied());
        }
    }

    fn expr(&mut self, e: &Spanned<Expr>) {
        match &e.node {
            Expr::Var(name) => self.var(name, e.span),
            Expr::Call { callee, args } => {
                self.expr(callee);
                for a in args.iter() {
                    self.expr(a);
                }
            }
            Expr::Lambda { params, body } => {
                self.frames.push((self.scope.len(), HashSet::new()));
                let ps = param_names(params);
                self.bind_all(&ps);
                self.block(&body.node);
                self.unbind_all(&ps);
                let (_, free) = self.frames.pop().expect("a lambda's frame");
                let mut free: Vec<Span> = free.into_iter().collect();
                free.sort();
                self.caps.insert(e.span, free);
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            Expr::Unary { expr, .. } => self.expr(expr),
            Expr::If {
                cond,
                then_block,
                else_block,
            } => {
                self.expr(cond);
                self.block(&then_block.node);
                self.block(&else_block.node);
            }
            Expr::Block(b) => self.block(b),
            Expr::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for arm in arms.iter() {
                    let mut vars = Vec::new();
                    pattern_vars(&arm.node.pat, &mut vars);
                    self.scoped(&vars, &arm.node.body);
                }
            }
            Expr::Handle { body, handler } => {
                self.expr(body);
                if let Some(r) = &handler.ret {
                    self.scoped(&[(r.binder.clone(), r.body.span)], &r.body);
                }
                for c in &handler.clauses {
                    self.scoped(&param_names(&c.node.params), &c.node.body);
                }
            }
            Expr::Resume { arg } => self.expr(arg),
            Expr::Int(_)
            | Expr::Float(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Unit
            | Expr::Qualified { .. }
            | Expr::Error => {}
        }
    }
}

/// A count: one, or many (none is absent).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum M {
    One,
    Many,
}

#[derive(Clone, Copy, Debug)]
struct Cnt {
    uses: M,
    /// How often the binding is called (a use other than as a callee counts
    /// many): for a `let`-bound lambda, how often it runs.
    calls: M,
    first: Span,
    many: Option<Many>,
}

type Frame = HashMap<Span, Cnt>;

/// Two counts in sequence: many, the later use named.
fn seq(a: Cnt, b: Cnt) -> Cnt {
    let (early, late) = if a.first.start <= b.first.start {
        (a.first, b.first)
    } else {
        (b.first, a.first)
    };
    Cnt {
        uses: M::Many,
        calls: M::Many,
        first: early,
        many: a.many.or(b.many).or(Some(Many {
            at: late,
            why: Why::Twice,
        })),
    }
}

/// The larger of two arms' counts.
fn larger(a: Cnt, b: Cnt) -> Cnt {
    let (x, y) = if a.uses >= b.uses { (a, b) } else { (b, a) };
    Cnt {
        uses: x.uses,
        calls: x.calls.max(y.calls),
        first: if x.first.start <= y.first.start {
            x.first
        } else {
            y.first
        },
        many: x.many.or(y.many),
    }
}

fn add(acc: &mut Frame, mut other: Frame) {
    if other.len() > acc.len() {
        std::mem::swap(acc, &mut other);
    }
    for (k, c) in other {
        let merged = match acc.get(&k) {
            Some(a) => seq(*a, c),
            None => c,
        };
        acc.insert(k, merged);
    }
}

fn max_of(mut a: Frame, mut b: Frame) -> Frame {
    if b.len() > a.len() {
        std::mem::swap(&mut a, &mut b);
    }
    for (k, c) in b {
        let merged = match a.get(&k) {
            Some(x) => larger(*x, c),
            None => c,
        };
        a.insert(k, merged);
    }
    a
}

/// A body's counts when it runs `m` times (`None`: never).
fn scale(f: Frame, m: Option<M>, why: Why) -> Frame {
    match m {
        None => Frame::new(),
        Some(M::One) => f,
        Some(M::Many) => f
            .into_iter()
            .map(|(k, c)| {
                let many = c.many.or(Some(Many { at: c.first, why }));
                (
                    k,
                    Cnt {
                        uses: M::Many,
                        calls: M::Many,
                        first: c.first,
                        many,
                    },
                )
            })
            .collect(),
    }
}

struct Counter<'a> {
    occ: &'a HashMap<Span, Span>,
    many: &'a mut HashMap<Span, Many>,
    /// Each `let`-bound lambda's body counts, by its binding: counted where
    /// the lambda is called (once a call, many a use of any other kind), so
    /// a call in one arm and a use of a capture in the other are one use
    /// (the review's C-1).
    bodies: HashMap<Span, Frame>,
}

impl Counter<'_> {
    /// `key`'s scope ends: its count is final.
    fn finish(&mut self, f: &mut Frame, key: Span) {
        if let Some(c) = f.remove(&key) {
            if c.uses == M::Many {
                if let Some(m) = c.many {
                    self.many.insert(key, m);
                }
            }
        }
    }

    fn function(&mut self, f: &FnDecl) {
        let mut fr = self.block(&f.body.node);
        for p in &f.params {
            self.finish(&mut fr, p.span);
        }
    }

    /// Its `let`-bound lambdas' bodies first, forwards (a body may call an
    /// earlier one); then backwards (counts add in any order).
    fn block(&mut self, b: &Block) -> Frame {
        for st in b.stmts.iter() {
            if let Stmt::Let { value, .. } = &st.node {
                if let Expr::Lambda { params, body } = &value.node {
                    let inner = self.lambda(params, body);
                    self.bodies.insert(value.span, inner);
                }
            }
        }
        let mut acc = match &b.tail {
            Some(t) => self.expr(t),
            None => Frame::new(),
        };
        for st in b.stmts.iter().rev() {
            match &st.node {
                Stmt::Let { value, .. } => {
                    let key = value.span;
                    if let Expr::Lambda { .. } = &value.node {
                        self.finish(&mut acc, key);
                        self.bodies.remove(&key);
                    } else {
                        self.finish(&mut acc, key);
                        let v = self.expr(value);
                        add(&mut acc, v);
                    }
                }
                Stmt::Expr(e) => {
                    let v = self.expr(e);
                    add(&mut acc, v);
                }
            }
        }
        acc
    }

    fn lambda(&mut self, params: &[Spanned<Param>], body: &Spanned<Block>) -> Frame {
        let mut fr = self.block(&body.node);
        for p in params {
            self.finish(&mut fr, p.span);
        }
        fr
    }

    fn occurrence(&self, span: Span, callee: bool) -> Frame {
        let mut f = Frame::new();
        if let Some(&b) = self.occ.get(&span) {
            if let Some(body) = self.bodies.get(&b) {
                let runs = if callee { M::One } else { M::Many };
                f = scale(body.clone(), Some(runs), Why::Closure(b));
            }
            f.insert(
                b,
                Cnt {
                    uses: M::One,
                    calls: if callee { M::One } else { M::Many },
                    first: span,
                    many: None,
                },
            );
        }
        f
    }

    fn expr(&mut self, e: &Spanned<Expr>) -> Frame {
        match &e.node {
            Expr::Var(_) => self.occurrence(e.span, false),
            Expr::Call { callee, args } => {
                let mut acc = match &callee.node {
                    Expr::Var(_) => self.occurrence(callee.span, true),
                    Expr::Lambda { params, body } => self.lambda(params, body),
                    _ => self.expr(callee),
                };
                for a in args.iter() {
                    let f = self.expr(a);
                    add(&mut acc, f);
                }
                acc
            }
            Expr::Lambda { params, body } => {
                let inner = self.lambda(params, body);
                scale(inner, Some(M::Many), Why::Closure(e.span))
            }
            Expr::Binary { lhs, rhs, .. } => {
                let mut a = self.expr(lhs);
                let b = self.expr(rhs);
                add(&mut a, b);
                a
            }
            Expr::Unary { expr, .. } => self.expr(expr),
            Expr::If {
                cond,
                then_block,
                else_block,
            } => {
                let mut a = self.expr(cond);
                let t = self.block(&then_block.node);
                let el = self.block(&else_block.node);
                add(&mut a, max_of(t, el));
                a
            }
            Expr::Block(b) => self.block(b),
            Expr::Match { scrutinee, arms } => {
                let mut a = self.expr(scrutinee);
                let mut best = Frame::new();
                for arm in arms.iter() {
                    let mut f = self.expr(&arm.node.body);
                    let mut vars = Vec::new();
                    pattern_vars(&arm.node.pat, &mut vars);
                    for (_, k) in vars {
                        self.finish(&mut f, k);
                    }
                    best = max_of(best, f);
                }
                add(&mut a, best);
                a
            }
            Expr::Handle { body, handler } => {
                let mut a = self.expr(body);
                if let Some(r) = &handler.ret {
                    let mut f = self.expr(&r.body);
                    self.finish(&mut f, r.body.span);
                    add(&mut a, f);
                }
                for c in &handler.clauses {
                    let mut f = self.expr(&c.node.body);
                    for p in &c.node.params {
                        self.finish(&mut f, p.span);
                    }
                    add(&mut a, scale(f, Some(M::Many), Why::Clause(c.span)));
                }
                a
            }
            Expr::Resume { arg } => self.expr(arg),
            Expr::Int(_)
            | Expr::Float(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Unit
            | Expr::Qualified { .. }
            | Expr::Error => Frame::new(),
        }
    }
}

/// The bindings live at a point of the backward walk, each with its first use
/// after the point.
type Live = BTreeMap<Span, Span>;

fn join(s: &mut Live, t: Live) {
    for (b, u) in t {
        s.entry(b)
            .and_modify(|x| {
                if u.start < x.start {
                    *x = u;
                }
            })
            .or_insert(u);
    }
}

struct Liveness<'a> {
    occ: &'a HashMap<Span, Span>,
    caps: &'a HashMap<Span, Vec<Span>>,
    ctors: &'a HashSet<&'a str>,
    held: &'a mut HashMap<Span, Vec<(Span, Span)>>,
    resumes: &'a mut HashMap<Span, Vec<Span>>,
    /// The `handle`s whose clauses the walk is in, innermost last.
    clauses: Vec<Span>,
    /// Values computed and waiting while a later operand is evaluated (a
    /// call's callee and earlier arguments, a binary operation's left side):
    /// a point there re-runs the rest with them (the review's U-A). Cleared
    /// on entering a body of its own.
    pending: Vec<(Span, Span)>,
}

impl Liveness<'_> {
    /// What an evaluated operand holds: a binding (with what a `let`-bound
    /// lambda captures), a lambda's captures, or the value itself (by its
    /// span; its type is read from inference's node table).
    fn value_of(&self, e: &Spanned<Expr>, out: &mut Vec<(Span, Span)>) {
        match &e.node {
            Expr::Var(_) => {
                if let Some(&b) = self.occ.get(&e.span) {
                    out.push((b, e.span));
                    if let Some(cs) = self.caps.get(&b) {
                        out.extend(cs.iter().map(|c| (*c, e.span)));
                    }
                }
            }
            Expr::Lambda { .. } => {
                if let Some(cs) = self.caps.get(&e.span) {
                    out.extend(cs.iter().map(|c| (*c, e.span)));
                }
            }
            Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Unit => {}
            Expr::Qualified { .. } | Expr::Error => {}
            _ => out.push((e.span, e.span)),
        }
    }

    /// Walk `e` with `extra` waiting.
    fn with_pending(&mut self, extra: Vec<(Span, Span)>, e: &Spanned<Expr>, s: &mut Live) {
        let n = self.pending.len();
        self.pending.extend(extra);
        self.expr(e, s);
        self.pending.truncate(n);
    }

    /// Walk a body of its own: nothing outside it waits there.
    fn own_body(&mut self, f: impl FnOnce(&mut Self)) {
        let saved = std::mem::take(&mut self.pending);
        f(self);
        self.pending = saved;
    }

    fn function(&mut self, f: &FnDecl) {
        let mut s = Live::new();
        self.block(&f.body.node, &mut s);
    }

    fn point(&mut self, at: Span, s: &Live) {
        let mut all = s.clone();
        for (b, u) in &self.pending {
            all.entry(*b).or_insert(*u);
        }
        self.held.insert(at, all.into_iter().collect());
    }

    fn block(&mut self, b: &Block, s: &mut Live) {
        if let Some(t) = &b.tail {
            self.expr(t, s);
        }
        for st in b.stmts.iter().rev() {
            match &st.node {
                Stmt::Let { value, .. } => {
                    s.remove(&value.span);
                    self.expr(value, s);
                }
                Stmt::Expr(e) => self.expr(e, s),
            }
        }
    }

    /// A call of a constructor, or of a builtin other than `op.perform`,
    /// performs nothing: not a point.
    fn is_point(&self, callee: &Spanned<Expr>) -> bool {
        match &callee.node {
            Expr::Var(n) => self.occ.contains_key(&callee.span) || !self.ctors.contains(n.as_str()),
            Expr::Qualified { module, name } => module == "op" && name == "perform",
            _ => true,
        }
    }

    fn expr(&mut self, e: &Spanned<Expr>, s: &mut Live) {
        match &e.node {
            Expr::Var(_) => {
                if let Some(&b) = self.occ.get(&e.span) {
                    s.insert(b, e.span);
                    if let Some(cs) = self.caps.get(&b) {
                        for c in cs {
                            s.insert(*c, e.span);
                        }
                    }
                }
            }
            Expr::Call { callee, args } => {
                if self.is_point(callee) {
                    self.point(e.span, s);
                }
                for (j, a) in args.iter().enumerate().rev() {
                    let mut waiting = Vec::new();
                    self.value_of(callee, &mut waiting);
                    for earlier in &args[..j] {
                        self.value_of(earlier, &mut waiting);
                    }
                    self.with_pending(waiting, a, s);
                }
                self.expr(callee, s);
            }
            Expr::Lambda { params, body } => {
                let mut inner = Live::new();
                self.own_body(|l| l.block(&body.node, &mut inner));
                for p in params {
                    inner.remove(&p.span);
                }
                join(s, inner);
            }
            Expr::Binary { lhs, rhs, .. } => {
                let mut waiting = Vec::new();
                self.value_of(lhs, &mut waiting);
                self.with_pending(waiting, rhs, s);
                self.expr(lhs, s);
            }
            Expr::Unary { expr, .. } => self.expr(expr, s),
            Expr::If {
                cond,
                then_block,
                else_block,
            } => {
                let mut t = s.clone();
                self.block(&then_block.node, &mut t);
                self.block(&else_block.node, s);
                join(s, t);
                self.expr(cond, s);
            }
            Expr::Block(b) => self.block(b, s),
            Expr::Match { scrutinee, arms } => {
                if !arms.is_empty() {
                    let out = std::mem::take(s);
                    for arm in arms.iter() {
                        let mut a = out.clone();
                        self.expr(&arm.node.body, &mut a);
                        let mut vars = Vec::new();
                        pattern_vars(&arm.node.pat, &mut vars);
                        for (_, k) in vars {
                            a.remove(&k);
                        }
                        join(s, a);
                    }
                }
                self.expr(scrutinee, s);
            }
            Expr::Handle { body, handler } => {
                let mut ret = Live::new();
                let mut cl = Live::new();
                let mut b = Live::new();
                // Its body, clauses and return clause are bodies of their own:
                // what escapes the `handle` is held by the `handle` itself.
                self.own_body(|l| {
                    if let Some(r) = &handler.ret {
                        l.expr(&r.body, &mut ret);
                        ret.remove(&r.body.span);
                    }
                    l.clauses.push(e.span);
                    for c in &handler.clauses {
                        let mut x = Live::new();
                        l.expr(&c.node.body, &mut x);
                        for p in &c.node.params {
                            x.remove(&p.span);
                        }
                        join(&mut cl, x);
                    }
                    l.clauses.pop();
                    b = ret.clone();
                    l.expr(body, &mut b);
                });
                self.point(e.span, s);
                join(s, b);
                join(s, cl);
                join(s, ret);
            }
            Expr::Resume { arg } => {
                self.point(e.span, s);
                if let Some(h) = self.clauses.last() {
                    self.resumes.entry(*h).or_default().push(e.span);
                }
                self.expr(arg, s);
            }
            Expr::Int(_)
            | Expr::Float(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Unit
            | Expr::Qualified { .. }
            | Expr::Error => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_module;
    use crate::Session;

    fn text(src: &str, s: Span) -> &str {
        &src[s.start as usize..s.end as usize]
    }

    #[test]
    fn bindings_used_many_times() {
        // `x` twice (the second use named); `y` once in each arm; `z` in a
        // closure called twice, counted at each call (and `g`); `w` in a closure never
        // called; `v` in a clause.
        let src = "effect E { fn e() -> Int }\n\
                   fn f(x, y, z, w, v, c) {\n\
                   let a = x + x\n\
                   let b = if c { y } else { y }\n\
                   let g = fn() { z }\n\
                   let h = fn() { w }\n\
                   let n = g() + g()\n\
                   handle { e() } with { E.e() -> resume(v) }\n\
                   }\n";
        let (m, _) = parse_module(&Session::new(), src);
        let facts = analyze(&m);
        let mut got: Vec<(String, String, Why)> = facts
            .many
            .iter()
            .map(|(k, m)| {
                (
                    text(src, *k).to_string(),
                    text(src, m.at).to_string(),
                    m.why,
                )
            })
            .collect();
        got.sort_by_key(|(k, _, _)| k.clone());
        let names: Vec<(&str, &str)> = got
            .iter()
            .map(|(k, a, _)| (k.as_str(), a.as_str()))
            .collect();
        assert_eq!(
            names,
            [("fn() { z }", "g"), ("v", "v"), ("x", "x"), ("z", "z")],
            "{got:?}"
        );
        let x_at = facts
            .many
            .iter()
            .find(|(k, _)| text(src, **k) == "x")
            .unwrap()
            .1
            .at;
        assert_eq!(&src[x_at.start as usize - 4..x_at.end as usize], "x + x");
        assert!(got
            .iter()
            .any(|(k, _, w)| k == "v" && matches!(w, Why::Clause(_))));
    }

    #[test]
    fn what_each_point_holds() {
        // At the first `flip()`: `a`, `u` (in the `handle`'s body) and `g`
        // with what it captures, `k`; not `b` (never used), not `x` (bound by
        // it). At the second, in the `handle`'s body: `u` alone. At the
        // `handle`: what follows it, not what only its body uses. `g()` holds
        // the value waiting on its left, `a + x + y` (an `Int`).
        let src = "effect multi F { fn flip() -> Int }\n\
                   fn h(a, b, k, u) {\n\
                   let g = fn() { k }\n\
                   let x = flip()\n\
                   let y = handle { let _ = flip()\n u } with multi { F.flip() -> resume(1) }\n\
                   a + x + y + g()\n\
                   }\n";
        let (m, _) = parse_module(&Session::new(), src);
        let facts = analyze(&m);
        let held_at = |start: usize, pat: &str| -> Vec<String> {
            let (_, held) = facts
                .held
                .iter()
                .find(|(s, _)| s.start as usize == start && text(src, **s).starts_with(pat))
                .unwrap_or_else(|| panic!("no point at {pat}"));
            let mut v: Vec<String> = held
                .iter()
                .map(|(b, _)| text(src, *b).to_string())
                .collect();
            v.sort();
            v
        };
        let flips: Vec<usize> = src.match_indices("flip()").map(|(i, _)| i).collect();
        // flips[0] is the declaration's; flips[3] is inside `F.flip()`.
        assert_eq!(held_at(flips[1], "flip()"), ["a", "fn() { k }", "k", "u"]);
        assert_eq!(held_at(flips[2], "flip()"), ["u"]);
        assert_eq!(held_at(src.find("g()").unwrap(), "g()"), ["a + x + y"]);
        assert_eq!(
            held_at(src.find("handle").unwrap(), "handle"),
            ["a", "flip()", "fn() { k }", "k"]
        );
        assert_eq!(facts.resumes.values().map(|v| v.len()).sum::<usize>(), 1);
    }
}
