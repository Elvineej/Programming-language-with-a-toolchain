//! Lexer: source text → spanned tokens.

use crate::diag::Diagnostic;
use crate::span::Span;
use crate::Session;
use logos::Logos;

#[derive(Logos, Clone, Debug, PartialEq)]
#[logos(skip r"[ \t\r\n\f]+")]
#[logos(skip r"//[^\n]*")]
#[logos(skip r"/\*([^*]|\*[^/])*\*/")]
pub enum TokenKind {
    #[regex(r"[0-9][0-9_]*", |lex| lex.slice().replace('_', "").parse().ok())]
    Int(i64),
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*", |lex| lex.slice().replace('_', "").parse().ok())]
    Float(f64),
    #[regex(r#""([^"\\]|\\.)*""#, |lex| unescape(lex.slice()))]
    Str(String),

    // Keywords (higher priority than Lower via exact-token match).
    #[token("pub")]
    KwPub,
    #[token("fn")]
    KwFn,
    #[token("let")]
    KwLet,
    #[token("import")]
    KwImport,
    #[token("if")]
    KwIf,
    #[token("else")]
    KwElse,
    #[token("type")]
    KwType,
    #[token("linear")]
    KwLinear,
    #[token("match")]
    KwMatch,
    #[token("case")]
    KwCase,
    #[token("effect")]
    KwEffect,
    #[token("trait")]
    KwTrait,
    #[token("impl")]
    KwImpl,
    #[token("handle")]
    KwHandle,
    #[token("with")]
    KwWith,
    #[token("multi")]
    KwMulti,
    #[token("return")]
    KwReturn,
    #[token("const")]
    KwConst,
    #[token("True")]
    True,
    #[token("False")]
    False,
    #[token("Unit")]
    Unit,

    #[regex(r"[a-z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Lower(String),
    #[regex(r"[A-Z][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Upper(String),

    // Multi-char operators must precede their single-char prefixes.
    #[token("+.")]
    PlusDot,
    #[token("-.")]
    MinusDot,
    #[token("*.")]
    StarDot,
    #[token("/.")]
    SlashDot,
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("<=")]
    Le,
    #[token(">=")]
    Ge,
    #[token("->")]
    Arrow,
    #[token("=>")]
    FatArrow,
    #[token("<>")]
    Concat,
    #[token("|>")]
    Pipe,
    #[token("&&")]
    AmpAmp,
    #[token("||")]
    PipePipe,
    #[token("..")]
    DotDot,

    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("=")]
    Eq,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("!")]
    Bang,
    #[token("?")]
    Question,
    #[token(".")]
    Dot,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
}

fn unescape(raw: &str) -> Option<String> {
    // raw includes surrounding quotes.
    let inner = &raw[1..raw.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                _ => return None,
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub fn lex(_session: &Session, text: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut tokens = Vec::new();
    let mut diags = Vec::new();
    let mut lexer = TokenKind::lexer(text);
    while let Some(result) = lexer.next() {
        let range = lexer.span();
        let span = Span::new(range.start as u32, range.end as u32);
        match result {
            Ok(kind) => tokens.push(Token { kind, span }),
            Err(()) => diags.push(
                Diagnostic::error("E0001", "unexpected character")
                    .with_label(span, "not a valid Elya token"),
            ),
        }
    }
    (tokens, diags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    fn kinds(text: &str) -> Vec<TokenKind> {
        let (toks, diags) = lex(&Session::new(), text);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        toks.into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lexes_keywords_idents_and_ops() {
        use TokenKind::*;
        assert_eq!(
            kinds("pub fn main"),
            vec![KwPub, KwFn, Lower("main".into())]
        );
        assert_eq!(kinds("1 + 2"), vec![Int(1), Plus, Int(2)]);
    }

    #[test]
    fn lexes_string_and_qualified_call_tokens() {
        use TokenKind::*;
        assert_eq!(
            kinds(r#"io.println("hi")"#),
            vec![
                Lower("io".into()),
                Dot,
                Lower("println".into()),
                LParen,
                Str("hi".into()),
                RParen
            ]
        );
    }

    #[test]
    fn function_keyword_does_not_eat_identifiers() {
        use TokenKind::*;
        assert_eq!(kinds("function"), vec![Lower("function".into())]);
    }

    #[test]
    fn unexpected_char_produces_diagnostic_not_panic() {
        let (_toks, diags) = lex(&Session::new(), "let x = §");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "E0001");
    }
}
