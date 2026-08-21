//! Slice 4d-2: the affine-resource analysis pass. A `linear type`'s values are
//! affine — used at most once (E0428) and never live across a perform of a
//! `multi` effect (E0429). Self-contained (shaped like `exhaust.rs`): it reads
//! the AST + the affine binding sites `infer` found + an op->multi table it
//! rebuilds from the effect declarations. Intra-function local, create-and-
//! consume; the callee-duplication and over-approximation gaps are documented
//! tracked obligations (spec §6).

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::{Span, Spanned};
use std::collections::{HashMap, HashSet};

/// Per-function state: each in-scope affine binding (by name) -> (use count,
/// crossed-a-multi-perform).
struct Ctx<'a> {
    affine_sites: &'a HashSet<Span>,
    // Read by the E0429 (multi-shot capture) path, added in Task 3.
    #[allow(dead_code)]
    multi_ops: &'a HashSet<String>,
    live: HashMap<String, (u32, bool)>,
    out: Vec<Diagnostic>,
}

pub fn check(module: &Module, affine_sites: &HashSet<Span>) -> Vec<Diagnostic> {
    // Operation names belonging to a `multi`-declared effect (rebuilt locally,
    // exactly as Slice 4d-1 built `Infer.effect_multi` — keeps the pass
    // self-contained, no dependency on inference internals).
    let mut multi_ops: HashSet<String> = HashSet::new();
    for d in &module.decls {
        if let Decl::Effect(e) = &d.node {
            if e.is_multi {
                for op in &e.ops {
                    multi_ops.insert(op.node.name.clone());
                }
            }
        }
    }
    let mut out = Vec::new();
    for d in &module.decls {
        if let Decl::Fn(f) = &d.node {
            let mut ctx = Ctx {
                affine_sites,
                multi_ops: &multi_ops,
                live: HashMap::new(),
                out: Vec::new(),
            };
            ctx.walk_block(&f.body.node);
            out.append(&mut ctx.out);
        }
    }
    out
}

impl<'a> Ctx<'a> {
    fn walk_block(&mut self, b: &Block) {
        for st in b.stmts.iter() {
            match &st.node {
                Stmt::Let { name, value } => {
                    self.walk_expr(value);
                    // A newly-bound affine value (its RHS span is the site).
                    if self.affine_sites.contains(&value.span) {
                        self.live.insert(name.clone(), (0, false));
                    }
                }
                Stmt::Expr(e) => self.walk_expr(e),
            }
        }
        if let Some(t) = &b.tail {
            self.walk_expr(t);
        }
    }

    fn walk_expr(&mut self, e: &Spanned<Expr>) {
        match &e.node {
            Expr::Var(name) => {
                if let Some((uses, _crossed)) = self.live.get_mut(name) {
                    *uses += 1;
                    if *uses == 2 {
                        self.out.push(
                            Diagnostic::error(
                                "E0428",
                                format!("affine value `{name}` used more than once"),
                            )
                            .with_label(e.span, "second use here")
                            .with_help(format!(
                                "`{name}` has an affine (`linear`) type; an affine value may be used at most once"
                            )),
                        );
                    }
                    // E0429 (multi-shot capture) is handled in Task 3.
                }
            }
            Expr::Call { callee, args } => {
                self.walk_expr(callee);
                for a in args.iter() {
                    self.walk_expr(a);
                }
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            Expr::Unary { expr, .. } => self.walk_expr(expr),
            Expr::If {
                cond,
                then_block,
                else_block,
            } => {
                self.walk_expr(cond);
                self.walk_block(&then_block.node);
                self.walk_block(&else_block.node);
            }
            Expr::Block(b) => self.walk_block(b),
            Expr::Match { scrutinee, arms } => {
                self.walk_expr(scrutinee);
                for arm in arms.iter() {
                    self.walk_expr(&arm.node.body);
                }
            }
            Expr::Handle { body, handler } => {
                self.walk_expr(body);
                for c in &handler.clauses {
                    self.walk_expr(&c.node.body);
                }
                if let Some(r) = &handler.ret {
                    self.walk_expr(&r.body);
                }
            }
            Expr::Lambda { body, .. } => self.walk_block(&body.node),
            Expr::Resume { arg } => self.walk_expr(arg),
            _ => {}
        }
    }
}
