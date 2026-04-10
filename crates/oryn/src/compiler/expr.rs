use std::collections::HashMap;

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{BinOp, Expression, Spanned, StringPart, UnaryOp};

use super::block::BlockMode;
use super::compile::Compiler;
use super::types::Instruction;

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

impl Compiler {
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
}

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
            Expression::StringInterp(parts) => {
                let num_parts = parts.len();
                for part in parts {
                    match part {
                        StringPart::Literal(s) => {
                            self.emit(Instruction::PushString(s), &span);
                        }
                        StringPart::Interp(expr) => {
                            let returned_type = self.compile_expr(expr);

                            // We don't need to convert to string if the result is already a string.
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
                } else if let Some(const_value) = self.output.module_constants.get(&name).cloned() {
                    // In-module reference to a `pub let` / `pub val`
                    // constant declared earlier at top level. Inline the
                    // literal directly so module bodies can use their own
                    // constants by bare name.
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
                // Module constant access: `math.PI` or `std.math.constants.TAU`.
                // Walk the object as a dotted path; if it matches an imported
                // module and `field` is a pub constant, inline the literal.
                if let Some(path) = extract_dotted_path(&object.node) {
                    let root = &path[0];
                    if self.locals.resolve(root).is_none() {
                        let key = path.join(".");
                        if let Some(exports) = self.modules.modules.get(&key) {
                            if let Some(const_value) = exports.constants.get(&field) {
                                let instr = const_value.to_instruction();
                                let result_type = const_value.resolved_type();
                                self.emit(instr, &span);
                                return result_type;
                            } else if !exports.functions.contains_key(&field) {
                                self.output.errors.push(OrynError::compiler(
                                    span.clone(),
                                    format!("undefined constant `{key}.{field}`"),
                                ));
                                self.emit(Instruction::PushInt(0), &span);
                                return ResolvedType::Unknown;
                            }
                        }
                    }
                }

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
                    // Runtime method dispatch. Before compiling, peek at
                    // the receiver's type so we can enforce cross-module
                    // method privacy.
                    let receiver_type = self.infer_object_type(&object.node);

                    if let ResolvedType::Object {
                        name: type_name,
                        module,
                    } = &receiver_type
                        && !module.is_empty()
                        && *module != self.current_module_path
                    {
                        let module_key = module.join(".");
                        if let Some(def) = self
                            .modules
                            .modules
                            .get(&module_key)
                            .and_then(|e| e.obj_defs.get(type_name))
                        {
                            if def.methods.contains_key(&method) {
                                let is_pub =
                                    def.method_is_pub.get(&method).copied().unwrap_or(false);
                                if !is_pub {
                                    self.output.errors.push(OrynError::compiler(
                                        span.clone(),
                                        format!(
                                            "method `{method}` is private to module `{module_key}`"
                                        ),
                                    ));
                                }
                            } else {
                                self.output.errors.push(OrynError::compiler(
                                    span.clone(),
                                    format!("undefined method `{module_key}.{type_name}.{method}`"),
                                ));
                            }
                        }
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
                match (&left.node, &right.node) {
                    (Expression::Int(l), Expression::Int(r)) => {
                        let folded = match &op {
                            BinOp::Add => l.checked_add(*r),
                            BinOp::Sub => l.checked_sub(*r),
                            BinOp::Mul => l.checked_mul(*r),
                            BinOp::Div if *r != 0 => l.checked_div(*r),
                            _ => None,
                        };

                        if let Some(result) = folded {
                            self.emit(Instruction::PushInt(result), &span);
                            return ResolvedType::Int;
                        }
                    }
                    (Expression::Float(l), Expression::Float(r)) => {
                        let folded = match &op {
                            BinOp::Add => Some(*l + *r),
                            BinOp::Sub => Some(*l - *r),
                            BinOp::Mul => Some(*l * *r),
                            BinOp::Div => Some(*l / *r),
                            _ => None,
                        };

                        if let Some(result) = folded.filter(|v| v.is_finite()) {
                            self.emit(Instruction::PushFloat(result), &span);
                            return ResolvedType::Float;
                        }
                    }
                    _ => {}
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
