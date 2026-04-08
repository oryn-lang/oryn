use std::collections::HashMap;

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{BinOp, Expression, Span, Spanned, Statement, TypeAnnotation, UnaryOp};

use super::tables::{FunctionSignature, FunctionTable, Locals, ObjTable};
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
    let mut output = CompilerOutput::default();
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
            let func = &output.functions[i];
            fn_table.register(func.name.clone(), i);

            // Store the type signature for call-site checking.
            if let Some(ref rt) = func.return_type {
                fn_table.signatures.insert(
                    func.name.clone(),
                    FunctionSignature {
                        param_types: func.param_types.clone(),
                        return_type: rt.clone(),
                    },
                );
            }
        }

        // Same for object definitions.
        for i in obj_count_before..output.obj_defs.len() {
            obj_table.register(
                output.obj_defs[i].name.clone(),
                output.obj_defs[i].fields.clone(),
                output.obj_defs[i].field_types.clone(),
                output.obj_defs[i].methods.clone(),
                output.obj_defs[i].signatures.clone(),
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
    obj_type: &ResolvedType,
    field: &str,
    span: &Span,
) -> Option<usize> {
    let type_name = match obj_type {
        ResolvedType::Object(name) => name,
        _ => {
            output.errors.push(OrynError::compiler(span.clone(), "cannot access field on non-object"));
            return None;
        }
    };

    let (_, def) = match obj_table.resolve(type_name) {
        Some(pair) => pair,
        None => {
            output.errors.push(OrynError::compiler(span.clone(), format!("undefined type `{type_name}`")));
            return None;
        }
    };

    match def.fields.iter().position(|f| f == field) {
        Some(idx) => Some(idx),
        None => {
            output.errors.push(OrynError::compiler(span.clone(), format!("unknown field `{field}` on type `{type_name}`")));
            None
        }
    }
}

// Resolves a type annotation to a `ResolvedType`.
fn resolve_type_annotation(
    ann: &TypeAnnotation,
    obj_table: &ObjTable,
) -> Result<ResolvedType, String> {
    match ann {
        TypeAnnotation::Named(name) => match name.as_str() {
            "i32" => Ok(ResolvedType::Int),
            "f32" => Ok(ResolvedType::Float),
            "bool" => Ok(ResolvedType::Bool),
            "String" => Ok(ResolvedType::Str),
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

// Checks that the expected and actual types match, and adds an error to the output if they don't.
fn check_types(
    output: &mut CompilerOutput,
    expected: &ResolvedType,
    actual: &ResolvedType,
    span: &Span,
    message: &str,
) {
    if *expected != ResolvedType::Unknown && *actual != ResolvedType::Unknown && expected != actual
    {
        output.errors.push(OrynError::compiler(
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

/// Compile a let or val binding. Resolves the type annotation (if present),
/// compiles the value expression, checks types, and defines the local.
#[allow(clippy::too_many_arguments)]
fn compile_binding(
    output: &mut CompilerOutput,
    fn_table: &FunctionTable,
    obj_table: &ObjTable,
    locals: &mut Locals,
    name: String,
    value: Spanned<Expression>,
    type_ann: Option<crate::parser::TypeAnnotation>,
    mutable: bool,
    span: &Span,
) {
    let declared_type =
        type_ann
            .as_ref()
            .map(|ann| match resolve_type_annotation(ann, obj_table) {
                Ok(t) => t,
                Err(msg) => {
                    output.errors.push(OrynError::compiler(span.clone(), msg));
                    ResolvedType::Unknown
                }
            });

    let inferred_type = compile_expression(output, fn_table, obj_table, locals, value);

    if let Some(ref decl) = declared_type {
        check_types(output, decl, &inferred_type, span, "type mismatch");
    }

    let resolved = declared_type.unwrap_or(inferred_type);
    let slot = locals.define(name, mutable, resolved);
    emit(output, Instruction::SetLocal(slot), span);
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
        param_types,
        param_local_fn,
        self_name,
        body,
        span,
        return_type,
    } = config;
    let func_idx = output.functions.len();
    let param_names: Vec<String> = params.iter().map(|p| p.0.clone()).collect();

    // Push a placeholder so the index is valid.
    output.functions.push(CompiledFunction {
        name: name.to_string(),
        arity: params.len(),
        params: param_names.clone(),
        param_types: param_types.clone(),
        return_type: return_type.clone(),
        num_locals: 0,
        instructions: Vec::new(),
        spans: Vec::new(),
    });

    let mut func_output = CompilerOutput::default();
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
        param_types,
        return_type,
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
        Statement::Let {
            name,
            value,
            type_ann,
        } => {
            compile_binding(
                output, fn_table, obj_table, locals, name, value, type_ann, true, &stmt_span,
            );
        }
        Statement::Val {
            name,
            value,
            type_ann,
        } => {
            compile_binding(
                output, fn_table, obj_table, locals, name, value, type_ann, false, &stmt_span,
            );
        }

        // -- Assignments --
        Statement::Assignment { name, value } => {
            let value_type = compile_expression(output, fn_table, obj_table, locals, value);

            if let Some((slot, mutable, stored_type)) = locals.resolve(&name) {
                if !mutable {
                    output.errors.push(OrynError::compiler(stmt_span.clone(), format!("cannot reassign val binding `{name}`")));
                }

                check_types(
                    output,
                    &stored_type,
                    &value_type,
                    &stmt_span,
                    "assignment type mismatch",
                );

                emit(output, Instruction::SetLocal(slot), &stmt_span);
            } else {
                output.errors.push(OrynError::compiler(stmt_span.clone(), format!("undefined variable `{name}`")));
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
                    None => (ResolvedType::Unknown, true),
                },
                _ => (ResolvedType::Unknown, true),
            };

            if !mutable {
                output.errors.push(OrynError::compiler(stmt_span.clone(), "cannot mutate field on val binding"));
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
            name,
            params,
            body,
            return_type,
        } => {
            // Pre-resolve param types so the closure doesn't need to
            // capture obj_table (which would cause a lifetime issue).
            let resolved_params: HashMap<String, ResolvedType> = params
                .iter()
                .map(|(name, ann)| {
                    let t = ann
                        .as_ref()
                        .map(|a| {
                            resolve_type_annotation(a, obj_table).unwrap_or(ResolvedType::Unknown)
                        })
                        .unwrap_or(ResolvedType::Unknown);
                    (name.clone(), t)
                })
                .collect();

            let param_fn = move |pname: &str, _ann: &Option<crate::parser::TypeAnnotation>| {
                let resolved = resolved_params
                    .get(pname)
                    .cloned()
                    .unwrap_or(ResolvedType::Unknown);
                (false, resolved)
            };

            for (param_name, ann) in &params {
                if ann.is_none() {
                    output.errors.push(OrynError::compiler(stmt_span.clone(), format!("parameter `{param_name}` requires a type annotation")));
                }
            }

            let param_types: Vec<ResolvedType> = params
                .iter()
                .map(|(_, ann)| {
                    ann.as_ref()
                        .map(|a| {
                            resolve_type_annotation(a, obj_table).unwrap_or(ResolvedType::Unknown)
                        })
                        .unwrap_or(ResolvedType::Unknown)
                })
                .collect();

            // No return type annotation = void function.
            let return_resolved = match &return_type {
                Some(rt) => resolve_type_annotation(rt, obj_table).unwrap_or(ResolvedType::Unknown),
                None => ResolvedType::Void,
            };

            compile_function_body(
                output,
                fn_table,
                obj_table,
                FunctionBodyConfig {
                    name: &name,
                    params: &params,
                    param_types,
                    param_local_fn: &param_fn,
                    self_name: Some(&name),
                    body,
                    span: &stmt_span,
                    return_type: Some(return_resolved),
                },
            );
        }
        Statement::Return(Some(expr)) => {
            let return_type = compile_expression(output, fn_table, obj_table, locals, expr);

            if let Some(ref expected) = locals.return_type {
                check_types(
                    output,
                    expected,
                    &return_type,
                    &stmt_span,
                    "return type mismatch",
                );
            }

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
            let mut field_types: Vec<ResolvedType> = Vec::new();
            let mut method_indices: HashMap<String, usize> = HashMap::new();
            let mut all_required: Vec<String> = Vec::new();

            for used_type in &uses {
                if let Some((_, def)) = obj_table.resolve(used_type) {
                    // Collect signatures from the used type.
                    for req in &def.signatures {
                        if !all_required.contains(req) {
                            all_required.push(req.clone());
                        }
                    }

                    for field in &def.fields {
                        if field_names.contains(field) {
                            output.errors.push(OrynError::compiler(stmt_span.clone(), format!("field `{field}` conflicts in `use {used_type}`")));
                        } else {
                            field_names.push(field.clone());
                        }
                    }

                    for (method_name, &func_idx) in &def.methods {
                        if method_indices.contains_key(method_name) {
                            output.errors.push(OrynError::compiler(stmt_span.clone(), format!(
                                    "method `{method_name}` conflicts in `use {used_type}`"
                                )));
                        } else {
                            method_indices.insert(method_name.clone(), func_idx);
                        }
                    }
                } else {
                    output.errors.push(OrynError::compiler(stmt_span.clone(), format!("undefined type `{used_type}` in use declaration")));
                }
            }

            // Then append this obj's own fields.
            for (field_name, type_ann, field_span) in fields {
                field_names.push(field_name.clone());

                match resolve_type_annotation(&type_ann, obj_table) {
                    Ok(t) => field_types.push(t),
                    Err(msg) => {
                        output.errors.push(OrynError::compiler(field_span, format!("field `{field_name}`: {msg}")));

                        field_types.push(ResolvedType::Unknown);
                    }
                }
            }

            // Build a temporary ObjTable that includes the current type
            // so method bodies can resolve self.field accesses.
            let mut inner_obj_table = ObjTable::new();
            for (tname, &idx) in &obj_table.names {
                inner_obj_table.names.insert(tname.clone(), idx);
            }
            inner_obj_table.defs = obj_table.defs.clone();
            inner_obj_table.register(
                name.clone(),
                field_names.clone(),
                field_types.clone(),
                HashMap::new(),
                Vec::new(),
            );

            // Collect this type's own required methods (bodyless declarations)
            // before the loop moves `methods`.
            let own_required: Vec<String> = methods
                .iter()
                .filter(|m| m.body.is_none())
                .map(|m| m.name.clone())
                .collect();

            for method in methods {
                if let Some(body) = method.body {
                    let obj_name = name.clone();

                    let param_fn = move |pname: &str, ann: &Option<TypeAnnotation>| {
                        if pname == "self" {
                            (true, ResolvedType::Object(obj_name.clone()))
                        } else {
                            let resolved = match ann {
                                Some(a) => ResolvedType::from_annotation(a),
                                None => ResolvedType::Unknown,
                            };
                            (false, resolved)
                        }
                    };

                    let func_idx = compile_function_body(
                        output,
                        fn_table,
                        &inner_obj_table,
                        FunctionBodyConfig {
                            name: &method.name,
                            params: &method.params,
                            param_types: Vec::new(),
                            param_local_fn: &param_fn,
                            self_name: None,
                            body,
                            span: &stmt_span,
                            return_type: None,
                        },
                    );

                    method_indices.insert(method.name.clone(), func_idx);
                }
            }

            // Check: every required method from used types must be satisfied.
            for req in &all_required {
                if !method_indices.contains_key(req) {
                    output.errors.push(OrynError::compiler(stmt_span.clone(), format!("object `{name}` is missing required method `{req}`")));
                }
            }

            // The final signatures for this type are its own bodyless
            // declarations, minus any that were already implemented, plus any
            // inherited requirements that were NOT satisfied here (so they
            // propagate upward).
            let mut final_required: Vec<String> = Vec::new();
            for req in own_required {
                if !method_indices.contains_key(&req) {
                    final_required.push(req);
                }
            }

            output.obj_defs.push(ObjDefInfo {
                name,
                fields: field_names,
                field_types: Vec::new(),
                methods: method_indices,
                signatures: final_required,
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
                output.errors.push(OrynError::compiler(stmt_span, "break outside of loop"));
            }
        }
        Statement::Continue => {
            if let Some(loop_ctx) = loops.last() {
                emit(output, Instruction::Jump(loop_ctx.start), &stmt_span);
            } else {
                output.errors.push(OrynError::compiler(stmt_span, "continue outside of loop"));
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
) -> ResolvedType {
    let span = expr.span.clone();
    match expr.node {
        Expression::Block(stmts) => {
            for stmt in stmts {
                compile_statement(output, fn_table, obj_table, loops, locals, stmt);
            }

            ResolvedType::Unknown
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
) -> ResolvedType {
    let span = expr.span.clone();

    match expr.node {
        // -- Literals --
        Expression::True => {
            emit(output, Instruction::PushBool(true), &span);
            ResolvedType::Bool
        }
        Expression::False => {
            emit(output, Instruction::PushBool(false), &span);
            ResolvedType::Bool
        }
        Expression::Float(n) => {
            emit(output, Instruction::PushFloat(n), &span);
            ResolvedType::Float
        }
        Expression::Int(n) => {
            emit(output, Instruction::PushInt(n), &span);
            ResolvedType::Int
        }
        Expression::String(s) => {
            emit(output, Instruction::PushString(s), &span);
            ResolvedType::Str
        }

        // -- Variables --
        Expression::Ident(name) => {
            if let Some((slot, _, resolved_type)) = locals.resolve(&name) {
                emit(output, Instruction::GetLocal(slot), &span);

                resolved_type
            } else {
                output.errors.push(OrynError::compiler(span.clone(), format!("undefined variable `{name}`")));

                emit(output, Instruction::PushInt(0), &span);

                ResolvedType::Unknown
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
                        output.errors.push(OrynError::compiler(span.clone(), format!("unknown field `{name}` on type `{type_name}`")));
                    }
                }

                let mut field_map: HashMap<String, Spanned<Expression>> =
                    fields.into_iter().collect();

                for def_field in &def_fields {
                    if let Some(value) = field_map.remove(def_field) {
                        compile_expression(output, fn_table, obj_table, locals, value);
                    } else {
                        output.errors.push(OrynError::compiler(span.clone(), format!(
                                "missing field `{def_field}` in `{type_name}` literal"
                            )));

                        emit(output, Instruction::PushInt(0), &span);
                    }
                }

                emit(output, Instruction::NewObject(type_idx, num_fields), &span);

                ResolvedType::Object(type_name)
            } else {
                output.errors.push(OrynError::compiler(span.clone(), format!("undefined type `{type_name}`")));

                emit(output, Instruction::PushInt(0), &span);

                ResolvedType::Unknown
            }
        }
        Expression::FieldAccess { object, field } => {
            let obj_type = match &object.node {
                Expression::Ident(name) => locals
                    .resolve(name)
                    .map(|(_, _, t)| t)
                    .unwrap_or(ResolvedType::Unknown),
                _ => ResolvedType::Unknown,
            };

            compile_expression(output, fn_table, obj_table, locals, *object);

            if let Some(field_idx) = resolve_field(output, obj_table, &obj_type, &field, &span) {
                emit(output, Instruction::GetField(field_idx), &span);
            } else {
                emit(output, Instruction::PushInt(0), &span);
            }

            ResolvedType::Unknown
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

            ResolvedType::Unknown
        }

        // -- Operators --
        Expression::BinaryOp { op, left, right } => {
            let left_type = compile_expression(output, fn_table, obj_table, locals, *left);
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

            match op {
                // Comparisons always return bool
                BinOp::Equals
                | BinOp::NotEquals
                | BinOp::LessThan
                | BinOp::GreaterThan
                | BinOp::LessThanEquals
                | BinOp::GreaterThanEquals => ResolvedType::Bool,
                // Logical ops return bool
                BinOp::And | BinOp::Or => ResolvedType::Bool,
                // Arithmetic returns the left operand's type
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => left_type,
            }
        }
        Expression::UnaryOp { op, expr: operand } => {
            let operand_type = compile_expression(output, fn_table, obj_table, locals, *operand);

            emit(
                output,
                match op {
                    UnaryOp::Not => Instruction::Not,
                    UnaryOp::Negate => Instruction::Negate,
                },
                &span,
            );

            operand_type
        }

        // -- Calls --
        Expression::Call { name, args } => {
            let arity = args.len();

            let mut arg_types = Vec::new();
            for arg in args {
                let arg_type = compile_expression(output, fn_table, obj_table, locals, arg);

                arg_types.push(arg_type);
            }

            if let Some(sig) = fn_table.signatures.get(&name) {
                for (i, (arg_type, param_type)) in
                    arg_types.iter().zip(&sig.param_types).enumerate()
                {
                    check_types(
                        output,
                        param_type,
                        arg_type,
                        &span,
                        &format!("argument {} type mismatch", i + 1),
                    );
                }
            }

            if let Some(idx) = fn_table.resolve(&name) {
                emit(output, Instruction::Call(idx, arity), &span);
            } else {
                emit(output, Instruction::CallBuiltin(name.clone(), arity), &span);
            }

            fn_table
                .signatures
                .get(&name)
                .map(|sig| sig.return_type.clone())
                .unwrap_or(ResolvedType::Unknown)
        }

        // -- Blocks --
        Expression::Block(stmts) => {
            let mut no_loops = Vec::new();

            for stmt in stmts {
                compile_statement(output, fn_table, obj_table, &mut no_loops, locals, stmt);
            }

            ResolvedType::Unknown
        }
    }
}
