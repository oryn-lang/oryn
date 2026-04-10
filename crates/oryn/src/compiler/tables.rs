use std::collections::HashMap;

use crate::compiler::types::ResolvedType;

use super::types::{MethodSignature, ObjDefInfo};

// ---------------------------------------------------------------------------
// Locals
// ---------------------------------------------------------------------------

/// Maps variable names to numeric slot indices during compilation.
/// The third tuple element tracks the variable's object type name
/// (if known), which enables compile-time field resolution. It's
/// populated from ObjLiteral assignments, variable-to-variable copies,
/// and typed function parameters.
#[derive(Clone)]
pub(super) struct Locals {
    // (slot, mutable, obj_type).
    slots: HashMap<String, (usize, bool, ResolvedType)>,
    pub(super) count: usize,
    pub(super) max_count: usize,
    pub(super) return_type: Option<ResolvedType>,
}

#[derive(Clone)]
pub(super) struct LocalsSnapshot {
    slots: HashMap<String, (usize, bool, ResolvedType)>,
    count: usize,
}

impl Locals {
    pub(super) fn new() -> Self {
        Self {
            slots: HashMap::new(),
            count: 0,
            max_count: 0,
            return_type: None,
        }
    }

    pub(super) fn define(&mut self, name: String, mutable: bool, obj_type: ResolvedType) -> usize {
        let slot = self.count;

        self.slots.insert(name, (slot, mutable, obj_type));
        self.count += 1;
        self.max_count = self.max_count.max(self.count);

        slot
    }

    pub(super) fn resolve(&self, name: &str) -> Option<(usize, bool, ResolvedType)> {
        self.slots.get(name).cloned()
    }

    pub(super) fn snapshot(&self) -> LocalsSnapshot {
        LocalsSnapshot {
            slots: self.slots.clone(),
            count: self.count,
        }
    }

    pub(super) fn restore(&mut self, snapshot: LocalsSnapshot) {
        self.slots = snapshot.slots;
        self.count = snapshot.count;
    }
}

// ---------------------------------------------------------------------------
// FunctionTable
// ---------------------------------------------------------------------------

/// Maps function names to their **absolute** index in the merged function
/// table. The `base_offset` lets the compiler produce absolute indices
/// even when compiling a module whose functions will eventually be appended
/// to a larger merged chunk. Pass local indices to `register`; they get
/// shifted by `base_offset` before being stored.
#[derive(Clone)]
pub(super) struct FunctionTable {
    pub(super) names: HashMap<String, usize>,
    pub(super) signatures: HashMap<String, FunctionSignature>,
    pub(super) base_offset: usize,
}

impl FunctionTable {
    pub(super) fn new(base_offset: usize) -> Self {
        Self {
            names: HashMap::new(),
            signatures: HashMap::new(),
            base_offset,
        }
    }

    /// Register a function by its local index (position in `output.functions`).
    /// The table stores the absolute index `base_offset + local_idx`.
    pub(super) fn register(&mut self, name: String, local_idx: usize) -> usize {
        let absolute = self.base_offset + local_idx;
        self.names.insert(name, absolute);
        absolute
    }

    /// Resolve a function name to its absolute index in the merged chunk.
    pub(super) fn resolve(&self, name: &str) -> Option<usize> {
        self.names.get(name).copied()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FunctionSignature {
    pub(crate) param_types: Vec<ResolvedType>,
    pub(crate) return_type: ResolvedType,
}

// ---------------------------------------------------------------------------
// ObjTable
// ---------------------------------------------------------------------------

/// Compile-time registry of object definitions. Parallel to FunctionTable
/// but for types. The `names` HashMap stores **absolute** obj_def indices
/// (shifted by `base_offset`). The `defs` vector is indexed locally (0..N
/// for this compilation unit). `resolve` returns the absolute index so
/// callers can emit `NewObject(absolute_idx, ..)` directly.
#[derive(Clone)]
pub(super) struct ObjTable {
    pub(super) names: HashMap<String, usize>,
    pub(super) defs: Vec<ObjDefInfo>,
    pub(super) base_offset: usize,
}

impl ObjTable {
    pub(super) fn new(base_offset: usize) -> Self {
        Self {
            names: HashMap::new(),
            defs: Vec::new(),
            base_offset,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn register(
        &mut self,
        name: String,
        fields: Vec<String>,
        field_types: Vec<ResolvedType>,
        field_is_pub: Vec<bool>,
        methods: HashMap<String, usize>,
        static_methods: HashMap<String, usize>,
        method_is_pub: HashMap<String, bool>,
        static_method_is_pub: HashMap<String, bool>,
        method_signatures: HashMap<String, FunctionSignature>,
        static_method_signatures: HashMap<String, FunctionSignature>,
        signatures: Vec<MethodSignature>,
        is_pub: bool,
    ) -> usize {
        let local = self.defs.len();
        let absolute = self.base_offset + local;

        self.names.insert(name.clone(), absolute);
        self.defs.push(ObjDefInfo {
            name,
            fields,
            field_types,
            field_is_pub,
            methods,
            static_methods,
            method_is_pub,
            static_method_is_pub,
            method_signatures,
            static_method_signatures,
            signatures,
            is_pub,
        });

        absolute
    }

    /// Resolve a type name to its absolute index and def. The def is looked
    /// up locally via `absolute - base_offset`.
    pub(super) fn resolve(&self, name: &str) -> Option<(usize, &ObjDefInfo)> {
        let absolute = *self.names.get(name)?;
        let local = absolute - self.base_offset;
        Some((absolute, &self.defs[local]))
    }
}
