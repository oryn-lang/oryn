use std::collections::HashMap;

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{Expression, Span, Spanned, Statement, TypeAnnotation};

use super::compile::{Compiler, LoopContext};
use super::func::FunctionBodyConfig;
use super::types::{Instruction, ListMethod};

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

        // An empty list literal produces `List(Unknown)`; without a
        // declared type there's nothing to reconcile against, which
        // would leave the user with a silently-Unknown element type.
        // Require an annotation in that case.
        if declared_type.is_none()
            && let ResolvedType::List(inner) = &inferred_type
            && matches!(**inner, ResolvedType::Unknown)
        {
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                "cannot infer element type of empty list literal; add a type annotation like `let xs: [int] = []`",
            ));
        }

        if let Some(ref decl) = declared_type {
            self.check_types(decl, &inferred_type, span, "type mismatch");
        }

        let resolved = declared_type.unwrap_or(inferred_type);
        self.output.type_map.insert(span.clone(), &resolved);
        let slot = self.locals.define(name, mutable, resolved);
        self.emit(Instruction::SetLocal(slot), span);
    }

    /// Extract a module-level `let` / `val` binding as a literal constant.
    /// `pub` bindings are stored in `output.module_constants` (and exported
    /// via [`ModuleExports`]); non-`pub` bindings are stored in
    /// `output.private_module_constants` and remain visible only to code in
    /// the same module. Non-literal values produce a compile error —
    /// modules are definitions-only and cannot execute expressions at
    /// import time.
    pub(super) fn extract_module_constant(
        &mut self,
        name: String,
        value: Spanned<Expression>,
        is_pub: bool,
        span: &Span,
    ) {
        let const_value = self
            .try_fold_expr(&value.node)
            .and_then(|value| value.to_const_value());

        match const_value {
            Some(v) => {
                if is_pub {
                    self.output.module_constants.insert(name, v);
                } else {
                    self.output.private_module_constants.insert(name, v);
                }
            }
            None => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!(
                        "module-level binding `{name}` must be a literal value (int, float, bool, or string)"
                    ),
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Statement compilation
// ---------------------------------------------------------------------------

impl Compiler {
    fn compile_conditional(
        &mut self,
        condition: Spanned<Expression>,
        body: Spanned<Expression>,
        else_body: Option<Spanned<Expression>>,
        run_body_on_false: bool,
        stmt_span: &Span,
    ) {
        self.compile_expr(condition);

        let branch_jump_idx = self.output.instructions.len();
        self.emit(Instruction::JumpIfFalse(0), stmt_span);

        if run_body_on_false {
            if let Some(else_body) = else_body {
                self.compile_body_expr(else_body);

                let jump_idx = self.output.instructions.len();
                self.emit(Instruction::Jump(0), stmt_span);

                let body_start = self.output.instructions.len();
                self.output.instructions[branch_jump_idx] = Instruction::JumpIfFalse(body_start);

                self.compile_body_expr(body);

                let end = self.output.instructions.len();
                self.output.instructions[jump_idx] = Instruction::Jump(end);
            } else {
                let jump_idx = self.output.instructions.len();
                self.emit(Instruction::Jump(0), stmt_span);

                let body_start = self.output.instructions.len();
                self.output.instructions[branch_jump_idx] = Instruction::JumpIfFalse(body_start);

                self.compile_body_expr(body);

                let end = self.output.instructions.len();
                self.output.instructions[jump_idx] = Instruction::Jump(end);
            }
        } else {
            self.compile_body_expr(body);

            if let Some(else_body) = else_body {
                let jump_idx = self.output.instructions.len();
                self.emit(Instruction::Jump(0), stmt_span);

                let else_start = self.output.instructions.len();
                self.output.instructions[branch_jump_idx] = Instruction::JumpIfFalse(else_start);

                self.compile_body_expr(else_body);

                let end = self.output.instructions.len();
                self.output.instructions[jump_idx] = Instruction::Jump(end);
            } else {
                let end = self.output.instructions.len();
                self.output.instructions[branch_jump_idx] = Instruction::JumpIfFalse(end);
            }
        }
    }

    pub(super) fn compile_stmt(&mut self, stmt: Spanned<Statement>) {
        let stmt_span = stmt.span.clone();

        match stmt.node {
            // -- Bindings --
            Statement::Let {
                name,
                value,
                type_ann,
                is_pub,
            } => {
                // In modules, let/val bindings must be literal values and
                // are extracted as module constants. They are NOT also bound
                // as runtime locals since modules are definitions-only.
                // `pub` bindings are exported; non-`pub` bindings are still
                // visible to code inside the same module but are not exported.
                if self.is_module() {
                    self.extract_module_constant(name, value, is_pub, &stmt_span);
                } else {
                    self.compile_binding(name, value, type_ann, true, &stmt_span);
                }
            }
            Statement::Val {
                name,
                value,
                type_ann,
                is_pub,
            } => {
                if self.is_module() {
                    self.extract_module_constant(name, value, is_pub, &stmt_span);
                } else {
                    self.compile_binding(name, value, type_ann, false, &stmt_span);
                }
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
            Statement::IndexAssignment {
                object,
                index,
                value,
            } => {
                let object_span = object.span.clone();
                let object_ty = self.compile_expr(object);

                let elem_ty = match &object_ty {
                    ResolvedType::List(inner) => (**inner).clone(),
                    ResolvedType::Unknown => ResolvedType::Unknown,
                    _ => {
                        self.output.errors.push(OrynError::compiler(
                            object_span,
                            format!(
                                "cannot index into non-list type `{}`",
                                object_ty.display_name()
                            ),
                        ));
                        ResolvedType::Unknown
                    }
                };

                let index_span = index.span.clone();
                let index_ty = self.compile_expr(index);
                self.check_types(
                    &ResolvedType::Int,
                    &index_ty,
                    &index_span,
                    "list index must be `int`",
                );

                let value_span = value.span.clone();
                let value_ty = self.compile_expr(value);
                self.check_types(
                    &elem_ty,
                    &value_ty,
                    &value_span,
                    "list element type mismatch",
                );

                self.emit(Instruction::ListSet, &stmt_span);
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
                is_pub,
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
                    None => ResolvedType::Unknown,
                };

                self.output
                    .type_map
                    .insert(stmt_span.clone(), &return_resolved);

                self.compile_function_body(FunctionBodyConfig {
                    name: &name,
                    params: &params,
                    param_types,
                    param_local_fn: &param_fn,
                    self_name: Some(&name),
                    body,
                    span: &stmt_span,
                    return_type: Some(return_resolved),
                    is_pub,
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
                is_pub,
            } => {
                self.compile_obj_def(name, fields, methods, uses, &stmt_span, is_pub);
            }

            // -- Control flow --
            Statement::If {
                condition,
                body,
                else_body,
            } => self.compile_conditional(condition, body, else_body, false, &stmt_span),
            Statement::Unless {
                condition,
                body,
                else_body,
            } => self.compile_conditional(condition, body, else_body, true, &stmt_span),
            Statement::While { condition, body } => {
                let loop_start = self.output.instructions.len();

                self.compile_expr(condition);

                let exit_jump_idx = self.output.instructions.len();
                self.emit(Instruction::JumpIfFalse(0), &stmt_span);

                self.loops.push(LoopContext {
                    continue_target: loop_start,
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
            Statement::For {
                name,
                iterable,
                body,
            } => {
                self.with_scope(|this| {
                    let iterable_span = iterable.span.clone();
                    let iterable_type = this.compile_expr(iterable);

                    match iterable_type.clone() {
                        ResolvedType::Range => {
                            this.compile_for_range(name, body, &stmt_span);
                        }
                        ResolvedType::List(elem_ty) => {
                            this.compile_for_list(name, *elem_ty, body, &stmt_span);
                        }
                        ResolvedType::Unknown => {
                            // Upstream error already reported; skip codegen.
                        }
                        other => {
                            this.output.errors.push(OrynError::compiler(
                                iterable_span,
                                format!(
                                    "for loop iterable must be a range or list, got `{}`",
                                    other.display_name()
                                ),
                            ));
                        }
                    }
                });
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
                    self.emit(Instruction::Jump(loop_ctx.continue_target), &stmt_span);
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

            Statement::Import { .. } => {}

            // -- Tests --
            Statement::Test { name, body } => {
                // Tests lower to zero-arity, non-public functions. They
                // are recorded in `output.tests` so the runner can
                // invoke each one directly; the synthetic function
                // name prevents user code from calling them.
                let synthetic_name = format!("__test_{}", self.output.tests.len());
                let fn_span = stmt_span.clone();

                let param_fn = |_: &str, _: &Option<TypeAnnotation>| (false, ResolvedType::Unknown);

                let function_idx = self.compile_function_body(FunctionBodyConfig {
                    name: &synthetic_name,
                    params: &[],
                    param_types: Vec::new(),
                    param_local_fn: &param_fn,
                    self_name: None,
                    body,
                    span: &fn_span,
                    return_type: None,
                    is_pub: false,
                });

                self.output.tests.push(crate::compiler::TestInfo {
                    display_name: name,
                    function_idx,
                    span: stmt_span,
                });
            }

            Statement::Assert { condition } => {
                let cond_span = condition.span.clone();
                let cond_type = self.compile_expr(condition);

                // The condition must be a bool. Unknown (inference gap)
                // passes silently; anything else produces a clear compile
                // error instead of letting the VM raise a generic type
                // error at runtime.
                self.check_types(
                    &ResolvedType::Bool,
                    &cond_type,
                    &cond_span,
                    "assert condition type mismatch",
                );

                self.emit(Instruction::Assert, &cond_span);
            }

            Statement::IfLet {
                name,
                value,
                body,
                else_body,
            } => {
                let scrutinee_type = self.compile_expr(value);

                let inner_type = match scrutinee_type.unwrap_nillable() {
                    Some(inner) => inner.clone(),
                    None => {
                        if !matches!(scrutinee_type, ResolvedType::Unknown) {
                            self.output.errors.push(crate::OrynError::compiler(
                                stmt_span.clone(),
                                format!(
                                    "`if let` requires a nillable type, got `{}`",
                                    scrutinee_type.display_name()
                                ),
                            ));
                        }
                        ResolvedType::Unknown
                    }
                };

                let jump_if_nil_idx = self.output.instructions.len();
                self.emit(Instruction::JumpIfNil(0), &stmt_span);

                // Then-branch: introduce `name: T` in a new scope.
                self.with_scope(|this| {
                    let slot = this.locals.define(name, false, inner_type);
                    this.emit(Instruction::SetLocal(slot), &stmt_span);
                    this.compile_body_expr(body);
                });

                if let Some(else_body) = else_body {
                    let jump_idx = self.output.instructions.len();
                    self.emit(Instruction::Jump(0), &stmt_span);

                    let else_start = self.output.instructions.len();
                    self.output.instructions[jump_if_nil_idx] = Instruction::JumpIfNil(else_start);

                    self.compile_body_expr(else_body);

                    let end = self.output.instructions.len();
                    self.output.instructions[jump_idx] = Instruction::Jump(end);
                } else {
                    let end = self.output.instructions.len();
                    self.output.instructions[jump_if_nil_idx] = Instruction::JumpIfNil(end);
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // For-loop codegen helpers
    // -----------------------------------------------------------------
    //
    // Both helpers assume the iterable's value has already been pushed
    // on the stack by the caller. They manage the full loop skeleton,
    // including `break` / `continue` patching, and leave nothing on the
    // stack after the loop exits.

    /// Emit bytecode for `for name in <range> { body }`. The range
    /// value is on the stack when this is called.
    fn compile_for_range(&mut self, name: String, body: Spanned<Expression>, stmt_span: &Span) {
        let range_slot = self
            .locals
            .define("@for_range".to_string(), false, ResolvedType::Range);
        self.emit(Instruction::SetLocal(range_slot), stmt_span);

        let item_slot = self.locals.define(name, false, ResolvedType::Int);

        let loop_start = self.output.instructions.len();
        self.emit(Instruction::GetLocal(range_slot), stmt_span);
        self.emit(Instruction::RangeHasNext, stmt_span);

        let exit_jump_idx = self.output.instructions.len();
        self.emit(Instruction::JumpIfFalse(0), stmt_span);

        self.emit(Instruction::GetLocal(range_slot), stmt_span);
        self.emit(Instruction::RangeNext, stmt_span);
        self.emit(Instruction::SetLocal(item_slot), stmt_span);

        self.loops.push(LoopContext {
            continue_target: loop_start,
            break_patches: Vec::new(),
        });

        self.compile_body_expr(body);
        self.emit(Instruction::Jump(loop_start), stmt_span);

        let end = self.output.instructions.len();
        self.output.instructions[exit_jump_idx] = Instruction::JumpIfFalse(end);

        let loop_ctx = self.loops.pop().expect("loop context missing");
        for patch_idx in loop_ctx.break_patches {
            self.output.instructions[patch_idx] = Instruction::Jump(end);
        }
    }

    /// Emit bytecode for `for name in <list> { body }`. The list value
    /// is on the stack when this is called. The loop variable binds
    /// with the list's element type so the body can index into nested
    /// lists or access fields on obj instances without annotation.
    ///
    /// Layout (all using existing opcodes — no new VM work):
    /// ```text
    ///   @for_list = <popped from stack>
    ///   @for_idx  = -1     ; pre-decrement so the first step lands at 0
    ///   @for_len  = @for_list.len()
    /// loop_start:
    ///   @for_idx  = @for_idx + 1
    ///   if @for_idx < @for_len: fall through; else break
    ///   item      = @for_list[@for_idx]
    ///   ...body...
    ///   jump loop_start
    /// end:
    /// ```
    ///
    /// `continue_target = loop_start` so `continue` re-runs the
    /// increment and the bounds check — standard for-each semantics.
    fn compile_for_list(
        &mut self,
        name: String,
        elem_ty: ResolvedType,
        body: Spanned<Expression>,
        stmt_span: &Span,
    ) {
        let list_ty = ResolvedType::List(Box::new(elem_ty.clone()));
        let list_slot = self.locals.define("@for_list".to_string(), false, list_ty);
        self.emit(Instruction::SetLocal(list_slot), stmt_span);

        // @for_idx = -1 so the first iteration increments to 0 cleanly.
        let idx_slot = self
            .locals
            .define("@for_idx".to_string(), false, ResolvedType::Int);
        self.emit(Instruction::PushInt(-1), stmt_span);
        self.emit(Instruction::SetLocal(idx_slot), stmt_span);

        // @for_len = @for_list.len() — cached once, not per iteration.
        let len_slot = self
            .locals
            .define("@for_len".to_string(), false, ResolvedType::Int);
        self.emit(Instruction::GetLocal(list_slot), stmt_span);
        self.emit(
            Instruction::CallListMethod(ListMethod::Len as u8, 0),
            stmt_span,
        );
        self.emit(Instruction::SetLocal(len_slot), stmt_span);

        let item_slot = self.locals.define(name, false, elem_ty);

        let loop_start = self.output.instructions.len();

        // @for_idx = @for_idx + 1
        self.emit(Instruction::GetLocal(idx_slot), stmt_span);
        self.emit(Instruction::PushInt(1), stmt_span);
        self.emit(Instruction::Add, stmt_span);
        self.emit(Instruction::SetLocal(idx_slot), stmt_span);

        // if @for_idx < @for_len { fall through } else { break }
        self.emit(Instruction::GetLocal(idx_slot), stmt_span);
        self.emit(Instruction::GetLocal(len_slot), stmt_span);
        self.emit(Instruction::LessThan, stmt_span);

        let exit_jump_idx = self.output.instructions.len();
        self.emit(Instruction::JumpIfFalse(0), stmt_span);

        // item = @for_list[@for_idx]
        self.emit(Instruction::GetLocal(list_slot), stmt_span);
        self.emit(Instruction::GetLocal(idx_slot), stmt_span);
        self.emit(Instruction::ListGet, stmt_span);
        self.emit(Instruction::SetLocal(item_slot), stmt_span);

        self.loops.push(LoopContext {
            continue_target: loop_start,
            break_patches: Vec::new(),
        });

        self.compile_body_expr(body);
        self.emit(Instruction::Jump(loop_start), stmt_span);

        let end = self.output.instructions.len();
        self.output.instructions[exit_jump_idx] = Instruction::JumpIfFalse(end);

        let loop_ctx = self.loops.pop().expect("loop context missing");
        for patch_idx in loop_ctx.break_patches {
            self.output.instructions[patch_idx] = Instruction::Jump(end);
        }
    }
}
