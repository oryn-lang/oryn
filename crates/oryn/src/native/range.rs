//! Range methods. See [`super`] for the registration model.

use gc_arena::Gc;

use crate::compiler::types::ResolvedType;
use crate::vm::value::{ListData, Value};

use super::{NativeFn, NativeReceiver, NativeRegistry, NativeSignature, pop_int, pop_range};

pub(crate) fn register(r: &mut NativeRegistry) {
    // -----------------------------------------------------------------
    // start: () -> int
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "start",
        receiver: Some(NativeReceiver::Range),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let r = pop_range(state, span)?;
            // Note: `current` advances during for-loop iteration. We
            // don't expose iteration state on `range` directly — for-
            // loops use a fresh range each pass — so the value of
            // `current` here always equals the originally-declared
            // start.
            state.stack.push(Value::Int(r.borrow().current));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // end: () -> int  — exclusive end (matches the source-level `..`)
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "end",
        receiver: Some(NativeReceiver::Range),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let r = pop_range(state, span)?;
            state.stack.push(Value::Int(r.borrow().end));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // len: () -> int
    //   `..`  — end - start
    //   `..=` — end - start + 1
    // Returns 0 for empty/inverted ranges.
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "len",
        receiver: Some(NativeReceiver::Range),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::Int,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let r = pop_range(state, span)?;
            let r = r.borrow();
            let raw = r.end - r.current + if r.inclusive { 1 } else { 0 };
            state.stack.push(Value::Int(raw.max(0)));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // contains: (int) -> bool
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "contains",
        receiver: Some(NativeReceiver::Range),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![ResolvedType::Int],
                return_type: ResolvedType::Bool,
            })
        },
        body: |state, _mc, _chunk, _writer, span| {
            let needle = pop_int(state, span)?;
            let r = pop_range(state, span)?;
            let r = r.borrow();
            let in_range = if r.inclusive {
                needle >= r.current && needle <= r.end
            } else {
                needle >= r.current && needle < r.end
            };
            state.stack.push(Value::Bool(in_range));
            Ok(())
        },
    });

    // -----------------------------------------------------------------
    // to_list: () -> [int]  — materialize the range into a list
    // -----------------------------------------------------------------
    r.register(NativeFn {
        name: "to_list",
        receiver: Some(NativeReceiver::Range),
        signature: |_, _| {
            Ok(NativeSignature {
                params: vec![],
                return_type: ResolvedType::List(Box::new(ResolvedType::Int)),
            })
        },
        body: |state, mc, _chunk, _writer, span| {
            let r = pop_range(state, span)?;
            let r = r.borrow();
            let end = if r.inclusive { r.end + 1 } else { r.end };
            let elements: Vec<Value> = (r.current..end).map(Value::Int).collect();
            let list = ListData { elements };
            state
                .stack
                .push(Value::List(Gc::new(mc, gc_arena::lock::RefLock::new(list))));
            Ok(())
        },
    });
}
