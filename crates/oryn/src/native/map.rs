//! Map methods. See [`super`] for the registration model.

use gc_arena::Gc;
use gc_arena::lock::RefLock;

use crate::compiler::types::ResolvedType;
use crate::errors::{RuntimeError, ValueType};
use crate::vm::value::{ListData, MapKey, Value};

use super::{NativeFn, NativeReceiver, NativeRegistry, NativeSignature, pop_any, pop_map};

/// Pull `(key_type, value_type)` out of a `Map(K, V)` receiver.
fn key_value_of(receiver: &ResolvedType) -> (ResolvedType, ResolvedType) {
    match receiver {
        ResolvedType::Map(k, v) => ((**k).clone(), (**v).clone()),
        _ => (ResolvedType::Unknown, ResolvedType::Unknown),
    }
}

/// Convert a runtime [`Value`] into a [`MapKey`]. Mirrors the
/// `map_key_from_value` helper inside `vm/exec.rs` — kept here so
/// the registry doesn't need access to that crate-private fn. Returns
/// a clean type error for non-key types.
fn value_to_map_key(
    value: &Value<'_>,
    span: &std::ops::Range<usize>,
) -> Result<MapKey, RuntimeError> {
    match value {
        Value::String(s) => Ok(MapKey::String(s.as_str().to_string())),
        Value::Int(i) => Ok(MapKey::Int(*i)),
        Value::Bool(b) => Ok(MapKey::Bool(*b)),
        other => Err(RuntimeError::TypeError {
            expected: ValueType::MapKey,
            actual: ValueType::from(other),
            span: Some(span.clone()),
        }),
    }
}

/// Lift a [`MapKey`] back into a [`Value`] for the `keys()` method.
fn map_key_to_value<'gc>(key: &MapKey, mc: &'gc gc_arena::Mutation<'gc>) -> Value<'gc> {
    match key {
        MapKey::String(s) => Value::String(Gc::new(mc, s.clone())),
        MapKey::Int(i) => Value::Int(*i),
        MapKey::Bool(b) => Value::Bool(*b),
    }
}

pub(crate) fn register(r: &mut NativeRegistry) {
    // -----------------------------------------------------------------
    // len: () -> int
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "len",
        receiver: Some(NativeReceiver::Map),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let m = pop_map(state, span)?;
            state
                .stack
                .push(Value::Int(m.borrow().entries.len() as i32));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // is_empty: () -> bool
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "is_empty",
        receiver: Some(NativeReceiver::Map),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Bool,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let m = pop_map(state, span)?;
            state.stack.push(Value::Bool(m.borrow().entries.is_empty()));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // contains_key: (K) -> bool
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "contains_key",
        receiver: Some(NativeReceiver::Map),
        signature: |recv, _| {
            let (k, _) = key_value_of(recv);
            Ok(NativeSignature {
                params: vec![k],
                return_type: ResolvedType::Bool,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let key_value = pop_any(state)?;
            let m = pop_map(state, span)?;
            let key = value_to_map_key(&key_value, span)?;
            let found = m.borrow().entries.iter().any(|(k, _)| *k == key);
            state.stack.push(Value::Bool(found));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // get: (K) -> maybe V
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "get",
        receiver: Some(NativeReceiver::Map),
        signature: |recv, _| {
            let (k, v) = key_value_of(recv);
            Ok(NativeSignature {
                params: vec![k],
                return_type: ResolvedType::Nillable(Box::new(v)),
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let key_value = pop_any(state)?;
            let m = pop_map(state, span)?;
            let key = value_to_map_key(&key_value, span)?;
            let result = m
                .borrow()
                .entries
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone());
            state.stack.push(result.unwrap_or(Value::Nil));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // insert: (K, V)  — mutating
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "insert",
        receiver: Some(NativeReceiver::Map),
        signature: |recv, _| {
            let (k, v) = key_value_of(recv);
            Ok(NativeSignature {
                params: vec![k, v],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let value = pop_any(state)?;
            let key_value = pop_any(state)?;
            let m = pop_map(state, span)?;
            let key = value_to_map_key(&key_value, span)?;
            let mut data = m.borrow_mut(mc);
            if let Some(slot) = data.entries.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = value;
            } else {
                data.entries.push((key, value));
            }
            drop(data);
            state.stack.push(Value::Int(0));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // remove: (K) -> maybe V  — mutating
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "remove",
        receiver: Some(NativeReceiver::Map),
        signature: |recv, _| {
            let (k, v) = key_value_of(recv);
            Ok(NativeSignature {
                params: vec![k],
                return_type: ResolvedType::Nillable(Box::new(v)),
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let key_value = pop_any(state)?;
            let m = pop_map(state, span)?;
            let key = value_to_map_key(&key_value, span)?;
            let mut data = m.borrow_mut(mc);
            let pos = data.entries.iter().position(|(k, _)| *k == key);
            let removed = pos.map(|i| data.entries.remove(i).1);
            drop(data);
            state.stack.push(removed.unwrap_or(Value::Nil));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // clear: ()  — mutating
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "clear",
        receiver: Some(NativeReceiver::Map),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let m = pop_map(state, span)?;
            m.borrow_mut(mc).entries.clear();
            state.stack.push(Value::Int(0));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // keys: () -> [K]  — insertion order
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "keys",
        receiver: Some(NativeReceiver::Map),
        signature: |recv, _| {
            let (k, _) = key_value_of(recv);
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::List(Box::new(k)),
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let m = pop_map(state, span)?;
            let elements: Vec<Value> = m
                .borrow()
                .entries
                .iter()
                .map(|(k, _)| map_key_to_value(k, mc))
                .collect();
            let list = ListData { elements };
            state
                .stack
                .push(Value::List(Gc::new(mc, RefLock::new(list))));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // values: () -> [V]  — insertion order
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "values",
        receiver: Some(NativeReceiver::Map),
        signature: |recv, _| {
            let (_, v) = key_value_of(recv);
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::List(Box::new(v)),
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let m = pop_map(state, span)?;
            let elements: Vec<Value> = m.borrow().entries.iter().map(|(_, v)| v.clone()).collect();
            let list = ListData { elements };
            state
                .stack
                .push(Value::List(Gc::new(mc, RefLock::new(list))));
            Ok(())
        },
    });
}
