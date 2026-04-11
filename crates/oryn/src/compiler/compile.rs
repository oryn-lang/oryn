use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::OrynError;
use crate::compiler::types::{EnumDefInfo, ModuleTable, ObjDefInfo, ResolvedType};
use crate::native::NativeRegistry;
use crate::parser::{Span, Spanned, Statement, TypeAnnotation};

use super::func::FunctionBodyConfig;
use super::obj::PreparedObjDef;
use super::tables::{BindingKind, EnumTable, FunctionSignature, FunctionTable, Locals, ObjTable};
use super::types::{CompiledFunction, CompilerOutput, Instruction};

// ---------------------------------------------------------------------------
// Loop tracking
// ---------------------------------------------------------------------------

pub(super) struct LoopContext {
    pub(super) continue_target: usize,
    pub(super) break_patches: Vec<usize>,
}

// ---------------------------------------------------------------------------
// Compiler state
// ---------------------------------------------------------------------------

pub(super) struct Compiler {
    pub(super) output: CompilerOutput,
    pub(super) fn_table: FunctionTable,
    pub(super) obj_table: ObjTable,
    /// Compile-time registry of enum declarations in this compilation
    /// unit. Cross-module enum imports are not supported in the
    /// initial enum slice, so this table always has `base_offset: 0`
    /// and lives entirely within the current compilation unit. The
    /// machinery is parallel to `obj_table` for future extension.
    pub(super) enum_table: EnumTable,
    pub(super) locals: Locals,
    pub(super) loops: Vec<LoopContext>,
    pub(super) modules: ModuleTable,
    /// Index of the first function this compilation unit will produce within
    /// the merged chunk. Used to convert local indices into absolute ones
    /// so module calls and entry-file calls share a single index space.
    pub(super) fn_base_offset: usize,
    /// Same as `fn_base_offset`, but for object definitions. Currently
    /// unused directly (the ObjTable tracks its own copy) but kept for
    /// symmetry and future lookups.
    #[allow(dead_code)]
    pub(super) obj_base_offset: usize,
    /// Dotted path of the module being compiled (e.g.
    /// `["math", "nested", "lib"]`). Empty for the entry file. Used to
    /// detect module-level context (for pub let/val extraction) and to
    /// determine privacy-boundary crossings.
    pub(super) current_module_path: Vec<String>,
    /// `true` while compiling the body of a `mut fn` method, `false`
    /// otherwise. Drives the rule that a plain `fn` method cannot
    /// call a `mut fn` method on `self` (or otherwise mutate `self`'s
    /// reachable state). Saved and restored at function-body
    /// boundaries by `compile_function_body`.
    pub(super) current_fn_is_mut: bool,
    /// Shared registry of native functions and methods. The compiler
    /// resolves source-level method calls and global function calls
    /// against this registry, baking the resulting indices into
    /// `CallNative` instructions. The same `Arc` is then handed to
    /// the `Chunk` so the VM can dispatch by index without needing
    /// to rebuild it.
    pub(super) native: Arc<NativeRegistry>,
    /// Names of obj defs that have been fully prepared (flattened
    /// fields, reserved method slots, signatures resolved). Phase A
    /// seeds placeholder entries in `obj_table` for every obj def in
    /// the module so that field-type annotations can forward-reference
    /// any named type, but `use` flattening must only read from
    /// parents that have been fully prepared — otherwise inherited
    /// fields/methods would come back empty. This set tracks which
    /// parents are safe to flatten from. Populated by
    /// [`super::obj::Compiler::prepare_obj_def`] as it finishes each
    /// obj def.
    pub(super) prepared_obj_names: HashSet<String>,
}

impl Compiler {
    fn new(
        fn_base_offset: usize,
        obj_base_offset: usize,
        current_module_path: Vec<String>,
        native: Arc<NativeRegistry>,
    ) -> Self {
        Self {
            output: CompilerOutput::default(),
            fn_table: FunctionTable::new(fn_base_offset),
            obj_table: ObjTable::new(obj_base_offset),
            enum_table: EnumTable::new(0),
            locals: Locals::new(),
            loops: Vec::new(),
            modules: ModuleTable::default(),
            fn_base_offset,
            obj_base_offset,
            current_module_path,
            current_fn_is_mut: false,
            native,
            prepared_obj_names: HashSet::new(),
        }
    }

    /// Seed a placeholder obj def in the obj_table and output so that
    /// forward references to this type (by name, in field annotations,
    /// or in parameter/return types) resolve successfully during
    /// Phase A. The placeholder is replaced in-place once
    /// [`super::obj::Compiler::prepare_obj_def`] runs for the obj.
    /// Pass the `is_pub` flag through so cross-module visibility checks
    /// see the right answer even before the real def lands.
    pub(super) fn seed_obj_placeholder(&mut self, name: String, is_pub: bool) {
        use std::collections::HashMap;
        self.obj_table.register(
            name.clone(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            Vec::new(),
            is_pub,
        );
        self.output.obj_defs.push(ObjDefInfo {
            name,
            fields: Vec::new(),
            field_types: Vec::new(),
            field_is_pub: Vec::new(),
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            method_is_pub: HashMap::new(),
            static_method_is_pub: HashMap::new(),
            method_signatures: HashMap::new(),
            static_method_signatures: HashMap::new(),
            signatures: Vec::new(),
            is_pub,
        });
        debug_assert_eq!(self.obj_table.defs.len(), self.output.obj_defs.len());
    }

    /// Seed a placeholder enum def in the enum_table and output so that
    /// forward references resolve during Phase A. Finalized in-place by
    /// [`Self::finalize_enum_def`] once variant payload types can be
    /// resolved against the fully-seeded type environment.
    pub(super) fn seed_enum_placeholder(&mut self, name: String, is_pub: bool, is_error: bool) {
        self.enum_table
            .register(name.clone(), Vec::new(), is_pub, is_error);
        self.output.enum_defs.push(EnumDefInfo {
            name,
            variants: Vec::new(),
            is_pub,
            is_error,
        });
        debug_assert_eq!(self.enum_table.defs.len(), self.output.enum_defs.len());
    }

    /// True if the current compilation unit is an imported module (not the
    /// entry file). Used to gate module-only behaviors like pub let/val
    /// constant extraction.
    pub(super) fn is_module(&self) -> bool {
        !self.current_module_path.is_empty()
    }

    /// Convert an absolute function index back to a local position in
    /// `self.output.functions` for indexing. Panics if `absolute` is below
    /// the compiler's base offset (indicates a cross-module index leaked
    /// into local lookup code, which would be a bug).
    pub(super) fn local_fn_idx(&self, absolute: usize) -> usize {
        absolute - self.fn_base_offset
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Compile a parsed AST into a [`CompilerOutput`].
///
/// `modules` is the set of imports already compiled and visible to this
/// unit. `fn_base_offset` and `obj_base_offset` shift every emitted
/// function/obj index so that the resulting bytecode lines up with the
/// merged chunk's absolute index space without a separate remapping pass.
/// `current_module_path` is empty for the entry file and the dotted
/// import path for an imported module — used to gate module-only
/// behaviors and to enforce cross-module privacy.
///
/// Compilation runs in two phases:
///
/// **Phase A (type environment)** seeds placeholder entries in the obj
/// and enum tables for every type declared in the module, then resolves
/// enum variant payload types and runs
/// [`Compiler::prepare_obj_def`] on every obj def. After Phase A the obj
/// table, enum table, and output's obj/enum def vectors all hold the
/// final field and method signature information for every declared type.
/// Method body bytecode has not been emitted yet, but every function
/// slot has been reserved so callers can emit `Call(abs_idx, arity)`
/// against a stable index. This pass gives field type annotations and
/// method signatures visibility into every type in the module, not just
/// those declared above them in source order.
///
/// **Phase B (bodies and module-level code)** compiles each obj def's
/// method bodies into their pre-allocated slots, then walks the
/// remaining top-level statements in source order so module-level
/// initializers (`let`/`val`), test blocks, and top-level expressions
/// preserve their execution order semantics.
pub(crate) fn compile(
    statements: Vec<Spanned<Statement>>,
    modules: ModuleTable,
    fn_base_offset: usize,
    obj_base_offset: usize,
    current_module_path: Vec<String>,
    native: Arc<NativeRegistry>,
) -> CompilerOutput {
    let mut c = Compiler::new(fn_base_offset, obj_base_offset, current_module_path, native);
    c.modules = modules;

    // ------------------------------------------------------------------
    // Phase A — seed type environment
    // ------------------------------------------------------------------

    // A0/A1: Walk statements once, bucket enum defs, obj defs, and
    // top-level functions for Phase A processing, and forward every
    // other statement to Phase B. Each enum/obj def also gets a
    // placeholder registered in its table so that forward references
    // from later Phase A sub-passes resolve by name.
    //
    // Duplicate top-level type or function names are rejected here.
    // Oryn has no overloading, so two `fn foo` declarations (or two
    // `struct Foo`, or two `enum Foo`) can never both be reachable —
    // under Phase A's name-keyed table seeding the second declaration
    // would silently shadow the first, producing surprising call
    // resolution at every earlier call site. Catch the collision now
    // and emit a clear error instead.
    let mut enum_preps: Vec<EnumPrep> = Vec::new();
    let mut obj_preps: Vec<ObjPrep> = Vec::new();
    let mut fn_data: Vec<FnData> = Vec::new();
    let mut phase_b_stmts: Vec<Spanned<Statement>> = Vec::new();
    let mut seen_type_names: HashSet<String> = HashSet::new();
    let mut seen_fn_names: HashSet<String> = HashSet::new();

    for stmt in statements {
        let Spanned { node, span } = stmt;
        match node {
            Statement::EnumDef {
                name,
                variants,
                is_pub,
                is_error,
            } => {
                if !seen_type_names.insert(name.clone()) {
                    c.output.errors.push(OrynError::compiler(
                        span,
                        format!("duplicate type declaration `{name}` in this module"),
                    ));
                    continue;
                }
                c.seed_enum_placeholder(name.clone(), is_pub, is_error);
                enum_preps.push(EnumPrep {
                    span,
                    name,
                    variants,
                    is_pub,
                    is_error,
                });
            }
            Statement::ObjDef {
                name,
                fields,
                methods,
                uses,
                is_pub,
            } => {
                if !seen_type_names.insert(name.clone()) {
                    c.output.errors.push(OrynError::compiler(
                        span,
                        format!("duplicate type declaration `{name}` in this module"),
                    ));
                    continue;
                }
                c.seed_obj_placeholder(name.clone(), is_pub);
                obj_preps.push(ObjPrep {
                    span,
                    name,
                    fields,
                    methods,
                    uses,
                    is_pub,
                });
            }
            Statement::Function {
                name,
                params,
                body,
                return_type,
                is_pub,
            } => {
                if !seen_fn_names.insert(name.clone()) {
                    c.output.errors.push(OrynError::compiler(
                        span,
                        format!("duplicate top-level function `{name}` in this module"),
                    ));
                    continue;
                }
                fn_data.push(FnData {
                    span,
                    name,
                    params,
                    body,
                    return_type,
                    is_pub,
                });
            }
            _ => phase_b_stmts.push(Spanned { node, span }),
        }
    }

    // A2: Resolve enum variant payload types into the seeded
    // placeholders. Cross-references between enums work because every
    // enum name is already in `enum_table` from the seeding pass.
    for ep in enum_preps {
        let EnumPrep {
            span,
            name,
            variants,
            is_pub,
            is_error,
        } = ep;
        c.finalize_enum_def(name, variants, &span, is_pub, is_error);
    }

    // A3: Topologically sort obj defs by their single-segment `use`
    // dependencies, then prepare each in dependency order. This lifts
    // the definition-order requirement for composition: `struct Guard {
    // use Health }` may appear anywhere in the module relative to
    // `struct Health`. Cycles (e.g. `A use B; B use A`) produce a
    // `use cycle` error for each member of the cycle and are skipped
    // in the prepare loop below — their placeholders stay empty.
    //
    // Cross-module `use` paths (multi-segment) never contribute to the
    // local DAG because their parents live in other compilation units
    // that are already fully compiled by the time this function runs.
    // Undefined single-segment `use` targets also don't contribute to
    // the DAG; prepare_obj_def's `prepared_obj_names` check emits the
    // "undefined type" error for those.
    let obj_count = obj_preps.len();
    let name_to_idx: HashMap<String, usize> = obj_preps
        .iter()
        .enumerate()
        .map(|(i, op)| (op.name.clone(), i))
        .collect();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); obj_count];
    let mut in_degree: Vec<usize> = vec![0; obj_count];
    for (idx, op) in obj_preps.iter().enumerate() {
        // A HashSet prevents a double-counted edge when the same obj
        // uses the same parent twice (which is itself a user error but
        // shouldn't corrupt in_degree counting).
        let mut seen: HashSet<usize> = HashSet::new();
        for used in &op.uses {
            if used.len() == 1
                && let Some(&parent_idx) = name_to_idx.get(&used[0])
                && parent_idx != idx
                && seen.insert(parent_idx)
            {
                children[parent_idx].push(idx);
                in_degree[idx] += 1;
            }
        }
    }

    // Kahn's algorithm.
    let mut queue: VecDeque<usize> = (0..obj_count).filter(|&i| in_degree[i] == 0).collect();
    let mut topo_order: Vec<usize> = Vec::with_capacity(obj_count);
    while let Some(idx) = queue.pop_front() {
        topo_order.push(idx);
        for &child in &children[idx] {
            in_degree[child] -= 1;
            if in_degree[child] == 0 {
                queue.push_back(child);
            }
        }
    }

    // Any obj still carrying a non-zero in-degree belongs to a cycle.
    // Emit a `use cycle` error for each, listing the other cyclic
    // parents the obj depends on so the message is actionable.
    if topo_order.len() < obj_count {
        let in_cycle: HashSet<usize> = (0..obj_count).filter(|&i| in_degree[i] > 0).collect();
        for &idx in &in_cycle {
            let op = &obj_preps[idx];
            let cyclic_parents: Vec<String> = op
                .uses
                .iter()
                .filter_map(|used| {
                    if used.len() != 1 {
                        return None;
                    }
                    let parent_idx = name_to_idx.get(&used[0])?;
                    if in_cycle.contains(parent_idx) {
                        Some(used[0].clone())
                    } else {
                        None
                    }
                })
                .collect();
            c.output.errors.push(OrynError::compiler(
                op.span.clone(),
                format!(
                    "object `{}` forms a `use` cycle with: {}",
                    op.name,
                    cyclic_parents.join(", ")
                ),
            ));
        }
    }

    // Prepare obj defs in topological order. Cyclic entries are left
    // untouched — their placeholder obj_table entries stay empty, and
    // Phase B2 skips them because they were never pushed into
    // `prepared_objs`.
    let mut obj_preps_opts: Vec<Option<ObjPrep>> = obj_preps.into_iter().map(Some).collect();
    let mut prepared_objs: Vec<PreparedObjDef> = Vec::with_capacity(topo_order.len());
    for idx in topo_order {
        let op = obj_preps_opts[idx]
            .take()
            .expect("topo_order should not visit an obj twice");
        let ObjPrep {
            span,
            name,
            fields,
            methods,
            uses,
            is_pub,
        } = op;
        let prepared = c.prepare_obj_def(name, fields, methods, uses, &span, is_pub);
        prepared_objs.push(prepared);
    }

    // A4: Pre-resolve each top-level function's parameter and return
    // types, reserve a placeholder `CompiledFunction` slot, and
    // register the function in `fn_table` with its full signature.
    // After this pass, any top-level function can be called from any
    // other top-level expression or body — source order no longer
    // matters for top-level function references.
    //
    // Slot reservation here also means that when Phase B compiles a
    // top-level expression like `let x = helper()`, the expression
    // compiler reads `helper`'s signature out of `fn_table` and emits
    // `Call(abs_idx, arity)` against the reserved slot. The slot's
    // bytecode is filled in later by the body-compile pass, but the
    // index is already stable.
    let mut fn_preps: Vec<FnPrep> = Vec::with_capacity(fn_data.len());
    for data in fn_data {
        let FnData {
            span,
            name,
            params,
            body,
            return_type,
            is_pub,
        } = data;

        // Resolve each parameter's type annotation. Missing annotations
        // are reported here and fall through as `Unknown` so the rest
        // of the function's signature still lines up.
        for param in &params {
            if param.type_ann.is_none() {
                c.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("parameter `{}` requires a type annotation", param.name),
                ));
            }
        }

        let resolved_params: HashMap<String, ResolvedType> = params
            .iter()
            .map(|p| {
                let t = p
                    .type_ann
                    .as_ref()
                    .map(|a| match c.resolve_type_annotation(a) {
                        Ok(t) => t,
                        Err(msg) => {
                            c.output.errors.push(OrynError::compiler(span.clone(), msg));
                            ResolvedType::Unknown
                        }
                    })
                    .unwrap_or(ResolvedType::Unknown);
                (p.name.clone(), t)
            })
            .collect();
        let param_types: Vec<ResolvedType> = params
            .iter()
            .map(|p| {
                resolved_params
                    .get(&p.name)
                    .cloned()
                    .unwrap_or(ResolvedType::Unknown)
            })
            .collect();
        let return_resolved = match &return_type {
            Some(rt) => match c.resolve_type_annotation(rt) {
                Ok(t) => t,
                Err(msg) => {
                    c.output.errors.push(OrynError::compiler(span.clone(), msg));
                    ResolvedType::Unknown
                }
            },
            None => ResolvedType::Unknown,
        };
        c.output.type_map.insert(span.clone(), &return_resolved);

        // Reserve the slot. The placeholder's `param_types` and
        // `return_type` are real — `expr.rs` reads them off the slot
        // when compiling call sites, and those reads must succeed
        // against the placeholder before the body bytecode exists.
        let local_idx = c.output.functions.len();
        let param_names_vec: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let param_is_mut_vec: Vec<bool> = params.iter().map(|p| p.is_mut).collect();
        c.output.functions.push(CompiledFunction {
            name: name.clone(),
            arity: params.len(),
            params: param_names_vec,
            param_types: param_types.clone(),
            param_is_mut: param_is_mut_vec.clone(),
            return_type: Some(return_resolved.clone()),
            num_locals: 0,
            instructions: Vec::new(),
            spans: Vec::new(),
            is_pub,
            is_mut: false,
        });
        c.fn_table.register(name.clone(), local_idx);
        c.fn_table.signatures.insert(
            name.clone(),
            FunctionSignature {
                param_types: param_types.clone(),
                return_type: return_resolved.clone(),
                param_is_mut: param_is_mut_vec,
                is_mut: false,
            },
        );

        fn_preps.push(FnPrep {
            span,
            name,
            params,
            body,
            is_pub,
            local_idx,
            param_types,
            resolved_params,
            return_resolved,
        });
    }

    // ------------------------------------------------------------------
    // Phase B — compile bodies and module-level code
    // ------------------------------------------------------------------

    // B1: Top-level statements in source order. These may reference
    // top-level functions (registered in A4), obj static/instance
    // methods (registered in A3), enum variants (registered in A2),
    // and module constants (registered here as the bindings execute).
    // Module-level execution order is preserved for let/val
    // initializers, tests, and top-level expressions — only type
    // declarations and top-level function definitions are hoisted out
    // of source order by Phase A.
    for stmt in phase_b_stmts {
        c.compile_stmt(stmt);
    }

    // B2: Compile top-level function bodies into their pre-allocated
    // slots. Bodies have visibility into every obj def, every other
    // top-level function, and every module constant registered by
    // this point.
    for fp in fn_preps {
        let FnPrep {
            span,
            name,
            params,
            body,
            is_pub,
            local_idx,
            param_types,
            resolved_params,
            return_resolved,
        } = fp;

        let param_fn = move |p: &crate::parser::Param| {
            let resolved = resolved_params
                .get(&p.name)
                .cloned()
                .unwrap_or(ResolvedType::Unknown);
            // `mut x: T` opts the parameter into mutability.
            // Without `mut`, top-level function params are always
            // immutable in Oryn (no opt-out at the call site).
            let kind = if p.is_mut {
                BindingKind::MutParam
            } else {
                BindingKind::Param
            };
            (kind, resolved)
        };

        c.compile_function_body(FunctionBodyConfig {
            name: &name,
            params: &params,
            param_types,
            param_local_fn: &param_fn,
            self_name: Some(&name),
            body,
            span: &span,
            return_type: Some(return_resolved),
            is_pub,
            is_mut: false,
            pre_allocated_local_idx: Some(local_idx),
        });
    }

    // B3: Compile obj method bodies into their pre-allocated slots.
    // Method bodies now have visibility into every declared type's
    // fields and method signatures, and into every top-level function
    // and module constant registered in B1.
    for prepared in prepared_objs {
        c.compile_obj_def_bodies(prepared);
    }

    c.output
}

/// Destructured `Statement::EnumDef` captured in Phase A0 for
/// finalization in Phase A2.
struct EnumPrep {
    span: Span,
    name: String,
    variants: Vec<crate::parser::EnumVariant>,
    is_pub: bool,
    is_error: bool,
}

/// Destructured `Statement::ObjDef` captured in Phase A0 for
/// preparation in Phase A3.
struct ObjPrep {
    span: Span,
    name: String,
    fields: Vec<crate::parser::ObjField>,
    methods: Vec<crate::parser::ObjMethod>,
    uses: Vec<Vec<String>>,
    is_pub: bool,
}

/// Destructured `Statement::Function` captured in Phase A0 before
/// type resolution. Phase A4 processes these into [`FnPrep`] entries
/// after all enum and obj types are available, so forward references
/// through function signatures resolve regardless of source order.
struct FnData {
    span: Span,
    name: String,
    params: Vec<crate::parser::Param>,
    body: Spanned<crate::parser::Expression>,
    return_type: Option<crate::parser::TypeAnnotation>,
    is_pub: bool,
}

/// Top-level function with its signature resolved and its function
/// slot reserved. Phase B2 walks these and compiles each body into
/// its pre-allocated slot via `compile_function_body`.
struct FnPrep {
    span: Span,
    name: String,
    params: Vec<crate::parser::Param>,
    body: Spanned<crate::parser::Expression>,
    is_pub: bool,
    local_idx: usize,
    param_types: Vec<ResolvedType>,
    resolved_params: HashMap<String, ResolvedType>,
    return_resolved: ResolvedType,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolves a type annotation against an ObjTable and a ModuleTable.
///
/// - Single-segment names like `int` or `Vec2` resolve as builtins or local
///   types via `obj_table`.
/// - Multi-segment names like `math.Vec2` resolve via `modules` — the prefix
///   names a module, the last segment names a type within that module.
pub(super) fn resolve_type(
    ann: &TypeAnnotation,
    obj_table: &ObjTable,
    enum_table: &EnumTable,
    modules: &ModuleTable,
) -> Result<ResolvedType, String> {
    match ann {
        TypeAnnotation::Named(path) => {
            if path.is_empty() {
                return Err("empty type path".to_string());
            }

            if path.len() == 1 {
                let name = &path[0];
                match name.as_str() {
                    "int" => Ok(ResolvedType::Int),
                    "float" => Ok(ResolvedType::Float),
                    "bool" => Ok(ResolvedType::Bool),
                    "string" => Ok(ResolvedType::Str),
                    "range" => Ok(ResolvedType::Range),
                    other => {
                        if obj_table.resolve(other).is_some() {
                            Ok(ResolvedType::Object {
                                name: other.to_string(),
                                module: vec![],
                            })
                        } else if let Some((_, def)) = enum_table.resolve(other) {
                            Ok(ResolvedType::Enum {
                                name: other.to_string(),
                                module: vec![],
                                is_error: def.is_error,
                            })
                        } else {
                            Err(format!("undefined type `{other}`"))
                        }
                    }
                }
            } else {
                // Multi-segment: split into module path and type name.
                let (type_name, module_path) = path.split_last().unwrap();
                let module_key = module_path.join(".");
                match modules.modules.get(&module_key) {
                    Some(exports) => {
                        if exports.obj_defs.contains_key(type_name) {
                            Ok(ResolvedType::Object {
                                name: type_name.clone(),
                                module: module_path.to_vec(),
                            })
                        } else if let Some(def) = exports.enum_defs.get(type_name) {
                            Ok(ResolvedType::Enum {
                                name: type_name.clone(),
                                module: module_path.to_vec(),
                                is_error: def.is_error,
                            })
                        } else {
                            Err(format!("undefined type `{module_key}.{type_name}`"))
                        }
                    }
                    None => Err(format!("undefined module `{module_key}`")),
                }
            }
        }
        TypeAnnotation::Nillable(inner) => {
            let inner_resolved = resolve_type(inner, obj_table, enum_table, modules)?;
            Ok(ResolvedType::Nillable(Box::new(inner_resolved)))
        }
        TypeAnnotation::ErrorUnion { error_enum, inner } => {
            let inner_resolved = resolve_type(inner, obj_table, enum_table, modules)?;
            // For the precise form (`error of E T`), resolve the
            // named error enum against the local table or the
            // imported module's exports. Validate that `E` was
            // declared with the `error` modifier — a plain enum
            // cannot be used here.
            let resolved_error_enum = match error_enum {
                None => None,
                Some(path) => {
                    if path.len() == 1 {
                        let name = &path[0];
                        match enum_table.resolve(name) {
                            Some((_, def)) if def.is_error => Some((name.clone(), Vec::new())),
                            Some(_) => {
                                return Err(format!(
                                    "`error ... of {name}` requires `{name}` to be declared as an `error enum`"
                                ));
                            }
                            None => {
                                return Err(format!("undefined error enum `{name}`"));
                            }
                        }
                    } else {
                        let (type_name, module_path) = path.split_last().unwrap();
                        let module_key = module_path.join(".");
                        match modules.modules.get(&module_key) {
                            Some(exports) => match exports.enum_defs.get(type_name) {
                                Some(def) if def.is_error => {
                                    Some((type_name.clone(), module_path.to_vec()))
                                }
                                Some(_) => {
                                    return Err(format!(
                                        "`error ... of {module_key}.{type_name}` requires `{type_name}` to be declared as an `error enum`"
                                    ));
                                }
                                None => {
                                    return Err(format!(
                                        "undefined error enum `{module_key}.{type_name}`"
                                    ));
                                }
                            },
                            None => return Err(format!("undefined module `{module_key}`")),
                        }
                    }
                }
            };
            Ok(ResolvedType::ErrorUnion {
                error_enum: resolved_error_enum,
                inner: Box::new(inner_resolved),
            })
        }
        TypeAnnotation::List(inner) => {
            let inner_resolved = resolve_type(inner, obj_table, enum_table, modules)?;
            Ok(ResolvedType::List(Box::new(inner_resolved)))
        }
        TypeAnnotation::Map(key, value) => {
            let key_resolved = resolve_type(key, obj_table, enum_table, modules)?;
            if !key_resolved.is_map_key_type() {
                return Err(format!(
                    "map key type must be `String`, `int`, or `bool`, got `{}`",
                    key_resolved.display_name()
                ));
            }
            let value_resolved = resolve_type(value, obj_table, enum_table, modules)?;
            Ok(ResolvedType::Map(
                Box::new(key_resolved),
                Box::new(value_resolved),
            ))
        }
        TypeAnnotation::Function {
            params,
            return_type,
        } => {
            let mut resolved_params: Vec<ResolvedType> = Vec::with_capacity(params.len());
            for p in params {
                resolved_params.push(resolve_type(p, obj_table, enum_table, modules)?);
            }
            // Default to `Nil` for void-returning functions, mirroring
            // how the rest of the compiler handles "no useful return".
            let resolved_return = match return_type {
                Some(rt) => resolve_type(rt, obj_table, enum_table, modules)?,
                None => ResolvedType::Nil,
            };
            Ok(ResolvedType::Function {
                params: resolved_params,
                return_type: Box::new(resolved_return),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Compiler helpers
// ---------------------------------------------------------------------------

impl Compiler {
    pub(super) fn emit(&mut self, instruction: Instruction, span: &Span) {
        self.output.instructions.push(instruction);
        self.output.spans.push(span.clone());
    }

    /// Resolve a field name to its index on an object type. Looks up
    /// the type in the local obj_table for in-module types, or in the
    /// imported `ModuleExports.obj_defs` for cross-module types.
    /// Enforces `pub` field visibility when crossing module boundaries.
    pub(super) fn resolve_field(
        &mut self,
        obj_type: &ResolvedType,
        field: &str,
        span: &Span,
    ) -> Option<usize> {
        let (type_name, type_module) = match obj_type {
            ResolvedType::Object { name, module } => (name.as_str(), module.clone()),
            _ => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    "cannot access field on non-object",
                ));
                return None;
            }
        };

        // Look up either locally (no module / same module) or via the
        // ModuleExports for an imported type.
        let crosses_module = !type_module.is_empty() && type_module != self.current_module_path;

        // Cloned ObjDefInfo for the lookup. We clone to avoid borrow
        // conflicts with self.output.errors below.
        let def: ObjDefInfo = if crosses_module {
            let module_key = type_module.join(".");
            match self
                .modules
                .modules
                .get(&module_key)
                .and_then(|e| e.obj_defs.get(type_name))
            {
                Some(d) => d.clone(),
                None => {
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!("undefined type `{module_key}.{type_name}`"),
                    ));
                    return None;
                }
            }
        } else {
            match self.obj_table.resolve(type_name) {
                Some((_, def)) => def.clone(),
                None => {
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!("undefined type `{type_name}`"),
                    ));
                    return None;
                }
            }
        };

        match def.fields.iter().position(|f| f == field) {
            Some(idx) => {
                // Cross-module access: enforce field privacy.
                if crosses_module && !def.field_is_pub.get(idx).copied().unwrap_or(false) {
                    let module_key = type_module.join(".");
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!("field `{field}` is private to module `{module_key}`"),
                    ));
                }
                Some(idx)
            }
            None => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("unknown field `{field}` on type `{type_name}`"),
                ));
                None
            }
        }
    }

    /// Resolves a type annotation against this compiler's obj_table and
    /// imported modules. Used for type-checking variable bindings,
    /// function parameters, and return types.
    ///
    /// When compiling a module, locally-defined object types are rewritten
    /// to carry the current module path so importers see them as
    /// cross-module references rather than as locals.
    pub(super) fn resolve_type_annotation(
        &self,
        ann: &TypeAnnotation,
    ) -> Result<ResolvedType, String> {
        let resolved = resolve_type(ann, &self.obj_table, &self.enum_table, &self.modules)?;
        Ok(self.attach_current_module(resolved))
    }

    /// If `ty` is `Object { module: [] }` and we're compiling a module,
    /// replace the empty module with the current module path so other
    /// compilation units can resolve the type back to its origin.
    pub(super) fn attach_current_module(&self, ty: ResolvedType) -> ResolvedType {
        if let ResolvedType::Object { name, module } = &ty
            && module.is_empty()
            && !self.current_module_path.is_empty()
        {
            return ResolvedType::Object {
                name: name.clone(),
                module: self.current_module_path.clone(),
            };
        }
        if let ResolvedType::Enum {
            name,
            module,
            is_error,
        } = &ty
            && module.is_empty()
            && !self.current_module_path.is_empty()
        {
            return ResolvedType::Enum {
                name: name.clone(),
                module: self.current_module_path.clone(),
                is_error: *is_error,
            };
        }
        if let ResolvedType::List(inner) = ty {
            return ResolvedType::List(Box::new(self.attach_current_module(*inner)));
        }
        if let ResolvedType::Map(key, value) = ty {
            return ResolvedType::Map(
                Box::new(self.attach_current_module(*key)),
                Box::new(self.attach_current_module(*value)),
            );
        }
        ty
    }

    pub(super) fn check_types(
        &mut self,
        expected: &ResolvedType,
        actual: &ResolvedType,
        span: &Span,
        message: &str,
    ) {
        if !expected.is_compatible_with(actual) {
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                format!(
                    "{}: expected `{}`, got `{}`",
                    message,
                    expected.display_name(),
                    actual.display_name()
                ),
            ));
        }
    }
}
