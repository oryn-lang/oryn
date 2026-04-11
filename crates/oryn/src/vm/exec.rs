use std::io::Write;
use std::ops::Range;

use gc_arena::lock::RefLock;
use gc_arena::{Arena, Gc, Rootable};

use crate::compiler::{BuiltinFunction, Instruction, ListMethod};
use crate::errors::{RuntimeError, ValueType};
use crate::vm::value::{EnumData, ListData, MapData, MapKey, ObjData, RangeValue};

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
            // Enum equality: same def_idx, same variant_idx, structural
            // payload equality. Identity (Gc pointer) is NOT used — two
            // independently constructed `FsResult.NotFound` values are
            // equal. Cross-enum comparisons (different def_idx) are
            // false rather than a type error so users can write
            // `result == FsResult.NotFound` against any enum-typed
            // expression without the compiler tracking which enum it is.
            (Value::Enum(l), Value::Enum(r)) => {
                let l_data = l.borrow();
                let r_data = r.borrow();
                let eq = l_data.def_idx == r_data.def_idx
                    && l_data.variant_idx == r_data.variant_idx
                    && l_data.payload == r_data.payload;
                $state.stack.push(Value::Bool(eq $op true));
            }
            // Nil comparisons: nil == nil is true, nil == non-nil is false.
            // For != the result is naturally inverted since the macro passes !=.
            (Value::Nil, Value::Nil) => {
                // For ==: true == true → true.  For !=: true != true → false.
                $state.stack.push(Value::Bool(true $op true));
            }
            (Value::Nil, _) | (_, Value::Nil) => {
                // For ==: true == false → false.  For !=: true != false → true.
                $state.stack.push(Value::Bool(true $op false));
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
                // Keep modest upfront capacity so the common case does not
                // immediately reallocate on first use, while still relying on
                // normal Vec growth for larger programs.
                stack: Vec::with_capacity(256),
                locals: Vec::with_capacity(128),
                frames: Vec::with_capacity(64),
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
        self.run_from_entry(chunk, writer, None)
    }

    /// Invoke a specific compiled function by absolute index with no
    /// arguments. The VM is reset before execution so no state from a
    /// prior run leaks into this call. Used by the `oryn test` runner
    /// to invoke zero-arity test bodies in isolation.
    ///
    /// Returns `Err(RuntimeError::ArityMismatch)` if the target function
    /// expects arguments, or `Err(RuntimeError::UndefinedFunction)` if
    /// `function_idx` is out of bounds.
    pub fn run_function(&mut self, chunk: &Chunk, function_idx: usize) -> Result<(), RuntimeError> {
        self.run_function_with_writer(chunk, function_idx, &mut std::io::stdout())
    }

    /// Same as [`VM::run_function`] but writes any script-side output
    /// (e.g. `print` calls inside the test body) to `writer` instead of
    /// stdout. Useful for capturing output in tests or redirecting to a
    /// log file from the host.
    pub fn run_function_with_writer(
        &mut self,
        chunk: &Chunk,
        function_idx: usize,
        writer: &mut impl Write,
    ) -> Result<(), RuntimeError> {
        let func =
            chunk
                .functions
                .get(function_idx)
                .ok_or_else(|| RuntimeError::UndefinedFunction {
                    name: format!("<function #{function_idx}>"),
                    span: None,
                })?;

        if func.arity != 0 {
            return Err(RuntimeError::ArityMismatch {
                name: func.name.clone(),
                expected: func.arity,
                actual: 0,
                span: None,
            });
        }

        self.run_from_entry(chunk, writer, Some(function_idx))
    }

    fn run_from_entry(
        &mut self,
        chunk: &Chunk,
        writer: &mut impl Write,
        entry_function_idx: Option<usize>,
    ) -> Result<(), RuntimeError> {
        self.arena.mutate_root(|mc, state| {
            // A VM instance is reusable across runs, so reset the live
            // execution state but keep the underlying allocations.
            state.stack.clear();
            state.locals.clear();
            state.frames.clear();

            // The initial frame is either the top-level instruction
            // stream (`entry_function_idx = None`) or a specific
            // compiled function's body. Both drive the exact same
            // interpreter loop below; `Return` pops the frame and the
            // loop terminates naturally when `frames` becomes empty.
            state.frames.push(CallFrame {
                function_idx: entry_function_idx,
                ip: 0,
                local_base: 0,
            });

            while !state.frames.is_empty() {
                // Snapshot the current frame metadata once per iteration.
                // Most instructions only need the instruction pointer,
                // current function body, and the frame's locals window base.
                let frame_idx = state.frames.len() - 1;
                let frame = &state.frames[frame_idx];
                let function_idx = frame.function_idx;
                let local_base = frame.local_base;
                let ip = frame.ip;
                let (instructions, _spans): (&[Instruction], &[Range<usize>]) =
                    Self::code_for(function_idx, chunk);

                if ip >= instructions.len() {
                    if function_idx.is_none() {
                        break;
                    }
                    state.stack.push(Value::Int(0));
                    if let Some(frame) = state.frames.pop() {
                        state.locals.truncate(frame.local_base);
                    }
                    continue;
                }

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
                    Instruction::PushNil => {
                        state.stack.push(Value::Nil);
                    }
                    Instruction::JumpIfNil(target) => {
                        if state.stack.is_empty() {
                            return Err(RuntimeError::StackUnderflow);
                        }
                        if state.stack.last() == Some(&Value::Nil) {
                            state.stack.pop();
                            state.frames.last_mut().unwrap().ip = *target;
                            continue;
                        }
                    }
                    Instruction::JumpIfError(target) => {
                        let top = state.stack.last().ok_or(RuntimeError::StackUnderflow)?;
                        if Self::value_is_error_enum(top, chunk) {
                            state.frames.last_mut().unwrap().ip = *target;
                            continue;
                        }
                    }
                    Instruction::UnwrapErrorOrTrap => {
                        let top = state.stack.last().ok_or(RuntimeError::StackUnderflow)?;
                        if Self::value_is_error_enum(top, chunk) {
                            let message = Self::format_value(top, chunk);
                            return Err(RuntimeError::ErrorUnwrapTrap {
                                message,
                                span: Self::current_span_from_state(&state.frames, chunk),
                            });
                        }
                    }
                    Instruction::ToString => {
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        let s = match &value {
                            Value::Bool(b) => b.to_string(),
                            Value::Float(f) => {
                                let s = f.to_string();
                                if s.contains('.') { s } else { format!("{s}.0") }
                            }
                            Value::Int(i) => i.to_string(),
                            Value::String(s) => s.as_str().to_string(),
                            Value::Object(obj_ref) => {
                                let data = obj_ref.borrow();
                                let type_name = &chunk.obj_defs[data.type_idx].name;
                                format!("<{type_name} instance>")
                            }
                            Value::Enum(enum_ref) => {
                                // Format as the source-equivalent
                                // constructor: nullary variants
                                // produce `EnumName.VariantName`,
                                // payload variants produce
                                // `EnumName.VariantName { field: value, ... }`.
                                // Payload values are formatted via
                                // `format_value` (see below) which
                                // recurses through compound types.
                                let data = enum_ref.borrow();
                                let def = &chunk.enum_defs[data.def_idx];
                                let variant = &def.variants[data.variant_idx];
                                if variant.field_names.is_empty() {
                                    format!("{}.{}", def.name, variant.name)
                                } else {
                                    let mut s = format!("{}.{} {{ ", def.name, variant.name);
                                    for (i, (field_name, field_value)) in variant
                                        .field_names
                                        .iter()
                                        .zip(data.payload.iter())
                                        .enumerate()
                                    {
                                        if i > 0 {
                                            s.push_str(", ");
                                        }
                                        s.push_str(field_name);
                                        s.push_str(": ");
                                        s.push_str(&Self::format_value(field_value, chunk));
                                    }
                                    s.push_str(" }");
                                    s
                                }
                            }
                            Value::Range(range_ref) => {
                                let range = range_ref.borrow();
                                let op = if range.inclusive { "..=" } else { ".." };
                                format!("{}{}{}", range.current, op, range.end)
                            }
                            Value::List(list_ref) => {
                                let data = list_ref.borrow();
                                format!("<list of {}>", data.elements.len())
                            }
                            Value::Map(map_ref) => {
                                let data = map_ref.borrow();
                                format!("<map of {}>", data.entries.len())
                            }
                            Value::Nil => "nil".to_string(),
                            Value::Uninitialized => {
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::String,
                                    actual: ValueType::from(&value),
                                    span: Self::current_span_from_state(&state.frames, chunk),
                                });
                            }
                        };

                        state.stack.push(Value::String(Gc::new(mc, s)));
                    }
                    Instruction::Concat(n) => {
                        let n = *n as usize;
                        if n == 0 {
                            state.stack.push(Value::String(Gc::new(mc, String::new())));
                        } else {
                            // Parts are on the stack in order: first-pushed is deepest.
                            // Split off the top N values to preserve ordering.
                            let start = state.stack.len().saturating_sub(n);
                            let parts = state.stack.split_off(start);

                            let total_len: usize = parts
                                .iter()
                                .map(|v| match v {
                                    Value::String(s) => s.len(),
                                    _ => 0,
                                })
                                .sum();
                            let mut result = String::with_capacity(total_len);

                            for value in &parts {
                                match value {
                                    Value::String(s) => result.push_str(s),
                                    _ => {
                                        return Err(RuntimeError::TypeError {
                                            expected: ValueType::String,
                                            actual: ValueType::from(value),
                                            span: Self::current_span_from_state(
                                                &state.frames,
                                                chunk,
                                            ),
                                        });
                                    }
                                }
                            }

                            state.stack.push(Value::String(Gc::new(mc, result)));
                        }
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
                        // Locals live in a single VM-wide stack. Each frame
                        // points at its window with local_base, and compiler
                        // slots are offsets within that window.
                        let value = state.locals[local_base + *slot].clone();

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
                        let local_idx = local_base + *slot;

                        // Top-level code grows the shared locals stack lazily.
                        // Function frames pre-reserve their entire locals
                        // window at call time, so this branch is effectively
                        // for script-level locals only.
                        if local_idx >= state.locals.len() {
                            state.locals.resize(local_idx + 1, Value::Uninitialized);
                        }

                        state.locals[local_idx] = value;
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
                    Instruction::GetField(field_name) => {
                        let obj = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        match obj {
                            Value::Object(obj_ref) => {
                                let data = obj_ref.borrow();
                                let obj_def = &chunk.obj_defs[data.type_idx];
                                let field_idx = obj_def
                                    .fields
                                    .iter()
                                    .position(|field| field == field_name)
                                    .ok_or_else(|| RuntimeError::UndefinedVariable {
                                        name: format!("{}.{}", obj_def.name, field_name),
                                        span: Self::current_span_from_state(&state.frames, chunk),
                                    })?;
                                let value = data.fields[field_idx].clone();

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
                    Instruction::SetField(field_name) => {
                        // Stack order: object was pushed first, then value.
                        // Pop in reverse: value first, then object.
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let obj = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        match obj {
                            Value::Object(obj_ref) => {
                                // borrow_mut requires the GC mutation context
                                // to maintain gc-arena's write barrier invariant.
                                let field_idx = {
                                    let data = obj_ref.borrow();
                                    let obj_def = &chunk.obj_defs[data.type_idx];
                                    obj_def
                                        .fields
                                        .iter()
                                        .position(|field| field == field_name)
                                        .ok_or_else(|| RuntimeError::UndefinedVariable {
                                            name: format!("{}.{}", obj_def.name, field_name),
                                            span: Self::current_span_from_state(
                                                &state.frames,
                                                chunk,
                                            ),
                                        })?
                                };
                                obj_ref.borrow_mut(mc).fields[field_idx] = value;
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
                        if let Some(frame) = state.frames.pop() {
                            // Discard the returning frame's locals window.
                            // Nested callers keep their locals because their
                            // slots live below frame.local_base.
                            state.locals.truncate(frame.local_base);
                        }

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
                    Instruction::CallBuiltin(builtin, arity) => {
                        let arity = *arity;

                        match builtin {
                            BuiltinFunction::Print => {
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
                                        Value::Enum(enum_ref) => {
                                            // Source-equivalent format:
                                            // `EnumName.VariantName` for
                                            // nullary, plus `{ field: value, ... }`
                                            // for payload variants. The
                                            // print path uses the same
                                            // shape as `format_value`
                                            // because enum values are
                                            // their own readable form.
                                            let data = enum_ref.borrow();
                                            let def = &chunk.enum_defs[data.def_idx];
                                            let variant = &def.variants[data.variant_idx];
                                            if variant.field_names.is_empty() {
                                                format!("{}.{}", def.name, variant.name)
                                            } else {
                                                let mut s =
                                                    format!("{}.{} {{ ", def.name, variant.name);
                                                for (i, (field_name, field_value)) in variant
                                                    .field_names
                                                    .iter()
                                                    .zip(data.payload.iter())
                                                    .enumerate()
                                                {
                                                    if i > 0 {
                                                        s.push_str(", ");
                                                    }
                                                    s.push_str(field_name);
                                                    s.push_str(": ");
                                                    s.push_str(&Self::format_value(
                                                        field_value,
                                                        chunk,
                                                    ));
                                                }
                                                s.push_str(" }");
                                                s
                                            }
                                        }
                                        Value::Range(range_ref) => {
                                            let range = range_ref.borrow();
                                            let op = if range.inclusive { "..=" } else { ".." };

                                            format!("{}{}{}", range.current, op, range.end)
                                        }
                                        Value::List(list_ref) => {
                                            let data = list_ref.borrow();
                                            format!("<list of {}>", data.elements.len())
                                        }
                                        Value::Map(map_ref) => {
                                            let data = map_ref.borrow();
                                            format!("<map of {}>", data.entries.len())
                                        }
                                        Value::String(s) => s.as_str().to_string(),
                                        Value::Nil => "nil".to_string(),
                                    })
                                    .collect();

                                let output_str = output.join(", ");
                                writer
                                    .write_all(output_str.as_bytes())
                                    .map_err(RuntimeError::IoError)?;
                                writer.write_all(b"\n").map_err(RuntimeError::IoError)?;

                                state.stack.push(Value::Int(0));
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
                        state.frames[frame_idx].ip += 1;

                        let local_base = state.locals.len();
                        // Reserve the callee's entire locals window up front.
                        // Function prologues populate parameter slots with
                        // SetLocal, and later locals reuse compiler-assigned
                        // slots inside this fixed-size region.
                        state
                            .locals
                            .resize(local_base + func.num_locals, Value::Uninitialized);
                        state.frames.push(CallFrame {
                            function_idx: Some(func_idx),
                            ip: 0,
                            local_base,
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
                        state.frames[frame_idx].ip += 1;

                        let local_base = state.locals.len();
                        // Methods use the same locals layout as functions;
                        // `self` is just parameter slot 0 in the callee's
                        // pre-reserved locals window.
                        state
                            .locals
                            .resize(local_base + func.num_locals, Value::Uninitialized);
                        state.frames.push(CallFrame {
                            function_idx: Some(func_idx),
                            ip: 0,
                            local_base,
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
                            state.frames[frame_idx].ip = *target;
                            continue;
                        }
                    }
                    Instruction::Jump(target) => {
                        state.frames[frame_idx].ip = *target;
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
                    Instruction::MakeList(n) => {
                        let n = *n as usize;
                        // The compiler pushed element values in source
                        // order (first element is deepest on the stack).
                        // split_off preserves that order when we move
                        // them into the backing Vec.
                        let elements: Vec<Value> = state.stack.split_off(state.stack.len() - n);
                        let list = ListData { elements };
                        state
                            .stack
                            .push(Value::List(Gc::new(mc, RefLock::new(list))));
                    }
                    Instruction::ListGet => {
                        let index = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let list = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        let index = match index {
                            Value::Int(i) => i,
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Int,
                                    actual: ValueType::from(&index),
                                    span,
                                });
                            }
                        };

                        let list_ref = match list {
                            Value::List(l) => l,
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::List,
                                    actual: ValueType::from(&list),
                                    span,
                                });
                            }
                        };

                        let data = list_ref.borrow();
                        let len = data.elements.len();
                        if index < 0 || (index as usize) >= len {
                            let span = Self::current_span_from_state(&state.frames, chunk);
                            return Err(RuntimeError::IndexOutOfBounds { index, len, span });
                        }
                        let value = data.elements[index as usize].clone();
                        state.stack.push(value);
                    }
                    Instruction::ListSet => {
                        // Stack order (compiled by stmt.rs): object, index, value.
                        // Pop in reverse: value, index, object.
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let index = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let list = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        let index = match index {
                            Value::Int(i) => i,
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Int,
                                    actual: ValueType::from(&index),
                                    span,
                                });
                            }
                        };

                        let list_ref = match list {
                            Value::List(l) => l,
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::List,
                                    actual: ValueType::from(&list),
                                    span,
                                });
                            }
                        };

                        let len = list_ref.borrow().elements.len();
                        if index < 0 || (index as usize) >= len {
                            let span = Self::current_span_from_state(&state.frames, chunk);
                            return Err(RuntimeError::IndexOutOfBounds { index, len, span });
                        }
                        list_ref.borrow_mut(mc).elements[index as usize] = value;
                    }
                    Instruction::MakeMap(n) => {
                        let n = *n as usize;
                        let mut entries = Vec::with_capacity(n);
                        let values = state.stack.split_off(state.stack.len() - (n * 2));
                        let mut values = values.into_iter();

                        while let Some(key_value) = values.next() {
                            let value = values.next().ok_or(RuntimeError::StackUnderflow)?;
                            let key = match Self::map_key_from_value(&key_value) {
                                Some(key) => key,
                                None => {
                                    let span = Self::current_span_from_state(&state.frames, chunk);
                                    return Err(RuntimeError::TypeError {
                                        expected: ValueType::MapKey,
                                        actual: ValueType::from(&key_value),
                                        span,
                                    });
                                }
                            };

                            if let Some((_, existing)) =
                                entries.iter_mut().find(|(existing, _)| existing == &key)
                            {
                                *existing = value;
                            } else {
                                entries.push((key, value));
                            }
                        }

                        let map = MapData { entries };
                        state.stack.push(Value::Map(Gc::new(mc, RefLock::new(map))));
                    }
                    Instruction::MapGet => {
                        let key_value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let map = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        let key = match Self::map_key_from_value(&key_value) {
                            Some(key) => key,
                            None => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::MapKey,
                                    actual: ValueType::from(&key_value),
                                    span,
                                });
                            }
                        };

                        match map {
                            Value::Map(map_ref) => {
                                let data = map_ref.borrow();
                                let value = data
                                    .entries
                                    .iter()
                                    .find(|(existing, _)| existing == &key)
                                    .map(|(_, value)| value.clone())
                                    .unwrap_or(Value::Nil);
                                state.stack.push(value);
                            }
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Map,
                                    actual: ValueType::from(&map),
                                    span,
                                });
                            }
                        }
                    }
                    Instruction::MapSet => {
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let key_value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        let map = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        let key = match Self::map_key_from_value(&key_value) {
                            Some(key) => key,
                            None => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::MapKey,
                                    actual: ValueType::from(&key_value),
                                    span,
                                });
                            }
                        };

                        match map {
                            Value::Map(map_ref) => {
                                let mut data = map_ref.borrow_mut(mc);
                                if let Some((_, existing)) = data
                                    .entries
                                    .iter_mut()
                                    .find(|(existing, _)| existing == &key)
                                {
                                    *existing = value;
                                } else {
                                    data.entries.push((key, value));
                                }
                            }
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Map,
                                    actual: ValueType::from(&map),
                                    span,
                                });
                            }
                        }
                    }
                    Instruction::CallListMethod(id, _arity) => {
                        // Decode the method id. An unknown id is a
                        // compiler bug — the compiler only ever emits
                        // ids that round-trip through `ListMethod::from_id`.
                        let method = ListMethod::from_id(*id).ok_or_else(|| {
                            RuntimeError::UndefinedFunction {
                                name: format!("<list method #{id}>"),
                                span: Self::current_span_from_state(&state.frames, chunk),
                            }
                        })?;

                        match method {
                            ListMethod::Len => {
                                let list = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                                match list {
                                    Value::List(list_ref) => {
                                        let len = list_ref.borrow().elements.len();
                                        state.stack.push(Value::Int(len as i32));
                                    }
                                    _ => {
                                        let span =
                                            Self::current_span_from_state(&state.frames, chunk);
                                        return Err(RuntimeError::TypeError {
                                            expected: ValueType::List,
                                            actual: ValueType::from(&list),
                                            span,
                                        });
                                    }
                                }
                            }
                            ListMethod::Push => {
                                // Stack order: list, value. Pop value first.
                                let value =
                                    state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                                let list = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                                match list {
                                    Value::List(list_ref) => {
                                        list_ref.borrow_mut(mc).elements.push(value);
                                        // Sentinel return — keeps stack
                                        // discipline uniform across all
                                        // method calls.
                                        state.stack.push(Value::Int(0));
                                    }
                                    _ => {
                                        let span =
                                            Self::current_span_from_state(&state.frames, chunk);
                                        return Err(RuntimeError::TypeError {
                                            expected: ValueType::List,
                                            actual: ValueType::from(&list),
                                            span,
                                        });
                                    }
                                }
                            }
                            ListMethod::Pop => {
                                let list = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                                match list {
                                    Value::List(list_ref) => {
                                        let popped = list_ref.borrow_mut(mc).elements.pop();
                                        state.stack.push(popped.unwrap_or(Value::Nil));
                                    }
                                    _ => {
                                        let span =
                                            Self::current_span_from_state(&state.frames, chunk);
                                        return Err(RuntimeError::TypeError {
                                            expected: ValueType::List,
                                            actual: ValueType::from(&list),
                                            span,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Instruction::Assert => {
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;

                        match value {
                            Value::Bool(true) => {
                                // Assertions leave nothing on the stack;
                                // `Statement::Assert` compiles as a
                                // statement, not an expression, so there
                                // is no value to carry forward.
                            }
                            Value::Bool(false) => {
                                let span = Self::current_span_from_state(&state.frames, chunk);

                                return Err(RuntimeError::AssertionFailed { span });
                            }
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
                    Instruction::MakeEnum(def_idx, variant_idx, payload_count) => {
                        // The compiler pushed payload values in
                        // declaration order. split_off pops them as a
                        // contiguous slice so payload indices line up
                        // with the EnumVariantInfo.
                        let payload_count = *payload_count;
                        let payload: Vec<Value> =
                            state.stack.split_off(state.stack.len() - payload_count);
                        let data = EnumData {
                            def_idx: *def_idx,
                            variant_idx: *variant_idx,
                            payload,
                        };
                        state
                            .stack
                            .push(Value::Enum(Gc::new(mc, RefLock::new(data))));
                    }
                    Instruction::EnumDiscriminant => {
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        match value {
                            Value::Enum(data_ref) => {
                                let variant_idx = data_ref.borrow().variant_idx;
                                state.stack.push(Value::Int(variant_idx as i32));
                            }
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Enum,
                                    actual: ValueType::from(&value),
                                    span,
                                });
                            }
                        }
                    }
                    Instruction::GetEnumPayload(field_idx) => {
                        // Pop an enum value, push payload[field_idx].
                        // Used by match codegen to extract bound
                        // payload fields into local slots inside an
                        // arm body. The compiler resolves the field
                        // index from the variant's declared field
                        // names, so an out-of-range index is a
                        // compiler bug, not user error.
                        let field_idx = *field_idx;
                        let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
                        match value {
                            Value::Enum(data_ref) => {
                                let data = data_ref.borrow();
                                let payload_value =
                                    data.payload.get(field_idx).cloned().unwrap_or(Value::Nil);
                                state.stack.push(payload_value);
                            }
                            _ => {
                                let span = Self::current_span_from_state(&state.frames, chunk);
                                return Err(RuntimeError::TypeError {
                                    expected: ValueType::Enum,
                                    actual: ValueType::from(&value),
                                    span,
                                });
                            }
                        }
                    }
                }

                state.frames[frame_idx].ip += 1;
            }

            Ok(())
        })
    }

    fn code_for(function_idx: Option<usize>, chunk: &Chunk) -> (&[Instruction], &[Range<usize>]) {
        match function_idx {
            None => (&chunk.instructions, &chunk.spans),
            Some(idx) => (
                &chunk.functions[idx].instructions,
                &chunk.functions[idx].spans,
            ),
        }
    }

    fn map_key_from_value(value: &Value<'_>) -> Option<MapKey> {
        match value {
            Value::String(s) => Some(MapKey::String(s.as_str().to_string())),
            Value::Int(i) => Some(MapKey::Int(*i)),
            Value::Bool(b) => Some(MapKey::Bool(*b)),
            _ => None,
        }
    }

    /// Returns `true` when `value` is a `Value::Enum` whose
    /// declaration was marked with the `error` modifier
    /// (`error enum Foo { ... }`). Used by `JumpIfError` and
    /// `UnwrapErrorOrTrap` to recognize error-side values in an
    /// `error T` union without a dedicated wrapper Value variant.
    fn value_is_error_enum(value: &Value<'_>, chunk: &Chunk) -> bool {
        if let Value::Enum(enum_ref) = value {
            let data = enum_ref.borrow();
            chunk
                .enum_defs
                .get(data.def_idx)
                .map(|def| def.is_error)
                .unwrap_or(false)
        } else {
            false
        }
    }

    /// Format a value for display in print output and string
    /// interpolation. Recursive — compound values format their
    /// children using this same function. The output is intended to
    /// be readable; for enums and primitives it is also
    /// source-equivalent (could be pasted back into a program).
    fn format_value(value: &Value<'_>, chunk: &Chunk) -> String {
        match value {
            Value::Bool(b) => b.to_string(),
            Value::Float(f) => {
                let s = f.to_string();
                if s.contains('.') { s } else { format!("{s}.0") }
            }
            Value::Int(i) => i.to_string(),
            Value::String(s) => format!("\"{}\"", s.as_str()),
            Value::Nil => "nil".to_string(),
            Value::Object(obj_ref) => {
                let data = obj_ref.borrow();
                let type_name = &chunk.obj_defs[data.type_idx].name;
                format!("<{type_name} instance>")
            }
            Value::Enum(enum_ref) => {
                let data = enum_ref.borrow();
                let def = &chunk.enum_defs[data.def_idx];
                let variant = &def.variants[data.variant_idx];
                if variant.field_names.is_empty() {
                    format!("{}.{}", def.name, variant.name)
                } else {
                    let mut s = format!("{}.{} {{ ", def.name, variant.name);
                    for (i, (field_name, field_value)) in variant
                        .field_names
                        .iter()
                        .zip(data.payload.iter())
                        .enumerate()
                    {
                        if i > 0 {
                            s.push_str(", ");
                        }
                        s.push_str(field_name);
                        s.push_str(": ");
                        s.push_str(&Self::format_value(field_value, chunk));
                    }
                    s.push_str(" }");
                    s
                }
            }
            Value::Range(range_ref) => {
                let r = range_ref.borrow();
                let op = if r.inclusive { "..=" } else { ".." };
                format!("{}{}{}", r.current, op, r.end)
            }
            Value::List(list_ref) => {
                let data = list_ref.borrow();
                format!("<list of {}>", data.elements.len())
            }
            Value::Map(map_ref) => {
                let data = map_ref.borrow();
                format!("<map of {}>", data.entries.len())
            }
            Value::Uninitialized => "<uninitialized>".to_string(),
        }
    }

    fn current_span_from_state(
        frames: &[CallFrame],
        chunk: &Chunk,
    ) -> Option<std::ops::Range<usize>> {
        let frame = frames.last()?;
        let (_instructions, spans) = Self::code_for(frame.function_idx, chunk);

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
            enum_defs: vec![],
            tests: vec![],
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
    fn builtin_print_executes() {
        let c = chunk(vec![
            Instruction::PushInt(1),
            Instruction::CallBuiltin(BuiltinFunction::Print, 1),
            Instruction::Pop,
        ]);

        let mut vm = VM::new();
        let mut output = Vec::new();
        vm.run_with_writer(&c, &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "1\n");
    }

    #[test]
    fn assert_true_runs_to_completion() {
        let c = crate::Chunk::compile("assert(true)").unwrap();
        let mut vm = VM::new();
        vm.run(&c).unwrap();
    }

    #[test]
    fn assert_false_raises_assertion_failed() {
        let c = crate::Chunk::compile("assert(false)").unwrap();
        let mut vm = VM::new();
        let err = vm.run(&c).expect_err("assert(false) should trap");
        assert!(matches!(
            err,
            RuntimeError::AssertionFailed { span: Some(_) }
        ));
    }

    #[test]
    fn run_function_invokes_test_body_in_isolation() {
        // Two tests in the same chunk: one passes, one fails. The runner
        // calls run_function for each; the order-independent result
        // confirms the loop tears down state cleanly between invocations.
        let c = crate::Chunk::compile(
            "test \"ok\" { assert(1 + 1 == 2) }\ntest \"bad\" { assert(1 == 2) }",
        )
        .unwrap();

        assert_eq!(c.tests().len(), 2);

        let passing = c.tests()[0].function_idx;
        let failing = c.tests()[1].function_idx;

        let mut vm = VM::new();
        vm.run_function(&c, passing).unwrap();

        let err = vm.run_function(&c, failing).expect_err("test should fail");
        assert!(matches!(err, RuntimeError::AssertionFailed { .. }));
    }

    #[test]
    fn run_function_rejects_arity_mismatch() {
        // Compile a one-arg function and ensure run_function refuses
        // to invoke it without arguments.
        let c = crate::Chunk::compile("fn id(x: int) -> int { return x }").unwrap();
        let mut vm = VM::new();
        let err = vm
            .run_function(&c, 0)
            .expect_err("run_function should reject non-zero-arity functions");
        assert!(matches!(
            err,
            RuntimeError::ArityMismatch {
                expected: 1,
                actual: 0,
                ..
            }
        ));
    }

    // -------------------------------------------------------------------
    // List execution tests
    // -------------------------------------------------------------------

    fn run_source(source: &str) -> Vec<u8> {
        let c = crate::Chunk::compile(source).unwrap_or_else(|errors| {
            panic!("compile failed: {errors:?}");
        });
        let mut vm = VM::new();
        let mut output = Vec::new();
        vm.run_with_writer(&c, &mut output).unwrap();
        output
    }

    #[test]
    fn list_literal_and_index_round_trip() {
        let out =
            run_source("let xs: [int] = [10, 20, 30]\nprint(xs[0])\nprint(xs[1])\nprint(xs[2])");
        assert_eq!(String::from_utf8(out).unwrap(), "10\n20\n30\n");
    }

    #[test]
    fn list_len_returns_element_count() {
        let out = run_source("let xs: [int] = [1, 2, 3, 4]\nprint(xs.len())");
        assert_eq!(String::from_utf8(out).unwrap(), "4\n");
    }

    #[test]
    fn list_push_mutates_in_place() {
        let out = run_source(
            "let xs: [int] = [1]\nxs.push(2)\nxs.push(3)\nprint(xs.len())\nprint(xs[2])",
        );
        assert_eq!(String::from_utf8(out).unwrap(), "3\n3\n");
    }

    #[test]
    fn list_pop_returns_nillable_element() {
        let out = run_source(
            "let xs: [int] = [1, 2, 3]\nif let last = xs.pop() { print(last) }\nprint(xs.len())",
        );
        assert_eq!(String::from_utf8(out).unwrap(), "3\n2\n");
    }

    #[test]
    fn list_pop_on_empty_returns_nil() {
        // Pop on empty list yields nil — confirmed via an orelse fallback.
        let out = run_source(
            "let xs: [int] = [1]\nlet _ = xs.pop()\nlet fallback = xs.pop() orelse 99\nprint(fallback)",
        );
        assert_eq!(String::from_utf8(out).unwrap(), "99\n");
    }

    #[test]
    fn list_index_assignment_mutates_in_place() {
        let out = run_source("let xs: [int] = [1, 2, 3]\nxs[1] = 99\nprint(xs[1])");
        assert_eq!(String::from_utf8(out).unwrap(), "99\n");
    }

    #[test]
    fn map_literal_and_index_round_trip() {
        let out = run_source(
            "let stats: {string: int} = {\"hp\": 10, \"mp\": 4}\nprint(stats[\"hp\"] orelse 0)\nprint(stats[\"mp\"] orelse 0)",
        );
        assert_eq!(String::from_utf8(out).unwrap(), "10\n4\n");
    }

    #[test]
    fn missing_map_key_returns_nil() {
        let out =
            run_source("let stats: {string: int} = {\"hp\": 10}\nprint(stats[\"xp\"] orelse 99)");
        assert_eq!(String::from_utf8(out).unwrap(), "99\n");
    }

    #[test]
    fn map_index_assignment_inserts_and_replaces() {
        let out = run_source(
            "let stats: {string: int} = {}\nstats[\"hp\"] = 10\nstats[\"hp\"] = 12\nprint(stats[\"hp\"] orelse 0)",
        );
        assert_eq!(String::from_utf8(out).unwrap(), "12\n");
    }

    #[test]
    fn list_get_out_of_bounds_raises_runtime_error() {
        let c = crate::Chunk::compile("let xs: [int] = [1, 2]\nlet y = xs[5]").unwrap();
        let mut vm = VM::new();
        let err = vm.run(&c).expect_err("expected out-of-bounds trap");
        assert!(matches!(
            err,
            RuntimeError::IndexOutOfBounds {
                index: 5,
                len: 2,
                ..
            }
        ));
    }

    #[test]
    fn list_set_out_of_bounds_raises_runtime_error() {
        let c = crate::Chunk::compile("let xs: [int] = [1, 2]\nxs[5] = 9").unwrap();
        let mut vm = VM::new();
        let err = vm.run(&c).expect_err("expected out-of-bounds trap");
        assert!(matches!(err, RuntimeError::IndexOutOfBounds { .. }));
    }

    #[test]
    fn nested_lists_round_trip() {
        let out =
            run_source("let xs: [[int]] = [[1, 2], [3, 4]]\nprint(xs[0][1])\nprint(xs[1][0])");
        assert_eq!(String::from_utf8(out).unwrap(), "2\n3\n");
    }

    #[test]
    fn list_of_obj_instances_stores_and_reads_fields() {
        let out = run_source(
            r#"struct Pt { x: int, y: int }
let ps: [Pt] = [Pt { x: 1, y: 2 }, Pt { x: 3, y: 4 }]
print(ps[0].x)
print(ps[1].y)"#,
        );
        assert_eq!(String::from_utf8(out).unwrap(), "1\n4\n");
    }

    #[test]
    fn list_of_strings_round_trip() {
        let out = run_source(
            r#"let names: [string] = ["alice", "bob"]
print(names[0])
print(names[1])"#,
        );
        assert_eq!(String::from_utf8(out).unwrap(), "alice\nbob\n");
    }

    // -------------------------------------------------------------------
    // For loops on lists
    // -------------------------------------------------------------------

    #[test]
    fn for_loop_over_int_list() {
        let out = run_source("let xs: [int] = [10, 20, 30]\nfor x in xs {\nprint(x)\n}");
        assert_eq!(String::from_utf8(out).unwrap(), "10\n20\n30\n");
    }

    #[test]
    fn for_loop_over_string_list() {
        let out =
            run_source("let names: [string] = [\"alice\", \"bob\"]\nfor n in names {\nprint(n)\n}");
        assert_eq!(String::from_utf8(out).unwrap(), "alice\nbob\n");
    }

    #[test]
    fn for_loop_over_empty_list_runs_zero_times() {
        let out = run_source("let xs: [int] = []\nprint(0)\nfor x in xs {\nprint(x)\n}\nprint(1)");
        assert_eq!(String::from_utf8(out).unwrap(), "0\n1\n");
    }

    #[test]
    fn for_loop_over_list_of_obj_instances_accesses_fields() {
        let out = run_source(
            "struct Pt { x: int, y: int }\nlet ps: [Pt] = [Pt { x: 1, y: 2 }, Pt { x: 3, y: 4 }]\nfor p in ps {\nprint(p.x)\nprint(p.y)\n}",
        );
        assert_eq!(String::from_utf8(out).unwrap(), "1\n2\n3\n4\n");
    }

    #[test]
    fn for_loop_over_list_supports_break() {
        let out = run_source(
            "let xs: [int] = [1, 2, 3, 4, 5]\nfor x in xs {\nif x == 3 { break }\nprint(x)\n}",
        );
        assert_eq!(String::from_utf8(out).unwrap(), "1\n2\n");
    }

    #[test]
    fn for_loop_over_list_supports_continue() {
        let out = run_source(
            "let xs: [int] = [1, 2, 3, 4]\nfor x in xs {\nif x == 2 { continue }\nprint(x)\n}",
        );
        assert_eq!(String::from_utf8(out).unwrap(), "1\n3\n4\n");
    }

    #[test]
    fn for_loop_over_list_still_allows_range_loops() {
        // Sanity: the type-dispatch in the For handler doesn't break
        // the existing range path.
        let out = run_source("for i in 0..3 {\nprint(i)\n}");
        assert_eq!(String::from_utf8(out).unwrap(), "0\n1\n2\n");
    }

    #[test]
    fn for_loop_over_nested_list_sums_elements() {
        let out = run_source(
            "let grid: [[int]] = [[1, 2], [3, 4]]\nfor row in grid {\nfor x in row {\nprint(x)\n}\n}",
        );
        assert_eq!(String::from_utf8(out).unwrap(), "1\n2\n3\n4\n");
    }
}
