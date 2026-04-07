use std::ops::Range;

use crate::parser::{BinOp, Expression, Span, Spanned, Statement, UnaryOp};

// Flat bytecode that the VM executes. The compiler's job is to walk the
// tree-shaped AST and flatten it into this linear sequence. The VM uses
// a stack, so operand order matters — left before right.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Instruction {
    PushBool(bool),
    PushInt(i32),
    LoadVar(String),
    StoreVar(String),
    SetLocal(String),
    Equal,
    NotEqual,
    LessThan,
    GreaterThan,
    LessThanEquals,
    GreaterThanEquals,
    And,
    Or,
    Not,
    Add,
    Sub,
    Mul,
    Div,
    Call(String, usize),
    // Pop the top of the stack.
    Pop,
    // Jump to a label if the top of the stack is false.
    JumpIfFalse(usize),
    // Jump to a label unconditionally.
    Jump(usize),
}

/// Compiled output: instructions paired with a parallel span table.
pub(crate) struct CompilerOutput {
    pub instructions: Vec<Instruction>,
    pub spans: Vec<Range<usize>>,
}

/// Tracks the addresses needed by `break` and `continue` inside a loop.
struct LoopContext {
    /// Instruction index of the loop condition (target for `continue`).
    start: usize,
    /// Indices of `Jump(0)` placeholders emitted by `break` statements.
    /// Patched to point past the loop after the body is compiled.
    break_patches: Vec<usize>,
}

pub(crate) fn compile(statements: Vec<Spanned<Statement>>) -> CompilerOutput {
    let mut output = CompilerOutput {
        instructions: Vec::new(),
        spans: Vec::new(),
    };
    let mut loops: Vec<LoopContext> = Vec::new();

    for stmt in statements {
        compile_statement(&mut output, &mut loops, stmt);
    }

    output
}

/// Push an instruction along with its source span.
fn emit(output: &mut CompilerOutput, instruction: Instruction, span: &Span) {
    output.instructions.push(instruction);
    output.spans.push(span.clone());
}

fn compile_statement(
    output: &mut CompilerOutput,
    loops: &mut Vec<LoopContext>,
    stmt: Spanned<Statement>,
) {
    let stmt_span = stmt.span.clone();
    match stmt.node {
        Statement::Let { name, value } => {
            compile_expression(output, value);
            emit(output, Instruction::StoreVar(name), &stmt_span);
        }
        Statement::Assignment { name, value } => {
            compile_expression(output, value);
            emit(output, Instruction::SetLocal(name), &stmt_span);
        }
        Statement::If {
            condition,
            body,
            else_body,
        } => {
            compile_expression(output, condition);

            let jump_if_false_idx = output.instructions.len();
            emit(output, Instruction::JumpIfFalse(0), &stmt_span);

            compile_expression_with_loops(output, loops, body);

            if let Some(else_body) = else_body {
                let jump_idx = output.instructions.len();
                emit(output, Instruction::Jump(0), &stmt_span);

                let else_start = output.instructions.len();
                output.instructions[jump_if_false_idx] = Instruction::JumpIfFalse(else_start);

                compile_expression_with_loops(output, loops, else_body);

                let end = output.instructions.len();
                output.instructions[jump_idx] = Instruction::Jump(end);
            } else {
                let end = output.instructions.len();
                output.instructions[jump_if_false_idx] = Instruction::JumpIfFalse(end);
            }
        }
        Statement::While { condition, body } => {
            // loop_start: the condition is re-evaluated each iteration.
            // `continue` jumps here.
            let loop_start = output.instructions.len();

            compile_expression(output, condition);

            // Exit the loop when the condition is false.
            let exit_jump_idx = output.instructions.len();
            emit(output, Instruction::JumpIfFalse(0), &stmt_span);

            // Push a loop context so break/continue inside the body
            // know where to jump.
            loops.push(LoopContext {
                start: loop_start,
                break_patches: Vec::new(),
            });

            compile_expression_with_loops(output, loops, body);

            // Jump back to re-check the condition.
            emit(output, Instruction::Jump(loop_start), &stmt_span);

            // "end" is right after the backward jump.
            let end = output.instructions.len();

            // Patch the condition's exit jump.
            output.instructions[exit_jump_idx] = Instruction::JumpIfFalse(end);

            // Patch all break statements to jump here.
            let loop_ctx = loops.pop().expect("loop context missing");
            for patch_idx in loop_ctx.break_patches {
                output.instructions[patch_idx] = Instruction::Jump(end);
            }
        }
        Statement::Break => {
            // Emit a Jump with a placeholder. The enclosing while
            // will patch it to point past the loop.
            if let Some(loop_ctx) = loops.last_mut() {
                let idx = output.instructions.len();
                emit(output, Instruction::Jump(0), &stmt_span);
                loop_ctx.break_patches.push(idx);
            }
            // break outside a loop is silently ignored for now.
            // TODO: make this a compile error.
        }
        Statement::Continue => {
            // Jump back to the loop condition.
            if let Some(loop_ctx) = loops.last() {
                emit(output, Instruction::Jump(loop_ctx.start), &stmt_span);
            }
            // continue outside a loop is silently ignored for now.
            // TODO: make this a compile error.
        }
        Statement::Expression(expr) => {
            let expr_span = expr.span.clone();
            compile_expression(output, expr);
            emit(output, Instruction::Pop, &expr_span);
        }
    }
}

/// Compile an expression, passing through the loop context stack so that
/// blocks inside if/while bodies can contain break/continue.
fn compile_expression_with_loops(
    output: &mut CompilerOutput,
    loops: &mut Vec<LoopContext>,
    expr: Spanned<Expression>,
) {
    let span = expr.span.clone();
    match expr.node {
        Expression::Block(stmts) => {
            for stmt in stmts {
                compile_statement(output, loops, stmt);
            }
        }
        // Everything else delegates to the loop-unaware version.
        other => compile_expression(output, Spanned { node: other, span }),
    }
}

fn compile_expression(output: &mut CompilerOutput, expr: Spanned<Expression>) {
    let span = expr.span.clone();
    match expr.node {
        Expression::True => {
            emit(output, Instruction::PushBool(true), &span);
        }
        Expression::False => {
            emit(output, Instruction::PushBool(false), &span);
        }
        Expression::Int(n) => {
            emit(output, Instruction::PushInt(n), &span);
        }
        Expression::Ident(name) => {
            emit(output, Instruction::LoadVar(name), &span);
        }
        Expression::BinaryOp { op, left, right } => {
            compile_expression(output, *left);
            compile_expression(output, *right);

            emit(
                output,
                match op {
                    BinOp::Equals => Instruction::Equal,
                    BinOp::NotEquals => Instruction::NotEqual,
                    BinOp::LessThan => Instruction::LessThan,
                    BinOp::GreaterThan => Instruction::GreaterThan,
                    BinOp::LessThanEquals => Instruction::LessThanEquals,
                    BinOp::GreaterThanEquals => Instruction::GreaterThanEquals,
                    BinOp::And => Instruction::And,
                    BinOp::Or => Instruction::Or,
                    BinOp::Add => Instruction::Add,
                    BinOp::Sub => Instruction::Sub,
                    BinOp::Mul => Instruction::Mul,
                    BinOp::Div => Instruction::Div,
                },
                &span,
            );
        }
        Expression::UnaryOp { op, expr: operand } => {
            compile_expression(output, *operand);

            emit(
                output,
                match op {
                    UnaryOp::Not => Instruction::Not,
                },
                &span,
            );
        }
        Expression::Call { name, args } => {
            let arity = args.len();

            for arg in args {
                compile_expression(output, arg);
            }

            emit(output, Instruction::Call(name, arity), &span);
        }
        Expression::Block(stmts) => {
            // When compiled outside a loop context (e.g. top-level),
            // blocks can't contain break/continue. Use an empty loop stack.
            let mut no_loops = Vec::new();
            for stmt in stmts {
                compile_statement(output, &mut no_loops, stmt);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create an unspanned expression (for unit tests that
    /// don't care about spans).
    fn spanned<T>(node: T) -> Spanned<T> {
        Spanned { node, span: 0..0 }
    }

    #[test]
    fn flattens_ast_to_instructions() {
        // A binary op should push left, push right, then the op instruction.
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
            vec![
                Instruction::PushInt(1),
                Instruction::PushInt(2),
                Instruction::Add,
                Instruction::Pop,
            ]
        );
        // Every instruction has a corresponding span.
        assert_eq!(output.instructions.len(), output.spans.len());
    }

    #[test]
    fn expression_statements_are_popped() {
        let stmts = vec![spanned(Statement::Expression(spanned(Expression::Int(1))))];
        let output = compile(stmts);

        assert_eq!(output.instructions.last(), Some(&Instruction::Pop));
    }
}
