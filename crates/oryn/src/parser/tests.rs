use super::*;

use crate::lexer::lex;

/// Helper: lex + parse source, assert no errors, return statements.
fn parse_ok(source: &str) -> Vec<Spanned<Statement>> {
    let (tokens, lex_errors) = lex(source);
    assert!(lex_errors.is_empty());
    let (stmts, parse_errors) = parse(tokens);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");

    stmts
}

#[test]
fn builds_ast_from_tokens() {
    let stmts = parse_ok("let x = 5");

    assert_eq!(stmts.len(), 1);
    assert!(matches!(&stmts[0].node, Statement::Let { name, .. } if name == "x"));
}

#[test]
fn reports_parse_errors() {
    let (tokens, _) = lex("let = 5");
    let (_, errors) = parse(tokens);

    assert!(!errors.is_empty());
}

#[test]
fn expressions_carry_spans() {
    let stmts = parse_ok("5 + 10");

    assert_eq!(stmts.len(), 1);
    // The whole expression "5 + 10" should span from 0..6
    assert_eq!(stmts[0].span.start, 0);
}
