use std::collections::HashMap;
use std::ops::Range;

use crate::OrynError;
use crate::compiler::tables::FunctionSignature;

// ---------------------------------------------------------------------------
// Bytecode instructions
// ---------------------------------------------------------------------------

/// Flat bytecode that the VM executes. The compiler's job is to walk the
/// tree-shaped AST and flatten it into this linear sequence. The VM uses
/// a stack, so operand order matters - left before right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinFunction {
    Print,
}

impl BuiltinFunction {
    pub fn name(self) -> &'static str {
        match self {
            BuiltinFunction::Print => "print",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    PushBool(bool),
    PushFloat(f32),
    PushInt(i32),
    PushString(String),
    ToString,
    Concat(u8),
    MakeRange(bool),
    GetLocal(usize),
    SetLocal(usize),
    NewObject(usize, usize),
    GetField(usize),
    SetField(usize),
    Return,
    Equal,
    NotEqual,
    LessThan,
    GreaterThan,
    LessThanEquals,
    GreaterThanEquals,
    Not,
    Negate,
    Add,
    Sub,
    Mul,
    Div,
    /// Call a user-defined function by index into the function table.
    Call(usize, usize),
    /// Call a method by name on an object.
    CallMethod(String, usize),
    /// Call a builtin function identified at compile time.
    CallBuiltin(BuiltinFunction, usize),
    Pop,
    JumpIfFalse(usize),
    Jump(usize),
    RangeHasNext,
    RangeNext,
    /// Push the nil value onto the stack.
    PushNil,
    /// Peek at TOS: if Nil, pop it and jump to target; if not Nil, leave it on the stack.
    JumpIfNil(usize),
    /// Peek at TOS: if Error, leave it on stack and jump to target; if not Error, leave it and fall through.
    JumpIfError(usize),
    /// Peek at TOS: if Error, produce a fatal runtime trap with the error message.
    /// If not Error, leave the value on the stack (it's the success value).
    UnwrapErrorOrTrap,
    /// Pop a String from the stack and push a `Value::Error(...)`.
    MakeError,
}

// ---------------------------------------------------------------------------
// Compiler output
// ---------------------------------------------------------------------------

/// Compiled output: instructions paired with a parallel span table.
#[derive(Default)]
pub struct CompilerOutput {
    pub instructions: Vec<Instruction>,
    pub spans: Vec<Range<usize>>,
    pub functions: Vec<CompiledFunction>,
    pub obj_defs: Vec<ObjDefInfo>,
    pub errors: Vec<OrynError>,
    /// Module-level `pub let` / `pub val` constants, extracted when compiling
    /// a module. Only non-empty for module compilation units; consumers
    /// access these via the owning module's [`ModuleExports`].
    pub(crate) module_constants: HashMap<String, ConstValue>,
    /// Module-level non-pub `let` / `val` constants. Visible to code inside
    /// the same module (functions and methods) but not exported via
    /// [`ModuleExports`] — callers importing the module cannot see them.
    pub(crate) private_module_constants: HashMap<String, ConstValue>,
    /// Span → type lookup populated during compilation and consumed by
    /// tools (LSP hover / inlay hints). See [`TypeMap`].
    pub type_map: TypeMap,
}

/// Span → type lookup populated while compiling a source file. Keys are
/// the `Spanned<Statement>.span` of each declaration (let/val binding,
/// function, obj method) so tooling can look up inferred types by the
/// same span the parser assigned — the LSP's [`SymbolInfo::full_span`]
/// uses the exact same value.
///
/// Values are pretty-printed type names (`"int"`, `"math.vec2.Vec2"`)
/// rather than the internal [`ResolvedType`] enum, which keeps the
/// public API stable while letting the LSP render hovers without
/// peeking at compiler internals.
#[derive(Debug, Default, Clone)]
pub struct TypeMap {
    by_span: HashMap<Range<usize>, String>,
}

impl TypeMap {
    /// Look up the resolved type at `span`. Returns `None` when the
    /// compiler didn't record anything (e.g. inference gave up and
    /// fell back to [`ResolvedType::Unknown`]).
    pub fn get(&self, span: &Range<usize>) -> Option<&str> {
        self.by_span.get(span).map(String::as_str)
    }

    /// True when no types were recorded for this compilation unit.
    pub fn is_empty(&self) -> bool {
        self.by_span.is_empty()
    }

    /// Record a type for a declaration span. Silently ignores
    /// [`ResolvedType::Unknown`] so consumers can distinguish "the
    /// compiler has an answer" from "no information".
    pub(crate) fn insert(&mut self, span: Range<usize>, ty: &ResolvedType) {
        if matches!(ty, ResolvedType::Unknown) {
            return;
        }
        self.by_span.insert(span, ty.display_name().into_owned());
    }
}

#[derive(Debug)]
pub struct CompiledFunction {
    pub name: String,
    pub arity: usize,
    pub params: Vec<String>,
    pub param_types: Vec<ResolvedType>,
    pub return_type: Option<ResolvedType>,
    pub num_locals: usize,
    pub instructions: Vec<Instruction>,
    pub spans: Vec<Range<usize>>,
    pub is_pub: bool,
}

/// Compile-time information about an object type. Stored in the
/// compiler's obj_table for in-module lookups, and cloned into
/// [`ModuleExports::obj_defs`] when the type is `pub`. Carries enough
/// information for the importing module to type-check field and method
/// access without re-parsing the source.
#[derive(Debug, Clone)]
pub struct ObjDefInfo {
    pub name: String,
    /// Field names in order — index = field offset in `NewObject`.
    pub fields: Vec<String>,
    pub field_types: Vec<ResolvedType>,
    /// Parallel to `fields`: whether each field is `pub` (visible across
    /// module boundaries). Inherited fields take their visibility from
    /// the originating type.
    pub field_is_pub: Vec<bool>,
    /// Method name -> function table index.
    pub methods: HashMap<String, usize>,
    /// Static method name -> function table index.
    pub static_methods: HashMap<String, usize>,
    /// Per-method visibility, parallel to `methods`.
    pub method_is_pub: HashMap<String, bool>,
    /// Per-static-method visibility, parallel to `static_methods`.
    pub static_method_is_pub: HashMap<String, bool>,
    /// Compiled signature (param types + return type) for each instance
    /// method. Used for cross-module method dispatch type inference and
    /// argument type checking.
    pub method_signatures: HashMap<String, FunctionSignature>,
    /// Same as `method_signatures` but for static methods.
    pub static_method_signatures: HashMap<String, FunctionSignature>,
    /// Full method signatures (declared without a body).
    /// Types that `use` this one must provide implementations
    /// matching the complete shape (name, params, return type).
    pub signatures: Vec<MethodSignature>,
    pub is_pub: bool,
}

/// A required method signature: name + parameter types (excluding self) + return type.
#[derive(Debug, Clone)]
pub struct MethodSignature {
    pub name: String,
    pub is_static: bool,
    /// Parameter types in order, excluding `self`.
    pub param_types: Vec<ResolvedType>,
    pub return_type: ResolvedType,
}

/// A module-level constant value, used for `pub let` / `pub val` bindings
/// that are exposed to importers. Only literal values are allowed for now;
/// non-literal expressions produce a compile error during module compilation.
#[derive(Clone)]
pub(crate) enum ConstValue {
    Int(i32),
    Float(f32),
    Bool(bool),
    String(String),
}

impl ConstValue {
    /// Emit the appropriate push instruction for this constant.
    pub(crate) fn to_instruction(&self) -> Instruction {
        match self {
            ConstValue::Int(n) => Instruction::PushInt(*n),
            ConstValue::Float(n) => Instruction::PushFloat(*n),
            ConstValue::Bool(b) => Instruction::PushBool(*b),
            ConstValue::String(s) => Instruction::PushString(s.clone()),
        }
    }

    /// The type of this constant, for type checking.
    pub(crate) fn resolved_type(&self) -> ResolvedType {
        match self {
            ConstValue::Int(_) => ResolvedType::Int,
            ConstValue::Float(_) => ResolvedType::Float,
            ConstValue::Bool(_) => ResolvedType::Bool,
            ConstValue::String(_) => ResolvedType::Str,
        }
    }
}

/// The pub-only surface area of a single compiled module, indexed by
/// the merged chunk's absolute indices. Importers look up cross-module
/// references through this struct rather than walking the merged
/// `CompilerOutput` directly.
#[derive(Clone)]
pub(crate) struct ModuleExports {
    /// `pub fn` name → absolute index in the merged chunk's function table.
    pub functions: HashMap<String, usize>,
    /// Compiled signatures (param types + return type) for each pub
    /// function, used for cross-module type checking.
    pub fn_signatures: HashMap<String, FunctionSignature>,
    /// `pub obj` name → absolute index in the merged chunk's obj table.
    pub objects: HashMap<String, usize>,
    /// Full `ObjDefInfo` (cloned) for each pub type. Lets importers
    /// enforce per-field/per-method privacy and construct qualified
    /// object literals without consulting the merged chunk directly.
    pub obj_defs: HashMap<String, ObjDefInfo>,
    /// `pub let` / `pub val` literal constants, inlined at the call site.
    pub constants: HashMap<String, ConstValue>,
}

/// All imported modules, keyed by their **full dot-joined path**
/// (e.g. "math" or "math.nested.library"). Registration uses the complete
/// path so the compiler can distinguish between flat and nested modules.
#[derive(Default)]
pub(crate) struct ModuleTable {
    pub modules: HashMap<String, ModuleExports>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ResolvedType {
    Int,
    Float,
    Bool,
    Str,
    Range,
    /// An object/struct type. `name` is the type's local name; `module`
    /// is the dotted path of the module that defined it (empty when the
    /// type was defined in the current compilation unit). The pair lets
    /// the compiler enforce cross-module field/method privacy.
    Object {
        name: String,
        module: Vec<String>,
    },
    /// `T?` — a nillable type wrapping an inner type.
    Nillable(Box<ResolvedType>),
    /// `!T` — an error union type wrapping an inner success type.
    ErrorUnion(Box<ResolvedType>),
    /// Internal-only nil type. Used for contextual typing of the `nil`
    /// literal. Not user-declarable.
    Nil,
    /// Internal-only error type. Used for contextual typing of the
    /// `Error(...)` constructor expression. Not user-declarable.
    Error,
    Unknown,
}

impl CompilerOutput {
    /// Build a [`ModuleExports`] from this output, including only `pub` items.
    /// Indices are remapped by the given offsets so they point into a merged output.
    pub(crate) fn build_module_exports(
        &self,
        fn_offset: usize,
        obj_offset: usize,
    ) -> ModuleExports {
        let mut functions = HashMap::new();
        let mut fn_signatures = HashMap::new();
        let mut objects = HashMap::new();

        for (i, func) in self.functions.iter().enumerate() {
            if func.is_pub {
                let remapped = fn_offset + i;
                functions.insert(func.name.clone(), remapped);
                if let Some(ref rt) = func.return_type {
                    fn_signatures.insert(
                        func.name.clone(),
                        FunctionSignature {
                            param_types: func.param_types.clone(),
                            return_type: rt.clone(),
                        },
                    );
                }
            }
        }

        let mut obj_defs = HashMap::new();
        for (i, obj_def) in self.obj_defs.iter().enumerate() {
            if obj_def.is_pub {
                objects.insert(obj_def.name.clone(), obj_offset + i);
                obj_defs.insert(obj_def.name.clone(), obj_def.clone());
            }
        }

        ModuleExports {
            functions,
            fn_signatures,
            objects,
            obj_defs,
            constants: self.module_constants.clone(),
        }
    }
}

impl ResolvedType {
    pub fn display_name(&self) -> std::borrow::Cow<'_, str> {
        match self {
            ResolvedType::Int => "int".into(),
            ResolvedType::Float => "float".into(),
            ResolvedType::Bool => "bool".into(),
            ResolvedType::Str => "String".into(),
            ResolvedType::Range => "Range".into(),
            ResolvedType::Object { name, module } => {
                if module.is_empty() {
                    name.as_str().into()
                } else {
                    format!("{}.{}", module.join("."), name).into()
                }
            }
            ResolvedType::Nillable(inner) => format!("{}?", inner.display_name()).into(),
            ResolvedType::ErrorUnion(inner) => format!("!{}", inner.display_name()).into(),
            ResolvedType::Nil => "nil".into(),
            ResolvedType::Error => "error".into(),
            ResolvedType::Unknown => "unknown".into(),
        }
    }

    /// Returns `true` if this is a `Nillable` type.
    pub(crate) fn is_nillable(&self) -> bool {
        matches!(self, ResolvedType::Nillable(_))
    }

    /// If this is `Nillable(T)`, returns `Some(&T)`. Otherwise `None`.
    pub(crate) fn unwrap_nillable(&self) -> Option<&ResolvedType> {
        match self {
            ResolvedType::Nillable(inner) => Some(inner),
            _ => None,
        }
    }

    /// Returns `true` if this is an `ErrorUnion` type.
    pub(crate) fn is_error_union(&self) -> bool {
        matches!(self, ResolvedType::ErrorUnion(_))
    }

    /// If this is `ErrorUnion(T)`, returns `Some(&T)`. Otherwise `None`.
    pub(crate) fn unwrap_error_union(&self) -> Option<&ResolvedType> {
        match self {
            ResolvedType::ErrorUnion(inner) => Some(inner),
            _ => None,
        }
    }

    /// Check whether `actual` is assignment-compatible with `self` as the
    /// expected type. This is more nuanced than simple equality:
    ///
    /// - `Unknown` is compatible with anything (inference gap).
    /// - `Nil` is compatible with any `Nillable(_)`.
    /// - `T` is compatible with `Nillable(T)` (value promotion).
    /// - `Error` is compatible with any `ErrorUnion(_)`.
    /// - `T` is compatible with `ErrorUnion(T)` (success value promotion).
    /// - Otherwise, structural equality is required.
    pub(crate) fn is_compatible_with(&self, actual: &ResolvedType) -> bool {
        // Unknown is a wildcard in both directions.
        if matches!(self, ResolvedType::Unknown) || matches!(actual, ResolvedType::Unknown) {
            return true;
        }

        // Exact match.
        if self == actual {
            return true;
        }

        // Nil → T? (nil literal assigned to nillable).
        if matches!(actual, ResolvedType::Nil) && self.is_nillable() {
            return true;
        }

        // T → T? (value promotion into nillable).
        if let ResolvedType::Nillable(inner) = self
            && inner.as_ref() == actual
        {
            return true;
        }

        // Error → !T (error constructor assigned to error union).
        if matches!(actual, ResolvedType::Error) && self.is_error_union() {
            return true;
        }

        // T → !T (success value promotion into error union).
        if let ResolvedType::ErrorUnion(inner) = self
            && inner.as_ref() == actual
        {
            return true;
        }

        false
    }
}
