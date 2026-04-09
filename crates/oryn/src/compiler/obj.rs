use std::collections::HashMap;

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{ObjMethod, Span, TypeAnnotation};

use super::compile::{Compiler, resolve_type};
use super::func::FunctionBodyConfig;
use super::types::{MethodSignature, ObjDefInfo};

// ---------------------------------------------------------------------------
// Object definition compilation
// ---------------------------------------------------------------------------

impl Compiler {
    pub(super) fn compile_obj_def(
        &mut self,
        name: String,
        fields: Vec<(String, TypeAnnotation, Span)>,
        methods: Vec<ObjMethod>,
        uses: Vec<String>,
        stmt_span: &Span,
    ) {
        let mut field_names: Vec<String> = Vec::new();
        let mut field_types: Vec<ResolvedType> = Vec::new();
        let mut method_indices: HashMap<String, usize> = HashMap::new();
        let mut all_required: Vec<MethodSignature> = Vec::new();

        for used_type in &uses {
            if let Some((_, def)) = self.obj_table.resolve(used_type) {
                for req in &def.signatures {
                    if !all_required.iter().any(|r| r.name == req.name) {
                        all_required.push(req.clone());
                    }
                }

                for field in &def.fields {
                    if field_names.contains(field) {
                        self.output.errors.push(OrynError::compiler(
                            stmt_span.clone(),
                            format!("field `{field}` conflicts in `use {used_type}`"),
                        ));
                    } else {
                        field_names.push(field.clone());
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
        for (field_name, type_ann, field_span) in fields {
            field_names.push(field_name.clone());

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
            HashMap::new(),
            Vec::new(),
        );

        // Collect this type's own required methods (bodyless declarations)
        // with their full signatures.
        let own_required: Vec<MethodSignature> = methods
            .iter()
            .filter(|m| m.body.is_none())
            .map(|m| {
                let param_types: Vec<ResolvedType> = m
                    .params
                    .iter()
                    .filter(|(pname, _)| pname != "self")
                    .map(|(_, ann)| {
                        ann.as_ref()
                            .map(|a| {
                                resolve_type(a, &self.obj_table).unwrap_or(ResolvedType::Unknown)
                            })
                            .unwrap_or(ResolvedType::Unknown)
                    })
                    .collect();

                let return_type = match &m.return_type {
                    Some(rt) => resolve_type(rt, &self.obj_table).unwrap_or(ResolvedType::Unknown),
                    None => ResolvedType::Void,
                };

                MethodSignature {
                    name: m.name.clone(),
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

                // Resolve param types once, derive both HashMap and Vec.
                let resolved_params: HashMap<String, ResolvedType> = method
                    .params
                    .iter()
                    .map(|(pname, ann)| {
                        let t = if pname == "self" {
                            ResolvedType::Object(obj_name.clone())
                        } else {
                            ann.as_ref()
                                .map(|a| {
                                    resolve_type(a, &self.obj_table)
                                        .unwrap_or(ResolvedType::Unknown)
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
                    Some(rt) => resolve_type(rt, &self.obj_table).unwrap_or(ResolvedType::Unknown),
                    None => ResolvedType::Void,
                };

                // Store the non-self param types and return type for shape checking.
                let non_self_params: Vec<ResolvedType> = method
                    .params
                    .iter()
                    .filter(|(pname, _)| pname != "self")
                    .map(|(pname, _)| {
                        resolved_params
                            .get(pname)
                            .cloned()
                            .unwrap_or(ResolvedType::Unknown)
                    })
                    .collect();
                compiled_signatures.insert(
                    method.name.clone(),
                    (non_self_params, return_resolved.clone()),
                );

                let obj_name_for_closure = obj_name.clone();
                let param_fn = move |pname: &str, _ann: &Option<TypeAnnotation>| {
                    if pname == "self" {
                        (true, ResolvedType::Object(obj_name_for_closure.clone()))
                    } else {
                        let resolved = resolved_params
                            .get(pname)
                            .cloned()
                            .unwrap_or(ResolvedType::Unknown);
                        (false, resolved)
                    }
                };

                let func_idx = self.compile_function_body(FunctionBodyConfig {
                    name: &method.name,
                    params: &method.params,
                    param_types,
                    param_local_fn: &param_fn,
                    self_name: None,
                    body,
                    span: stmt_span,
                    return_type: Some(return_resolved),
                });

                method_indices.insert(method.name.clone(), func_idx);
            }
        }

        // Restore the obj_table (remove the temporary self-registration).
        self.obj_table = parent_obj_table;

        // Check: every required method from used types must be satisfied
        // with matching shape (param types + return type).
        for req in &all_required {
            if !method_indices.contains_key(&req.name) {
                self.output.errors.push(OrynError::compiler(
                    stmt_span.clone(),
                    format!("object `{name}` is missing required method `{}`", req.name),
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
            if !method_indices.contains_key(&req.name) {
                final_required.push(req);
            }
        }

        self.output.obj_defs.push(ObjDefInfo {
            name,
            fields: field_names,
            field_types,
            methods: method_indices,
            signatures: final_required,
        });
    }
}
