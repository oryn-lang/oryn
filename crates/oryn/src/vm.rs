use std::collections::HashMap;
use std::io::Write;

use crate::compiler;
use crate::compiler::Instruction;
use crate::errors::{OrynError, RuntimeError};
use crate::lexer;
use crate::parser;

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub(crate) enum Value {
    Bool(bool),
    Int(i32),
}

/// Compiled bytecode ready to be run by a [`VM`].
///
/// ```
/// let chunk = oryn::Chunk::compile("let x = 5\nprint(x)").unwrap();
/// let mut vm = oryn::VM::new();
///
/// vm.run(&chunk).unwrap();
/// ```
#[derive(Debug)]
pub struct Chunk {
    pub(crate) instructions: Vec<Instruction>,
}

impl Chunk {
    /// Compiles source code into a [`Chunk`].
    ///
    /// ```
    /// let chunk = oryn::Chunk::compile("let x = 1 + 2").unwrap();
    /// ```
    ///
    /// Returns lex/parse errors if the source is invalid:
    ///
    /// ```
    /// let err = oryn::Chunk::compile("let = @").unwrap_err();
    ///
    /// assert!(!err.is_empty());
    /// ```
    pub fn compile(source: &str) -> Result<Self, Vec<OrynError>> {
        let (tokens, lex_errors) = lexer::lex(source);
        let (statements, parse_errors) = parser::parse(tokens);

        let errors: Vec<_> = lex_errors.into_iter().chain(parse_errors).collect();
        if !errors.is_empty() {
            return Err(errors);
        }

        let instructions = compiler::compile(statements);
        Ok(Self { instructions })
    }

    /// Returns all lex and parse errors without compiling. An empty
    /// vec means the source is valid.
    ///
    /// ```
    /// assert!(oryn::Chunk::check("let x = 5").is_empty());
    /// assert!(!oryn::Chunk::check("let = @").is_empty());
    /// ```
    pub fn check(source: &str) -> Vec<OrynError> {
        let (tokens, lex_errors) = lexer::lex(source);
        let (_, parse_errors) = parser::parse(tokens);
        lex_errors.into_iter().chain(parse_errors).collect()
    }
}

/// Stack-based virtual machine that executes compiled [`Chunk`]s.
///
/// ```
/// let chunk = oryn::Chunk::compile("print(1 + 2)").unwrap();
/// let mut vm = oryn::VM::new();
///
/// vm.run(&chunk).unwrap();
/// ```
pub struct VM {
    ip: usize,
}

impl VM {
    /// Creates a new VM.
    pub fn new() -> Self {
        Self { ip: 0 }
    }

    /// Runs a compiled [`Chunk`]. Can be called multiple times with
    /// different chunks on the same VM.
    ///
    /// ```
    /// let greet = oryn::Chunk::compile("print(1)").unwrap();
    /// let add = oryn::Chunk::compile("print(2 + 3)").unwrap();
    ///
    /// let mut vm = oryn::VM::new();
    ///
    /// vm.run(&greet).unwrap();
    /// vm.run(&add).unwrap();
    /// ```
    pub fn run(&mut self, chunk: &Chunk) -> Result<(), RuntimeError> {
        self.run_with_writer(chunk, &mut std::io::stdout())
    }

    pub fn run_with_writer(
        &mut self,
        chunk: &Chunk,
        writer: &mut impl Write,
    ) -> Result<(), RuntimeError> {
        let mut stack: Vec<Value> = Vec::new();
        let mut variables: HashMap<String, Value> = HashMap::new();

        self.ip = 0;

        while self.ip < chunk.instructions.len() {
            match &chunk.instructions[self.ip] {
                Instruction::PushBool(b) => {
                    stack.push(Value::Bool(*b));
                }
                Instruction::PushInt(n) => {
                    stack.push(Value::Int(*n));
                }
                Instruction::LoadVar(name) => {
                    let value = variables
                        .get(name)
                        .ok_or_else(|| RuntimeError::UndefinedVariable(name.clone()))?;

                    stack.push(value.clone());
                }
                Instruction::StoreVar(name) => {
                    let value = stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                    variables.insert(name.clone(), value);
                }
                Instruction::Equal => {
                    let right = stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                    stack.push(Value::Bool(left == right));
                }
                Instruction::NotEqual => {
                    let right = stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                    stack.push(Value::Bool(left != right));
                }
                Instruction::LessThan => {
                    let right = stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                    stack.push(Value::Bool(left < right));
                }
                Instruction::GreaterThan => {
                    let right = stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                    stack.push(Value::Bool(left > right));
                }
                Instruction::LessThanEquals => {
                    let right = stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                    stack.push(Value::Bool(left <= right));
                }
                Instruction::GreaterThanEquals => {
                    let right = stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                    stack.push(Value::Bool(left >= right));
                }
                Instruction::And => {
                    let Value::Bool(right) = stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    let Value::Bool(left) = stack.pop().ok_or(RuntimeError::StackUnderflow)? else {
                        return Err(RuntimeError::StackUnderflow);
                    };

                    stack.push(Value::Bool(left && right));
                }
                Instruction::Or => {
                    let Value::Bool(right) = stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    let Value::Bool(left) = stack.pop().ok_or(RuntimeError::StackUnderflow)? else {
                        return Err(RuntimeError::StackUnderflow);
                    };

                    stack.push(Value::Bool(left || right));
                }
                Instruction::Not => {
                    let value = stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                    stack.push(Value::Bool(!matches!(value, Value::Bool(true))));
                }

                Instruction::Add => {
                    let Value::Int(right) = stack.pop().ok_or(RuntimeError::StackUnderflow)? else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    let Value::Int(left) = stack.pop().ok_or(RuntimeError::StackUnderflow)? else {
                        return Err(RuntimeError::StackUnderflow);
                    };

                    stack.push(Value::Int(left + right));
                }
                Instruction::Sub => {
                    let Value::Int(right) = stack.pop().ok_or(RuntimeError::StackUnderflow)? else {
                        return Err(RuntimeError::StackUnderflow);
                    };

                    let Value::Int(left) = stack.pop().ok_or(RuntimeError::StackUnderflow)? else {
                        return Err(RuntimeError::StackUnderflow);
                    };

                    stack.push(Value::Int(left - right));
                }
                Instruction::Mul => {
                    let Value::Int(right) = stack.pop().ok_or(RuntimeError::StackUnderflow)? else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    let Value::Int(left) = stack.pop().ok_or(RuntimeError::StackUnderflow)? else {
                        return Err(RuntimeError::StackUnderflow);
                    };

                    stack.push(Value::Int(left * right));
                }
                Instruction::Div => {
                    let Value::Int(right) = stack.pop().ok_or(RuntimeError::StackUnderflow)? else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    let Value::Int(left) = stack.pop().ok_or(RuntimeError::StackUnderflow)? else {
                        return Err(RuntimeError::StackUnderflow);
                    };

                    stack.push(Value::Int(left / right));
                }
                Instruction::Call(name, arity) => {
                    // `split_off` grabs the last `arity` values — exactly
                    // the args that were pushed left-to-right by the compiler.
                    let args: Vec<Value> = stack.split_off(stack.len() - arity);

                    // Builtins are handled inline for now. Every call pushes
                    // a return value so the caller can use it or `Pop` it.
                    match name.as_str() {
                        "print" => {
                            let output: Vec<String> = args
                                .iter()
                                .map(|a| match a {
                                    Value::Int(n) => n.to_string(),
                                    Value::Bool(b) => b.to_string(),
                                })
                                .collect();

                            let output_str = output.join(", ");
                            writer
                                .write_all(output_str.as_bytes())
                                .map_err(RuntimeError::IoError)?;
                            writer.write_all(b"\n").map_err(RuntimeError::IoError)?;

                            stack.push(Value::Int(0));
                        }
                        _ => return Err(RuntimeError::UndefinedFunction(name.clone())),
                    }
                }
                Instruction::Pop => {
                    stack.pop();
                }
            }

            self.ip += 1;
        }

        Ok(())
    }
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executes_instructions_on_stack() {
        let chunk = Chunk {
            instructions: vec![
                Instruction::PushInt(10),
                Instruction::PushInt(3),
                Instruction::Add,
                Instruction::StoreVar("x".into()),
                Instruction::LoadVar("x".into()),
                Instruction::Pop,
            ],
        };

        let mut vm = VM::new();
        vm.run(&chunk).unwrap();
    }

    #[test]
    fn undefined_variable_is_runtime_error() {
        let chunk = Chunk {
            instructions: vec![Instruction::LoadVar("nope".into()), Instruction::Pop],
        };

        let mut vm = VM::new();
        let err = vm.run(&chunk).unwrap_err();

        assert!(matches!(err, RuntimeError::UndefinedVariable(name) if name == "nope"));
    }

    #[test]
    fn undefined_function_is_runtime_error() {
        let chunk = Chunk {
            instructions: vec![
                Instruction::PushInt(1),
                Instruction::Call("nope".into(), 1),
                Instruction::Pop,
            ],
        };

        let mut vm = VM::new();
        let err = vm.run(&chunk).unwrap_err();

        assert!(matches!(err, RuntimeError::UndefinedFunction(name) if name == "nope"));
    }
}
