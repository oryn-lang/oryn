use crate::parser::{BinOp, Expression, Statement};

// Flat bytecode that the VM executes. The compiler's job is to walk the
// tree-shaped AST and flatten it into this linear sequence. The VM uses
// a stack, so operand order matters — left before right.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Instruction {
    PushInt(i32),
    LoadVar(String),
    StoreVar(String),
    Add,
    Sub,
    Mul,
    Div,
    Call(String, usize),
    Pop,
}

pub(crate) fn compile(statements: Vec<Statement>) -> Vec<Instruction> {
    let mut instructions = Vec::new();

    for stmt in statements {
        compile_statement(&mut instructions, stmt);
    }

    instructions
}

fn compile_statement(instructions: &mut Vec<Instruction>, stmt: Statement) {
    match stmt {
        Statement::Let { name, value } => {
            // Evaluate the right-hand side, then store the result.
            compile_expression(instructions, value);

            instructions.push(Instruction::StoreVar(name));
        }
        Statement::Expression(expr) => {
            // Expression statements (like `print(x)`) still leave a value
            // on the stack, so we `Pop` it to keep the stack clean.
            compile_expression(instructions, expr);

            instructions.push(Instruction::Pop);
        }
    }
}

fn compile_expression(instructions: &mut Vec<Instruction>, expr: Expression) {
    match expr {
        Expression::Int(n) => {
            instructions.push(Instruction::PushInt(n));
        }
        Expression::Ident(name) => {
            instructions.push(Instruction::LoadVar(name));
        }
        Expression::BinaryOp { op, left, right } => {
            // Left goes on the stack first, then right. The op instruction
            // pops both and pushes the result — order matters for `-` and `/`.
            compile_expression(instructions, *left);
            compile_expression(instructions, *right);

            instructions.push(match op {
                BinOp::Add => Instruction::Add,
                BinOp::Sub => Instruction::Sub,
                BinOp::Mul => Instruction::Mul,
                BinOp::Div => Instruction::Div,
            });
        }
        Expression::Call { name, args } => {
            // Push all args left-to-right, then `Call` tells the VM how
            // many values to pull off the stack for this function.
            let arity = args.len();

            for arg in args {
                compile_expression(instructions, arg);
            }

            instructions.push(Instruction::Call(name, arity));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_let() {
        let stmts = vec![Statement::Let {
            name: "x".into(),
            value: Expression::Int(42),
        }];

        let instructions = compile(stmts);

        assert_eq!(
            instructions,
            vec![Instruction::PushInt(42), Instruction::StoreVar("x".into())]
        );
    }

    #[test]
    fn test_compile_binary_op() {
        let stmts = vec![Statement::Expression(Expression::BinaryOp {
            op: BinOp::Add,
            left: Box::new(Expression::Int(1)),
            right: Box::new(Expression::Int(2)),
        })];

        let instructions = compile(stmts);

        assert_eq!(
            instructions,
            vec![
                Instruction::PushInt(1),
                Instruction::PushInt(2),
                Instruction::Add,
                Instruction::Pop,
            ]
        );
    }

    #[test]
    fn test_compile_call() {
        let stmts = vec![Statement::Expression(Expression::Call {
            name: "print".into(),
            args: vec![Expression::Int(10)],
        })];

        let instructions = compile(stmts);

        assert_eq!(
            instructions,
            vec![
                Instruction::PushInt(10),
                Instruction::Call("print".into(), 1),
                Instruction::Pop,
            ]
        );
    }
}
