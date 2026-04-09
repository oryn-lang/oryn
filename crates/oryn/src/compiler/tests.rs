use super::*;

use crate::parser::{BinOp, Expression, Spanned, Statement};

fn spanned<T>(node: T) -> Spanned<T> {
    Spanned { node, span: 0..0 }
}

#[test]
fn flattens_ast_to_instructions() {
    let stmts = vec![spanned(Statement::Expression(spanned(
        Expression::BinaryOp {
            op: BinOp::Add,
            left: Box::new(spanned(Expression::Int(1))),
            right: Box::new(spanned(Expression::Int(2))),
        },
    )))];

    let output = compile(stmts);

    assert_eq!(
        output.instructions,
        vec![Instruction::PushInt(3), Instruction::Pop,]
    );
    assert_eq!(output.instructions.len(), output.spans.len());
}

#[test]
fn expression_statements_are_popped() {
    let stmts = vec![spanned(Statement::Expression(spanned(Expression::Int(1))))];
    let output = compile(stmts);

    assert_eq!(output.instructions.last(), Some(&Instruction::Pop));
}
