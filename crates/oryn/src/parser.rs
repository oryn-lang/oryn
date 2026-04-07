use chumsky::input::{Input as _, MappedInput};
use chumsky::prelude::*;

use crate::errors::OrynError;
use crate::lexer::Token;

// Chumsky needs tokens paired with their source spans.
type Spanned = (Token, SimpleSpan);
// The input type chumsky operates on — a slice of `Spanned` tokens.
type TokenInput<'src> = MappedInput<'src, Token, SimpleSpan, &'src [Spanned]>;

/// A top-level statement in the AST.
#[derive(Debug)]
pub enum Statement {
    Let { name: String, value: Expression },
    Expression(Expression),
}

/// An expression node in the AST.
#[derive(Debug)]
pub enum Expression {
    True,
    False,
    Int(i32),
    Ident(String),
    BinaryOp {
        op: BinOp,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    UnaryOp {
        op: UnaryOp,
        expr: Box<Expression>,
    },
    Call {
        name: String,
        args: Vec<Expression>,
    },
}

/// A binary operator.
#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Equals,
    NotEquals,
    LessThan,
    GreaterThan,
    LessThanEquals,
    GreaterThanEquals,
    And,
    Or,
    Add,
    Sub,
    Mul,
    Div,
}

/// A unary operator.
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Not,
}

/// Parses a token stream into an AST. Returns the statements and any
/// [`OrynError::Parser`] errors. Partial output is returned even when
/// there are errors.
///
/// ```
/// let (tokens, _) = oryn::lex("let x = 5");
/// let (ast, errors) = oryn::parse(tokens);
///
/// assert!(errors.is_empty());
/// assert_eq!(ast.len(), 1);
/// ```
pub fn parse(tokens: Vec<(Token, std::ops::Range<usize>)>) -> (Vec<Statement>, Vec<OrynError>) {
    // Convert lexer spans (Range<usize>) into chumsky's SimpleSpan type.
    let tokens: Vec<Spanned> = tokens
        .into_iter()
        .map(|(t, s)| (t, SimpleSpan::from(s)))
        .collect();

    // Chumsky needs a span representing "end of input" for error reporting.
    let end = tokens.last().map(|(_, s)| s.end).unwrap_or(0);
    let end = SimpleSpan::from(end..end);

    // split_token_span gives chumsky a stream it can pull (token, span) pairs from.
    let input = tokens.as_slice().split_token_span(end);
    // into_output_errors returns both partial output and collected errors,
    // rather than failing on the first error.
    let (output, errors) = program().parse(input).into_output_errors();

    (
        output.unwrap_or_default(),
        errors
            .into_iter()
            .map(|e| OrynError::Parser {
                span: e.span().start..e.span().end,
                message: format!("{e}"),
            })
            .collect(),
    )
}

// Atoms are the smallest, indivisible expressions: literals, identifiers,
// function calls, and parenthesized sub-expressions. Takes the full `expr`
// parser as a parameter so atoms can contain nested expressions (e.g. in
// call args or parens).
fn atom<'src>(
    expr: impl Parser<'src, TokenInput<'src>, Expression, extra::Err<Rich<'src, Token, SimpleSpan>>>
    + Clone,
) -> impl Parser<'src, TokenInput<'src>, Expression, extra::Err<Rich<'src, Token, SimpleSpan>>> + Clone
{
    // select! matches a single token and extracts data from it.
    let bool_lit = select! { Token::True => Expression::True, Token::False => Expression::False };

    let int = select! { Token::Int(n) => Expression::Int(n) };

    // An identifier optionally followed by (args) becomes a call; otherwise
    // it stays as a plain identifier. .then(...or_not()) tries the call
    // syntax but backtracks if there are no parens.
    let ident_or_call = select! { Token::Ident(name) => name }
        .then(
            expr.clone()
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LeftParen), just(Token::RightParen))
                .or_not(),
        )
        .map(|(name, args)| match args {
            Some(args) => Expression::Call { name, args },
            None => Expression::Ident(name),
        });

    // Parenthesized expression: just strips the parens and returns the inner expr.
    let paren = expr.delimited_by(just(Token::LeftParen), just(Token::RightParen));

    bool_lit
        .or(int)
        .or(ident_or_call)
        .or(paren)
        .labelled("expression")
}

fn program<'src>()
-> impl Parser<'src, TokenInput<'src>, Vec<Statement>, extra::Err<Rich<'src, Token, SimpleSpan>>> {
    // recursive() lets the expression parser refer to itself, which is needed
    // because atoms can contain sub-expressions (parens, call args).
    let expr = recursive(|expr| {
        let atom = atom(expr.clone());

        // foldl builds a left-associative chain: it parses one atom, then
        // zero or more (op, atom) pairs, folding them into nested BinaryOps.
        // * and / bind tighter, so they're parsed first as "product".
        let product = atom.clone().foldl(
            choice((
                just(Token::Multiply).to(BinOp::Mul),
                just(Token::Divide).to(BinOp::Div),
            ))
            .then(atom)
            .repeated(),
            |left, (op, right)| Expression::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
        );

        // + and - are lower precedence, so they wrap products.
        // "1 + 2 * 3" parses as "1 + (2 * 3)" because product runs first.
        let sum = product.clone().foldl(
            choice((
                just(Token::Plus).to(BinOp::Add),
                just(Token::Minus).to(BinOp::Sub),
            ))
            .then(product)
            .repeated(),
            |left, (op, right)| Expression::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
        );

        // Comparison operators are lower precedence, so they wrap sums.
        let comparison = sum.clone().foldl(
            choice((
                just(Token::EqualsEquals).to(BinOp::Equals),
                just(Token::NotEquals).to(BinOp::NotEquals),
                just(Token::LessThan).to(BinOp::LessThan),
                just(Token::GreaterThan).to(BinOp::GreaterThan),
                just(Token::LessThanEquals).to(BinOp::LessThanEquals),
                just(Token::GreaterThanEquals).to(BinOp::GreaterThanEquals),
            ))
            .then(sum)
            .repeated(),
            |left, (op, right)| Expression::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
        );

        // Not operators are lower precedence, so they wrap comparisons.
        let not = just(Token::Not)
            .repeated()
            .foldr(comparison, |_op, expr| Expression::UnaryOp {
                op: UnaryOp::Not,
                expr: Box::new(expr),
            })
            .boxed();

        // And operators are lower precedence, so they wrap not.
        let and = not.clone().foldl(
            just(Token::And).to(BinOp::And).then(not).repeated(),
            |left, (op, right)| Expression::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
        );

        // Or operators are lower precedence, so they wrap Ands.
        let or = and.clone().foldl(
            just(Token::Or).to(BinOp::Or).then(and).repeated(),
            |left, (op, right)| Expression::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
        );

        or.labelled("expression")
    });

    // "let x = <expr>"
    let let_stmt = just(Token::Let)
        .ignore_then(select! { Token::Ident(name) => name }.labelled("variable name"))
        .then_ignore(just(Token::Equals))
        .then(expr.clone())
        .map(|(name, value)| Statement::Let { name, value })
        .labelled("let statement");

    // A bare expression as a statement (e.g. a function call).
    let expr_stmt = expr.map(Statement::Expression);

    let stmt = let_stmt.or(expr_stmt).labelled("statement");

    // Statements are separated by one or more newlines (blank lines are fine).
    // Leading newlines are skipped so files can start with blank lines.
    let newlines = just(Token::Newline).repeated().at_least(1);

    newlines
        .clone()
        .or_not()
        .ignore_then(stmt.separated_by(newlines).allow_trailing().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    /// Helper: lex + parse source, assert no errors, return statements.
    fn parse_ok(source: &str) -> Vec<Statement> {
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
        assert!(matches!(&stmts[0], Statement::Let { name, .. } if name == "x"));
    }

    #[test]
    fn reports_parse_errors() {
        let (tokens, _) = lex("let = 5");
        let (_, errors) = parse(tokens);

        assert!(!errors.is_empty());
    }
}
