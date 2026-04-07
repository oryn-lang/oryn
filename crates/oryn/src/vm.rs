use std::io::Write;
use std::ops::Range;

use crate::compiler;
use crate::compiler::{CompiledFunction, Instruction};
use crate::errors::{OrynError, RuntimeError, ValueType};
use crate::lexer;
use crate::parser;

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub(crate) enum Value {
    Bool(bool),
    Int(i32),
}

/// A call frame on the VM's call stack. Each function invocation
/// (including top-level code) gets its own frame with an instruction
/// pointer and a fixed-size array of local variable slots.
struct CallFrame {
    // None = top-level, Some(i) = functions[i].
    function_idx: Option<usize>,
    ip: usize,
    // Local variables indexed by slot number. Slot indices are
    // assigned at compile time so access is O(1) with no hashing.
    locals: Vec<Value>,
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
    pub(crate) spans: Vec<Range<usize>>,
    pub(crate) functions: Vec<CompiledFunction>,
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

        let output = compiler::compile(statements);

        Ok(Self {
            instructions: output.instructions,
            spans: output.spans,
            functions: output.functions,
        })
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

    /// Returns a human-readable disassembly of the compiled bytecode.
    ///
    /// ```
    /// let chunk = oryn::Chunk::compile("let x = 5\nprint(x)").unwrap();
    /// let output = chunk.disassemble();
    ///
    /// assert!(output.contains("SetLocal"));
    /// assert!(output.contains("CallBuiltin"));
    /// ```
    pub fn disassemble(&self) -> String {
        use std::fmt::Write;

        let mut out = String::new();

        writeln!(out, "== <main> ==").unwrap();
        disassemble_instructions(&mut out, &self.instructions);

        for func in &self.functions {
            let params = func.params.join(", ");
            writeln!(out, "\n== {}({}) ==", func.name, params).unwrap();
            disassemble_instructions(&mut out, &func.instructions);
        }

        out
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
    stack: Vec<Value>,
    frames: Vec<CallFrame>,
}

impl VM {
    /// Creates a new VM.
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            frames: Vec::new(),
        }
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
        self.stack.clear();
        self.frames.clear();

        // Top-level code runs in the first frame with a growable
        // locals vec (we don't track num_locals for top-level yet).
        self.frames.push(CallFrame {
            function_idx: None,
            ip: 0,
            locals: Vec::new(),
        });

        while let Some(frame) = self.frames.last() {
            let (instructions, _spans): (&[Instruction], &[Range<usize>]) = match frame.function_idx
            {
                None => (&chunk.instructions, &chunk.spans),
                Some(idx) => (
                    &chunk.functions[idx].instructions,
                    &chunk.functions[idx].spans,
                ),
            };

            if frame.ip >= instructions.len() {
                if frame.function_idx.is_none() {
                    break;
                }
                self.stack.push(Value::Int(0));
                self.frames.pop();
                continue;
            }

            let ip = frame.ip;
            let instruction = &instructions[ip];

            match instruction {
                Instruction::PushBool(b) => {
                    self.stack.push(Value::Bool(*b));
                }
                Instruction::PushInt(n) => {
                    self.stack.push(Value::Int(*n));
                }
                Instruction::GetLocal(slot) => {
                    let frame = self.frames.last().unwrap();
                    let value = frame.locals[*slot].clone();
                    self.stack.push(value);
                }
                Instruction::SetLocal(slot) => {
                    let value = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let frame = self.frames.last_mut().unwrap();

                    // Grow the locals vec if needed (top-level code
                    // doesn't pre-allocate).
                    if *slot >= frame.locals.len() {
                        frame.locals.resize(*slot + 1, Value::Int(0));
                    }

                    frame.locals[*slot] = value;
                }
                Instruction::Return => {
                    self.frames.pop();
                    continue;
                }
                Instruction::Equal => {
                    let right = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    self.stack.push(Value::Bool(left == right));
                }
                Instruction::NotEqual => {
                    let right = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    self.stack.push(Value::Bool(left != right));
                }
                Instruction::LessThan => {
                    let right = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    self.stack.push(Value::Bool(left < right));
                }
                Instruction::GreaterThan => {
                    let right = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    self.stack.push(Value::Bool(left > right));
                }
                Instruction::LessThanEquals => {
                    let right = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    self.stack.push(Value::Bool(left <= right));
                }
                Instruction::GreaterThanEquals => {
                    let right = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    let left = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    self.stack.push(Value::Bool(left >= right));
                }
                Instruction::And => {
                    let Value::Bool(right) =
                        self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    let Value::Bool(left) = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    self.stack.push(Value::Bool(left && right));
                }
                Instruction::Or => {
                    let Value::Bool(right) =
                        self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    let Value::Bool(left) = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    self.stack.push(Value::Bool(left || right));
                }
                Instruction::Not => {
                    let value = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                    self.stack
                        .push(Value::Bool(!matches!(value, Value::Bool(true))));
                }
                Instruction::Add => {
                    let Value::Int(right) = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    let Value::Int(left) = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    self.stack.push(Value::Int(left + right));
                }
                Instruction::Sub => {
                    let Value::Int(right) = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    let Value::Int(left) = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    self.stack.push(Value::Int(left - right));
                }
                Instruction::Mul => {
                    let Value::Int(right) = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    let Value::Int(left) = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    self.stack.push(Value::Int(left * right));
                }
                Instruction::Div => {
                    let Value::Int(right) = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    let Value::Int(left) = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                    else {
                        return Err(RuntimeError::StackUnderflow);
                    };
                    self.stack.push(Value::Int(left / right));
                }
                Instruction::CallBuiltin(name, arity) => {
                    let arity = *arity;
                    match name.as_str() {
                        "print" => {
                            let args: Vec<Value> = self.stack.split_off(self.stack.len() - arity);

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

                            self.stack.push(Value::Int(0));
                        }
                        _ => {
                            return Err(RuntimeError::UndefinedFunction {
                                name: name.clone(),
                                span: self.current_span(chunk),
                            });
                        }
                    }
                }
                Instruction::Call(func_idx, arity) => {
                    let func_idx = *func_idx;
                    let arity = *arity;

                    let func = &chunk.functions[func_idx];
                    if arity != func.arity {
                        return Err(RuntimeError::ArityMismatch {
                            name: func.name.clone(),
                            expected: func.arity,
                            actual: arity,
                            span: self.current_span(chunk),
                        });
                    }

                    // Advance caller's ip past the Call before pushing
                    // the new frame.
                    self.frames.last_mut().unwrap().ip += 1;

                    // Pre-allocate the locals vec to the right size.
                    // The function's first instructions will SetLocal
                    // the params into their slots.
                    self.frames.push(CallFrame {
                        function_idx: Some(func_idx),
                        ip: 0,
                        locals: vec![Value::Int(0); func.num_locals],
                    });

                    continue;
                }
                Instruction::Pop => {
                    self.stack.pop();
                }
                Instruction::JumpIfFalse(target) => {
                    let condition_value = self.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                    let Value::Bool(condition) = condition_value else {
                        return Err(RuntimeError::TypeError {
                            expected: ValueType::Bool,
                            actual: ValueType::from(&condition_value),
                            span: self.current_span(chunk),
                        });
                    };

                    if !condition {
                        self.frames.last_mut().unwrap().ip = *target;
                        continue;
                    }
                }
                Instruction::Jump(target) => {
                    self.frames.last_mut().unwrap().ip = *target;
                    continue;
                }
            }

            self.frames.last_mut().unwrap().ip += 1;
        }

        Ok(())
    }

    fn current_span(&self, chunk: &Chunk) -> Option<Range<usize>> {
        let frame = self.frames.last()?;

        let spans = match frame.function_idx {
            None => &chunk.spans,
            Some(idx) => &chunk.functions[idx].spans,
        };

        spans.get(frame.ip).cloned()
    }
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

fn disassemble_instructions(out: &mut String, instructions: &[Instruction]) {
    use std::fmt::Write;

    for (i, instr) in instructions.iter().enumerate() {
        let formatted = match instr {
            Instruction::PushBool(b) => format!("PushBool {b}"),
            Instruction::PushInt(n) => format!("PushInt {n}"),
            Instruction::GetLocal(slot) => format!("GetLocal {slot}"),
            Instruction::SetLocal(slot) => format!("SetLocal {slot}"),
            Instruction::Return => "Return".to_string(),
            Instruction::Equal => "Equal".to_string(),
            Instruction::NotEqual => "NotEqual".to_string(),
            Instruction::LessThan => "LessThan".to_string(),
            Instruction::GreaterThan => "GreaterThan".to_string(),
            Instruction::LessThanEquals => "LessThanEquals".to_string(),
            Instruction::GreaterThanEquals => "GreaterThanEquals".to_string(),
            Instruction::And => "And".to_string(),
            Instruction::Or => "Or".to_string(),
            Instruction::Not => "Not".to_string(),
            Instruction::Add => "Add".to_string(),
            Instruction::Sub => "Sub".to_string(),
            Instruction::Mul => "Mul".to_string(),
            Instruction::Div => "Div".to_string(),
            Instruction::Call(idx, arity) => {
                let s = if *arity == 1 { "arg" } else { "args" };
                format!("Call fn#{idx} ({arity} {s})")
            }
            Instruction::CallBuiltin(name, arity) => {
                let s = if *arity == 1 { "arg" } else { "args" };
                format!("CallBuiltin \"{name}\" ({arity} {s})")
            }
            Instruction::Pop => "Pop".to_string(),
            Instruction::JumpIfFalse(target) => format!("JumpIfFalse -> {target:04}"),
            Instruction::Jump(target) => format!("Jump -> {target:04}"),
        };

        writeln!(out, "{i:04}  {formatted}").unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(instructions: Vec<Instruction>) -> Chunk {
        let len = instructions.len();
        Chunk {
            instructions,
            spans: vec![0..0; len],
            functions: vec![],
        }
    }

    #[test]
    fn executes_instructions_on_stack() {
        let c = chunk(vec![
            Instruction::PushInt(10),
            Instruction::PushInt(3),
            Instruction::Add,
            Instruction::SetLocal(0),
            Instruction::GetLocal(0),
            Instruction::Pop,
        ]);

        let mut vm = VM::new();
        vm.run(&c).unwrap();
    }

    #[test]
    fn undefined_function_is_runtime_error() {
        let c = chunk(vec![
            Instruction::PushInt(1),
            Instruction::CallBuiltin("nope".into(), 1),
            Instruction::Pop,
        ]);

        let mut vm = VM::new();
        let err = vm.run(&c).unwrap_err();

        assert!(matches!(
            err,
            RuntimeError::UndefinedFunction { ref name, .. } if name == "nope"
        ));
    }
}
