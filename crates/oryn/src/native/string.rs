//! String methods. See [`super`] for the registration model and the
//! "How to add a native method" workflow.

use gc_arena::Gc;

use crate::compiler::types::ResolvedType;
use crate::vm::value::{ListData, Value};

use super::{NativeFn, NativeReceiver, NativeRegistry, NativeSignature, pop_int, pop_string};

pub(crate) fn register(r: &mut NativeRegistry) {
    // -----------------------------------------------------------------
    // len: () -> int  — UTF-8 byte length, matches Lua `#s`
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "len",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let s = pop_string(state, span)?;
            state.stack.push(Value::Int(s.len() as i32));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // to_upper: () -> string
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "to_upper",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Str,
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let s = pop_string(state, span)?;
            let upper = s.to_uppercase();
            state.stack.push(Value::String(Gc::new(mc, upper)));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // to_lower: () -> string
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "to_lower",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Str,
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let s = pop_string(state, span)?;
            let lower = s.to_lowercase();
            state.stack.push(Value::String(Gc::new(mc, lower)));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // trim: () -> string
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "trim",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Str,
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let s = pop_string(state, span)?;
            let trimmed = s.trim().to_string();
            state.stack.push(Value::String(Gc::new(mc, trimmed)));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // contains: (string) -> bool
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "contains",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Str],
                return_type: ResolvedType::Bool,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let needle = pop_string(state, span)?;
            let haystack = pop_string(state, span)?;
            state
                .stack
                .push(Value::Bool(haystack.contains(needle.as_str())));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // starts_with: (string) -> bool
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "starts_with",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Str],
                return_type: ResolvedType::Bool,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let prefix = pop_string(state, span)?;
            let s = pop_string(state, span)?;
            state
                .stack
                .push(Value::Bool(s.starts_with(prefix.as_str())));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // ends_with: (string) -> bool
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "ends_with",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Str],
                return_type: ResolvedType::Bool,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let suffix = pop_string(state, span)?;
            let s = pop_string(state, span)?;
            state.stack.push(Value::Bool(s.ends_with(suffix.as_str())));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // index_of: (string) -> maybe int
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "index_of",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Str],
                return_type: ResolvedType::Nillable(Box::new(ResolvedType::Int)),
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let needle = pop_string(state, span)?;
            let haystack = pop_string(state, span)?;
            match haystack.find(needle.as_str()) {
                Some(idx) => state.stack.push(Value::Int(idx as i32)),
                None => state.stack.push(Value::Nil),
            }
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // replace: (string, string) -> string  — replace all occurrences
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "replace",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Str, ResolvedType::Str],
                return_type: ResolvedType::Str,
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let with = pop_string(state, span)?;
            let from = pop_string(state, span)?;
            let s = pop_string(state, span)?;
            let result = s.replace(from.as_str(), with.as_str());
            state.stack.push(Value::String(Gc::new(mc, result)));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // split: (string) -> [string]
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "split",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Str],
                return_type: ResolvedType::List(Box::new(ResolvedType::Str)),
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let sep = pop_string(state, span)?;
            let s = pop_string(state, span)?;
            let parts: Vec<Value> = s
                .split(sep.as_str())
                .map(|p| Value::String(Gc::new(mc, p.to_string())))
                .collect();
            let list = ListData { elements: parts };
            state
                .stack
                .push(Value::List(Gc::new(mc, gc_arena::lock::RefLock::new(list))));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // repeat: (int) -> string
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "repeat",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Int],
                return_type: ResolvedType::Str,
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let count = pop_int(state, span)?;
            let s = pop_string(state, span)?;
            let result = if count <= 0 {
                String::new()
            } else {
                s.repeat(count as usize)
            };
            state.stack.push(Value::String(Gc::new(mc, result)));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // chars: () -> [string]  — one element per UTF-8 char
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "chars",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::List(Box::new(ResolvedType::Str)),
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let s = pop_string(state, span)?;
            let elements: Vec<Value> = s
                .chars()
                .map(|c| Value::String(Gc::new(mc, c.to_string())))
                .collect();
            let list = ListData { elements };
            state
                .stack
                .push(Value::List(Gc::new(mc, gc_arena::lock::RefLock::new(list))));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // parse_int: () -> maybe int
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "parse_int",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Nillable(Box::new(ResolvedType::Int)),
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let s = pop_string(state, span)?;
            match s.trim().parse::<i32>() {
                Ok(n) => state.stack.push(Value::Int(n)),
                Err(_) => state.stack.push(Value::Nil),
            }
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // parse_float: () -> maybe float
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "parse_float",
        receiver: Some(NativeReceiver::String),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Nillable(Box::new(ResolvedType::Float)),
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let s = pop_string(state, span)?;
            match s.trim().parse::<f32>() {
                Ok(n) => state.stack.push(Value::Float(n)),
                Err(_) => state.stack.push(Value::Nil),
            }
            Ok(())
        },
    });
}
