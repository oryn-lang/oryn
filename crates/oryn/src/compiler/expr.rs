use std::collections::{HashMap, HashSet};

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{
    BinOp, Expression, Param, Span, Spanned, Statement, StringPart, TypeAnnotation, UnaryOp,
};

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
                .map(|entry| entry.obj_type)
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
                ResolvedType::Map(_, value) => ResolvedType::Nillable(value),
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

        // Two-segment with payload: dispatch to enum constructor
        // when the path matches `EnumName.VariantName` for a known
        // enum. The user wrote `FsResult.Ok { content: "hi" }`,
        // which the parser produced as ObjLiteral with type_name
        // `["FsResult", "Ok"]`. Cross-module enum constructors
        // (`some_module.SomeEnum.Variant`) aren't supported in
        // Slice 1+2 — only the local two-segment form.
        if type_name.len() == 2
            && self
                .enum_table
                .resolve_variant(&type_name[0], &type_name[1])
                .is_some()
        {
            return self.compile_enum_constructor(
                &type_name[0].clone(),
                &type_name[1].clone(),
                fields,
                span,
            );
        }

        // Single-segment: look up in local obj_table.
        if type_name.len() == 1 {
            let local_name = &type_name[0];
            if let Some((type_idx, def)) = self.obj_table.resolve(local_name) {
                let def_fields = def.fields.clone();
                let def_field_types = def.field_types.clone();
                let num_fields = def_fields.len();

                let mut seen_fields = HashSet::new();
                for (fname, _) in &fields {
                    if !seen_fields.insert(fname.clone()) {
                        self.output.errors.push(OrynError::compiler(
                            span.clone(),
                            format!("duplicate field `{fname}` in `{local_name}` literal"),
                        ));
                    }
                    if !def_fields.contains(fname) {
                        self.output.errors.push(OrynError::compiler(
                            span.clone(),
                            format!("unknown field `{fname}` on type `{local_name}`"),
                        ));
                    }
                }

                let mut field_map: HashMap<String, Spanned<Expression>> =
                    fields.into_iter().collect();

                for (field_idx, def_field) in def_fields.iter().enumerate() {
                    if let Some(value) = field_map.remove(def_field) {
                        let value_span = value.span.clone();
                        let value_type = self.compile_expr(value);
                        let expected_type = def_field_types
                            .get(field_idx)
                            .cloned()
                            .unwrap_or(ResolvedType::Unknown);
                        self.check_types(
                            &expected_type,
                            &value_type,
                            &value_span,
                            &format!("field `{def_field}` type mismatch"),
                        );
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
        let def_field_types = imported_def.field_types.clone();
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
        let mut seen_fields = HashSet::new();
        for (fname, _) in &fields {
            if !seen_fields.insert(fname.clone()) {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("duplicate field `{fname}` in `{module_key}.{last}` literal"),
                ));
            }
            if !def_fields.iter().any(|f| f == fname) {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!("unknown field `{fname}` on type `{module_key}.{last}`"),
                ));
            }
        }

        // All fields must be supplied.
        let mut field_map: HashMap<String, Spanned<Expression>> = fields.into_iter().collect();

        for (field_idx, def_field) in def_fields.iter().enumerate() {
            if let Some(value) = field_map.remove(def_field) {
                let value_span = value.span.clone();
                let value_type = self.compile_expr(value);
                let expected_type = def_field_types
                    .get(field_idx)
                    .cloned()
                    .unwrap_or(ResolvedType::Unknown);
                self.check_types(
                    &expected_type,
                    &value_type,
                    &value_span,
                    &format!("field `{def_field}` type mismatch"),
                );
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

        if matches!(list_method, ListMethod::Push | ListMethod::Pop)
            && let Some(name) = expression_root_name(&object.node)
            && let Some(entry) = self.locals.resolve(name)
            && !entry.is_mutable()
        {
            let op = format!("call mutating method `{}` on", list_method.name());
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                super::stmt::immutability_error(name, &entry.kind, &op),
            ));
        }

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

fn expression_root_name(expr: &Expression) -> Option<&str> {
    match expr {
        Expression::Ident(name) => Some(name),
        Expression::FieldAccess { object, .. } | Expression::Index { object, .. } => {
            expression_root_name(&object.node)
        }
        _ => None,
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
                if let Some(entry) = self.locals.resolve(&name) {
                    self.emit(Instruction::GetLocal(entry.slot), &span);

                    entry.obj_type
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
                } else if let Some(idx) = self.fn_table.resolve(&name) {
                    // First-class function reference: a bare identifier
                    // that resolves to a top-level function in a value
                    // position becomes a `MakeFunction(idx)` instruction
                    // pushing a `Value::Function(idx)` onto the stack.
                    // The result type is `Function { params, return }`
                    // derived from the function's signature.
                    self.emit(Instruction::MakeFunction(idx), &span);
                    if let Some(sig) = self.fn_table.signatures.get(&name) {
                        ResolvedType::Function {
                            params: sig.param_types.clone(),
                            return_type: Box::new(sig.return_type.clone()),
                        }
                    } else {
                        ResolvedType::Unknown
                    }
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
                // Enum constructor for nullary variants: `Color.Red`
                // parses as FieldAccess { object: Ident("Color"),
                // field: "Red" }, and if `Color` is a known enum,
                // this isn't a real field access — it's a
                // constructor expression. Detect that here BEFORE
                // attempting to compile `object` as a value (it
                // isn't one). If the variant doesn't exist on the
                // enum, we still dispatch into the constructor path
                // so it can produce a clean "no variant" error
                // instead of leaking "undefined variable Color".
                if let Expression::Ident(enum_name) = &object.node
                    && self.locals.resolve(enum_name).is_none()
                    && self.enum_table.resolve(enum_name).is_some()
                {
                    return self.compile_enum_constructor(
                        &enum_name.clone(),
                        &field,
                        Vec::new(),
                        &span,
                    );
                }

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
                    self.emit(Instruction::GetField(field.clone()), &span);
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

                        // -- Mutability checks for `mut fn` calls --
                        // Done before compiling the receiver/args so the
                        // errors point at the right span.
                        let callee_is_mut = signature.as_ref().map(|s| s.is_mut).unwrap_or(false);
                        if callee_is_mut {
                            // Rule 1: a `mut fn` cannot be called on a
                            // `val`-rooted receiver. Walks the receiver
                            // expression back to its root binding and
                            // consults its kind.
                            if let Some(root_name) = expression_root_name(&object.node)
                                && let Some(entry) = self.locals.resolve(root_name)
                                && !entry.is_mutable()
                            {
                                let op = format!("call mutating method `{method}` on");
                                self.output.errors.push(OrynError::compiler(
                                    span.clone(),
                                    super::stmt::immutability_error(root_name, &entry.kind, &op),
                                ));
                            }
                            // Rule 2: a plain `fn` method cannot call a
                            // `mut fn` method on `self`. The current
                            // function's mutability is tracked on the
                            // Compiler. If the receiver is `self` and
                            // the current function isn't a `mut fn`, the
                            // call would let mutation flow through a
                            // method whose contract said it wouldn't.
                            if let Some(root_name) = expression_root_name(&object.node)
                                && root_name == "self"
                                && !self.current_fn_is_mut
                            {
                                self.output.errors.push(OrynError::compiler(
                                    span.clone(),
                                    format!(
                                        "cannot call mutating method `{method}` from a non-mutating method; declare the enclosing method's receiver as `mut self` to allow mutation"
                                    ),
                                ));
                            }
                        }

                        if let Some(func_idx) = func_idx {
                            self.compile_expr(*object);

                            let mut arg_types = Vec::new();
                            let mut arg_root_names: Vec<Option<String>> = Vec::new();
                            for arg in args {
                                // Capture each argument's root name (if
                                // any) so the val-into-mut-param check
                                // can consult the locals table after
                                // the type check.
                                arg_root_names
                                    .push(expression_root_name(&arg.node).map(|s| s.to_string()));
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
                                // Rule 3: a `mut` parameter cannot
                                // accept a `val`-rooted argument. The
                                // argument's mutation rights would
                                // exceed the caller's, breaking val's
                                // promise.
                                for (i, root) in arg_root_names.iter().enumerate() {
                                    let param_wants_mut =
                                        sig.param_is_mut.get(i).copied().unwrap_or(false);
                                    if param_wants_mut
                                        && let Some(name) = root
                                        && let Some(entry) = self.locals.resolve(name)
                                        && !entry.is_mutable()
                                    {
                                        let op = format!(
                                            "pass to mut parameter {} of `{method}`",
                                            i + 1
                                        );
                                        self.output.errors.push(OrynError::compiler(
                                            span.clone(),
                                            super::stmt::immutability_error(name, &entry.kind, &op),
                                        ));
                                    }
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
            //
            // Three dispatch shapes, picked by inspecting the call's
            // target:
            //
            //   1. Direct named call — target is a bare `Ident` whose
            //      name resolves to a top-level function or builtin.
            //      Emits the fast `Call(idx, arity)` or
            //      `CallBuiltin(b, arity)` instruction. This is the
            //      hot path that every existing call site takes.
            //
            //   2. Local-bound function value — target is a bare
            //      `Ident` that resolves to a local of type
            //      `Function { ... }` (e.g., `let f = double; f(21)`).
            //      Compiles the local read to push the function value,
            //      then emits `CallValue(arity)` for indirect dispatch.
            //
            //   3. General indirect call — target is any other
            //      expression (parenthesized callable, anonymous
            //      function, field access holding a function, etc.).
            //      Compiles the target normally and emits
            //      `CallValue(arity)`.
            Expression::Call { target, args } => {
                let arity = args.len();

                // Try the direct-name fast path first.
                if let Expression::Ident(name) = &target.node {
                    let name = name.clone();

                    // Direct call to a top-level function or builtin.
                    // Take this path even if a local with the same
                    // name exists in scope — the existing semantics
                    // shadow locals only for value reads, not for
                    // calls.
                    let is_top_level_fn = self.fn_table.resolve(&name).is_some();
                    let is_builtin = Self::builtin_from_name(&name).is_some();

                    if is_top_level_fn || is_builtin {
                        return self.compile_direct_named_call(name, args, &span);
                    }

                    // Not a top-level fn or builtin — try resolving
                    // as a local with function type. If it's a local
                    // but not function-typed, we still fall through
                    // to the generic indirect path so the type checker
                    // can produce the right error.
                    if let Some(entry) = self.locals.resolve(&name) {
                        let local_type = entry.obj_type.clone();
                        return self.compile_indirect_call(*target, args, local_type, &span);
                    }

                    // Truly undefined name — produce a clean error
                    // and emit a placeholder so downstream type
                    // checking has something to work with.
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!("undefined function `{name}`"),
                    ));
                    // Compile args anyway so any errors inside them
                    // surface to the user.
                    for arg in args {
                        self.compile_expr(arg);
                    }
                    for _ in 0..arity {
                        self.emit(Instruction::Pop, &span);
                    }
                    self.emit(Instruction::PushInt(0), &span);
                    return ResolvedType::Unknown;
                }

                // Target is a non-identifier expression — compile it
                // and dispatch through CallValue based on its type.
                let target_inner = *target;
                let target_type = ResolvedType::Unknown; // placeholder; real type comes from compiling the target
                self.compile_indirect_call(target_inner, args, target_type, &span)
            }

            // -- Blocks --
            Expression::Nil => {
                self.emit(Instruction::PushNil, &span);

                ResolvedType::Nil
            }

            Expression::Try(inner_expr) => {
                let inner_type = self.compile_expr(*inner_expr);

                let success_type = match &inner_type {
                    ResolvedType::ErrorUnion {
                        error_enum: inner_enum,
                        inner: t,
                    } => {
                        // Verify the enclosing function returns an
                        // error union and, when both sides are
                        // precise, that their error enums agree.
                        // A loose caller can absorb any inner; a
                        // precise caller only accepts matching
                        // precise inners (or a loose inner falls
                        // through to runtime dispatch).
                        if let Some(ref return_type) = self.locals.return_type {
                            match return_type {
                                ResolvedType::ErrorUnion {
                                    error_enum: caller_enum,
                                    ..
                                } => {
                                    if let (Some(caller), Some(inner)) = (caller_enum, inner_enum)
                                        && caller != inner
                                    {
                                        let fmt_enum =
                                            |(name, module): &(String, Vec<String>)| -> String {
                                                if module.is_empty() {
                                                    name.clone()
                                                } else {
                                                    format!("{}.{}", module.join("."), name)
                                                }
                                            };
                                        self.output.errors.push(crate::OrynError::compiler(
                                            span.clone(),
                                            format!(
                                                "`try` cannot propagate `{}`: enclosing function expects error enum `{}`",
                                                fmt_enum(inner),
                                                fmt_enum(caller),
                                            ),
                                        ));
                                    }
                                }
                                ResolvedType::Unknown => {}
                                _ => {
                                    self.output.errors.push(crate::OrynError::compiler(
                                        span.clone(),
                                        format!(
                                            "`try` requires the enclosing function to return an error union type, but it returns `{}`",
                                            return_type.display_name()
                                        ),
                                    ));
                                }
                            }
                        }
                        (**t).clone()
                    }
                    ResolvedType::Unknown => ResolvedType::Unknown,
                    other => {
                        self.output.errors.push(crate::OrynError::compiler(
                            span.clone(),
                            format!(
                                "`try` requires an error union type, got `{}`",
                                other.display_name()
                            ),
                        ));
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

            // -- Maps --
            Expression::MapLiteral(entries) => {
                if entries.is_empty() {
                    // Empty literals carry no key/value types. The
                    // surrounding context reconciles this against any
                    // declared annotation and reports "cannot infer" if
                    // none is present.
                    self.emit(Instruction::MakeMap(0), &span);
                    return ResolvedType::Map(
                        Box::new(ResolvedType::Unknown),
                        Box::new(ResolvedType::Unknown),
                    );
                }

                let count = entries.len();
                let mut entries = entries.into_iter();
                let (first_key, first_value) = entries.next().unwrap();
                let key_ty = self.compile_expr(first_key);
                if !key_ty.is_map_key_type() {
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!(
                            "map key type must be `String`, `int`, or `bool`, got `{}`",
                            key_ty.display_name()
                        ),
                    ));
                }
                let value_ty = self.compile_expr(first_value);

                for (key, value) in entries {
                    let key_span = key.span.clone();
                    let actual_key_ty = self.compile_expr(key);
                    if !actual_key_ty.is_map_key_type() {
                        self.output.errors.push(OrynError::compiler(
                            key_span.clone(),
                            format!(
                                "map key type must be `String`, `int`, or `bool`, got `{}`",
                                actual_key_ty.display_name()
                            ),
                        ));
                    }
                    self.check_types(&key_ty, &actual_key_ty, &key_span, "map key type mismatch");

                    let value_span = value.span.clone();
                    let actual_value_ty = self.compile_expr(value);
                    self.check_types(
                        &value_ty,
                        &actual_value_ty,
                        &value_span,
                        "map value type mismatch",
                    );
                }

                self.emit(Instruction::MakeMap(count as u32), &span);
                ResolvedType::Map(Box::new(key_ty), Box::new(value_ty))
            }

            Expression::Index { object, index } => {
                let object_span = object.span.clone();
                let object_ty = self.compile_expr(*object);

                let (key_ty, result_ty, instruction, key_message) = match &object_ty {
                    ResolvedType::List(inner) => (
                        ResolvedType::Int,
                        (**inner).clone(),
                        Instruction::ListGet,
                        "list index must be `int`",
                    ),
                    ResolvedType::Map(key, value) => (
                        (**key).clone(),
                        ResolvedType::Nillable(Box::new((**value).clone())),
                        Instruction::MapGet,
                        "map key type mismatch",
                    ),
                    ResolvedType::Unknown => (
                        ResolvedType::Unknown,
                        ResolvedType::Unknown,
                        Instruction::ListGet,
                        "index type mismatch",
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
                            Instruction::ListGet,
                            "index type mismatch",
                        )
                    }
                };

                let index_span = index.span.clone();
                let index_ty = self.compile_expr(*index);
                self.check_types(&key_ty, &index_ty, &index_span, key_message);

                self.emit(instruction, &span);
                result_ty
            }

            Expression::Match { scrutinee, arms } => {
                self.compile_match_expression(*scrutinee, arms, &span)
            }

            Expression::If {
                condition,
                body,
                else_body,
            } => self.compile_if_expression(*condition, *body, else_body.map(|b| *b), &span),

            Expression::IfLet {
                name,
                value,
                body,
                else_body,
            } => self.compile_if_let_expression(name, *value, *body, else_body.map(|b| *b), &span),

            Expression::AnonymousFunction {
                params,
                return_type,
                body,
            } => self.compile_anonymous_function(params, return_type, *body, &span),
        }
    }

    /// Compile `if cond { body } else { else_body }` as a value-
    /// producing expression. Both branches must produce the same
    /// type when an else is present; in expression position the
    /// no-else form has type `nil` (the body's value is computed
    /// then discarded, and `nil` is pushed in its place — so a
    /// no-else `if` can only be bound to a `nil`-typed slot, which
    /// in practice means it's only useful in statement position
    /// where the wrapping `Statement::Expression` Pop discards the
    /// result anyway).
    ///
    /// Codegen shape (with else):
    /// ```text
    ///   <compile cond>           ; [bool]
    ///   JumpIfFalse → else       ; pops bool
    ///   <compile body>           ; [body_value]
    ///   Jump → end
    /// else:
    ///   <compile else_body>      ; [else_value]
    /// end:
    /// ```
    ///
    /// No-else form:
    /// ```text
    ///   <compile cond>           ; [bool]
    ///   JumpIfFalse → else       ; pops bool
    ///   <compile body>           ; [body_value]
    ///   Pop                      ; discard body value
    ///   PushNil                  ; standardize result
    ///   Jump → end
    /// else:
    ///   PushNil
    /// end:
    /// ```
    pub(super) fn compile_if_expression(
        &mut self,
        condition: Spanned<Expression>,
        body: Spanned<Expression>,
        else_body: Option<Spanned<Expression>>,
        span: &Span,
    ) -> ResolvedType {
        let cond_span = condition.span.clone();
        let cond_ty = self.compile_expr(condition);
        self.check_types(
            &ResolvedType::Bool,
            &cond_ty,
            &cond_span,
            "if condition type mismatch",
        );

        let branch_jump_idx = self.output.instructions.len();
        self.emit(Instruction::JumpIfFalse(0), span);

        // Compile the body branch in its own scope so any locals
        // declared inside don't leak across the if/else split.
        let body_span = body.span.clone();
        let body_ty = self.with_scope(|this| this.compile_value_body(body));

        if let Some(else_body) = else_body {
            // With-else form: body's value stays on the stack;
            // jump over the else block.
            let jump_to_end_idx = self.output.instructions.len();
            self.emit(Instruction::Jump(0), span);

            let else_start = self.output.instructions.len();
            self.output.instructions[branch_jump_idx] = Instruction::JumpIfFalse(else_start);

            let else_span = else_body.span.clone();
            let else_ty = self.with_scope(|this| this.compile_value_body(else_body));

            let end = self.output.instructions.len();
            self.output.instructions[jump_to_end_idx] = Instruction::Jump(end);

            // Reconcile branch types. The merged result is the
            // body's type when both agree (or when the else type
            // is Unknown — common for branches that hit upstream
            // errors). Mismatched non-Unknown types are a hard
            // error.
            if body_ty != ResolvedType::Unknown
                && else_ty != ResolvedType::Unknown
                && body_ty != else_ty
            {
                self.output.errors.push(crate::OrynError::compiler(
                    else_span,
                    format!(
                        "if branches must produce the same type: body has `{}`, else has `{}`",
                        body_ty.display_name(),
                        else_ty.display_name()
                    ),
                ));
            }

            if body_ty != ResolvedType::Unknown {
                body_ty
            } else {
                else_ty
            }
        } else {
            // No-else form: discard the body's value and push nil
            // so the if-expression has uniform "one value on TOS"
            // shape regardless of which branch ran.
            self.emit(Instruction::Pop, &body_span);
            self.emit(Instruction::PushNil, span);

            let jump_to_end_idx = self.output.instructions.len();
            self.emit(Instruction::Jump(0), span);

            let else_start = self.output.instructions.len();
            self.output.instructions[branch_jump_idx] = Instruction::JumpIfFalse(else_start);
            self.emit(Instruction::PushNil, span);

            let end = self.output.instructions.len();
            self.output.instructions[jump_to_end_idx] = Instruction::Jump(end);

            ResolvedType::Nil
        }
    }

    /// Compile `if let x = value { body } else { else_body }` as a
    /// value-producing expression. The body branch fires when
    /// `value` is non-nil (for a nillable scrutinee) or non-error
    /// (for an error-union scrutinee), binding the unwrapped value
    /// to `x` as an immutable local for the body's scope.
    ///
    /// The else branch does NOT bind the error/nil value — it runs
    /// as a plain fallback. Use `match` if you need to destructure
    /// the error side.
    pub(super) fn compile_if_let_expression(
        &mut self,
        name: String,
        value: Spanned<Expression>,
        body: Spanned<Expression>,
        else_body: Option<Spanned<Expression>>,
        span: &Span,
    ) -> ResolvedType {
        let scrutinee_type = self.compile_expr(value);

        // Three accepted shapes:
        //   * Nillable(T)       — JumpIfNil branching, binds T
        //   * ErrorUnion { T }  — JumpIfError branching, binds T
        //   * Unknown           — upstream error; proceed silently
        enum IfLetMode {
            Nillable,
            ErrorUnion,
            Unknown,
        }
        let (mode, inner_type) = if let Some(inner) = scrutinee_type.unwrap_nillable() {
            (IfLetMode::Nillable, inner.clone())
        } else if let ResolvedType::ErrorUnion { inner, .. } = &scrutinee_type {
            (IfLetMode::ErrorUnion, (**inner).clone())
        } else if matches!(scrutinee_type, ResolvedType::Unknown) {
            (IfLetMode::Unknown, ResolvedType::Unknown)
        } else {
            self.output.errors.push(crate::OrynError::compiler(
                span.clone(),
                format!(
                    "`if let` requires a nillable or error union type, got `{}`",
                    scrutinee_type.display_name()
                ),
            ));
            (IfLetMode::Unknown, ResolvedType::Unknown)
        };

        // Emit the branch test. Both JumpIfNil and JumpIfError are
        // peek-then-maybe-jump, but they leave different stack
        // state on the jump path that the else branch must clean up:
        //   * JumpIfNil pops the nil before jumping — else-start is clean.
        //   * JumpIfError leaves the error on TOS — else-start must Pop.
        let branch_jump_idx = self.output.instructions.len();
        match mode {
            IfLetMode::Nillable | IfLetMode::Unknown => {
                self.emit(Instruction::JumpIfNil(0), span);
            }
            IfLetMode::ErrorUnion => {
                self.emit(Instruction::JumpIfError(0), span);
            }
        }

        // Then-branch: introduce `name: T` in a new scope and bind
        // the unwrapped value to it. The compiled body expression
        // leaves its value on the stack.
        let body_span = body.span.clone();
        let body_ty = self.with_scope(|this| {
            let slot = this
                .locals
                .define(name, super::tables::BindingKind::Val, inner_type);
            this.emit(Instruction::SetLocal(slot), span);
            this.compile_value_body(body)
        });

        if let Some(else_body) = else_body {
            let jump_to_end_idx = self.output.instructions.len();
            self.emit(Instruction::Jump(0), span);

            let else_start = self.output.instructions.len();
            // Patch the branch test's target to the else start.
            match mode {
                IfLetMode::Nillable | IfLetMode::Unknown => {
                    self.output.instructions[branch_jump_idx] = Instruction::JumpIfNil(else_start);
                }
                IfLetMode::ErrorUnion => {
                    self.output.instructions[branch_jump_idx] =
                        Instruction::JumpIfError(else_start);
                    // JumpIfError leaves the error on TOS; pop it
                    // so the else body starts with a clean stack.
                    self.emit(Instruction::Pop, span);
                }
            }

            let else_span = else_body.span.clone();
            let else_ty = self.with_scope(|this| this.compile_value_body(else_body));

            let end = self.output.instructions.len();
            self.output.instructions[jump_to_end_idx] = Instruction::Jump(end);

            if body_ty != ResolvedType::Unknown
                && else_ty != ResolvedType::Unknown
                && body_ty != else_ty
            {
                self.output.errors.push(crate::OrynError::compiler(
                    else_span,
                    format!(
                        "if let branches must produce the same type: body has `{}`, else has `{}`",
                        body_ty.display_name(),
                        else_ty.display_name()
                    ),
                ));
            }

            if body_ty != ResolvedType::Unknown {
                body_ty
            } else {
                else_ty
            }
        } else {
            // No-else form: discard the body's value and push nil
            // (parallel to the plain `if` no-else case).
            self.emit(Instruction::Pop, &body_span);
            self.emit(Instruction::PushNil, span);

            let jump_to_end_idx = self.output.instructions.len();
            self.emit(Instruction::Jump(0), span);

            let else_start = self.output.instructions.len();
            match mode {
                IfLetMode::Nillable | IfLetMode::Unknown => {
                    self.output.instructions[branch_jump_idx] = Instruction::JumpIfNil(else_start);
                }
                IfLetMode::ErrorUnion => {
                    self.output.instructions[branch_jump_idx] =
                        Instruction::JumpIfError(else_start);
                    // Pop the leftover error value before the
                    // implicit nil result is pushed.
                    self.emit(Instruction::Pop, span);
                }
            }
            self.emit(Instruction::PushNil, span);

            let end = self.output.instructions.len();
            self.output.instructions[jump_to_end_idx] = Instruction::Jump(end);

            ResolvedType::Nil
        }
    }

    /// Compile a direct named call to a top-level function or
    /// builtin. This is the fast path for `foo(args)` where `foo`
    /// resolves to a registered function name.
    ///
    /// Type-checks the arguments against the function signature
    /// (including the val-into-mut-param check), then emits the
    /// direct dispatch instruction (`Call(idx, arity)` or
    /// `CallBuiltin(b, arity)`).
    fn compile_direct_named_call(
        &mut self,
        name: String,
        args: Vec<Spanned<Expression>>,
        span: &Span,
    ) -> ResolvedType {
        let arity = args.len();

        // Capture each argument's root name BEFORE compiling, so the
        // val-into-mut-param check can consult the locals table after
        // type checking.
        let arg_root_names: Vec<Option<String>> = args
            .iter()
            .map(|a| expression_root_name(&a.node).map(|s| s.to_string()))
            .collect();

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
            let sig_param_is_mut = sig.param_is_mut.clone();
            for (i, (arg_type, param_type)) in arg_types.iter().zip(&sig_params).enumerate() {
                self.check_types(
                    param_type,
                    arg_type,
                    span,
                    &format!("argument {} type mismatch", i + 1),
                );
            }
            // Val-into-mut-param check.
            for (i, root) in arg_root_names.iter().enumerate() {
                let param_wants_mut = sig_param_is_mut.get(i).copied().unwrap_or(false);
                if param_wants_mut
                    && let Some(arg_name) = root
                    && let Some(entry) = self.locals.resolve(arg_name)
                    && !entry.is_mutable()
                {
                    let op = format!("pass to mut parameter {} of `{name}`", i + 1);
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        super::stmt::immutability_error(arg_name, &entry.kind, &op),
                    ));
                }
            }
        }

        if let Some(idx) = self.fn_table.resolve(&name) {
            self.emit(Instruction::Call(idx, arity), span);
        } else if let Some(builtin) = Self::builtin_from_name(&name) {
            self.emit(Instruction::CallBuiltin(builtin, arity), span);
        } else {
            // Caller verified at least one of these resolves before
            // calling this helper, so this branch is dead in practice.
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                format!("undefined function `{name}`"),
            ));
            self.emit(Instruction::PushInt(0), span);
        }

        self.fn_table
            .signatures
            .get(&name)
            .map(|sig| sig.return_type.clone())
            .unwrap_or(ResolvedType::Unknown)
    }

    /// Compile an indirect call through a function value. The
    /// `target` is any expression that evaluates to a `Function` or
    /// `Closure` value at runtime; the compiler verifies its type is
    /// `ResolvedType::Function { ... }` before emitting `CallValue`.
    ///
    /// `_target_type_hint` is the locally-resolved type for the
    /// target if known via shortcut (e.g., looked up directly from
    /// the locals table for a bare ident); pass `Unknown` to defer
    /// to whatever the target's compiled-expression type yields.
    fn compile_indirect_call(
        &mut self,
        target: Spanned<Expression>,
        args: Vec<Spanned<Expression>>,
        _target_type_hint: ResolvedType,
        span: &Span,
    ) -> ResolvedType {
        let arity = args.len();
        if arity > u8::MAX as usize {
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                format!("call has too many arguments ({arity}); maximum is 255"),
            ));
            self.emit(Instruction::PushInt(0), span);
            return ResolvedType::Unknown;
        }

        // Compile the target first; the result is on TOS.
        let target_type = self.compile_expr(target);

        // Extract the function signature from the target's type.
        let (param_types, return_type) = match &target_type {
            ResolvedType::Function {
                params,
                return_type,
            } => (params.clone(), (**return_type).clone()),
            ResolvedType::Unknown => {
                // Upstream error already reported; type-check args
                // against Unknown to absorb any further mismatches
                // and emit the call so the stack stays balanced.
                for arg in args {
                    self.compile_expr(arg);
                }
                self.emit(Instruction::CallValue(arity as u8), span);
                return ResolvedType::Unknown;
            }
            other => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!(
                        "cannot call value of type `{}` — only function values are callable",
                        other.display_name()
                    ),
                ));
                // Drop the non-callable target and absorb the args.
                self.emit(Instruction::Pop, span);
                for arg in args {
                    self.compile_expr(arg);
                }
                for _ in 0..arity {
                    self.emit(Instruction::Pop, span);
                }
                self.emit(Instruction::PushInt(0), span);
                return ResolvedType::Unknown;
            }
        };

        // Arity check.
        if arity != param_types.len() {
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                format!(
                    "arity mismatch: expected {} arguments, got {arity}",
                    param_types.len()
                ),
            ));
        }

        // Type-check args against the function-type signature.
        for (i, arg) in args.into_iter().enumerate() {
            let arg_type = self.compile_expr(arg);
            if let Some(expected) = param_types.get(i) {
                self.check_types(
                    expected,
                    &arg_type,
                    span,
                    &format!("argument {} type mismatch", i + 1),
                );
            }
        }

        self.emit(Instruction::CallValue(arity as u8), span);
        return_type
    }

    /// Compile an anonymous function expression `fn(params) -> T { body }`.
    ///
    /// Steps:
    /// 1. Resolve the param types and the return type.
    /// 2. Walk the body to enumerate captures (free variables that
    ///    resolve to outer locals). Reject mutations of captured names
    ///    inline as part of the same walk.
    /// 3. Compile the body as a synthetic top-level function whose
    ///    parameter list is `[user_params..., captured_locals...]`.
    /// 4. At the construction site, push each captured value onto the
    ///    stack via `GetLocal(outer_slot)`, then emit `MakeClosure` (or
    ///    `MakeFunction` if there are no captures).
    /// 5. Return type: `Function { user_param_types, return_type }`.
    ///    The captures are invisible at the type level — they live on
    ///    the runtime closure value, not in the function's signature.
    fn compile_anonymous_function(
        &mut self,
        params: Vec<Param>,
        return_type: Option<TypeAnnotation>,
        body: Spanned<Expression>,
        span: &Span,
    ) -> ResolvedType {
        // 1. Resolve param types.
        let mut param_types: Vec<ResolvedType> = Vec::with_capacity(params.len());
        for p in &params {
            let t = match &p.type_ann {
                Some(ann) => match self.resolve_type_annotation(ann) {
                    Ok(t) => t,
                    Err(msg) => {
                        self.output
                            .errors
                            .push(OrynError::compiler(span.clone(), msg));
                        ResolvedType::Unknown
                    }
                },
                None => {
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!(
                            "anonymous function parameter `{}` requires a type annotation",
                            p.name
                        ),
                    ));
                    ResolvedType::Unknown
                }
            };
            param_types.push(t);
        }

        // Resolve return type. Default to Nil for void functions.
        let return_resolved = match &return_type {
            Some(rt) => match self.resolve_type_annotation(rt) {
                Ok(t) => t,
                Err(msg) => {
                    self.output
                        .errors
                        .push(OrynError::compiler(span.clone(), msg));
                    ResolvedType::Unknown
                }
            },
            None => ResolvedType::Nil,
        };

        // 2. Capture analysis: walk the body once and enumerate free
        //    variables that resolve to outer locals.
        let user_param_names: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
        let mut captures: Vec<(String, ResolvedType, super::tables::BindingKind)> = Vec::new();
        let mut seen_captures: HashSet<String> = HashSet::new();
        self.collect_captures(
            &body.node,
            &user_param_names,
            &mut captures,
            &mut seen_captures,
            span,
        );

        // 3. Build the synthetic param list — user params + captures.
        //    Each capture appears as an extra param at the end of the
        //    function's parameter slot layout, so the body can address
        //    it via plain `GetLocal` like any other local.
        let mut synthetic_params: Vec<Param> = params.clone();
        for (name, _ty, _kind) in &captures {
            synthetic_params.push(Param {
                name: name.clone(),
                type_ann: None, // we'll bypass type_ann by passing types directly via param_local_fn
                is_mut: false,
            });
        }

        // The synthetic param types include user param types followed
        // by capture types (in capture order).
        let mut synthetic_param_types: Vec<ResolvedType> = param_types.clone();
        for (_name, ty, _kind) in &captures {
            synthetic_param_types.push(ty.clone());
        }

        // 4. Compile the body as a synthetic top-level function.
        //    Generate a unique name based on the source span so two
        //    anonymous functions never collide.
        let synthetic_name = format!("@anon@{}", span.start);

        // Param-local callback: maps each synthetic param to its
        // (BindingKind, ResolvedType). User params become Param;
        // captures become Param too (read-only inside the closure
        // body — the capture-mutability check enforced this above).
        let synth_param_types_clone = synthetic_param_types.clone();
        let synth_params_clone = synthetic_params.clone();
        let param_local_fn = move |p: &Param| -> (super::tables::BindingKind, ResolvedType) {
            let idx = synth_params_clone
                .iter()
                .position(|sp| sp.name == p.name)
                .unwrap_or(0);
            let ty = synth_param_types_clone
                .get(idx)
                .cloned()
                .unwrap_or(ResolvedType::Unknown);
            (super::tables::BindingKind::Param, ty)
        };

        let synth_idx = self.compile_function_body(super::func::FunctionBodyConfig {
            name: &synthetic_name,
            params: &synthetic_params,
            param_types: synthetic_param_types,
            param_local_fn: &param_local_fn,
            self_name: None,
            body,
            return_type: Some(return_resolved.clone()),
            span,
            is_pub: false,
            is_mut: false,
            pre_allocated_local_idx: None,
        });

        // 5. At the construction site, push each captured value onto
        //    the stack (in capture order, matching what MakeClosure
        //    expects), then emit MakeFunction or MakeClosure.
        let n_captures = captures.len();
        if n_captures == 0 {
            self.emit(Instruction::MakeFunction(synth_idx), span);
        } else {
            for (cap_name, _ty, _kind) in &captures {
                if let Some(entry) = self.locals.resolve(cap_name) {
                    self.emit(Instruction::GetLocal(entry.slot), span);
                }
            }
            self.emit(Instruction::MakeClosure(synth_idx, n_captures as u8), span);
        }

        ResolvedType::Function {
            params: param_types,
            return_type: Box::new(return_resolved),
        }
    }

    /// Walk an expression looking for free variables that resolve to
    /// outer locals (captures) and for assignments that target a
    /// captured name (which we reject because closures are read-only
    /// on captures).
    ///
    /// `local_set` starts as the closure's own params and grows as
    /// the walker enters nested `let`/`val`/`if let` scopes inside
    /// the body. Anything not in `local_set` that DOES resolve in
    /// `self.locals` (the outer scope at construction time) becomes
    /// a capture.
    ///
    /// Nested anonymous functions are walked recursively but their
    /// own params are added to a fresh local set for that walk —
    /// captures of an inner closure are also captures of the outer.
    fn collect_captures(
        &mut self,
        expr: &Expression,
        local_set: &HashSet<String>,
        captures: &mut Vec<(String, ResolvedType, super::tables::BindingKind)>,
        seen: &mut HashSet<String>,
        span: &Span,
    ) {
        match expr {
            Expression::Ident(name) => {
                if !local_set.contains(name)
                    && !seen.contains(name)
                    && let Some(entry) = self.locals.resolve(name)
                {
                    seen.insert(name.clone());
                    captures.push((name.clone(), entry.obj_type.clone(), entry.kind));
                }
            }
            Expression::BinaryOp { left, right, .. } => {
                self.collect_captures(&left.node, local_set, captures, seen, span);
                self.collect_captures(&right.node, local_set, captures, seen, span);
            }
            Expression::UnaryOp { expr: inner, .. } => {
                self.collect_captures(&inner.node, local_set, captures, seen, span);
            }
            Expression::Call { target, args } => {
                self.collect_captures(&target.node, local_set, captures, seen, span);
                for arg in args {
                    self.collect_captures(&arg.node, local_set, captures, seen, span);
                }
            }
            Expression::MethodCall { object, args, .. } => {
                self.collect_captures(&object.node, local_set, captures, seen, span);
                for arg in args {
                    self.collect_captures(&arg.node, local_set, captures, seen, span);
                }
            }
            Expression::FieldAccess { object, .. } => {
                self.collect_captures(&object.node, local_set, captures, seen, span);
            }
            Expression::Index { object, index } => {
                self.collect_captures(&object.node, local_set, captures, seen, span);
                self.collect_captures(&index.node, local_set, captures, seen, span);
            }
            Expression::Range { start, end, .. } => {
                self.collect_captures(&start.node, local_set, captures, seen, span);
                self.collect_captures(&end.node, local_set, captures, seen, span);
            }
            Expression::Try(inner) | Expression::UnwrapError(inner) => {
                self.collect_captures(&inner.node, local_set, captures, seen, span);
            }
            Expression::Coalesce { left, right } => {
                self.collect_captures(&left.node, local_set, captures, seen, span);
                self.collect_captures(&right.node, local_set, captures, seen, span);
            }
            Expression::ListLiteral(elements) => {
                for el in elements {
                    self.collect_captures(&el.node, local_set, captures, seen, span);
                }
            }
            Expression::MapLiteral(entries) => {
                for (k, v) in entries {
                    self.collect_captures(&k.node, local_set, captures, seen, span);
                    self.collect_captures(&v.node, local_set, captures, seen, span);
                }
            }
            Expression::ObjLiteral { fields, .. } => {
                for (_name, v) in fields {
                    self.collect_captures(&v.node, local_set, captures, seen, span);
                }
            }
            Expression::Block(stmts) => {
                // Track block-local lets so they don't get captured.
                let mut block_locals = local_set.clone();
                for stmt in stmts {
                    self.collect_captures_stmt(&stmt.node, &mut block_locals, captures, seen, span);
                }
            }
            Expression::If {
                condition,
                body,
                else_body,
            } => {
                self.collect_captures(&condition.node, local_set, captures, seen, span);
                self.collect_captures(&body.node, local_set, captures, seen, span);
                if let Some(eb) = else_body {
                    self.collect_captures(&eb.node, local_set, captures, seen, span);
                }
            }
            Expression::IfLet {
                value,
                name,
                body,
                else_body,
            } => {
                self.collect_captures(&value.node, local_set, captures, seen, span);
                let mut body_locals = local_set.clone();
                body_locals.insert(name.clone());
                self.collect_captures(&body.node, &body_locals, captures, seen, span);
                if let Some(eb) = else_body {
                    self.collect_captures(&eb.node, local_set, captures, seen, span);
                }
            }
            Expression::Match { scrutinee, arms } => {
                self.collect_captures(&scrutinee.node, local_set, captures, seen, span);
                for arm in arms {
                    let mut arm_locals = local_set.clone();
                    // Add pattern bindings to the arm-local scope so
                    // they're not captured. Wildcards bind nothing;
                    // ok patterns bind the success name; variant
                    // patterns bind their destructured field names.
                    use crate::parser::Pattern;
                    match &arm.pattern.node {
                        Pattern::Wildcard => {}
                        Pattern::Ok { name } => {
                            arm_locals.insert(name.clone());
                        }
                        Pattern::Variant { bindings, .. } => {
                            if let Some(bs) = bindings {
                                for binding in bs {
                                    arm_locals.insert(binding.name.clone());
                                }
                            }
                        }
                    }
                    self.collect_captures(&arm.body.node, &arm_locals, captures, seen, span);
                }
            }
            Expression::StringInterp(parts) => {
                for part in parts {
                    if let crate::parser::StringPart::Interp(e) = part {
                        self.collect_captures(&e.node, local_set, captures, seen, span);
                    }
                }
            }
            Expression::AnonymousFunction { params, body, .. } => {
                // Walk into the nested closure body, but with a
                // fresh local set seeded only by the inner closure's
                // own params. The outer closure's local set is NOT
                // visible to the inner closure body — only the
                // outer's captures (via self.locals) are. Captures
                // discovered inside the inner body are added to the
                // OUTER closure's capture list too, since the outer
                // body needs to make them available for the inner
                // construction site.
                let mut inner_locals: HashSet<String> = HashSet::new();
                for p in params {
                    inner_locals.insert(p.name.clone());
                }
                self.collect_captures(&body.node, &inner_locals, captures, seen, span);
            }
            // Literals, string-literal-only forms, and bare keywords have
            // no captures.
            Expression::Int(_)
            | Expression::Float(_)
            | Expression::True
            | Expression::False
            | Expression::String(_)
            | Expression::Nil => {}
        }
    }

    /// Statement-level capture walk for blocks. Tracks let/val
    /// declarations as block-local so they don't get captured, and
    /// recursively walks expressions.
    fn collect_captures_stmt(
        &mut self,
        stmt: &Statement,
        block_locals: &mut HashSet<String>,
        captures: &mut Vec<(String, ResolvedType, super::tables::BindingKind)>,
        seen: &mut HashSet<String>,
        span: &Span,
    ) {
        match stmt {
            Statement::Let { name, value, .. } | Statement::Val { name, value, .. } => {
                self.collect_captures(&value.node, block_locals, captures, seen, span);
                block_locals.insert(name.clone());
            }
            Statement::Assignment { name, value } => {
                // Reject assignment to a captured outer name. We
                // detect this by: name is NOT in block_locals AND
                // does resolve to an outer local.
                if !block_locals.contains(name) && self.locals.resolve(name).is_some() {
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!(
                            "cannot mutate captured value `{name}` inside an anonymous function — captures are read-only; put mutable state in a struct field"
                        ),
                    ));
                }
                self.collect_captures(&value.node, block_locals, captures, seen, span);
            }
            Statement::FieldAssignment { object, value, .. }
            | Statement::IndexAssignment { object, value, .. } => {
                // Mutating a field/index of a captured value is
                // allowed (the closure can mutate the contents of a
                // captured struct/list/map even though the binding
                // itself is read-only). Just walk both sides.
                self.collect_captures(&object.node, block_locals, captures, seen, span);
                self.collect_captures(&value.node, block_locals, captures, seen, span);
            }
            Statement::Return(opt) => {
                if let Some(e) = opt {
                    self.collect_captures(&e.node, block_locals, captures, seen, span);
                }
            }
            Statement::While { condition, body } => {
                self.collect_captures(&condition.node, block_locals, captures, seen, span);
                self.collect_captures(&body.node, block_locals, captures, seen, span);
            }
            Statement::For {
                name,
                iterable,
                body,
            } => {
                self.collect_captures(&iterable.node, block_locals, captures, seen, span);
                let mut for_locals = block_locals.clone();
                for_locals.insert(name.clone());
                self.collect_captures(&body.node, &for_locals, captures, seen, span);
            }
            Statement::Expression(e) => {
                self.collect_captures(&e.node, block_locals, captures, seen, span);
            }
            Statement::Assert { condition } => {
                self.collect_captures(&condition.node, block_locals, captures, seen, span);
            }
            // Statements that don't reference outer state.
            Statement::Function { .. }
            | Statement::ObjDef { .. }
            | Statement::EnumDef { .. }
            | Statement::Import { .. }
            | Statement::Test { .. }
            | Statement::Break
            | Statement::Continue => {}
        }
    }
}
