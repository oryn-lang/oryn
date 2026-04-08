use std::collections::HashMap;

use crate::OrynError;
use crate::parser::{BinOp, Expression, Span, Spanned, Statement, UnaryOp};

use super::tables::{FunctionTable, Locals, ObjTable};
use super::types::{CompiledFunction, CompilerOutput, FunctionBodyConfig, Instruction, ObjDefInfo};

// ---------------------------------------------------------------------------
// Loop tracking
// ---------------------------------------------------------------------------

struct LoopContext {
    start: usize,
    break_patches: Vec<usize>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

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
                output.obj_defs[i].methods.clone(),
            );
        }
    }

    output
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

/// Shared compilation logic for functions and methods. Reserves a slot in
/// the function table, compiles the body with its own locals and output,
/// then writes the result back. Returns the function table index.
fn compile_function_body(
    output: &mut CompilerOutput,
    fn_table: &FunctionTable,
    obj_table: &ObjTable,
    config: FunctionBodyConfig<'_>,
) -> usize {
    let FunctionBodyConfig {
        name,
        params,
        param_local_fn,
        self_name,
        body,
        span,
    } = config;
    let func_idx = output.functions.len();
    let param_names: Vec<String> = params.iter().map(|p| p.0.clone()).collect();

    // Push a placeholder so the index is valid.
    output.functions.push(CompiledFunction {
        name: name.to_string(),
        arity: params.len(),
        params: param_names.clone(),
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
    for param in params {
        let (mutable, obj_type) = param_local_fn(&param.0, &param.1);
        func_locals.define(param.0.clone(), mutable, obj_type);
    }

    // Pop params from the stack into locals in reverse order.
    for pname in param_names.iter().rev() {
        // SAFETY: We just defined every param in the loop above,
        // so resolve is guaranteed to succeed.
        let slot = func_locals.resolve(pname.as_str()).unwrap();
        emit(&mut func_output, Instruction::SetLocal(slot.0), span);
    }

    // Build a function table that includes all previously defined functions.
    // If self_name is set, also register this function for recursion.
    let mut inner_fn_table = FunctionTable::new();
    for (fname, idx) in &fn_table.names {
        inner_fn_table.register(fname.clone(), *idx);
    }
    if let Some(self_name) = self_name {
        inner_fn_table.register(self_name.to_string(), func_idx);
    }

    compile_expression_with_loops(
        &mut func_output,
        &inner_fn_table,
        obj_table,
        &mut Vec::new(),
        &mut func_locals,
        body,
    );

    emit(&mut func_output, Instruction::PushInt(0), span);
    emit(&mut func_output, Instruction::Return, span);

    output.functions[func_idx] = CompiledFunction {
        name: name.to_string(),
        arity: params.len(),
        params: param_names,
        num_locals: func_locals.count,
        instructions: func_output.instructions,
        spans: func_output.spans,
    };

    output.functions.extend(func_output.functions);
    output.errors.extend(func_output.errors);

    func_idx
}

// ---------------------------------------------------------------------------
// Statement compilation
// ---------------------------------------------------------------------------

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
        // -- Bindings --
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

        // -- Assignments --
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
        Statement::FieldAssignment {
            object,
            field,
            value,
        } => {
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

        // -- Functions --
        Statement::Function {
            name, params, body, ..
        } => {
            let param_fn = |_name: &str, ann: &Option<crate::parser::TypeAnnotation>| {
                let obj_type = ann.as_ref().map(|t| match t {
                    crate::parser::TypeAnnotation::Named(n) => n.clone(),
                });
                (false, obj_type)
            };

            compile_function_body(
                output,
                fn_table,
                obj_table,
                FunctionBodyConfig {
                    name: &name,
                    params: &params,
                    param_local_fn: &param_fn,
                    self_name: Some(&name),
                    body,
                    span: &stmt_span,
                },
            );
        }
        Statement::Return(Some(expr)) => {
            compile_expression(output, fn_table, obj_table, locals, expr);
            emit(output, Instruction::Return, &stmt_span);
        }
        Statement::Return(None) => {
            emit(output, Instruction::PushInt(0), &stmt_span);
            emit(output, Instruction::Return, &stmt_span);
        }

        // -- Objects --
        Statement::ObjDef {
            name,
            fields,
            methods,
            uses,
        } => {
            let mut field_names: Vec<String> = Vec::new();
            let mut method_indices: HashMap<String, usize> = HashMap::new();

            for used_type in &uses {
                if let Some((_, def)) = obj_table.resolve(used_type) {
                    for field in &def.fields {
                        if field_names.contains(field) {
                            output.errors.push(OrynError::Compiler {
                                span: stmt_span.clone(),
                                message: format!("field `{field}` conflicts in `use {used_type}`"),
                            });
                        } else {
                            field_names.push(field.clone());
                        }
                    }

                    for (method_name, &func_idx) in &def.methods {
                        if method_indices.contains_key(method_name) {
                            output.errors.push(OrynError::Compiler {
                                span: stmt_span.clone(),
                                message: format!(
                                    "method `{method_name}` conflicts in `use {used_type}`"
                                ),
                            });
                        } else {
                            method_indices.insert(method_name.clone(), func_idx);
                        }
                    }
                } else {
                    output.errors.push(OrynError::Compiler {
                        span: stmt_span.clone(),
                        message: format!("undefined type `{used_type}` in use declaration"),
                    });
                }
            }

            // Then append this obj's own fields.
            for (name, _) in fields {
                field_names.push(name);
            }

            // Build a temporary ObjTable that includes the current type
            // so method bodies can resolve self.field accesses.
            let mut inner_obj_table = ObjTable::new();
            for (tname, &idx) in &obj_table.names {
                inner_obj_table.names.insert(tname.clone(), idx);
            }
            inner_obj_table.defs = obj_table.defs.clone();
            inner_obj_table.register(name.clone(), field_names.clone(), HashMap::new());

            for method in methods {
                let obj_name = name.clone();
                let param_fn = move |pname: &str, ann: &Option<crate::parser::TypeAnnotation>| {
                    if pname == "self" {
                        (true, Some(obj_name.clone()))
                    } else {
                        let obj_type = ann.as_ref().map(|t| match t {
                            crate::parser::TypeAnnotation::Named(n) => n.clone(),
                        });
                        (false, obj_type)
                    }
                };

                let func_idx = compile_function_body(
                    output,
                    fn_table,
                    &inner_obj_table,
                    FunctionBodyConfig {
                        name: &method.name,
                        params: &method.params,
                        param_local_fn: &param_fn,
                        self_name: None,
                        body: method.body,
                        span: &stmt_span,
                    },
                );

                method_indices.insert(method.name.clone(), func_idx);
            }

            output.obj_defs.push(ObjDefInfo {
                name,
                fields: field_names,
                methods: method_indices,
            });
        }

        // -- Control flow --
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
            } else {
                output.errors.push(OrynError::Compiler {
                    span: stmt_span,
                    message: "break outside of loop".into(),
                });
            }
        }
        Statement::Continue => {
            if let Some(loop_ctx) = loops.last() {
                emit(output, Instruction::Jump(loop_ctx.start), &stmt_span);
            } else {
                output.errors.push(OrynError::Compiler {
                    span: stmt_span,
                    message: "continue outside of loop".into(),
                });
            }
        }

        // -- Expression statements --
        Statement::Expression(expr) => {
            let expr_span = expr.span.clone();
            compile_expression(output, fn_table, obj_table, locals, expr);
            emit(output, Instruction::Pop, &expr_span);
        }
    }
}

// ---------------------------------------------------------------------------
// Expression compilation
// ---------------------------------------------------------------------------

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
        // -- Literals --
        Expression::True => emit(output, Instruction::PushBool(true), &span),
        Expression::False => emit(output, Instruction::PushBool(false), &span),
        Expression::Float(n) => emit(output, Instruction::PushFloat(n), &span),
        Expression::Int(n) => emit(output, Instruction::PushInt(n), &span),
        Expression::String(s) => emit(output, Instruction::PushString(s), &span),

        // -- Variables --
        Expression::Ident(name) => {
            if let Some(slot) = locals.resolve(&name) {
                emit(output, Instruction::GetLocal(slot.0), &span);
            } else {
                output.errors.push(OrynError::Compiler {
                    span: span.clone(),
                    message: format!("undefined variable `{name}`"),
                });
                emit(output, Instruction::PushInt(0), &span);
            }
        }

        // -- Objects --
        Expression::ObjLiteral { type_name, fields } => {
            // The user can write fields in any order, but the VM expects
            // them on the stack in definition order. We reorder by iterating
            // over the definition's field list and pulling each value from
            // a HashMap of what the user wrote.
            if let Some((type_idx, def)) = obj_table.resolve(&type_name) {
                let def_fields = def.fields.clone();
                let num_fields = def_fields.len();

                for (name, _) in &fields {
                    if !def_fields.contains(name) {
                        output.errors.push(OrynError::Compiler {
                            span: span.clone(),
                            message: format!("unknown field `{name}` on type `{type_name}`"),
                        });
                    }
                }

                let mut field_map: HashMap<String, Spanned<Expression>> =
                    fields.into_iter().collect();

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
        Expression::MethodCall {
            object,
            method,
            args,
        } => {
            compile_expression(output, fn_table, obj_table, locals, *object);

            let arity = args.len();
            for arg in args {
                compile_expression(output, fn_table, obj_table, locals, arg);
            }

            emit(output, Instruction::CallMethod(method, arity), &span);
        }

        // -- Operators --
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

        // -- Calls --
        Expression::Call { name, args } => {
            let arity = args.len();

            for arg in args {
                compile_expression(output, fn_table, obj_table, locals, arg);
            }

            if let Some(idx) = fn_table.resolve(&name) {
                emit(output, Instruction::Call(idx, arity), &span);
            } else {
                emit(output, Instruction::CallBuiltin(name, arity), &span);
            }
        }

        // -- Blocks --
        Expression::Block(stmts) => {
            let mut no_loops = Vec::new();
            for stmt in stmts {
                compile_statement(output, fn_table, obj_table, &mut no_loops, locals, stmt);
            }
        }
    }
}
