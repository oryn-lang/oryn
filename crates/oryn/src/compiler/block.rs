use crate::compiler::types::{Instruction, ResolvedType};
use crate::parser::{Expression, Spanned, Statement};

use super::compile::Compiler;

// ---------------------------------------------------------------------------
// Block mode
// ---------------------------------------------------------------------------

/// Controls whether a block inherits the enclosing loop context
/// (if/while bodies) or runs with a fresh, empty one (bare expression blocks).
pub(super) enum BlockMode {
    InheritLoops,
    FreshLoops,
}

// ---------------------------------------------------------------------------
// Block compilation
// ---------------------------------------------------------------------------

impl Compiler {
    pub(super) fn with_scope<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved_locals = self.locals.snapshot();
        let result = f(self);

        self.locals.restore(saved_locals);

        result
    }

    /// Compile a block of statements with explicit loop-context control.
    pub(super) fn compile_block(
        &mut self,
        stmts: Vec<Spanned<Statement>>,
        mode: BlockMode,
    ) -> ResolvedType {
        let saved_loops = match mode {
            BlockMode::FreshLoops => Some(std::mem::take(&mut self.loops)),
            BlockMode::InheritLoops => None,
        };

        let result = self.with_scope(|this| {
            for stmt in stmts {
                this.compile_stmt(stmt);
            }

            ResolvedType::Unknown
        });

        if let Some(saved) = saved_loops {
            self.loops = saved;
        }

        result
    }

    /// Compile a body expression (the body of if/while/function).
    /// If the expression is a Block, loops are inherited from the enclosing
    /// context. Otherwise, compile as a normal expression.
    pub(super) fn compile_body_expr(&mut self, expr: Spanned<Expression>) -> ResolvedType {
        let span = expr.span.clone();
        match expr.node {
            Expression::Block(stmts) => self.compile_block(stmts, BlockMode::InheritLoops),
            other => self.compile_expr(Spanned { node: other, span }),
        }
    }

    /// Compile a body expression in a context that EXPECTS a value
    /// on the stack (the body of an `if`-as-expression, for
    /// example). For non-block bodies this is the same as
    /// [`Self::compile_expr`] — the value naturally lands on the
    /// stack. For block bodies (`{ stmt; stmt; expr }`), the
    /// last statement's value is preserved instead of being
    /// popped: if it's a `Statement::Expression`, the expression
    /// is compiled directly (no trailing Pop) and its type is
    /// returned; if the block ends in any other statement, the
    /// block is compiled normally and a `nil` is pushed at the
    /// end so the stack invariant holds.
    pub(super) fn compile_value_body(&mut self, expr: Spanned<Expression>) -> ResolvedType {
        let span = expr.span.clone();
        match expr.node {
            Expression::Block(stmts) => self.compile_value_block(stmts),
            other => self.compile_expr(Spanned { node: other, span }),
        }
    }

    /// Compile a sequence of statements as the body of an
    /// expression context. Same as [`Self::compile_block`] but
    /// keeps the last expression-statement's value on the stack
    /// instead of popping it. See [`Self::compile_value_body`]
    /// for callers.
    pub(super) fn compile_value_block(&mut self, stmts: Vec<Spanned<Statement>>) -> ResolvedType {
        if stmts.is_empty() {
            // An empty block has no expression to evaluate. Push
            // nil so the surrounding expression sees a uniform
            // "one value on TOS" shape.
            self.emit(Instruction::PushNil, &(0..0));
            return ResolvedType::Nil;
        }

        self.with_scope(|this| {
            let last_idx = stmts.len() - 1;
            let mut result_ty = ResolvedType::Nil;

            for (i, stmt) in stmts.into_iter().enumerate() {
                if i == last_idx {
                    // Last statement: if it's an expression
                    // statement, compile its expression directly
                    // (NO trailing Pop) so the value flows out.
                    // Otherwise compile normally and push nil at
                    // the end so the stack invariant holds.
                    if let Statement::Expression(inner) = stmt.node {
                        result_ty = this.compile_expr(inner);
                    } else {
                        let stmt_span = stmt.span.clone();
                        this.compile_stmt(Spanned {
                            node: stmt.node,
                            span: stmt_span.clone(),
                        });
                        this.emit(Instruction::PushNil, &stmt_span);
                    }
                } else {
                    this.compile_stmt(stmt);
                }
            }

            result_ty
        })
    }
}
