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
    #[token("return")]
    Return,
    #[token("struct")]
    Struct,
    #[token("enum")]
    Enum,
    #[token("match")]
    Match,
    #[token("use")]
    Use,
    #[token("for")]
    For,
    #[token("in")]
    In,
    #[token("pub")]
    Pub,
    #[token("mut")]
    Mut,
    #[token("import")]
    Import,
    #[token("try")]
    Try,
    #[token("must")]
    Must,
    #[token("maybe")]
    Maybe,
    #[token("error")]
    Error,
    /// `of` — used inside precise error-union type annotations
    /// (`error of MathError int`). Hard keyword; reserved across
    /// the whole grammar, even though it currently only appears in
    /// type position.
    #[token("of")]
    Of,
    /// `ok` — success-side pattern in a match arm over an error
    /// union (`ok v => ...`). Hard keyword; reserved across the
    /// whole grammar.
    #[token("ok")]
    Ok,
    #[token("nil")]
    Nil,
    #[token("test")]
    Test,
    #[token("assert")]
    Assert,

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
    #[token("orelse")]
    Orelse,
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
    #[token("=>")]
    FatArrow,
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
    #[token("[")]
    LeftBracket,
    #[token("]")]
    RightBracket,
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
            Token::Return => write!(f, "return"),
            Token::Struct => write!(f, "struct"),
            Token::Enum => write!(f, "enum"),
            Token::Match => write!(f, "match"),
            Token::Use => write!(f, "use"),
            Token::For => write!(f, "for"),
            Token::In => write!(f, "in"),
            Token::Pub => write!(f, "pub"),
            Token::Mut => write!(f, "mut"),
            Token::Import => write!(f, "import"),
            Token::Try => write!(f, "try"),
            Token::Must => write!(f, "must"),
            Token::Maybe => write!(f, "maybe"),
            Token::Error => write!(f, "error"),
            Token::Of => write!(f, "of"),
            Token::Ok => write!(f, "ok"),
            Token::Nil => write!(f, "nil"),
            Token::Test => write!(f, "test"),
            Token::Assert => write!(f, "assert"),
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
            Token::Orelse => write!(f, "orelse"),
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
            Token::FatArrow => write!(f, "=>"),
            Token::Comma => write!(f, ","),
            Token::LeftParen => write!(f, "("),
            Token::RightParen => write!(f, ")"),
            Token::LeftCurly => write!(f, "{{"),
            Token::RightCurly => write!(f, "}}"),
            Token::LeftBracket => write!(f, "["),
            Token::RightBracket => write!(f, "]"),
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

    #[test]
    fn tokenizes_nil_and_error_tokens() {
        // `maybe T` — prefix nillable type modifier (Slice 5).
        let (tokens, errors) = lex("let x: maybe int = nil");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();
        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Let,
                Token::Ident("x".into()),
                Token::Colon,
                Token::Maybe,
                Token::Ident("int".into()),
                Token::Equals,
                Token::Nil,
            ]
        );

        // try keyword
        let (tokens, errors) = lex("try foo()");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();
        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Try,
                Token::Ident("foo".into()),
                Token::LeftParen,
                Token::RightParen,
            ]
        );

        // nil coalescing
        let (tokens, errors) = lex("a orelse b");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();
        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Ident("a".into()),
                Token::Orelse,
                Token::Ident("b".into()),
            ]
        );

        // `must` — error unwrap operator (Slice 5; replaces `!expr`).
        let (tokens, errors) = lex("must foo()");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();
        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Must,
                Token::Ident("foo".into()),
                Token::LeftParen,
                Token::RightParen,
            ]
        );
    }

    #[test]
    fn bare_question_and_bang_are_lex_errors_after_rename() {
        // After Slice 5: `?` and bare `!` are no longer language
        // tokens. They produce lex errors so users get a clear
        // signal that they're using removed syntax.
        let (_, errors) = lex("let x: int?");
        assert!(!errors.is_empty(), "expected lex error for bare `?`");

        let (_, errors) = lex("!y");
        assert!(!errors.is_empty(), "expected lex error for bare `!`");

        // `!=` still lexes as the not-equals operator (logos
        // longest-match keeps the digraph intact).
        let (tokens, _) = lex("a != b");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();
        assert!(kinds.contains(&Token::NotEquals));
    }

    #[test]
    fn return_and_struct_are_keywords_not_idents() {
        // Pin the rename: `return` and `struct` lex as their own
        // tokens, not as identifiers. The previous abbreviated
        // forms `rn` and `obj` no longer exist.
        let (tokens, errors) = lex("return\nstruct Foo");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();

        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Return,
                Token::Newline,
                Token::Struct,
                Token::Ident("Foo".into()),
            ]
        );
    }

    #[test]
    fn rn_and_obj_lex_as_identifiers_after_rename() {
        // After the W2/W3 rename, the old keywords `rn` and `obj`
        // are just identifiers. They're free to be used as variable
        // names by user code.
        let (tokens, errors) = lex("let rn = 1\nlet obj = 2");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();

        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Let,
                Token::Ident("rn".into()),
                Token::Equals,
                Token::Int(1),
                Token::Newline,
                Token::Let,
                Token::Ident("obj".into()),
                Token::Equals,
                Token::Int(2),
            ]
        );
    }

    #[test]
    fn unless_lexes_as_identifier_after_removal() {
        // W6: `unless` was removed; it's now a regular ident.
        let (tokens, errors) = lex("unless ready { print(0) }");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();

        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Ident("unless".into()),
                Token::Ident("ready".into()),
                Token::LeftCurly,
                Token::Ident("print".into()),
                Token::LeftParen,
                Token::Int(0),
                Token::RightParen,
                Token::RightCurly,
            ]
        );
    }

    #[test]
    fn tokenizes_test_and_assert_keywords() {
        let (tokens, errors) = lex("test \"adds\" { assert(1 == 1) }");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();

        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Test,
                Token::String("adds".into()),
                Token::LeftCurly,
                Token::Assert,
                Token::LeftParen,
                Token::Int(1),
                Token::EqualsEquals,
                Token::Int(1),
                Token::RightParen,
                Token::RightCurly,
            ]
        );

        // Identifiers that start with `test`/`assert` stay as identifiers.
        let (tokens, errors) = lex("tester asserting");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();
        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Ident("tester".into()),
                Token::Ident("asserting".into()),
            ]
        );
    }

    #[test]
    fn bang_does_not_break_not_equals() {
        let (tokens, errors) = lex("a != b");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();
        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Ident("a".into()),
                Token::NotEquals,
                Token::Ident("b".into()),
            ]
        );
    }

    #[test]
    fn orelse_keyword_and_word_boundary() {
        // `orelse` as a keyword with spaces
        let (tokens, errors) = lex("x orelse y");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();
        assert!(errors.is_empty());
        assert_eq!(
            kinds,
            vec![
                Token::Ident("x".into()),
                Token::Orelse,
                Token::Ident("y".into()),
            ]
        );

        // Identifiers that start with `orelse` stay as identifiers —
        // logos' longest-match rule keeps word boundaries intact.
        let (tokens, errors) = lex("orelseb");
        let kinds: Vec<_> = tokens.into_iter().map(|(t, _)| t).collect();
        assert!(errors.is_empty());
        assert_eq!(kinds, vec![Token::Ident("orelseb".into())]);
    }
}
