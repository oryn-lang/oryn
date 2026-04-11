//! Enum declarations, constructor expressions, and match codegen.
//!
//! Tagged-union (sum) types in Oryn. An `enum Name { Variant1, ... }`
//! declaration registers a new type in the compiler's enum table; each
//! variant is either nullary (no payload) or carries named-field
//! payloads with the same shape as obj fields. Constructor expressions
//! `Name.Variant` build values; pattern matching with `match` (in
//! `expr.rs`) destructures them.
//!
//! See `WARTS.md` and the design discussion attached to this slice
//! for the full design rationale.

use crate::OrynError;
use crate::compiler::types::ResolvedType;
use crate::parser::{EnumVariant, MatchArm, Pattern, Span, Spanned};

use super::compile::Compiler;
use super::types::{EnumDefInfo, EnumVariantInfo, Instruction};

// Note: `compile_enum_def` registers the enum in BOTH
// `self.enum_table` (compile-time lookup) and
// `self.output.enum_defs` (runtime metadata accessible from the VM
// via `Chunk.enum_defs`). The `EnumTable` stores compile-time
// indexed lookups; the `output.enum_defs` vector stores the same
// metadata in declaration order so VM print/equality paths can
// reach it via the integer `def_idx`.

impl Compiler {
    /// Compile an `enum Name { ... }` declaration: resolve every
    /// variant's payload field types, register the enum in the
    /// compiler's enum table, and emit no bytecode (the declaration
    /// is purely a type-system thing).
    ///
    /// `is_error` is `true` when the declaration is prefixed with
    /// `error` (`error enum Foo { ... }`). Error enums' values promote
    /// into the error side of any `error T` union and are recognized
    /// by the VM's `JumpIfError` / `UnwrapErrorOrTrap` handlers via
    /// the flag on [`EnumDefInfo`].
    pub(super) fn compile_enum_def(
        &mut self,
        name: String,
        variants: Vec<EnumVariant>,
        stmt_span: &Span,
        is_pub: bool,
        is_error: bool,
    ) {
        // Reject empty enums at the compiler level. The parser
        // also rejects them, but defensive double-check.
        if variants.is_empty() {
            self.output.errors.push(OrynError::compiler(
                stmt_span.clone(),
                format!("enum `{name}` must have at least one variant"),
            ));
            return;
        }

        let mut variant_infos: Vec<EnumVariantInfo> = Vec::new();
        let mut seen_names: Vec<String> = Vec::new();

        for variant in variants {
            if seen_names.contains(&variant.name) {
                self.output.errors.push(OrynError::compiler(
                    variant.span.clone(),
                    format!("duplicate variant `{}` in enum `{name}`", variant.name),
                ));
                // Don't push the duplicate; keep going so the rest
                // of the enum still type-checks coherently.
                continue;
            }
            seen_names.push(variant.name.clone());

            // Resolve each payload field's type. Same machinery as
            // obj field type resolution.
            let mut field_names: Vec<String> = Vec::new();
            let mut field_types: Vec<ResolvedType> = Vec::new();
            let mut field_dup = false;
            for field in &variant.fields {
                if field_names.contains(&field.name) {
                    self.output.errors.push(OrynError::compiler(
                        field.span.clone(),
                        format!(
                            "duplicate field `{}` in variant `{}` of enum `{name}`",
                            field.name, variant.name
                        ),
                    ));
                    field_dup = true;
                    continue;
                }
                field_names.push(field.name.clone());
                match self.resolve_type_annotation(&field.type_ann) {
                    Ok(t) => field_types.push(t),
                    Err(msg) => {
                        self.output.errors.push(OrynError::compiler(
                            field.span.clone(),
                            format!(
                                "field `{}` in variant `{}` of enum `{name}`: {msg}",
                                field.name, variant.name
                            ),
                        ));
                        field_types.push(ResolvedType::Unknown);
                    }
                }
            }
            // Even with field errors we still register the variant
            // so subsequent constructor / pattern code can find it
            // by name. The Unknown placeholders keep type checking
            // permissive in the presence of upstream errors.
            let _ = field_dup;

            variant_infos.push(EnumVariantInfo {
                name: variant.name,
                field_names,
                field_types,
            });
        }

        // Register the enum in the compile-time table for lookup
        // by name during constructor / pattern compilation.
        let abs_idx =
            self.enum_table
                .register(name.clone(), variant_infos.clone(), is_pub, is_error);

        // Mirror the registration into output.enum_defs so the VM
        // can reach the metadata at runtime via the integer def_idx
        // baked into Value::Enum and Instruction::MakeEnum. The
        // EnumTable currently uses base_offset=0, so the absolute
        // index lines up with output.enum_defs.len() exactly.
        debug_assert_eq!(abs_idx, self.output.enum_defs.len());
        self.output.enum_defs.push(EnumDefInfo {
            name,
            variants: variant_infos,
            is_pub,
            is_error,
        });
    }

    /// Compile a constructor expression like `FsResult.NotFound` or
    /// `FsResult.Ok { content: "hi" }`. Called from `expr.rs` when
    /// the parser produces an enum-typed dotted path or an obj
    /// literal whose type name resolves to an enum.
    ///
    /// `fields` is the supplied named-field set from the literal (or
    /// empty for nullary variants). Field order in the source doesn't
    /// matter; the compiler reorders to declaration order before
    /// emitting `MakeEnum`. Missing fields, extra fields, and
    /// type-mismatched fields all produce errors but the function
    /// still emits `MakeEnum` with the right stack shape so the
    /// downstream type-check stays coherent.
    pub(super) fn compile_enum_constructor(
        &mut self,
        enum_name: &str,
        variant_name: &str,
        fields: Vec<(String, crate::parser::Spanned<crate::parser::Expression>)>,
        span: &Span,
    ) -> ResolvedType {
        // Resolve the variant. We clone the variant_info immediately
        // so we can release the borrow on `self.enum_table` and call
        // `&mut self` methods (compile_expr, check_types) below.
        let Some((enum_idx, variant_idx, variant_info)) = self
            .enum_table
            .resolve_variant(enum_name, variant_name)
            .map(|(e, v, info)| (e, v, info.clone()))
        else {
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                format!("no variant `{variant_name}` on enum `{enum_name}`"),
            ));
            self.emit(Instruction::PushInt(0), span);
            return ResolvedType::Unknown;
        };

        let expected_count = variant_info.field_names.len();
        let supplied_count = fields.len();

        if expected_count == 0 && supplied_count > 0 {
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                format!("variant `{enum_name}.{variant_name}` is nullary; remove the field block"),
            ));
        } else if expected_count > 0 && supplied_count == 0 {
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                format!(
                    "variant `{enum_name}.{variant_name}` requires fields {{ {} }}",
                    variant_info.field_names.join(", ")
                ),
            ));
        }

        // Drain the supplied fields into a name → expression map so
        // each can be consumed by value when we look it up below.
        // Detect duplicate field names while we're building the map.
        let mut supplied: std::collections::HashMap<
            String,
            crate::parser::Spanned<crate::parser::Expression>,
        > = std::collections::HashMap::new();
        for (name, value_expr) in fields {
            if supplied.contains_key(&name) {
                self.output.errors.push(OrynError::compiler(
                    value_expr.span.clone(),
                    format!("duplicate field `{name}` in variant `{enum_name}.{variant_name}`"),
                ));
                continue;
            }
            // Validate the name belongs to the variant. We still
            // insert so the later "missing field" loop produces
            // sensible numbers.
            if !variant_info.field_names.contains(&name) {
                self.output.errors.push(OrynError::compiler(
                    value_expr.span.clone(),
                    format!("unknown field `{name}` on variant `{enum_name}.{variant_name}`"),
                ));
                // Skip storing it — there's no expected slot to put
                // it in, and we don't want it shadowing a later
                // legitimate field of the same name.
                continue;
            }
            supplied.insert(name, value_expr);
        }

        // Push field values in declaration order. For each expected
        // field we either consume the supplied value or push a
        // sentinel placeholder if it's missing.
        for (expected_name, expected_type) in variant_info
            .field_names
            .iter()
            .zip(variant_info.field_types.iter())
        {
            match supplied.remove(expected_name) {
                Some(value_expr) => {
                    let value_span = value_expr.span.clone();
                    let value_type = self.compile_expr(value_expr);
                    self.check_types(
                        expected_type,
                        &value_type,
                        &value_span,
                        &format!(
                            "field `{expected_name}` of variant `{enum_name}.{variant_name}` type mismatch"
                        ),
                    );
                }
                None => {
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!(
                            "missing field `{expected_name}` in variant `{enum_name}.{variant_name}`"
                        ),
                    ));
                    // Placeholder so MakeEnum still finds the right
                    // number of values on the stack.
                    self.emit(Instruction::PushInt(0), span);
                }
            }
        }

        self.emit(
            Instruction::MakeEnum(enum_idx, variant_idx, expected_count),
            span,
        );

        // Look up the is_error flag from the enum table we already
        // resolved via `resolve_variant` above. The result type
        // mirrors the flag so downstream type checks (return-type
        // coercion into `error T`) can see it without a second lookup.
        let is_error = self
            .enum_table
            .resolve(enum_name)
            .map(|(_, def)| def.is_error)
            .unwrap_or(false);

        ResolvedType::Enum {
            name: enum_name.to_string(),
            module: self.current_module_path.clone(),
            is_error,
        }
    }

    /// Compile a `match` expression. Slice 3 supports tag-only and
    /// payload-binding patterns (`Variant` and `Variant { field, … }`)
    /// plus the wildcard `_`. Slice 4 hardens exhaustiveness reporting
    /// and rejects wildcards that come after every variant has already
    /// been listed explicitly.
    ///
    /// **Codegen shape** (scrutinee stashed in an anonymous local
    /// so each arm can re-read it for both the discriminant test and
    /// payload extraction):
    /// ```text
    ///   <compile scrutinee>             ; [scrut]
    ///   SetLocal(scrut_slot)            ; []
    /// arm_n (variant pattern):
    ///   GetLocal(scrut_slot)            ; [scrut]
    ///   EnumDiscriminant                ; [int]
    ///   PushInt(variant_idx)
    ///   Equal                           ; [bool]
    ///   JumpIfFalse → arm_n+1           ; pops bool; falls through if true
    ///   ; matched — bind any payload fields
    ///   GetLocal(scrut_slot); GetEnumPayload(field_idx); SetLocal(b1)
    ///   GetLocal(scrut_slot); GetEnumPayload(field_idx); SetLocal(b2)
    ///   …
    ///   <compile arm body>              ; body's value lands on TOS
    ///   Jump → end
    /// arm_n (wildcard):
    ///   <compile arm body>              ; no test, no fetch
    ///   Jump → end
    /// end:
    /// ```
    ///
    /// **Per-arm scoping**: payload bindings live only in the arm
    /// body. We snapshot `Locals` before each body and restore
    /// after, so binding slots don't leak across arms. The
    /// `Locals.max_count` field tracks the high-water mark so the
    /// function reserves enough storage for whichever arm needs
    /// the most slots.
    ///
    /// **No fall-through cleanup**: each arm body lands its value
    /// directly on the stack, then jumps to `end`. There's no
    /// stranded discriminant to drop. The exhaustiveness check
    /// prevents unmatched scrutinees at compile time.
    ///
    /// **Exhaustiveness** (Slice 4): a set-difference check
    /// (declared variants − arm-covered variants) runs after
    /// codegen. If any variant is uncovered AND no `_` arm is
    /// present, an error is pushed listing the missing variants
    /// fully qualified. Conversely, a `_` arm that appears AFTER
    /// every variant is already covered is itself flagged as
    /// unreachable.
    ///
    /// **Result type**: every arm body's type is computed and
    /// the first non-Unknown arm's type is used as the match
    /// expression's type. Subsequent arms must agree.
    pub(super) fn compile_match_expression(
        &mut self,
        scrutinee: Spanned<crate::parser::Expression>,
        arms: Vec<MatchArm>,
        span: &Span,
    ) -> ResolvedType {
        if arms.is_empty() {
            self.output.errors.push(OrynError::compiler(
                span.clone(),
                "match expression requires at least one arm",
            ));
            self.emit(Instruction::PushInt(0), span);
            return ResolvedType::Unknown;
        }

        // 1. Compile the scrutinee. Its type must resolve to an enum.
        let scrutinee_type = self.compile_expr(scrutinee);
        let (enum_name, enum_def): (String, EnumDefInfo) = match &scrutinee_type {
            ResolvedType::Enum { name, .. } => match self.enum_table.resolve(name) {
                Some((_, def)) => (name.clone(), def.clone()),
                None => {
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        format!("unknown enum `{name}` for match scrutinee"),
                    ));
                    self.emit(Instruction::Pop, span);
                    self.emit(Instruction::PushInt(0), span);
                    return ResolvedType::Unknown;
                }
            },
            ResolvedType::Unknown => {
                self.emit(Instruction::Pop, span);
                self.emit(Instruction::PushInt(0), span);
                return ResolvedType::Unknown;
            }
            other => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!(
                        "match scrutinee must be an enum value, got `{}`",
                        other.display_name()
                    ),
                ));
                self.emit(Instruction::Pop, span);
                self.emit(Instruction::PushInt(0), span);
                return ResolvedType::Unknown;
            }
        };

        // 2. Stash the scrutinee in an anonymous local so each arm
        //    can re-read it for the discriminant test and for
        //    payload extraction. The local is named with a leading
        //    `@` so user code cannot collide with it.
        let scrut_slot = self.locals.define(
            "@match_scrutinee".to_string(),
            super::tables::BindingKind::Internal,
            scrutinee_type.clone(),
        );
        self.emit(Instruction::SetLocal(scrut_slot), span);

        // 3. Per-arm dispatch + body. Track coverage for
        //    exhaustiveness, collect end-jumps for patching, and
        //    reconcile arm result types.
        let mut covered: Vec<bool> = vec![false; enum_def.variants.len()];
        let mut has_wildcard = false;
        let mut end_jumps: Vec<usize> = Vec::new();
        let mut result_type: Option<ResolvedType> = None;
        let mut wildcard_warning_pushed = false;

        for arm in arms {
            let MatchArm {
                pattern,
                body,
                span: arm_span,
            } = arm;

            // Resolve pattern → (variant_idx, bindings). None for
            // variant_idx means wildcard or unresolvable. Coverage
            // is tracked here so the exhaustiveness pass below sees
            // the post-arm state. Slice 4 also flags wildcards that
            // come after every variant has been listed.
            //
            // For variant patterns, also validate the brace
            // bindings (if any) against the variant's payload
            // declaration. The validated `(field_idx, name, type)`
            // triples are used during codegen to emit the
            // GetEnumPayload + SetLocal sequence inside the per-arm
            // scope.
            struct ResolvedBinding {
                field_idx: usize,
                name: String,
                ty: ResolvedType,
            }

            let mut resolved_bindings: Vec<ResolvedBinding> = Vec::new();
            let variant_idx: Option<usize> = match &pattern.node {
                Pattern::Wildcard => {
                    if has_wildcard && !wildcard_warning_pushed {
                        self.output.errors.push(OrynError::compiler(
                            pattern.span.clone(),
                            "match arm is unreachable: a previous `_` already matches everything"
                                .to_string(),
                        ));
                        wildcard_warning_pushed = true;
                    }
                    // Slice 4: a wildcard arm that appears after
                    // every variant has already been covered is
                    // itself unreachable. Diagnose it pointing at
                    // the wildcard's own span so the user can
                    // delete it cleanly.
                    if !has_wildcard && covered.iter().all(|c| *c) {
                        self.output.errors.push(OrynError::compiler(
                            pattern.span.clone(),
                            format!(
                                "wildcard arm is unreachable: all variants of enum `{enum_name}` are already covered",
                            ),
                        ));
                    }
                    has_wildcard = true;
                    None
                }
                Pattern::Variant {
                    enum_name: pat_enum,
                    variant_name,
                    bindings,
                } => {
                    if pat_enum != &enum_name {
                        self.output.errors.push(OrynError::compiler(
                            pattern.span.clone(),
                            format!(
                                "pattern type `{pat_enum}` does not match scrutinee enum `{enum_name}`"
                            ),
                        ));
                        None
                    } else {
                        match enum_def
                            .variants
                            .iter()
                            .position(|v| &v.name == variant_name)
                        {
                            Some(idx) => {
                                if covered[idx] {
                                    self.output.errors.push(OrynError::compiler(
                                        pattern.span.clone(),
                                        format!(
                                            "duplicate match arm: variant `{enum_name}.{variant_name}` already covered"
                                        ),
                                    ));
                                }
                                covered[idx] = true;

                                // Validate payload bindings against
                                // the variant's declared fields.
                                // `bindings == None` means the user
                                // wrote a tag-only pattern (no
                                // braces) — always allowed regardless
                                // of whether the variant has a
                                // payload. `bindings == Some(...)`
                                // means braces were present and we
                                // need to validate the contents.
                                let variant = &enum_def.variants[idx];
                                if let Some(bs) = bindings {
                                    if variant.field_names.is_empty() {
                                        // Nullary variant + braces:
                                        // category error, mirrors
                                        // the constructor-side rule.
                                        // Covers both empty `{ }`
                                        // and bogus `{ x }` cases.
                                        self.output.errors.push(OrynError::compiler(
                                            pattern.span.clone(),
                                            format!(
                                                "variant `{enum_name}.{variant_name}` is nullary; remove the `{{ }}` block from the pattern"
                                            ),
                                        ));
                                    } else if bs.is_empty() {
                                        // Empty braces on a payload
                                        // variant: useless syntax,
                                        // tell the user to drop them.
                                        self.output.errors.push(OrynError::compiler(
                                            pattern.span.clone(),
                                            format!(
                                                "empty `{{ }}` block in pattern for `{enum_name}.{variant_name}`; drop the braces or list at least one field binding"
                                            ),
                                        ));
                                    } else {
                                        // Detect duplicate `name`
                                        // collisions inside the
                                        // same brace block — the
                                        // user wrote e.g.
                                        // `Variant { x, x }` or
                                        // `Variant { x: a, y: a }`.
                                        let mut seen_local_names: Vec<&str> = Vec::new();

                                        for binding in bs {
                                            match variant
                                                .field_names
                                                .iter()
                                                .position(|f| f == &binding.field)
                                            {
                                                Some(field_idx) => {
                                                    if seen_local_names
                                                        .contains(&binding.name.as_str())
                                                    {
                                                        self.output.errors.push(
                                                            OrynError::compiler(
                                                                binding.span.clone(),
                                                                format!(
                                                                    "duplicate binding name `{}` in pattern for `{enum_name}.{variant_name}`",
                                                                    binding.name
                                                                ),
                                                            ),
                                                        );
                                                    } else {
                                                        seen_local_names.push(&binding.name);
                                                    }

                                                    resolved_bindings.push(ResolvedBinding {
                                                        field_idx,
                                                        name: binding.name.clone(),
                                                        ty: variant.field_types[field_idx].clone(),
                                                    });
                                                }
                                                None => {
                                                    self.output.errors.push(OrynError::compiler(
                                                        binding.span.clone(),
                                                        format!(
                                                            "unknown field `{}` on variant `{enum_name}.{variant_name}`",
                                                            binding.field
                                                        ),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }

                                Some(idx)
                            }
                            None => {
                                self.output.errors.push(OrynError::compiler(
                                    pattern.span.clone(),
                                    format!("no variant `{variant_name}` on enum `{enum_name}`"),
                                ));
                                None
                            }
                        }
                    }
                }
            };

            // Emit per-arm dispatch.
            //
            // Variant arms: GetLocal(scrut) → EnumDiscriminant →
            //               PushInt → Equal → JumpIfFalse → next.
            //               Then bind any payload fields, then
            //               compile the body.
            // Wildcard arms: skip the test entirely; the body
            //                always runs (until either matched or
            //                an earlier arm took the value).
            let skip_arm_jump_idx: Option<usize> = if let Some(idx) = variant_idx {
                self.emit(Instruction::GetLocal(scrut_slot), &arm_span);
                self.emit(Instruction::EnumDiscriminant, &arm_span);
                self.emit(Instruction::PushInt(idx as i32), &arm_span);
                self.emit(Instruction::Equal, &arm_span);
                let jump_idx = self.output.instructions.len();
                self.emit(Instruction::JumpIfFalse(0), &arm_span); // patched below
                Some(jump_idx)
            } else {
                None
            };

            // Compile the arm body inside a fresh scope so payload
            // bindings vanish at the end. The scope also bounds the
            // resolved binding locals introduced via the
            // GetEnumPayload sequence below.
            let body_span = body.span.clone();
            let body_type = self.with_scope(|this| {
                // Bind any resolved payload fields as locals in the
                // arm body. Each binding emits:
                //   GetLocal(scrut); GetEnumPayload(idx); SetLocal(slot)
                for binding in &resolved_bindings {
                    this.emit(Instruction::GetLocal(scrut_slot), &arm_span);
                    this.emit(Instruction::GetEnumPayload(binding.field_idx), &arm_span);
                    let slot = this.locals.define(
                        binding.name.clone(),
                        super::tables::BindingKind::Let,
                        binding.ty.clone(),
                    );
                    this.emit(Instruction::SetLocal(slot), &arm_span);
                }
                this.compile_expr(body)
            });

            match &result_type {
                None => result_type = Some(body_type),
                Some(existing) => {
                    if existing != &ResolvedType::Unknown
                        && body_type != ResolvedType::Unknown
                        && existing != &body_type
                    {
                        self.output.errors.push(OrynError::compiler(
                            body_span,
                            format!(
                                "match arm result type mismatch: expected `{}`, got `{}`",
                                existing.display_name(),
                                body_type.display_name()
                            ),
                        ));
                    }
                }
            }

            // Jump to end so subsequent arms don't run after a match.
            let end_jump_idx = self.output.instructions.len();
            self.emit(Instruction::Jump(0), &arm_span); // patched below
            end_jumps.push(end_jump_idx);

            // Patch the per-arm skip jump (variant patterns only)
            // so the false branch lands at the next arm's test.
            if let Some(jump_idx) = skip_arm_jump_idx {
                let next_arm_addr = self.output.instructions.len();
                self.output.instructions[jump_idx] = Instruction::JumpIfFalse(next_arm_addr);
            }
        }

        // Patch all end-jumps to land here. No fall-through
        // cleanup is needed: each arm body produces its value
        // directly on the stack, and the exhaustiveness check
        // ensures we don't reach the end without a match at
        // runtime. (For the diagnostic-only case where compilation
        // proceeds despite a non-exhaustive match, an unmatched
        // value would land here with whatever the last
        // JumpIfFalse left on the stack — but the user already
        // has a hard compile error, so the runtime path is moot.)
        let end_addr = self.output.instructions.len();
        for jump_idx in end_jumps {
            self.output.instructions[jump_idx] = Instruction::Jump(end_addr);
        }

        // Exhaustiveness check.
        if !has_wildcard {
            let missing: Vec<String> = enum_def
                .variants
                .iter()
                .zip(covered.iter())
                .filter(|(_, c)| !**c)
                .map(|(v, _)| format!("{enum_name}.{}", v.name))
                .collect();
            if !missing.is_empty() {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!(
                        "non-exhaustive match: missing variants `{}` (add `_ => ...` to catch the rest)",
                        missing.join("`, `"),
                    ),
                ));
            }
        }

        result_type.unwrap_or(ResolvedType::Unknown)
    }
}
