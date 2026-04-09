use std::collections::HashMap;

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{Expression, Span, Spanned, Statement, TypeAnnotation};

use super::compile::{Compiler, LoopContext};
use super::func::FunctionBodyConfig;
use super::types::Instruction;

// ---------------------------------------------------------------------------
// Binding compilation
// ---------------------------------------------------------------------------

impl Compiler {
    /// Compile a let or val binding.
    pub(super) fn compile_binding(
        &mut self,
        name: String,
        value: Spanned<Expression>,
        type_ann: Option<TypeAnnotation>,
        mutable: bool,
        span: &Span,
    ) {
        let declared_type = type_ann
            .as_ref()
            .map(|ann| match self.resolve_type_annotation(ann) {
                Ok(t) => t,
                Err(msg) => {
                    self.output
                        .errors
                        .push(OrynError::compiler(span.clone(), msg));
                    ResolvedType::Unknown
                }
            });

        let inferred_type = self.compile_expr(value);

        if let Some(ref decl) = declared_type {
            self.check_types(decl, &inferred_type, span, "type mismatch");
        }

        let resolved = declared_type.unwrap_or(inferred_type);
        let slot = self.locals.define(name, mutable, resolved);
        self.emit(Instruction::SetLocal(slot), span);
    }
}

// ---------------------------------------------------------------------------
// Statement compilation
// ---------------------------------------------------------------------------

impl Compiler {
    pub(super) fn compile_stmt(&mut self, stmt: Spanned<Statement>) {
        let stmt_span = stmt.span.clone();

        match stmt.node {
            // -- Bindings --
            Statement::Let {
                name,
                value,
                type_ann,
            } => {
                self.compile_binding(name, value, type_ann, true, &stmt_span);
            }
            Statement::Val {
                name,
                value,
                type_ann,
            } => {
                self.compile_binding(name, value, type_ann, false, &stmt_span);
            }

            // -- Assignments --
            Statement::Assignment { name, value } => {
                let value_type = self.compile_expr(value);

                if let Some((slot, mutable, stored_type)) = self.locals.resolve(&name) {
                    if !mutable {
                        self.output.errors.push(OrynError::compiler(
                            stmt_span.clone(),
                            format!("cannot reassign val binding `{name}`"),
                        ));
                    }

                    self.check_types(
                        &stored_type,
                        &value_type,
                        &stmt_span,
                        "assignment type mismatch",
                    );

                    self.emit(Instruction::SetLocal(slot), &stmt_span);
                } else {
                    self.output.errors.push(OrynError::compiler(
                        stmt_span.clone(),
                        format!("undefined variable `{name}`"),
                    ));
                }
            }
            Statement::FieldAssignment {
                object,
                field,
                value,
            } => {
                let (obj_type, mutable) = match &object.node {
                    Expression::Ident(name) => match self.locals.resolve(name) {
                        Some((_, m, t)) => (t, m),
                        None => (ResolvedType::Unknown, true),
                    },
                    _ => (ResolvedType::Unknown, true),
                };

                if !mutable {
                    self.output.errors.push(OrynError::compiler(
                        stmt_span.clone(),
                        "cannot mutate field on val binding",
                    ));
                }

                self.compile_expr(object);
                self.compile_expr(value);

                if let Some(field_idx) = self.resolve_field(&obj_type, &field, &stmt_span) {
                    self.emit(Instruction::SetField(field_idx), &stmt_span);
                }
            }

            // -- Functions --
            Statement::Function {
                name,
                params,
                body,
                return_type,
            } => {
                // Resolve param types once, then derive both the HashMap
                // (for the closure) and the Vec (for FunctionBodyConfig).
                let resolved_params: HashMap<String, ResolvedType> = params
                    .iter()
                    .map(|(name, ann)| {
                        let t = ann
                            .as_ref()
                            .map(|a| {
                                self.resolve_type_annotation(a)
                                    .unwrap_or(ResolvedType::Unknown)
                            })
                            .unwrap_or(ResolvedType::Unknown);
                        (name.clone(), t)
                    })
                    .collect();

                let param_types: Vec<ResolvedType> = params
                    .iter()
                    .map(|(name, _)| {
                        resolved_params
                            .get(name)
                            .cloned()
                            .unwrap_or(ResolvedType::Unknown)
                    })
                    .collect();

                let param_fn = move |pname: &str, _ann: &Option<TypeAnnotation>| {
                    let resolved = resolved_params
                        .get(pname)
                        .cloned()
                        .unwrap_or(ResolvedType::Unknown);
                    (false, resolved)
                };

                for (param_name, ann) in &params {
                    if ann.is_none() {
                        self.output.errors.push(OrynError::compiler(
                            stmt_span.clone(),
                            format!("parameter `{param_name}` requires a type annotation"),
                        ));
                    }
                }

                let return_resolved = match &return_type {
                    Some(rt) => self
                        .resolve_type_annotation(rt)
                        .unwrap_or(ResolvedType::Unknown),
                    None => ResolvedType::Void,
                };

                self.compile_function_body(FunctionBodyConfig {
                    name: &name,
                    params: &params,
                    param_types,
                    param_local_fn: &param_fn,
                    self_name: Some(&name),
                    body,
                    span: &stmt_span,
                    return_type: Some(return_resolved),
                });
            }
            Statement::Return(Some(expr)) => {
                let return_type = self.compile_expr(expr);

                if let Some(ref expected) = self.locals.return_type {
                    let expected = expected.clone();
                    self.check_types(&expected, &return_type, &stmt_span, "return type mismatch");
                }

                self.emit(Instruction::Return, &stmt_span);
            }
            Statement::Return(None) => {
                self.emit(Instruction::PushInt(0), &stmt_span);
                self.emit(Instruction::Return, &stmt_span);
            }

            // -- Objects --
            Statement::ObjDef {
                name,
                fields,
                methods,
                uses,
            } => {
                self.compile_obj_def(name, fields, methods, uses, &stmt_span);
            }

            // -- Control flow --
            Statement::If {
                condition,
                body,
                else_body,
            } => {
                self.compile_expr(condition);

                let jump_if_false_idx = self.output.instructions.len();
                self.emit(Instruction::JumpIfFalse(0), &stmt_span);

                self.compile_body_expr(body);

                if let Some(else_body) = else_body {
                    let jump_idx = self.output.instructions.len();
                    self.emit(Instruction::Jump(0), &stmt_span);

                    let else_start = self.output.instructions.len();
                    self.output.instructions[jump_if_false_idx] =
                        Instruction::JumpIfFalse(else_start);

                    self.compile_body_expr(else_body);

                    let end = self.output.instructions.len();
                    self.output.instructions[jump_idx] = Instruction::Jump(end);
                } else {
                    let end = self.output.instructions.len();
                    self.output.instructions[jump_if_false_idx] = Instruction::JumpIfFalse(end);
                }
            }
            Statement::While { condition, body } => {
                let loop_start = self.output.instructions.len();

                self.compile_expr(condition);

                let exit_jump_idx = self.output.instructions.len();
                self.emit(Instruction::JumpIfFalse(0), &stmt_span);

                self.loops.push(LoopContext {
                    start: loop_start,
                    break_patches: Vec::new(),
                });

                self.compile_body_expr(body);

                self.emit(Instruction::Jump(loop_start), &stmt_span);

                let end = self.output.instructions.len();
                self.output.instructions[exit_jump_idx] = Instruction::JumpIfFalse(end);

                let loop_ctx = self.loops.pop().expect("loop context missing");
                for patch_idx in loop_ctx.break_patches {
                    self.output.instructions[patch_idx] = Instruction::Jump(end);
                }
            }
            Statement::Break => {
                if self.loops.is_empty() {
                    self.output
                        .errors
                        .push(OrynError::compiler(stmt_span, "break outside of loop"));
                } else {
                    let idx = self.output.instructions.len();
                    self.emit(Instruction::Jump(0), &stmt_span);
                    self.loops.last_mut().unwrap().break_patches.push(idx);
                }
            }
            Statement::Continue => {
                if let Some(loop_ctx) = self.loops.last() {
                    let start = loop_ctx.start;
                    self.emit(Instruction::Jump(start), &stmt_span);
                } else {
                    self.output
                        .errors
                        .push(OrynError::compiler(stmt_span, "continue outside of loop"));
                }
            }

            // -- Expression statements --
            Statement::Expression(expr) => {
                let expr_span = expr.span.clone();
                self.compile_expr(expr);
                self.emit(Instruction::Pop, &expr_span);
            }
        }
    }
}
