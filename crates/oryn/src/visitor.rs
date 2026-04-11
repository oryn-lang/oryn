//! Shared AST visitor trait and default walk functions.
//!
//! The walk functions encode the canonical traversal order for every
//! Statement and Expression variant. Implementors override `visit_stmt`
//! and `visit_expr` to attach side effects (symbol collection, bytecode
//! emission, etc.), then call the corresponding `walk_*` function to
//! recurse into children.

use crate::parser::{Expression, ObjMethod, Pattern, Span, Spanned, Statement, StringPart};

/// Trait for walking the Oryn AST with custom side effects at each node.
///
/// Default implementations call the matching `walk_*` function, so an
/// empty impl walks the entire tree with no side effects. Override
/// `visit_stmt` or `visit_expr` to intercept nodes, then call
/// `walk_stmt`/`walk_expr` to continue the default traversal.
pub trait AstVisitor {
    /// Called for each statement. Override to inspect or record, then
    /// call `walk_stmt(self, stmt)` to recurse into children.
    fn visit_stmt(&mut self, stmt: &Spanned<Statement>) {
        walk_stmt(self, stmt);
    }

    /// Called for each expression. Override to inspect or record, then
    /// call `walk_expr(self, expr)` to recurse into children.
    fn visit_expr(&mut self, expr: &Spanned<Expression>) {
        walk_expr(self, expr);
    }

    /// Called when entering a new scope (function body, block, method body).
    fn enter_scope(&mut self) {}

    /// Called when leaving a scope.
    fn exit_scope(&mut self) {}

    /// Called when a name is introduced (let, val, fn, param, obj).
    /// `name_span` is the span of just the name token.
    /// `stmt_span` is the span of the enclosing statement.
    fn on_define(&mut self, _name: &str, _name_span: &Span, _stmt_span: &Span) {}

    /// Called when a name is referenced (variable use, function call, assignment target, `use` type).
    fn on_reference(&mut self, _name: &str, _span: &Span) {}
}

/// Walk a slice of statements, visiting each in order.
pub fn walk_stmts<V: AstVisitor + ?Sized>(visitor: &mut V, stmts: &[Spanned<Statement>]) {
    for stmt in stmts {
        visitor.visit_stmt(stmt);
    }
}

/// Default walk for a single statement. Dispatches to children based on variant.
pub fn walk_stmt<V: AstVisitor + ?Sized>(visitor: &mut V, stmt: &Spanned<Statement>) {
    match &stmt.node {
        Statement::Let { name, value, .. } | Statement::Val { name, value, .. } => {
            visitor.on_define(name, &stmt.span, &stmt.span);
            visitor.visit_expr(value);
        }

        Statement::Assignment { name, value } => {
            visitor.on_reference(name, &stmt.span);
            visitor.visit_expr(value);
        }

        Statement::FieldAssignment { object, value, .. } => {
            visitor.visit_expr(object);
            visitor.visit_expr(value);
        }

        Statement::IndexAssignment {
            object,
            index,
            value,
        } => {
            visitor.visit_expr(object);
            visitor.visit_expr(index);
            visitor.visit_expr(value);
        }

        Statement::Function {
            name, params, body, ..
        } => {
            visitor.on_define(name, &stmt.span, &stmt.span);

            visitor.enter_scope();
            for param in params {
                visitor.on_define(&param.name, &stmt.span, &stmt.span);
            }
            visitor.visit_expr(body);
            visitor.exit_scope();
        }

        Statement::Return(Some(expr)) => {
            visitor.visit_expr(expr);
        }
        Statement::Return(None) => {}

        Statement::ObjDef {
            name,
            methods,
            uses,
            ..
        } => {
            visitor.on_define(name, &stmt.span, &stmt.span);

            for used_type in uses {
                for segment in used_type {
                    visitor.on_reference(segment, &stmt.span);
                }
            }

            for method in methods {
                walk_obj_method(visitor, method, &stmt.span);
            }
        }

        Statement::EnumDef { name, variants, .. } => {
            visitor.on_define(name, &stmt.span, &stmt.span);
            // Each variant introduces a name in the enum's namespace,
            // accessed as `EnumName.VariantName`. The visitor's
            // current model doesn't have a "qualified name" concept,
            // so we just record the bare variant name as a definition
            // — good enough for hover/completion to find them.
            for variant in variants {
                visitor.on_define(&variant.name, &variant.span, &variant.span);
            }
        }

        Statement::While { condition, body } => {
            visitor.visit_expr(condition);
            visitor.visit_expr(body);
        }

        Statement::For {
            name,
            iterable,
            body,
        } => {
            visitor.visit_expr(iterable);
            visitor.enter_scope();
            visitor.on_define(name, &stmt.span, &stmt.span);
            visitor.visit_expr(body);
            visitor.exit_scope();
        }

        Statement::Break | Statement::Continue => {}

        Statement::Expression(expr) => {
            visitor.visit_expr(expr);
        }

        Statement::Import { .. } => {}

        Statement::Test { body, .. } => {
            visitor.enter_scope();
            visitor.visit_expr(body);
            visitor.exit_scope();
        }

        Statement::Assert { condition } => {
            visitor.visit_expr(condition);
        }
    }
}

/// Walk an object method declaration. Registers the method name as a
/// definition, enters a scope for parameters, and walks the body if present.
fn walk_obj_method<V: AstVisitor + ?Sized>(visitor: &mut V, method: &ObjMethod, obj_span: &Span) {
    visitor.on_define(&method.name, obj_span, obj_span);

    if let Some(body) = &method.body {
        visitor.enter_scope();
        for param in &method.params {
            visitor.on_define(&param.name, obj_span, obj_span);
        }
        visitor.visit_expr(body);
        visitor.exit_scope();
    }
}

/// Default walk for a single expression. Dispatches to children based on variant.
pub fn walk_expr<V: AstVisitor + ?Sized>(visitor: &mut V, expr: &Spanned<Expression>) {
    match &expr.node {
        Expression::Nil
        | Expression::True
        | Expression::False
        | Expression::Float(_)
        | Expression::Int(_)
        | Expression::String(_) => {}

        Expression::StringInterp(parts) => {
            for part in parts {
                if let StringPart::Interp(expr) = part {
                    visitor.visit_expr(expr);
                }
            }
        }

        Expression::Ident(name) => {
            visitor.on_reference(name, &expr.span);
        }

        Expression::ObjLiteral { fields, .. } => {
            for (_, value) in fields {
                visitor.visit_expr(value);
            }
        }

        Expression::FieldAccess { object, .. } => {
            visitor.visit_expr(object);
        }

        Expression::MethodCall { object, args, .. } => {
            visitor.visit_expr(object);
            for arg in args {
                visitor.visit_expr(arg);
            }
        }

        Expression::BinaryOp { left, right, .. } => {
            visitor.visit_expr(left);
            visitor.visit_expr(right);
        }

        Expression::Range { start, end, .. } => {
            visitor.visit_expr(start);
            visitor.visit_expr(end);
        }

        Expression::UnaryOp { expr: operand, .. } => {
            visitor.visit_expr(operand);
        }

        Expression::Call { name, args } => {
            visitor.on_reference(name, &expr.span);
            for arg in args {
                visitor.visit_expr(arg);
            }
        }

        Expression::Try(inner) | Expression::UnwrapError(inner) => {
            visitor.visit_expr(inner);
        }

        Expression::Coalesce { left, right } => {
            visitor.visit_expr(left);
            visitor.visit_expr(right);
        }

        Expression::ListLiteral(elements) => {
            for element in elements {
                visitor.visit_expr(element);
            }
        }

        Expression::MapLiteral(entries) => {
            for (key, value) in entries {
                visitor.visit_expr(key);
                visitor.visit_expr(value);
            }
        }

        Expression::Index { object, index } => {
            visitor.visit_expr(object);
            visitor.visit_expr(index);
        }

        Expression::Block(stmts) => {
            visitor.enter_scope();
            walk_stmts(visitor, stmts);
            visitor.exit_scope();
        }

        Expression::Match { scrutinee, arms } => {
            visitor.visit_expr(scrutinee);
            for arm in arms {
                // Patterns may reference enum and variant names; record
                // those as references so the LSP can hover/jump to the
                // enum definition. Wildcard patterns have nothing to
                // walk.
                if let Pattern::Variant {
                    enum_name,
                    variant_name,
                    bindings: _,
                } = &arm.pattern.node
                {
                    visitor.on_reference(enum_name, &arm.pattern.span);
                    visitor.on_reference(variant_name, &arm.pattern.span);
                }
                visitor.visit_expr(&arm.body);
            }
        }

        Expression::If {
            condition,
            body,
            else_body,
        } => {
            visitor.visit_expr(condition);
            visitor.visit_expr(body);
            if let Some(else_body) = else_body {
                visitor.visit_expr(else_body);
            }
        }

        Expression::IfLet {
            value,
            name,
            body,
            else_body,
        } => {
            visitor.visit_expr(value);
            visitor.enter_scope();
            visitor.on_define(name, &expr.span, &expr.span);
            visitor.visit_expr(body);
            visitor.exit_scope();
            if let Some(else_body) = else_body {
                visitor.visit_expr(else_body);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lex, parse};

    /// Test visitor that records events for assertions.
    struct TestVisitor {
        events: Vec<String>,
    }

    impl TestVisitor {
        fn new() -> Self {
            Self { events: Vec::new() }
        }
    }

    impl AstVisitor for TestVisitor {
        fn visit_stmt(&mut self, stmt: &Spanned<Statement>) {
            let kind = match &stmt.node {
                Statement::Let { .. } => "let",
                Statement::Val { .. } => "val",
                Statement::Function { .. } => "fn",
                Statement::Assignment { .. } => "assign",
                Statement::ObjDef { .. } => "obj",
                Statement::EnumDef { .. } => "enum",
                Statement::While { .. } => "while",
                Statement::For { .. } => "for",
                Statement::Return(_) => "return",
                Statement::Break => "break",
                Statement::Continue => "continue",
                Statement::Expression(_) => "expr_stmt",
                Statement::FieldAssignment { .. } => "field_assign",
                Statement::IndexAssignment { .. } => "index_assign",
                Statement::Import { .. } => "import",
                Statement::Test { .. } => "test",
                Statement::Assert { .. } => "assert",
            };
            self.events.push(format!("stmt:{kind}"));
            walk_stmt(self, stmt);
        }

        fn visit_expr(&mut self, expr: &Spanned<Expression>) {
            let kind = match &expr.node {
                Expression::Int(_) => "int",
                Expression::Ident(_) => "ident",
                Expression::Call { .. } => "call",
                Expression::Block(_) => "block",
                Expression::BinaryOp { .. } => "binop",
                _ => "other",
            };
            self.events.push(format!("expr:{kind}"));
            walk_expr(self, expr);
        }

        fn enter_scope(&mut self) {
            self.events.push("scope:enter".to_string());
        }

        fn exit_scope(&mut self) {
            self.events.push("scope:exit".to_string());
        }

        fn on_define(&mut self, name: &str, _span: &Span, _stmt_span: &Span) {
            self.events.push(format!("define:{name}"));
        }

        fn on_reference(&mut self, name: &str, _span: &Span) {
            self.events.push(format!("ref:{name}"));
        }
    }

    fn visit(source: &str) -> Vec<String> {
        let (tokens, _) = lex(source);
        let (stmts, _) = parse(tokens);
        let mut visitor = TestVisitor::new();
        walk_stmts(&mut visitor, &stmts);
        visitor.events
    }

    #[test]
    fn let_binding_visits_define_and_value() {
        let events = visit("let x = 5");
        assert!(events.contains(&"stmt:let".to_string()));
        assert!(events.contains(&"define:x".to_string()));
        assert!(events.contains(&"expr:int".to_string()));
    }

    #[test]
    fn function_enters_scope_for_body() {
        let events = visit(
            "fn foo(a) {
return a\n}",
        );
        assert!(events.contains(&"define:foo".to_string()));
        assert!(events.contains(&"scope:enter".to_string()));
        assert!(events.contains(&"define:a".to_string()));
        assert!(events.contains(&"scope:exit".to_string()));
    }

    #[test]
    fn ident_reference_fires() {
        let events = visit("let x = 1\nlet y = x");
        let refs: Vec<_> = events.iter().filter(|e| e.starts_with("ref:")).collect();
        assert!(refs.contains(&&"ref:x".to_string()));
    }

    #[test]
    fn block_scoping() {
        let events = visit("fn foo() {\nlet x = 1\n}");
        let scope_enters = events.iter().filter(|e| *e == "scope:enter").count();
        let scope_exits = events.iter().filter(|e| *e == "scope:exit").count();
        assert_eq!(scope_enters, scope_exits);
        assert!(scope_enters >= 1);
    }

    #[test]
    fn obj_def_visits_name_and_methods() {
        let events = visit("struct Foo {\nfn bar(self) {\nprint(1)\n}\n}");
        assert!(events.contains(&"define:Foo".to_string()));
        assert!(events.contains(&"define:bar".to_string()));
        assert!(events.contains(&"define:self".to_string()));
    }

    #[test]
    fn obj_use_fires_reference() {
        let events = visit(
            "struct Base {\nfn hello(self) {\nprint(1)\n}\n}
struct Child {\nuse Base\n}",
        );
        assert!(events.contains(&"ref:Base".to_string()));
    }

    #[test]
    fn call_fires_reference() {
        let events = visit("fn foo() {\nprint(1)\n}\nfoo()");
        let foo_refs: Vec<_> = events.iter().filter(|e| *e == "ref:foo").collect();
        assert_eq!(foo_refs.len(), 1);
    }

    #[test]
    fn assignment_fires_reference() {
        let events = visit("let x = 1\nx = 2");
        assert!(events.contains(&"ref:x".to_string()));
    }
}
