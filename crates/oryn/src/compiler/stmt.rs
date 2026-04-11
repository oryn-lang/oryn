use std::collections::HashMap;

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{Expression, Span, Spanned, Statement, TypeAnnotation};

use super::compile::{Compiler, LoopContext};
use super::func::FunctionBodyConfig;
use super::tables::BindingKind;
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
        kind: BindingKind,
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
        if declared_type.is_none()
            && let ResolvedType::Map(key, value) = &inferred_type
            && (matches!(**key, ResolvedType::Unknown) || matches!(**value, ResolvedType::Unknown))
        {
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                "cannot infer key/value types of empty map literal; add a type annotation like `let m: {string: int} = {}`",
            ));
        }

        if let Some(ref decl) = declared_type {
            self.check_types(decl, &inferred_type, span, "type mismatch");
        }

        let resolved = declared_type.unwrap_or(inferred_type);
        self.output.type_map.insert(span.clone(), &resolved);
        let slot = self.locals.define(name, kind, resolved);
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
                    self.compile_binding(name, value, type_ann, BindingKind::Let, &stmt_span);
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
                    self.compile_binding(name, value, type_ann, BindingKind::Val, &stmt_span);
                }
            }

            // -- Assignments --
            Statement::Assignment { name, value } => {
                // Reject `self = ...` unconditionally — even inside a
                // `mut fn`. The local for self is conceptually a
                // receiver borrow; rebinding it would silently no-op
                // for the caller. See WARTS.md mutability cluster.
                if name == "self" {
                    self.output.errors.push(OrynError::compiler(
                        stmt_span.clone(),
                        "cannot reassign `self`; assign to its fields instead",
                    ));
                    let _ = self.compile_expr(value);
                    return;
                }

                let value_type = self.compile_expr(value);

                if let Some(entry) = self.locals.resolve(&name) {
                    if !entry.is_mutable() {
                        self.output.errors.push(OrynError::compiler(
                            stmt_span.clone(),
                            immutability_error(&name, &entry.kind, "reassign"),
                        ));
                    }

                    self.check_types(
                        &entry.obj_type,
                        &value_type,
                        &stmt_span,
                        "assignment type mismatch",
                    );

                    self.emit(Instruction::SetLocal(entry.slot), &stmt_span);
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
                if let Some(name) = field_assignment_root_name(&object.node)
                    && let Some(entry) = self.locals.resolve(name)
                    && !entry.is_mutable()
                {
                    self.output.errors.push(OrynError::compiler(
                        stmt_span.clone(),
                        immutability_error(name, &entry.kind, "mutate indexed value of"),
                    ));
                }

                let object_span = object.span.clone();
                let object_ty = self.compile_expr(object);

                let (key_ty, value_slot_ty, instruction, key_message, value_message) =
                    match &object_ty {
                        ResolvedType::List(inner) => (
                            ResolvedType::Int,
                            (**inner).clone(),
                            Instruction::ListSet,
                            "list index must be `int`",
                            "list element type mismatch",
                        ),
                        ResolvedType::Map(key, value) => (
                            (**key).clone(),
                            (**value).clone(),
                            Instruction::MapSet,
                            "map key type mismatch",
                            "map value type mismatch",
                        ),
                        ResolvedType::Unknown => (
                            ResolvedType::Unknown,
                            ResolvedType::Unknown,
                            Instruction::ListSet,
                            "index type mismatch",
                            "indexed assignment value type mismatch",
                        ),
                        _ => {
                            self.output.errors.push(OrynError::compiler(
                                object_span,
                                format!(
                                    "cannot index into non-list/map type `{}`",
                                    object_ty.display_name()
                                ),
                            ));
                            (
                                ResolvedType::Unknown,
                                ResolvedType::Unknown,
                                Instruction::ListSet,
                                "index type mismatch",
                                "indexed assignment value type mismatch",
                            )
                        }
                    };

                let index_span = index.span.clone();
                let index_ty = self.compile_expr(index);
                self.check_types(&key_ty, &index_ty, &index_span, key_message);

                let value_span = value.span.clone();
                let value_ty = self.compile_expr(value);
                self.check_types(&value_slot_ty, &value_ty, &value_span, value_message);

                self.emit(instruction, &stmt_span);
            }
            Statement::FieldAssignment {
                object,
                field,
                value,
            } => {
                // Source-accurate immutability check: walk the lvalue
                // back to its root binding and consult its kind. The
                // parser only produces FieldAccess/Index chains rooted
                // at an Ident, so the root must be in `locals`. If
                // we can't find it (compiler bug or future parser
                // change), default to permissive — the type checker
                // will catch undefined variables anyway.
                if let Some(name) = field_assignment_root_name(&object.node)
                    && let Some(entry) = self.locals.resolve(name)
                    && !entry.is_mutable()
                {
                    self.output.errors.push(OrynError::compiler(
                        stmt_span.clone(),
                        immutability_error(name, &entry.kind, "mutate field of"),
                    ));
                }

                let obj_type = self.infer_object_type(&object.node);

                self.compile_expr(object);
                self.compile_expr(value);

                if self.resolve_field(&obj_type, &field, &stmt_span).is_some() {
                    self.emit(Instruction::SetField(field), &stmt_span);
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
                    .map(|p| {
                        let t = p
                            .type_ann
                            .as_ref()
                            .map(|a| {
                                self.resolve_type_annotation(a)
                                    .unwrap_or(ResolvedType::Unknown)
                            })
                            .unwrap_or(ResolvedType::Unknown);
                        (p.name.clone(), t)
                    })
                    .collect();

                let param_types: Vec<ResolvedType> = params
                    .iter()
                    .map(|p| {
                        resolved_params
                            .get(&p.name)
                            .cloned()
                            .unwrap_or(ResolvedType::Unknown)
                    })
                    .collect();

                let param_fn = move |p: &crate::parser::Param| {
                    let resolved = resolved_params
                        .get(&p.name)
                        .cloned()
                        .unwrap_or(ResolvedType::Unknown);
                    // `mut x: T` opts the parameter into mutability.
                    // Without `mut`, top-level function params are
                    // always immutable in Oryn (no opt-out at the
                    // call site).
                    let kind = if p.is_mut {
                        BindingKind::MutParam
                    } else {
                        BindingKind::Param
                    };
                    (kind, resolved)
                };

                for param in &params {
                    if param.type_ann.is_none() {
                        self.output.errors.push(OrynError::compiler(
                            stmt_span.clone(),
                            format!("parameter `{}` requires a type annotation", param.name),
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
                    is_mut: false,
                    pre_allocated_local_idx: None,
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

            Statement::EnumDef {
                name,
                variants,
                is_pub,
            } => {
                self.compile_enum_def(name, variants, &stmt_span, is_pub);
            }

            // -- Control flow --
            //
            // `if` and `if let` are now expressions (Slice 5 W26
            // lift); they reach the compiler via `Statement::Expression`,
            // handled below.
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

                let param_fn =
                    |_: &crate::parser::Param| (BindingKind::Param, ResolvedType::Unknown);

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
                    is_mut: false,
                    pre_allocated_local_idx: None,
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
        let range_slot = self.locals.define(
            "@for_range".to_string(),
            BindingKind::Internal,
            ResolvedType::Range,
        );
        self.emit(Instruction::SetLocal(range_slot), stmt_span);

        let item_slot = self
            .locals
            .define(name, BindingKind::ForIndex, ResolvedType::Int);

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
        let list_slot = self
            .locals
            .define("@for_list".to_string(), BindingKind::Internal, list_ty);
        self.emit(Instruction::SetLocal(list_slot), stmt_span);

        // @for_idx = -1 so the first iteration increments to 0 cleanly.
        let idx_slot = self.locals.define(
            "@for_idx".to_string(),
            BindingKind::Internal,
            ResolvedType::Int,
        );
        self.emit(Instruction::PushInt(-1), stmt_span);
        self.emit(Instruction::SetLocal(idx_slot), stmt_span);

        // @for_len = @for_list.len() — cached once, not per iteration.
        let len_slot = self.locals.define(
            "@for_len".to_string(),
            BindingKind::Internal,
            ResolvedType::Int,
        );
        self.emit(Instruction::GetLocal(list_slot), stmt_span);
        self.emit(
            Instruction::CallListMethod(ListMethod::Len as u8, 0),
            stmt_span,
        );
        self.emit(Instruction::SetLocal(len_slot), stmt_span);

        let item_slot = self.locals.define(name, BindingKind::ForIndex, elem_ty);

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

fn field_assignment_root_name(expr: &Expression) -> Option<&str> {
    match expr {
        Expression::Ident(name) => Some(name),
        Expression::FieldAccess { object, .. } | Expression::Index { object, .. } => {
            field_assignment_root_name(&object.node)
        }
        _ => None,
    }
}

/// Source-accurate phrasing for an immutability rejection. Used by
/// every assignment / mutating-method site so the error message
/// reflects *why* the binding is immutable, not just the catch-all
/// "val binding". W24 fix.
///
/// `op` describes what the user tried to do, in a phrase that fits
/// after "cannot " — e.g. "reassign", "mutate field of",
/// "call mutating method `push` on".
pub(super) fn immutability_error(name: &str, kind: &BindingKind, op: &str) -> String {
    // `self` inside a method that takes plain `self` (not `mut self`)
    // is bound with kind `Param` for enforcement purposes, but the
    // user-facing message should explain the actual rule (the
    // method's receiver isn't declared `mut self`) rather than
    // calling `self` a parameter.
    if name == "self" {
        return format!(
            "cannot {op} `self` in a non-mutating method; declare the method's receiver as `mut self` to allow mutation"
        );
    }
    match kind {
        BindingKind::Val => {
            format!("cannot {op} val binding `{name}`")
        }
        BindingKind::Param => {
            format!(
                "cannot {op} parameter `{name}`; parameters are immutable (mark with `mut` to allow mutation)"
            )
        }
        BindingKind::ForIndex => {
            format!("cannot {op} for-loop variable `{name}`")
        }
        BindingKind::SelfRef => {
            // Reaching here for `self` would mean SelfRef is mutable
            // (it is) and yet an immutability check fired — i.e. a
            // compiler bug. Phrase generically.
            format!("cannot {op} `{name}`")
        }
        BindingKind::Let | BindingKind::MutParam | BindingKind::Internal => {
            // Mutable kinds shouldn't reach this function, but if they
            // do (compiler bug) we still emit something coherent
            // rather than panicking.
            format!("cannot {op} `{name}` (compiler bug: mutable kind reached immutability_error)")
        }
    }
}
