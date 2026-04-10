use super::types::{BuiltinFunction, ModuleTable};
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

    let output = compile(stmts, ModuleTable::default(), 0, 0, vec![]);

    assert_eq!(
        output.instructions,
        vec![Instruction::PushInt(3), Instruction::Pop,]
    );
    assert_eq!(output.instructions.len(), output.spans.len());
}

#[test]
fn expression_statements_are_popped() {
    let stmts = vec![spanned(Statement::Expression(spanned(Expression::Int(1))))];
    let output = compile(stmts, ModuleTable::default(), 0, 0, vec![]);

    assert_eq!(output.instructions.last(), Some(&Instruction::Pop));
}

#[test]
fn builtin_calls_are_lowered_to_typed_builtins() {
    let stmts = vec![spanned(Statement::Expression(spanned(Expression::Call {
        name: "print".to_string(),
        args: vec![spanned(Expression::Int(1))],
    })))];

    let output = compile(stmts, ModuleTable::default(), 0, 0, vec![]);

    assert_eq!(
        output.instructions,
        vec![
            Instruction::PushInt(1),
            Instruction::CallBuiltin(BuiltinFunction::Print, 1),
            Instruction::Pop,
        ]
    );
}

#[test]
fn assert_lowers_to_assert_instruction() {
    let chunk = crate::Chunk::compile("assert(true)").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Assert))
    );
}

#[test]
fn assert_rejects_non_bool_condition() {
    let errors = crate::Chunk::compile("assert(5)").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("assert") || format!("{e}").contains("type")),
        "expected a type-related compile error for non-bool assert, got {errors:?}"
    );
}

#[test]
fn test_blocks_are_collected_into_tests_vec() {
    let chunk =
        crate::Chunk::compile("test \"one\" { assert(true) }\ntest \"two\" { assert(1 == 1) }")
            .unwrap();

    assert_eq!(chunk.tests().len(), 2);
    assert_eq!(chunk.tests()[0].display_name, "one");
    assert_eq!(chunk.tests()[1].display_name, "two");
}

#[test]
fn test_body_compiles_as_function() {
    // Each test should produce a synthetic function with a body
    // containing at least one Assert instruction.
    let chunk = crate::Chunk::compile("test \"ok\" { assert(true) }").unwrap();

    assert_eq!(chunk.tests().len(), 1);
    let idx = chunk.tests()[0].function_idx;
    let func = &chunk.functions[idx];
    assert!(
        func.instructions
            .iter()
            .any(|i| matches!(i, Instruction::Assert)),
        "test body should contain an Assert instruction, got {:?}",
        func.instructions
    );
}
