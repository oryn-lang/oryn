use std::fmt::{Display, Formatter, Result};

use logos::{Logos, Span};

use crate::errors::OrynError;

/// A single lexical token produced by [`lex`].
///
/// [`Token::Comment`] is captured as a real token so that trivia-aware
/// consumers (hover doc comments, future doc tests) can walk them. The
/// parser never sees them because [`lex`] filters them out before
/// returning; use [`lex_all`] to get the unfiltered stream.
#[derive(Debug, PartialEq, Clone, Logos)]
#[logos(skip r"[ \t]+")]
pub enum Token {
    // Keywords.
    #[token("let")]
    Let,
    #[token("val")]
    Val,
    #[token("fn")]
    Fn,
    #[token("rn")]
    Rn,
    #[token("obj")]
    Obj,
    #[token("use")]
    Use,
    #[token("for")]
    For,
    #[token("in")]
    In,
    #[token("pub")]
    Pub,
    #[token("import")]
    Import,

    // Literals.
    #[token("true")]
    True,
    #[token("false")]
    False,
    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().parse::<f32>().ok())]
    Float(f32),
    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i32>().ok())]
    Int(i32),
    #[regex(r#""(\\.|[^"\\])*""#, |lex| {
        let s = lex.slice();

        // Strip the surrounding quotes.
        s[1..s.len()-1].to_string()
    })]
    String(String),

    // Operators.
    #[token("=")]
    Equals,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Multiply,
    #[token("/")]
    Divide,
    #[token("==")]
    EqualsEquals,
    #[token("!=")]
    NotEquals,
    #[token("<")]
    LessThan,
    #[token(">")]
    GreaterThan,
    #[token("<=")]
    LessThanEquals,
    #[token(">=")]
    GreaterThanEquals,
    #[token("and")]
    And,
    #[token("or")]
    Or,
    #[token("not")]
    Not,

    // Control flow.
    #[token("if")]
    If,
    #[token("elif")]
    Elif,
    #[token("else")]
    Else,
    #[token("while")]
    While,
    #[token("break")]
    Break,
    #[token("continue")]
    Continue,

    // Punctuation.
    #[token("..=")]
    DotDotEquals,
    #[token("..")]
    DotDot,
    #[token(".")]
    Dot,
    #[token(":")]
    Colon,
    #[token("->")]
    Arrow,
    #[token(",")]
    Comma,
    #[token("(")]
    LeftParen,
    #[token(")")]
    RightParen,
    #[token("{")]
    LeftCurly,
    #[token("}")]
    RightCurly,
    #[token("\n")]
    Newline,

    // Identifiers.
    #[regex("[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),

    // Trivia. Line comments are captured (not skipped) so the LSP
    // and future doc-test runner can attach them to declarations via
    // `DocTable`. [`lex`] filters these out before the parser sees
    // them; [`lex_all`] preserves them.
    #[regex(r"//[^\n]*", |lex| lex.slice().to_string(), priority = 3, allow_greedy = true)]
    Comment(String),
}

// Needed by chumsky's `Rich` error type so it can format "expected X, found Y"
// messages with human-readable token names.
impl Display for Token {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Token::Let => write!(f, "let"),
            Token::Val => write!(f, "val"),
            Token::Fn => write!(f, "fn"),
            Token::Rn => write!(f, "rn"),
            Token::Obj => write!(f, "obj"),
            Token::Use => write!(f, "use"),
            Token::For => write!(f, "for"),
            Token::In => write!(f, "in"),
            Token::Pub => write!(f, "pub"),
            Token::Import => write!(f, "import"),
            Token::True => write!(f, "true"),
            Token::False => write!(f, "false"),
            Token::Float(n) => write!(f, "{n}"),
            Token::Int(n) => write!(f, "{n}"),
            Token::String(s) => write!(f, "{s}"),
            Token::Equals => write!(f, "="),
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Multiply => write!(f, "*"),
            Token::Divide => write!(f, "/"),
            Token::EqualsEquals => write!(f, "=="),
            Token::NotEquals => write!(f, "!="),
            Token::LessThan => write!(f, "<"),
            Token::GreaterThan => write!(f, ">"),
            Token::LessThanEquals => write!(f, "<="),
            Token::GreaterThanEquals => write!(f, ">="),
            Token::And => write!(f, "and"),
            Token::Or => write!(f, "or"),
            Token::Not => write!(f, "not"),
            Token::If => write!(f, "if"),
            Token::Else => write!(f, "else"),
            Token::Elif => write!(f, "elif"),
            Token::While => write!(f, "while"),
            Token::Break => write!(f, "break"),
            Token::Continue => write!(f, "continue"),
            Token::DotDotEquals => write!(f, "..="),
            Token::DotDot => write!(f, ".."),
            Token::Dot => write!(f, "."),
            Token::Colon => write!(f, ":"),
            Token::Arrow => write!(f, "->"),
            Token::Comma => write!(f, ","),
            Token::LeftParen => write!(f, "("),
            Token::RightParen => write!(f, ")"),
            Token::LeftCurly => write!(f, "{{"),
            Token::RightCurly => write!(f, "}}"),
            Token::Newline => write!(f, "newline"),
            Token::Ident(name) => write!(f, "{name}"),
            Token::Comment(text) => write!(f, "{text}"),
        }
    }
}

/// Tokenizes source code. Returns tokens paired with byte-offset spans,
/// plus any [`OrynError::Lexer`] errors for unrecognized characters.
///
/// [`Token::Comment`] tokens are filtered out so the parser never sees
/// them. Use [`lex_all`] if you need the full trivia-preserving stream
/// (e.g. for building a [`crate::DocTable`]).
///
/// ```
/// let (tokens, errors) = oryn::lex("let x = 5");
///
/// assert!(errors.is_empty());
/// assert_eq!(tokens[0].0, oryn::Token::Let);
/// ```
pub fn lex(source: &str) -> (Vec<(Token, Span)>, Vec<OrynError>) {
    let (tokens, errors) = lex_all(source);
    let tokens = tokens
        .into_iter()
        .filter(|(tok, _)| !matches!(tok, Token::Comment(_)))
        .collect();
    (tokens, errors)
}

/// Like [`lex`], but preserves [`Token::Comment`] trivia. Used by
/// [`crate::DocTable`] to associate doc comments with declarations.
pub fn lex_all(source: &str) -> (Vec<(Token, Span)>, Vec<OrynError>) {
    let mut lex = Token::lexer(source);
    let mut tokens = Vec::new();
    let mut errors = Vec::new();

    while let Some(token) = lex.next() {
        match token {
            Ok(token) => tokens.push((token, lex.span())),
            // logos returns `Err(())` for unrecognized input, we just
            // need the span to report where it happened.
            Err(()) => errors.push(OrynError::Lexer { span: lex.span() }),
        }
    }

    (tokens, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_source() {
        let (tokens, errors) = lex("let x = 5 + 10");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();

        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Let,
                Token::Ident("x".into()),
                Token::Equals,
                Token::Int(5),
                Token::Plus,
                Token::Int(10),
            ]
        );
    }

    #[test]
    fn reports_invalid_characters() {
        let (_, errors) = lex("let x = @");

        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn skips_line_comments() {
        let (tokens, errors) = lex("// leading comment\nlet x = 5 // trailing\nprint(x)");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();

        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Newline,
                Token::Let,
                Token::Ident("x".into()),
                Token::Equals,
                Token::Int(5),
                Token::Newline,
                Token::Ident("print".into()),
                Token::LeftParen,
                Token::Ident("x".into()),
                Token::RightParen,
            ]
        );
    }

    #[test]
    fn comment_only_lines_are_skipped() {
        let (tokens, errors) = lex("// just a comment");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();

        assert!(errors.is_empty());
        assert!(kinds.is_empty());
    }

    #[test]
    fn lex_all_preserves_comments_with_spans() {
        let source = "// top\nlet x = 5 // trailing";
        let (tokens, errors) = lex_all(source);

        assert!(errors.is_empty());

        let comments: Vec<_> = tokens
            .iter()
            .filter_map(|(t, span)| match t {
                Token::Comment(text) => Some((text.clone(), span.clone())),
                _ => None,
            })
            .collect();

        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].0, "// top");
        assert_eq!(&source[comments[0].1.clone()], "// top");
        assert_eq!(comments[1].0, "// trailing");
        assert_eq!(&source[comments[1].1.clone()], "// trailing");
    }

    #[test]
    fn lex_all_and_lex_agree_on_non_comment_tokens() {
        let source = "// doc\nlet x = 5\n// more\nlet y = 6";
        let (filtered, _) = lex(source);
        let (all, _) = lex_all(source);

        let all_non_comment: Vec<_> = all
            .into_iter()
            .filter(|(t, _)| !matches!(t, Token::Comment(_)))
            .collect();

        assert_eq!(filtered, all_non_comment);
    }
}
