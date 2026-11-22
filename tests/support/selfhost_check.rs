//! The fragments of the checker in Elya's stages (plan
//! docs/superpowers/plans/2026-11-15-elya-checker-in-elya.md, step 4): the
//! smallest stage whose fragment holds a program, read off its syntax.

use elya::ast::{Block, Decl, Expr, Module, Pattern, Stmt, TypeAnn};
use elya::span::Spanned;

/// First-order programs with type errors, for F1's diagnostics: one each,
/// and several in one program (their order is compared too).
pub const FOCUSED_F1: &[&str] = &[
    "pub fn main() -> Int { 1 + True }\n",
    "pub fn main() -> Int { if 1 { 2 } else { 3 } }\n",
    "pub fn main() -> Int { if True { 2 } else { \"x\" } }\n",
    "fn f(x: Int) -> Int { x }\npub fn main() -> Int { f(1, 2) }\n",
    "fn f(x: Int, y: Int) -> Int { x + y }\npub fn main() -> Int { f(1) }\n",
    "fn f(x: Int) -> Bool { x }\npub fn main() -> Int { 0 }\n",
    "pub fn main() -> Int { let x: String = 1\n 2 }\n",
    "pub fn main() -> Int { !3 }\n",
    "pub fn main() -> Float { -.1 }\n",
    "pub fn main() -> Int { 1.5 +. 2 }\n",
    "pub fn main() -> Bool { \"a\" == 1 }\n",
    "pub fn main() -> String { \"a\" <> 1 <> True }\n",
    "pub fn main() -> Int { if 1 < 2.0 { \"a\" } else { 3 + False } }\n",
    "fn g(x) { x + 1 }\nfn h(y) { g(y) && True }\npub fn main() -> Int { h(1) }\n",
    "fn a(n) { b(n) + 1 }\nfn b(n) { if n { a(n) } else { 0 } }\npub fn main() -> Int { a(True) + b(1) }\n",
    "pub fn main() -> Int { let x = 1\n let y = x <> \"s\"\n if y { x } else { 1.0 } }\n",
    // The review's (2026-11-22): the value restriction, the levels a
    // non-value `let` and a binding lower, and the group's member order.
    "fn g(y) { g(y) }\npub fn main() -> Int { let b = g(1)\n let c = b\n if c { 1 } else { c } }\n",
    "fn g(y) { g(y) }\npub fn main() -> Int { let a = { let b = g(1)\n let c = b\n if c { 1 } else { c } }\n a }\n",
    "fn g(y) { g(y) }\nfn f(x) { let a = x == g(1)\n let c = x\n if c { 1 } else { c } }\npub fn main() -> Int { 0 }\n",
    "fn a(n) { let e = 1 + True\n b(n) }\nfn b(n) { let e = if 2 { 1 } else { 2 }\n a(n) }\npub fn main() -> Int { a(1) }\n",
];

/// The checker's imports: the parser in Elya and its lexer.
pub fn reader(name: &str) -> Option<(String, String)> {
    let file = match name {
        "parse" => "selfhost/parse.elya",
        "lex" => "selfhost/lex.elya",
        _ => return None,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("selfhost").is_dir())?
        .to_path_buf();
    let text = std::fs::read_to_string(root.join(file)).ok()?;
    Some((file.to_owned(), text))
}

const BASE: &[&str] = &["Int", "Float", "Bool", "String", "Unit"];

/// The stage a program needs, 1 to 6 (F1 first order ... F6 the grants).
pub fn stage(m: &Module) -> u8 {
    let fns = m
        .decls
        .iter()
        .filter_map(|d| match &d.node {
            Decl::Fn(f) => Some(f.name.clone()),
            _ => None,
        })
        .collect();
    let mut s = Stage(1, fns);
    if !m.imports.is_empty() {
        s.at(6);
    }
    for d in &m.decls {
        match &d.node {
            Decl::Effect(_) => s.at(4),
            Decl::Type(t) => {
                s.at(if t.is_linear { 5 } else { 2 });
                for v in &t.variants {
                    for f in &v.node.fields {
                        s.ann(f);
                    }
                }
            }
            Decl::Fn(f) => {
                if f.effect_row.is_some() {
                    s.at(4);
                }
                for p in &f.params {
                    s.binder(&p.node.name);
                    if p.node.borrow {
                        s.at(5);
                    }
                    if let Some(a) = &p.node.ann {
                        s.ann(a);
                    }
                }
                if let Some(a) = &f.ret_ann {
                    s.ann(a);
                }
                s.block(&f.body.node);
            }
        }
    }
    s.0
}

/// The stage so far, and the top-level functions' names: in F1 a call's
/// callee is one of them, and they are used only as callees.
struct Stage(u8, Vec<String>);

impl Stage {
    fn at(&mut self, n: u8) {
        self.0 = self.0.max(n);
    }

    /// A parameter or `let` named like a top-level function shadows it, so a
    /// call of that name may be a call of a local (the review's D1):
    /// conservatively, not F1.
    fn binder(&mut self, name: &str) {
        if self.1.iter().any(|f| f == name) {
            self.at(2)
        }
    }

    fn ann(&mut self, a: &Spanned<TypeAnn>) {
        let t = &a.node;
        if t.row.is_some() {
            self.at(4);
        }
        if t.name == "Array" || t.name == "Buffer" {
            self.at(5);
        } else if !BASE.contains(&t.name.as_str()) || !t.args.is_empty() {
            // A base type given arguments is an error the checker's
            // annotations report (F3; the review's D2).
            self.at(3);
        }
        for x in &t.args {
            self.ann(x);
        }
    }

    fn block(&mut self, b: &Block) {
        for st in b.stmts.iter() {
            match &st.node {
                Stmt::Let { name, ann, value } => {
                    self.binder(name);
                    if let Some(a) = ann {
                        self.ann(a);
                    }
                    self.expr(value);
                }
                Stmt::Expr(e) => self.expr(e),
            }
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
    }

    fn expr(&mut self, e: &Spanned<Expr>) {
        match &e.node {
            Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Unit => {}
            // A function used as a value is higher order (F2).
            Expr::Var(n) => {
                if self.1.contains(n) {
                    self.at(2)
                }
            }
            Expr::Error => {}
            Expr::Qualified { module, .. } => match module.as_str() {
                "int" | "float" | "string" => self.at(2),
                "array" | "buffer" => self.at(5),
                _ => self.at(4),
            },
            Expr::Call { callee, args } => {
                match &callee.node {
                    // A call of a top-level function is first order; of a
                    // local, a constructor or an operation, it is not.
                    Expr::Var(n) => {
                        if !self.1.contains(n) {
                            self.at(2)
                        }
                    }
                    // A call of a call's result, or of any other
                    // expression, is higher order.
                    _ => {
                        self.at(2);
                        self.expr(callee)
                    }
                }
                args.iter().for_each(|a| self.expr(a));
            }
            Expr::Unary { expr, .. } => self.expr(expr),
            Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
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
            Expr::Handle { .. } | Expr::Resume { .. } => self.at(4),
            Expr::Match { scrutinee, arms } => {
                self.at(2);
                self.expr(scrutinee);
                for a in arms.iter() {
                    self.pattern(&a.node.pat);
                    self.expr(&a.node.body);
                }
            }
            Expr::Lambda { params, body } => {
                self.at(2);
                for p in params {
                    if let Some(a) = &p.node.ann {
                        self.ann(a);
                    }
                }
                self.block(&body.node);
            }
        }
    }

    fn pattern(&mut self, _p: &Spanned<Pattern>) {}
}
