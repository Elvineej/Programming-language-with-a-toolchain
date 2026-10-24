//! Recursive-descent (declarations) + Pratt (expressions) parser.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::lex::{lex, Token, TokenKind};
use crate::span::{spanned, Span, Spanned};
use crate::Session;
use std::rc::Rc;

struct Parser<'a> {
    tokens: &'a [Token],
    /// The source, for the layout rule: whether a newline separates two tokens.
    text: &'a str,
    /// The file's first offset in the program's span space (modules part 1 D5):
    /// spans are `base + offset in text`.
    base: u32,
    pos: usize,
    diags: Vec<Diagnostic>,
    /// Inside braces (a block, `match` arms, a handler's clauses) a newline before
    /// an infix operator or a call's `(` ends the expression; inside parentheses it
    /// is whitespace (spec 2026-10-23-elya-literals-and-layout D4).
    braces: bool,
    /// "unexpected end of file" was reported: the missing `}`s after it are not
    /// reported again (spec 2026-10-23-elya-parser-recovery D1).
    eof_reported: bool,
    /// The first token of the declaration being parsed (D4's resume point is
    /// found from it).
    decl_start: usize,
    /// Parser errors raised, counting those D3 did not report (spec
    /// 2026-10-24-elya-diagnostics-at-the-edges): "did this fail with an error"
    /// is this count, not the length of `diags`.
    raised: usize,
    /// D4: how many `expr`, `pattern` and `type_ann` calls are open.
    depth: usize,
    /// D4: the height of the tallest expression returned by an `expr` call since
    /// the enclosing `expr` began (each call folds its own height in).
    inner_h: usize,
    /// D4: after E0102 the parser reads nothing more of the declaration (every
    /// `peek` sees the end, nothing is reported) until `module` resumes at this
    /// token: the next declaration.
    stop: Option<usize>,
}

/// D4: the deepest an expression may be nested (levels: expression nodes on a
/// path, a parenthesised expression counting as one), and the most `expr`,
/// `pattern` and `type_ann` calls open at once.
pub const MAX_NESTING: usize = 1024;

/// `Int`'s largest magnitude: `9223372036854775807`, or `9223372036854775808`
/// under a folded `-`.
const INT_MAX_MAGNITUDE: u128 = i64::MAX as u128;

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token], text: &'a str, base: u32) -> Parser<'a> {
        Parser {
            tokens,
            text,
            base,
            pos: 0,
            diags: Vec::new(),
            braces: true,
            eof_reported: false,
            decl_start: 0,
            raised: 0,
            depth: 0,
            inner_h: 0,
            stop: None,
        }
    }

    /// Run `f` with the layout rule on (`true`, braces) or off (parentheses),
    /// restoring the enclosing setting after, on every path.
    fn with_braces<T>(&mut self, on: bool, f: impl FnOnce(&mut Self) -> T) -> T {
        let outer = self.braces;
        self.braces = on;
        let r = f(self);
        self.braces = outer;
        r
    }

    /// Whether a newline separates the token at `pos` from the one before it
    /// (comments between them included).
    fn newline_before(&self, pos: usize) -> bool {
        if pos == 0 || pos >= self.tokens.len() {
            return false;
        }
        let from = self.tokens[pos - 1].span.end;
        let to = self.tokens[pos].span.start;
        self.source(from, to).is_some_and(|gap| gap.contains('\n'))
    }

    /// The source text between two span offsets of this file.
    fn source(&self, from: u32, to: u32) -> Option<&'a str> {
        let from = from.checked_sub(self.base)? as usize;
        let to = to.checked_sub(self.base)? as usize;
        self.text.get(from..to)
    }

    /// A `-` at `pos` written against the token after it (`-x`, `-1`): the prefix
    /// style, which starts a statement at a line's start.
    fn glued_minus(&self, pos: usize) -> bool {
        match (self.tokens.get(pos), self.tokens.get(pos + 1)) {
            // `-.` too (spec 2026-10-24-elya-float-part-2 D2).
            (Some(m), Some(next)) => {
                matches!(m.kind, TokenKind::Minus | TokenKind::MinusDot)
                    && m.span.end == next.span.start
            }
            _ => false,
        }
    }

    /// E0101 at the operator starting this line (D4).
    fn layout_error(&mut self) {
        let at = self.peek_span();
        let text = self.source(at.start, at.end).unwrap_or("");
        self.push(
            Diagnostic::error(
                "E0101",
                "an operator at the start of a line does not continue the line before",
            )
            .with_label(at, "starts a line")
            .with_help(format!(
                "end the line before with `{text}`, or put the whole expression in parentheses"
            )),
        );
    }

    /// An integer literal's value: its magnitude, negated when a `-` was folded into
    /// it, or E0002 when it does not fit in `Int` (D2). Parsing goes on after the
    /// error, at 0, so one bad literal is one diagnostic.
    fn int_literal(&mut self, negative: bool, magnitude: u128, span: Span) -> i64 {
        let limit = if negative {
            INT_MAX_MAGNITUDE + 1
        } else {
            INT_MAX_MAGNITUDE
        };
        if magnitude > limit {
            self.push(
                Diagnostic::error("E0002", "integer literal out of range")
                    .with_label(span, "does not fit in `Int`")
                    .with_help("`Int` is 64-bit: from -9223372036854775808 to 9223372036854775807"),
            );
            return 0;
        }
        if negative {
            (magnitude as i128).wrapping_neg() as i64
        } else {
            magnitude as i64
        }
    }

    fn peek(&self) -> Option<&TokenKind> {
        if self.stop.is_some() {
            return None; // D4: the declaration ends at E0102
        }
        self.tokens.get(self.pos).map(|t| &t.kind)
    }

    fn peek_span(&self) -> Span {
        // At the end of the file: an empty span there, in this file (it was
        // offset 0, which in a program of several files is the root's first
        // byte; the modules review's R4).
        let eof = self.base + self.text.len() as u32;
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or(Span::new(eof, eof))
    }

    fn bump(&mut self) -> Option<&Token> {
        if self.stop.is_some() {
            return None;
        }
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
        self.push(Diagnostic::error("E0100", msg).with_label(span, "here"));
    }

    /// Every parser diagnostic goes through here. D3: the end of the file is
    /// reported once -- an error labelled at the end of the file, exactly where
    /// the previous one was, is not reported (elsewhere two errors at one token
    /// are two missing tokens, and both are reported: the review's F1). D4:
    /// nothing is reported after E0102 until the next declaration.
    fn push(&mut self, d: Diagnostic) {
        self.raised += 1;
        if self.stop.is_some() {
            return;
        }
        let at = |d: &Diagnostic| d.labels.first().map(|l| l.span);
        if let Some(last) = self.diags.last() {
            let eof = self.base + self.text.len() as u32;
            if at(last).is_some() && at(last) == at(&d) && at(&d).map(|s| s.start) == Some(eof) {
                return;
            }
        }
        self.diags.push(d);
    }

    /// D4: E0102 at `at`, once; then the declaration stops (see `stop`).
    fn too_deep(&mut self, at: Span) {
        if self.stop.is_some() {
            return;
        }
        self.push(
            Diagnostic::error("E0102", "expression nested too deeply")
                .with_label(at, format!("more than {MAX_NESTING} levels deep here"))
                .with_help("bind an inner part with `let`, or move it into a function"),
        );
        self.stop = Some(self.resume_point());
    }

    /// D4: where parsing resumes after E0102 -- the first declaration after the
    /// current token at which every `{` opened since this declaration began is
    /// closed (an operation `fn name` inside the same `effect { }` is not one:
    /// the review's F2). When the braces never balance (a truncated file), the
    /// first declaration at a line's start, then the end of the file.
    fn resume_point(&self) -> usize {
        let brace = |k: &TokenKind| match k {
            TokenKind::LBrace => 1,
            TokenKind::RBrace => -1,
            _ => 0,
        };
        let mut open: i64 = self.tokens[self.decl_start.min(self.pos)..self.pos]
            .iter()
            .map(|t| brace(&t.kind))
            .sum();
        for i in self.pos..self.tokens.len() {
            if open <= 0 && i > self.pos && self.starts_decl(i) {
                return i;
            }
            open += brace(&self.tokens[i].kind);
        }
        (self.pos + 1..self.tokens.len())
            .find(|&i| self.starts_decl(i) && self.newline_before(i))
            .unwrap_or(self.tokens.len())
    }

    /// D4: open one more `expr`, `pattern` or `type_ann` call, or E0102 past the
    /// limit (and `false`).
    fn enter(&mut self) -> bool {
        if self.stop.is_some() {
            return false;
        }
        if self.depth >= MAX_NESTING {
            let at = self.peek_span();
            self.too_deep(at);
            return false;
        }
        self.depth += 1;
        true
    }

    /// D1: a parse that ran out of tokens without saying so is E0100 "unexpected
    /// end of file", once.
    fn eof_error(&mut self) {
        self.raised += 1;
        if self.eof_reported || self.stop.is_some() {
            return;
        }
        self.eof_reported = true;
        let at = self.peek_span();
        self.push(
            Diagnostic::error("E0100", "unexpected end of file")
                .with_label(at, "the file ends here"),
        );
        self.raised -= 1; // counted once, above
    }

    /// A closing `}`: eaten, or E0100 "expected `}`" -- at the end of the file
    /// once, however many braces are still open there (D1; the review's F2).
    fn close_brace(&mut self) -> bool {
        let end = self.peek_span();
        if self.eat(&TokenKind::RBrace) {
            return true;
        }
        if self.stop.is_some() {
            self.raised += 1;
            return false;
        }
        if self.peek().is_none() {
            if !self.eof_reported {
                self.eof_reported = true;
                self.error(end, "expected `}`");
            }
        } else {
            self.error(end, "expected `}`");
        }
        false
    }

    /// D4: a token that starts a top-level declaration: `pub`, `import`,
    /// `effect`, `type` or `linear` (the review's F1), or `fn` followed by a name (a lambda's `fn` is
    /// followed by `(`).
    fn starts_decl(&self, pos: usize) -> bool {
        match self.tokens.get(pos).map(|t| &t.kind) {
            Some(TokenKind::KwPub)
            | Some(TokenKind::KwImport)
            | Some(TokenKind::KwEffect)
            | Some(TokenKind::KwType)
            | Some(TokenKind::KwLinear) => true,
            Some(TokenKind::KwFn) => matches!(
                self.tokens.get(pos + 1).map(|t| &t.kind),
                Some(TokenKind::Lower(_))
            ),
            _ => false,
        }
    }

    /// D2: after a statement failed (it began at token `start`), skip to where
    /// the next one begins: a token that starts a line and can start a statement
    /// (not an operator: E0101's `+` is part of the bad statement), with no
    /// bracket opened since `start` left open; a line-start `let` when no `{` is left open; the
    /// block's own `}` (not consumed); a declaration at a line's start; or the
    /// end of the file. An unclosed `(` or `[` is dropped by the `}` that closes
    /// the brace around it.
    fn skip_statement(&mut self, start: usize) {
        let mut open: Vec<TokenKind> = Vec::new();
        let push_or_pop = |open: &mut Vec<TokenKind>, k: &TokenKind| -> bool {
            match k {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                    open.push(k.clone());
                    true
                }
                TokenKind::RParen | TokenKind::RBracket => {
                    let want = if *k == TokenKind::RParen {
                        TokenKind::LParen
                    } else {
                        TokenKind::LBracket
                    };
                    if open.last() == Some(&want) {
                        open.pop();
                    }
                    true
                }
                TokenKind::RBrace => {
                    // Close up to and including the innermost `{`; with none open
                    // it is the block's own `}`.
                    match open.iter().rposition(|o| *o == TokenKind::LBrace) {
                        Some(i) => {
                            open.truncate(i);
                            true
                        }
                        None => false,
                    }
                }
                _ => true,
            }
        };
        for i in start..self.pos {
            let k = self.tokens[i].kind.clone();
            push_or_pop(&mut open, &k);
        }
        let mut first = self.pos == start;
        while let Some(k) = self.peek().cloned() {
            if !first {
                let line_start = self.newline_before(self.pos);
                if line_start
                    && ((open.is_empty() && can_start_statement(&k)) || self.starts_decl(self.pos))
                {
                    return;
                }
                if line_start && k == TokenKind::KwLet && !open.contains(&TokenKind::LBrace) {
                    return;
                }
            }
            first = false;
            if !push_or_pop(&mut open, &k) {
                return; // the block's `}`
            }
            self.bump();
        }
    }

    // ---- Expressions (Pratt) ----

    /// Pratt expression parser. `min_bp` is the minimum binding power.
    ///
    /// D4 of the diagnostics spec: at most `MAX_NESTING` calls open, and the
    /// height of what it returns folded into the enclosing call's `inner_h`.
    fn expr(&mut self, min_bp: u8) -> Option<Spanned<Expr>> {
        if !self.enter() {
            return None;
        }
        let outer = std::mem::replace(&mut self.inner_h, 0);
        let r = self.expr_chain(min_bp);
        self.inner_h = outer.max(self.inner_h);
        self.depth -= 1;
        r
    }

    /// D4: `h` levels is E0102 past the limit, at `at`.
    fn check_height(&mut self, h: usize, at: Span) -> Option<()> {
        if h > MAX_NESTING {
            self.too_deep(at);
            return None;
        }
        Some(())
    }

    /// `expr`'s body; leaves its result's height in `inner_h`. An atom is one
    /// level more than the expressions inside it, each step of a chain (an
    /// operator, a call) one more than both its sides: a chain is built by this
    /// loop, not by recursion, and is limited as nesting is.
    fn expr_chain(&mut self, min_bp: u8) -> Option<Spanned<Expr>> {
        let mut lhs = self.atom()?;
        let mut h = self.inner_h + 1;
        self.check_height(h, lhs.span)?;
        // `cloned()` frees the immutable borrow so the body can mutate `self`.
        while let Some(kind) = self.peek().cloned() {
            // postfix call: `expr(...)`; in braces, not across a newline (D4).
            if kind == TokenKind::LParen {
                if self.braces && self.newline_before(self.pos) {
                    break;
                }
                self.inner_h = 0;
                lhs = self.finish_call(lhs)?;
                h = h.max(self.inner_h) + 1;
                self.check_height(h, lhs.span)?;
                continue;
            }
            let Some((l_bp, r_bp, op)) = infix_bp(&kind) else {
                break;
            };
            if l_bp < min_bp {
                break;
            }
            // In braces an infix operator does not continue across a newline (D4):
            // a glued `-` starts the next statement, any other operator is E0101.
            if self.braces && self.newline_before(self.pos) {
                if self.glued_minus(self.pos) {
                    break;
                }
                self.layout_error();
                return None;
            }
            self.bump(); // operator
            self.inner_h = 0;
            let rhs = self.expr(r_bp)?;
            let span = lhs.span.merge(rhs.span);
            h = h.max(self.inner_h) + 1;
            self.check_height(h, span)?;
            lhs = spanned(
                Expr::Binary {
                    op,
                    lhs: Rc::new(lhs),
                    rhs: Rc::new(rhs),
                },
                span,
            );
        }
        self.inner_h = h;
        Some(lhs)
    }

    fn finish_call(&mut self, callee: Spanned<Expr>) -> Option<Spanned<Expr>> {
        let start = callee.span;
        self.bump(); // '('
        let args = self.with_braces(false, |p| {
            let mut args = Vec::new();
            if p.peek() != Some(&TokenKind::RParen) {
                loop {
                    args.push(p.expr(0)?);
                    if !p.eat(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            Some(args)
        })?;
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
            TokenKind::Int(m) => {
                self.bump();
                let n = self.int_literal(false, m, span);
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
                // A `-` directly before a numeric literal is part of it (D1).
                let lit = self.peek_span();
                match self.peek().cloned() {
                    Some(TokenKind::Int(m)) => {
                        self.bump();
                        let n = self.int_literal(true, m, span.merge(lit));
                        return Some(spanned(Expr::Int(n), span.merge(lit)));
                    }
                    Some(TokenKind::Float(x)) => {
                        self.bump();
                        return Some(spanned(Expr::Float(-x), span.merge(lit)));
                    }
                    _ => {}
                }
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
            // `-.e` (spec 2026-10-24-elya-float-part-2 D2).
            TokenKind::MinusDot => {
                self.bump();
                let e = self.expr(100)?;
                let s = span.merge(e.span);
                Some(spanned(
                    Expr::Unary {
                        op: UnOp::NegF,
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
                let e = self.with_braces(false, |p| p.expr(0))?;
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
                let arg = self.with_braces(false, |p| p.expr(0))?;
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
            self.decl_start = self.pos;
            let reported = self.raised;
            match self.peek() {
                Some(TokenKind::KwImport) => {
                    if let Some(i) = self.import() {
                        imports.push(i);
                    } else {
                        if self.raised == reported {
                            self.eof_error();
                        }
                        self.recover_to_decl();
                    }
                }
                Some(TokenKind::KwPub) | Some(TokenKind::KwFn) => {
                    let start = self.peek_span();
                    let is_pub = self.eat(&TokenKind::KwPub);
                    let name = match (
                        self.tokens.get(self.pos).map(|t| &t.kind),
                        self.tokens.get(self.pos + 1),
                    ) {
                        (Some(TokenKind::KwFn), Some(t)) => match &t.kind {
                            TokenKind::Lower(n) => Some(spanned(n.clone(), t.span)),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(d) = self.fn_decl(is_pub) {
                        decls.push(d);
                    } else {
                        if self.raised == reported {
                            self.eof_error();
                        }
                        // D3: a function whose header failed is still a known
                        // name, so its callers are not reported. Inference never
                        // runs on a module with a parse error.
                        if let Some(name) = name {
                            let span = start.merge(name.span);
                            decls.push(spanned(
                                Decl::Fn(FnDecl {
                                    is_pub,
                                    name: name.node,
                                    params: Vec::new(),
                                    effect_row: None,
                                    ret_ann: None,
                                    body: Rc::new(spanned(
                                        Block {
                                            stmts: Rc::new([]),
                                            tail: Some(Rc::new(spanned(Expr::Error, span))),
                                        },
                                        span,
                                    )),
                                }),
                                span,
                            ));
                        }
                        self.recover_to_decl();
                    }
                }
                Some(TokenKind::KwEffect) => {
                    if let Some(e) = self.effect_decl() {
                        decls.push(e);
                    } else {
                        if self.raised == reported {
                            self.eof_error();
                        }
                        self.recover_to_decl();
                    }
                }
                Some(TokenKind::KwType) | Some(TokenKind::KwLinear) => {
                    if let Some(t) = self.type_decl() {
                        decls.push(t);
                    } else {
                        // D1, missed by the first version (the review's F4).
                        if self.raised == reported {
                            self.eof_error();
                        }
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
            // D4: after E0102, the next declaration.
            if let Some(resume) = self.stop.take() {
                self.pos = resume;
                self.depth = 0;
                self.inner_h = 0;
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
        // D1: it was accepted unchecked, also at the end of the file.
        if !self.close_brace() {
            return None;
        }
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
        if !self.close_brace() {
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
        // The arms are braces (D4); the scrutinee is in the enclosing setting.
        self.with_braces(true, |p| {
            while p.peek().is_some() && p.peek() != Some(&TokenKind::RBrace) {
                arms.push(p.match_arm()?);
            }
            Some(())
        })?;
        let end = self.peek_span();
        if !self.close_brace() {
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

    /// A pattern; D4 counts it as one more call open.
    fn pattern(&mut self) -> Option<Spanned<Pattern>> {
        if !self.enter() {
            return None;
        }
        let r = self.pattern_inner();
        self.depth -= 1;
        r
    }

    fn pattern_inner(&mut self) -> Option<Spanned<Pattern>> {
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
            TokenKind::Int(m) => {
                self.bump();
                let n = self.int_literal(false, m, span);
                Some(spanned(Pattern::Lit(PatLit::Int(n)), span))
            }
            // A negative literal (D3): the `-` folded into the number, as in
            // expressions.
            TokenKind::Minus => {
                self.bump();
                let lit = self.peek_span();
                match self.peek().cloned() {
                    Some(TokenKind::Int(m)) => {
                        self.bump();
                        let n = self.int_literal(true, m, span.merge(lit));
                        Some(spanned(Pattern::Lit(PatLit::Int(n)), span.merge(lit)))
                    }
                    _ => {
                        self.error(lit, "expected an integer after `-` in a pattern");
                        None
                    }
                }
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

    /// A type annotation; D4 counts it as one more call open.
    fn type_ann(&mut self) -> Option<Spanned<TypeAnn>> {
        if !self.enter() {
            return None;
        }
        let r = self.type_ann_inner();
        self.depth -= 1;
        r
    }

    fn type_ann_inner(&mut self) -> Option<Spanned<TypeAnn>> {
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
        // `with any` (2026-10-16): `any` is contextual, a keyword only here.
        let any = matches!(self.peek(), Some(TokenKind::Lower(n)) if n == "any");
        if any {
            let at = self.peek_span();
            self.bump();
            if multi {
                self.error(at, "a `with any` handler is one-shot: drop `multi`");
                return None;
            }
        }
        if !self.eat(&TokenKind::LBrace) {
            self.error(self.peek_span(), "expected `{`");
            return None;
        }
        // The clauses are braces (D4); the body before `with` is in the enclosing
        // setting.
        let (clauses, ret) = self.with_braces(true, |p| {
            let mut clauses: Vec<Spanned<OpClause>> = Vec::new();
            let mut ret = None;
            while p.peek().is_some() && p.peek() != Some(&TokenKind::RBrace) {
                // A clause body is no statement list: a `-` starting a line after a
                // clause cannot begin a statement here, so it is E0101 (the review's
                // R4).
                if p.peek() == Some(&TokenKind::Minus) && p.newline_before(p.pos) {
                    p.layout_error();
                    return None;
                }
                if p.peek() == Some(&TokenKind::KwReturn) {
                    p.bump(); // return
                    if !p.eat(&TokenKind::LParen) {
                        p.error(p.peek_span(), "expected `(`");
                        return None;
                    }
                    let binder = match p.peek()?.clone() {
                        TokenKind::Lower(n) => {
                            p.bump();
                            n
                        }
                        _ => {
                            p.error(p.peek_span(), "expected binder");
                            return None;
                        }
                    };
                    if !p.eat(&TokenKind::RParen) {
                        p.error(p.peek_span(), "expected `)`");
                        return None;
                    }
                    if !p.eat(&TokenKind::Arrow) {
                        p.error(p.peek_span(), "expected `->`");
                        return None;
                    }
                    let body_r = p.expr(0)?;
                    ret = Some(ReturnClause {
                        binder,
                        body: Rc::new(body_r),
                    });
                } else {
                    let c = p.op_clause()?;
                    if any {
                        let ok =
                            c.node.effect.is_none() && c.node.op == "op" && c.node.params.len() == 1;
                        if !ok {
                            p.error(
                                c.span,
                                "a `with any` handler has one operation clause, `op(o)`, and a `return` clause",
                            );
                            return None;
                        }
                        if !clauses.is_empty() {
                            p.error(c.span, "a `with any` handler has one `op(o)` clause");
                            return None;
                        }
                    }
                    clauses.push(c);
                }
            }
            Some((clauses, ret))
        })?;
        let end = self.peek_span();
        if any && clauses.is_empty() {
            self.error(end, "a `with any` handler needs its `op(o)` clause");
            return None;
        }
        if !self.close_brace() {
            return None;
        }
        Some(spanned(
            Expr::Handle {
                body: Rc::new(body),
                handler: Rc::new(Handler {
                    multi,
                    any,
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
        let mut end;
        loop {
            let seg_span = self.peek_span();
            match self.peek()?.clone() {
                TokenKind::Lower(seg) => {
                    self.bump();
                    path.push(seg);
                    end = seg_span;
                }
                _ => {
                    self.error(seg_span, "expected module path segment");
                    return None;
                }
            }
            // `/ {` starts the effect allow-list, not another segment.
            let list_next = matches!(
                self.tokens.get(self.pos + 1).map(|t| &t.kind),
                Some(TokenKind::LBrace)
            );
            if self.peek() != Some(&TokenKind::Slash) || list_next {
                break;
            }
            self.bump(); // '/'
        }
        // `as` is contextual: a keyword only here (modules part 1 D2).
        let mut alias = None;
        if matches!(self.peek(), Some(TokenKind::Lower(n)) if n == "as") {
            self.bump();
            let at = self.peek_span();
            match self.peek().cloned() {
                Some(TokenKind::Lower(n)) => {
                    self.bump();
                    alias = Some(n);
                    end = at;
                }
                _ => {
                    self.error(at, "expected a name after `as`");
                    return None;
                }
            }
        }
        let mut effects = None;
        if self.eat(&TokenKind::Slash) {
            let list_start = self.peek_span();
            let row = self.effect_row();
            end = end.merge(list_start);
            if let Some(last) = row.last() {
                end = end.merge(last.span);
            }
            effects = Some(row);
        }
        Some(spanned(
            Import {
                path,
                alias,
                effects,
            },
            start.merge(end),
        ))
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
        self.close_brace();
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
        self.with_braces(true, |p| p.block_inner())
    }

    fn block_inner(&mut self) -> Option<Spanned<Block>> {
        let start = self.peek_span();
        if !self.eat(&TokenKind::LBrace) {
            self.error(start, "expected `{`");
            return None;
        }
        let mut stmts = Vec::new();
        let mut tail = None;
        while self.peek().is_some() && self.peek() != Some(&TokenKind::RBrace) {
            // A declaration at a line's start: this block's `}` is missing (D1).
            if self.newline_before(self.pos) && self.starts_decl(self.pos) {
                break;
            }
            let at = self.pos;
            let first = self.peek_span();
            let reported = self.raised;
            if self.peek() == Some(&TokenKind::KwLet) {
                // D2: a bad `let` is one error; its name, when read, stays bound.
                let name = match self.tokens.get(at + 1).map(|t| &t.kind) {
                    Some(TokenKind::Lower(n)) => Some(n.clone()),
                    _ => None,
                };
                match self.let_stmt() {
                    Some(st) => stmts.push(st),
                    None => {
                        let span = self.recover_statement(at, reported, first);
                        let error = spanned(Expr::Error, span);
                        stmts.push(spanned(
                            match name {
                                Some(name) => Stmt::Let {
                                    name,
                                    ann: None,
                                    value: error,
                                },
                                None => Stmt::Expr(error),
                            },
                            span,
                        ));
                    }
                }
            } else {
                match self.expr(0) {
                    Some(e) => {
                        if self.peek() == Some(&TokenKind::RBrace) {
                            tail = Some(e);
                        } else {
                            let span = e.span;
                            stmts.push(spanned(Stmt::Expr(e), span));
                        }
                    }
                    None => {
                        let span = self.recover_statement(at, reported, first);
                        stmts.push(spanned(Stmt::Expr(spanned(Expr::Error, span)), span));
                    }
                }
            }
        }
        let end = self.peek_span();
        // A block's value is its last expression (spec
        // 2026-10-24-elya-block-tails-and-exponents D1): one that ends in a `let`,
        // or has nothing in it, has none.
        if tail.is_none() && self.stop.is_none() && self.peek() == Some(&TokenKind::RBrace) {
            match stmts.last() {
                None => self.push(
                    Diagnostic::error("E0103", "a block has no value")
                        .with_label(start.merge(end), "an empty block")
                        .with_help("write the value it has, `Unit` for none"),
                ),
                Some(st) if matches!(&st.node, Stmt::Let { value, .. } if !matches!(value.node, Expr::Error)) =>
                {
                    let at = st.span;
                    self.push(
                        Diagnostic::error("E0103", "a block cannot end with `let`")
                            .with_label(at, "the block's last statement")
                            .with_help(
                                "end the block with its value: the bound name, another expression, or `Unit`",
                            ),
                    );
                }
                Some(_) => {}
            }
        }
        self.close_brace();
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

    /// D2: a statement that began at token `at` failed; report the end of the
    /// file if nothing was said, skip to the next statement, and return the
    /// span the error node covers.
    fn recover_statement(&mut self, at: usize, reported: usize, first: Span) -> Span {
        if self.raised == reported && self.peek().is_none() {
            self.eof_error();
        }
        self.skip_statement(at);
        let last = self.pos.max(at + 1).min(self.tokens.len()) - 1;
        match self.tokens.get(last) {
            Some(t) if self.pos > at => first.merge(t.span),
            _ => first,
        }
    }

    fn block_expr(&mut self) -> Option<Spanned<Expr>> {
        let b = self.block()?;
        Some(spanned(Expr::Block(b.node), b.span))
    }

    fn recover_to_decl(&mut self) {
        // Synchronize: skip tokens until a declaration (D4: a `fn` only when a
        // name follows it) or EOF.
        while self.peek().is_some() {
            if self.starts_decl(self.pos) {
                return;
            }
            self.bump();
        }
    }
}

/// D2: a token a statement can begin with (an expression's first token, or `let`).
fn can_start_statement(k: &TokenKind) -> bool {
    use TokenKind as T;
    matches!(
        k,
        T::Int(_)
            | T::Float(_)
            | T::Str(_)
            | T::True
            | T::False
            | T::Unit
            | T::Lower(_)
            | T::Upper(_)
            | T::KwLet
            | T::KwIf
            | T::KwMatch
            | T::KwHandle
            | T::KwFn
            | T::LParen
            | T::LBrace
            | T::Minus
            | T::MinusDot
            | T::Bang
    )
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
        T::LtDot => (30, 31, BinOp::LtF),
        T::LeDot => (30, 31, BinOp::LeF),
        T::GtDot => (30, 31, BinOp::GtF),
        T::GeDot => (30, 31, BinOp::GeF),
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
    let mut p = Parser::new(&tokens, text, 0);
    let e = p.expr(0);
    diags.append(&mut p.diags);
    (e, diags)
}

pub fn parse_module(session: &Session, text: &str) -> (Module, Vec<Diagnostic>) {
    parse_module_at(session, text, 0)
}

/// [`parse_module`] for a file whose spans start at `base` in the program's span
/// space (modules part 1 D5).
pub fn parse_module_at(session: &Session, text: &str, base: u32) -> (Module, Vec<Diagnostic>) {
    let (tokens, mut diags) = crate::lex::lex_at(session, text, base);
    let mut p = Parser::new(&tokens, text, base);
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
