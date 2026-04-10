use std::collections::HashMap;

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{ObjField, ObjMethod, Span, TypeAnnotation};

use super::compile::{Compiler, resolve_type};
use super::func::FunctionBodyConfig;
use super::tables::FunctionSignature;
use super::types::{MethodSignature, ObjDefInfo};

// ---------------------------------------------------------------------------
// Object definition compilation
// ---------------------------------------------------------------------------

impl Compiler {
    pub(super) fn compile_obj_def(
        &mut self,
        name: String,
        fields: Vec<ObjField>,
        methods: Vec<ObjMethod>,
        uses: Vec<String>,
        stmt_span: &Span,
        is_pub: bool,
    ) {
        let mut field_names: Vec<String> = Vec::new();
        let mut field_types: Vec<ResolvedType> = Vec::new();
        let mut field_is_pub: Vec<bool> = Vec::new();
        let mut method_indices: HashMap<String, usize> = HashMap::new();
        let mut static_method_indices: HashMap<String, usize> = HashMap::new();
        let mut method_is_pub: HashMap<String, bool> = HashMap::new();
        let mut static_method_is_pub: HashMap<String, bool> = HashMap::new();
        let mut method_signatures: HashMap<String, FunctionSignature> = HashMap::new();
        let mut static_method_signatures: HashMap<String, FunctionSignature> = HashMap::new();
        let mut all_required: Vec<MethodSignature> = Vec::new();

        for used_type in &uses {
            if let Some((_, def)) = self.obj_table.resolve(used_type) {
                for req in &def.signatures {
                    if !all_required.iter().any(|r| r.name == req.name) {
                        all_required.push(req.clone());
                    }
                }

                for (i, field) in def.fields.iter().enumerate() {
                    if field_names.contains(field) {
                        self.output.errors.push(OrynError::compiler(
                            stmt_span.clone(),
                            format!("field `{field}` conflicts in `use {used_type}`"),
                        ));
                    } else {
                        field_names.push(field.clone());
                        field_is_pub.push(def.field_is_pub.get(i).copied().unwrap_or(false));
                    }
                }

                for (method_name, &func_idx) in &def.methods {
                    if method_indices.contains_key(method_name) {
                        self.output.errors.push(OrynError::compiler(
                            stmt_span.clone(),
                            format!("method `{method_name}` conflicts in `use {used_type}`"),
                        ));
                    } else {
                        method_indices.insert(method_name.clone(), func_idx);
                        method_is_pub.insert(
                            method_name.clone(),
                            def.method_is_pub.get(method_name).copied().unwrap_or(false),
                        );
                        if let Some(sig) = def.method_signatures.get(method_name) {
                            method_signatures.insert(method_name.clone(), sig.clone());
                        }
                    }
                }

                for (method_name, &func_idx) in &def.static_methods {
                    if static_method_indices.contains_key(method_name) {
                        self.output.errors.push(OrynError::compiler(
                            stmt_span.clone(),
                            format!("static method `{method_name}` conflicts in `use {used_type}`"),
                        ));
                    } else {
                        static_method_indices.insert(method_name.clone(), func_idx);
                        static_method_is_pub.insert(
                            method_name.clone(),
                            def.static_method_is_pub
                                .get(method_name)
                                .copied()
                                .unwrap_or(false),
                        );
                        if let Some(sig) = def.static_method_signatures.get(method_name) {
                            static_method_signatures.insert(method_name.clone(), sig.clone());
                        }
                    }
                }
            } else {
                self.output.errors.push(OrynError::compiler(
                    stmt_span.clone(),
                    format!("undefined type `{used_type}` in use declaration"),
                ));
            }
        }

        // Append this obj's own fields.
        for field in fields {
            let ObjField {
                name: field_name,
                type_ann,
                span: field_span,
                is_pub: field_pub,
            } = field;
            field_names.push(field_name.clone());
            field_is_pub.push(field_pub);

            match self.resolve_type_annotation(&type_ann) {
                Ok(t) => field_types.push(t),
                Err(msg) => {
                    self.output.errors.push(OrynError::compiler(
                        field_span,
                        format!("field `{field_name}`: {msg}"),
                    ));
                    field_types.push(ResolvedType::Unknown);
                }
            }
        }

        // Build a temporary ObjTable that includes the current type
        // so method bodies can resolve self.field accesses.
        let parent_obj_table = self.obj_table.clone();
        self.obj_table.register(
            name.clone(),
            field_names.clone(),
            field_types.clone(),
            field_is_pub.clone(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            Vec::new(),
            is_pub,
        );

        // Collect this type's own required methods (bodyless declarations)
        // with their full signatures.
        let own_required: Vec<MethodSignature> = methods
            .iter()
            .filter(|m| m.body.is_none())
            .map(|m| {
                let is_static = !m.params.iter().any(|(pname, _)| pname == "self");
                let param_types: Vec<ResolvedType> = m
                    .params
                    .iter()
                    .filter(|(pname, _)| is_static || pname != "self")
                    .map(|(_, ann)| {
                        ann.as_ref()
                            .map(|a| {
                                self.attach_current_module(
                                    resolve_type(a, &self.obj_table, &self.modules)
                                        .unwrap_or(ResolvedType::Unknown),
                                )
                            })
                            .unwrap_or(ResolvedType::Unknown)
                    })
                    .collect();

                let return_type = match &m.return_type {
                    Some(rt) => self.attach_current_module(
                        resolve_type(rt, &self.obj_table, &self.modules)
                            .unwrap_or(ResolvedType::Unknown),
                    ),
                    None => ResolvedType::Void,
                };

                MethodSignature {
                    name: m.name.clone(),
                    is_static,
                    param_types,
                    return_type,
                }
            })
            .collect();

        // Track the compiled method signatures for shape checking.
        let mut compiled_signatures: HashMap<String, (Vec<ResolvedType>, ResolvedType)> =
            HashMap::new();

        for method in methods {
            if let Some(body) = method.body {
                let obj_name = name.clone();
                let is_static = !method.params.iter().any(|(pname, _)| pname == "self");

                // Resolve param types once, derive both HashMap and Vec.
                let resolved_params: HashMap<String, ResolvedType> = method
                    .params
                    .iter()
                    .map(|(pname, ann)| {
                        let t = if pname == "self" {
                            ResolvedType::Object {
                                name: obj_name.clone(),
                                module: self.current_module_path.clone(),
                            }
                        } else {
                            ann.as_ref()
                                .map(|a| {
                                    self.attach_current_module(
                                        resolve_type(a, &self.obj_table, &self.modules)
                                            .unwrap_or(ResolvedType::Unknown),
                                    )
                                })
                                .unwrap_or(ResolvedType::Unknown)
                        };
                        (pname.clone(), t)
                    })
                    .collect();

                let param_types: Vec<ResolvedType> = method
                    .params
                    .iter()
                    .map(|(pname, _)| {
                        resolved_params
                            .get(pname)
                            .cloned()
                            .unwrap_or(ResolvedType::Unknown)
                    })
                    .collect();

                let return_resolved = match &method.return_type {
                    Some(rt) => self.attach_current_module(
                        resolve_type(rt, &self.obj_table, &self.modules)
                            .unwrap_or(ResolvedType::Unknown),
                    ),
                    None => ResolvedType::Void,
                };

                let sig_params: Vec<ResolvedType> = method
                    .params
                    .iter()
                    .filter(|(pname, _)| is_static || pname != "self")
                    .map(|(pname, _)| {
                        resolved_params
                            .get(pname)
                            .cloned()
                            .unwrap_or(ResolvedType::Unknown)
                    })
                    .collect();
                compiled_signatures.insert(
                    method.name.clone(),
                    (sig_params.clone(), return_resolved.clone()),
                );

                let obj_name_for_closure = obj_name.clone();
                let module_for_closure = self.current_module_path.clone();
                let param_fn = move |pname: &str, _ann: &Option<TypeAnnotation>| {
                    if pname == "self" {
                        (
                            true,
                            ResolvedType::Object {
                                name: obj_name_for_closure.clone(),
                                module: module_for_closure.clone(),
                            },
                        )
                    } else {
                        let resolved = resolved_params
                            .get(pname)
                            .cloned()
                            .unwrap_or(ResolvedType::Unknown);
                        (false, resolved)
                    }
                };

                // Per-method visibility — defaults to private. The
                // containing object can be `pub` while individual methods
                // remain hidden, mirroring Rust's pub-on-fields rule.
                let method_pub = method.is_pub;

                let func_idx = self.compile_function_body(FunctionBodyConfig {
                    name: &method.name,
                    params: &method.params,
                    param_types,
                    param_local_fn: &param_fn,
                    self_name: None,
                    body,
                    span: stmt_span,
                    is_pub: method_pub,
                    return_type: Some(return_resolved.clone()),
                });

                let sig = FunctionSignature {
                    param_types: sig_params.clone(),
                    return_type: return_resolved,
                };

                if is_static {
                    static_method_indices.insert(method.name.clone(), func_idx);
                    static_method_is_pub.insert(method.name.clone(), method_pub);
                    static_method_signatures.insert(method.name.clone(), sig);
                    if let Some(def) = self.obj_table.defs.last_mut() {
                        def.static_methods.insert(method.name.clone(), func_idx);
                    }
                } else {
                    method_indices.insert(method.name.clone(), func_idx);
                    method_is_pub.insert(method.name.clone(), method_pub);
                    method_signatures.insert(method.name.clone(), sig);
                }
            }
        }

        // Restore the obj_table (remove the temporary self-registration).
        self.obj_table = parent_obj_table;

        // Check: every required method from used types must be satisfied
        // with matching shape (param types + return type).
        for req in &all_required {
            let has_impl = if req.is_static {
                static_method_indices.contains_key(&req.name)
            } else {
                method_indices.contains_key(&req.name)
            };

            if !has_impl {
                self.output.errors.push(OrynError::compiler(
                    stmt_span.clone(),
                    format!(
                        "object `{name}` is missing required {} `{}`",
                        if req.is_static {
                            "static method"
                        } else {
                            "method"
                        },
                        req.name
                    ),
                ));
            } else if let Some((impl_params, impl_return)) = compiled_signatures.get(&req.name) {
                // Check parameter count.
                if impl_params.len() != req.param_types.len() {
                    self.output.errors.push(OrynError::compiler(
                        stmt_span.clone(),
                        format!(
                            "method `{}` has {} parameter(s) but signature requires {}",
                            req.name,
                            impl_params.len(),
                            req.param_types.len()
                        ),
                    ));
                } else {
                    // Check each parameter type.
                    for (i, (impl_t, req_t)) in impl_params.iter().zip(&req.param_types).enumerate()
                    {
                        if *req_t != ResolvedType::Unknown
                            && *impl_t != ResolvedType::Unknown
                            && impl_t != req_t
                        {
                            self.output.errors.push(OrynError::compiler(
                                stmt_span.clone(),
                                format!(
                                    "method `{}` parameter {} type mismatch: expected `{}`, got `{}`",
                                    req.name,
                                    i + 1,
                                    req_t.display_name(),
                                    impl_t.display_name()
                                ),
                            ));
                        }
                    }
                }

                // Check return type.
                if req.return_type != ResolvedType::Unknown
                    && *impl_return != ResolvedType::Unknown
                    && *impl_return != req.return_type
                {
                    self.output.errors.push(OrynError::compiler(
                        stmt_span.clone(),
                        format!(
                            "method `{}` return type mismatch: expected `{}`, got `{}`",
                            req.name,
                            req.return_type.display_name(),
                            impl_return.display_name()
                        ),
                    ));
                }
            }
            // If the method was inherited (not in compiled_signatures),
            // it already matched when the parent type was compiled, so skip.
        }

        let mut final_required: Vec<MethodSignature> = Vec::new();
        for req in own_required {
            let has_impl = if req.is_static {
                static_method_indices.contains_key(&req.name)
            } else {
                method_indices.contains_key(&req.name)
            };

            if !has_impl {
                final_required.push(req);
            }
        }

        self.output.obj_defs.push(ObjDefInfo {
            name,
            fields: field_names,
            field_types,
            field_is_pub,
            methods: method_indices,
            static_methods: static_method_indices,
            method_is_pub,
            static_method_is_pub,
            method_signatures,
            static_method_signatures,
            signatures: final_required,
            is_pub,
        });
    }
}
