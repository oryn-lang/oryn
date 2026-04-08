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
    pub num_locals: usize,
    pub instructions: Vec<Instruction>,
    pub spans: Vec<Range<usize>>,
}

#[derive(Debug, Clone)]
pub struct ObjDefInfo {
    pub name: String,
    /// Field names in order - index = field offset.
    pub fields: Vec<String>,
    /// Method name -> function table index.
    pub methods: HashMap<String, usize>,
}

// ---------------------------------------------------------------------------
// Compile-time helper config
// ---------------------------------------------------------------------------

/// Callback that determines (mutable, obj_type) for each parameter.
pub(super) type ParamLocalFn = dyn Fn(&str, &Option<TypeAnnotation>) -> (bool, Option<String>);

/// Configuration for compiling a function or method body.
pub(super) struct FunctionBodyConfig<'a> {
    pub name: &'a str,
    pub params: &'a [(String, Option<TypeAnnotation>)],
    pub param_local_fn: &'a ParamLocalFn,
    /// If Some, registers the function under this name for recursion.
    pub self_name: Option<&'a str>,
    pub body: crate::parser::Spanned<crate::parser::Expression>,
    pub span: &'a crate::parser::Span,
}
