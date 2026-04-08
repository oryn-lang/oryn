use std::collections::HashMap;
use std::ops::Range;

use crate::OrynError;
use crate::parser::TypeAnnotation;

// ---------------------------------------------------------------------------
// Bytecode instructions
// ---------------------------------------------------------------------------

/// Flat bytecode that the VM executes. The compiler's job is to walk the
/// tree-shaped AST and flatten it into this linear sequence. The VM uses
/// a stack, so operand order matters - left before right.
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    PushBool(bool),
    PushFloat(f32),
    PushInt(i32),
    PushString(String),
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
    And,
    Or,
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
    /// Call a builtin function by name.
    CallBuiltin(String, usize),
    Pop,
    JumpIfFalse(usize),
    Jump(usize),
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
}

#[derive(Debug, Clone)]
pub struct ObjDefInfo {
    pub name: String,
    /// Field names in order - index = field offset.
    pub fields: Vec<String>,
    pub field_types: Vec<ResolvedType>,
    /// Method name -> function table index.
    pub methods: HashMap<String, usize>,
    /// Method signatures (declared without a body).
    /// Types that `use` this one must provide implementations.
    pub signatures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ResolvedType {
    Int,
    Float,
    Bool,
    Str,
    Void,
    Object(String),
    Unknown,
}

impl ResolvedType {
    pub fn from_annotation(ann: &crate::parser::TypeAnnotation) -> Self {
        match ann {
            crate::parser::TypeAnnotation::Named(n) => match n.as_str() {
                "i32" => ResolvedType::Int,
                "f32" => ResolvedType::Float,
                "bool" => ResolvedType::Bool,
                "String" => ResolvedType::Str,
                other => ResolvedType::Object(other.to_string()),
            },
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            ResolvedType::Int => "i32",
            ResolvedType::Float => "f32",
            ResolvedType::Bool => "bool",
            ResolvedType::Str => "String",
            ResolvedType::Void => "void",
            ResolvedType::Object(name) => name.as_str(),
            ResolvedType::Unknown => "unknown",
        }
    }
}

// ---------------------------------------------------------------------------
// Compile-time helper config
// ---------------------------------------------------------------------------

/// Callback that determines (mutable, obj_type) for each parameter.
pub(super) type ParamLocalFn = dyn Fn(&str, &Option<TypeAnnotation>) -> (bool, ResolvedType);

/// Configuration for compiling a function or method body.
pub(super) struct FunctionBodyConfig<'a> {
    pub name: &'a str,
    pub params: &'a [(String, Option<TypeAnnotation>)],
    pub param_types: Vec<ResolvedType>,
    pub param_local_fn: &'a ParamLocalFn,
    /// If Some, registers the function under this name for recursion.
    pub self_name: Option<&'a str>,
    pub body: crate::parser::Spanned<crate::parser::Expression>,
    pub return_type: Option<ResolvedType>,
    pub span: &'a crate::parser::Span,
}
