use std::collections::HashMap;

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{BinOp, Expression, Spanned, StringPart, UnaryOp};

use super::block::BlockMode;
use super::compile::Compiler;
use super::types::{BuiltinFunction, ConstValue, Instruction, ListMethod};

/// Walk a chain of `FieldAccess` nodes rooted in an `Ident` and collect
/// the path segments. Returns `Some(["math", "nested", "lib"])` for an
/// expression like `math.nested.lib`, or `None` if the expression contains
/// anything other than pure identifier/field-access.
fn extract_dotted_path(expr: &Expression) -> Option<Vec<String>> {
    match expr {
        Expression::Ident(name) => Some(vec![name.clone()]),
        Expression::FieldAccess { object, field } => {
            let mut path = extract_dotted_path(&object.node)?;
            path.push(field.clone());
            Some(path)
        }
        _ => None,
    }
}

#[derive(Clone)]
pub(super) enum FoldedValue {
    Bool(bool),
    Float(f32),
    Int(i32),
    Nil,
    String(String),
}

impl FoldedValue {
    fn resolved_type(&self) -> ResolvedType {
        match self {
            FoldedValue::Bool(_) => ResolvedType::Bool,
            FoldedValue::Float(_) => ResolvedType::Float,
            FoldedValue::Int(_) => ResolvedType::Int,
            FoldedValue::Nil => ResolvedType::Nil,
            FoldedValue::String(_) => ResolvedType::Str,
        }
    }

    fn to_instruction(&self) -> Instruction {
        match self {
            FoldedValue::Bool(v) => Instruction::PushBool(*v),
            FoldedValue::Float(v) => Instruction::PushFloat(*v),
            FoldedValue::Int(v) => Instruction::PushInt(*v),
            FoldedValue::Nil => Instruction::PushNil,
            FoldedValue::String(v) => Instruction::PushString(v.clone()),
        }
    }

    pub(super) fn to_const_value(&self) -> Option<ConstValue> {
        match self {
            FoldedValue::Bool(v) => Some(ConstValue::Bool(*v)),
            FoldedValue::Float(v) => Some(ConstValue::Float(*v)),
            FoldedValue::Int(v) => Some(ConstValue::Int(*v)),
            FoldedValue::String(v) => Some(ConstValue::String(v.clone())),
            FoldedValue::Nil => None,
        }
    }

    fn to_runtime_string(&self) -> String {
        match self {
            FoldedValue::Bool(v) => v.to_string(),
            FoldedValue::Float(v) => {
                let s = v.to_string();
                if s.contains('.') { s } else { format!("{s}.0") }
            }
            FoldedValue::Int(v) => v.to_string(),
            FoldedValue::Nil => "nil".to_string(),
            FoldedValue::String(v) => v.clone(),
        }
    }
}

impl Compiler {
    fn builtin_from_name(name: &str) -> Option<BuiltinFunction> {
        match name {
            "print" => Some(BuiltinFunction::Print),
            _ => None,
        }
    }

    fn module_const_from_path(
        &self,
        expr: &Expression,
        field: Option<&str>,
    ) -> Option<FoldedValue> {
        let path = extract_dotted_path(expr)?;
        let root = path.first()?;
        if self.locals.resolve(root).is_some() {
            return None;
        }

        match field {
            Some(field) => {
                let key = path.join(".");
                self.modules
                    .modules
                    .get(&key)
                    .and_then(|exports| exports.constants.get(field))
                    .map(|value| match value {
                        ConstValue::Int(v) => FoldedValue::Int(*v),
                        ConstValue::Float(v) => FoldedValue::Float(*v),
                        ConstValue::Bool(v) => FoldedValue::Bool(*v),
                        ConstValue::String(v) => FoldedValue::String(v.clone()),
                    })
            }
            None if path.len() == 1 => self
                .output
                .module_constants
                .get(root)
                .or_else(|| self.output.private_module_constants.get(root))
                .map(|value| match value {
                    ConstValue::Int(v) => FoldedValue::Int(*v),
                    ConstValue::Float(v) => FoldedValue::Float(*v),
                    ConstValue::Bool(v) => FoldedValue::Bool(*v),
                    ConstValue::String(v) => FoldedValue::String(v.clone()),
                }),
            None => None,
        }
    }

    pub(super) fn try_fold_expr(&self, expr: &Expression) -> Option<FoldedValue> {
        match expr {
            Expression::True => Some(FoldedValue::Bool(true)),
            Expression::False => Some(FoldedValue::Bool(false)),
            Expression::Float(v) => Some(FoldedValue::Float(*v)),
            Expression::Int(v) => Some(FoldedValue::Int(*v)),
            Expression::Nil => Some(FoldedValue::Nil),
            Expression::String(v) => Some(FoldedValue::String(v.clone())),
            Expression::Ident(_) => self.module_const_from_path(expr, None),
            Expression::FieldAccess { object, field } => {
                self.module_const_from_path(&object.node, Some(field))
            }
            Expression::StringInterp(parts) => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        StringPart::Literal(v) => result.push_str(v),
                        StringPart::Interp(expr) => {
                            result.push_str(&self.try_fold_expr(&expr.node)?.to_runtime_string());
                        }
                    }
                }
                Some(FoldedValue::String(result))
            }
            Expression::UnaryOp { op, expr } => {
                let value = self.try_fold_expr(&expr.node)?;
                match (op, value) {
                    (UnaryOp::Not, FoldedValue::Bool(v)) => Some(FoldedValue::Bool(!v)),
                    (UnaryOp::Negate, FoldedValue::Int(v)) => v.checked_neg().map(FoldedValue::Int),
                    (UnaryOp::Negate, FoldedValue::Float(v)) => Some(FoldedValue::Float(-v)),
                    _ => None,
                }
            }
            Expression::BinaryOp { op, left, right } => {
                let left = self.try_fold_expr(&left.node)?;
                match op {
                    BinOp::And => match left {
                        FoldedValue::Bool(false) => Some(FoldedValue::Bool(false)),
                        FoldedValue::Bool(true) => match self.try_fold_expr(&right.node)? {
                            FoldedValue::Bool(v) => Some(FoldedValue::Bool(v)),
                            _ => None,
                        },
                        _ => None,
                    },
                    BinOp::Or => match left {
                        FoldedValue::Bool(true) => Some(FoldedValue::Bool(true)),
                        FoldedValue::Bool(false) => match self.try_fold_expr(&right.node)? {
                            FoldedValue::Bool(v) => Some(FoldedValue::Bool(v)),
                            _ => None,
                        },
                        _ => None,
                    },
                    _ => {
                        let right = self.try_fold_expr(&right.node)?;
                        match (op, left, right) {
                            (BinOp::Add, FoldedValue::Int(l), FoldedValue::Int(r)) => {
                                l.checked_add(r).map(FoldedValue::Int)
                            }
                            (BinOp::Sub, FoldedValue::Int(l), FoldedValue::Int(r)) => {
                                l.checked_sub(r).map(FoldedValue::Int)
                            }
                            (BinOp::Mul, FoldedValue::Int(l), FoldedValue::Int(r)) => {
                                l.checked_mul(r).map(FoldedValue::Int)
                            }
                            (BinOp::Div, FoldedValue::Int(l), FoldedValue::Int(r)) if r != 0 => {
                                l.checked_div(r).map(FoldedValue::Int)
                            }
                            (BinOp::Add, FoldedValue::Float(l), FoldedValue::Float(r)) => {
                                Some(FoldedValue::Float(l + r))
                                    .filter(|v| matches!(v, FoldedValue::Float(f) if f.is_finite()))
                            }
                            (BinOp::Sub, FoldedValue::Float(l), FoldedValue::Float(r)) => {
                                Some(FoldedValue::Float(l - r))
                                    .filter(|v| matches!(v, FoldedValue::Float(f) if f.is_finite()))
                            }
                            (BinOp::Mul, FoldedValue::Float(l), FoldedValue::Float(r)) => {
                                Some(FoldedValue::Float(l * r))
                                    .filter(|v| matches!(v, FoldedValue::Float(f) if f.is_finite()))
                            }
                            (BinOp::Div, FoldedValue::Float(l), FoldedValue::Float(r))
                                if r != 0.0 =>
                            {
                                Some(FoldedValue::Float(l / r))
                                    .filter(|v| matches!(v, FoldedValue::Float(f) if f.is_finite()))
                            }
                            (BinOp::Equals, FoldedValue::Int(l), FoldedValue::Int(r)) => {
                                Some(FoldedValue::Bool(l == r))
                            }
                            (BinOp::Equals, FoldedValue::Float(l), FoldedValue::Float(r)) => {
                                Some(FoldedValue::Bool(l == r))
                            }
                            (BinOp::Equals, FoldedValue::Bool(l), FoldedValue::Bool(r)) => {
                                Some(FoldedValue::Bool(l == r))
                            }
                            (BinOp::Equals, FoldedValue::String(l), FoldedValue::String(r)) => {
                                Some(FoldedValue::Bool(l == r))
                            }
                            (BinOp::Equals, FoldedValue::Nil, FoldedValue::Nil) => {
                                Some(FoldedValue::Bool(true))
                            }
                            (BinOp::Equals, FoldedValue::Nil, _)
                            | (BinOp::Equals, _, FoldedValue::Nil) => {
                                Some(FoldedValue::Bool(false))
                            }
                            (BinOp::NotEquals, FoldedValue::Int(l), FoldedValue::Int(r)) => {
                                Some(FoldedValue::Bool(l != r))
                            }
                            (BinOp::NotEquals, FoldedValue::Float(l), FoldedValue::Float(r)) => {
                                Some(FoldedValue::Bool(l != r))
                            }
                            (BinOp::NotEquals, FoldedValue::Bool(l), FoldedValue::Bool(r)) => {
                                Some(FoldedValue::Bool(l != r))
                            }
                            (BinOp::NotEquals, FoldedValue::String(l), FoldedValue::String(r)) => {
                                Some(FoldedValue::Bool(l != r))
                            }
                            (BinOp::NotEquals, FoldedValue::Nil, FoldedValue::Nil) => {
                                Some(FoldedValue::Bool(false))
                            }
                            (BinOp::NotEquals, FoldedValue::Nil, _)
                            | (BinOp::NotEquals, _, FoldedValue::Nil) => {
                                Some(FoldedValue::Bool(true))
                            }
                            (BinOp::LessThan, FoldedValue::Int(l), FoldedValue::Int(r)) => {
                                Some(FoldedValue::Bool(l < r))
                            }
                            (BinOp::LessThan, FoldedValue::Float(l), FoldedValue::Float(r)) => {
                                Some(FoldedValue::Bool(l < r))
                            }
                            (BinOp::LessThan, FoldedValue::String(l), FoldedValue::String(r)) => {
                                Some(FoldedValue::Bool(l < r))
                            }
                            (BinOp::GreaterThan, FoldedValue::Int(l), FoldedValue::Int(r)) => {
                                Some(FoldedValue::Bool(l > r))
                            }
                            (BinOp::GreaterThan, FoldedValue::Float(l), FoldedValue::Float(r)) => {
                                Some(FoldedValue::Bool(l > r))
                            }
                            (
                                BinOp::GreaterThan,
                                FoldedValue::String(l),
                                FoldedValue::String(r),
                            ) => Some(FoldedValue::Bool(l > r)),
                            (BinOp::LessThanEquals, FoldedValue::Int(l), FoldedValue::Int(r)) => {
                                Some(FoldedValue::Bool(l <= r))
                            }
                            (
                                BinOp::LessThanEquals,
                                FoldedValue::Float(l),
                                FoldedValue::Float(r),
                            ) => Some(FoldedValue::Bool(l <= r)),
                            (
                                BinOp::LessThanEquals,
                                FoldedValue::String(l),
                                FoldedValue::String(r),
                            ) => Some(FoldedValue::Bool(l <= r)),
                            (
                                BinOp::GreaterThanEquals,
                                FoldedValue::Int(l),
                                FoldedValue::Int(r),
                            ) => Some(FoldedValue::Bool(l >= r)),
                            (
                                BinOp::GreaterThanEquals,
                                FoldedValue::Float(l),
                                FoldedValue::Float(r),
                            ) => Some(FoldedValue::Bool(l >= r)),
                            (
                                BinOp::GreaterThanEquals,
                                FoldedValue::String(l),
                                FoldedValue::String(r),
                            ) => Some(FoldedValue::Bool(l >= r)),
                            _ => None,
                        }
                    }
                }
            }
            Expression::Coalesce { left, right } => match self.try_fold_expr(&left.node)? {
                FoldedValue::Nil => self.try_fold_expr(&right.node),
                value => Some(value),
            },
            _ => None,
        }
    }

    /// Best-effort type inference for the receiver of a method call,
    /// used to enforce cross-module method privacy without actually
    /// compiling the receiver. Recognizes idents (resolved via locals)
    /// and field accesses (resolved via the obj_table or imported defs).
    pub(super) fn infer_object_type(&self, expr: &Expression) -> ResolvedType {
        match expr {
            Expression::Ident(name) => self
                .locals
                .resolve(name)
                .map(|(_, _, t)| t)
                .unwrap_or(ResolvedType::Unknown),
            Expression::FieldAccess { object, field } => {
                let parent_type = self.infer_object_type(&object.node);
                if let ResolvedType::Object { name, module } = parent_type {
                    let crosses = !module.is_empty() && module != self.current_module_path;
                    let def = if crosses {
                        let module_key = module.join(".");
                        self.modules
                            .modules
                            .get(&module_key)
                            .and_then(|e| e.obj_defs.get(&name))
                            .cloned()
                    } else {
                        self.obj_table.resolve(&name).map(|(_, d)| d.clone())
                    };
                    if let Some(def) = def
                        && let Some(idx) = def.fields.iter().position(|f| f == field)
                    {
                        return def
                            .field_types
                            .get(idx)
                            .cloned()
                            .unwrap_or(ResolvedType::Unknown);
                    }
                }
                ResolvedType::Unknown
            }
            Expression::ObjLiteral { type_name, .. } => {
                if type_name.is_empty() {
                    return ResolvedType::Unknown;
                }
                if type_name.len() == 1 {
                    let local_name = &type_name[0];
                    if self.obj_table.resolve(local_name).is_some() {
                        return ResolvedType::Object {
                            name: local_name.clone(),
                            module: self.current_module_path.clone(),
                        };
                    }
                } else if let Some((name, module)) = type_name.split_last() {
                    let key = module.join(".");
                    if self
                        .modules
                        .modules
                        .get(&key)
                        .and_then(|exports| exports.obj_defs.get(name))
                        .is_some()
                    {
                        return ResolvedType::Object {
                            name: name.clone(),
                            module: module.to_vec(),
                        };
                    }
                }
                ResolvedType::Unknown
            }
            // `xs[i]` has the list's element type. Used by the field-access
            // and method-call inference paths so `ps[0].x` and
            // `items[0].method()` resolve correctly.
            Expression::Index { object, .. } => match self.infer_object_type(&object.node) {
                ResolvedType::List(inner) => *inner,
                _ => ResolvedType::Unknown,
            },
            _ => ResolvedType::Unknown,
        }
    }

    /// Compile an object literal, supporting both bare type names like
    /// `Vec2 { x: 1.0 }` and qualified ones like `math.Vec2 { x: 1.0 }`.
    /// Returns the resulting type so the caller can plumb it into type
    /// checking and local variable shapes.
    pub(super) fn compile_obj_literal(
        &mut self,
        type_name: Vec<String>,
        fields: Vec<(String, Spanned<Expression>)>,
        span: &crate::parser::Span,
    ) -> ResolvedType {
        if type_name.is_empty() {
            self.output
                .errors
                .push(OrynError::compiler(span.clone(), "empty type path"));
            self.emit(Instruction::PushInt(0), span);
            return ResolvedType::Unknown;
        }

        // Single-segment: look up in local obj_table.
        if type_name.len() == 1 {
            let local_name = &type_name[0];
            if let Some((type_idx, def)) = self.obj_table.resolve(local_name) {
                let def_fields = def.fields.clone();
                let num_fields = def_fields.len();

                for (fname, _) in &fields {
                    if !def_fields.contains(fname) {
                        self.output.errors.push(OrynError::compiler(
                            span.clone(),
                            format!("unknown field `{fname}` on type `{local_name}`"),
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
                            format!("missing field `{def_field}` in `{local_name}` literal"),
                        ));
                        self.emit(Instruction::PushInt(0), span);
                    }
                }

                self.emit(Instruction::NewObject(type_idx, num_fields), span);
                return ResolvedType::Object {
                    name: local_name.clone(),
                    module: self.current_module_path.clone(),
                };
            } else {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("undefined type `{local_name}`"),
                ));
                self.emit(Instruction::PushInt(0), span);
                return ResolvedType::Unknown;
            }
        }

        // Multi-segment: split into module path + type name, look up
        // the imported obj_def via ModuleExports.
        let (last, prefix) = type_name.split_last().unwrap();
        let module_key = prefix.join(".");

        let exports = match self.modules.modules.get(&module_key) {
            Some(e) => e,
            None => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("undefined module `{module_key}`"),
                ));
                self.emit(Instruction::PushInt(0), span);
                return ResolvedType::Unknown;
            }
        };

        let imported_def = match exports.obj_defs.get(last) {
            Some(d) => d.clone(),
            None => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("undefined type `{module_key}.{last}`"),
                ));
                self.emit(Instruction::PushInt(0), span);
                return ResolvedType::Unknown;
            }
        };

        let type_idx = match exports.objects.get(last) {
            Some(&idx) => idx,
            None => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("type `{module_key}.{last}` is not pub"),
                ));
                self.emit(Instruction::PushInt(0), span);
                return ResolvedType::Unknown;
            }
        };

        let def_fields = imported_def.fields.clone();
        let def_field_pub = imported_def.field_is_pub.clone();
        let num_fields = def_fields.len();

        // Cross-module construction: types with any private fields cannot
        // be constructed via literal from outside the defining module.
        // The defining module must expose a `pub fn new(...)` constructor.
        let has_private_field = def_field_pub.iter().any(|p| !*p);
        if has_private_field {
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                format!(
                    "type `{module_key}.{last}` has private fields and cannot be constructed via literal from outside module `{module_key}`"
                ),
            ));
            for (_, value) in fields {
                self.compile_expr(value);
            }
            self.emit(Instruction::PushInt(0), span);
            return ResolvedType::Unknown;
        }

        // Every named field in the literal must exist on the type.
        for (fname, _) in &fields {
            if !def_fields.iter().any(|f| f == fname) {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("unknown field `{fname}` on type `{module_key}.{last}`"),
                ));
            }
        }

        // All fields must be supplied.
        let mut field_map: HashMap<String, Spanned<Expression>> = fields.into_iter().collect();

        for def_field in &def_fields {
            if let Some(value) = field_map.remove(def_field) {
                self.compile_expr(value);
            } else {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("missing field `{def_field}` in `{module_key}.{last}` literal"),
                ));
                self.emit(Instruction::PushInt(0), span);
            }
        }

        self.emit(Instruction::NewObject(type_idx, num_fields), span);
        ResolvedType::Object {
            name: last.clone(),
            module: prefix.to_vec(),
        }
    }

    /// Compile a method call on a list receiver. Dispatches against the
    /// [`ListMethod`] table: look up the method by name, compile the
    /// receiver and arguments, type-check each argument against the
    /// method's parameter types (with `SelfElement` replaced by the
    /// receiver's element type), emit a single
    /// [`Instruction::CallListMethod`], and return the method's
    /// declared return type.
    ///
    /// Adding a new list method is a one-place change in the table —
    /// no changes needed here.
    pub(super) fn compile_list_method(
        &mut self,
        object: Spanned<Expression>,
        method: &str,
        args: Vec<Spanned<Expression>>,
        elem_ty: &ResolvedType,
        span: &crate::parser::Span,
    ) -> ResolvedType {
        let list_method = match ListMethod::from_name(method) {
            Some(m) => m,
            None => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("unknown list method `{method}` (expected `len`, `push`, or `pop`)"),
                ));
                self.compile_expr(object);
                for arg in args {
                    self.compile_expr(arg);
                }
                self.emit(Instruction::PushInt(0), span);
                return ResolvedType::Unknown;
            }
        };

        let expected_params = list_method.param_types(elem_ty);
        let arity = args.len();

        if arity != expected_params.len() {
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                format!(
                    "list `{}` takes {} argument(s), got {arity}",
                    list_method.name(),
                    expected_params.len()
                ),
            ));
            // Compile the receiver and args anyway so any nested errors
            // still surface, then push a sentinel to keep stack discipline.
            self.compile_expr(object);
            for arg in args {
                self.compile_expr(arg);
            }
            self.emit(Instruction::PushInt(0), span);
            return ResolvedType::Unknown;
        }

        // Receiver first (method-call stack convention).
        self.compile_expr(object);

        for (i, (arg, expected_ty)) in args.into_iter().zip(expected_params.iter()).enumerate() {
            let arg_span = arg.span.clone();
            let arg_ty = self.compile_expr(arg);
            self.check_types(
                expected_ty,
                &arg_ty,
                &arg_span,
                &format!(
                    "list `{}` argument {} type mismatch",
                    list_method.name(),
                    i + 1
                ),
            );
        }

        self.emit(
            Instruction::CallListMethod(list_method as u8, arity as u8),
            span,
        );

        list_method.return_type(elem_ty)
    }
}

// ---------------------------------------------------------------------------
// Expression compilation
// ---------------------------------------------------------------------------

impl Compiler {
    pub(super) fn compile_expr(&mut self, expr: Spanned<Expression>) -> ResolvedType {
        let span = expr.span.clone();

        if let Some(folded) = self.try_fold_expr(&expr.node) {
            self.emit(folded.to_instruction(), &span);
            return folded.resolved_type();
        }

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
            Expression::StringInterp(parts) => {
                let num_parts = parts.len();
                for part in parts {
                    match part {
                        StringPart::Literal(s) => self.emit(Instruction::PushString(s), &span),
                        StringPart::Interp(expr) => {
                            let returned_type = self.compile_expr(expr);
                            if returned_type != ResolvedType::Str {
                                self.emit(Instruction::ToString, &span);
                            }
                        }
                    }
                }
                self.emit(Instruction::Concat(num_parts as u8), &span);
                ResolvedType::Str
            }

            // -- Variables --
            Expression::Ident(name) => {
                if let Some((slot, _, resolved_type)) = self.locals.resolve(&name) {
                    self.emit(Instruction::GetLocal(slot), &span);

                    resolved_type
                } else if let Some(const_value) = self
                    .output
                    .module_constants
                    .get(&name)
                    .or_else(|| self.output.private_module_constants.get(&name))
                    .cloned()
                {
                    // In-module reference to a `let` / `val` constant
                    // declared earlier at top level. Inline the literal
                    // directly so module bodies can use their own
                    // constants — both exported and private — by bare name.
                    let instr = const_value.to_instruction();
                    let result_type = const_value.resolved_type();
                    self.emit(instr, &span);
                    result_type
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
                self.compile_obj_literal(type_name, fields, &span)
            }
            Expression::FieldAccess { object, field } => {
                if let Some(path) = extract_dotted_path(&object.node)
                    && self.locals.resolve(&path[0]).is_none()
                {
                    let key = path.join(".");
                    if let Some(exports) = self.modules.modules.get(&key)
                        && !exports.functions.contains_key(&field)
                        && !exports.constants.contains_key(&field)
                    {
                        self.output.errors.push(OrynError::compiler(
                            span.clone(),
                            format!("undefined constant `{key}.{field}`"),
                        ));
                        self.emit(Instruction::PushInt(0), &span);
                        return ResolvedType::Unknown;
                    }
                }

                let obj_type = self.infer_object_type(&object.node);

                self.compile_expr(*object);

                if let Some(field_idx) = self.resolve_field(&obj_type, &field, &span) {
                    self.emit(Instruction::GetField(field_idx), &span);
                    match obj_type {
                        ResolvedType::Object {
                            ref name,
                            ref module,
                        } if module.is_empty() || *module == self.current_module_path => self
                            .obj_table
                            .resolve(name)
                            .and_then(|(_, def)| def.field_types.get(field_idx).cloned())
                            .unwrap_or(ResolvedType::Unknown),
                        ResolvedType::Object {
                            ref name,
                            ref module,
                        } => self
                            .modules
                            .modules
                            .get(&module.join("."))
                            .and_then(|exports| exports.obj_defs.get(name))
                            .and_then(|def| def.field_types.get(field_idx).cloned())
                            .unwrap_or(ResolvedType::Unknown),
                        _ => ResolvedType::Unknown,
                    }
                } else {
                    self.emit(Instruction::PushInt(0), &span);
                    ResolvedType::Unknown
                }
            }
            Expression::MethodCall {
                object,
                method,
                args,
            } => {
                let arity = args.len();

                // Walk the receiver as a dotted path. Three cases worth
                // distinguishing:
                //   1. Path matches a module → module function call.
                //   2. Path[..n-1] is a module and path[n-1] is a type
                //      in that module → cross-module static method call.
                //   3. Anything else → fall through to runtime dispatch.
                let dotted = extract_dotted_path(&object.node);

                let module_call = match &dotted {
                    Some(path) if !path.is_empty() => {
                        let root = &path[0];
                        if self.locals.resolve(root).is_none() {
                            let key = path.join(".");
                            if self.modules.modules.contains_key(&key) {
                                Some(key)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                // Cross-module static method call: split the path so the
                // last segment is the type and the prefix is the module.
                let qualified_static = if module_call.is_none() {
                    match &dotted {
                        Some(path) if path.len() >= 2 => {
                            let root = &path[0];
                            if self.locals.resolve(root).is_none() {
                                let (type_name, module_path) = path.split_last().unwrap();
                                let module_key = module_path.join(".");
                                self.modules
                                    .modules
                                    .get(&module_key)
                                    .and_then(|e| e.obj_defs.get(type_name).cloned())
                                    .map(|def| (module_key, type_name.clone(), def))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                } else {
                    None
                };

                if let Some((module_key, type_name, def)) = qualified_static {
                    if let Some(&func_idx) = def.static_methods.get(&method) {
                        let is_pub = def
                            .static_method_is_pub
                            .get(&method)
                            .copied()
                            .unwrap_or(false);
                        if !is_pub {
                            self.output.errors.push(OrynError::compiler(
                                span.clone(),
                                format!(
                                    "static method `{type_name}.{method}` is private to module `{module_key}`"
                                ),
                            ));
                        }

                        let signature = def.static_method_signatures.get(&method).cloned();

                        let mut arg_types = Vec::new();
                        for arg in args {
                            arg_types.push(self.compile_expr(arg));
                        }

                        if let Some(ref sig) = signature {
                            if arity != sig.param_types.len() {
                                self.output.errors.push(OrynError::compiler(
                                    span.clone(),
                                    format!(
                                        "arity mismatch: expected {} arguments, got {}",
                                        sig.param_types.len(),
                                        arity
                                    ),
                                ));
                            }
                            for (i, (arg_type, param_type)) in
                                arg_types.iter().zip(&sig.param_types).enumerate()
                            {
                                self.check_types(
                                    param_type,
                                    arg_type,
                                    &span,
                                    &format!("argument {} type mismatch", i + 1),
                                );
                            }
                        }

                        self.emit(Instruction::Call(func_idx, arity), &span);
                        return signature
                            .map(|s| s.return_type)
                            .unwrap_or(ResolvedType::Unknown);
                    } else {
                        self.output.errors.push(OrynError::compiler(
                            span.clone(),
                            format!("undefined static method `{module_key}.{type_name}.{method}`"),
                        ));
                        self.emit(Instruction::PushInt(0), &span);
                        return ResolvedType::Unknown;
                    }
                }

                if let Some(module_name) = module_call {
                    let exports = self.modules.modules.get(&module_name).unwrap();
                    if let Some(&func_idx) = exports.functions.get(&method) {
                        let signature = exports.fn_signatures.get(&method).cloned();

                        let mut arg_types = Vec::new();
                        for arg in args {
                            arg_types.push(self.compile_expr(arg));
                        }

                        if let Some(ref sig) = signature {
                            if arity != sig.param_types.len() {
                                self.output.errors.push(OrynError::compiler(
                                    span.clone(),
                                    format!(
                                        "arity mismatch: expected {} arguments, got {}",
                                        sig.param_types.len(),
                                        arity
                                    ),
                                ));
                            }
                            for (i, (arg_type, param_type)) in
                                arg_types.iter().zip(&sig.param_types).enumerate()
                            {
                                self.check_types(
                                    param_type,
                                    arg_type,
                                    &span,
                                    &format!("argument {} type mismatch", i + 1),
                                );
                            }
                        }

                        self.emit(Instruction::Call(func_idx, arity), &span);

                        return signature
                            .map(|s| s.return_type)
                            .unwrap_or(ResolvedType::Unknown);
                    } else {
                        self.output.errors.push(OrynError::compiler(
                            span.clone(),
                            format!("undefined function `{module_name}.{method}`"),
                        ));
                        self.emit(Instruction::PushInt(0), &span);
                        return ResolvedType::Unknown;
                    }
                }

                let static_call = match &object.node {
                    Expression::Ident(type_name)
                        if self.locals.resolve(type_name).is_none()
                            && self.obj_table.resolve(type_name).is_some() =>
                    {
                        Some((
                            type_name.clone(),
                            self.obj_table
                                .resolve(type_name)
                                .and_then(|(_, def)| def.static_methods.get(&method).copied()),
                        ))
                    }
                    _ => None,
                };

                if let Some((type_name, static_info)) = static_call {
                    if let Some(func_idx) = static_info {
                        // func_idx is absolute; convert to local to index
                        // into the current compilation unit's functions.
                        let local = self.local_fn_idx(func_idx);
                        let (param_types, return_type) = {
                            let func = &self.output.functions[local];

                            (func.param_types.clone(), func.return_type.clone())
                        };

                        let mut arg_types = Vec::new();
                        for arg in args {
                            arg_types.push(self.compile_expr(arg));
                        }

                        if arity != param_types.len() {
                            self.output.errors.push(OrynError::compiler(
                                span.clone(),
                                format!(
                                    "arity mismatch: expected {} arguments, got {}",
                                    param_types.len(),
                                    arity
                                ),
                            ));
                        }
                        for (i, (arg_type, param_type)) in
                            arg_types.iter().zip(&param_types).enumerate()
                        {
                            self.check_types(
                                param_type,
                                arg_type,
                                &span,
                                &format!("argument {} type mismatch", i + 1),
                            );
                        }

                        self.emit(Instruction::Call(func_idx, arity), &span);

                        return_type.unwrap_or(ResolvedType::Unknown)
                    } else {
                        self.output.errors.push(OrynError::compiler(
                            span.clone(),
                            format!("undefined static method `{}.{method}`", type_name),
                        ));

                        self.emit(Instruction::PushInt(0), &span);

                        ResolvedType::Unknown
                    }
                } else {
                    let receiver_type = self.infer_object_type(&object.node);

                    let direct_method = match &receiver_type {
                        ResolvedType::Object { name, module }
                            if module.is_empty() || *module == self.current_module_path =>
                        {
                            self.obj_table.resolve(name).map(|(_, def)| {
                                (
                                    name.clone(),
                                    module.clone(),
                                    def.methods.get(&method).copied(),
                                    def.method_is_pub.get(&method).copied().unwrap_or(true),
                                    def.method_signatures.get(&method).cloned(),
                                )
                            })
                        }
                        ResolvedType::Object { name, module } => {
                            let module_key = module.join(".");
                            self.modules
                                .modules
                                .get(&module_key)
                                .and_then(|exports| exports.obj_defs.get(name))
                                .map(|def| {
                                    (
                                        name.clone(),
                                        module.clone(),
                                        def.methods.get(&method).copied(),
                                        def.method_is_pub.get(&method).copied().unwrap_or(false),
                                        def.method_signatures.get(&method).cloned(),
                                    )
                                })
                        }
                        _ => None,
                    };

                    // List receiver: hard-coded builtin methods (`len`,
                    // `push`, `pop`). Dispatched before the object-method
                    // lookup so users can't accidentally shadow them.
                    if let ResolvedType::List(elem_ty) = receiver_type.clone() {
                        return self.compile_list_method(*object, &method, args, &elem_ty, &span);
                    }

                    if let Some((type_name, module, func_idx, is_pub, signature)) = direct_method {
                        if !module.is_empty() && module != self.current_module_path && !is_pub {
                            self.output.errors.push(OrynError::compiler(
                                span.clone(),
                                format!(
                                    "method `{method}` is private to module `{}`",
                                    module.join(".")
                                ),
                            ));
                        }

                        if let Some(func_idx) = func_idx {
                            self.compile_expr(*object);

                            let mut arg_types = Vec::new();
                            for arg in args {
                                arg_types.push(self.compile_expr(arg));
                            }

                            if let Some(ref sig) = signature {
                                if arity != sig.param_types.len() {
                                    self.output.errors.push(OrynError::compiler(
                                        span.clone(),
                                        format!(
                                            "arity mismatch: expected {} arguments, got {}",
                                            sig.param_types.len(),
                                            arity
                                        ),
                                    ));
                                }
                                for (i, (arg_type, param_type)) in
                                    arg_types.iter().zip(&sig.param_types).enumerate()
                                {
                                    self.check_types(
                                        param_type,
                                        arg_type,
                                        &span,
                                        &format!("argument {} type mismatch", i + 1),
                                    );
                                }
                            }

                            self.emit(Instruction::Call(func_idx, arity + 1), &span);
                            return signature
                                .map(|s| s.return_type)
                                .unwrap_or(ResolvedType::Unknown);
                        }

                        let qualified = if module.is_empty() {
                            format!("{type_name}.{method}")
                        } else {
                            format!("{}.{}.{}", module.join("."), type_name, method)
                        };
                        self.output.errors.push(OrynError::compiler(
                            span.clone(),
                            format!("undefined method `{qualified}`"),
                        ));
                        self.emit(Instruction::PushInt(0), &span);
                        return ResolvedType::Unknown;
                    }

                    self.compile_expr(*object);

                    for arg in args {
                        self.compile_expr(arg);
                    }

                    self.emit(Instruction::CallMethod(method, arity), &span);

                    ResolvedType::Unknown
                }
            }

            // -- Operators --
            Expression::BinaryOp { op, left, right } => {
                // Short-circuit `and` / `or` — must be handled before the
                // general path so the RHS is only compiled on the taken branch.
                if matches!(op, BinOp::And | BinOp::Or) {
                    let left_span = left.span.clone();
                    let right_span = right.span.clone();

                    let left_type = self.compile_expr(*left);
                    self.check_types(
                        &ResolvedType::Bool,
                        &left_type,
                        &left_span,
                        "logical operand must be `bool`",
                    );

                    let jump_if_false_idx = self.output.instructions.len();
                    self.emit(Instruction::JumpIfFalse(0), &span);

                    match op {
                        BinOp::And => {
                            // LHS was true; evaluate RHS as the result.
                            let right_type = self.compile_expr(*right);
                            self.check_types(
                                &ResolvedType::Bool,
                                &right_type,
                                &right_span,
                                "logical operand must be `bool`",
                            );
                            let jump_to_end_idx = self.output.instructions.len();
                            self.emit(Instruction::Jump(0), &span);

                            let false_target = self.output.instructions.len();
                            self.emit(Instruction::PushBool(false), &span);
                            let end_addr = self.output.instructions.len();

                            self.output.instructions[jump_if_false_idx] =
                                Instruction::JumpIfFalse(false_target);
                            self.output.instructions[jump_to_end_idx] = Instruction::Jump(end_addr);
                        }
                        BinOp::Or => {
                            // LHS was true — the result is true; skip RHS.
                            self.emit(Instruction::PushBool(true), &span);
                            let jump_to_end_idx = self.output.instructions.len();
                            self.emit(Instruction::Jump(0), &span);

                            let rhs_target = self.output.instructions.len();
                            let right_type = self.compile_expr(*right);
                            self.check_types(
                                &ResolvedType::Bool,
                                &right_type,
                                &right_span,
                                "logical operand must be `bool`",
                            );
                            let end_addr = self.output.instructions.len();

                            self.output.instructions[jump_if_false_idx] =
                                Instruction::JumpIfFalse(rhs_target);
                            self.output.instructions[jump_to_end_idx] = Instruction::Jump(end_addr);
                        }
                        _ => unreachable!(),
                    }

                    return ResolvedType::Bool;
                }

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
                        BinOp::Add => Instruction::Add,
                        BinOp::Sub => Instruction::Sub,
                        BinOp::Mul => Instruction::Mul,
                        BinOp::Div => Instruction::Div,
                        BinOp::And | BinOp::Or => unreachable!("handled above"),
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
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => left_type,
                    BinOp::And | BinOp::Or => unreachable!("handled above"),
                }
            }
            Expression::Range {
                start,
                end,
                inclusive,
            } => {
                let start_type = self.compile_expr(*start);
                let end_type = self.compile_expr(*end);

                self.check_types(
                    &ResolvedType::Int,
                    &start_type,
                    &span,
                    "range start type mismatch",
                );

                self.check_types(
                    &ResolvedType::Int,
                    &end_type,
                    &span,
                    "range end type mismatch",
                );

                self.emit(Instruction::MakeRange(inclusive), &span);

                ResolvedType::Range
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
                // `Error("message")` — builtin error constructor.
                if name == "Error" {
                    if args.len() != 1 {
                        self.output.errors.push(crate::OrynError::compiler(
                            span.clone(),
                            format!("`Error` expects exactly 1 argument, got {}", args.len()),
                        ));

                        self.emit(Instruction::PushInt(0), &span);

                        return ResolvedType::Error;
                    }

                    let arg_type = self.compile_expr(args.into_iter().next().unwrap());

                    self.check_types(
                        &ResolvedType::Str,
                        &arg_type,
                        &span,
                        "`Error` argument must be a String",
                    );

                    self.emit(Instruction::MakeError, &span);

                    return ResolvedType::Error;
                }

                let arity = args.len();

                let mut arg_types = Vec::new();
                for arg in args {
                    let arg_type = self.compile_expr(arg);
                    arg_types.push(arg_type);
                }

                if let Some(sig) = self.fn_table.signatures.get(&name) {
                    if arity != sig.param_types.len() {
                        self.output.errors.push(OrynError::compiler(
                            span.clone(),
                            format!(
                                "arity mismatch: expected {} arguments, got {}",
                                sig.param_types.len(),
                                arity
                            ),
                        ));
                    }
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
                } else if let Some(builtin) = Self::builtin_from_name(&name) {
                    self.emit(Instruction::CallBuiltin(builtin, arity), &span);
                } else {
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!("undefined function `{name}`"),
                    ));
                    self.emit(Instruction::PushInt(0), &span);
                }

                self.fn_table
                    .signatures
                    .get(&name)
                    .map(|sig| sig.return_type.clone())
                    .unwrap_or(ResolvedType::Unknown)
            }

            // -- Blocks --
            Expression::Nil => {
                self.emit(Instruction::PushNil, &span);

                ResolvedType::Nil
            }

            Expression::Try(inner_expr) => {
                let inner_type = self.compile_expr(*inner_expr);

                let success_type = match inner_type.unwrap_error_union() {
                    Some(t) => {
                        // Verify the enclosing function returns an error union.
                        if let Some(ref return_type) = self.locals.return_type
                            && !return_type.is_error_union()
                            && !matches!(return_type, ResolvedType::Unknown)
                        {
                            self.output.errors.push(crate::OrynError::compiler(
                                    span.clone(),
                                    format!(
                                        "`try` requires the enclosing function to return an error union type, but it returns `{}`",
                                        return_type.display_name()
                                    ),
                                ));
                        }
                        t.clone()
                    }
                    None => {
                        if !matches!(inner_type, ResolvedType::Unknown) {
                            self.output.errors.push(crate::OrynError::compiler(
                                span.clone(),
                                format!(
                                    "`try` requires an error union type, got `{}`",
                                    inner_type.display_name()
                                ),
                            ));
                        }

                        ResolvedType::Unknown
                    }
                };

                // JumpIfError(propagate) — if error, leave on stack and jump
                let jump_if_error_idx = self.output.instructions.len();
                self.emit(Instruction::JumpIfError(0), &span);

                // Success path: value on stack, skip propagation
                let jump_to_end_idx = self.output.instructions.len();
                self.emit(Instruction::Jump(0), &span);

                // Propagation path: error is on TOS, return it
                let propagate_addr = self.output.instructions.len();
                self.output.instructions[jump_if_error_idx] =
                    Instruction::JumpIfError(propagate_addr);
                self.emit(Instruction::Return, &span);

                let end_addr = self.output.instructions.len();
                self.output.instructions[jump_to_end_idx] = Instruction::Jump(end_addr);

                success_type
            }

            Expression::UnwrapError(inner_expr) => {
                let inner_type = self.compile_expr(*inner_expr);

                let success_type = match inner_type.unwrap_error_union() {
                    Some(t) => t.clone(),
                    None => {
                        if !matches!(inner_type, ResolvedType::Unknown) {
                            self.output.errors.push(crate::OrynError::compiler(
                                span.clone(),
                                format!(
                                    "`!` unwrap requires an error union type, got `{}`",
                                    inner_type.display_name()
                                ),
                            ));
                        }

                        ResolvedType::Unknown
                    }
                };

                self.emit(Instruction::UnwrapErrorOrTrap, &span);

                success_type
            }

            Expression::Coalesce { left, right } => {
                let left_type = self.compile_expr(*left);

                let inner_type = match left_type.unwrap_nillable() {
                    Some(inner) => inner.clone(),
                    None => {
                        if !matches!(left_type, ResolvedType::Unknown) {
                            self.output.errors.push(crate::OrynError::compiler(
                                span.clone(),
                                format!(
                                    "`orelse` requires a nillable type on the left, got `{}`",
                                    left_type.display_name()
                                ),
                            ));
                        }
                        ResolvedType::Unknown
                    }
                };

                // JumpIfNil(fallback) — if nil, pop it and jump to fallback
                let jump_if_nil_idx = self.output.instructions.len();
                self.emit(Instruction::JumpIfNil(0), &span);

                // Not nil: value is on the stack, skip fallback
                let jump_to_end_idx = self.output.instructions.len();
                self.emit(Instruction::Jump(0), &span);

                // Fallback: nil was popped, compile the right-hand side
                let fallback_addr = self.output.instructions.len();
                self.output.instructions[jump_if_nil_idx] = Instruction::JumpIfNil(fallback_addr);

                let right_type = self.compile_expr(*right);
                self.check_types(
                    &inner_type,
                    &right_type,
                    &span,
                    "orelse fallback type mismatch",
                );

                let end_addr = self.output.instructions.len();
                self.output.instructions[jump_to_end_idx] = Instruction::Jump(end_addr);

                inner_type
            }

            Expression::Block(stmts) => self.compile_block(stmts, BlockMode::FreshLoops),

            // -- Lists --
            Expression::ListLiteral(elements) => {
                if elements.is_empty() {
                    // Empty literals carry no element type. The
                    // surrounding context (typically `compile_binding`)
                    // reconciles this against any declared annotation
                    // and reports "cannot infer" if none is present.
                    self.emit(Instruction::MakeList(0), &span);
                    return ResolvedType::List(Box::new(ResolvedType::Unknown));
                }

                let count = elements.len();
                let mut elements = elements.into_iter();
                let first = elements.next().unwrap();
                let elem_ty = self.compile_expr(first);

                for element in elements {
                    let element_span = element.span.clone();
                    let actual_ty = self.compile_expr(element);
                    self.check_types(
                        &elem_ty,
                        &actual_ty,
                        &element_span,
                        "list element type mismatch",
                    );
                }

                self.emit(Instruction::MakeList(count as u32), &span);
                ResolvedType::List(Box::new(elem_ty))
            }

            Expression::Index { object, index } => {
                let object_span = object.span.clone();
                let object_ty = self.compile_expr(*object);

                let element_ty = match &object_ty {
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
                let index_ty = self.compile_expr(*index);
                self.check_types(
                    &ResolvedType::Int,
                    &index_ty,
                    &index_span,
                    "list index must be `int`",
                );

                self.emit(Instruction::ListGet, &span);
                element_ty
            }
        }
    }
}
