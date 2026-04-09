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
pub(super) struct Locals {
    // (slot, mutable, obj_type).
    slots: HashMap<String, (usize, bool, ResolvedType)>,
    pub(super) count: usize,
    pub(super) return_type: Option<ResolvedType>,
}

impl Locals {
    pub(super) fn new() -> Self {
        Self {
            slots: HashMap::new(),
            count: 0,
            return_type: None,
        }
    }

    pub(super) fn define(&mut self, name: String, mutable: bool, obj_type: ResolvedType) -> usize {
        let slot = self.count;

        self.slots.insert(name, (slot, mutable, obj_type));
        self.count += 1;

        slot
    }

    pub(super) fn resolve(&self, name: &str) -> Option<(usize, bool, ResolvedType)> {
        self.slots.get(name).cloned()
    }
}

// ---------------------------------------------------------------------------
// FunctionTable
// ---------------------------------------------------------------------------

/// Maps function names to their index in the function table.
/// Separate from the function table itself so we can look up
/// indices without borrowing the output.
#[derive(Clone)]
pub(super) struct FunctionTable {
    pub(super) names: HashMap<String, usize>,
    pub(super) signatures: HashMap<String, FunctionSignature>,
}

impl FunctionTable {
    pub(super) fn new() -> Self {
        Self {
            names: HashMap::new(),
            signatures: HashMap::new(),
        }
    }

    pub(super) fn register(&mut self, name: String, idx: usize) {
        self.names.insert(name, idx);
    }

    pub(super) fn resolve(&self, name: &str) -> Option<usize> {
        self.names.get(name).copied()
    }
}

#[derive(Clone)]
pub(super) struct FunctionSignature {
    pub(super) param_types: Vec<ResolvedType>,
    pub(super) return_type: ResolvedType,
}

// ---------------------------------------------------------------------------
// ObjTable
// ---------------------------------------------------------------------------

/// Compile-time registry of object definitions. Parallel to FunctionTable
/// but for types. Maps type names to their field layouts so the compiler
/// can resolve field accesses to integer indices without runtime lookups.
#[derive(Clone)]
pub(super) struct ObjTable {
    pub(super) names: HashMap<String, usize>,
    pub(super) defs: Vec<ObjDefInfo>,
}

impl ObjTable {
    pub(super) fn new() -> Self {
        Self {
            names: HashMap::new(),
            defs: Vec::new(),
        }
    }

    pub(super) fn register(
        &mut self,
        name: String,
        fields: Vec<String>,
        field_types: Vec<ResolvedType>,
        methods: HashMap<String, usize>,
        signatures: Vec<MethodSignature>,
    ) -> usize {
        let idx = self.defs.len();

        self.names.insert(name.clone(), idx);
        self.defs.push(ObjDefInfo {
            name,
            fields,
            field_types,
            methods,
            signatures,
        });

        idx
    }

    pub(super) fn resolve(&self, name: &str) -> Option<(usize, &ObjDefInfo)> {
        let idx = *self.names.get(name)?;

        Some((idx, &self.defs[idx]))
    }
}
