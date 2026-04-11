use crate::compiler::types::ResolvedType;
use crate::parser::{Expression, Param, Span, Spanned};

use super::compile::Compiler;
use super::tables::{BindingKind, Locals};
use super::types::{CompiledFunction, Instruction};

// ---------------------------------------------------------------------------
// Function body config
// ---------------------------------------------------------------------------

/// Callback that determines (BindingKind, obj_type) for each parameter.
/// The kind tells the locals table whether the param is `Param` (default,
/// immutable), `MutParam` (declared `mut`), or `SelfRef` (the `self`
/// parameter on a `mut fn` method). Top-level functions never produce
/// `SelfRef`. The full `&Param` is passed so the callback can consult
/// the `is_mut` flag in addition to the name.
pub(super) type ParamLocalFn = dyn Fn(&Param) -> (BindingKind, ResolvedType);

/// Configuration for compiling a function or method body.
pub(super) struct FunctionBodyConfig<'a> {
    pub name: &'a str,
    pub params: &'a [Param],
    pub param_types: Vec<ResolvedType>,
    pub param_local_fn: &'a ParamLocalFn,
    /// If Some, registers the function under this name for recursion.
    pub self_name: Option<&'a str>,
    pub body: Spanned<Expression>,
    pub return_type: Option<ResolvedType>,
    pub span: &'a Span,
    pub is_pub: bool,
    /// `true` for `mut fn` methods, `false` for plain `fn` methods
    /// and all top-level functions. Stored on the resulting
    /// `CompiledFunction` so callers can enforce val-receiver and
    /// non-mut-context rules.
    pub is_mut: bool,
    /// If Some, write the compiled function into the existing slot at
    /// this LOCAL index (position within `output.functions`) instead of
    /// pushing a new slot. The caller must have reserved the slot before
    /// calling. Used by `compile_obj_def` to allocate method slots in a
    /// signature pre-pass so methods can call each other regardless of
    /// declaration order.
    pub pre_allocated_local_idx: Option<usize>,
}

// ---------------------------------------------------------------------------
// Function body compilation
// ---------------------------------------------------------------------------

impl Compiler {
    /// Shared compilation logic for functions and methods. Uses save/restore
    /// to isolate the function's locals, loops, and bytecode from the parent.
    ///
    /// Returns the **absolute** function index (within the merged chunk)
    /// so callers can emit `Call(absolute_idx, arity)` directly.
    pub(super) fn compile_function_body(&mut self, config: FunctionBodyConfig<'_>) -> usize {
        let FunctionBodyConfig {
            name,
            params,
            param_types,
            param_local_fn,
            self_name,
            body,
            span,
            return_type,
            is_pub,
            is_mut,
            pre_allocated_local_idx,
        } = config;

        let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        // Per-parameter mut flags. For methods, the `self` slot's
        // entry is `is_mut` (matching the method's `mut fn` keyword);
        // for non-self params and top-level functions, it's the
        // parameter's own `mut` annotation.
        let param_is_mut: Vec<bool> = params
            .iter()
            .map(|p| if p.name == "self" { is_mut } else { p.is_mut })
            .collect();
        let local_idx = match pre_allocated_local_idx {
            Some(idx) => idx,
            None => {
                let idx = self.output.functions.len();
                // Push a placeholder so the local position is valid.
                self.output.functions.push(CompiledFunction {
                    name: name.to_string(),
                    arity: params.len(),
                    params: param_names.clone(),
                    param_types: param_types.clone(),
                    param_is_mut: param_is_mut.clone(),
                    return_type: return_type.clone(),
                    num_locals: 0,
                    instructions: Vec::new(),
                    spans: Vec::new(),
                    is_pub,
                    is_mut,
                });
                idx
            }
        };
        let absolute_idx = self.fn_base_offset + local_idx;

        // Save parent state.
        let parent_locals = std::mem::replace(&mut self.locals, Locals::new());
        let parent_loops = std::mem::take(&mut self.loops);
        let parent_instructions = std::mem::take(&mut self.output.instructions);
        let parent_spans = std::mem::take(&mut self.output.spans);
        let parent_fn_table = self.fn_table.clone();
        let parent_fn_is_mut = self.current_fn_is_mut;
        self.current_fn_is_mut = is_mut;

        // Set up function-scoped locals.
        self.locals.return_type = return_type.clone();
        for param in params {
            let (kind, obj_type) = param_local_fn(param);
            self.locals.define(param.name.clone(), kind, obj_type);
        }

        // Pop params from the stack into locals in reverse order.
        for pname in param_names.iter().rev() {
            let entry = self.locals.resolve(pname.as_str()).unwrap();
            self.emit(Instruction::SetLocal(entry.slot), span);
        }

        // Register the function for recursion if needed. The fn_table
        // stores the absolute index (register() shifts by base_offset).
        if let Some(self_name) = self_name {
            self.fn_table.register(self_name.to_string(), local_idx);
        }

        // Compile the body.
        self.compile_body_expr(body);

        // Default return.
        self.emit(Instruction::PushInt(0), span);
        self.emit(Instruction::Return, span);

        // Harvest the function's bytecode.
        let func_instructions =
            std::mem::replace(&mut self.output.instructions, parent_instructions);
        let func_spans = std::mem::replace(&mut self.output.spans, parent_spans);
        let func_num_locals = self.locals.max_count;

        // Restore parent state.
        self.locals = parent_locals;
        self.loops = parent_loops;
        self.fn_table = parent_fn_table;
        self.current_fn_is_mut = parent_fn_is_mut;

        // Write the compiled function.
        self.output.functions[local_idx] = CompiledFunction {
            name: name.to_string(),
            arity: params.len(),
            params: param_names,
            param_types,
            param_is_mut,
            return_type,
            num_locals: func_num_locals,
            instructions: func_instructions,
            spans: func_spans,
            is_pub,
            is_mut,
        };

        absolute_idx
    }
}
