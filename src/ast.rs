//! Abstract syntax tree (Slice 1 subset). Every node is `Spanned`.

use crate::span::Spanned;

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub imports: Vec<Spanned<Import>>,
    pub decls: Vec<Spanned<Decl>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub path: Vec<String>,
    pub alias: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decl {
    Fn(FnDecl),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    pub is_pub: bool,
    pub name: String,
    pub params: Vec<Spanned<Param>>,
    /// Slice 1 retains only effect head names; the type checker arrives in Slice 2.
    pub effect_row: Vec<String>,
    pub body: Spanned<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stmts: Vec<Spanned<Stmt>>,
    pub tail: Option<Box<Spanned<Expr>>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Let { name: String, value: Spanned<Expr> },
    Expr(Spanned<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Unit,
    Var(String),
    Qualified {
        module: String,
        name: String,
    },
    Call {
        callee: Box<Spanned<Expr>>,
        args: Vec<Spanned<Expr>>,
    },
    Unary {
        op: UnOp,
        expr: Box<Spanned<Expr>>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Spanned<Expr>>,
        rhs: Box<Spanned<Expr>>,
    },
    If {
        cond: Box<Spanned<Expr>>,
        then_block: Spanned<Block>,
        else_block: Spanned<Block>,
    },
    Block(Block),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    AddF,
    SubF,
    MulF,
    DivF,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Concat,
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

pub fn pretty(m: &Module) -> String {
    let mut s = String::from("(module");
    for d in &m.decls {
        s.push(' ');
        pretty_decl(&d.node, &mut s);
    }
    s.push(')');
    s
}

pub fn pretty_expr_public(e: &Expr) -> String {
    let mut s = String::new();
    pretty_expr(e, &mut s);
    s
}

fn pretty_decl(d: &Decl, s: &mut String) {
    match d {
        Decl::Fn(f) => {
            s.push_str(&format!("(fn {} (", f.name));
            for (i, p) in f.params.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                s.push_str(&p.node.name);
            }
            s.push_str(") ");
            pretty_block(&f.body.node, s);
            s.push(')');
        }
    }
}

fn pretty_block(b: &Block, s: &mut String) {
    s.push_str("(block");
    for st in &b.stmts {
        s.push(' ');
        match &st.node {
            Stmt::Let { name, value } => {
                s.push_str(&format!("(let {} ", name));
                pretty_expr(&value.node, s);
                s.push(')');
            }
            Stmt::Expr(e) => pretty_expr(&e.node, s),
        }
    }
    if let Some(tail) = &b.tail {
        s.push(' ');
        pretty_expr(&tail.node, s);
    }
    s.push(')');
}

fn pretty_expr(e: &Expr, s: &mut String) {
    match e {
        Expr::Int(n) => s.push_str(&n.to_string()),
        Expr::Float(x) => s.push_str(&x.to_string()),
        Expr::Str(v) => s.push_str(&format!("{v:?}")),
        Expr::Bool(b) => s.push_str(if *b { "True" } else { "False" }),
        Expr::Unit => s.push_str("Unit"),
        Expr::Var(name) => s.push_str(name),
        Expr::Qualified { module, name } => s.push_str(&format!("{module}.{name}")),
        Expr::Call { callee, args } => {
            s.push_str("(call ");
            pretty_expr(&callee.node, s);
            for a in args {
                s.push(' ');
                pretty_expr(&a.node, s);
            }
            s.push(')');
        }
        Expr::Unary { op, expr } => {
            s.push_str(&format!("({} ", unop_sym(*op)));
            pretty_expr(&expr.node, s);
            s.push(')');
        }
        Expr::Binary { op, lhs, rhs } => {
            s.push_str(&format!("({} ", binop_sym(*op)));
            pretty_expr(&lhs.node, s);
            s.push(' ');
            pretty_expr(&rhs.node, s);
            s.push(')');
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            s.push_str("(if ");
            pretty_expr(&cond.node, s);
            s.push(' ');
            pretty_block(&then_block.node, s);
            s.push(' ');
            pretty_block(&else_block.node, s);
            s.push(')');
        }
        Expr::Block(b) => pretty_block(b, s),
    }
}

fn binop_sym(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::AddF => "+.",
        BinOp::SubF => "-.",
        BinOp::MulF => "*.",
        BinOp::DivF => "/.",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Concat => "<>",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}

fn unop_sym(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::{spanned, Span};

    fn sp<T>(node: T) -> Spanned<T> {
        spanned(node, Span::EMPTY)
    }

    #[test]
    fn pretty_prints_binary_expr() {
        let e = Expr::Binary {
            op: BinOp::Add,
            lhs: Box::new(sp(Expr::Int(1))),
            rhs: Box::new(sp(Expr::Int(2))),
        };
        let m = Module {
            imports: vec![],
            decls: vec![sp(Decl::Fn(FnDecl {
                is_pub: false,
                name: "f".into(),
                params: vec![],
                effect_row: vec![],
                body: sp(Block {
                    stmts: vec![],
                    tail: Some(Box::new(sp(e))),
                }),
            }))],
        };
        assert_eq!(pretty(&m), "(module (fn f () (block (+ 1 2))))");
    }
}
