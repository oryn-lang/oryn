use gc_arena::{Collect, Gc, lock::RefLock};

#[derive(Debug, Clone, PartialEq, Collect)]
#[collect(no_drop)]
pub(crate) enum Value<'gc> {
    /// Sentinel for local variable slots that haven't been written to yet.
    /// A GetLocal hitting this is a compiler bug (the compiler rejects
    /// reads of undefined variables), so the VM treats it as a fatal error.
    Uninitialized,
    /// The `nil` value — represents the absence of a value for `T?` types.
    Nil,
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
    /// A homogeneous list. Element typing is enforced statically at
    /// compile time and erased at runtime; storage is just a
    /// `Vec<Value<'gc>>` wrapped in `RefLock` for in-place mutation.
    List(Gc<'gc, RefLock<ListData<'gc>>>),
    /// A homogeneous map. Key/value typing is enforced statically at
    /// compile time and erased at runtime; keys are stored as primitive
    /// owned values so lookup does not depend on GC pointer identity.
    Map(Gc<'gc, RefLock<MapData<'gc>>>),
    /// A tagged-union (enum) value. `def_idx` is the index into
    /// `Chunk.enum_defs` (gives access to the enum's name and the
    /// variant's name list for printing). `variant_idx` is the
    /// discriminant — the position of the active variant in the
    /// enum's declaration order, used by match codegen for arm
    /// dispatch. `payload` holds the variant's named-field values
    /// in declaration order; nullary variants have an empty payload.
    ///
    /// Enum values are GC-managed because their payload may
    /// transitively hold heap-allocated values (strings, lists,
    /// other enums, objects). The `RefLock` is reserved for
    /// future payload mutation but isn't currently used in
    /// Slice 1+2 — payload extraction lands in Slice 3.
    Enum(Gc<'gc, RefLock<EnumData<'gc>>>),
}

/// Runtime storage for a list value. Mirrors [`ObjData`] — the fields
/// of an object and the elements of a list are both heterogeneous-at-
/// the-VM-level vectors of [`Value`].
#[derive(Debug, PartialEq, Collect)]
#[collect(no_drop)]
pub(crate) struct ListData<'gc> {
    pub elements: Vec<Value<'gc>>,
}

#[derive(Debug, Clone, PartialEq, Collect)]
#[collect(no_drop)]
pub(crate) enum MapKey {
    String(String),
    Int(i32),
    Bool(bool),
}

#[derive(Debug, PartialEq, Collect)]
#[collect(no_drop)]
pub(crate) struct MapData<'gc> {
    pub entries: Vec<(MapKey, Value<'gc>)>,
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

/// Runtime storage for an enum value. Mirrors [`ObjData`] but adds
/// the variant discriminant. The compiler resolves variant names
/// to indices at compile time, so `match` arm dispatch is just an
/// integer comparison against `variant_idx`.
#[derive(Debug, PartialEq, Collect)]
#[collect(no_drop)]
pub(crate) struct EnumData<'gc> {
    /// Index into `Chunk.enum_defs` — gives access to the enum's
    /// name (for printing) and to the active variant's metadata.
    pub def_idx: usize,
    /// Index of the active variant within the enum's declaration
    /// order. The match codegen generates `EnumDiscriminant`
    /// followed by integer comparisons against this value.
    pub variant_idx: usize,
    /// Payload field values in the variant's declaration order.
    /// Empty for nullary variants. Slice 3 will add bytecode to
    /// extract these into pattern bindings.
    pub payload: Vec<Value<'gc>>,
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
