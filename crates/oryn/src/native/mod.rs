//! Native function registry — the unified dispatch path for everything
//! that "ships with the language and gets called from user programs":
//! methods on `string`, `range`, `[T]`, and `{K: V}`, plus free-standing
//! globals like `print`, `len`, `to_string`, `parse_int`, and `assert`.
//!
//! # Why a registry?
//!
//! The pre-registry implementation had two parallel ad-hoc dispatch
//! paths: a `ListMethod` enum + `CallListMethod` instruction for list
//! methods, and a `BuiltinFunction` enum + `CallBuiltin` instruction for
//! globals. Adding a single new method required edits across four files
//! and never reused logic. String, range, and map had no methods at all.
//!
//! The registry collapses both paths into one. Each native is a
//! [`NativeFn`] entry — a name, an optional receiver type, a signature
//! computer, and a body. The compiler resolves method calls to indices
//! into the registry; the VM executes by index via a single
//! `Instruction::CallNative(idx, arity)` handler.
//!
//! # Adding a new native method
//!
//! Pick the file by receiver:
//!
//! - String methods → [`string`]
//! - Range methods → [`range`]
//! - List methods → [`list`]
//! - Map methods → [`map`]
//! - Free functions → [`globals`]
//!
//! Add a new [`NativeFn`] block in that file's `register` function.
//! Each block has four pieces:
//!
//! ```ignore
//! r.register(NativeFn {
//!     name: "to_upper",                    // user-facing name
//!     receiver: Some(NativeReceiver::String), // None = global function
//!     signature: |_recv, _args| Ok(NativeSignature {
//!         params: vec![],
//!         return_type: ResolvedType::Str,
//!     }),
//!     body: |state, mc, _chunk, _writer, _span| {
//!         let s = pop_string(state)?;
//!         state.stack.push(Value::String(Gc::new(mc, s.to_uppercase())));
//!         Ok(())
//!     },
//! });
//! ```
//!
//! The signature closure has the receiver type and the already-resolved
//! argument types in scope, so generic methods like `[T].push(T)` can
//! peek at the receiver's element type and stitch the right param list
//! together — a bidirectional type check.
//!
//! # Higher-order methods
//!
//! Methods like `[T].map(fn(T) -> U)` cannot live in this registry
//! because the VM dispatch loop is not re-entrant — a native body
//! can't call back into the VM to invoke a user-supplied closure on
//! each list element. Those methods are handled by [`intrinsics`],
//! which the compiler consults *before* the registry: if the call
//! matches an intrinsic, the compiler emits bytecode for it directly
//! (a desugared `for` loop with `CallValue` per iteration) instead of
//! emitting a `CallNative` instruction.

use std::collections::HashMap;
use std::io::Write;
use std::ops::Range;

use crate::compiler::types::ResolvedType;
use crate::errors::RuntimeError;
use crate::vm::chunk::Chunk;
use crate::vm::value::VmState;

pub(crate) mod globals;
pub(crate) mod intrinsics;
pub(crate) mod list;
pub(crate) mod map;
pub(crate) mod range;
pub(crate) mod string;

/// Receiver kind that a [`NativeFn`] binds to. Methods scope to one
/// receiver kind; globals (functions, not methods) use [`receiver: None`].
///
/// `List` and `Map` are coarse — they match any element/key/value type.
/// The [`NativeFn::signature`] closure does the fine-grained type
/// checking against the actual receiver type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NativeReceiver {
    String,
    Range,
    List,
    Map,
}

impl NativeReceiver {
    /// Pick the receiver kind that matches a [`ResolvedType`], if any.
    /// Returns `None` for receivers that have no native methods (objects,
    /// enums, primitives).
    pub(crate) fn from_type(ty: &ResolvedType) -> Option<Self> {
        match ty {
            ResolvedType::Str => Some(NativeReceiver::String),
            ResolvedType::Range => Some(NativeReceiver::Range),
            ResolvedType::List(_) => Some(NativeReceiver::List),
            ResolvedType::Map(_, _) => Some(NativeReceiver::Map),
            _ => None,
        }
    }
}

/// The resolved signature of a native function — a parameter list and
/// a return type. The signature is computed lazily by [`NativeFn::signature`]
/// so generic methods can build it from the actual receiver type.
#[derive(Debug, Clone)]
pub(crate) struct NativeSignature {
    pub(crate) params: Vec<ResolvedType>,
    pub(crate) return_type: ResolvedType,
}

/// Function pointer type for the per-method signature computer.
///
/// `receiver` is the receiver type (e.g. `[int]` for a list method).
/// For globals it is [`ResolvedType::Nil`] — globals ignore it.
///
/// `args` is the list of already-resolved argument types. Most signature
/// closures ignore this — but it's available for the rare case where
/// a method's parameter shape depends on what's being passed in (overload
/// disambiguation, variadic functions like `print`).
///
/// Returning `Err` produces a clean compile error at the call site.
pub(crate) type NativeSignatureFn =
    fn(receiver: &ResolvedType, args: &[ResolvedType]) -> Result<NativeSignature, String>;

/// Function pointer type for the per-method runtime body.
///
/// At call time:
/// 1. The arguments are on top of the VM stack in push order.
/// 2. For methods, the receiver is below the arguments.
/// 3. The body pops the arguments and (for methods) the receiver,
///    pushes its result back onto the stack.
/// 4. The body returns `Ok(())` on success or a [`RuntimeError`] on
///    failure (type mismatch, index out of bounds, etc.).
///
/// Every body **must** push exactly one value onto the stack — methods
/// that have no logical return value push `Value::Int(0)` as a sentinel
/// to keep the stack discipline uniform across all calls.
pub(crate) type NativeBodyFn = for<'gc> fn(
    state: &mut VmState<'gc>,
    mc: &'gc gc_arena::Mutation<'gc>,
    chunk: &Chunk,
    writer: &mut dyn Write,
    span: &Range<usize>,
) -> Result<(), RuntimeError>;

/// A registered native function. The four fields are everything the
/// compiler and VM need to dispatch a call to it.
///
/// `Debug` prints just the name and receiver — function pointers are
/// opaque and there's nothing useful to show beyond that.
pub(crate) struct NativeFn {
    pub(crate) name: &'static str,
    pub(crate) receiver: Option<NativeReceiver>,
    pub(crate) signature: NativeSignatureFn,
    pub(crate) body: NativeBodyFn,
}

impl std::fmt::Debug for NativeFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeFn")
            .field("name", &self.name)
            .field("receiver", &self.receiver)
            .finish()
    }
}

/// The flat registry built once at compiler construction and shared by
/// reference between the compiler and the VM. The compiler uses
/// [`lookup_method`] / [`lookup_global`] to resolve names to indices;
/// the VM uses [`get`] to dispatch by index.
///
/// The registry is intentionally append-only after construction — a
/// new entry would change every existing index, which would invalidate
/// any in-flight `Chunk`. Build it once, share it everywhere.
#[derive(Debug)]
pub(crate) struct NativeRegistry {
    fns: Vec<NativeFn>,
    methods: HashMap<(NativeReceiver, &'static str), u32>,
    globals: HashMap<&'static str, u32>,
}

impl NativeRegistry {
    /// Register every native that ships with the language. The order
    /// here determines the indices baked into bytecode, so adding a
    /// new module here is fine but reordering existing modules would
    /// invalidate cached chunks (we don't cache yet, but the principle
    /// stands).
    pub(crate) fn build() -> Self {
        let mut r = Self {
            fns: Vec::new(),
            methods: HashMap::new(),
            globals: HashMap::new(),
        };
        string::register(&mut r);
        range::register(&mut r);
        list::register(&mut r);
        map::register(&mut r);
        globals::register(&mut r);
        r
    }

    /// Add a single entry to the registry. Panics on a duplicate
    /// `(receiver, name)` or duplicate global name — those are author
    /// bugs, not user errors, and would silently shadow each other if
    /// we let them through.
    pub(crate) fn register(&mut self, native: NativeFn) {
        let idx = self.fns.len() as u32;
        match native.receiver {
            Some(recv) => {
                let key = (recv, native.name);
                if self.methods.insert(key, idx).is_some() {
                    panic!(
                        "duplicate native method `{:?}::{}`",
                        native.receiver, native.name
                    );
                }
            }
            None => {
                if self.globals.insert(native.name, idx).is_some() {
                    panic!("duplicate native global `{}`", native.name);
                }
            }
        }
        self.fns.push(native);
    }

    /// Look up a method by receiver type and name. Returns the entry's
    /// index in the flat table plus a reference to the entry itself.
    /// The compiler uses the index to emit `CallNative(idx, arity)`
    /// and the entry to type-check the arguments.
    pub(crate) fn lookup_method(
        &self,
        receiver: &ResolvedType,
        name: &str,
    ) -> Option<(u32, &NativeFn)> {
        let kind = NativeReceiver::from_type(receiver)?;
        let idx = *self.methods.get(&(kind, name))?;
        Some((idx, &self.fns[idx as usize]))
    }

    /// Look up a global function by name. Same shape as
    /// [`lookup_method`] — returns the index and the entry.
    pub(crate) fn lookup_global(&self, name: &str) -> Option<(u32, &NativeFn)> {
        let idx = *self.globals.get(name)?;
        Some((idx, &self.fns[idx as usize]))
    }

    /// Direct lookup by index. Used by the VM at dispatch time. Panics
    /// on an out-of-range index — that would be a compiler bug since
    /// the compiler only emits indices it produced via lookup.
    pub(crate) fn get(&self, idx: u32) -> &NativeFn {
        &self.fns[idx as usize]
    }

    /// All registered method names for a given receiver kind. The LSP
    /// uses this to power method-name completion.
    #[allow(dead_code)]
    pub(crate) fn methods_for(&self, recv: NativeReceiver) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .methods
            .keys()
            .filter(|(r, _)| *r == recv)
            .map(|(_, name)| *name)
            .collect();
        names.sort();
        names
    }

    /// All registered global function names, sorted alphabetically.
    /// Used by the LSP for completion outside of method calls.
    #[allow(dead_code)]
    pub(crate) fn global_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.globals.keys().copied().collect();
        names.sort();
        names
    }
}

// ---------------------------------------------------------------------------
// Helper utilities for native bodies
// ---------------------------------------------------------------------------

use crate::errors::ValueType;
use crate::vm::value::Value;

/// Pop the top of stack as a string. Errors out with a clean type
/// error if the value isn't a string.
pub(crate) fn pop_string<'gc>(
    state: &mut VmState<'gc>,
    span: &Range<usize>,
) -> Result<gc_arena::Gc<'gc, String>, RuntimeError> {
    let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
    match value {
        Value::String(s) => Ok(s),
        other => Err(RuntimeError::TypeError {
            expected: ValueType::String,
            actual: ValueType::from(&other),
            span: Some(span.clone()),
        }),
    }
}

/// Pop the top of stack as an int. Errors out with a clean type error
/// if the value isn't an int.
pub(crate) fn pop_int<'gc>(
    state: &mut VmState<'gc>,
    span: &Range<usize>,
) -> Result<i32, RuntimeError> {
    let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
    match value {
        Value::Int(n) => Ok(n),
        other => Err(RuntimeError::TypeError {
            expected: ValueType::Int,
            actual: ValueType::from(&other),
            span: Some(span.clone()),
        }),
    }
}

/// Pop the top of stack as a list (the GC handle, not the elements).
pub(crate) fn pop_list<'gc>(
    state: &mut VmState<'gc>,
    span: &Range<usize>,
) -> Result<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::vm::value::ListData<'gc>>>, RuntimeError>
{
    let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
    match value {
        Value::List(l) => Ok(l),
        other => Err(RuntimeError::TypeError {
            expected: ValueType::List,
            actual: ValueType::from(&other),
            span: Some(span.clone()),
        }),
    }
}

/// Pop the top of stack as a map (the GC handle, not the entries).
pub(crate) fn pop_map<'gc>(
    state: &mut VmState<'gc>,
    span: &Range<usize>,
) -> Result<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::vm::value::MapData<'gc>>>, RuntimeError>
{
    let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
    match value {
        Value::Map(m) => Ok(m),
        other => Err(RuntimeError::TypeError {
            expected: ValueType::Map,
            actual: ValueType::from(&other),
            span: Some(span.clone()),
        }),
    }
}

/// Pop the top of stack as a range (the GC handle).
pub(crate) fn pop_range<'gc>(
    state: &mut VmState<'gc>,
    span: &Range<usize>,
) -> Result<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::vm::value::RangeValue>>, RuntimeError>
{
    let value = state.stack.pop().ok_or(RuntimeError::StackUnderflow)?;
    match value {
        Value::Range(r) => Ok(r),
        other => Err(RuntimeError::TypeError {
            expected: ValueType::Range,
            actual: ValueType::from(&other),
            span: Some(span.clone()),
        }),
    }
}

/// Pop the top of stack as a generic value with no type check.
pub(crate) fn pop_any<'gc>(state: &mut VmState<'gc>) -> Result<Value<'gc>, RuntimeError> {
    state.stack.pop().ok_or(RuntimeError::StackUnderflow)
}

/// Pop `n` values from the stack as a contiguous slice in original
/// push order (the first pushed value is at index 0). Used by methods
/// like `print` that consume a variable number of arguments.
pub(crate) fn pop_n<'gc>(
    state: &mut VmState<'gc>,
    n: usize,
) -> Result<Vec<Value<'gc>>, RuntimeError> {
    if state.stack.len() < n {
        return Err(RuntimeError::StackUnderflow);
    }
    Ok(state.stack.split_off(state.stack.len() - n))
}
