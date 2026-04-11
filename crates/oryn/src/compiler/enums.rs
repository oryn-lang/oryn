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
    pub(super) fn compile_enum_def(
        &mut self,
        name: String,
        variants: Vec<EnumVariant>,
        stmt_span: &Span,
        is_pub: bool,
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
        let abs_idx = self
            .enum_table
            .register(name.clone(), variant_infos.clone(), is_pub);

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

        ResolvedType::Enum {
            name: enum_name.to_string(),
            module: self.current_module_path.clone(),
        }
    }

    /// Compile a `match` expression. Slice 1+2 supports only
    /// discriminant-only patterns (variant paths and `_`); payload
    /// destructuring lands in Slice 3.
    ///
    /// **Codegen shape** (per arm, with `Dup` for the discriminant):
    /// ```text
    ///   <compile scrutinee>      ; scrutinee on stack
    ///   EnumDiscriminant         ; replace with variant_idx (Int)
    /// arm_n:
    ///   Dup                      ; clone the discriminant for the test
    ///   PushInt(arm_variant_idx)
    ///   Equal                    ; pops both, pushes Bool
    ///   JumpIfFalse → arm_n+1    ; pops Bool; falls through if true
    ///   Pop                      ; matched — drop the spare discriminant
    ///   <compile arm body>       ; body's value lands on TOS
    ///   Jump → end
    /// arm_n+1:
    ///   ...
    /// end:
    /// ```
    ///
    /// **Wildcards** skip the Dup/PushInt/Equal/JumpIfFalse
    /// dance — they always match, so we just `Pop` the
    /// discriminant and run the body.
    ///
    /// **Fall-through safety net**: after the final arm, any
    /// path that reaches the end without matching still has the
    /// original discriminant on the stack. We drop it and push
    /// `Nil` so the match expression has a uniform "one value on
    /// TOS" shape. This branch is unreachable for exhaustive
    /// (or wildcard-terminated) matches but keeps stack
    /// discipline coherent for non-exhaustive ones (which also
    /// produce a compile error, so the runtime path is
    /// best-effort).
    ///
    /// **Exhaustiveness**: a set-difference check (declared
    /// variants − arm-covered variants) runs after codegen. If
    /// any variant is uncovered AND no `_` arm is present, an
    /// error is pushed but codegen still completes so downstream
    /// type checking stays coherent.
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

        // 2. Replace the scrutinee with its discriminant. From
        //    here on the discriminant (an Int) sits on TOS and is
        //    consumed/restored via Dup as each arm tests it.
        self.emit(Instruction::EnumDiscriminant, span);

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

            // Resolve pattern → variant_idx (None means wildcard
            // or unresolvable). Variants are checked against the
            // scrutinee's enum, and coverage is tracked.
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
                    has_wildcard = true;
                    None
                }
                Pattern::Variant {
                    enum_name: pat_enum,
                    variant_name,
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
            // Stack invariant entering an arm: `[..., discriminant]`.
            // Variant arms: Dup → PushInt → Equal → JumpIfFalse;
            //               on the matched fall-through, Pop the
            //               spare discriminant before running the
            //               body. The body leaves its result on
            //               TOS, then Jump → end.
            // Wildcard arms: skip the test, just Pop the
            //                discriminant and run the body.
            let skip_arm_jump_idx: Option<usize> = if let Some(idx) = variant_idx {
                self.emit(Instruction::Dup, &arm_span);
                self.emit(Instruction::PushInt(idx as i32), &arm_span);
                self.emit(Instruction::Equal, &arm_span);
                let jump_idx = self.output.instructions.len();
                self.emit(Instruction::JumpIfFalse(0), &arm_span); // patched below
                self.emit(Instruction::Pop, &arm_span);
                Some(jump_idx)
            } else {
                self.emit(Instruction::Pop, &arm_span);
                None
            };

            // Compile the arm body and reconcile its type.
            let body_span = body.span.clone();
            let body_type = self.compile_expr(body);
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

        // Fall-through safety net (see doc comment above).
        self.emit(Instruction::Pop, span);
        self.emit(Instruction::PushNil, span);

        // Patch all end-jumps to land here, AFTER the cleanup.
        let end_addr = self.output.instructions.len();
        for jump_idx in end_jumps {
            self.output.instructions[jump_idx] = Instruction::Jump(end_addr);
        }

        // Exhaustiveness check.
        if !has_wildcard {
            let missing: Vec<&str> = enum_def
                .variants
                .iter()
                .zip(covered.iter())
                .filter(|(_, c)| !**c)
                .map(|(v, _)| v.name.as_str())
                .collect();
            if !missing.is_empty() {
                self.output.errors.push(OrynError::compiler(
                    span.clone(),
                    format!(
                        "non-exhaustive match: missing variants `{}` of enum `{enum_name}` (add `_ => ...` to catch the rest)",
                        missing.join("`, `"),
                    ),
                ));
            }
        }

        result_type.unwrap_or(ResolvedType::Unknown)
    }
}
