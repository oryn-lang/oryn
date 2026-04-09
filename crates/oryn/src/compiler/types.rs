use std::collections::HashMap;
use std::ops::Range;

use crate::OrynError;

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
    /// Full method signatures (declared without a body).
    /// Types that `use` this one must provide implementations
    /// matching the complete shape (name, params, return type).
    pub signatures: Vec<MethodSignature>,
}

/// A required method signature: name + parameter types (excluding self) + return type.
#[derive(Debug, Clone)]
pub struct MethodSignature {
    pub name: String,
    /// Parameter types in order, excluding `self`.
    pub param_types: Vec<ResolvedType>,
    pub return_type: ResolvedType,
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
