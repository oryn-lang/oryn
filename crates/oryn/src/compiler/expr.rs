use std::collections::HashMap;

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{BinOp, Expression, Spanned, UnaryOp};

use super::block::BlockMode;
use super::compile::Compiler;
use super::types::Instruction;

// ---------------------------------------------------------------------------
// Expression compilation
// ---------------------------------------------------------------------------

impl Compiler {
    pub(super) fn compile_expr(&mut self, expr: Spanned<Expression>) -> ResolvedType {
        let span = expr.span.clone();

        match expr.node {
            // -- Literals --
            Expression::True => {
                self.emit(Instruction::PushBool(true), &span);
                ResolvedType::Bool
            }
            Expression::False => {
                self.emit(Instruction::PushBool(false), &span);
                ResolvedType::Bool
            }
            Expression::Float(n) => {
                self.emit(Instruction::PushFloat(n), &span);
                ResolvedType::Float
            }
            Expression::Int(n) => {
                self.emit(Instruction::PushInt(n), &span);
                ResolvedType::Int
            }
            Expression::String(s) => {
                self.emit(Instruction::PushString(s), &span);
                ResolvedType::Str
            }

            // -- Variables --
            Expression::Ident(name) => {
                if let Some((slot, _, resolved_type)) = self.locals.resolve(&name) {
                    self.emit(Instruction::GetLocal(slot), &span);
                    resolved_type
                } else {
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!("undefined variable `{name}`"),
                    ));
                    self.emit(Instruction::PushInt(0), &span);
                    ResolvedType::Unknown
                }
            }

            // -- Objects --
            Expression::ObjLiteral { type_name, fields } => {
                if let Some((type_idx, def)) = self.obj_table.resolve(&type_name) {
                    let def_fields = def.fields.clone();
                    let num_fields = def_fields.len();

                    for (name, _) in &fields {
                        if !def_fields.contains(name) {
                            self.output.errors.push(OrynError::compiler(
                                span.clone(),
                                format!("unknown field `{name}` on type `{type_name}`"),
                            ));
                        }
                    }

                    let mut field_map: HashMap<String, Spanned<Expression>> =
                        fields.into_iter().collect();

                    for def_field in &def_fields {
                        if let Some(value) = field_map.remove(def_field) {
                            self.compile_expr(value);
                        } else {
                            self.output.errors.push(OrynError::compiler(
                                span.clone(),
                                format!("missing field `{def_field}` in `{type_name}` literal"),
                            ));
                            self.emit(Instruction::PushInt(0), &span);
                        }
                    }

                    self.emit(Instruction::NewObject(type_idx, num_fields), &span);
                    ResolvedType::Object(type_name)
                } else {
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!("undefined type `{type_name}`"),
                    ));
                    self.emit(Instruction::PushInt(0), &span);
                    ResolvedType::Unknown
                }
            }
            Expression::FieldAccess { object, field } => {
                let obj_type = match &object.node {
                    Expression::Ident(name) => self
                        .locals
                        .resolve(name)
                        .map(|(_, _, t)| t)
                        .unwrap_or(ResolvedType::Unknown),
                    _ => ResolvedType::Unknown,
                };

                self.compile_expr(*object);

                if let Some(field_idx) = self.resolve_field(&obj_type, &field, &span) {
                    self.emit(Instruction::GetField(field_idx), &span);
                } else {
                    self.emit(Instruction::PushInt(0), &span);
                }

                ResolvedType::Unknown
            }
            Expression::MethodCall {
                object,
                method,
                args,
            } => {
                self.compile_expr(*object);

                let arity = args.len();
                for arg in args {
                    self.compile_expr(arg);
                }

                self.emit(Instruction::CallMethod(method, arity), &span);
                ResolvedType::Unknown
            }

            // -- Operators --
            Expression::BinaryOp { op, left, right } => {
                let left_type = self.compile_expr(*left);
                self.compile_expr(*right);

                self.emit(
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
                    BinOp::Equals
                    | BinOp::NotEquals
                    | BinOp::LessThan
                    | BinOp::GreaterThan
                    | BinOp::LessThanEquals
                    | BinOp::GreaterThanEquals => ResolvedType::Bool,
                    BinOp::And | BinOp::Or => ResolvedType::Bool,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => left_type,
                }
            }
            Expression::UnaryOp { op, expr: operand } => {
                let operand_type = self.compile_expr(*operand);

                self.emit(
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
                    let arg_type = self.compile_expr(arg);
                    arg_types.push(arg_type);
                }

                if let Some(sig) = self.fn_table.signatures.get(&name) {
                    let sig_params = sig.param_types.clone();
                    for (i, (arg_type, param_type)) in arg_types.iter().zip(&sig_params).enumerate()
                    {
                        self.check_types(
                            param_type,
                            arg_type,
                            &span,
                            &format!("argument {} type mismatch", i + 1),
                        );
                    }
                }

                if let Some(idx) = self.fn_table.resolve(&name) {
                    self.emit(Instruction::Call(idx, arity), &span);
                } else {
                    self.emit(Instruction::CallBuiltin(name.clone(), arity), &span);
                }

                self.fn_table
                    .signatures
                    .get(&name)
                    .map(|sig| sig.return_type.clone())
                    .unwrap_or(ResolvedType::Unknown)
            }

            // -- Blocks --
            Expression::Block(stmts) => self.compile_block(stmts, BlockMode::FreshLoops),
        }
    }
}
