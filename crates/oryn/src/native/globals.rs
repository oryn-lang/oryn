//! Global free-functions: `print`, `len`, `to_string`, `parse_int`,
//! `assert`. These have `receiver: None` so the registry's
//! `lookup_global` resolves them by bare name from a call site.

use std::io::Write;

use gc_arena::Gc;

use crate::compiler::types::ResolvedType;
use crate::errors::{RuntimeError, ValueType};
use crate::vm::chunk::Chunk;
use crate::vm::value::Value;

use super::{NativeFn, NativeRegistry, NativeSignature, pop_any, pop_n};

/// Format a value for `print` and direct `to_string` calls. The
/// top-level form leaves strings unquoted (so `print("hi")` produces
/// `hi`), but nested string fields inside enums and lists are
/// quoted via [`format_value_nested`] for clarity. This mirrors the
/// existing pre-registry behaviour exactly so user output stays
/// stable across the refactor.
pub(crate) fn format_value(value: &Value<'_>, chunk: &Chunk) -> String {
    match value {
        Value::String(s) => s.as_str().to_string(),
        _ => format_value_nested(value, chunk),
    }
}

/// Recursive helper used inside compound types (enum payloads, list
/// elements, map values). String values are wrapped in quotes here
/// so the output is unambiguous; primitives keep their bare form.
pub(crate) fn format_value_nested(value: &Value<'_>, chunk: &Chunk) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => {
            let s = f.to_string();
            if s.contains('.') { s } else { format!("{s}.0") }
        }
        Value::String(s) => format!("\"{}\"", s.as_str()),
        Value::Range(r) => {
            let r = r.borrow();
            let op = if r.inclusive { "..=" } else { ".." };
            format!("{}{}{}", r.current, op, r.end)
        }
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
                    s.push_str(&format_value_nested(field_value, chunk));
                }
                s.push_str(" }");
                s
            }
        }
        Value::List(list_ref) => {
            let data = list_ref.borrow();
            let mut s = String::from("[");
            for (i, v) in data.elements.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&format_value_nested(v, chunk));
            }
            s.push(']');
            s
        }
        Value::Map(map_ref) => {
            let data = map_ref.borrow();
            let mut s = String::from("{");
            for (i, (k, v)) in data.entries.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                use crate::vm::value::MapKey;
                let key_str = match k {
                    MapKey::String(s) => format!("\"{}\"", s),
                    MapKey::Int(i) => i.to_string(),
                    MapKey::Bool(b) => b.to_string(),
                };
                s.push_str(&key_str);
                s.push_str(": ");
                s.push_str(&format_value_nested(v, chunk));
            }
            s.push('}');
            s
        }
        Value::Function(idx) => {
            let name = chunk
                .functions
                .get(*idx)
                .map(|f| f.name.as_str())
                .unwrap_or("?");
            format!("<fn {name}>")
        }
        Value::Closure(c) => {
            let name = chunk
                .functions
                .get(c.fn_idx)
                .map(|f| f.name.as_str())
                .unwrap_or("?");
            format!("<closure {name}>")
        }
        Value::Uninitialized => "<uninitialized>".to_string(),
    }
}

pub(crate) fn register(r: &mut NativeRegistry) {
    // -----------------------------------------------------------------
    // print(...) -> nil
    //
    // Variadic — accepts any arity. The signature closure ignores
    // the formal `params` field; the compiler special-cases `print`
    // by reading the call's actual arity from the call site rather
    // than from the registered signature.
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "print",
        receiver: None,
        signature: |_, _| {
            Ok(NativeSignature {
                // Empty params list — the compiler treats `print` as
                // variadic and skips the per-arg type-check.
                params: vec![],
                return_type: ResolvedType::Nil,
            })
        },
        body: print_body,
    });

    // -----------------------------------------------------------------
    // len(string | [T] | {K:V}) -> int
    //
    // Polymorphic length: dispatches at runtime on the value's
    // shape. Kept around for compatibility with the existing
    // `len(xs)` calls in tests and examples; the per-receiver
    // `.len()` methods are the preferred form going forward.
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "len",
        receiver: None,
        signature: |_, args| {
            // Single arg, returns int. The arg type is constrained at
            // compile time only by the runtime body's type check —
            // accept anything here and let the body raise a clean
            // type error if the value isn't measurable.
            if args.len() != 1 {
                return Err(format!("len() takes 1 argument, got {}", args.len()));
            }
            Ok(NativeSignature {
                params: vec![args[0].clone()],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let value = pop_any(state)?;
            let len = match &value {
                Value::String(s) => s.len() as i32,
                Value::List(l) => l.borrow().elements.len() as i32,
                Value::Map(m) => m.borrow().entries.len() as i32,
                Value::Range(r) => {
                    let r = r.borrow();
                    let raw = r.end - r.current + if r.inclusive { 1 } else { 0 };
                    raw.max(0)
                }
                other => {
                    return Err(RuntimeError::TypeError {
                        expected: ValueType::String,
                        actual: ValueType::from(other),
                        span: Some(span.clone()),
                    });
                }
            };
            state.stack.push(Value::Int(len));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // to_string(T) -> string
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "to_string",
        receiver: None,
        signature: |_, args| {
            if args.len() != 1 {
                return Err(format!("to_string() takes 1 argument, got {}", args.len()));
            }
            Ok(NativeSignature {
                params: vec![args[0].clone()],
                return_type: ResolvedType::Str,
            })
        },
        body: |state, mc, chunk, _writer, _span| {
            let value = pop_any(state)?;
            let s = format_value(&value, chunk);
            state.stack.push(Value::String(Gc::new(mc, s)));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // parse_int(string) -> maybe int
    //
    // Wrapper that delegates to the same parsing logic as the
    // `string.parse_int()` method. Kept as a free function for
    // call sites that prefer the function-style form.
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "parse_int",
        receiver: None,
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Str],
                return_type: ResolvedType::Nillable(Box::new(ResolvedType::Int)),
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let value = pop_any(state)?;
            let s = match &value {
                Value::String(s) => s.as_str(),
                other => {
                    return Err(RuntimeError::TypeError {
                        expected: ValueType::String,
                        actual: ValueType::from(other),
                        span: Some(span.clone()),
                    });
                }
            };
            match s.trim().parse::<i32>() {
                Ok(n) => state.stack.push(Value::Int(n)),
                Err(_) => state.stack.push(Value::Nil),
            }
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // assert(bool) -> nil
    //
    // The test harness's primary affordance. Note: the existing
    // `Instruction::Assert` continues to handle the language-level
    // `assert(...)` statement form; this global is here so users
    // can call `assert` indirectly (e.g. as a callback).
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "assert",
        receiver: None,
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Bool],
                return_type: ResolvedType::Nil,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let value = pop_any(state)?;
            match value {
                Value::Bool(true) => {
                    state.stack.push(Value::Int(0));
                    Ok(())
                }
                Value::Bool(false) => Err(RuntimeError::AssertionFailed {
                    span: Some(span.clone()),
                }),
                other => Err(RuntimeError::TypeError {
                    expected: ValueType::Bool,
                    actual: ValueType::from(&other),
                    span: Some(span.clone()),
                }),
            }
        },
    });
}

/// `print` body: pop the args (the compiler tracks the actual arity
/// at the call site and the registry's CallNative carries it as the
/// second operand), format each, write joined by ", " and a newline.
fn print_body<'gc>(
    state: &mut crate::vm::value::VmState<'gc>,
    _mc: &'gc gc_arena::Mutation<'gc>,
    chunk: &Chunk,
    writer: &mut dyn Write,
    _span: &std::ops::Range<usize>,
) -> Result<(), RuntimeError> {
    // Read arity from a stashed slot. We can't get the call site's
    // arity from the body alone, so the dispatcher in vm/exec.rs sets
    // a thread-local-like field on `VmState` before invoking us.
    let arity = state.last_native_arity as usize;
    let args = pop_n(state, arity)?;
    let parts: Vec<String> = args.iter().map(|v| format_value(v, chunk)).collect();
    let joined = parts.join(", ");
    writer
        .write_all(joined.as_bytes())
        .map_err(RuntimeError::IoError)?;
    writer.write_all(b"\n").map_err(RuntimeError::IoError)?;
    state.stack.push(Value::Int(0));
    Ok(())
}
