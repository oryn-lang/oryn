use crate::compiler::types::ResolvedType;
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

        for stmt in stmts {
            self.compile_stmt(stmt);
        }

        if let Some(saved) = saved_loops {
            self.loops = saved;
        }

        ResolvedType::Unknown
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
}
