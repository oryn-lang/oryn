use std::fmt::{Display, Formatter, Result};

use logos::{Logos, Span};

use crate::errors::OrynError;

/// A single lexical token produced by [`lex`].
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

    // Literals.
    #[token("true")]
    True,
    #[token("false")]
    False,
    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().parse::<f32>().ok())]
    Float(f32),
    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i32>().ok())]
    Int(i32),
    #[regex(r#""[^"]*""#, |lex| {
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
            Token::Comma => write!(f, ","),
            Token::LeftParen => write!(f, "("),
            Token::RightParen => write!(f, ")"),
            Token::LeftCurly => write!(f, "{{"),
            Token::RightCurly => write!(f, "}}"),
            Token::Newline => write!(f, "newline"),
            Token::Ident(name) => write!(f, "{name}"),
        }
    }
}

/// Tokenizes source code. Returns tokens paired with byte-offset spans,
/// plus any [`OrynError::Lexer`] errors for unrecognized characters.
///
/// ```
/// let (tokens, errors) = oryn::lex("let x = 5");
///
/// assert!(errors.is_empty());
/// assert_eq!(tokens[0].0, oryn::Token::Let);
/// ```
pub fn lex(source: &str) -> (Vec<(Token, Span)>, Vec<OrynError>) {
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
}
