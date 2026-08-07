//! Recursive-descent (declarations) + Pratt (expressions) parser.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::lex::{lex, Token, TokenKind};
use crate::span::{spanned, Span, Spanned};
use crate::Session;
use std::rc::Rc;

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    diags: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Parser<'a> {
        Parser {
            tokens,
            pos: 0,
            diags: Vec::new(),
        }
    }

    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos).map(|t| &t.kind)
    }

    fn peek_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or(Span::EMPTY)
    }

    fn bump(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.peek() == Some(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.diags
            .push(Diagnostic::error("E0100", msg).with_label(span, "here"));
    }

    // ---- Expressions (Pratt) ----

    /// Pratt expression parser. `min_bp` is the minimum binding power.
    fn expr(&mut self, min_bp: u8) -> Option<Spanned<Expr>> {
        let mut lhs = self.atom()?;
        // `cloned()` frees the immutable borrow so the body can mutate `self`.
        while let Some(kind) = self.peek().cloned() {
            // postfix call: `expr(...)`
            if kind == TokenKind::LParen {
                lhs = self.finish_call(lhs)?;
                continue;
            }
            let Some((l_bp, r_bp, op)) = infix_bp(&kind) else {
                break;
            };
            if l_bp < min_bp {
                break;
            }
            self.bump(); // operator
            let rhs = self.expr(r_bp)?;
            let span = lhs.span.merge(rhs.span);
            lhs = spanned(
                Expr::Binary {
                    op,
                    lhs: Rc::new(lhs),
                    rhs: Rc::new(rhs),
                },
                span,
            );
        }
        Some(lhs)
    }

    fn finish_call(&mut self, callee: Spanned<Expr>) -> Option<Spanned<Expr>> {
        let start = callee.span;
        self.bump(); // '('
        let mut args = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                let arg = self.expr(0)?;
                args.push(arg);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        let end = self.peek_span();
        if !self.eat(&TokenKind::RParen) {
            self.error(end, "expected `)` to close call");
            return None;
        }
        Some(spanned(
            Expr::Call {
                callee: Rc::new(callee),
                args: args.into(),
            },
            start.merge(end),
        ))
    }

    fn atom(&mut self) -> Option<Spanned<Expr>> {
        let span = self.peek_span();
        match self.peek()?.clone() {
            TokenKind::KwIf => self.if_expr(),
            TokenKind::LBrace => self.block_expr(),
            TokenKind::Int(n) => {
                self.bump();
                Some(spanned(Expr::Int(n), span))
            }
            TokenKind::Float(x) => {
                self.bump();
                Some(spanned(Expr::Float(x), span))
            }
            TokenKind::Str(s) => {
                self.bump();
                Some(spanned(Expr::Str(s), span))
            }
            TokenKind::True => {
                self.bump();
                Some(spanned(Expr::Bool(true), span))
            }
            TokenKind::False => {
                self.bump();
                Some(spanned(Expr::Bool(false), span))
            }
            TokenKind::Unit => {
                self.bump();
                Some(spanned(Expr::Unit, span))
            }
            TokenKind::Minus => {
                self.bump();
                let e = self.expr(100)?;
                let s = span.merge(e.span);
                Some(spanned(
                    Expr::Unary {
                        op: UnOp::Neg,
                        expr: Rc::new(e),
                    },
                    s,
                ))
            }
            TokenKind::Bang => {
                self.bump();
                let e = self.expr(100)?;
                let s = span.merge(e.span);
                Some(spanned(
                    Expr::Unary {
                        op: UnOp::Not,
                        expr: Rc::new(e),
                    },
                    s,
                ))
            }
            TokenKind::LParen => {
                self.bump();
                let e = self.expr(0)?;
                let end = self.peek_span();
                if !self.eat(&TokenKind::RParen) {
                    self.error(end, "expected `)`");
                    return None;
                }
                Some(e)
            }
            TokenKind::Lower(name) => {
                self.bump();
                if self.eat(&TokenKind::Dot) {
                    let member_span = self.peek_span();
                    match self.peek()?.clone() {
                        TokenKind::Lower(member) => {
                            self.bump();
                            Some(spanned(
                                Expr::Qualified {
                                    module: name,
                                    name: member,
                                },
                                span.merge(member_span),
                            ))
                        }
                        _ => {
                            self.error(member_span, "expected identifier after `.`");
                            None
                        }
                    }
                } else {
                    Some(spanned(Expr::Var(name), span))
                }
            }
            _ => {
                self.error(span, "expected an expression");
                None
            }
        }
    }

    // ---- Declarations ----

    fn module(&mut self) -> Module {
        let mut imports = Vec::new();
        let mut decls = Vec::new();
        while self.peek().is_some() {
            let before = self.pos;
            match self.peek() {
                Some(TokenKind::KwImport) => {
                    if let Some(i) = self.import() {
                        imports.push(i);
                    } else {
                        self.recover_to_decl();
                    }
                }
                Some(TokenKind::KwPub) | Some(TokenKind::KwFn) => {
                    let is_pub = self.eat(&TokenKind::KwPub);
                    if let Some(d) = self.fn_decl(is_pub) {
                        decls.push(d);
                    } else {
                        self.recover_to_decl();
                    }
                }
                Some(TokenKind::KwEffect) => {
                    if let Some(e) = self.effect_decl() {
                        decls.push(e);
                    } else {
                        self.recover_to_decl();
                    }
                }
                _ => {
                    let span = self.peek_span();
                    self.error(span, "expected `import`, `fn`, `pub fn`, or `effect`");
                    self.recover_to_decl();
                }
            }
            if self.pos == before {
                self.bump(); // ensure forward progress
            }
        }
        Module { imports, decls }
    }

    fn effect_decl(&mut self) -> Option<Spanned<Decl>> {
        let start = self.peek_span();
        self.bump(); // effect
        let name = match self.peek()?.clone() {
            TokenKind::Upper(n) => {
                self.bump();
                n
            }
            _ => {
                self.error(self.peek_span(), "expected effect name (uppercase)");
                return None;
            }
        };
        if !self.eat(&TokenKind::LBrace) {
            self.error(self.peek_span(), "expected `{`");
            return None;
        }
        let mut ops = Vec::new();
        while self.peek() == Some(&TokenKind::KwFn) {
            let op = self.op_sig()?;
            ops.push(op);
        }
        let end = self.peek_span();
        self.eat(&TokenKind::RBrace);
        Some(spanned(
            Decl::Effect(EffectDecl { name, ops }),
            start.merge(end),
        ))
    }

    fn op_sig(&mut self) -> Option<Spanned<OpSig>> {
        let start = self.peek_span();
        self.bump(); // fn
        let name = match self.peek()?.clone() {
            TokenKind::Lower(n) => {
                self.bump();
                n
            }
            _ => {
                self.error(self.peek_span(), "expected operation name");
                return None;
            }
        };
        if !self.eat(&TokenKind::LParen) {
            self.error(self.peek_span(), "expected `(`");
            return None;
        }
        let mut params = Vec::new();
        let mut param_tys = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                let pspan = self.peek_span();
                let pname = match self.peek()?.clone() {
                    TokenKind::Lower(n) => {
                        self.bump();
                        n
                    }
                    _ => {
                        self.error(pspan, "expected parameter name");
                        return None;
                    }
                };
                if !self.eat(&TokenKind::Colon) {
                    self.error(self.peek_span(), "operation params need a type");
                    return None;
                }
                let ty = self.type_ann()?;
                params.push(spanned(Param { name: pname }, pspan));
                param_tys.push(ty);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        if !self.eat(&TokenKind::RParen) {
            self.error(self.peek_span(), "expected `)`");
            return None;
        }
        if !self.eat(&TokenKind::Arrow) {
            self.error(self.peek_span(), "operation needs a return type `-> T`");
            return None;
        }
        let ret = self.type_ann()?;
        let end = ret.span;
        Some(spanned(
            OpSig {
                name,
                params,
                param_tys,
                ret,
            },
            start.merge(end),
        ))
    }

    fn type_ann(&mut self) -> Option<Spanned<TypeAnn>> {
        let span = self.peek_span();
        let name = match self.peek()?.clone() {
            TokenKind::Upper(n) => {
                self.bump();
                n
            }
            TokenKind::Unit => {
                self.bump();
                "Unit".to_string()
            }
            TokenKind::Lower(n) => {
                self.bump();
                n // type variable
            }
            _ => {
                self.error(span, "expected a type");
                return None;
            }
        };
        let mut args = Vec::new();
        if self.eat(&TokenKind::LParen) {
            if self.peek() != Some(&TokenKind::RParen) {
                loop {
                    args.push(self.type_ann()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.eat(&TokenKind::RParen);
        }
        Some(spanned(TypeAnn { name, args }, span))
    }

    fn import(&mut self) -> Option<Spanned<Import>> {
        let start = self.peek_span();
        self.bump(); // import
        let mut path = Vec::new();
        loop {
            let seg_span = self.peek_span();
            match self.peek()?.clone() {
                TokenKind::Lower(seg) => {
                    self.bump();
                    path.push(seg);
                }
                _ => {
                    self.error(seg_span, "expected module path segment");
                    return None;
                }
            }
            if !self.eat(&TokenKind::Slash) {
                break;
            }
        }
        let end = self.peek_span();
        Some(spanned(Import { path, alias: None }, start.merge(end)))
    }

    fn fn_decl(&mut self, is_pub: bool) -> Option<Spanned<Decl>> {
        let start = self.peek_span();
        if !self.eat(&TokenKind::KwFn) {
            self.error(start, "expected `fn`");
            return None;
        }
        let name = match self.peek()?.clone() {
            TokenKind::Lower(n) => {
                self.bump();
                n
            }
            _ => {
                self.error(self.peek_span(), "expected function name");
                return None;
            }
        };
        if !self.eat(&TokenKind::LParen) {
            self.error(self.peek_span(), "expected `(`");
            return None;
        }
        let mut params = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                let pspan = self.peek_span();
                match self.peek()?.clone() {
                    TokenKind::Lower(pn) => {
                        self.bump();
                        if self.eat(&TokenKind::Colon) {
                            self.skip_type_annotation();
                        }
                        params.push(spanned(Param { name: pn }, pspan));
                    }
                    _ => {
                        self.error(pspan, "expected parameter name");
                        return None;
                    }
                }
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        if !self.eat(&TokenKind::RParen) {
            self.error(self.peek_span(), "expected `)`");
            return None;
        }
        // optional effect row: `/ { Name, Name }`
        let effect_row = if self.eat(&TokenKind::Slash) {
            self.effect_row()
        } else {
            Vec::new()
        };
        // optional return type: `-> Type`
        if self.eat(&TokenKind::Arrow) {
            self.skip_type_annotation();
        }
        let body = self.block()?;
        let end = body.span;
        Some(spanned(
            Decl::Fn(FnDecl {
                is_pub,
                name,
                params,
                effect_row,
                body: Rc::new(body),
            }),
            start.merge(end),
        ))
    }

    /// Parse `{ Name, Name }` after `/`, keeping only effect head names.
    fn effect_row(&mut self) -> Vec<String> {
        let mut names = Vec::new();
        if !self.eat(&TokenKind::LBrace) {
            self.error(self.peek_span(), "expected `{` for effect row");
            return names;
        }
        if self.peek() != Some(&TokenKind::RBrace) {
            loop {
                match self.peek().cloned() {
                    Some(TokenKind::Upper(n)) => {
                        self.bump();
                        // skip optional effect type args: `(...)`
                        if self.eat(&TokenKind::LParen) {
                            self.skip_balanced_parens();
                        }
                        names.push(n);
                    }
                    _ => {
                        self.error(self.peek_span(), "expected effect name");
                        break;
                    }
                }
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.eat(&TokenKind::RBrace);
        names
    }

    /// Slice 1 has no type checker: consume a type annotation and discard it.
    fn skip_type_annotation(&mut self) {
        match self.peek().cloned() {
            Some(TokenKind::Upper(_)) | Some(TokenKind::Lower(_)) | Some(TokenKind::Unit) => {
                self.bump();
                if self.eat(&TokenKind::LParen) {
                    self.skip_balanced_parens();
                }
            }
            Some(TokenKind::LParen) => {
                self.bump();
                self.skip_balanced_parens();
            }
            _ => { /* nothing to skip */ }
        }
    }

    fn skip_balanced_parens(&mut self) {
        let mut depth = 1;
        while depth > 0 {
            match self.bump().map(|t| t.kind.clone()) {
                Some(TokenKind::LParen) => depth += 1,
                Some(TokenKind::RParen) => depth -= 1,
                None => break,
                _ => {}
            }
        }
    }

    fn block(&mut self) -> Option<Spanned<Block>> {
        let start = self.peek_span();
        if !self.eat(&TokenKind::LBrace) {
            self.error(start, "expected `{`");
            return None;
        }
        let mut stmts = Vec::new();
        let mut tail = None;
        while self.peek().is_some() && self.peek() != Some(&TokenKind::RBrace) {
            if self.peek() == Some(&TokenKind::KwLet) {
                if let Some(s) = self.let_stmt() {
                    stmts.push(s);
                } else {
                    break;
                }
            } else {
                let e = self.expr(0)?;
                if self.peek() == Some(&TokenKind::RBrace) {
                    tail = Some(e);
                } else {
                    let span = e.span;
                    stmts.push(spanned(Stmt::Expr(e), span));
                }
            }
        }
        let end = self.peek_span();
        self.eat(&TokenKind::RBrace);
        Some(spanned(
            Block {
                stmts: stmts.into(),
                tail: tail.map(Rc::new),
            },
            start.merge(end),
        ))
    }

    fn let_stmt(&mut self) -> Option<Spanned<Stmt>> {
        let start = self.peek_span();
        self.bump(); // let
        let name = match self.peek()?.clone() {
            TokenKind::Lower(n) => {
                self.bump();
                n
            }
            _ => {
                self.error(self.peek_span(), "expected binding name");
                return None;
            }
        };
        if self.eat(&TokenKind::Colon) {
            self.skip_type_annotation();
        }
        if !self.eat(&TokenKind::Eq) {
            self.error(self.peek_span(), "expected `=` in let binding");
            return None;
        }
        let value = self.expr(0)?;
        let span = start.merge(value.span);
        Some(spanned(Stmt::Let { name, value }, span))
    }

    fn if_expr(&mut self) -> Option<Spanned<Expr>> {
        let start = self.peek_span();
        self.bump(); // if
        let cond = self.expr(0)?;
        let then_block = self.block()?;
        if !self.eat(&TokenKind::KwElse) {
            self.error(self.peek_span(), "expected `else` (if is an expression)");
            return None;
        }
        let else_block = self.block()?;
        let span = start.merge(else_block.span);
        Some(spanned(
            Expr::If {
                cond: Rc::new(cond),
                then_block: Rc::new(then_block),
                else_block: Rc::new(else_block),
            },
            span,
        ))
    }

    fn block_expr(&mut self) -> Option<Spanned<Expr>> {
        let b = self.block()?;
        Some(spanned(Expr::Block(b.node), b.span))
    }

    fn recover_to_decl(&mut self) {
        // Synchronize: skip tokens until a declaration keyword or EOF.
        while let Some(k) = self.peek() {
            if matches!(
                k,
                TokenKind::KwFn | TokenKind::KwPub | TokenKind::KwImport | TokenKind::KwEffect
            ) {
                return;
            }
            self.bump();
        }
    }
}

/// Binding powers: (left_bp, right_bp, op). Higher binds tighter.
fn infix_bp(kind: &TokenKind) -> Option<(u8, u8, BinOp)> {
    use TokenKind as T;
    let (l, r, op) = match kind {
        T::PipePipe => (10, 11, BinOp::Or),
        T::AmpAmp => (20, 21, BinOp::And),
        T::EqEq => (30, 31, BinOp::Eq),
        T::NotEq => (30, 31, BinOp::Ne),
        T::Lt => (30, 31, BinOp::Lt),
        T::Le => (30, 31, BinOp::Le),
        T::Gt => (30, 31, BinOp::Gt),
        T::Ge => (30, 31, BinOp::Ge),
        T::Concat => (40, 41, BinOp::Concat),
        T::Plus => (50, 51, BinOp::Add),
        T::Minus => (50, 51, BinOp::Sub),
        T::PlusDot => (50, 51, BinOp::AddF),
        T::MinusDot => (50, 51, BinOp::SubF),
        T::Star => (60, 61, BinOp::Mul),
        T::Slash => (60, 61, BinOp::Div),
        T::Percent => (60, 61, BinOp::Rem),
        T::StarDot => (60, 61, BinOp::MulF),
        T::SlashDot => (60, 61, BinOp::DivF),
        _ => return None,
    };
    Some((l, r, op))
}

/// Test/convenience entry: parse a single expression from source text.
pub fn parse_expr_str(session: &Session, text: &str) -> (Option<Spanned<Expr>>, Vec<Diagnostic>) {
    let (tokens, mut diags) = lex(session, text);
    let mut p = Parser::new(&tokens);
    let e = p.expr(0);
    diags.append(&mut p.diags);
    (e, diags)
}

pub fn parse_module(session: &Session, text: &str) -> (Module, Vec<Diagnostic>) {
    let (tokens, mut diags) = lex(session, text);
    let mut p = Parser::new(&tokens);
    let m = p.module();
    diags.append(&mut p.diags);
    (m, diags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::pretty_expr_public;
    use crate::Session;

    fn p(text: &str) -> String {
        let (e, diags) = parse_expr_str(&Session::new(), text);
        assert!(diags.is_empty(), "diags: {diags:?}");
        pretty_expr_public(&e.unwrap().node)
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(p("1 + 2 * 3"), "(+ 1 (* 2 3))");
        assert_eq!(p("1 - 2 - 3"), "(- (- 1 2) 3)");
        assert_eq!(p("(1 + 2) * 3"), "(* (+ 1 2) 3)");
    }

    #[test]
    fn calls_and_qualified() {
        assert_eq!(p(r#"io.println("hi")"#), r#"(call io.println "hi")"#);
        assert_eq!(p("f(1, 2)"), "(call f 1 2)");
    }

    #[test]
    fn parses_hello_world_module() {
        let src = "import elya/io\n\npub fn main() / {IO} {\n  io.println(\"Hello, Elya!\")\n}\n";
        let (m, diags) = parse_module(&Session::new(), src);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(
            crate::ast::pretty(&m),
            r#"(module (fn main () (block (call io.println "Hello, Elya!"))))"#
        );
        let Decl::Fn(f) = &m.decls[0].node else {
            panic!("expected fn")
        };
        assert_eq!(f.effect_row, vec!["IO".to_string()]);
        assert!(f.is_pub);
    }

    #[test]
    fn parses_let_and_if() {
        let src = "fn f() {\n  let x = 1\n  if x { 2 } else { 3 }\n}\n";
        let (m, diags) = parse_module(&Session::new(), src);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(
            crate::ast::pretty(&m),
            "(module (fn f () (block (let x 1) (if x (block 2) (block 3)))))"
        );
    }

    #[test]
    fn parses_effect_declaration() {
        let src = "effect Log {\n  fn log(msg: String) -> Unit\n}\n";
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "diags: {d:?}");
        assert_eq!(crate::ast::pretty(&m), "(module (effect Log (log)))");
        let Decl::Effect(e) = &m.decls[0].node else {
            panic!("expected effect")
        };
        assert_eq!(e.name, "Log");
        assert_eq!(e.ops[0].node.name, "log");
        assert_eq!(e.ops[0].node.param_tys[0].node.name, "String");
        assert_eq!(e.ops[0].node.ret.node.name, "Unit");
    }

    #[test]
    fn recovers_and_reports_multiple_errors() {
        let src = "fn () { 1 }\npub fn ok() { 2 }\n";
        let (m, diags) = parse_module(&Session::new(), src);
        assert!(!diags.is_empty(), "expected at least one diagnostic");
        assert!(diags.iter().all(|d| d.code == "E0100"));
        let names: Vec<_> = m
            .decls
            .iter()
            .filter_map(|d| match &d.node {
                Decl::Fn(f) => Some(f.name.clone()),
                Decl::Effect(_) => None,
            })
            .collect();
        assert!(names.contains(&"ok".to_string()), "names: {names:?}");
    }
}
