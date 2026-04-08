use std::collections::HashMap;
use std::ops::Range;

use crate::{
    OrynError,
    parser::{BinOp, Expression, Span, Spanned, Statement, UnaryOp},
};

// Flat bytecode that the VM executes. The compiler's job is to walk the
// tree-shaped AST and flatten it into this linear sequence. The VM uses
// a stack, so operand order matters - left before right.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Instruction {
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
    // Call a user-defined function by index into the function table.
    Call(usize, usize),
    // Call a builtin function by name.
    CallBuiltin(String, usize),
    Pop,
    JumpIfFalse(usize),
    Jump(usize),
}

/// Compiled output: instructions paired with a parallel span table.
pub(crate) struct CompilerOutput {
    pub instructions: Vec<Instruction>,
    pub spans: Vec<Range<usize>>,
    pub functions: Vec<CompiledFunction>,
    pub obj_defs: Vec<ObjDefInfo>,
    pub errors: Vec<OrynError>,
}

struct LoopContext {
    start: usize,
    break_patches: Vec<usize>,
}

/// Maps variable names to numeric slot indices during compilation.
/// The third tuple element tracks the variable's object type name
/// (if known), which enables compile-time field resolution. It's
/// populated from ObjLiteral assignments, variable-to-variable copies,
/// and typed function parameters.
struct Locals {
    // (slot, mutable, obj_type).
    slots: HashMap<String, (usize, bool, Option<String>)>,
    count: usize,
}

impl Locals {
    fn new() -> Self {
        Self {
            slots: HashMap::new(),
            count: 0,
        }
    }

    fn define(&mut self, name: String, mutable: bool, obj_type: Option<String>) -> usize {
        let slot = self.count;

        self.slots.insert(name, (slot, mutable, obj_type));
        self.count += 1;

        slot
    }

    fn resolve(&self, name: &str) -> Option<(usize, bool, Option<String>)> {
        self.slots.get(name).cloned()
    }
}

/// Maps function names to their index in the function table.
/// Separate from the function table itself so we can look up
/// indices without borrowing the output.
struct FunctionTable {
    names: HashMap<String, usize>,
}

impl FunctionTable {
    fn new() -> Self {
        Self {
            names: HashMap::new(),
        }
    }

    fn register(&mut self, name: String, idx: usize) {
        self.names.insert(name, idx);
    }

    fn resolve(&self, name: &str) -> Option<usize> {
        self.names.get(name).copied()
    }
}

#[derive(Debug)]
pub(crate) struct CompiledFunction {
    pub name: String,
    pub arity: usize,
    pub params: Vec<String>,
    pub num_locals: usize,
    pub instructions: Vec<Instruction>,
    pub spans: Vec<Range<usize>>,
}

#[derive(Debug, Clone)]
pub(crate) struct ObjDefInfo {
    pub name: String,
    // Field names in order - index = field offset.
    pub fields: Vec<String>,
}

/// Compile-time registry of object definitions. Parallel to FunctionTable
/// but for types. Maps type names to their field layouts so the compiler
/// can resolve field accesses to integer indices without runtime lookups.
struct ObjTable {
    names: HashMap<String, usize>,
    defs: Vec<ObjDefInfo>,
}

impl ObjTable {
    fn new() -> Self {
        Self {
            names: HashMap::new(),
            defs: Vec::new(),
        }
    }

    fn register(&mut self, name: String, fields: Vec<String>) -> usize {
        let idx = self.defs.len();

        self.names.insert(name.clone(), idx);
        self.defs.push(ObjDefInfo { name, fields });

        idx
    }

    fn resolve(&self, name: &str) -> Option<(usize, &ObjDefInfo)> {
        let idx = *self.names.get(name)?;

        Some((idx, &self.defs[idx]))
    }
}

pub(crate) fn compile(statements: Vec<Spanned<Statement>>) -> CompilerOutput {
    let mut output = CompilerOutput {
        instructions: Vec::new(),
        spans: Vec::new(),
        functions: Vec::new(),
        obj_defs: Vec::new(),
        errors: Vec::new(),
    };

    let mut loops: Vec<LoopContext> = Vec::new();
    let mut locals = Locals::new();
    let mut fn_table = FunctionTable::new();
    let mut obj_table = ObjTable::new();

    for stmt in statements {
        let fn_count_before = output.functions.len();
        let obj_count_before = output.obj_defs.len();

        compile_statement(
            &mut output,
            &fn_table,
            &obj_table,
            &mut loops,
            &mut locals,
            stmt,
        );

        // If a new function was added, register it in the lookup table
        // so subsequent statements can call it.
        for i in fn_count_before..output.functions.len() {
            fn_table.register(output.functions[i].name.clone(), i);
        }

        // Same for object definitions.
        for i in obj_count_before..output.obj_defs.len() {
            obj_table.register(
                output.obj_defs[i].name.clone(),
                output.obj_defs[i].fields.clone(),
            );
        }
    }

    output
}

fn emit(output: &mut CompilerOutput, instruction: Instruction, span: &Span) {
    output.instructions.push(instruction);
    output.spans.push(span.clone());
}

/// Resolve a field name to its index on an object type. Returns the index
/// on success, or pushes a compiler error and returns None.
fn resolve_field(
    output: &mut CompilerOutput,
    obj_table: &ObjTable,
    obj_type: &Option<String>,
    field: &str,
    span: &Span,
) -> Option<usize> {
    let type_name = match obj_type {
        Some(name) => name,
        None => {
            output.errors.push(OrynError::Compiler {
                span: span.clone(),
                message: "cannot access field on non-object".into(),
            });
            return None;
        }
    };

    let (_, def) = match obj_table.resolve(type_name) {
        Some(pair) => pair,
        None => {
            output.errors.push(OrynError::Compiler {
                span: span.clone(),
                message: format!("undefined type `{type_name}`"),
            });
            return None;
        }
    };

    match def.fields.iter().position(|f| f == field) {
        Some(idx) => Some(idx),
        None => {
            output.errors.push(OrynError::Compiler {
                span: span.clone(),
                message: format!("unknown field `{field}` on type `{type_name}`"),
            });
            None
        }
    }
}

fn compile_statement(
    output: &mut CompilerOutput,
    fn_table: &FunctionTable,
    obj_table: &ObjTable,
    loops: &mut Vec<LoopContext>,
    locals: &mut Locals,
    stmt: Spanned<Statement>,
) {
    let stmt_span = stmt.span.clone();

    match stmt.node {
        Statement::Let { name, value, .. } => {
            let obj_type = match &value.node {
                Expression::ObjLiteral { type_name, .. } => Some(type_name.clone()),
                Expression::Ident(src) => locals.resolve(src).and_then(|(_, _, t)| t),
                _ => None,
            };

            compile_expression(output, fn_table, obj_table, locals, value);

            let slot = locals.define(name, true, obj_type);

            emit(output, Instruction::SetLocal(slot), &stmt_span);
        }
        Statement::Val { name, value, .. } => {
            let obj_type = match &value.node {
                Expression::ObjLiteral { type_name, .. } => Some(type_name.clone()),
                Expression::Ident(src) => locals.resolve(src).and_then(|(_, _, t)| t),
                _ => None,
            };

            compile_expression(output, fn_table, obj_table, locals, value);

            let slot = locals.define(name, false, obj_type);

            emit(output, Instruction::SetLocal(slot), &stmt_span);
        }
        Statement::Assignment { name, value } => {
            compile_expression(output, fn_table, obj_table, locals, value);

            if let Some((slot, mutable, _)) = locals.resolve(&name) {
                if !mutable {
                    output.errors.push(OrynError::Compiler {
                        span: stmt_span.clone(),
                        message: format!("cannot reassign val binding `{name}`"),
                    });
                }

                emit(output, Instruction::SetLocal(slot), &stmt_span);
            } else {
                output.errors.push(OrynError::Compiler {
                    span: stmt_span.clone(),
                    message: format!("undefined variable `{name}`"),
                });
            }
        }
        Statement::Function {
            name, params, body, ..
        } => {
            // Reserve the function's index so recursive calls and
            // later calls resolve correctly.
            let func_idx = output.functions.len();

            // Register in the lookup table BEFORE compiling the body.
            // This is what makes recursion work - fib can find itself.
            // We use a mutable ref to fn_table here via a small trick:
            // the FunctionTable is passed immutably to other functions,
            // but we need to mutate it here. We'll handle this by
            // registering before the recursive compile call.

            // Push a placeholder so the index is valid.
            output.functions.push(CompiledFunction {
                name: name.clone(),
                arity: params.len(),
                params: params.clone().into_iter().map(|p| p.0).collect(),
                num_locals: 0,
                instructions: Vec::new(),
                spans: Vec::new(),
            });

            let mut func_output = CompilerOutput {
                instructions: Vec::new(),
                spans: Vec::new(),
                functions: Vec::new(),
                obj_defs: Vec::new(),
                errors: Vec::new(),
            };

            let mut func_locals = Locals::new();
            for param in &params {
                // Parameters are immutable, so we define them as such.
                // Extract object type from type annotation if present.
                let obj_type = param.1.as_ref().map(|t| match t {
                    crate::parser::TypeAnnotation::Named(name) => name.clone(),
                });
                func_locals.define(param.0.clone(), false, obj_type);
            }

            for param in params.iter().rev() {
                // SAFETY: We just defined every param in the loop above,
                // so resolve is guaranteed to succeed.
                let slot = func_locals.resolve(param.0.as_str()).unwrap();

                emit(&mut func_output, Instruction::SetLocal(slot.0), &stmt_span);
            }

            // Build a function table that includes this function (for
            // recursion) plus all previously defined functions.
            let mut inner_fn_table = FunctionTable::new();
            for (name, idx) in &fn_table.names {
                inner_fn_table.register(name.clone(), *idx);
            }

            inner_fn_table.register(name.clone(), func_idx);

            compile_expression_with_loops(
                &mut func_output,
                &inner_fn_table,
                obj_table,
                &mut Vec::new(),
                &mut func_locals,
                body,
            );

            emit(&mut func_output, Instruction::PushInt(0), &stmt_span);
            emit(&mut func_output, Instruction::Return, &stmt_span);

            output.functions[func_idx] = CompiledFunction {
                name: name.clone(),
                arity: params.len(),
                params: params.into_iter().map(|p| p.0).collect(),
                num_locals: func_locals.count,
                instructions: func_output.instructions,
                spans: func_output.spans,
            };

            output.functions.extend(func_output.functions);
            output.errors.extend(func_output.errors);
        }
        Statement::Return(Some(expr)) => {
            compile_expression(output, fn_table, obj_table, locals, expr);
            emit(output, Instruction::Return, &stmt_span);
        }
        Statement::Return(None) => {
            emit(output, Instruction::PushInt(0), &stmt_span);
            emit(output, Instruction::Return, &stmt_span);
        }
        Statement::ObjDef { name, fields } => {
            let field_names: Vec<String> = fields.into_iter().map(|(name, _)| name).collect();
            output.obj_defs.push(ObjDefInfo {
                name,
                fields: field_names,
            });
        }
        Statement::FieldAssignment {
            object,
            field,
            value,
        } => {
            // Resolve type and mutability BEFORE compiling (which moves object).
            let (obj_type, mutable) = match &object.node {
                Expression::Ident(name) => match locals.resolve(name) {
                    Some((_, m, t)) => (t, m),
                    None => (None, true),
                },
                _ => (None, true),
            };

            if !mutable {
                output.errors.push(OrynError::Compiler {
                    span: stmt_span.clone(),
                    message: "cannot mutate field on val binding".into(),
                });
            }

            compile_expression(output, fn_table, obj_table, locals, object);
            compile_expression(output, fn_table, obj_table, locals, value);

            if let Some(field_idx) = resolve_field(output, obj_table, &obj_type, &field, &stmt_span)
            {
                emit(output, Instruction::SetField(field_idx), &stmt_span);
            }
        }
        Statement::If {
            condition,
            body,
            else_body,
        } => {
            compile_expression(output, fn_table, obj_table, locals, condition);

            let jump_if_false_idx = output.instructions.len();
            emit(output, Instruction::JumpIfFalse(0), &stmt_span);

            compile_expression_with_loops(output, fn_table, obj_table, loops, locals, body);

            if let Some(else_body) = else_body {
                let jump_idx = output.instructions.len();
                emit(output, Instruction::Jump(0), &stmt_span);

                let else_start = output.instructions.len();
                output.instructions[jump_if_false_idx] = Instruction::JumpIfFalse(else_start);

                compile_expression_with_loops(
                    output, fn_table, obj_table, loops, locals, else_body,
                );

                let end = output.instructions.len();
                output.instructions[jump_idx] = Instruction::Jump(end);
            } else {
                let end = output.instructions.len();
                output.instructions[jump_if_false_idx] = Instruction::JumpIfFalse(end);
            }
        }
        Statement::While { condition, body } => {
            let loop_start = output.instructions.len();

            compile_expression(output, fn_table, obj_table, locals, condition);

            let exit_jump_idx = output.instructions.len();
            emit(output, Instruction::JumpIfFalse(0), &stmt_span);

            loops.push(LoopContext {
                start: loop_start,
                break_patches: Vec::new(),
            });

            compile_expression_with_loops(output, fn_table, obj_table, loops, locals, body);

            emit(output, Instruction::Jump(loop_start), &stmt_span);

            let end = output.instructions.len();
            output.instructions[exit_jump_idx] = Instruction::JumpIfFalse(end);

            let loop_ctx = loops.pop().expect("loop context missing");
            for patch_idx in loop_ctx.break_patches {
                output.instructions[patch_idx] = Instruction::Jump(end);
            }
        }
        Statement::Break => {
            if let Some(loop_ctx) = loops.last_mut() {
                let idx = output.instructions.len();
                emit(output, Instruction::Jump(0), &stmt_span);
                loop_ctx.break_patches.push(idx);
            }
        }
        Statement::Continue => {
            if let Some(loop_ctx) = loops.last() {
                emit(output, Instruction::Jump(loop_ctx.start), &stmt_span);
            }
        }
        Statement::Expression(expr) => {
            let expr_span = expr.span.clone();
            compile_expression(output, fn_table, obj_table, locals, expr);
            emit(output, Instruction::Pop, &expr_span);
        }
    }
}

fn compile_expression_with_loops(
    output: &mut CompilerOutput,
    fn_table: &FunctionTable,
    obj_table: &ObjTable,
    loops: &mut Vec<LoopContext>,
    locals: &mut Locals,
    expr: Spanned<Expression>,
) {
    let span = expr.span.clone();
    match expr.node {
        Expression::Block(stmts) => {
            for stmt in stmts {
                compile_statement(output, fn_table, obj_table, loops, locals, stmt);
            }
        }
        other => compile_expression(
            output,
            fn_table,
            obj_table,
            locals,
            Spanned { node: other, span },
        ),
    }
}

fn compile_expression(
    output: &mut CompilerOutput,
    fn_table: &FunctionTable,
    obj_table: &ObjTable,
    locals: &mut Locals,
    expr: Spanned<Expression>,
) {
    let span = expr.span.clone();

    match expr.node {
        Expression::True => {
            emit(output, Instruction::PushBool(true), &span);
        }
        Expression::False => {
            emit(output, Instruction::PushBool(false), &span);
        }
        Expression::Float(n) => {
            emit(output, Instruction::PushFloat(n), &span);
        }
        Expression::Int(n) => {
            emit(output, Instruction::PushInt(n), &span);
        }
        Expression::String(s) => {
            emit(output, Instruction::PushString(s), &span);
        }
        Expression::Ident(name) => {
            if let Some(slot) = locals.resolve(&name) {
                emit(output, Instruction::GetLocal(slot.0), &span);
            } else {
                output.errors.push(OrynError::Compiler {
                    span: span.clone(),
                    message: format!("undefined variable `{name}`"),
                });
                // Emit a placeholder so the rest of the compilation
                // can continue without cascading errors.
                emit(output, Instruction::PushInt(0), &span);
            }
        }
        Expression::ObjLiteral { type_name, fields } => {
            // Object literal compilation: Vec2 { y: 2, x: 1 }
            // The user can write fields in any order, but the VM expects
            // them on the stack in definition order (x=0, y=1). We
            // reorder by iterating over the definition's field list and
            // pulling each value from a HashMap of what the user wrote.
            // After all values are on the stack, NewObject pops them all
            // and allocates the object.
            if let Some((type_idx, def)) = obj_table.resolve(&type_name) {
                let def_fields = def.fields.clone();
                let num_fields = def_fields.len();

                // Check for extra fields not in the definition.
                for (name, _) in &fields {
                    if !def_fields.contains(name) {
                        output.errors.push(OrynError::Compiler {
                            span: span.clone(),
                            message: format!("unknown field `{name}` on type `{type_name}`"),
                        });
                    }
                }

                // Build a map of field name -> value for O(1) lookup.
                let mut field_map: HashMap<String, Spanned<Expression>> =
                    fields.into_iter().collect();

                // Compile field values in definition order so the stack
                // layout matches field indices when NewObject pops them.
                for def_field in &def_fields {
                    if let Some(value) = field_map.remove(def_field) {
                        compile_expression(output, fn_table, obj_table, locals, value);
                    } else {
                        output.errors.push(OrynError::Compiler {
                            span: span.clone(),
                            message: format!(
                                "missing field `{def_field}` in `{type_name}` literal"
                            ),
                        });
                        emit(output, Instruction::PushInt(0), &span);
                    }
                }

                emit(output, Instruction::NewObject(type_idx, num_fields), &span);
            } else {
                output.errors.push(OrynError::Compiler {
                    span: span.clone(),
                    message: format!("undefined type `{type_name}`"),
                });
                emit(output, Instruction::PushInt(0), &span);
            }
        }
        Expression::FieldAccess { object, field } => {
            // Field access is resolved entirely at compile time. The
            // compiler looks up the variable's tracked object type,
            // then maps the field name to an integer index via ObjTable.
            // At runtime, GetField(2) is a direct array index.
            // Resolve type BEFORE compiling (which moves object).
            let obj_type = match &object.node {
                Expression::Ident(name) => locals.resolve(name).and_then(|(_, _, t)| t),
                _ => None,
            };

            compile_expression(output, fn_table, obj_table, locals, *object);

            if let Some(field_idx) = resolve_field(output, obj_table, &obj_type, &field, &span) {
                emit(output, Instruction::GetField(field_idx), &span);
            } else {
                emit(output, Instruction::PushInt(0), &span);
            }
        }
        Expression::BinaryOp { op, left, right } => {
            compile_expression(output, fn_table, obj_table, locals, *left);
            compile_expression(output, fn_table, obj_table, locals, *right);

            emit(
                output,
                match op {
                    BinOp::Equals => Instruction::Equal,
                    BinOp::NotEquals => Instruction::NotEqual,
                    BinOp::LessThan => Instruction::LessThan,
                    BinOp::GreaterThan => Instruction::GreaterThan,
                    BinOp::LessThanEquals => Instruction::LessThanEquals,
                    BinOp::GreaterThanEquals => Instruction::GreaterThanEquals,
                    BinOp::And => Instruction::And,
                    BinOp::Or => Instruction::Or,
                    BinOp::Add => Instruction::Add,
                    BinOp::Sub => Instruction::Sub,
                    BinOp::Mul => Instruction::Mul,
                    BinOp::Div => Instruction::Div,
                },
                &span,
            );
        }
        Expression::UnaryOp { op, expr: operand } => {
            compile_expression(output, fn_table, obj_table, locals, *operand);

            emit(
                output,
                match op {
                    UnaryOp::Not => Instruction::Not,
                    UnaryOp::Negate => Instruction::Negate,
                },
                &span,
            );
        }
        Expression::Call { name, args } => {
            let arity = args.len();

            for arg in args {
                compile_expression(output, fn_table, obj_table, locals, arg);
            }

            // Resolve function name at compile time. User-defined
            // functions get Call(index), everything else gets
            // CallBuiltin(name) for the VM to check at runtime.
            if let Some(idx) = fn_table.resolve(&name) {
                emit(output, Instruction::Call(idx, arity), &span);
            } else {
                emit(output, Instruction::CallBuiltin(name, arity), &span);
            }
        }
        Expression::Block(stmts) => {
            let mut no_loops = Vec::new();
            for stmt in stmts {
                compile_statement(output, fn_table, obj_table, &mut no_loops, locals, stmt);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spanned<T>(node: T) -> Spanned<T> {
        Spanned { node, span: 0..0 }
    }

    #[test]
    fn flattens_ast_to_instructions() {
        let stmts = vec![spanned(Statement::Expression(spanned(
            Expression::BinaryOp {
                op: BinOp::Add,
                left: Box::new(spanned(Expression::Int(1))),
                right: Box::new(spanned(Expression::Int(2))),
            },
        )))];

        let output = compile(stmts);

        assert_eq!(
            output.instructions,
            vec![
                Instruction::PushInt(1),
                Instruction::PushInt(2),
                Instruction::Add,
                Instruction::Pop,
            ]
        );
        assert_eq!(output.instructions.len(), output.spans.len());
    }

    #[test]
    fn expression_statements_are_popped() {
        let stmts = vec![spanned(Statement::Expression(spanned(Expression::Int(1))))];
        let output = compile(stmts);

        assert_eq!(output.instructions.last(), Some(&Instruction::Pop));
    }
}
