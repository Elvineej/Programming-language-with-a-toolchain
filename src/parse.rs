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
            TokenKind::KwHandle => self.handle_expr(),
            TokenKind::KwFn => self.lambda_expr(),
            TokenKind::KwMatch => self.match_expr(),
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
            // `resume` is contextually reserved: `resume(EXPR)` is a dedicated node.
            TokenKind::Lower(ref n) if n == "resume" => {
                self.bump(); // resume
                if !self.eat(&TokenKind::LParen) {
                    self.error(self.peek_span(), "expected `(` after `resume`");
                    return None;
                }
                let arg = self.expr(0)?;
                let end = self.peek_span();
                if !self.eat(&TokenKind::RParen) {
                    self.error(end, "expected `)`");
                    return None;
                }
                Some(spanned(Expr::Resume { arg: Rc::new(arg) }, span.merge(end)))
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
            // A constructor reference (`Nil`, `Cons`, `Some`) is a `Var`; a call
            // `Cons(h, t)` is handled by the postfix-call rule in `expr`.
            TokenKind::Upper(name) => {
                self.bump();
                Some(spanned(Expr::Var(name), span))
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
                Some(TokenKind::KwType) | Some(TokenKind::KwLinear) => {
                    if let Some(t) = self.type_decl() {
                        decls.push(t);
                    } else {
                        self.recover_to_decl();
                    }
                }
                _ => {
                    let span = self.peek_span();
                    self.error(
                        span,
                        "expected `import`, `fn`, `pub fn`, `effect`, or `type`",
                    );
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
                     // Optional resumption modifier: `effect multi Name` (reuses `KwMulti`).
        let is_multi = self.eat(&TokenKind::KwMulti);
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
        // Optional type parameters: `effect State(s)` (mirrors `type_decl`).
        let mut params = Vec::new();
        if self.eat(&TokenKind::LParen) {
            if self.peek() != Some(&TokenKind::RParen) {
                loop {
                    match self.peek()?.clone() {
                        TokenKind::Lower(p) => {
                            self.bump();
                            params.push(p);
                        }
                        _ => {
                            self.error(self.peek_span(), "expected type parameter (lowercase)");
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
        }
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
            Decl::Effect(EffectDecl {
                name,
                params,
                is_multi,
                ops,
            }),
            start.merge(end),
        ))
    }

    // `[linear] type NAME(p, …) { Variant, Variant(T, …), … }`
    fn type_decl(&mut self) -> Option<Spanned<Decl>> {
        let start = self.peek_span();
        // Optional `linear` modifier (Slice 4d-2). Reached via either the `KwType`
        // or the `KwLinear` dispatch arm in `module()`.
        let is_linear = self.eat(&TokenKind::KwLinear);
        if is_linear {
            if !self.eat(&TokenKind::KwType) {
                self.error(self.peek_span(), "expected `type` after `linear`");
                return None;
            }
        } else {
            self.bump(); // type
        }
        let name = match self.peek()?.clone() {
            TokenKind::Upper(n) => {
                self.bump();
                n
            }
            _ => {
                self.error(self.peek_span(), "expected type name (uppercase)");
                return None;
            }
        };
        let mut params = Vec::new();
        if self.eat(&TokenKind::LParen) {
            if self.peek() != Some(&TokenKind::RParen) {
                loop {
                    match self.peek()?.clone() {
                        TokenKind::Lower(p) => {
                            self.bump();
                            params.push(p);
                        }
                        _ => {
                            self.error(self.peek_span(), "expected type parameter (lowercase)");
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
        }
        if !self.eat(&TokenKind::LBrace) {
            self.error(self.peek_span(), "expected `{`");
            return None;
        }
        let mut variants = Vec::new();
        if self.peek() != Some(&TokenKind::RBrace) {
            loop {
                variants.push(self.variant_decl()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        let end = self.peek_span();
        if !self.eat(&TokenKind::RBrace) {
            self.error(self.peek_span(), "expected `}`");
            return None;
        }
        Some(spanned(
            Decl::Type(TypeDecl {
                name,
                params,
                is_linear,
                variants,
            }),
            start.merge(end),
        ))
    }

    fn variant_decl(&mut self) -> Option<Spanned<VariantDecl>> {
        let start = self.peek_span();
        let name = match self.peek()?.clone() {
            TokenKind::Upper(n) => {
                self.bump();
                n
            }
            _ => {
                self.error(self.peek_span(), "expected constructor name (uppercase)");
                return None;
            }
        };
        let mut fields = Vec::new();
        let mut end = start;
        if self.eat(&TokenKind::LParen) {
            if self.peek() != Some(&TokenKind::RParen) {
                loop {
                    fields.push(self.type_ann()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            end = self.peek_span();
            if !self.eat(&TokenKind::RParen) {
                self.error(self.peek_span(), "expected `)`");
                return None;
            }
        }
        Some(spanned(VariantDecl { name, fields }, start.merge(end)))
    }

    // `match EXPR { PAT -> EXPR … }` — arms are whitespace-separated (no comma).
    /// Parse an anonymous function `fn ( params ) { block }` in expression position.
    fn lambda_expr(&mut self) -> Option<Spanned<Expr>> {
        let start = self.peek_span();
        self.bump(); // `fn`
        if !self.eat(&TokenKind::LParen) {
            self.error(self.peek_span(), "expected `(` after `fn` in a lambda");
            return None;
        }
        let mut params = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                let pspan = self.peek_span();
                match self.peek()?.clone() {
                    TokenKind::Lower(pn) => {
                        self.bump();
                        let ann = if self.eat(&TokenKind::Colon) {
                            Some(self.type_ann()?)
                        } else {
                            None
                        };
                        params.push(spanned(Param { name: pn, ann }, pspan));
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
        let body = self.block()?;
        let end = body.span;
        Some(spanned(
            Expr::Lambda {
                params,
                body: Rc::new(body),
            },
            start.merge(end),
        ))
    }

    fn match_expr(&mut self) -> Option<Spanned<Expr>> {
        let start = self.peek_span();
        self.bump(); // match
        let scrutinee = self.expr(0)?;
        if !self.eat(&TokenKind::LBrace) {
            self.error(self.peek_span(), "expected `{` after match scrutinee");
            return None;
        }
        let mut arms = Vec::new();
        while self.peek().is_some() && self.peek() != Some(&TokenKind::RBrace) {
            arms.push(self.match_arm()?);
        }
        let end = self.peek_span();
        if !self.eat(&TokenKind::RBrace) {
            self.error(self.peek_span(), "expected `}`");
            return None;
        }
        Some(spanned(
            Expr::Match {
                scrutinee: Rc::new(scrutinee),
                arms: arms.into(),
            },
            start.merge(end),
        ))
    }

    fn match_arm(&mut self) -> Option<Spanned<MatchArm>> {
        let start = self.peek_span();
        let pat = self.pattern()?;
        if !self.eat(&TokenKind::Arrow) {
            self.error(self.peek_span(), "expected `->` in match arm");
            return None;
        }
        let body = self.expr(0)?;
        let end = body.span;
        Some(spanned(
            MatchArm {
                pat,
                body: Rc::new(body),
            },
            start.merge(end),
        ))
    }

    fn pattern(&mut self) -> Option<Spanned<Pattern>> {
        let span = self.peek_span();
        match self.peek()?.clone() {
            // Constructor pattern: `Nil`, `Some(p)`, `Cons(h, t)`.
            TokenKind::Upper(name) => {
                self.bump();
                let mut args = Vec::new();
                let mut end = span;
                if self.eat(&TokenKind::LParen) {
                    if self.peek() != Some(&TokenKind::RParen) {
                        loop {
                            args.push(self.pattern()?);
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    end = self.peek_span();
                    if !self.eat(&TokenKind::RParen) {
                        self.error(self.peek_span(), "expected `)`");
                        return None;
                    }
                }
                Some(spanned(Pattern::Ctor { name, args }, span.merge(end)))
            }
            // `_` is the wildcard; any other lowercase name binds a variable.
            TokenKind::Lower(name) => {
                self.bump();
                let p = if name == "_" {
                    Pattern::Wild
                } else {
                    Pattern::Var(name)
                };
                Some(spanned(p, span))
            }
            // Literal patterns.
            TokenKind::Int(n) => {
                self.bump();
                Some(spanned(Pattern::Lit(PatLit::Int(n)), span))
            }
            TokenKind::Str(s) => {
                self.bump();
                Some(spanned(Pattern::Lit(PatLit::Str(s)), span))
            }
            TokenKind::True => {
                self.bump();
                Some(spanned(Pattern::Lit(PatLit::Bool(true)), span))
            }
            TokenKind::False => {
                self.bump();
                Some(spanned(Pattern::Lit(PatLit::Bool(false)), span))
            }
            TokenKind::Unit => {
                self.bump();
                Some(spanned(Pattern::Lit(PatLit::Unit), span))
            }
            _ => {
                self.error(span, "expected a pattern");
                None
            }
        }
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
                params.push(spanned(
                    Param {
                        name: pname,
                        ann: None,
                    },
                    pspan,
                ));
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
        // A function type (2026-10-09): `fn(A, B) / {E} -> R`; the row is
        // optional (none written: any effects).
        if self.eat(&TokenKind::KwFn) {
            if !self.eat(&TokenKind::LParen) {
                self.error(self.peek_span(), "expected `(` in a function type");
                return None;
            }
            let mut args = Vec::new();
            if self.peek() != Some(&TokenKind::RParen) {
                loop {
                    args.push(self.type_ann()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            if !self.eat(&TokenKind::RParen) {
                self.error(self.peek_span(), "expected `)` in a function type");
                return None;
            }
            let row = if self.eat(&TokenKind::Slash) {
                Some(self.effect_row())
            } else {
                None
            };
            if !self.eat(&TokenKind::Arrow) {
                self.error(self.peek_span(), "expected `->` in a function type");
                return None;
            }
            let ret = self.type_ann()?;
            let end = ret.span;
            args.push(ret);
            return Some(spanned(
                TypeAnn {
                    name: "fn".to_string(),
                    args,
                    row,
                },
                span.merge(end),
            ));
        }
        // `(T)` is T (the old annotation skipper accepted it; 2026-10-09).
        if self.eat(&TokenKind::LParen) {
            if self.eat(&TokenKind::RParen) {
                return Some(spanned(
                    TypeAnn {
                        name: "Unit".to_string(),
                        args: Vec::new(),
                        row: None,
                    },
                    span,
                ));
            }
            let inner = self.type_ann()?;
            if !self.eat(&TokenKind::RParen) {
                self.error(
                    self.peek_span(),
                    "expected `)` -- tuple types are not supported in annotations",
                );
                return None;
            }
            return Some(inner);
        }
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
        Some(spanned(
            TypeAnn {
                name,
                args,
                row: None,
            },
            span,
        ))
    }

    fn handle_expr(&mut self) -> Option<Spanned<Expr>> {
        let start = self.peek_span();
        self.bump(); // handle
        let body = self.expr(0)?;
        if !self.eat(&TokenKind::KwWith) {
            self.error(self.peek_span(), "expected `with`");
            return None;
        }
        let multi = self.eat(&TokenKind::KwMulti);
        if !self.eat(&TokenKind::LBrace) {
            self.error(self.peek_span(), "expected `{`");
            return None;
        }
        let mut clauses = Vec::new();
        let mut ret = None;
        while self.peek().is_some() && self.peek() != Some(&TokenKind::RBrace) {
            if self.peek() == Some(&TokenKind::KwReturn) {
                self.bump(); // return
                if !self.eat(&TokenKind::LParen) {
                    self.error(self.peek_span(), "expected `(`");
                    return None;
                }
                let binder = match self.peek()?.clone() {
                    TokenKind::Lower(n) => {
                        self.bump();
                        n
                    }
                    _ => {
                        self.error(self.peek_span(), "expected binder");
                        return None;
                    }
                };
                if !self.eat(&TokenKind::RParen) {
                    self.error(self.peek_span(), "expected `)`");
                    return None;
                }
                if !self.eat(&TokenKind::Arrow) {
                    self.error(self.peek_span(), "expected `->`");
                    return None;
                }
                let body_r = self.expr(0)?;
                ret = Some(ReturnClause {
                    binder,
                    body: Rc::new(body_r),
                });
            } else {
                let c = self.op_clause()?;
                clauses.push(c);
            }
        }
        let end = self.peek_span();
        self.eat(&TokenKind::RBrace);
        Some(spanned(
            Expr::Handle {
                body: Rc::new(body),
                handler: Rc::new(Handler {
                    multi,
                    clauses,
                    ret,
                }),
            },
            start.merge(end),
        ))
    }

    fn op_clause(&mut self) -> Option<Spanned<OpClause>> {
        let start = self.peek_span();
        // `Effect.op(params)` (Effect optional: `op(params)`)
        let (effect, op) = match self.peek()?.clone() {
            TokenKind::Upper(eff) => {
                self.bump();
                if !self.eat(&TokenKind::Dot) {
                    self.error(self.peek_span(), "expected `.` after effect name");
                    return None;
                }
                let op = match self.peek()?.clone() {
                    TokenKind::Lower(o) => {
                        self.bump();
                        o
                    }
                    _ => {
                        self.error(self.peek_span(), "expected operation name");
                        return None;
                    }
                };
                (Some(eff), op)
            }
            TokenKind::Lower(o) => {
                self.bump();
                (None, o)
            }
            _ => {
                self.error(self.peek_span(), "expected `Effect.op` clause");
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
                    TokenKind::Lower(n) => {
                        self.bump();
                        params.push(spanned(Param { name: n, ann: None }, pspan));
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
        if !self.eat(&TokenKind::Arrow) {
            self.error(self.peek_span(), "expected `->`");
            return None;
        }
        let body = self.expr(0)?;
        let end = body.span;
        Some(spanned(
            OpClause {
                effect,
                op,
                params,
                body: Rc::new(body),
            },
            start.merge(end),
        ))
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
                        let ann = if self.eat(&TokenKind::Colon) {
                            Some(self.type_ann()?)
                        } else {
                            None
                        };
                        params.push(spanned(Param { name: pn, ann }, pspan));
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
        // optional effect row: `/ { Name, Name }`. `None` distinguishes an
        // absent annotation (infer the row) from an explicit pure `/ {}`.
        let effect_row = if self.eat(&TokenKind::Slash) {
            Some(self.effect_row())
        } else {
            None
        };
        // optional return type: `-> Type` (checked since 2026-10-09)
        let ret_ann = if self.eat(&TokenKind::Arrow) {
            Some(self.type_ann()?)
        } else {
            None
        };
        let body = self.block()?;
        let end = body.span;
        Some(spanned(
            Decl::Fn(FnDecl {
                is_pub,
                name,
                params,
                effect_row,
                ret_ann,
                body: Rc::new(body),
            }),
            start.merge(end),
        ))
    }

    /// Parse `{ Name, Name }` after `/`, keeping effect head names with spans.
    fn effect_row(&mut self) -> Vec<Spanned<String>> {
        let mut names = Vec::new();
        if !self.eat(&TokenKind::LBrace) {
            self.error(self.peek_span(), "expected `{` for effect row");
            return names;
        }
        if self.peek() != Some(&TokenKind::RBrace) {
            loop {
                let lspan = self.peek_span();
                match self.peek().cloned() {
                    Some(TokenKind::Upper(n)) => {
                        self.bump();
                        // skip optional effect type args: `(...)`
                        if self.eat(&TokenKind::LParen) {
                            self.skip_balanced_parens();
                        }
                        names.push(spanned(n, lspan));
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
        let ann = if self.eat(&TokenKind::Colon) {
            Some(self.type_ann()?)
        } else {
            None
        };
        if !self.eat(&TokenKind::Eq) {
            self.error(self.peek_span(), "expected `=` in let binding");
            return None;
        }
        let value = self.expr(0)?;
        let span = start.merge(value.span);
        Some(spanned(Stmt::Let { name, ann, value }, span))
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
                TokenKind::KwFn
                    | TokenKind::KwPub
                    | TokenKind::KwImport
                    | TokenKind::KwEffect
                    | TokenKind::KwType
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
    fn lambda_parses_and_round_trips() {
        assert_eq!(p("fn(x, y) { x + y }"), "(fn (x y) (block (+ x y)))");
        assert_eq!(p("fn() { 0 }"), "(fn () (block 0))");
        // A lambda as a call argument.
        assert_eq!(
            p("map(xs, fn(n) { n * 2 })"),
            "(call map xs (fn (n) (block (* n 2))))"
        );
    }

    #[test]
    fn effect_multi_modifier_parses() {
        let src = "effect multi Flip { fn flip() -> Bool }\n\
                   effect Exn { fn fail() -> Unit }\n\
                   pub fn main() { io.println(\"x\") }\n";
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "parse: {d:?}");
        let Decl::Effect(flip) = &m.decls[0].node else {
            panic!("expected effect")
        };
        let Decl::Effect(exn) = &m.decls[1].node else {
            panic!("expected effect")
        };
        assert!(flip.is_multi, "effect multi Flip -> is_multi");
        assert!(!exn.is_multi, "unmarked effect -> one-shot");
    }

    #[test]
    fn effect_decl_with_type_param_parses() {
        let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
                   pub fn main() { io.println(\"x\") }\n";
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "parse: {d:?}");
        let Decl::Effect(e) = &m.decls[0].node else {
            panic!("expected effect")
        };
        assert_eq!(e.params, vec!["s".to_string()]);
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
        let labels: Vec<&str> = f
            .effect_row
            .as_ref()
            .unwrap()
            .iter()
            .map(|l| l.node.as_str())
            .collect();
        assert_eq!(labels, ["IO"]);
        assert!(f.is_pub);
    }

    #[test]
    fn parses_type_decl() {
        let src = "type List(a) {\n  Nil,\n  Cons(a, List(a))\n}\n";
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "diags: {d:?}");
        assert_eq!(
            crate::ast::pretty(&m),
            "(module (type List (Nil) (Cons a (List a))))"
        );
    }

    #[test]
    fn parses_match() {
        let src = "fn f(o) {\n  match o {\n    None -> 0\n    Some(x) -> x\n  }\n}\n";
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "diags: {d:?}");
        assert_eq!(
            crate::ast::pretty(&m),
            "(module (fn f (o) (block (match o (None 0) (Some (x) x)))))"
        );
    }

    #[test]
    fn distinguishes_absent_from_explicit_pure_row() {
        let (m1, _) = parse_module(&Session::new(), "fn f() { 1 }\n");
        let Decl::Fn(f1) = &m1.decls[0].node else {
            panic!("expected fn")
        };
        assert!(f1.effect_row.is_none(), "unannotated => None");
        let (m2, _) = parse_module(&Session::new(), "fn f() / {} { 1 }\n");
        let Decl::Fn(f2) = &m2.decls[0].node else {
            panic!("expected fn")
        };
        assert_eq!(
            f2.effect_row.as_ref().map(|r| r.len()),
            Some(0),
            "explicit pure => Some([])"
        );
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
    fn parses_handle_with_resume_and_return() {
        let src = "fn f() {\n  handle g() with {\n    Log.log(m) -> resume(m)\n    return(r) -> r\n  }\n}\n";
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "diags: {d:?}");
        assert_eq!(
            crate::ast::pretty(&m),
            "(module (fn f () (block (handle (call g) (Log.log (m) (resume m)) (return r r)))))"
        );
    }

    #[test]
    fn parses_multi_handler() {
        let src = "fn f() {\n  handle g() with multi {\n    Flip.flip() -> resume(True)\n  }\n}\n";
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "diags: {d:?}");
        let Decl::Fn(f) = &m.decls[0].node else {
            panic!("expected fn")
        };
        let tail = f.body.node.tail.as_ref().unwrap();
        let Expr::Handle { handler, .. } = &tail.node else {
            panic!("expected handle")
        };
        assert!(handler.multi);
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
                Decl::Type(_) => None,
            })
            .collect();
        assert!(names.contains(&"ok".to_string()), "names: {names:?}");
    }
}
