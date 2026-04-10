use crate::OrynError;
use crate::compiler::types::{ModuleTable, ObjDefInfo, ResolvedType};
use crate::parser::{Span, Spanned, Statement, TypeAnnotation};

use super::tables::{FunctionSignature, FunctionTable, Locals, ObjTable};
use super::types::{CompilerOutput, Instruction};

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
}

impl Compiler {
    fn new(
        fn_base_offset: usize,
        obj_base_offset: usize,
        current_module_path: Vec<String>,
    ) -> Self {
        Self {
            output: CompilerOutput::default(),
            fn_table: FunctionTable::new(fn_base_offset),
            obj_table: ObjTable::new(obj_base_offset),
            locals: Locals::new(),
            loops: Vec::new(),
            modules: ModuleTable::default(),
            fn_base_offset,
            obj_base_offset,
            current_module_path,
        }
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
pub(crate) fn compile(
    statements: Vec<Spanned<Statement>>,
    modules: ModuleTable,
    fn_base_offset: usize,
    obj_base_offset: usize,
    current_module_path: Vec<String>,
) -> CompilerOutput {
    let mut c = Compiler::new(fn_base_offset, obj_base_offset, current_module_path);
    c.modules = modules;

    for stmt in statements {
        let fn_count_before = c.output.functions.len();
        let obj_count_before = c.output.obj_defs.len();

        c.compile_stmt(stmt);
        c.sync_tables(fn_count_before, obj_count_before);
    }

    c.output
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolves a type annotation against an ObjTable and a ModuleTable.
///
/// - Single-segment names like `i32` or `Vec2` resolve as builtins or local
///   types via `obj_table`.
/// - Multi-segment names like `math.Vec2` resolve via `modules` — the prefix
///   names a module, the last segment names a type within that module.
pub(super) fn resolve_type(
    ann: &TypeAnnotation,
    obj_table: &ObjTable,
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
                    "i32" => Ok(ResolvedType::Int),
                    "f32" => Ok(ResolvedType::Float),
                    "bool" => Ok(ResolvedType::Bool),
                    "String" => Ok(ResolvedType::Str),
                    "Range" => Ok(ResolvedType::Range),
                    other => {
                        if obj_table.resolve(other).is_some() {
                            Ok(ResolvedType::Object {
                                name: other.to_string(),
                                module: vec![],
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
                        } else {
                            Err(format!("undefined type `{module_key}.{type_name}`"))
                        }
                    }
                    None => Err(format!("undefined module `{module_key}`")),
                }
            }
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
        let resolved = resolve_type(ann, &self.obj_table, &self.modules)?;
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
        ty
    }

    pub(super) fn check_types(
        &mut self,
        expected: &ResolvedType,
        actual: &ResolvedType,
        span: &Span,
        message: &str,
    ) {
        if *expected != ResolvedType::Unknown
            && *actual != ResolvedType::Unknown
            && expected != actual
        {
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

    /// Register newly compiled functions and objects in the lookup tables
    /// so subsequent statements can reference them.
    fn sync_tables(&mut self, fn_count_before: usize, obj_count_before: usize) {
        for i in fn_count_before..self.output.functions.len() {
            let func = &self.output.functions[i];
            self.fn_table.register(func.name.clone(), i);

            if let Some(ref rt) = func.return_type {
                self.fn_table.signatures.insert(
                    func.name.clone(),
                    FunctionSignature {
                        param_types: func.param_types.clone(),
                        return_type: rt.clone(),
                    },
                );
            }
        }

        for i in obj_count_before..self.output.obj_defs.len() {
            self.obj_table.register(
                self.output.obj_defs[i].name.clone(),
                self.output.obj_defs[i].fields.clone(),
                self.output.obj_defs[i].field_types.clone(),
                self.output.obj_defs[i].field_is_pub.clone(),
                self.output.obj_defs[i].methods.clone(),
                self.output.obj_defs[i].static_methods.clone(),
                self.output.obj_defs[i].method_is_pub.clone(),
                self.output.obj_defs[i].static_method_is_pub.clone(),
                self.output.obj_defs[i].method_signatures.clone(),
                self.output.obj_defs[i].static_method_signatures.clone(),
                self.output.obj_defs[i].signatures.clone(),
                self.output.obj_defs[i].is_pub,
            );
        }
    }
}
