//! Abstract syntax tree (Slice 1 subset). Every node is `Spanned`.

use crate::span::Spanned;
use std::rc::Rc;

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
    Effect(EffectDecl),
    Type(TypeDecl),
}

/// A parametric algebraic data type: `type List(a) { Nil, Cons(a, List(a)) }`.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeDecl {
    pub name: String,
    pub params: Vec<String>,
    pub variants: Vec<Spanned<VariantDecl>>,
}

/// One value constructor of an ADT, with positional, typed payload fields.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantDecl {
    pub name: String,
    pub fields: Vec<Spanned<TypeAnn>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    pub is_pub: bool,
    pub name: String,
    pub params: Vec<Spanned<Param>>,
    /// Effect-row annotation: `None` = unannotated (row inferred, Slice 3b),
    /// `Some(vec![])` = explicit pure `/ {}`, `Some([Log@span, …])` = declared
    /// exactly those effects. Spans point diagnostics at the declared effect.
    pub effect_row: Option<Vec<Spanned<String>>>,
    pub body: Rc<Spanned<Block>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
}

/// A surface type annotation. Slice 3a: base names (`Int`, `String`, `Unit`, …)
/// with optional args (for future generic types). Retained for effect operation
/// signatures so Slice 3b's type checker has them.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeAnn {
    pub name: String,
    pub args: Vec<Spanned<TypeAnn>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpSig {
    pub name: String,
    pub params: Vec<Spanned<Param>>,
    pub param_tys: Vec<Spanned<TypeAnn>>,
    pub ret: Spanned<TypeAnn>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectDecl {
    pub name: String,
    pub ops: Vec<Spanned<OpSig>>,
}

/// One operation clause in a handler: `Effect.op(params) -> body`.
#[derive(Clone, Debug, PartialEq)]
pub struct OpClause {
    pub effect: Option<String>,
    pub op: String,
    pub params: Vec<Spanned<Param>>,
    pub body: Rc<Spanned<Expr>>,
}

/// The optional `return(x) -> body` clause of a handler.
#[derive(Clone, Debug, PartialEq)]
pub struct ReturnClause {
    pub binder: String,
    pub body: Rc<Spanned<Expr>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Handler {
    pub multi: bool,
    pub clauses: Vec<Spanned<OpClause>>,
    pub ret: Option<ReturnClause>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stmts: Rc<[Spanned<Stmt>]>,
    pub tail: Option<Rc<Spanned<Expr>>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Let { name: String, value: Spanned<Expr> },
    Expr(Spanned<Expr>),
}

/// A `match` pattern. Slice 4a: constructor, variable, and wildcard (literal
/// patterns arrive in a later increment). Nested arbitrarily via `Ctor.args`.
#[derive(Clone, Debug, PartialEq)]
pub enum Pattern {
    Wild,
    Var(String),
    Ctor {
        name: String,
        args: Vec<Spanned<Pattern>>,
    },
}

/// One arm of a `match`: `pat -> body`.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchArm {
    pub pat: Spanned<Pattern>,
    pub body: Rc<Spanned<Expr>>,
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
        callee: Rc<Spanned<Expr>>,
        args: Rc<[Spanned<Expr>]>,
    },
    Unary {
        op: UnOp,
        expr: Rc<Spanned<Expr>>,
    },
    Binary {
        op: BinOp,
        lhs: Rc<Spanned<Expr>>,
        rhs: Rc<Spanned<Expr>>,
    },
    If {
        cond: Rc<Spanned<Expr>>,
        then_block: Rc<Spanned<Block>>,
        else_block: Rc<Spanned<Block>>,
    },
    Block(Block),
    Handle {
        body: Rc<Spanned<Expr>>,
        handler: Rc<Handler>,
    },
    Resume {
        arg: Rc<Spanned<Expr>>,
    },
    Match {
        scrutinee: Rc<Spanned<Expr>>,
        arms: Rc<[Spanned<MatchArm>]>,
    },
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
        Decl::Effect(e) => {
            s.push_str(&format!("(effect {}", e.name));
            for op in &e.ops {
                s.push_str(&format!(" ({})", op.node.name));
            }
            s.push(')');
        }
        Decl::Type(t) => {
            s.push_str(&format!("(type {}", t.name));
            for v in &t.variants {
                s.push_str(&format!(" ({}", v.node.name));
                for f in &v.node.fields {
                    s.push(' ');
                    pretty_type_ann(&f.node, s);
                }
                s.push(')');
            }
            s.push(')');
        }
    }
}

fn pretty_type_ann(t: &TypeAnn, s: &mut String) {
    if t.args.is_empty() {
        s.push_str(&t.name);
    } else {
        s.push('(');
        s.push_str(&t.name);
        for a in &t.args {
            s.push(' ');
            pretty_type_ann(&a.node, s);
        }
        s.push(')');
    }
}

fn pretty_pattern(p: &Pattern, s: &mut String) {
    match p {
        Pattern::Wild => s.push('_'),
        Pattern::Var(x) => s.push_str(x),
        Pattern::Ctor { name, args } => {
            s.push_str(name);
            if !args.is_empty() {
                s.push_str(" (");
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        s.push(' ');
                    }
                    pretty_pattern(&a.node, s);
                }
                s.push(')');
            }
        }
    }
}

fn pretty_block(b: &Block, s: &mut String) {
    s.push_str("(block");
    for st in b.stmts.iter() {
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
            for a in args.iter() {
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
        Expr::Handle { body, handler } => {
            s.push_str("(handle ");
            pretty_expr(&body.node, s);
            for c in &handler.clauses {
                let c = &c.node;
                let eff = c.effect.clone().unwrap_or_default();
                s.push_str(&format!(" ({}.{} (", eff, c.op));
                for (i, p) in c.params.iter().enumerate() {
                    if i > 0 {
                        s.push(' ');
                    }
                    s.push_str(&p.node.name);
                }
                s.push_str(") ");
                pretty_expr(&c.body.node, s);
                s.push(')');
            }
            if let Some(r) = &handler.ret {
                s.push_str(&format!(" (return {} ", r.binder));
                pretty_expr(&r.body.node, s);
                s.push(')');
            }
            s.push(')');
        }
        Expr::Resume { arg } => {
            s.push_str("(resume ");
            pretty_expr(&arg.node, s);
            s.push(')');
        }
        Expr::Match { scrutinee, arms } => {
            s.push_str("(match ");
            pretty_expr(&scrutinee.node, s);
            for arm in arms.iter() {
                s.push_str(" (");
                pretty_pattern(&arm.node.pat.node, s);
                s.push(' ');
                pretty_expr(&arm.node.body.node, s);
                s.push(')');
            }
            s.push(')');
        }
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
    fn pretty_prints_type_decl_and_match() {
        // type Opt(a) { None, Some(a) }
        let ty = Decl::Type(TypeDecl {
            name: "Opt".into(),
            params: vec!["a".into()],
            variants: vec![
                sp(VariantDecl {
                    name: "None".into(),
                    fields: vec![],
                }),
                sp(VariantDecl {
                    name: "Some".into(),
                    fields: vec![sp(TypeAnn {
                        name: "a".into(),
                        args: vec![],
                    })],
                }),
            ],
        });
        // fn f(o) { match o { None -> 0  Some(x) -> x } }
        let m = Expr::Match {
            scrutinee: Rc::new(sp(Expr::Var("o".into()))),
            arms: vec![
                sp(MatchArm {
                    pat: sp(Pattern::Ctor {
                        name: "None".into(),
                        args: vec![],
                    }),
                    body: Rc::new(sp(Expr::Int(0))),
                }),
                sp(MatchArm {
                    pat: sp(Pattern::Ctor {
                        name: "Some".into(),
                        args: vec![sp(Pattern::Var("x".into()))],
                    }),
                    body: Rc::new(sp(Expr::Var("x".into()))),
                }),
            ]
            .into(),
        };
        let module = Module {
            imports: vec![],
            decls: vec![
                sp(ty),
                sp(Decl::Fn(FnDecl {
                    is_pub: false,
                    name: "f".into(),
                    params: vec![sp(Param { name: "o".into() })],
                    effect_row: None,
                    body: Rc::new(sp(Block {
                        stmts: vec![].into(),
                        tail: Some(Rc::new(sp(m))),
                    })),
                })),
            ],
        };
        assert_eq!(
            pretty(&module),
            "(module (type Opt (None) (Some a)) (fn f (o) (block (match o (None 0) (Some (x) x)))))"
        );
    }

    #[test]
    fn pretty_prints_binary_expr() {
        let e = Expr::Binary {
            op: BinOp::Add,
            lhs: Rc::new(sp(Expr::Int(1))),
            rhs: Rc::new(sp(Expr::Int(2))),
        };
        let m = Module {
            imports: vec![],
            decls: vec![sp(Decl::Fn(FnDecl {
                is_pub: false,
                name: "f".into(),
                params: vec![],
                effect_row: None,
                body: Rc::new(sp(Block {
                    stmts: vec![].into(),
                    tail: Some(Rc::new(sp(e))),
                })),
            }))],
        };
        assert_eq!(pretty(&m), "(module (fn f () (block (+ 1 2))))");
    }

    #[test]
    fn pretty_prints_effect_decl_and_handle() {
        // effect Log { fn log(msg: String) -> Unit }
        let eff = Decl::Effect(EffectDecl {
            name: "Log".into(),
            ops: vec![sp(OpSig {
                name: "log".into(),
                params: vec![sp(Param { name: "msg".into() })],
                param_tys: vec![sp(TypeAnn {
                    name: "String".into(),
                    args: vec![],
                })],
                ret: sp(TypeAnn {
                    name: "Unit".into(),
                    args: vec![],
                }),
            })],
        });
        // handle x with { Log.log(m) -> resume(m) return(r) -> r }
        let handler = Handler {
            multi: false,
            clauses: vec![sp(OpClause {
                effect: Some("Log".into()),
                op: "log".into(),
                params: vec![sp(Param { name: "m".into() })],
                body: Rc::new(sp(Expr::Resume {
                    arg: Rc::new(sp(Expr::Var("m".into()))),
                })),
            })],
            ret: Some(ReturnClause {
                binder: "r".into(),
                body: Rc::new(sp(Expr::Var("r".into()))),
            }),
        };
        let handle = Expr::Handle {
            body: Rc::new(sp(Expr::Var("x".into()))),
            handler: Rc::new(handler),
        };
        let m = Module {
            imports: vec![],
            decls: vec![
                sp(eff),
                sp(Decl::Fn(FnDecl {
                    is_pub: false,
                    name: "f".into(),
                    params: vec![],
                    effect_row: None,
                    body: Rc::new(sp(Block {
                        stmts: vec![].into(),
                        tail: Some(Rc::new(sp(handle))),
                    })),
                })),
            ],
        };
        assert_eq!(
            pretty(&m),
            "(module (effect Log (log)) (fn f () (block (handle x (Log.log (m) (resume m)) (return r r)))))"
        );
    }
}
