use crate::OrynError;
use crate::compiler::types::ResolvedType;
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
}

impl Compiler {
    fn new() -> Self {
        Self {
            output: CompilerOutput::default(),
            fn_table: FunctionTable::new(),
            obj_table: ObjTable::new(),
            locals: Locals::new(),
            loops: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub(crate) fn compile(statements: Vec<Spanned<Statement>>) -> CompilerOutput {
    let mut c = Compiler::new();

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

/// Resolves a type annotation against an ObjTable. Used by both the Compiler
/// method and obj.rs (which needs a temporarily-extended table).
pub(super) fn resolve_type(
    ann: &TypeAnnotation,
    obj_table: &ObjTable,
) -> Result<ResolvedType, String> {
    match ann {
        TypeAnnotation::Named(name) => match name.as_str() {
            "i32" => Ok(ResolvedType::Int),
            "f32" => Ok(ResolvedType::Float),
            "bool" => Ok(ResolvedType::Bool),
            "String" => Ok(ResolvedType::Str),
            "Range" => Ok(ResolvedType::Range),
            other => {
                if obj_table.resolve(other).is_some() {
                    Ok(ResolvedType::Object(other.to_string()))
                } else {
                    Err(format!("undefined type `{other}`"))
                }
            }
        },
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

    /// Resolve a field name to its index on an object type.
    pub(super) fn resolve_field(
        &mut self,
        obj_type: &ResolvedType,
        field: &str,
        span: &Span,
    ) -> Option<usize> {
        let type_name = match obj_type {
            ResolvedType::Object(name) => name.as_str(),
            _ => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    "cannot access field on non-object",
                ));
                return None;
            }
        };

        let (_, def) = match self.obj_table.resolve(type_name) {
            Some(pair) => pair,
            None => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("undefined type `{type_name}`"),
                ));
                return None;
            }
        };

        match def.fields.iter().position(|f| f == field) {
            Some(idx) => Some(idx),
            None => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("unknown field `{field}` on type `{type_name}`"),
                ));
                None
            }
        }
    }

    /// Resolves a type annotation against this compiler's obj_table.
    pub(super) fn resolve_type_annotation(
        &self,
        ann: &TypeAnnotation,
    ) -> Result<ResolvedType, String> {
        resolve_type(ann, &self.obj_table)
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
                self.output.obj_defs[i].methods.clone(),
                self.output.obj_defs[i].static_methods.clone(),
                self.output.obj_defs[i].signatures.clone(),
            );
        }
    }
}
