use std::collections::HashMap;
use std::ops::Range;

use oryn::{Expression, Spanned, Statement, Token};

/// What kind of symbol a definition introduces.
#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Variable,
    Function,
    Parameter,
}

/// A definition site: where a name is introduced (let, fn, param).
#[derive(Debug)]
pub struct SymbolInfo {
    pub name: String,
    /// Byte span of just the name token.
    pub name_span: Range<usize>,
    /// Byte span of the entire enclosing statement.
    pub full_span: Range<usize>,
    pub kind: SymbolKind,
    /// For functions, the parameter names.
    pub params: Option<Vec<String>>,
    /// 0 = top-level, 1+ = nested in function/block.
    pub scope_depth: usize,
}

/// A reference site: where a name is used (ident, call, assignment target).
#[derive(Debug)]
pub struct SymbolRef {
    /// Kept for debugging and test assertions.
    #[allow(dead_code)]
    pub name: String,
    /// Byte span of just the name token.
    pub name_span: Range<usize>,
    /// Index into SymbolTable.definitions, if resolved.
    pub definition_idx: Option<usize>,
}

/// All definitions and references found in a source file.
#[derive(Debug)]
pub struct SymbolTable {
    pub definitions: Vec<SymbolInfo>,
    pub references: Vec<SymbolRef>,
}

/// Analyze source code and build a symbol table mapping definitions
/// to their references, with scope-aware resolution.
pub fn analyze(source: &str) -> SymbolTable {
    let (tokens, _) = oryn::lex(source);
    let (stmts, _) = oryn::parse(tokens.clone());

    analyze_from(&tokens, &stmts)
}

/// Build a symbol table from pre-lexed tokens and a parsed AST.
/// Used by the LSP to avoid double-lexing/parsing.
pub fn analyze_from(tokens: &[(Token, Range<usize>)], stmts: &[Spanned<Statement>]) -> SymbolTable {
    // Collect all ident tokens sorted by position for fast lookup.
    let idents: Vec<(&str, Range<usize>)> = tokens
        .iter()
        .filter_map(|(tok, span)| match tok {
            Token::Ident(name) => Some((name.as_str(), span.clone())),
            _ => None,
        })
        .collect();

    let mut table = SymbolTable {
        definitions: Vec::new(),
        references: Vec::new(),
    };
    // Scope stack: each entry maps names to definition indices.
    let mut scopes: Vec<HashMap<String, usize>> = vec![HashMap::new()];

    for stmt in stmts {
        walk_statement(&idents, &mut table, &mut scopes, stmt);
    }

    table
}

/// Find the first ident token matching `name` within `range`.
fn find_ident(
    idents: &[(&str, Range<usize>)],
    name: &str,
    range: &Range<usize>,
) -> Option<Range<usize>> {
    idents
        .iter()
        .find(|(n, span)| *n == name && span.start >= range.start && span.end <= range.end)
        .map(|(_, span)| span.clone())
}

/// Resolve a name reference against the scope stack (innermost first).
fn resolve(scopes: &[HashMap<String, usize>], name: &str) -> Option<usize> {
    for scope in scopes.iter().rev() {
        if let Some(&idx) = scope.get(name) {
            return Some(idx);
        }
    }
    None
}

fn walk_statement(
    idents: &[(&str, Range<usize>)],
    table: &mut SymbolTable,
    scopes: &mut Vec<HashMap<String, usize>>,
    stmt: &Spanned<Statement>,
) {
    match &stmt.node {
        Statement::Let { name, value, .. } | Statement::Val { name, value, .. } => {
            // The name token is the first ident matching `name` in the statement span.
            if let Some(name_span) = find_ident(idents, name, &stmt.span) {
                let idx = table.definitions.len();
                table.definitions.push(SymbolInfo {
                    name: name.clone(),
                    name_span,
                    full_span: stmt.span.clone(),
                    kind: SymbolKind::Variable,
                    params: None,
                    scope_depth: scopes.len() - 1,
                });
                // Register in current scope.
                if let Some(scope) = scopes.last_mut() {
                    scope.insert(name.clone(), idx);
                }
            }
            // Walk the value expression for references.
            walk_expression(idents, table, scopes, value);
        }
        Statement::Function {
            name, params, body, ..
        } => {
            // Register the function name as a definition.
            if let Some(name_span) = find_ident(idents, name, &stmt.span) {
                let idx = table.definitions.len();
                table.definitions.push(SymbolInfo {
                    name: name.clone(),
                    name_span,
                    full_span: stmt.span.clone(),
                    kind: SymbolKind::Function,
                    params: Some(params.clone().into_iter().map(|p| p.0).collect()),
                    scope_depth: scopes.len() - 1,
                });
                if let Some(scope) = scopes.last_mut() {
                    scope.insert(name.clone(), idx);
                }
            }

            // Push a new scope for the function body.
            scopes.push(HashMap::new());

            // Register parameters as definitions in the function scope.
            for (param_name, _type_ann) in params {
                if let Some(param_span) = find_ident(idents, param_name, &stmt.span) {
                    let idx = table.definitions.len();
                    table.definitions.push(SymbolInfo {
                        name: param_name.clone(),
                        name_span: param_span,
                        full_span: stmt.span.clone(),
                        kind: SymbolKind::Parameter,
                        params: None,
                        scope_depth: scopes.len() - 1,
                    });
                    if let Some(scope) = scopes.last_mut() {
                        scope.insert(param_name.clone(), idx);
                    }
                }
            }

            walk_expression(idents, table, scopes, body);

            scopes.pop();
        }
        Statement::Assignment { name, value } => {
            // The assignment target is a reference to an existing definition.
            if let Some(name_span) = find_ident(idents, name, &stmt.span) {
                table.references.push(SymbolRef {
                    name: name.clone(),
                    name_span,
                    definition_idx: resolve(scopes, name),
                });
            }
            walk_expression(idents, table, scopes, value);
        }
        Statement::If {
            condition,
            body,
            else_body,
        } => {
            walk_expression(idents, table, scopes, condition);
            walk_expression(idents, table, scopes, body);
            if let Some(else_body) = else_body {
                walk_expression(idents, table, scopes, else_body);
            }
        }
        Statement::While { condition, body } => {
            walk_expression(idents, table, scopes, condition);
            walk_expression(idents, table, scopes, body);
        }
        Statement::Return(Some(expr)) => {
            walk_expression(idents, table, scopes, expr);
        }
        Statement::Expression(expr) => {
            walk_expression(idents, table, scopes, expr);
        }
        Statement::ObjDef { .. } => {
            // TODO: register object type as a symbol
        }
        Statement::FieldAssignment { object, value, .. } => {
            walk_expression(idents, table, scopes, object);
            walk_expression(idents, table, scopes, value);
        }
        Statement::Break | Statement::Continue | Statement::Return(None) => {}
    }
}

fn walk_expression(
    idents: &[(&str, Range<usize>)],
    table: &mut SymbolTable,
    scopes: &mut Vec<HashMap<String, usize>>,
    expr: &Spanned<Expression>,
) {
    match &expr.node {
        Expression::Ident(name) => {
            if let Some(name_span) = find_ident(idents, name, &expr.span) {
                table.references.push(SymbolRef {
                    name: name.clone(),
                    name_span,
                    definition_idx: resolve(scopes, name),
                });
            }
        }
        Expression::Call { name, args } => {
            // The call name is a reference.
            if let Some(name_span) = find_ident(idents, name, &expr.span) {
                table.references.push(SymbolRef {
                    name: name.clone(),
                    name_span,
                    definition_idx: resolve(scopes, name),
                });
            }
            for arg in args {
                walk_expression(idents, table, scopes, arg);
            }
        }
        Expression::BinaryOp { left, right, .. } => {
            walk_expression(idents, table, scopes, left);
            walk_expression(idents, table, scopes, right);
        }
        Expression::UnaryOp { expr: operand, .. } => {
            walk_expression(idents, table, scopes, operand);
        }
        Expression::Block(stmts) => {
            scopes.push(HashMap::new());
            for stmt in stmts {
                walk_statement(idents, table, scopes, stmt);
            }
            scopes.pop();
        }
        Expression::ObjLiteral { fields, .. } => {
            for (_, value) in fields {
                walk_expression(idents, table, scopes, value);
            }
        }
        Expression::FieldAccess { object, .. } => {
            walk_expression(idents, table, scopes, object);
        }
        // Literals have no names to resolve.
        Expression::True
        | Expression::False
        | Expression::Float(_)
        | Expression::Int(_)
        | Expression::String(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn let_binding_creates_definition() {
        let table = analyze("let x = 5");

        assert_eq!(table.definitions.len(), 1);
        assert_eq!(table.definitions[0].name, "x");
        assert_eq!(table.definitions[0].kind, SymbolKind::Variable);
    }

    #[test]
    fn function_creates_definition_and_params() {
        let table = analyze("fn add(a, b) {\nrn a + b\n}");

        // 3 definitions: add (Function), a (Parameter), b (Parameter)
        assert_eq!(table.definitions.len(), 3);
        assert_eq!(table.definitions[0].name, "add");
        assert_eq!(table.definitions[0].kind, SymbolKind::Function);
        assert_eq!(table.definitions[1].name, "a");
        assert_eq!(table.definitions[1].kind, SymbolKind::Parameter);
        assert_eq!(table.definitions[2].name, "b");
        assert_eq!(table.definitions[2].kind, SymbolKind::Parameter);

        // 2 references: a and b in the body
        assert_eq!(table.references.len(), 2);
        assert_eq!(table.references[0].definition_idx, Some(1)); // a -> param a
        assert_eq!(table.references[1].definition_idx, Some(2)); // b -> param b
    }

    #[test]
    fn variable_reference_resolves() {
        let table = analyze("let x = 1\nlet y = x + 2");

        assert_eq!(table.definitions.len(), 2);
        assert_eq!(table.references.len(), 1);
        assert_eq!(table.references[0].name, "x");
        assert_eq!(table.references[0].definition_idx, Some(0));
    }

    #[test]
    fn shadowing_resolves_to_inner() {
        let table = analyze("let x = 1\nfn foo() {\nlet x = 2\nprint(x)\n}");

        // Definitions: outer x (0), foo (1), inner x (2)
        // The reference to x in print(x) should resolve to inner x (2)
        let x_refs: Vec<&SymbolRef> = table.references.iter().filter(|r| r.name == "x").collect();
        assert_eq!(x_refs.len(), 1);
        assert_eq!(x_refs[0].definition_idx, Some(2));
    }

    #[test]
    fn call_reference_resolves_to_function() {
        let table = analyze("fn greet() {\nprint(1)\n}\ngreet()");

        let greet_refs: Vec<&SymbolRef> = table
            .references
            .iter()
            .filter(|r| r.name == "greet")
            .collect();
        assert_eq!(greet_refs.len(), 1);
        assert_eq!(greet_refs[0].definition_idx, Some(0));
    }
}
