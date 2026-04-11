use std::collections::HashMap;

use crate::compiler::types::ResolvedType;

use super::types::{EnumDefInfo, EnumVariantInfo, MethodSignature, ObjDefInfo};

// ---------------------------------------------------------------------------
// Locals
// ---------------------------------------------------------------------------

/// What kind of binding a local came from. Tracked so the compiler
/// can phrase immutability errors source-accurately rather than
/// always blaming "val binding". Each variant carries enough info
/// to format a decent diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum BindingKind {
    /// `let x = ...` — a rebindable binding. Mutable.
    Let,
    /// `val x = ...` — a value. Transitively immutable.
    Val,
    /// A function or method parameter without `mut`. Always immutable
    /// in Oryn — there's no opt-out at the call site, only at the
    /// declaration site via `MutParam`.
    Param,
    /// A function or method parameter declared `mut`. Mutable through
    /// the function body, but `val`-rooted values cannot be passed in.
    MutParam,
    /// The `self` parameter inside a `mut fn` method. Mutable for
    /// field/index/method writes, but cannot be the LHS of a bare
    /// assignment (`self = ...` is always rejected).
    SelfRef,
    /// A `for x in ...` loop variable. Always immutable. Bound fresh
    /// per iteration.
    ForIndex,
    /// An internal compiler-generated local (e.g. `@for_list`,
    /// `@for_idx`). Treated as immutable to prevent user code from
    /// accidentally referencing or shadowing them; not user-visible
    /// in errors because their names start with `@`.
    Internal,
}

impl BindingKind {
    /// Whether this binding can be reassigned via the `=` operator
    /// or have its data mutated through fields, indexes, or
    /// mutating methods.
    pub(super) fn is_mutable(&self) -> bool {
        matches!(
            self,
            BindingKind::Let | BindingKind::MutParam | BindingKind::SelfRef
        )
    }
}

/// One slot in the locals table. Replaces the previous
/// `(slot, mutable, obj_type)` tuple. The `kind` field is the
/// authoritative source for mutability — `is_mutable()` is derived,
/// never stored separately.
#[derive(Clone, Debug)]
pub(super) struct LocalEntry {
    pub(super) slot: usize,
    pub(super) kind: BindingKind,
    pub(super) obj_type: ResolvedType,
}

impl LocalEntry {
    pub(super) fn is_mutable(&self) -> bool {
        self.kind.is_mutable()
    }
}

/// Maps variable names to numeric slot indices during compilation.
/// The `obj_type` field tracks the variable's object type name
/// (if known), which enables compile-time field resolution. It's
/// populated from ObjLiteral assignments, variable-to-variable copies,
/// and typed function parameters.
#[derive(Clone)]
pub(super) struct Locals {
    slots: HashMap<String, LocalEntry>,
    pub(super) count: usize,
    pub(super) max_count: usize,
    pub(super) return_type: Option<ResolvedType>,
}

#[derive(Clone)]
pub(super) struct LocalsSnapshot {
    slots: HashMap<String, LocalEntry>,
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

    pub(super) fn define(
        &mut self,
        name: String,
        kind: BindingKind,
        obj_type: ResolvedType,
    ) -> usize {
        let slot = self.count;

        self.slots.insert(
            name,
            LocalEntry {
                slot,
                kind,
                obj_type,
            },
        );
        self.count += 1;
        self.max_count = self.max_count.max(self.count);

        slot
    }

    pub(super) fn resolve(&self, name: &str) -> Option<LocalEntry> {
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
    /// Whether each parameter is declared `mut`. Parallel to
    /// `param_types`. For methods, this does NOT include `self` —
    /// `self`'s mutability is decided by the method's own `is_mut`
    /// flag (the `mut fn` keyword), not by a parameter declaration.
    pub(crate) param_is_mut: Vec<bool>,
    /// Whether the function is a `mut fn` method. Always `false` for
    /// top-level functions and for plain `fn` methods. Used by the
    /// caller to enforce the val-receiver and non-mut-context rules.
    pub(crate) is_mut: bool,
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

// ---------------------------------------------------------------------------
// EnumTable
// ---------------------------------------------------------------------------

/// Compile-time registry of enum definitions. Parallel to ObjTable
/// but for enum types. The `names` HashMap stores **absolute**
/// enum_def indices (shifted by `base_offset`); the `defs` vector
/// is indexed locally. `resolve_variant` returns the absolute enum
/// index, the variant index within the enum, and the variant info,
/// so callers can emit `MakeEnum(enum_idx, variant_idx, ..)` directly.
#[derive(Clone)]
pub(super) struct EnumTable {
    pub(super) names: HashMap<String, usize>,
    pub(super) defs: Vec<EnumDefInfo>,
    pub(super) base_offset: usize,
}

impl EnumTable {
    pub(super) fn new(base_offset: usize) -> Self {
        Self {
            names: HashMap::new(),
            defs: Vec::new(),
            base_offset,
        }
    }

    pub(super) fn register(
        &mut self,
        name: String,
        variants: Vec<EnumVariantInfo>,
        is_pub: bool,
        is_error: bool,
    ) -> usize {
        let local = self.defs.len();
        let absolute = self.base_offset + local;
        self.names.insert(name.clone(), absolute);
        self.defs.push(EnumDefInfo {
            name,
            variants,
            is_pub,
            is_error,
        });
        absolute
    }

    /// Resolve an enum name to its absolute index and def.
    pub(super) fn resolve(&self, name: &str) -> Option<(usize, &EnumDefInfo)> {
        let absolute = *self.names.get(name)?;
        let local = absolute - self.base_offset;
        Some((absolute, &self.defs[local]))
    }

    /// Resolve `EnumName.VariantName` to a (enum_idx, variant_idx,
    /// variant_info) triple. Returns `None` if the enum doesn't
    /// exist or the variant isn't on it.
    pub(super) fn resolve_variant(
        &self,
        enum_name: &str,
        variant_name: &str,
    ) -> Option<(usize, usize, &EnumVariantInfo)> {
        let (enum_idx, def) = self.resolve(enum_name)?;
        let variant_idx = def.variants.iter().position(|v| v.name == variant_name)?;
        let variant_info = &def.variants[variant_idx];
        Some((enum_idx, variant_idx, variant_info))
    }
}
