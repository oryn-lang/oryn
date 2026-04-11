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

// Note: `finalize_enum_def` populates BOTH `self.enum_table`
// (compile-time lookup) and `self.output.enum_defs` (runtime metadata
// accessible from the VM via `Chunk.enum_defs`). The `EnumTable`
// stores compile-time indexed lookups; the `output.enum_defs` vector
// stores the same metadata in declaration order so VM print/equality
// paths can reach it via the integer `def_idx`. Under the Phase A /
// Phase B compile pipeline (`compile.rs`), every enum is seeded with
// an empty placeholder via [`Compiler::seed_enum_placeholder`] before
// any enum's variants are resolved, and `finalize_enum_def` fills
// the placeholder in place.

impl Compiler {
    /// Resolve an enum's variant payload types and write the result
    /// into the seeded placeholder entry in both `enum_table` and
    /// `output.enum_defs`. Called from Phase A2 in `compile.rs` after
    /// every enum placeholder has been seeded, so payload type
    /// annotations can forward-reference any enum or obj in the module.
    pub(super) fn finalize_enum_def(
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

        // Update both the enum_table and the output.enum_defs entry
        // in place at the placeholder's local index. Phase A seeding
        // grows the two vectors in lockstep, so the same index covers
        // both. EnumTable uses base_offset=0 currently, so the
        // absolute index returned by `resolve` is also the local one.
        let local_idx = self
            .enum_table
            .resolve(&name)
            .map(|(absolute, _)| absolute - self.enum_table.base_offset)
            .expect("enum placeholder missing; seed it before calling finalize_enum_def");
        let finalized = EnumDefInfo {
            name,
            variants: variant_infos,
            is_pub,
            is_error,
        };
        self.enum_table.defs[local_idx] = finalized.clone();
        self.output.enum_defs[local_idx] = finalized;
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

        // 1. Compile the scrutinee. Accept one of two shapes:
        //    * Plain enum — existing single-enum dispatch, arms are
        //      variants of that enum plus an optional wildcard.
        //    * Error union — arms include `ok v` for the success
        //      side, variant patterns for error-enum variants, and
        //      optional wildcard. In loose mode, arm variants can
        //      come from any in-scope error enum. In precise mode,
        //      variants must come from the named error enum.
        let scrutinee_type = self.compile_expr(scrutinee);
        let match_mode: MatchMode = match &scrutinee_type {
            ResolvedType::Enum { name, .. } => match self.enum_table.resolve(name) {
                Some((_, def)) => MatchMode::PlainEnum {
                    enum_name: name.clone(),
                    enum_def: def.clone(),
                },
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
            ResolvedType::ErrorUnion { error_enum, inner } => {
                let precise = match error_enum {
                    None => None,
                    Some((name, _module)) => match self.enum_table.resolve(name) {
                        Some((_, def)) => Some(def.clone()),
                        None => {
                            self.output.errors.push(OrynError::compiler(
                                span.clone(),
                                format!("unknown precise error enum `{name}` for match scrutinee"),
                            ));
                            self.emit(Instruction::Pop, span);
                            self.emit(Instruction::PushInt(0), span);
                            return ResolvedType::Unknown;
                        }
                    },
                };
                MatchMode::ErrorUnion {
                    success_type: (**inner).clone(),
                    precise,
                }
            }
            ResolvedType::Unknown => {
                self.emit(Instruction::Pop, span);
                self.emit(Instruction::PushInt(0), span);
                return ResolvedType::Unknown;
            }
            other => {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!(
                        "match scrutinee must be an enum or error union, got `{}`",
                        other.display_name()
                    ),
                ));
                self.emit(Instruction::Pop, span);
                self.emit(Instruction::PushInt(0), span);
                return ResolvedType::Unknown;
            }
        };

        // 2. Stash the scrutinee in an anonymous local so each arm
        //    can re-read it for dispatch tests and payload extraction.
        let scrut_slot = self.locals.define(
            "@match_scrutinee".to_string(),
            super::tables::BindingKind::Internal,
            scrutinee_type.clone(),
        );
        self.emit(Instruction::SetLocal(scrut_slot), span);

        // 3. Per-arm dispatch + body. The kinds of arms we need to
        //    handle depend on the match mode; see `ResolvedArm` below.
        let mut end_jumps: Vec<usize> = Vec::new();
        let mut result_type: Option<ResolvedType> = None;
        let mut wildcard_warning_pushed = false;

        // Coverage for plain-enum exhaustiveness.
        let mut plain_covered: Vec<bool> = match &match_mode {
            MatchMode::PlainEnum { enum_def, .. } => vec![false; enum_def.variants.len()],
            _ => Vec::new(),
        };
        // Coverage for precise error-union exhaustiveness.
        let mut precise_covered: Vec<bool> = match &match_mode {
            MatchMode::ErrorUnion {
                precise: Some(def), ..
            } => vec![false; def.variants.len()],
            _ => Vec::new(),
        };
        let mut has_wildcard = false;
        let mut has_ok_arm = false;

        for arm in arms {
            let MatchArm {
                pattern,
                body,
                span: arm_span,
            } = arm;

            let resolved = self.resolve_arm_pattern(
                &pattern,
                &match_mode,
                &mut plain_covered,
                &mut precise_covered,
                &mut has_wildcard,
                &mut has_ok_arm,
                &mut wildcard_warning_pushed,
            );

            // Emit per-arm dispatch. Each arm ends in one of:
            //   (a) a JumpIfFalse (variant arms) that lands at the
            //       next arm's dispatch start; or
            //   (b) a JumpIfError + Pop cleanup (ok arms); or
            //   (c) no test at all (wildcard and unresolvable arms).
            //
            // For each arm we collect the patch sites for its
            // "arm failed, go to next arm" jumps and patch them
            // once we know the address of the next arm.
            let mut skip_patch_sites: Vec<usize> = Vec::new();
            match &resolved {
                ResolvedArm::Unresolvable | ResolvedArm::Wildcard => {
                    // No pre-body dispatch; body always runs.
                }
                ResolvedArm::Ok { .. } => {
                    // `ok v` arm: the success side is "scrut is not
                    // an error enum". Use JumpIfError as a peek test
                    // to skip. If we take the skip, the scrut value
                    // we pushed to probe is still on the stack and
                    // needs cleanup at the skip target.
                    self.emit(Instruction::GetLocal(scrut_slot), &arm_span);
                    let jump_idx = self.output.instructions.len();
                    self.emit(Instruction::JumpIfError(0), &arm_span); // patched below
                    skip_patch_sites.push(jump_idx);
                }
                ResolvedArm::Variant {
                    enum_def_idx,
                    variant_idx,
                    ..
                } => {
                    // Error-union variants need both a def_idx check
                    // and a variant_idx check. Plain-enum matches
                    // skip the def_idx check because the scrutinee
                    // type already pins the enum.
                    if matches!(match_mode, MatchMode::ErrorUnion { .. }) {
                        self.emit(Instruction::GetLocal(scrut_slot), &arm_span);
                        self.emit(Instruction::EnumDefIdx, &arm_span);
                        self.emit(Instruction::PushInt(*enum_def_idx as i32), &arm_span);
                        self.emit(Instruction::Equal, &arm_span);
                        let jump_idx = self.output.instructions.len();
                        self.emit(Instruction::JumpIfFalse(0), &arm_span);
                        skip_patch_sites.push(jump_idx);
                    }
                    self.emit(Instruction::GetLocal(scrut_slot), &arm_span);
                    self.emit(Instruction::EnumDiscriminant, &arm_span);
                    self.emit(Instruction::PushInt(*variant_idx as i32), &arm_span);
                    self.emit(Instruction::Equal, &arm_span);
                    let jump_idx = self.output.instructions.len();
                    self.emit(Instruction::JumpIfFalse(0), &arm_span);
                    skip_patch_sites.push(jump_idx);
                }
            }

            // Compile the arm body inside a fresh scope so bindings
            // introduced by the pattern vanish at the end.
            let body_span = body.span.clone();
            let body_type = self.with_scope(|this| {
                match &resolved {
                    ResolvedArm::Ok { name, success_type } => {
                        // The JumpIfError peek left the scrut value
                        // on the stack when we fell through. Bind it
                        // to the `ok` name as an immutable local.
                        let slot = this.locals.define(
                            name.clone(),
                            super::tables::BindingKind::Val,
                            success_type.clone(),
                        );
                        this.emit(Instruction::SetLocal(slot), &arm_span);
                    }
                    ResolvedArm::Variant { bindings, .. } => {
                        // Each payload binding emits:
                        //   GetLocal(scrut); GetEnumPayload(idx); SetLocal(slot)
                        for binding in bindings {
                            this.emit(Instruction::GetLocal(scrut_slot), &arm_span);
                            this.emit(Instruction::GetEnumPayload(binding.field_idx), &arm_span);
                            let slot = this.locals.define(
                                binding.name.clone(),
                                super::tables::BindingKind::Let,
                                binding.ty.clone(),
                            );
                            this.emit(Instruction::SetLocal(slot), &arm_span);
                        }
                    }
                    ResolvedArm::Wildcard | ResolvedArm::Unresolvable => {}
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

            // Jump past the remaining arms after a matched body
            // runs. Patched to the final end address below.
            let end_jump_idx = self.output.instructions.len();
            self.emit(Instruction::Jump(0), &arm_span);
            end_jumps.push(end_jump_idx);

            // Patch the per-arm skip jumps so they land at the next
            // arm's dispatch start. For `ok v` arms, the skip
            // target needs to pop the leftover scrut value that was
            // peeked by JumpIfError — we insert a Pop instruction
            // right here so the next arm starts with a clean stack.
            if !skip_patch_sites.is_empty() {
                if matches!(resolved, ResolvedArm::Ok { .. }) {
                    // JumpIfError leaves the peeked value on the
                    // stack; clean it up before the next arm runs.
                    let pop_addr = self.output.instructions.len();
                    self.emit(Instruction::Pop, &arm_span);
                    for jump_idx in &skip_patch_sites {
                        self.output.instructions[*jump_idx] = Instruction::JumpIfError(pop_addr);
                    }
                } else {
                    let next_arm_addr = self.output.instructions.len();
                    for jump_idx in &skip_patch_sites {
                        // JumpIfFalse after each comparison.
                        self.output.instructions[*jump_idx] =
                            Instruction::JumpIfFalse(next_arm_addr);
                    }
                }
            }
        }

        // Patch all end-jumps.
        let end_addr = self.output.instructions.len();
        for jump_idx in end_jumps {
            self.output.instructions[jump_idx] = Instruction::Jump(end_addr);
        }

        // 4. Exhaustiveness checks. Differ per match mode.
        match &match_mode {
            MatchMode::PlainEnum {
                enum_name,
                enum_def,
            } => {
                if !has_wildcard {
                    let missing: Vec<String> = enum_def
                        .variants
                        .iter()
                        .zip(plain_covered.iter())
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
            }
            MatchMode::ErrorUnion {
                precise: Some(def), ..
            } => {
                // Precise scrutinee: full exhaustiveness possible.
                // Require `ok v` + every variant of `def`, OR `_`.
                if !has_wildcard {
                    let mut missing: Vec<String> = def
                        .variants
                        .iter()
                        .zip(precise_covered.iter())
                        .filter(|(_, c)| !**c)
                        .map(|(v, _)| format!("{}.{}", def.name, v.name))
                        .collect();
                    if !has_ok_arm {
                        missing.push("ok".to_string());
                    }
                    if !missing.is_empty() {
                        self.output.errors.push(OrynError::compiler(
                            span.clone(),
                            format!(
                                "non-exhaustive match: missing `{}` (add `_ => ...` to catch the rest)",
                                missing.join("`, `"),
                            ),
                        ));
                    }
                }
            }
            MatchMode::ErrorUnion { precise: None, .. } => {
                // Loose scrutinee: we cannot know which error enums
                // can show up, so a wildcard is always required
                // unless the user listed `ok v` and somehow every
                // possible error variant (not statically checkable).
                // Require `_` in loose mode, period.
                if !has_wildcard {
                    self.output.errors.push(OrynError::compiler(
                        span.clone(),
                        "non-exhaustive match on loose `error T`: add `_ => ...` to catch unmatched errors",
                    ));
                }
            }
        }

        result_type.unwrap_or(ResolvedType::Unknown)
    }

    /// Resolve one match arm's pattern against the active match mode,
    /// updating coverage tracking and pushing diagnostics for
    /// category errors (wrong enum, unknown variant, `ok` in plain
    /// match, etc.). Returns a `ResolvedArm` describing how codegen
    /// should handle this arm.
    #[allow(clippy::too_many_arguments)]
    fn resolve_arm_pattern(
        &mut self,
        pattern: &Spanned<Pattern>,
        match_mode: &MatchMode,
        plain_covered: &mut [bool],
        precise_covered: &mut [bool],
        has_wildcard: &mut bool,
        has_ok_arm: &mut bool,
        wildcard_warning_pushed: &mut bool,
    ) -> ResolvedArm {
        match &pattern.node {
            Pattern::Wildcard => {
                if *has_wildcard && !*wildcard_warning_pushed {
                    self.output.errors.push(OrynError::compiler(
                        pattern.span.clone(),
                        "match arm is unreachable: a previous `_` already matches everything"
                            .to_string(),
                    ));
                    *wildcard_warning_pushed = true;
                }
                // In plain-enum mode, flag a wildcard that comes
                // after every variant is already covered.
                if let MatchMode::PlainEnum { enum_name, .. } = match_mode
                    && !*has_wildcard
                    && plain_covered.iter().all(|c| *c)
                {
                    self.output.errors.push(OrynError::compiler(
                        pattern.span.clone(),
                        format!(
                            "wildcard arm is unreachable: all variants of enum `{enum_name}` are already covered",
                        ),
                    ));
                }
                *has_wildcard = true;
                ResolvedArm::Wildcard
            }
            Pattern::Ok { name } => match match_mode {
                MatchMode::ErrorUnion { success_type, .. } => {
                    if *has_ok_arm {
                        self.output.errors.push(OrynError::compiler(
                            pattern.span.clone(),
                            "duplicate `ok` arm in match".to_string(),
                        ));
                    }
                    *has_ok_arm = true;
                    ResolvedArm::Ok {
                        name: name.clone(),
                        success_type: success_type.clone(),
                    }
                }
                MatchMode::PlainEnum { enum_name, .. } => {
                    self.output.errors.push(OrynError::compiler(
                        pattern.span.clone(),
                        format!(
                            "`ok` pattern is only valid when matching an error union; scrutinee has type `{enum_name}`"
                        ),
                    ));
                    ResolvedArm::Unresolvable
                }
            },
            Pattern::Variant {
                enum_name: pat_enum,
                variant_name,
                bindings,
            } => match match_mode {
                MatchMode::PlainEnum {
                    enum_name,
                    enum_def,
                } => {
                    if pat_enum != enum_name {
                        self.output.errors.push(OrynError::compiler(
                            pattern.span.clone(),
                            format!(
                                "pattern type `{pat_enum}` does not match scrutinee enum `{enum_name}`"
                            ),
                        ));
                        return ResolvedArm::Unresolvable;
                    }
                    let Some(variant_idx) = enum_def
                        .variants
                        .iter()
                        .position(|v| &v.name == variant_name)
                    else {
                        self.output.errors.push(OrynError::compiler(
                            pattern.span.clone(),
                            format!("no variant `{variant_name}` on enum `{enum_name}`"),
                        ));
                        return ResolvedArm::Unresolvable;
                    };
                    if plain_covered[variant_idx] {
                        self.output.errors.push(OrynError::compiler(
                            pattern.span.clone(),
                            format!(
                                "duplicate match arm: variant `{enum_name}.{variant_name}` already covered"
                            ),
                        ));
                    }
                    plain_covered[variant_idx] = true;
                    // For plain-enum matches, the (enum_def_idx)
                    // check is unused — codegen branches on
                    // match_mode and skips the def-idx step.
                    let enum_def_idx = self
                        .enum_table
                        .resolve(enum_name)
                        .map(|(idx, _)| idx)
                        .unwrap_or(0);
                    let variant = &enum_def.variants[variant_idx];
                    let resolved_bindings = self.resolve_variant_bindings(
                        pat_enum,
                        variant_name,
                        variant,
                        bindings.as_deref(),
                        &pattern.span,
                    );
                    ResolvedArm::Variant {
                        enum_def_idx,
                        variant_idx,
                        bindings: resolved_bindings,
                    }
                }
                MatchMode::ErrorUnion { precise, .. } => {
                    // Look the pattern's enum up in the local error
                    // enum table. Validate it exists and is an
                    // `error enum`.
                    let Some((enum_def_idx, def)) = self
                        .enum_table
                        .resolve(pat_enum)
                        .map(|(i, d)| (i, d.clone()))
                    else {
                        self.output.errors.push(OrynError::compiler(
                            pattern.span.clone(),
                            format!("unknown enum `{pat_enum}` in match arm"),
                        ));
                        return ResolvedArm::Unresolvable;
                    };
                    if !def.is_error {
                        self.output.errors.push(OrynError::compiler(
                            pattern.span.clone(),
                            format!(
                                "`{pat_enum}` is not an `error enum`; only error enums may appear in a match on an error union"
                            ),
                        ));
                        return ResolvedArm::Unresolvable;
                    }
                    // Precise-mode constraint: the pattern's enum
                    // must be the named error enum.
                    if let Some(precise_def) = precise
                        && pat_enum != &precise_def.name
                    {
                        self.output.errors.push(OrynError::compiler(
                            pattern.span.clone(),
                            format!(
                                "pattern enum `{pat_enum}` does not match scrutinee's precise error enum `{}`",
                                precise_def.name
                            ),
                        ));
                        return ResolvedArm::Unresolvable;
                    }
                    let Some(variant_idx) =
                        def.variants.iter().position(|v| &v.name == variant_name)
                    else {
                        self.output.errors.push(OrynError::compiler(
                            pattern.span.clone(),
                            format!("no variant `{variant_name}` on enum `{pat_enum}`"),
                        ));
                        return ResolvedArm::Unresolvable;
                    };
                    // Precise mode tracks coverage for exhaustiveness.
                    if precise.is_some() {
                        if precise_covered[variant_idx] {
                            self.output.errors.push(OrynError::compiler(
                                pattern.span.clone(),
                                format!(
                                    "duplicate match arm: variant `{pat_enum}.{variant_name}` already covered"
                                ),
                            ));
                        }
                        precise_covered[variant_idx] = true;
                    }
                    let variant = &def.variants[variant_idx];
                    let resolved_bindings = self.resolve_variant_bindings(
                        pat_enum,
                        variant_name,
                        variant,
                        bindings.as_deref(),
                        &pattern.span,
                    );
                    ResolvedArm::Variant {
                        enum_def_idx,
                        variant_idx,
                        bindings: resolved_bindings,
                    }
                }
            },
        }
    }

    /// Validate a payload-binding block against the variant's declared
    /// fields and return the resolved `(field_idx, name, ty)` triples
    /// ready for codegen. Extracted from the body of the original
    /// match function so both plain-enum and error-union modes can
    /// reuse it.
    fn resolve_variant_bindings(
        &mut self,
        enum_name: &str,
        variant_name: &str,
        variant: &super::types::EnumVariantInfo,
        bindings: Option<&[crate::parser::PatternBinding]>,
        pattern_span: &Span,
    ) -> Vec<ResolvedBinding> {
        let mut resolved: Vec<ResolvedBinding> = Vec::new();
        let Some(bs) = bindings else {
            return resolved;
        };
        if variant.field_names.is_empty() {
            self.output.errors.push(OrynError::compiler(
                pattern_span.clone(),
                format!(
                    "variant `{enum_name}.{variant_name}` is nullary; remove the `{{ }}` block from the pattern"
                ),
            ));
            return resolved;
        }
        if bs.is_empty() {
            self.output.errors.push(OrynError::compiler(
                pattern_span.clone(),
                format!(
                    "empty `{{ }}` block in pattern for `{enum_name}.{variant_name}`; drop the braces or list at least one field binding"
                ),
            ));
            return resolved;
        }
        let mut seen_local_names: Vec<&str> = Vec::new();
        for binding in bs {
            match variant.field_names.iter().position(|f| f == &binding.field) {
                Some(field_idx) => {
                    if seen_local_names.contains(&binding.name.as_str()) {
                        self.output.errors.push(OrynError::compiler(
                            binding.span.clone(),
                            format!(
                                "duplicate binding name `{}` in pattern for `{enum_name}.{variant_name}`",
                                binding.name
                            ),
                        ));
                    } else {
                        seen_local_names.push(&binding.name);
                    }
                    resolved.push(ResolvedBinding {
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
        resolved
    }
}

/// The mode of a `match` expression, determined by the scrutinee's
/// resolved type. Each mode has different rules for which patterns
/// are accepted, how codegen dispatches, and how exhaustiveness is
/// measured.
enum MatchMode {
    /// Scrutinee is a plain enum. Arms match variants of that
    /// specific enum (plus wildcard).
    PlainEnum {
        enum_name: String,
        enum_def: EnumDefInfo,
    },
    /// Scrutinee is an `error T` union (loose or precise). Arms
    /// include `ok v` for the success side plus variant patterns
    /// for error-enum variants. Precise mode names the allowed
    /// error enum and enables full exhaustiveness.
    ErrorUnion {
        /// The success type `T` — bound into the `ok v` arm's body.
        success_type: ResolvedType,
        /// Precise form's error enum, if any. `None` is the loose
        /// form and cannot be exhaustively matched.
        precise: Option<EnumDefInfo>,
    },
}

/// One resolved match arm, ready for codegen.
enum ResolvedArm {
    /// Pattern resolution failed (unknown variant, wrong enum, etc.).
    /// The body still compiles for diagnostic coverage but no
    /// dispatch is emitted.
    Unresolvable,
    /// Catch-all wildcard arm.
    Wildcard,
    /// `ok name` arm binding the success side of an error union.
    Ok {
        name: String,
        success_type: ResolvedType,
    },
    /// A variant pattern with resolved payload bindings.
    Variant {
        /// Absolute index of the variant's enum in the enum def table.
        /// Used by error-union dispatch to compare against `EnumDefIdx`.
        enum_def_idx: usize,
        variant_idx: usize,
        bindings: Vec<ResolvedBinding>,
    },
}

/// A resolved payload binding ready for codegen.
struct ResolvedBinding {
    field_idx: usize,
    name: String,
    ty: ResolvedType,
}
