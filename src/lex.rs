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
    /// An integer literal's magnitude, saturated: the range check is the parser's
    /// (E0002), since `-9223372036854775808` is in range and `9223372036854775808`
    /// alone is not (spec 2026-10-23-elya-literals-and-layout D2).
    #[regex(r"[0-9][0-9_]*", |lex| lex.slice().replace('_', "").parse::<u128>().unwrap_or(u128::MAX))]
    Int(u128),
    /// A float literal: digits on both sides of the `.`, and an optional exponent
    /// (`1.0e19`, `5.0e-324`; spec 2026-10-24-elya-block-tails-and-exponents
    /// D2). Out of range (infinite, or a non-zero literal rounding to zero) is
    /// E0002, reported by `lex_at`.
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?", |lex| lex.slice().replace('_', "").parse().ok())]
    Float(f64),
    /// D2: an exponent on an integer (`1e19`) is E0003; it is read as the float
    /// it means, so parsing goes on.
    #[regex(r"[0-9][0-9_]*[eE][+-]?[0-9][0-9_]*", |lex| lex.slice().replace('_', "").parse().ok())]
    IntExponent(f64),
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
    // Float comparisons (spec 2026-10-24-elya-float-part-2 D1).
    #[token("<.")]
    LtDot,
    #[token("<=.")]
    LeDot,
    #[token(">.")]
    GtDot,
    #[token(">=.")]
    GeDot,
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

pub fn lex(session: &Session, text: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    lex_at(session, text, 0)
}

/// [`lex`] with every span shifted by `base` (a file's first offset in the
/// program's span space, modules part 1 D5).
/// D2: a float literal names a finite float, and a non-zero one is not 0;
/// otherwise E0002.
fn float_range(slice: &str, x: f64, span: Span, diags: &mut Vec<Diagnostic>) {
    let mantissa = slice.split(['e', 'E']).next().unwrap_or("");
    let nonzero = mantissa.bytes().any(|b| (b'1'..=b'9').contains(&b));
    if !x.is_finite() || (x == 0.0 && nonzero) {
        diags.push(
            Diagnostic::error("E0002", "float literal out of range")
                .with_label(span, "not a finite, non-zero `Float`")
                .with_help("`Float` is 64-bit: magnitudes from about 4.9e-324 to 1.8e308"),
        );
    }
}

pub fn lex_at(_session: &Session, text: &str, base: u32) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut tokens = Vec::new();
    let mut diags = Vec::new();
    let mut lexer = TokenKind::lexer(text);
    while let Some(result) = lexer.next() {
        let range = lexer.span();
        let span = Span::new(base + range.start as u32, base + range.end as u32);
        match result {
            Ok(TokenKind::IntExponent(x)) => {
                let slice = lexer.slice();
                let (mantissa, exponent) = slice.split_at(slice.find(['e', 'E']).unwrap_or(0));
                // A `_` before the `e` is not carried into the help (the review's F2).
                let mantissa = mantissa.trim_end_matches('_');
                diags.push(
                    Diagnostic::error("E0003", "an exponent needs a float literal")
                        .with_label(span, "an integer with an exponent")
                        .with_help(format!("write `{mantissa}.0{exponent}`")),
                );
                // Out of range as well: both (the review's F1).
                float_range(slice, x, span, &mut diags);
                tokens.push(Token {
                    kind: TokenKind::Float(x),
                    span,
                });
            }
            Ok(TokenKind::Float(x)) => {
                float_range(lexer.slice(), x, span, &mut diags);
                tokens.push(Token {
                    kind: TokenKind::Float(x),
                    span,
                });
            }
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
