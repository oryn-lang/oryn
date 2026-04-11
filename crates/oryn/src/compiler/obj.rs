use std::collections::{HashMap, HashSet};

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{ObjField, ObjMethod, Span};

use super::compile::{Compiler, resolve_type};
use super::func::FunctionBodyConfig;
use super::tables::{BindingKind, FunctionSignature};
use super::types::{CompiledFunction, MethodSignature, ObjDefInfo};

/// Reason an inherited method signature is allowed to be replaced by an
/// own declaration. Recorded in the pre-pass and used to phrase override
/// errors precisely.
enum OverrideKind {
    /// Single inherited candidate. The own signature must match the
    /// inherited signature, with a Self-return covariance carve-out
    /// (returning the using type instead of the defining type).
    SingleParent { source_type: String },
    /// Multiple inherited candidates from different `use` clauses. The
    /// own declaration is an explicit conflict resolution; signatures are
    /// not checked because the user is intentionally picking neither
    /// inherited version.
    MultiUseResolution,
}

// ---------------------------------------------------------------------------
// Object definition compilation
// ---------------------------------------------------------------------------

impl Compiler {
    pub(super) fn compile_obj_def(
        &mut self,
        name: String,
        fields: Vec<ObjField>,
        methods: Vec<ObjMethod>,
        uses: Vec<Vec<String>>,
        stmt_span: &Span,
        is_pub: bool,
    ) {
        // Pre-scan own declarations so the use loop knows which inherited
        // collisions are explicitly resolved by the using type. Methods and
        // statics can be resolved (replaced) this way; fields cannot,
        // because field layout has fixed offsets.
        let own_method_names: HashSet<String> = methods
            .iter()
            .filter(|m| m.params.iter().any(|p| p.name == "self"))
            .map(|m| m.name.clone())
            .collect();
        let own_static_names: HashSet<String> = methods
            .iter()
            .filter(|m| !m.params.iter().any(|p| p.name == "self"))
            .map(|m| m.name.clone())
            .collect();

        let mut field_names: Vec<String> = Vec::new();
        let mut field_types: Vec<ResolvedType> = Vec::new();
        let mut field_is_pub: Vec<bool> = Vec::new();
        // Records which `use` clause an inherited field came from, for
        // precise own-vs-inherited conflict messages.
        let mut inherited_field_source: HashMap<String, String> = HashMap::new();
        let mut method_indices: HashMap<String, usize> = HashMap::new();
        let mut static_method_indices: HashMap<String, usize> = HashMap::new();
        let mut method_is_pub: HashMap<String, bool> = HashMap::new();
        let mut static_method_is_pub: HashMap<String, bool> = HashMap::new();
        let mut method_signatures: HashMap<String, FunctionSignature> = HashMap::new();
        let mut static_method_signatures: HashMap<String, FunctionSignature> = HashMap::new();
        // Records which type provided each inherited method/static, used
        // by the override sig check to recognise Self-return covariance.
        let mut inherited_method_source: HashMap<String, String> = HashMap::new();
        let mut inherited_static_source: HashMap<String, String> = HashMap::new();
        // Names where the using type's own declaration is acting as an
        // explicit resolution of a multi-use conflict. The override sig
        // check is suppressed for these because the user is intentionally
        // picking neither inherited version.
        let mut multi_use_resolved_methods: HashSet<String> = HashSet::new();
        let mut multi_use_resolved_statics: HashSet<String> = HashSet::new();
        let mut all_required: Vec<MethodSignature> = Vec::new();

        for used_path in &uses {
            let display_name = used_path.join(".");
            let source_type_name = used_path.last().cloned().unwrap_or_default();

            // Resolve either locally (single segment) or via imported
            // module exports (multi-segment). In either case we get a
            // cloned ObjDefInfo that the inheritance logic can consume
            // uniformly — imported defs carry absolute function indices
            // already, so no remapping is needed.
            let def: Option<ObjDefInfo> = if used_path.len() == 1 {
                self.obj_table
                    .resolve(&used_path[0])
                    .map(|(_, d)| d.clone())
            } else {
                let (type_name, module_path) = used_path.split_last().unwrap();
                let module_key = module_path.join(".");
                self.modules
                    .modules
                    .get(&module_key)
                    .and_then(|exports| exports.obj_defs.get(type_name))
                    .cloned()
            };

            if let Some(def) = def {
                for req in &def.signatures {
                    if !all_required.iter().any(|r| r.name == req.name) {
                        all_required.push(req.clone());
                    }
                }

                for (i, field) in def.fields.iter().enumerate() {
                    if field_names.contains(field) {
                        // Field collisions across `use` clauses are
                        // always errors; field layout cannot accommodate
                        // two slots with the same name. Note: own-vs-
                        // inherited field collisions are caught later in
                        // the own-fields loop.
                        self.output.errors.push(OrynError::compiler(
                            stmt_span.clone(),
                            format!("field `{field}` conflicts in `use {display_name}`"),
                        ));
                    } else {
                        field_names.push(field.clone());
                        field_types.push(
                            def.field_types
                                .get(i)
                                .cloned()
                                .unwrap_or(ResolvedType::Unknown),
                        );
                        field_is_pub.push(def.field_is_pub.get(i).copied().unwrap_or(false));
                        inherited_field_source.insert(field.clone(), display_name.clone());
                    }
                }

                for (method_name, &func_idx) in &def.methods {
                    if method_indices.contains_key(method_name) {
                        if own_method_names.contains(method_name) {
                            // The using type explicitly resolves this
                            // multi-use conflict by declaring its own
                            // version. Skip the cross-use error; the
                            // override pre-pass will replace whatever's
                            // currently in the table without sig check.
                            multi_use_resolved_methods.insert(method_name.clone());
                            continue;
                        }
                        self.output.errors.push(OrynError::compiler(
                            stmt_span.clone(),
                            format!("method `{method_name}` conflicts in `use {display_name}`"),
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
                        inherited_method_source
                            .insert(method_name.clone(), source_type_name.clone());
                    }
                }

                for (method_name, &func_idx) in &def.static_methods {
                    if static_method_indices.contains_key(method_name) {
                        if own_static_names.contains(method_name) {
                            multi_use_resolved_statics.insert(method_name.clone());
                            continue;
                        }
                        self.output.errors.push(OrynError::compiler(
                            stmt_span.clone(),
                            format!(
                                "static method `{method_name}` conflicts in `use {display_name}`"
                            ),
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
                        inherited_static_source
                            .insert(method_name.clone(), source_type_name.clone());
                    }
                }
            } else {
                self.output.errors.push(OrynError::compiler(
                    stmt_span.clone(),
                    format!("undefined type `{display_name}` in use declaration"),
                ));
            }
        }

        // Append this obj's own fields. Field layout has fixed offsets,
        // so a name that collides with an inherited field is an error —
        // there's no override semantics for fields.
        for field in fields {
            let ObjField {
                name: field_name,
                type_ann,
                span: field_span,
                is_pub: field_pub,
            } = field;

            if let Some(source) = inherited_field_source.get(&field_name) {
                self.output.errors.push(OrynError::compiler(
                    field_span.clone(),
                    format!(
                        "field `{field_name}` conflicts with field inherited from `use {source}`; fields cannot be overridden"
                    ),
                ));
                // Skip the push so we don't end up with a duplicate slot.
                continue;
            }
            if field_names.contains(&field_name) {
                // Two own fields with the same name (no inherited involved).
                self.output.errors.push(OrynError::compiler(
                    field_span.clone(),
                    format!("duplicate field `{field_name}`"),
                ));
                continue;
            }

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
        // so method bodies can resolve self.field accesses. At this
        // point only inherited methods are populated; the pre-pass below
        // will mutate this entry to add own-method signatures and
        // indices before any bodies are compiled, so methods can call
        // each other regardless of declaration order.
        let parent_obj_table = self.obj_table.clone();
        self.obj_table.register(
            name.clone(),
            field_names.clone(),
            field_types.clone(),
            field_is_pub.clone(),
            method_indices.clone(),
            static_method_indices.clone(),
            method_is_pub.clone(),
            static_method_is_pub.clone(),
            method_signatures.clone(),
            static_method_signatures.clone(),
            all_required.clone(),
            is_pub,
        );

        // Collect this type's own required methods (bodyless declarations)
        // with their full signatures.
        let own_required: Vec<MethodSignature> = methods
            .iter()
            .filter(|m| m.body.is_none())
            .map(|m| {
                let is_static = !m.params.iter().any(|p| p.name == "self");
                let param_types: Vec<ResolvedType> = m
                    .params
                    .iter()
                    .filter(|p| is_static || p.name != "self")
                    .map(|p| {
                        p.type_ann
                            .as_ref()
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
                    None => ResolvedType::Unknown,
                };

                MethodSignature {
                    name: m.name.clone(),
                    is_static,
                    param_types,
                    return_type,
                    is_mut: m.is_mut(),
                }
            })
            .collect();

        // -----------------------------------------------------------------
        // Pre-pass: resolve method signatures and reserve function slots.
        //
        // Before any body is compiled we walk every method body once to
        // (a) compute its signature, (b) push a placeholder
        // CompiledFunction so the absolute index is fixed, and (c) apply
        // override semantics against any inherited entry. The full
        // method tables are then written back into the obj_table
        // snapshot so pass 2 can resolve intra-obj `self.method()` calls
        // regardless of declaration order.
        // -----------------------------------------------------------------

        // Resolved per-method metadata produced by the pre-pass and
        // consumed by the body compile pass. The local index points at
        // the placeholder slot pushed during the pre-pass.
        struct PreparedMethod {
            local_idx: usize,
            param_types: Vec<ResolvedType>,
            return_type: ResolvedType,
            resolved_params: HashMap<String, ResolvedType>,
        }

        let mut prepared: HashMap<String, PreparedMethod> = HashMap::new();
        // Track signatures per body method for the required-method shape
        // check after pass 2.
        let mut compiled_signatures: HashMap<String, (Vec<ResolvedType>, ResolvedType)> =
            HashMap::new();

        for method in &methods {
            if method.body.is_none() {
                continue;
            }
            let is_static = !method.params.iter().any(|p| p.name == "self");

            let resolved_params: HashMap<String, ResolvedType> = method
                .params
                .iter()
                .map(|p| {
                    let t = if p.name == "self" {
                        ResolvedType::Object {
                            name: name.clone(),
                            module: self.current_module_path.clone(),
                        }
                    } else {
                        p.type_ann
                            .as_ref()
                            .map(|a| {
                                self.attach_current_module(
                                    resolve_type(a, &self.obj_table, &self.modules)
                                        .unwrap_or(ResolvedType::Unknown),
                                )
                            })
                            .unwrap_or(ResolvedType::Unknown)
                    };
                    (p.name.clone(), t)
                })
                .collect();

            let param_types: Vec<ResolvedType> = method
                .params
                .iter()
                .map(|p| {
                    resolved_params
                        .get(&p.name)
                        .cloned()
                        .unwrap_or(ResolvedType::Unknown)
                })
                .collect();

            let return_resolved = match &method.return_type {
                Some(rt) => self.attach_current_module(
                    resolve_type(rt, &self.obj_table, &self.modules)
                        .unwrap_or(ResolvedType::Unknown),
                ),
                None => ResolvedType::Unknown,
            };

            self.output
                .type_map
                .insert(method.span.clone(), &return_resolved);

            // Signature stored without `self` for caller-facing checks.
            let sig_params: Vec<ResolvedType> = method
                .params
                .iter()
                .filter(|p| is_static || p.name != "self")
                .map(|p| {
                    resolved_params
                        .get(&p.name)
                        .cloned()
                        .unwrap_or(ResolvedType::Unknown)
                })
                .collect();
            compiled_signatures.insert(
                method.name.clone(),
                (sig_params.clone(), return_resolved.clone()),
            );

            // Per-parameter mut flags. The signature stores them
            // alongside the type so callers can check val-into-mut
            // arguments without re-resolving the AST.
            let sig_param_is_mut: Vec<bool> = method
                .params
                .iter()
                .filter(|p| is_static || p.name != "self")
                .map(|p| p.is_mut)
                .collect();
            let new_sig = FunctionSignature {
                param_types: sig_params.clone(),
                return_type: return_resolved.clone(),
                param_is_mut: sig_param_is_mut,
                is_mut: method.is_mut(),
            };

            // -- Override resolution -------------------------------------
            // If the name already exists in the inherited tables, this
            // is an override. We allow it as long as the signature
            // matches (with Self-return covariance), and we suppress the
            // sig check entirely when the override is acting as an
            // explicit resolution of a multi-use conflict.
            let already_inherited = if is_static {
                static_method_indices.contains_key(&method.name)
            } else {
                method_indices.contains_key(&method.name)
            };
            if already_inherited {
                let multi_resolved = if is_static {
                    multi_use_resolved_statics.contains(&method.name)
                } else {
                    multi_use_resolved_methods.contains(&method.name)
                };
                let override_kind = if multi_resolved {
                    OverrideKind::MultiUseResolution
                } else {
                    let source = if is_static {
                        inherited_static_source.get(&method.name).cloned()
                    } else {
                        inherited_method_source.get(&method.name).cloned()
                    };
                    OverrideKind::SingleParent {
                        source_type: source.unwrap_or_default(),
                    }
                };

                if let OverrideKind::SingleParent { source_type } = &override_kind {
                    let inherited_sig = if is_static {
                        static_method_signatures.get(&method.name).cloned()
                    } else {
                        method_signatures.get(&method.name).cloned()
                    };
                    if let Some(inherited) = inherited_sig {
                        self.check_override_signature(
                            &name,
                            &method.name,
                            source_type,
                            &inherited,
                            &new_sig,
                            is_static,
                            stmt_span,
                        );
                    }
                }
            }

            // Reserve a function slot now so the method's absolute index
            // is fixed before any body is compiled. The placeholder is
            // overwritten in pass 2 by `compile_function_body`.
            let local_idx = self.output.functions.len();
            let absolute_idx = self.fn_base_offset + local_idx;
            let param_names_vec: Vec<String> =
                method.params.iter().map(|p| p.name.clone()).collect();
            // `mut self` lives in the param list, so per-param mut
            // info reads directly from each Param's is_mut flag with
            // no special case for self.
            let param_is_mut_vec: Vec<bool> = method.params.iter().map(|p| p.is_mut).collect();
            self.output.functions.push(CompiledFunction {
                name: method.name.clone(),
                arity: method.params.len(),
                params: param_names_vec,
                param_types: param_types.clone(),
                param_is_mut: param_is_mut_vec,
                return_type: Some(return_resolved.clone()),
                num_locals: 0,
                instructions: Vec::new(),
                spans: Vec::new(),
                is_pub: method.is_pub,
                is_mut: method.is_mut(),
            });

            // Update the per-obj tables with the (possibly overridden)
            // entry. HashMap::insert replaces any existing value, which
            // is exactly the override semantics we want.
            if is_static {
                static_method_indices.insert(method.name.clone(), absolute_idx);
                static_method_is_pub.insert(method.name.clone(), method.is_pub);
                static_method_signatures.insert(method.name.clone(), new_sig);
            } else {
                method_indices.insert(method.name.clone(), absolute_idx);
                method_is_pub.insert(method.name.clone(), method.is_pub);
                method_signatures.insert(method.name.clone(), new_sig);
            }

            prepared.insert(
                method.name.clone(),
                PreparedMethod {
                    local_idx,
                    param_types,
                    return_type: return_resolved,
                    resolved_params,
                },
            );
        }

        // Mutate the obj_table snapshot so pass 2 sees the full method
        // tables (own + inherited, after override resolution). Without
        // this, an own method `a` calling `self.b()` for a sibling own
        // method `b` would fail to resolve.
        if let Some(def) = self.obj_table.defs.last_mut() {
            def.methods = method_indices.clone();
            def.static_methods = static_method_indices.clone();
            def.method_is_pub = method_is_pub.clone();
            def.static_method_is_pub = static_method_is_pub.clone();
            def.method_signatures = method_signatures.clone();
            def.static_method_signatures = static_method_signatures.clone();
        }

        // -----------------------------------------------------------------
        // Pass 2: compile each method body into its pre-allocated slot.
        // -----------------------------------------------------------------
        for method in methods {
            // Compute the method's mutability before destructuring
            // its body — `is_mut()` borrows from `params`, which is
            // partially moved by the body destructure below.
            let method_is_mut = method.is_mut();
            let Some(body) = method.body else {
                continue;
            };
            let Some(prep) = prepared.remove(&method.name) else {
                continue;
            };
            let PreparedMethod {
                local_idx,
                param_types,
                return_type,
                resolved_params,
            } = prep;

            let obj_name_for_closure = name.clone();
            let module_for_closure = self.current_module_path.clone();
            // `self` and non-self params share the same binding rule
            // now: `mut x` (or `mut self`) makes it mutable, plain
            // `x` (or plain `self`) makes it immutable. The two cases
            // collapse cleanly because `mut self` is encoded the same
            // way as any other `mut`-prefixed parameter. The only
            // difference is that `self`'s type is the enclosing obj.
            let param_fn = move |p: &crate::parser::Param| {
                let kind = match (p.name == "self", p.is_mut) {
                    // `mut self` — bound mutably so the body can
                    // write self's fields, call mutating methods on
                    // self's list fields, and call other `mut self`
                    // methods on self.
                    (true, true) => BindingKind::SelfRef,
                    // Plain `self` — read-only. Modeled as `Param`
                    // so the immutability check rejects writes
                    // through self with the same error machinery as
                    // any other immutable binding.
                    (true, false) => BindingKind::Param,
                    // `mut x: T` (non-self) — opt-in mutable param.
                    (false, true) => BindingKind::MutParam,
                    // Plain `x: T` — immutable, no opt-out.
                    (false, false) => BindingKind::Param,
                };
                let ty = if p.name == "self" {
                    ResolvedType::Object {
                        name: obj_name_for_closure.clone(),
                        module: module_for_closure.clone(),
                    }
                } else {
                    resolved_params
                        .get(&p.name)
                        .cloned()
                        .unwrap_or(ResolvedType::Unknown)
                };
                (kind, ty)
            };

            self.compile_function_body(FunctionBodyConfig {
                name: &method.name,
                params: &method.params,
                param_types,
                param_local_fn: &param_fn,
                self_name: None,
                body,
                span: stmt_span,
                is_pub: method.is_pub,
                is_mut: method_is_mut,
                return_type: Some(return_type),
                pre_allocated_local_idx: Some(local_idx),
            });
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
            // its param/return shape already matched when the parent
            // type was compiled, so skip the shape recheck. The mut
            // agreement check below still runs, because the inherited
            // method may take a `self` of different mutability from
            // what the required signature wants.

            // Check `mut fn` agreement. A `mut fn` signature must be
            // implemented by a `mut fn` method, and a plain `fn`
            // signature by a plain `fn`. The mutability contract is
            // part of the type, not just the body, so we run this
            // for both own-implemented and inherited methods.
            if has_impl {
                let impl_is_mut = if req.is_static {
                    static_method_signatures
                        .get(&req.name)
                        .map(|s| s.is_mut)
                        .unwrap_or(false)
                } else {
                    method_signatures
                        .get(&req.name)
                        .map(|s| s.is_mut)
                        .unwrap_or(false)
                };
                if impl_is_mut != req.is_mut {
                    let want = if req.is_mut {
                        "`mut self`"
                    } else {
                        "plain `self`"
                    };
                    let got = if impl_is_mut {
                        "`mut self`"
                    } else {
                        "plain `self`"
                    };
                    self.output.errors.push(OrynError::compiler(
                        stmt_span.clone(),
                        format!(
                            "method `{}` mutability mismatch: signature requires {want}, implementation has {got}",
                            req.name
                        ),
                    ));
                }
            }
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

    /// Verify that an own method signature matches the inherited
    /// signature it overrides. Param types must agree pairwise; the
    /// return type must agree exactly, except that an override may
    /// return the using type where the inherited method returned its
    /// defining type (Self-return covariance). `Unknown` matches
    /// anything to keep the check permissive in the presence of
    /// type-resolution errors elsewhere.
    ///
    /// Mut interaction (W12): a `mut fn` may be overridden by a plain
    /// `fn` (override is *more* restrictive — strictly safer for val
    /// callers). A plain `fn` may NOT be overridden by a `mut fn`
    /// (would let mutation flow where the inherited contract said it
    /// couldn't, breaking val callers who relied on the parent
    /// signature). Same direction Java/C# use for visibility narrowing.
    #[allow(clippy::too_many_arguments)]
    fn check_override_signature(
        &mut self,
        own_obj_name: &str,
        method_name: &str,
        source_type_name: &str,
        inherited: &FunctionSignature,
        own: &FunctionSignature,
        is_static: bool,
        stmt_span: &Span,
    ) {
        let kind = if is_static { "static method" } else { "method" };

        // Mut/non-mut asymmetry: stricter overrides allowed, looser
        // ones forbidden. Static methods don't dispatch through self
        // and so the rule doesn't apply to them.
        if !is_static && own.is_mut && !inherited.is_mut {
            self.output.errors.push(OrynError::compiler(
                stmt_span.clone(),
                format!(
                    "{kind} `{method_name}` overrides `use {source_type_name}`'s `{method_name}` with `mut self`, but the inherited method takes plain `self`; cannot widen mutation contract"
                ),
            ));
        }

        if inherited.param_types.len() != own.param_types.len() {
            self.output.errors.push(OrynError::compiler(
                stmt_span.clone(),
                format!(
                    "{kind} `{method_name}` overrides `use {source_type_name}`'s `{method_name}` but has {} parameter(s) instead of {}",
                    own.param_types.len(),
                    inherited.param_types.len(),
                ),
            ));
            return;
        }

        for (i, (inh, own_t)) in inherited
            .param_types
            .iter()
            .zip(&own.param_types)
            .enumerate()
        {
            if *inh == ResolvedType::Unknown || *own_t == ResolvedType::Unknown {
                continue;
            }
            if inh != own_t {
                self.output.errors.push(OrynError::compiler(
                    stmt_span.clone(),
                    format!(
                        "{kind} `{method_name}` overrides `use {source_type_name}`'s `{method_name}` but parameter {} type differs: expected `{}`, got `{}`",
                        i + 1,
                        inh.display_name(),
                        own_t.display_name(),
                    ),
                ));
            }
        }

        // Self-return covariance: an override may return the using type
        // where the inherited method returned its defining type. We
        // recognise this by comparing the inherited return type to an
        // Object whose name matches the source type, and the override
        // return type to an Object whose name matches the using type.
        let self_covariant = matches!(
            (&inherited.return_type, &own.return_type),
            (
                ResolvedType::Object { name: inh_name, .. },
                ResolvedType::Object { name: own_name, module: own_module },
            ) if inh_name == source_type_name
                && own_name == own_obj_name
                && (own_module.is_empty() || *own_module == self.current_module_path)
        );

        if !self_covariant
            && inherited.return_type != ResolvedType::Unknown
            && own.return_type != ResolvedType::Unknown
            && inherited.return_type != own.return_type
        {
            self.output.errors.push(OrynError::compiler(
                stmt_span.clone(),
                format!(
                    "{kind} `{method_name}` overrides `use {source_type_name}`'s `{method_name}` but return type differs: expected `{}`, got `{}`",
                    inherited.return_type.display_name(),
                    own.return_type.display_name(),
                ),
            ));
        }
    }
}
