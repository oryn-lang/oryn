use gc_arena::{Collect, Gc};

#[derive(Debug, Clone, PartialEq, PartialOrd, Collect)]
#[collect(no_drop)]
pub(crate) enum Value<'gc> {
    /// Sentinel for local variable slots that haven't been written to yet.
    /// A GetLocal hitting this is a compiler bug (the compiler rejects
    /// reads of undefined variables), so the VM treats it as a fatal error.
    Uninitialized,
    Bool(bool),
    Float(f32),
    Int(i32),
    String(Gc<'gc, String>),
}

/// A call frame on the VM's call stack. Each function invocation
/// (including top-level code) gets its own frame with an instruction
/// pointer and a fixed-size array of local variable slots.
#[derive(Debug, Collect)]
#[collect(no_drop)]
pub(super) struct CallFrame<'gc> {
    pub function_idx: Option<usize>,
    pub ip: usize,
    // Local variables indexed by slot number. Slot indices are
    // assigned at compile time so access is O(1) with no hashing.
    pub locals: Vec<Value<'gc>>,
}

/// The GC-rooted state of the VM: the value stack and the call stack.
#[derive(Collect)]
#[collect(no_drop)]
pub(super) struct VmState<'gc> {
    pub stack: Vec<Value<'gc>>,
    pub frames: Vec<CallFrame<'gc>>,
}
