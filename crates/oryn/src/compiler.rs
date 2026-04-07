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

pub(crate) fn compile(statements: Vec<Spanned<Statement>>) -> CompilerOutput {
    let mut output = CompilerOutput {
        instructions: Vec::new(),
        spans: Vec::new(),
    };

    for stmt in statements {
        compile_statement(&mut output, stmt);
    }

    output
}

/// Push an instruction along with its source span.
fn emit(output: &mut CompilerOutput, instruction: Instruction, span: &Span) {
    output.instructions.push(instruction);
    output.spans.push(span.clone());
}

fn compile_statement(output: &mut CompilerOutput, stmt: Spanned<Statement>) {
    let stmt_span = stmt.span.clone();
    match stmt.node {
        Statement::Let { name, value } => {
            // Evaluate the right-hand side, then store the result.
            compile_expression(output, value);
            emit(output, Instruction::StoreVar(name), &stmt_span);
        }
        Statement::Assignment { name, value } => {
            // Evaluate the right-hand side, then store the result.
            compile_expression(output, value);
            emit(output, Instruction::SetLocal(name), &stmt_span);
        }
        Statement::If {
            condition,
            body,
            else_body,
        } => {
            // Compile the condition, leaving a bool on the stack.
            compile_expression(output, condition);

            // Emit JumpIfFalse with a placeholder target.
            let jump_if_false_idx = output.instructions.len();
            emit(output, Instruction::JumpIfFalse(0), &stmt_span);

            // Compile the then-body block. Block compiles each inner
            // statement, which handle their own pops.
            compile_expression(output, body);

            if let Some(else_body) = else_body {
                // Emit unconditional Jump to skip the else branch.
                let jump_idx = output.instructions.len();
                emit(output, Instruction::Jump(0), &stmt_span);

                // Patch JumpIfFalse to point here (start of else).
                let else_start = output.instructions.len();
                output.instructions[jump_if_false_idx] = Instruction::JumpIfFalse(else_start);

                // Compile the else-body block (or elif desugared into a block).
                compile_expression(output, else_body);

                // Patch Jump to point here (end of if/else).
                let end = output.instructions.len();
                output.instructions[jump_idx] = Instruction::Jump(end);
            } else {
                // No else branch, patch JumpIfFalse to skip the body.
                let end = output.instructions.len();
                output.instructions[jump_if_false_idx] = Instruction::JumpIfFalse(end);
            }
        }
        Statement::Expression(expr) => {
            // Expression statements (like `print(x)`) still leave a value
            // on the stack, so we `Pop` it to keep the stack clean.
            let expr_span = expr.span.clone();
            compile_expression(output, expr);
            emit(output, Instruction::Pop, &expr_span);
        }
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
            // Left goes on the stack first, then right. The op instruction
            // pops both and pushes the result — order matters for `-` and `/`.
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
            // Push all args left-to-right, then `Call` tells the VM how
            // many values to pull off the stack for this function.
            let arity = args.len();

            for arg in args {
                compile_expression(output, arg);
            }

            emit(output, Instruction::Call(name, arity), &span);
        }
        Expression::Block(stmts) => {
            // A block compiles each statement sequentially. Each statement
            // handles its own stack effects (expression statements pop
            // their values), so the block itself leaves nothing on the stack.
            for stmt in stmts {
                compile_statement(output, stmt);
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
