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

    // Literals.
    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i32>().ok())]
    Int(i32),

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

    // Punctuation.
    #[token(",")]
    Comma,
    #[token("(")]
    LeftParen,
    #[token(")")]
    RightParen,
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
            Token::Int(n) => write!(f, "{n}"),
            Token::Equals => write!(f, "="),
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Multiply => write!(f, "*"),
            Token::Divide => write!(f, "/"),
            Token::Comma => write!(f, ","),
            Token::LeftParen => write!(f, "("),
            Token::RightParen => write!(f, ")"),
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
            // logos returns `Err(())` for unrecognized input — we just
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
    fn test_lex() {
        let (tokens, errors) = lex("let x = 5 + 10");

        assert!(errors.is_empty());
        assert_eq!(tokens[0].0, Token::Let);
        assert_eq!(tokens[1].0, Token::Ident("x".to_string()));
        assert_eq!(tokens[2].0, Token::Equals);
        assert_eq!(tokens[3].0, Token::Int(5));
        assert_eq!(tokens[4].0, Token::Plus);
        assert_eq!(tokens[5].0, Token::Int(10));
    }
}
