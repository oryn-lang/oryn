use super::types::{BuiltinFunction, ListMethod, ModuleTable};
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

// ---------------------------------------------------------------------------
// List type checking and opcode emission
// ---------------------------------------------------------------------------

#[test]
fn list_literal_emits_make_list() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2, 3]").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::MakeList(3))),
        "expected MakeList(3), got {:?}",
        chunk.instructions
    );
}

#[test]
fn list_index_emits_list_get() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2, 3]\nlet y = xs[0]").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::ListGet)),
        "expected ListGet, got {:?}",
        chunk.instructions
    );
}

#[test]
fn list_index_assignment_emits_list_set() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2, 3]\nxs[0] = 42").unwrap();
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::ListSet)),
        "expected ListSet, got {:?}",
        chunk.instructions
    );
}

#[test]
fn list_len_method_emits_call_list_method() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2, 3]\nlet n = xs.len()").unwrap();
    let expected = ListMethod::Len as u8;
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::CallListMethod(id, 0) if *id == expected))
    );
}

#[test]
fn list_push_method_emits_call_list_method() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1]\nxs.push(2)").unwrap();
    let expected = ListMethod::Push as u8;
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::CallListMethod(id, 1) if *id == expected))
    );
}

#[test]
fn list_pop_method_emits_call_list_method() {
    let chunk = crate::Chunk::compile("let xs: [int] = [1, 2]\nlet last = xs.pop()").unwrap();
    let expected = ListMethod::Pop as u8;
    assert!(
        chunk
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::CallListMethod(id, 0) if *id == expected))
    );
}

#[test]
fn unknown_list_method_is_compile_error() {
    let errors = crate::Chunk::compile("let xs: [int] = [1]\nxs.frobnicate()").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("unknown list method `frobnicate`")),
        "expected unknown-method error, got {errors:?}"
    );
}

#[test]
fn list_method_wrong_arity_is_error() {
    let errors = crate::Chunk::compile("let xs: [int] = [1]\nxs.len(99)").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("list `len` takes 0 argument(s)")),
        "expected arity error, got {errors:?}"
    );
}

#[test]
fn heterogeneous_list_literal_is_type_error() {
    let errors = crate::Chunk::compile(r#"let xs = [1, "hello"]"#).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("list element type mismatch")),
        "expected a list element type mismatch, got {errors:?}"
    );
}

#[test]
fn empty_list_without_annotation_is_error() {
    let errors = crate::Chunk::compile("let xs = []").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("empty list literal")),
        "expected empty-list error, got {errors:?}"
    );
}

#[test]
fn wrong_element_type_rejected_against_annotation() {
    let errors = crate::Chunk::compile(r#"let xs: [int] = ["a"]"#).unwrap_err();
    assert!(
        errors.iter().any(|e| {
            let s = format!("{e}");
            s.contains("type mismatch") || s.contains("element type")
        }),
        "expected a type mismatch, got {errors:?}"
    );
}

#[test]
fn indexing_non_list_is_error() {
    let errors = crate::Chunk::compile("let x = 5\nlet y = x[0]").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("cannot index into non-list type")),
        "expected non-list index error, got {errors:?}"
    );
}

#[test]
fn string_index_is_error() {
    let errors = crate::Chunk::compile("let xs: [int] = [1]\nlet y = xs[\"a\"]").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("list index must be `int`")),
        "expected int-index error, got {errors:?}"
    );
}

#[test]
fn push_argument_type_checked_against_element_type() {
    let errors = crate::Chunk::compile("let xs: [int] = [1]\nxs.push(\"a\")").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("list `push` argument 1 type mismatch")),
        "expected push type mismatch, got {errors:?}"
    );
}

#[test]
fn list_type_round_trips_through_display_name() {
    // Compile a function taking [int] and returning [int] — verify
    // the error rendering for a type mismatch shows `[int]` properly.
    let errors = crate::Chunk::compile(
        "fn head(xs: [int]) -> int { rn xs[0] }\nlet y: [String] = head([1, 2])",
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("[int]") || format!("{e}").contains("[String]")),
        "expected list type in error message, got {errors:?}"
    );
}
