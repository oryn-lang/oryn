use gc_arena::{Collect, Gc, lock::RefLock};

#[derive(Debug, Clone, PartialEq, Collect)]
#[collect(no_drop)]
pub(crate) enum Value<'gc> {
    /// Sentinel for local variable slots that haven't been written to yet.
    /// A GetLocal hitting this is a compiler bug (the compiler rejects
    /// reads of undefined variables), so the VM treats it as a fatal error.
    Uninitialized,
    Bool(bool),
    Float(f32),
    Int(i32),
    Range(Gc<'gc, RefLock<RangeValue>>),
    // RefLock is gc-arena's GC-aware RefCell. It provides interior
    // mutability for field writes: .borrow() to read, .borrow_mut(mc)
    // to write (requires the mutation context from arena.mutate_root).
    // Gc wraps the whole thing so objects are heap-allocated, reference-
    // counted, and collected by the GC. Cloning a Value::Object copies
    // the Gc pointer (alias), not the underlying data.
    Object(Gc<'gc, RefLock<ObjData<'gc>>>),
    String(Gc<'gc, String>),
}

#[derive(Debug, PartialEq, Collect)]
#[collect(no_drop)]
pub(crate) struct ObjData<'gc> {
    // Index into Chunk.obj_defs for the type name and field layout.
    pub type_idx: usize,
    // Field values in definition order. Field index is resolved at
    // compile time, so access is a direct array index with no hashing.
    pub fields: Vec<Value<'gc>>,
}

#[derive(Debug, PartialEq, Collect)]
#[collect(no_drop)]
pub(crate) struct RangeValue {
    pub current: i32,
    pub end: i32,
    pub inclusive: bool,
}

/// A call frame on the VM's call stack. Each function invocation
/// (including top-level code) gets its own frame with an instruction
/// pointer and a base offset into the VM's shared locals stack.
///
/// The compiler assigns local slots per function body, so at runtime
/// a local access is just `frame.local_base + slot`. This avoids
/// allocating a fresh Vec for every call; nested calls carve out a
/// new window at the end of VmState.locals and Return truncates it.
#[derive(Debug, Collect)]
#[collect(no_drop)]
pub(super) struct CallFrame {
    pub function_idx: Option<usize>,
    pub ip: usize,
    // Base offset into VmState.locals for this frame's local slots.
    pub local_base: usize,
}

/// The GC-rooted state of the VM.
///
/// `stack` is the operand/value stack used by bytecode execution.
/// `locals` is a shared storage area for all active call frames.
/// `frames` tracks the current call stack and each frame's locals window.
#[derive(Collect)]
#[collect(no_drop)]
pub(super) struct VmState<'gc> {
    pub stack: Vec<Value<'gc>>,
    pub locals: Vec<Value<'gc>>,
    pub frames: Vec<CallFrame>,
}
