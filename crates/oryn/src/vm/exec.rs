use std::io::Write;
use std::ops::Range;

use gc_arena::{Arena, Gc, Rootable};

use crate::compiler::Instruction;
use crate::errors::{RuntimeError, ValueType};

use super::chunk::Chunk;
use super::value::{CallFrame, Value, VmState};

/// Stack-based virtual machine that executes compiled [`Chunk`]s.
///
/// ```
/// let chunk = oryn::Chunk::compile("print(1 + 2)").unwrap();
/// let mut vm = oryn::VM::new();
///
/// vm.run(&chunk).unwrap();
/// ```
pub struct VM {
    arena: Arena<Rootable![VmState<'_>]>,
}

impl VM {
    /// Creates a new VM.
    pub fn new() -> Self {
        Self {
            arena: Arena::new(|_| VmState {
                stack: Vec::new(),
                frames: Vec::new(),
            }),
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
        self.arena.mutate_root(|mc, state| {
            state.stack.clear();
            state.frames.clear();

            state.frames.push(CallFrame {
                function_idx: None,
                ip: 0,
                locals: Vec::new(),
            });

            while let Some(frame) = state.frames.last() {
                let (instructions, _spans): (&[Instruction], &[Range<usize>]) =
                    match frame.function_idx {
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
                    state.stack.push(Value::Int(0));
                    state.frames.pop();
                    continue;
                }

                let ip = frame.ip;
                let instruction = &instructions[ip];

                match instruction {
                    Instruction::PushBool(b) => {
                        state.stack.push(Value::Bool(*b));
                    }
                    Instruction::PushInt(n) => {
                        state.stack.push(Value::Int(*n));
                    }
                    Instruction::PushString(s) => {
                        state.stack.push(Value::String(Gc::new(mc, s.clone())));
                    }
                    Instruction::GetLocal(slot) => {
                        let frame = state.frames.last().unwrap();
                        let value = frame.locals[*slot].clone();

                        state.stack.push(value);
                    }
                    Instruction::SetLocal(slot) => {
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let frame = state.frames.last_mut().unwrap();

                        if *slot >= frame.locals.len() {
                            frame.locals.resize(*slot + 1, Value::Int(0));
                        }

                        frame.locals[*slot] = value;
                    }
                    Instruction::Return => {
                        state.frames.pop();
                        continue;
                    }
                    Instruction::Equal => {
                        let right = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let left = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        state.stack.push(Value::Bool(left == right));
                    }
                    Instruction::NotEqual => {
                        let right = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let left = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        state.stack.push(Value::Bool(left != right));
                    }
                    Instruction::LessThan => {
                        let right = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let left = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        state.stack.push(Value::Bool(left < right));
                    }
                    Instruction::GreaterThan => {
                        let right = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let left = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        state.stack.push(Value::Bool(left > right));
                    }
                    Instruction::LessThanEquals => {
                        let right = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let left = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        state.stack.push(Value::Bool(left <= right));
                    }
                    Instruction::GreaterThanEquals => {
                        let right = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let left = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        state.stack.push(Value::Bool(left >= right));
                    }
                    Instruction::And => {
                        let Value::Bool(right) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };
                        let Value::Bool(left) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };

                        state.stack.push(Value::Bool(left && right));
                    }
                    Instruction::Or => {
                        let Value::Bool(right) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };
                        let Value::Bool(left) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };

                        state.stack.push(Value::Bool(left || right));
                    }
                    Instruction::Not => {
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        state
                            .stack
                            .push(Value::Bool(!matches!(value, Value::Bool(true))));
                    }
                    Instruction::Add => {
                        let Value::Int(right) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };
                        let Value::Int(left) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };

                        state.stack.push(Value::Int(left + right));
                    }
                    Instruction::Sub => {
                        let Value::Int(right) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };
                        let Value::Int(left) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };

                        state.stack.push(Value::Int(left - right));
                    }
                    Instruction::Mul => {
                        let Value::Int(right) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };
                        let Value::Int(left) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };

                        state.stack.push(Value::Int(left * right));
                    }
                    Instruction::Div => {
                        let Value::Int(right) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };
                        let Value::Int(left) =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?
                        else {
                            return Err(RuntimeError::StackUnderflow);
                        };

                        state.stack.push(Value::Int(left / right));
                    }
                    Instruction::CallBuiltin(name, arity) => {
                        let arity = *arity;

                        match name.as_str() {
                            "print" => {
                                let args: Vec<Value> =
                                    state.stack.split_off(state.stack.len() - arity);

                                let output: Vec<String> = args
                                    .iter()
                                    .map(|a| match a {
                                        Value::Int(n) => n.to_string(),
                                        Value::Bool(b) => b.to_string(),
                                        Value::String(s) => s.as_str().to_string(),
                                    })
                                    .collect();

                                let output_str = output.join(", ");
                                writer
                                    .write_all(output_str.as_bytes())
                                    .map_err(RuntimeError::IoError)?;
                                writer.write_all(b"\n").map_err(RuntimeError::IoError)?;

                                state.stack.push(Value::Int(0));
                            }
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);

                                return Err(RuntimeError::UndefinedFunction {
                                    name: name.clone(),
                                    span,
                                });
                            }
                        }
                    }
                    Instruction::Call(func_idx, arity) => {
                        let func_idx = *func_idx;
                        let arity = *arity;

                        let func = &chunk.functions[func_idx];
                        if arity != func.arity {
                            let span = Self::current_span_from_state(&state.frames, chunk);

                            return Err(RuntimeError::ArityMismatch {
                                name: func.name.clone(),
                                expected: func.arity,
                                actual: arity,
                                span,
                            });
                        }

                        state.frames.last_mut().unwrap().ip += 1;

                        state.frames.push(CallFrame {
                            function_idx: Some(func_idx),
                            ip: 0,
                            locals: vec![Value::Int(0); func.num_locals],
                        });

                        continue;
                    }
                    Instruction::Pop => {
                        state.stack.pop();
                    }
                    Instruction::JumpIfFalse(target) => {
                        let condition_value =
                            state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        let Value::Bool(condition) = condition_value else {
                            let span = Self::current_span_from_state(&state.frames, chunk);

                            return Err(RuntimeError::TypeError {
                                expected: ValueType::Bool,
                                actual: ValueType::from(&condition_value),
                                span,
                            });
                        };

                        if !condition {
                            state.frames.last_mut().unwrap().ip = *target;
                            continue;
                        }
                    }
                    Instruction::Jump(target) => {
                        state.frames.last_mut().unwrap().ip = *target;
                        continue;
                    }
                }

                state.frames.last_mut().unwrap().ip += 1;
            }

            Ok(())
        })
    }

    fn current_span_from_state(
        frames: &[CallFrame],
        chunk: &Chunk,
    ) -> Option<std::ops::Range<usize>> {
        let frame = frames.last()?;

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
