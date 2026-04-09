use std::io::Write;
use std::ops::Range;

use gc_arena::lock::RefLock;
use gc_arena::{Arena, Gc, Rootable};

use crate::compiler::Instruction;
use crate::errors::{RuntimeError, ValueType};
use crate::vm::value::{ObjData, RangeValue};

use super::chunk::Chunk;
use super::value::{CallFrame, Value, VmState};

/// Helper macro for binary arithmetic ops (Add, Sub, Mul).
/// Pops two values, applies checked integer arithmetic or float arithmetic,
/// and pushes the result. Reports TypeMismatch for incompatible types.
macro_rules! arithmetic_op {
    ($state:expr, $frames:expr, $chunk:expr, $op_str:expr, $checked_method:ident, $float_op:tt) => {{
        let right = $state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
        let left = $state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

        match (left, right) {
            (Value::Int(l), Value::Int(r)) => {
                let result = l.$checked_method(r).ok_or_else(|| {
                    RuntimeError::IntegerOverflow {
                        span: VM::current_span_from_state($frames, $chunk),
                    }
                })?;
                $state.stack.push(Value::Int(result));
            }
            (Value::Float(l), Value::Float(r)) => {
                $state.stack.push(Value::Float(l $float_op r));
            }
            (ref l, ref r) => {
                let span = VM::current_span_from_state($frames, $chunk);
                return Err(RuntimeError::TypeMismatch {
                    op: $op_str,
                    left: ValueType::from(l),
                    right: ValueType::from(r),
                    span,
                });
            }
        };
    }};
}

/// Helper macro for equality ops (Equal, NotEqual).
/// Both operands must be the same type.
macro_rules! equality_op {
    ($state:expr, $frames:expr, $chunk:expr, $op:tt) => {{
        let right = $state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
        let left = $state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
        match (&left, &right) {
            (Value::Int(l), Value::Int(r)) => {
                $state.stack.push(Value::Bool(*l $op *r));
            }
            (Value::Float(l), Value::Float(r)) => {
                $state.stack.push(Value::Bool(*l $op *r));
            }
            (Value::Bool(l), Value::Bool(r)) => {
                $state.stack.push(Value::Bool(*l $op *r));
            }
            (Value::String(l), Value::String(r)) => {
                $state.stack.push(Value::Bool(**l $op **r));
            }
            (l, r) => {
                let span = VM::current_span_from_state($frames, $chunk);
                return Err(RuntimeError::TypeMismatch {
                    op: stringify!($op),
                    left: ValueType::from(l),
                    right: ValueType::from(r),
                    span,
                });
            }
        }
    }};
}

/// Helper macro for ordering ops (LessThan, GreaterThan, etc.).
/// Only numeric and string types support ordering.
macro_rules! ordering_op {
    ($state:expr, $frames:expr, $chunk:expr, $op:tt) => {{
        let right = $state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
        let left = $state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
        match (&left, &right) {
            (Value::Int(l), Value::Int(r)) => {
                $state.stack.push(Value::Bool(*l $op *r));
            }
            (Value::Float(l), Value::Float(r)) => {
                $state.stack.push(Value::Bool(*l $op *r));
            }
            (Value::String(l), Value::String(r)) => {
                $state.stack.push(Value::Bool(**l $op **r));
            }
            (l, r) => {
                let span = VM::current_span_from_state($frames, $chunk);
                return Err(RuntimeError::TypeMismatch {
                    op: stringify!($op),
                    left: ValueType::from(l),
                    right: ValueType::from(r),
                    span,
                });
            }
        }
    }};
}

/// Helper macro for boolean logic ops (And, Or).
/// Pops two values, checks both are Bool, applies the operator.
/// Reports TypeError if either operand is not a Bool.
macro_rules! bool_binary_op {
    ($state:expr, $frames:expr, $chunk:expr, $op:tt) => {{
        let right = $state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
        let left = $state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

        match (left, right) {
            (Value::Bool(l), Value::Bool(r)) => {
                $state.stack.push(Value::Bool(l $op r));
            }
            (Value::Bool(_), ref other) | (ref other, Value::Bool(_)) => {
                let span = VM::current_span_from_state($frames, $chunk);
                return Err(RuntimeError::TypeError {
                    expected: ValueType::Bool,
                    actual: ValueType::from(other),
                    span,
                });
            }
            (ref left_val, _) => {
                let span = VM::current_span_from_state($frames, $chunk);
                return Err(RuntimeError::TypeError {
                    expected: ValueType::Bool,
                    actual: ValueType::from(left_val),
                    span,
                });
            }
        }
    }};
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
                    Instruction::PushFloat(n) => {
                        state.stack.push(Value::Float(*n));
                    }
                    Instruction::PushInt(n) => {
                        state.stack.push(Value::Int(*n));
                    }
                    Instruction::PushString(s) => {
                        state.stack.push(Value::String(Gc::new(mc, s.clone())));
                    }
                    Instruction::MakeRange(inclusive) => {
                        let end = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let start = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        match (start, end) {
                            (Value::Int(start), Value::Int(end)) => {
                                let range = RangeValue {
                                    current: start,
                                    end,
                                    inclusive: *inclusive,
                                };

                                state
                                    .stack
                                    .push(Value::Range(Gc::new(mc, RefLock::new(range))));
                            }
                            (ref left, ref right) => {
                                let span = Self::current_span_from_state(&state.frames, chunk);

                                return Err(RuntimeError::TypeMismatch {
                                    op: "..",
                                    left: ValueType::from(left),
                                    right: ValueType::from(right),
                                    span,
                                });
                            }
                        }
                    }
                    Instruction::GetLocal(slot) => {
                        // SAFETY: A main frame is always pushed before the
                        // execution loop, and frames are only popped on
                        // Return (which continues past ip advancement).
                        // The frame stack is never empty during dispatch.
                        let frame = state.frames.last().unwrap();
                        let value = frame.locals[*slot].clone();

                        if matches!(value, Value::Uninitialized) {
                            let span = Self::current_span_from_state(&state.frames, chunk);
                            return Err(RuntimeError::UndefinedVariable {
                                name: format!("local#{slot}"),
                                span,
                            });
                        }

                        state.stack.push(value);
                    }
                    Instruction::SetLocal(slot) => {
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        // SAFETY: Same invariant as GetLocal - frame stack
                        // is never empty during instruction dispatch.
                        let frame = state.frames.last_mut().unwrap();

                        if *slot >= frame.locals.len() {
                            frame.locals.resize(*slot + 1, Value::Uninitialized);
                        }

                        frame.locals[*slot] = value;
                    }
                    Instruction::NewObject(type_idx, num_fields) => {
                        // The compiler pushed field values in definition
                        // order. split_off pops them as a contiguous slice
                        // so field indices line up with the ObjDefInfo.
                        let num_fields = *num_fields;
                        let fields: Vec<Value> =
                            state.stack.split_off(state.stack.len() - num_fields);
                        let obj = ObjData {
                            type_idx: *type_idx,
                            fields,
                        };

                        state
                            .stack
                            .push(Value::Object(Gc::new(mc, RefLock::new(obj))));
                    }
                    Instruction::GetField(field_idx) => {
                        let obj = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        match obj {
                            Value::Object(obj_ref) => {
                                let data = obj_ref.borrow();
                                let value = data.fields[*field_idx].clone();

                                state.stack.push(value);
                            }
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);

                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Object,
                                    actual: ValueType::from(&obj),
                                    span,
                                });
                            }
                        }
                    }
                    Instruction::SetField(field_idx) => {
                        // Stack order: object was pushed first, then value.
                        // Pop in reverse: value first, then object.
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let obj = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        match obj {
                            Value::Object(obj_ref) => {
                                // borrow_mut requires the GC mutation context
                                // to maintain gc-arena's write barrier invariant.
                                obj_ref.borrow_mut(mc).fields[*field_idx] = value;
                            }
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);

                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Object,
                                    actual: ValueType::from(&obj),
                                    span,
                                });
                            }
                        }
                    }
                    Instruction::Return => {
                        state.frames.pop();

                        continue;
                    }
                    Instruction::Equal => {
                        equality_op!(state, &state.frames, chunk, ==);
                    }
                    Instruction::NotEqual => {
                        equality_op!(state, &state.frames, chunk, !=);
                    }
                    Instruction::LessThan => {
                        ordering_op!(state, &state.frames, chunk, <);
                    }
                    Instruction::GreaterThan => {
                        ordering_op!(state, &state.frames, chunk, >);
                    }
                    Instruction::LessThanEquals => {
                        ordering_op!(state, &state.frames, chunk, <=);
                    }
                    Instruction::GreaterThanEquals => {
                        ordering_op!(state, &state.frames, chunk, >=);
                    }
                    Instruction::And => {
                        bool_binary_op!(state, &state.frames, chunk, &&);
                    }
                    Instruction::Or => {
                        bool_binary_op!(state, &state.frames, chunk, ||);
                    }
                    Instruction::Not => {
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        match value {
                            Value::Bool(b) => state.stack.push(Value::Bool(!b)),
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Bool,
                                    actual: ValueType::from(&value),
                                    span,
                                });
                            }
                        }
                    }
                    Instruction::Negate => {
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        match value {
                            Value::Int(n) => {
                                let result = n.checked_neg().ok_or_else(|| {
                                    RuntimeError::IntegerOverflow {
                                        span: Self::current_span_from_state(&state.frames, chunk),
                                    }
                                })?;
                                state.stack.push(Value::Int(result));
                            }
                            Value::Float(n) => state.stack.push(Value::Float(-n)),
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Int,
                                    actual: ValueType::from(&value),
                                    span,
                                });
                            }
                        }
                    }
                    Instruction::Add => {
                        arithmetic_op!(state, &state.frames, chunk, "+", checked_add, +);
                    }
                    Instruction::Sub => {
                        arithmetic_op!(state, &state.frames, chunk, "-", checked_sub, -);
                    }
                    Instruction::Mul => {
                        arithmetic_op!(state, &state.frames, chunk, "*", checked_mul, *);
                    }
                    Instruction::Div => {
                        let right = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let left = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        match (left, right) {
                            (Value::Int(_), Value::Int(0)) => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::DivisionByZero { span });
                            }
                            (Value::Float(_), Value::Float(0.0)) => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::DivisionByZero { span });
                            }
                            (Value::Int(l), Value::Int(r)) => state.stack.push(Value::Int(l / r)),
                            (Value::Float(l), Value::Float(r)) => {
                                state.stack.push(Value::Float(l / r))
                            }
                            (ref l, ref r) => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeMismatch {
                                    op: "/",
                                    left: ValueType::from(l),
                                    right: ValueType::from(r),
                                    span,
                                });
                            }
                        };
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
                                        Value::Uninitialized => "<uninitialized>".to_string(),
                                        Value::Float(f) => {
                                            let s = f.to_string();

                                            if s.contains('.') { s } else { format!("{s}.0") }
                                        }
                                        Value::Int(n) => n.to_string(),
                                        Value::Bool(b) => b.to_string(),
                                        Value::Object(obj_ref) => {
                                            let data = obj_ref.borrow();
                                            let type_name = &chunk.obj_defs[data.type_idx].name;

                                            format!("<{type_name} instance>")
                                        }
                                        Value::Range(range_ref) => {
                                            let range = range_ref.borrow();
                                            let op = if range.inclusive { "..=" } else { ".." };

                                            format!("{}{}{}", range.current, op, range.end)
                                        }
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

                        // SAFETY: Same frame-stack invariant. We advance the
                        // caller's ip before pushing the callee's frame.
                        state.frames.last_mut().unwrap().ip += 1;

                        state.frames.push(CallFrame {
                            function_idx: Some(func_idx),
                            ip: 0,
                            locals: vec![Value::Uninitialized; func.num_locals],
                        });

                        continue;
                    }
                    Instruction::CallMethod(method_name, arity) => {
                        let arity = *arity;

                        // The object (self) is on the stack below the args.
                        // Peek at it to find its type.
                        let obj_pos = state.stack.len() - arity - 1;
                        let obj = &state.stack[obj_pos];

                        let type_idx = match obj {
                            Value::Object(obj_ref) => obj_ref.borrow().type_idx,
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Object,
                                    actual: ValueType::from(obj),
                                    span,
                                });
                            }
                        };

                        // Look up the method on the object's type definition.
                        let obj_def = &chunk.obj_defs[type_idx];
                        let func_idx = match obj_def.methods.get(method_name.as_str()) {
                            Some(idx) => *idx,
                            None => {
                                let span = Self::current_span_from_state(&state.frames, chunk);

                                return Err(RuntimeError::UndefinedFunction {
                                    name: format!("{}.{}", obj_def.name, method_name),
                                    span,
                                });
                            }
                        };

                        let func = &chunk.functions[func_idx];

                        // The caller passes `arity` args plus the object (self).
                        // The compiled method's arity includes self.
                        let total_args = arity + 1;
                        if total_args != func.arity {
                            let span = Self::current_span_from_state(&state.frames, chunk);
                            return Err(RuntimeError::ArityMismatch {
                                name: format!("{}.{}", obj_def.name, method_name),
                                expected: func.arity - 1, // exclude self in message
                                actual: arity,
                                span,
                            });
                        }

                        // SAFETY: Same frame-stack invariant.
                        state.frames.last_mut().unwrap().ip += 1;

                        state.frames.push(CallFrame {
                            function_idx: Some(func_idx),
                            ip: 0,
                            locals: vec![Value::Uninitialized; func.num_locals],
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
                            // SAFETY: Same frame-stack invariant.
                            state.frames.last_mut().unwrap().ip = *target;
                            continue;
                        }
                    }
                    Instruction::Jump(target) => {
                        // SAFETY: Same frame-stack invariant.
                        state.frames.last_mut().unwrap().ip = *target;
                        continue;
                    }
                    Instruction::RangeHasNext => {
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        match value {
                            Value::Range(range_ref) => {
                                let range = range_ref.borrow();

                                let has_next = if range.inclusive {
                                    range.current <= range.end
                                } else {
                                    range.current < range.end
                                };

                                state.stack.push(Value::Bool(has_next));
                            }
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);

                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Range,
                                    actual: ValueType::from(&value),
                                    span,
                                });
                            }
                        }
                    }
                    Instruction::RangeNext => {
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        match value {
                            Value::Range(range_ref) => {
                                let mut range = range_ref.borrow_mut(mc);
                                let next = range.current;

                                range.current = range.current.checked_add(1).ok_or_else(|| {
                                    RuntimeError::IntegerOverflow {
                                        span: Self::current_span_from_state(&state.frames, chunk),
                                    }
                                })?;

                                state.stack.push(Value::Int(next));
                            }
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);

                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Range,
                                    actual: ValueType::from(&value),
                                    span,
                                });
                            }
                        }
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
            obj_defs: vec![],
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
