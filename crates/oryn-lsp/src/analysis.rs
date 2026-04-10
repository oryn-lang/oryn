use std::collections::HashMap;
use std::ops::Range;

use oryn::{
    AstVisitor, Expression, ObjField, ObjMethod, Spanned, Statement, Token, TypeAnnotation,
    walk_expr, walk_stmt,
};

/// Format a `TypeAnnotation` as a human-readable string (e.g. `"int"`,
/// `"Vec2?"`, `"!String"`).
fn format_type_annotation(ann: &TypeAnnotation) -> String {
    match ann {
        TypeAnnotation::Named(p) => p.join("."),
        TypeAnnotation::Nillable(inner) => format!("{}?", format_type_annotation(inner)),
        TypeAnnotation::ErrorUnion(inner) => format!("!{}", format_type_annotation(inner)),
        TypeAnnotation::List(inner) => format!("[{}]", format_type_annotation(inner)),
    }
}

/// What kind of symbol a definition introduces.
#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Variable,
    Function,
    Parameter,
    Object,
    /// A field declared inside an `obj` body.
    Field,
    Module,
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
    /// For functions, the parameter names with types (e.g. "a: int").
    pub params: Option<Vec<String>>,
    /// The type annotation as a string (e.g. "int", "Vec2").
    pub type_name: Option<String>,
    /// For functions, the return type annotation.
    pub return_type: Option<String>,
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
    let idents: Vec<(&str, Range<usize>)> = tokens
        .iter()
        .filter_map(|(tok, span)| match tok {
            Token::Ident(name) => Some((name.as_str(), span.clone())),
            _ => None,
        })
        .collect();

    let mut visitor = LspVisitor {
        idents: &idents,
        table: SymbolTable {
            definitions: Vec::new(),
            references: Vec::new(),
        },
        scopes: vec![HashMap::new()],
    };

    oryn::walk_stmts(&mut visitor, stmts);

    visitor.table
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

// ---------------------------------------------------------------------------
// Visitor implementation
// ---------------------------------------------------------------------------

struct LspVisitor<'a> {
    idents: &'a [(&'a str, Range<usize>)],
    table: SymbolTable,
    scopes: Vec<HashMap<String, usize>>,
}

impl LspVisitor<'_> {
    fn scope_depth(&self) -> usize {
        self.scopes.len() - 1
    }

    fn register_definition(
        &mut self,
        name: &str,
        stmt_span: &Range<usize>,
        kind: SymbolKind,
        params: Option<Vec<String>>,
        type_name: Option<String>,
        return_type: Option<String>,
    ) {
        if let Some(name_span) = find_ident(self.idents, name, stmt_span) {
            let idx = self.table.definitions.len();
            self.table.definitions.push(SymbolInfo {
                name: name.to_string(),
                name_span,
                full_span: stmt_span.clone(),
                kind,
                params,
                type_name,
                return_type,
                scope_depth: self.scope_depth(),
            });
            if let Some(scope) = self.scopes.last_mut() {
                scope.insert(name.to_string(), idx);
            }
        }
    }

    fn register_reference(&mut self, name: &str, span: &Range<usize>) {
        if let Some(name_span) = find_ident(self.idents, name, span) {
            self.table.references.push(SymbolRef {
                name: name.to_string(),
                name_span,
                definition_idx: resolve(&self.scopes, name),
            });
        }
    }
}

impl AstVisitor for LspVisitor<'_> {
    fn visit_stmt(&mut self, stmt: &Spanned<Statement>) {
        // Handle definitions and references that need richer info
        // than on_define/on_reference provide, then delegate traversal.
        match &stmt.node {
            Statement::Let {
                name,
                value,
                type_ann,
                is_pub: _,
            }
            | Statement::Val {
                name,
                value,
                type_ann,
                is_pub: _,
            } => {
                let type_name = type_ann.as_ref().map(format_type_annotation);
                self.register_definition(
                    name,
                    &stmt.span,
                    SymbolKind::Variable,
                    None,
                    type_name,
                    None,
                );
                self.visit_expr(value);
            }

            Statement::Function {
                name,
                params,
                body,
                return_type,
                is_pub: _,
            } => {
                let param_strs: Vec<String> = params
                    .iter()
                    .map(|(pname, ann)| match ann {
                        Some(t) => format!("{pname}: {}", format_type_annotation(t)),
                        None => pname.clone(),
                    })
                    .collect();

                let ret = return_type.as_ref().map(format_type_annotation);

                self.register_definition(
                    name,
                    &stmt.span,
                    SymbolKind::Function,
                    Some(param_strs),
                    None,
                    ret,
                );

                self.enter_scope();
                for (param_name, type_ann) in params {
                    let type_name = type_ann.as_ref().map(format_type_annotation);
                    self.register_definition(
                        param_name,
                        &stmt.span,
                        SymbolKind::Parameter,
                        None,
                        type_name,
                        None,
                    );
                }
                self.visit_expr(body);
                self.exit_scope();
            }

            Statement::Assignment { name, value } => {
                self.register_reference(name, &stmt.span);
                self.visit_expr(value);
            }

            Statement::ObjDef {
                name,
                fields,
                methods,
                uses,
                ..
            } => {
                self.register_definition(name, &stmt.span, SymbolKind::Object, None, None, None);

                for used_type in uses {
                    for segment in used_type {
                        self.register_reference(segment, &stmt.span);
                    }
                }

                for field in fields {
                    self.visit_obj_field(field);
                }

                for method in methods {
                    self.visit_obj_method(method);
                }
            }

            Statement::Import { path } => {
                // Register the full dotted path as a Module symbol so the
                // root segment shows up as a known identifier in hover and
                // go-to-definition. The full path becomes the symbol name.
                let dotted = path.join(".");
                self.register_definition(&dotted, &stmt.span, SymbolKind::Module, None, None, None);
                // Also register the root segment so a bare reference like
                // `math` (in `math.add(...)`) resolves to this import.
                if let Some(root) = path.first() {
                    self.register_definition(
                        root,
                        &stmt.span,
                        SymbolKind::Module,
                        None,
                        None,
                        None,
                    );
                }
            }

            // For all other statements, use the default walk.
            _ => {
                walk_stmt(self, stmt);
            }
        }
    }

    fn visit_expr(&mut self, expr: &Spanned<Expression>) {
        match &expr.node {
            Expression::Ident(name) => {
                self.register_reference(name, &expr.span);
            }

            Expression::Call { name, args } => {
                self.register_reference(name, &expr.span);
                for arg in args {
                    self.visit_expr(arg);
                }
            }

            // For all other expressions, use the default walk.
            _ => {
                walk_expr(self, expr);
            }
        }
    }

    fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn exit_scope(&mut self) {
        self.scopes.pop();
    }
}

impl LspVisitor<'_> {
    fn visit_obj_field(&mut self, field: &ObjField) {
        let type_name = format_type_annotation(&field.type_ann);
        self.register_definition(
            &field.name,
            &field.span,
            SymbolKind::Field,
            None,
            Some(type_name),
            None,
        );
    }

    fn visit_obj_method(&mut self, method: &ObjMethod) {
        let param_strs: Vec<String> = method
            .params
            .iter()
            .map(|(pname, ann)| match ann {
                Some(t) => format!("{pname}: {}", format_type_annotation(t)),
                None => pname.clone(),
            })
            .collect();

        let ret = method.return_type.as_ref().map(format_type_annotation);

        self.register_definition(
            &method.name,
            &method.span,
            SymbolKind::Function,
            Some(param_strs),
            None,
            ret,
        );

        if let Some(body) = &method.body {
            self.enter_scope();
            for (param_name, type_ann) in &method.params {
                let type_name = type_ann.as_ref().map(format_type_annotation);
                self.register_definition(
                    param_name,
                    &method.span,
                    SymbolKind::Parameter,
                    None,
                    type_name,
                    None,
                );
            }
            self.visit_expr(body);
            self.exit_scope();
        }
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

    #[test]
    fn obj_def_creates_definition() {
        let table = analyze("obj Vec2 {\nx: int\ny: int\n}");

        let obj_defs: Vec<&SymbolInfo> = table
            .definitions
            .iter()
            .filter(|d| d.kind == SymbolKind::Object)
            .collect();
        assert_eq!(obj_defs.len(), 1);
        assert_eq!(obj_defs[0].name, "Vec2");
    }

    #[test]
    fn obj_method_creates_definition() {
        let table = analyze("obj Foo {\nfn bar(self) {\nprint(1)\n}\n}");

        assert!(
            table
                .definitions
                .iter()
                .any(|d| d.name == "Foo" && d.kind == SymbolKind::Object)
        );
        assert!(
            table
                .definitions
                .iter()
                .any(|d| d.name == "bar" && d.kind == SymbolKind::Function)
        );
        assert!(
            table
                .definitions
                .iter()
                .any(|d| d.name == "self" && d.kind == SymbolKind::Parameter)
        );
    }

    #[test]
    fn obj_use_creates_reference() {
        let table =
            analyze("obj Base {\nfn hello(self) {\nprint(1)\n}\n}\nobj Child {\nuse Base\n}");

        let base_refs: Vec<&SymbolRef> = table
            .references
            .iter()
            .filter(|r| r.name == "Base")
            .collect();
        assert_eq!(base_refs.len(), 1);
        assert!(base_refs[0].definition_idx.is_some());
    }

    #[test]
    fn obj_method_body_walks_expressions() {
        let table = analyze("let x = 5\nobj Foo {\nfn bar(self) {\nprint(x)\n}\n}");

        let x_refs: Vec<&SymbolRef> = table.references.iter().filter(|r| r.name == "x").collect();
        assert_eq!(x_refs.len(), 1);
        assert_eq!(x_refs[0].definition_idx, Some(0));
    }

    #[test]
    fn obj_fields_are_registered() {
        let source = "obj Vec2 {\nx: int\ny: int\n}";
        let table = analyze(source);

        let fields: Vec<&SymbolInfo> = table
            .definitions
            .iter()
            .filter(|d| d.kind == SymbolKind::Field)
            .collect();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "x");
        assert_eq!(fields[0].type_name.as_deref(), Some("int"));
        assert_eq!(fields[1].name, "y");
    }

    #[test]
    fn obj_field_full_span_covers_only_the_field_line() {
        let source = "obj Vec2 {\nx: int\ny: int\n}";
        let table = analyze(source);

        let field = table
            .definitions
            .iter()
            .find(|d| d.kind == SymbolKind::Field && d.name == "x")
            .expect("missing field x");
        // The field's full_span should not cover the entire obj.
        assert!(field.full_span.end - field.full_span.start < source.len());
        assert_eq!(&source[field.full_span.clone()], "x: int");
    }

    #[test]
    fn obj_method_full_span_is_method_only_not_whole_obj() {
        let source = "obj Foo {\nfn bar(self) {\nrn 1\n}\n}";
        let table = analyze(source);

        let method = table
            .definitions
            .iter()
            .find(|d| d.kind == SymbolKind::Function && d.name == "bar")
            .expect("missing method bar");
        // The method's full_span must start at `fn`, not at `obj`.
        let snippet = &source[method.full_span.clone()];
        assert!(snippet.starts_with("fn bar"), "got: {snippet:?}");
        assert!(!snippet.contains("obj Foo"), "got: {snippet:?}");
    }
}
