//! List methods (the non-higher-order ones — `map`, `filter`, etc. live
//! in [`super::intrinsics`]). See [`super`] for the registration model.

use gc_arena::Gc;
use gc_arena::lock::RefLock;

use crate::compiler::types::ResolvedType;
use crate::errors::{RuntimeError, ValueType};
use crate::vm::value::{ListData, Value};

use super::{
    NativeFn, NativeReceiver, NativeRegistry, NativeSignature, pop_any, pop_int, pop_list,
};

/// Element type of a list receiver. The compiler always passes a
/// `List(elem)` here; for empty literals or unknown receivers it
/// passes `List(Unknown)`. Returning `Unknown` means "no constraint".
fn elem_of(receiver: &ResolvedType) -> ResolvedType {
    match receiver {
        ResolvedType::List(inner) => (**inner).clone(),
        _ => ResolvedType::Unknown,
    }
}

/// Structural equality on values. Used by `contains` and `index_of`.
/// Mirrors the VM's existing structural-equality semantics: scalars
/// compare by value, strings by content, enums by def+variant+payload,
/// nil-to-nil is true. For compound types we use direct value equality
/// which works for the comparable cases users actually pass to
/// `contains`/`index_of`.
fn values_equal<'a>(a: &Value<'a>, b: &Value<'a>) -> bool {
    match (a, b) {
        (Value::Int(l), Value::Int(r)) => l == r,
        (Value::Float(l), Value::Float(r)) => l == r,
        (Value::Bool(l), Value::Bool(r)) => l == r,
        (Value::String(l), Value::String(r)) => l.as_str() == r.as_str(),
        (Value::Nil, Value::Nil) => true,
        (Value::Enum(l), Value::Enum(r)) => {
            let ld = l.borrow();
            let rd = r.borrow();
            ld.def_idx == rd.def_idx && ld.variant_idx == rd.variant_idx && ld.payload == rd.payload
        }
        _ => false,
    }
}

pub(crate) fn register(r: &mut NativeRegistry) {
    // -----------------------------------------------------------------
    // len: () -> int
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "len",
        receiver: Some(NativeReceiver::List),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let l = pop_list(state, span)?;
            state
                .stack
                .push(Value::Int(l.borrow().elements.len() as i32));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // is_empty: () -> bool
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "is_empty",
        receiver: Some(NativeReceiver::List),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Bool,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let l = pop_list(state, span)?;
            state
                .stack
                .push(Value::Bool(l.borrow().elements.is_empty()));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // push: (T)  — mutating
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "push",
        receiver: Some(NativeReceiver::List),
        signature: |recv, _| {
            Ok(NativeSignature {
                params: vec![elem_of(recv)],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let value = pop_any(state)?;
            let l = pop_list(state, span)?;
            l.borrow_mut(mc).elements.push(value);
            state.stack.push(Value::Int(0));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // pop: () -> maybe T  — mutating
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "pop",
        receiver: Some(NativeReceiver::List),
        signature: |recv, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Nillable(Box::new(elem_of(recv))),
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let l = pop_list(state, span)?;
            let popped = l.borrow_mut(mc).elements.pop();
            state.stack.push(popped.unwrap_or(Value::Nil));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // insert: (int, T)  — mutating; raises IndexOutOfBounds
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "insert",
        receiver: Some(NativeReceiver::List),
        signature: |recv, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Int, elem_of(recv)],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let value = pop_any(state)?;
            let idx = pop_int(state, span)?;
            let l = pop_list(state, span)?;
            let len = l.borrow().elements.len();
            if idx < 0 || (idx as usize) > len {
                return Err(RuntimeError::IndexOutOfBounds {
                    index: idx,
                    len,
                    span: Some(span.clone()),
                });
            }
            l.borrow_mut(mc).elements.insert(idx as usize, value);
            state.stack.push(Value::Int(0));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // remove: (int) -> T  — mutating; raises IndexOutOfBounds
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "remove",
        receiver: Some(NativeReceiver::List),
        signature: |recv, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Int],
                return_type: elem_of(recv),
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let idx = pop_int(state, span)?;
            let l = pop_list(state, span)?;
            let len = l.borrow().elements.len();
            if idx < 0 || (idx as usize) >= len {
                return Err(RuntimeError::IndexOutOfBounds {
                    index: idx,
                    len,
                    span: Some(span.clone()),
                });
            }
            let removed = l.borrow_mut(mc).elements.remove(idx as usize);
            state.stack.push(removed);
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // clear: ()  — mutating
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "clear",
        receiver: Some(NativeReceiver::List),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let l = pop_list(state, span)?;
            l.borrow_mut(mc).elements.clear();
            state.stack.push(Value::Int(0));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // contains: (T) -> bool
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "contains",
        receiver: Some(NativeReceiver::List),
        signature: |recv, _| {
            Ok(NativeSignature {
                params: vec![elem_of(recv)],
                return_type: ResolvedType::Bool,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let needle = pop_any(state)?;
            let l = pop_list(state, span)?;
            let found = l.borrow().elements.iter().any(|v| values_equal(v, &needle));
            state.stack.push(Value::Bool(found));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // index_of: (T) -> maybe int
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "index_of",
        receiver: Some(NativeReceiver::List),
        signature: |recv, _| {
            Ok(NativeSignature {
                params: vec![elem_of(recv)],
                return_type: ResolvedType::Nillable(Box::new(ResolvedType::Int)),
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let needle = pop_any(state)?;
            let l = pop_list(state, span)?;
            let pos = l
                .borrow()
                .elements
                .iter()
                .position(|v| values_equal(v, &needle));
            match pos {
                Some(i) => state.stack.push(Value::Int(i as i32)),
                None => state.stack.push(Value::Nil),
            }
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // first: () -> maybe T
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "first",
        receiver: Some(NativeReceiver::List),
        signature: |recv, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Nillable(Box::new(elem_of(recv))),
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let l = pop_list(state, span)?;
            let first = l.borrow().elements.first().cloned();
            state.stack.push(first.unwrap_or(Value::Nil));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // last: () -> maybe T
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "last",
        receiver: Some(NativeReceiver::List),
        signature: |recv, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Nillable(Box::new(elem_of(recv))),
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let l = pop_list(state, span)?;
            let last = l.borrow().elements.last().cloned();
            state.stack.push(last.unwrap_or(Value::Nil));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // reverse: ()  — mutating
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "reverse",
        receiver: Some(NativeReceiver::List),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let l = pop_list(state, span)?;
            l.borrow_mut(mc).elements.reverse();
            state.stack.push(Value::Int(0));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // sort: ()  — mutating; only for [int], [float], [string]
    //
    // The signature closure rejects unsorted element types statically
    // so the runtime body never sees a list it can't compare.
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "sort",
        receiver: Some(NativeReceiver::List),
        signature: |recv, _| {
            let elem = elem_of(recv);
            match elem {
                ResolvedType::Int
                | ResolvedType::Float
                | ResolvedType::Str
                | ResolvedType::Unknown => Ok(NativeSignature {
                    params: vec![],
                    return_type: ResolvedType::Int,
                }),
                _ => Err(format!(
                    "list `sort` requires elements of type `int`, `float`, or `string`, got `[{}]`",
                    elem.display_name()
                )),
            }
        },
        body: |state, mc, _chunk, _writer, span| {
            let l = pop_list(state, span)?;
            // The compiler enforces that this only runs on
            // sortable element types — but the actual list at
            // runtime is type-erased, so we still match on
            // the first element to pick the comparator. Empty
            // lists are no-ops.
            let mut data = l.borrow_mut(mc);
            if data.elements.is_empty() {
                state.stack.push(Value::Int(0));
                return Ok(());
            }
            match &data.elements[0] {
                Value::Int(_) => {
                    data.elements.sort_by(|a, b| match (a, b) {
                        (Value::Int(l), Value::Int(r)) => l.cmp(r),
                        _ => std::cmp::Ordering::Equal,
                    });
                }
                Value::Float(_) => {
                    data.elements.sort_by(|a, b| match (a, b) {
                        (Value::Float(l), Value::Float(r)) => {
                            l.partial_cmp(r).unwrap_or(std::cmp::Ordering::Equal)
                        }
                        _ => std::cmp::Ordering::Equal,
                    });
                }
                Value::String(_) => {
                    data.elements.sort_by(|a, b| match (a, b) {
                        (Value::String(l), Value::String(r)) => l.as_str().cmp(r.as_str()),
                        _ => std::cmp::Ordering::Equal,
                    });
                }
                other => {
                    return Err(RuntimeError::TypeError {
                        expected: ValueType::Int,
                        actual: ValueType::from(other),
                        span: Some(span.clone()),
                    });
                }
            }
            drop(data);
            state.stack.push(Value::Int(0));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // concat: ([T]) -> [T]  — non-mutating
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "concat",
        receiver: Some(NativeReceiver::List),
        signature: |recv, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::List(Box::new(elem_of(recv)))],
                return_type: ResolvedType::List(Box::new(elem_of(recv))),
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let other = pop_list(state, span)?;
            let l = pop_list(state, span)?;
            let mut combined = l.borrow().elements.clone();
            combined.extend(other.borrow().elements.iter().cloned());
            let list = ListData { elements: combined };
            state
                .stack
                .push(Value::List(Gc::new(mc, RefLock::new(list))));
            Ok(())
        },
    });
}
