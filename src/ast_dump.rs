//! The parser's output as text, one node a line (self-hosting part 4, spec
//! `docs/superpowers/specs/2026-11-15-elya-parser-in-elya-design.md` D2): the
//! form `selfhost/parse.elya` prints and `tests/selfhost_parse.rs` compares
//! byte for byte. Complete: every field the parser sets, every span.
//!
//! A line is `<depth> <Kind> <start> <end>` and then the node's payload,
//! fields separated by one space; children follow, one deeper, in source
//! order. A node with no span of its own prints the span of the syntax it
//! stands for (a row: from its first label to its tail) or `- -`. Text is
//! quoted and escaped as the lexer's dump escapes it; a float literal prints
//! its source text (Elya cannot yet read a float's value: part 3 left E0002
//! out for the same reason).

use crate::ast::*;
use crate::span::{Span, Spanned};
use std::fmt::Write;

/// The dump of a module parsed from `src`.
pub fn dump(m: &Module, src: &str) -> String {
    let mut d = Dump {
        out: String::new(),
        src,
    };
    d.line(0, "Module", None, "");
    for i in &m.imports {
        d.import(1, i);
    }
    for decl in &m.decls {
        d.decl(1, decl);
    }
    d.out
}

/// The parser's diagnostics, one a line after the tree: `error <code> <start>
/// <end> <message>`, the span its first label's (the lexer's dump has no
/// message; a parser's errors are told apart by theirs). A warning's line
/// begins `warning` (the checker's E0426, exhaustiveness's E0431; spec
/// 2026-11-15-elya-checker-in-elya D2).
pub fn dump_diagnostics(diags: &[crate::diag::Diagnostic]) -> String {
    let mut out = String::new();
    for d in diags {
        let (start, end) = d
            .labels
            .first()
            .map_or(("-".to_string(), "-".to_string()), |l| {
                (l.span.start.to_string(), l.span.end.to_string())
            });
        let kind = match d.severity {
            crate::diag::Severity::Error => "error",
            crate::diag::Severity::Warning => "warning",
        };
        writeln!(
            out,
            "{kind} {} {start} {end} {}",
            d.code,
            quoted(&d.message)
        )
        .expect("a String takes writes");
    }
    out
}

/// `text` quoted, `\n`, `\t`, `\r`, `\\` and `"` escaped (the lexer's dump).
pub fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn flag(on: bool, word: &str) -> &str {
    if on {
        word
    } else {
        "-"
    }
}

struct Dump<'a> {
    out: String,
    src: &'a str,
}

impl Dump<'_> {
    fn line(&mut self, depth: usize, kind: &str, span: Option<Span>, payload: &str) {
        match span {
            Some(s) => write!(self.out, "{depth} {kind} {} {}", s.start, s.end),
            None => write!(self.out, "{depth} {kind} - -"),
        }
        .expect("a String takes writes");
        if !payload.is_empty() {
            self.out.push(' ');
            self.out.push_str(payload);
        }
        self.out.push('\n');
    }

    fn import(&mut self, depth: usize, i: &Spanned<Import>) {
        let alias = i.node.alias.clone().unwrap_or_else(|| "-".to_string());
        let n = i.node.effects.as_ref().map_or(-1, |e| e.len() as i64);
        self.line(
            depth,
            "Import",
            Some(i.span),
            &format!("{} {alias} {n}", i.node.path.join("/")),
        );
        for e in i.node.effects.iter().flatten() {
            self.line(depth + 1, "Allow", Some(e.span), &e.node);
        }
    }

    fn decl(&mut self, depth: usize, d: &Spanned<Decl>) {
        match &d.node {
            Decl::Fn(f) => {
                self.line(
                    depth,
                    "Fn",
                    Some(d.span),
                    &format!("{} {}", flag(f.is_pub, "pub"), f.name),
                );
                for p in &f.params {
                    self.param(depth + 1, p);
                }
                match &f.effect_row {
                    Some(r) => self.row(depth + 1, r),
                    None => self.line(depth + 1, "NoRow", None, ""),
                }
                match &f.ret_ann {
                    Some(t) => self.ty(depth + 1, t),
                    None => self.line(depth + 1, "NoType", None, ""),
                }
                self.block(depth + 1, &f.body);
            }
            Decl::Effect(e) => {
                let params = if e.params.is_empty() {
                    "-".to_string()
                } else {
                    e.params.join(",")
                };
                self.line(
                    depth,
                    "Effect",
                    Some(d.span),
                    &format!("{} {} {params}", flag(e.is_multi, "multi"), e.name),
                );
                for op in &e.ops {
                    self.line(
                        depth + 1,
                        "Op",
                        Some(op.span),
                        &format!("{} {}", op.node.name, op.node.param_tys.len()),
                    );
                    for p in &op.node.params {
                        self.param(depth + 2, p);
                    }
                    for t in &op.node.param_tys {
                        self.ty(depth + 2, t);
                    }
                    self.ty(depth + 2, &op.node.ret);
                }
            }
            Decl::Type(t) => {
                let params = if t.params.is_empty() {
                    "-".to_string()
                } else {
                    t.params.join(",")
                };
                self.line(
                    depth,
                    "Type",
                    Some(d.span),
                    &format!("{} {} {params}", flag(t.is_linear, "linear"), t.name),
                );
                for v in &t.variants {
                    self.line(depth + 1, "Variant", Some(v.span), &v.node.name);
                    for f in &v.node.fields {
                        self.ty(depth + 2, f);
                    }
                }
            }
        }
    }

    fn param(&mut self, depth: usize, p: &Spanned<Param>) {
        self.line(
            depth,
            "Param",
            Some(p.span),
            &format!("{} {}", flag(p.node.borrow, "borrow"), p.node.name),
        );
        if let Some(t) = &p.node.ann {
            self.ty(depth + 1, t);
        }
    }

    fn ty(&mut self, depth: usize, t: &Spanned<TypeAnn>) {
        self.line(depth, "Ty", Some(t.span), &t.node.name);
        for a in &t.node.args {
            self.ty(depth + 1, a);
        }
        if let Some(r) = &t.node.row {
            self.row(depth + 1, r);
        }
    }

    fn row(&mut self, depth: usize, r: &RowAnn) {
        let tail = r.tail.as_ref().map_or("-".to_string(), |t| t.node.clone());
        self.line(depth, "Row", None, &tail);
        for l in &r.labels {
            let n = l.args.as_ref().map_or(-1, |a| a.len() as i64);
            self.line(
                depth + 1,
                "Label",
                Some(l.name.span),
                &format!("{} {n}", l.name.node),
            );
            for a in l.args.iter().flatten() {
                self.ty(depth + 2, a);
            }
        }
    }

    fn block(&mut self, depth: usize, b: &Spanned<Block>) {
        self.line(depth, "Block", Some(b.span), "");
        self.block_items(depth, &b.node);
    }

    fn block_items(&mut self, depth: usize, b: &Block) {
        for st in b.stmts.iter() {
            match &st.node {
                Stmt::Let { name, ann, value } => {
                    self.line(depth + 1, "Let", Some(st.span), name);
                    match ann {
                        Some(t) => self.ty(depth + 2, t),
                        None => self.line(depth + 2, "NoType", None, ""),
                    }
                    self.expr(depth + 2, value);
                }
                Stmt::Expr(e) => {
                    self.line(depth + 1, "Do", Some(st.span), "");
                    self.expr(depth + 2, e);
                }
            }
        }
        if let Some(t) = &b.tail {
            self.line(depth + 1, "Tail", None, "");
            self.expr(depth + 2, t);
        }
    }

    fn pattern(&mut self, depth: usize, p: &Spanned<Pattern>) {
        let (kind, payload) = match &p.node {
            Pattern::Wild => ("PWild", String::new()),
            Pattern::Var(x) => ("PVar", x.clone()),
            Pattern::Ctor { name, args } => ("PCtor", format!("{name} {}", args.len())),
            Pattern::Lit(PatLit::Int(n)) => ("PInt", n.to_string()),
            Pattern::Lit(PatLit::Bool(b)) => ("PBool", if *b { "True" } else { "False" }.into()),
            Pattern::Lit(PatLit::Str(s)) => ("PStr", quoted(s)),
            Pattern::Lit(PatLit::Unit) => ("PUnit", String::new()),
        };
        self.line(depth, kind, Some(p.span), &payload);
        if let Pattern::Ctor { args, .. } = &p.node {
            for a in args {
                self.pattern(depth + 1, a);
            }
        }
    }

    fn expr(&mut self, depth: usize, e: &Spanned<Expr>) {
        let s = Some(e.span);
        match &e.node {
            Expr::Int(n) => self.line(depth, "Int", s, &n.to_string()),
            Expr::Float(_) => {
                let text = self
                    .src
                    .get(e.span.start as usize..e.span.end as usize)
                    .unwrap_or("?")
                    .replace('_', "");
                self.line(depth, "Float", s, &text)
            }
            Expr::Str(v) => self.line(depth, "Str", s, &quoted(v)),
            Expr::Bool(b) => self.line(depth, "Bool", s, if *b { "True" } else { "False" }),
            Expr::Unit => self.line(depth, "Unit", s, ""),
            Expr::Error => self.line(depth, "Error", s, ""),
            Expr::Var(x) => self.line(depth, "Var", s, x),
            Expr::Qualified { module, name } => {
                self.line(depth, "Qualified", s, &format!("{module} {name}"))
            }
            Expr::Call { callee, args } => {
                self.line(depth, "Call", s, &args.len().to_string());
                self.expr(depth + 1, callee);
                for a in args.iter() {
                    self.expr(depth + 1, a);
                }
            }
            Expr::Unary { op, expr } => {
                let sym = match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "!",
                    UnOp::NegF => "-.",
                };
                self.line(depth, "Unary", s, sym);
                self.expr(depth + 1, expr);
            }
            Expr::Binary { op, lhs, rhs } => {
                self.line(depth, "Binary", s, binop_sym(*op));
                self.expr(depth + 1, lhs);
                self.expr(depth + 1, rhs);
            }
            Expr::If {
                cond,
                then_block,
                else_block,
            } => {
                self.line(depth, "If", s, "");
                self.expr(depth + 1, cond);
                self.block(depth + 1, then_block);
                self.block(depth + 1, else_block);
            }
            Expr::Block(b) => {
                self.line(depth, "Block", s, "");
                self.block_items(depth, b);
            }
            Expr::Lambda { params, body } => {
                self.line(depth, "Lambda", s, &params.len().to_string());
                for p in params {
                    self.param(depth + 1, p);
                }
                self.block(depth + 1, body);
            }
            Expr::Handle { body, handler } => {
                self.line(
                    depth,
                    "Handle",
                    s,
                    &format!(
                        "{} {}",
                        flag(handler.multi, "multi"),
                        flag(handler.any, "any")
                    ),
                );
                self.expr(depth + 1, body);
                for c in &handler.clauses {
                    let eff = c.node.effect.clone().unwrap_or_else(|| "-".to_string());
                    self.line(
                        depth + 1,
                        "Clause",
                        Some(c.span),
                        &format!("{eff} {}", c.node.op),
                    );
                    for p in &c.node.params {
                        self.param(depth + 2, p);
                    }
                    self.expr(depth + 2, &c.node.body);
                }
                if let Some(r) = &handler.ret {
                    self.line(depth + 1, "Return", None, &r.binder);
                    self.expr(depth + 2, &r.body);
                }
            }
            Expr::Resume { arg } => {
                self.line(depth, "Resume", s, "");
                self.expr(depth + 1, arg);
            }
            Expr::Match { scrutinee, arms } => {
                self.line(depth, "Match", s, &arms.len().to_string());
                self.expr(depth + 1, scrutinee);
                for arm in arms.iter() {
                    self.line(depth + 1, "Arm", Some(arm.span), "");
                    self.pattern(depth + 2, &arm.node.pat);
                    self.expr(depth + 2, &arm.node.body);
                }
            }
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
        BinOp::LtF => "<.",
        BinOp::LeF => "<=.",
        BinOp::GtF => ">.",
        BinOp::GeF => ">=.",
        BinOp::Concat => "<>",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}
