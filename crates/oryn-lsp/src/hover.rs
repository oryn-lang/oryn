use std::path::Path;

use lsp_types::{Hover, HoverContents, MarkedString, Position};

use crate::analysis::{SymbolKind, SymbolTable};
use crate::diagnostics::{position_to_offset, span_to_range};
use crate::resolver;

/// Build hover info for the token at the given cursor position.
/// Uses the symbol table for rich info on identifiers, falls back
/// to token-level descriptions for keywords and operators. When the
/// local lookup misses and `file_path` is provided, also tries
/// cross-file resolution for dotted chains like `math.vec2.Vec2`.
pub fn hover(
    source: &str,
    pos: Position,
    symbol_table: &SymbolTable,
    docs: &oryn::DocTable,
    types: &oryn::TypeMap,
    file_path: Option<&Path>,
) -> Option<Hover> {
    let offset = position_to_offset(source, pos)?;
    let (tokens, _) = oryn::lex(source);

    let (token, span) = tokens
        .into_iter()
        .find(|(_, span)| offset >= span.start && offset < span.end)?;

    // For strings with interpolation, check if the cursor is on a
    // symbol reference inside the string before falling back to the
    // generic "String literal" hover.
    if matches!(&token, oryn::Token::String(_))
        && let Some(info) = hover_reference(offset, symbol_table, source, docs, types)
    {
        return Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(info)),
            range: Some(span_to_range(source, span)),
        });
    }

    let contents = match &token {
        oryn::Token::Ident(name) => {
            hover_ident(name, offset, symbol_table, source, docs, types, file_path)
        }
        oryn::Token::Int(n) => Some(format!("`{n}` - int literal")),
        oryn::Token::Float(n) => Some(format!("`{n}` - float literal")),
        oryn::Token::String(s) => Some(format!("`\"{s}\"` - String literal")),
        oryn::Token::True => Some("`true` - bool literal".to_string()),
        oryn::Token::False => Some("`false` - bool literal".to_string()),
        oryn::Token::Let => Some("`let` - declare a mutable variable".to_string()),
        oryn::Token::Val => Some("`val` - declare an immutable variable".to_string()),
        oryn::Token::Obj => Some("`obj` - declare an object type".to_string()),
        oryn::Token::Use => {
            Some("`use` - compose fields and methods from another type".to_string())
        }
        oryn::Token::Fn => Some("`fn` - declare a function".to_string()),
        oryn::Token::Rn => Some("`rn` - return a value from a function".to_string()),
        oryn::Token::If => Some("`if` - conditional branch".to_string()),
        oryn::Token::Unless => {
            Some("`unless` - conditional branch when the condition is false".to_string())
        }
        oryn::Token::Elif => Some("`elif` - else-if branch".to_string()),
        oryn::Token::Else => Some("`else` - fallback branch".to_string()),
        oryn::Token::While => Some("`while` - loop while condition is true".to_string()),
        oryn::Token::Break => Some("`break` - exit the current loop".to_string()),
        oryn::Token::Continue => Some("`continue` - skip to next loop iteration".to_string()),
        oryn::Token::And => Some("`and` - logical AND".to_string()),
        oryn::Token::Or => Some("`or` - logical OR".to_string()),
        oryn::Token::Not => Some("`not` - logical NOT".to_string()),
        oryn::Token::Plus => Some("`+` - addition".to_string()),
        oryn::Token::Minus => Some("`-` - subtraction".to_string()),
        oryn::Token::Multiply => Some("`*` - multiplication".to_string()),
        oryn::Token::Divide => Some("`/` - division".to_string()),
        oryn::Token::Equals => Some("`=` - assignment".to_string()),
        oryn::Token::EqualsEquals => Some("`==` - equality comparison".to_string()),
        oryn::Token::NotEquals => Some("`!=` - inequality comparison".to_string()),
        oryn::Token::LessThan => Some("`<` - less than".to_string()),
        oryn::Token::GreaterThan => Some("`>` - greater than".to_string()),
        oryn::Token::LessThanEquals => Some("`<=` - less than or equal".to_string()),
        oryn::Token::GreaterThanEquals => Some("`>=` - greater than or equal".to_string()),
        oryn::Token::LeftBracket => Some(
            "`[` - list literal or list index (e.g. `[1, 2, 3]`, `xs[0]`, `[int]`)".to_string(),
        ),
        oryn::Token::RightBracket => {
            Some("`]` - closes a list literal, index expression, or list type".to_string())
        }
        _ => None,
    }?;

    Some(Hover {
        contents: HoverContents::Scalar(MarkedString::String(contents)),
        range: Some(span_to_range(source, span)),
    })
}

/// Check if the cursor offset falls on a symbol reference (e.g. inside
/// a string interpolation) and return hover info if so.
fn hover_reference(
    offset: usize,
    table: &SymbolTable,
    source: &str,
    docs: &oryn::DocTable,
    types: &oryn::TypeMap,
) -> Option<String> {
    for reference in &table.references {
        if offset >= reference.name_span.start
            && offset < reference.name_span.end
            && let Some(def_idx) = reference.definition_idx
        {
            return Some(format_definition(
                &table.definitions[def_idx],
                source,
                docs,
                types,
            ));
        }
    }
    None
}

/// Build hover info for an identifier by looking it up in the symbol table.
/// Falls back to cross-file resolution (via dotted-chain walk) when the
/// local lookup finds nothing and a `file_path` is available.
#[allow(clippy::too_many_arguments)]
fn hover_ident(
    name: &str,
    offset: usize,
    table: &SymbolTable,
    source: &str,
    docs: &oryn::DocTable,
    types: &oryn::TypeMap,
    file_path: Option<&Path>,
) -> Option<String> {
    // Check if the cursor is on a definition.
    for def in &table.definitions {
        if offset >= def.name_span.start && offset < def.name_span.end {
            return Some(format_definition(def, source, docs, types));
        }
    }

    // Check if it's a reference that resolves to a definition in-file.
    for reference in &table.references {
        if offset >= reference.name_span.start && offset < reference.name_span.end {
            if let Some(def_idx) = reference.definition_idx {
                return Some(format_definition(
                    &table.definitions[def_idx],
                    source,
                    docs,
                    types,
                ));
            }
            // Unresolved in-file: fall through to cross-file lookup
            // before giving up, since `math.add` is recorded as a
            // reference to `math` but `add` is not registered locally.
            break;
        }
    }

    // Cross-file dotted-chain lookup: handles `math.add`, `guard.Guard.spawn`,
    // etc. Formats the result against the *imported module's* source and
    // DocTable so doc comments from the target file are shown. Cross-file
    // type inference isn't wired up yet — pass an empty TypeMap so the
    // annotation-only path is used for imported symbols.
    if let Some(path) = file_path
        && let Some(resolved) = resolver::resolve_cross_file(source, offset, path)
    {
        let def = &resolved.module_table.definitions[resolved.def_idx];
        let module_docs = oryn::DocTable::build(&resolved.module_source);
        let module_types = oryn::TypeMap::default();
        return Some(format_definition(
            def,
            &resolved.module_source,
            &module_docs,
            &module_types,
        ));
    }

    // Builtin list methods: `xs.len()`, `xs.push(y)`, `xs.pop()`. These
    // aren't recorded in the symbol table because they live in the
    // [`oryn::ListMethod`] table, not user source. Detect them by
    // checking whether the hovered ident is a known list method name
    // AND is preceded by a `.` token — the heuristic for a method
    // call. Not receiver-type-aware, but that would require cross-
    // referencing the TypeMap at the span of the expression before `.`,
    // and the simpler form catches every real-world case.
    if let Some(list_method) = oryn::ListMethod::from_name(name)
        && ident_is_method_call(source, offset)
    {
        return Some(format_list_method(list_method));
    }

    Some(format!("`{name}` - identifier"))
}

/// Returns true when the identifier at `offset` is immediately preceded
/// by a `.` token (skipping whitespace), i.e. looks like `expr.name`.
/// Used to disambiguate list method hovers from plain identifier hovers.
fn ident_is_method_call(source: &str, offset: usize) -> bool {
    // Walk backwards through whitespace until we find the character
    // immediately before the identifier token.
    let mut i = offset;
    // First scan to the start of this identifier.
    while i > 0 {
        let prev = source.as_bytes()[i - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            i -= 1;
        } else {
            break;
        }
    }
    // Now skip whitespace before the ident (Oryn allows none, but be safe).
    while i > 0 {
        let prev = source.as_bytes()[i - 1];
        if prev == b' ' || prev == b'\t' {
            i -= 1;
        } else {
            break;
        }
    }
    i > 0 && source.as_bytes()[i - 1] == b'.'
}

/// Format a [`oryn::ListMethod`] as a hover string with the method
/// signature and a short description. Matches the shape of user-
/// function hover output so editors render them consistently.
fn format_list_method(method: oryn::ListMethod) -> String {
    let (sig, desc) = match method {
        oryn::ListMethod::Len => (
            "fn len(self) -> int",
            "Return the number of elements currently in the list.",
        ),
        oryn::ListMethod::Push => (
            "fn push(self, value: T)",
            "Append `value` to the end of the list. The argument must match the list's element type `T`.",
        ),
        oryn::ListMethod::Pop => (
            "fn pop(self) -> T?",
            "Remove and return the last element, or `nil` if the list is empty. `T` is the list's element type.",
        ),
    };
    format!("```oryn\n{sig}\n```\n\n{desc}")
}

fn format_definition(
    def: &crate::analysis::SymbolInfo,
    source: &str,
    docs: &oryn::DocTable,
    types: &oryn::TypeMap,
) -> String {
    let signature = format_signature(def, types);
    match docs.lookup_above(source, def.full_span.start) {
        Some(doc) => format!("{doc}\n\n{signature}"),
        None => signature,
    }
}

/// Resolve the "best" type string for a definition:
/// - explicit annotation wins (already recorded by `analysis.rs`)
/// - else fall back to the compiler's inferred type via `TypeMap`
/// - else None
fn resolved_type_for<'a>(
    def: &'a crate::analysis::SymbolInfo,
    types: &'a oryn::TypeMap,
) -> Option<&'a str> {
    def.type_name
        .as_deref()
        .or_else(|| types.get(&def.full_span))
}

fn format_signature(def: &crate::analysis::SymbolInfo, types: &oryn::TypeMap) -> String {
    match def.kind {
        SymbolKind::Function => {
            let params = def
                .params
                .as_ref()
                .map(|p| p.join(", "))
                .unwrap_or_default();
            let inferred_return = types.get(&def.full_span);
            let ret = match def.return_type.as_deref().or(inferred_return) {
                Some(rt) if rt != "void" => format!(" -> {rt}"),
                _ => String::new(),
            };
            format!("```oryn\nfn {}({}){}\n```", def.name, params, ret)
        }
        SymbolKind::Variable => {
            let type_str = match resolved_type_for(def, types) {
                Some(t) => format!(": {t}"),
                None => String::new(),
            };
            format!("`let {}{}`", def.name, type_str)
        }
        SymbolKind::Parameter => {
            let type_str = match &def.type_name {
                Some(t) => format!(": {t}"),
                None => String::new(),
            };
            format!("`{}{}`  - parameter", def.name, type_str)
        }
        SymbolKind::Object => {
            format!("```oryn\nobj {}\n```", def.name)
        }
        SymbolKind::Field => {
            let type_str = match &def.type_name {
                Some(t) => format!(": {t}"),
                None => String::new(),
            };
            format!("`{}{}`  - field", def.name, type_str)
        }
        SymbolKind::Module => {
            format!("```oryn\nimport {}\n```", def.name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze;
    use crate::diagnostics::offset_to_position;

    fn hover_at(source: &str, needle: &str) -> String {
        let offset = source.find(needle).expect("needle not in source");
        let symbols = analyze(source);
        let docs = oryn::DocTable::build(source);
        let (_, types) = oryn::Chunk::check_with_types(source);
        let pos = offset_to_position(source, offset);
        let result =
            hover(source, pos, &symbols, &docs, &types, None).expect("hover returned None");
        match result.contents {
            lsp_types::HoverContents::Scalar(lsp_types::MarkedString::String(s)) => s,
            other => panic!("unexpected hover contents: {other:?}"),
        }
    }

    #[test]
    fn doc_comment_above_top_level_function_is_shown() {
        let source = "// adds two ints\nfn add(a, b) {\nrn a + b\n}";
        let out = hover_at(source, "add(");
        assert!(out.contains("adds two ints"), "got: {out}");
        assert!(out.contains("fn add(a, b)"), "got: {out}");
    }

    #[test]
    fn doc_comment_above_let_is_shown() {
        let source = "// the answer\nlet x = 42";
        let out = hover_at(source, "x = 42");
        assert!(out.contains("the answer"), "got: {out}");
    }

    #[test]
    fn doc_comment_above_obj_is_shown() {
        let source = "// 2D vector\nobj Vec2 {\nx: int\ny: int\n}";
        let out = hover_at(source, "Vec2");
        assert!(out.contains("2D vector"), "got: {out}");
        assert!(out.contains("obj Vec2"), "got: {out}");
    }

    #[test]
    fn doc_comment_above_obj_field_is_shown() {
        let source = "obj Vec2 {\n// horizontal coordinate\nx: int\ny: int\n}";
        let out = hover_at(source, "x: int");
        assert!(out.contains("horizontal coordinate"), "got: {out}");
        assert!(out.contains("- field"), "got: {out}");
    }

    #[test]
    fn doc_comment_above_obj_method_is_shown() {
        let source =
            "obj Foo {\nx: int\n// returns the value\nfn get(self) -> int {\nrn self.x\n}\n}";
        let out = hover_at(source, "get");
        assert!(out.contains("returns the value"), "got: {out}");
        assert!(out.contains("fn get"), "got: {out}");
    }

    #[test]
    fn declaration_without_doc_comment_is_unchanged() {
        let source = "fn add(a, b) {\nrn a + b\n}";
        let out = hover_at(source, "add");
        // No doc comment, output is just the signature.
        assert!(!out.contains("\n\n"), "got: {out}");
        assert!(out.contains("fn add(a, b)"), "got: {out}");
    }

    #[test]
    fn multi_line_doc_comment_is_joined_with_newlines() {
        let source = "// first line\n// second line\nfn foo() {}";
        let out = hover_at(source, "foo");
        assert!(out.contains("first line\nsecond line"), "got: {out}");
    }

    #[test]
    fn inferred_type_shown_for_let_with_no_annotation() {
        let source = "let x = 5";
        let out = hover_at(source, "x =");
        assert!(out.contains("let x: int"), "got: {out}");
    }

    #[test]
    fn inferred_type_from_obj_literal() {
        let source = "obj Vec2 {\nx: int\ny: int\n}\nlet v = Vec2 { x: 1, y: 2 }";
        let out = hover_at(source, "v =");
        assert!(out.contains("let v: Vec2"), "got: {out}");
    }

    #[test]
    fn annotation_still_wins_over_inference() {
        // Explicit annotation should be used verbatim (same spelling
        // the user wrote), not the compiler's normalized form.
        let source = "let x: int = 5";
        let out = hover_at(source, "x:");
        assert!(out.contains("let x: int"), "got: {out}");
    }

    #[test]
    fn inferred_return_type_shown_for_function() {
        // `fn add(a, b)` — wait, params need annotations. Use a
        // void-returning function with no annotation, then check the
        // signature does NOT include `-> void`.
        let source = "fn greet() {\nprint(\"hi\")\n}";
        let out = hover_at(source, "greet");
        assert!(out.contains("fn greet()"), "got: {out}");
        assert!(!out.contains("-> void"), "got: {out}");
    }

    /// End-to-end: hovering on a `let` whose RHS is a cross-module
    /// static method call should show the return type from the
    /// imported module (`Color` in this case). This is the scenario
    /// the user explicitly asked to fix.
    #[test]
    fn inferred_type_from_cross_module_call() {
        use std::path::PathBuf;

        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let main_path = manifest
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples")
            .join("modules")
            .join("main.on");
        assert!(main_path.exists(), "fixture missing: {main_path:?}");

        let source = std::fs::read_to_string(&main_path).expect("read main.on");
        let symbols = analyze(&source);
        let docs = oryn::DocTable::build(&source);
        let (_, types) = oryn::Chunk::check_file_with_types(&main_path, &source);

        // Cursor on `red` in `let red = math.colors.Color.rgb(...)`.
        // Use "red =" as a distinctive needle (no other "red =" in
        // main.on).
        let needle = "red =";
        let offset = source.find(needle).expect("needle not in main.on");
        let pos = offset_to_position(&source, offset);

        let result = hover(&source, pos, &symbols, &docs, &types, Some(&main_path))
            .expect("hover returned None");

        let out = match result.contents {
            HoverContents::Scalar(MarkedString::String(s)) => s,
            other => panic!("unexpected hover contents: {other:?}"),
        };

        assert!(
            out.contains("let red: Color") || out.contains("let red: math.colors.Color"),
            "cross-module inferred type missing — got: {out}"
        );
    }

    #[test]
    fn list_method_len_shows_signature() {
        let source = "let xs: [int] = [1, 2, 3]\nlet n = xs.len()";
        let out = hover_at(source, "len()");
        assert!(out.contains("fn len(self) -> int"), "got: {out}");
        assert!(out.contains("number of elements"), "got: {out}");
    }

    #[test]
    fn list_method_push_shows_signature() {
        let source = "let xs: [int] = [1]\nxs.push(2)";
        let out = hover_at(source, "push(");
        assert!(out.contains("fn push(self, value: T)"), "got: {out}");
        assert!(out.contains("element type"), "got: {out}");
    }

    #[test]
    fn list_method_pop_shows_nillable_return() {
        let source = "let xs: [int] = [1, 2]\nlet last = xs.pop()";
        let out = hover_at(source, "pop()");
        assert!(out.contains("fn pop(self) -> T?"), "got: {out}");
        assert!(out.contains("list is empty"), "got: {out}");
    }

    #[test]
    fn list_typed_let_shows_list_type() {
        let source = "let xs: [int] = [1, 2, 3]";
        let out = hover_at(source, "xs:");
        assert!(out.contains("let xs: [int]"), "got: {out}");
    }

    #[test]
    fn inferred_list_type_is_shown() {
        // No annotation — the compiler infers [int] from the literal.
        let source = "let xs = [1, 2, 3]";
        let out = hover_at(source, "xs =");
        assert!(out.contains("let xs: [int]"), "got: {out}");
    }

    /// Hovering on a static method call that crosses module boundaries
    /// (`math.vec2.Vec2.origin()`) should resolve to the method in
    /// vec2.on AND prepend its `//` doc comment from that file's source.
    #[test]
    fn cross_file_hover_on_static_method_shows_doc_comment() {
        use std::path::PathBuf;

        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let main_path = manifest
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples")
            .join("modules")
            .join("main.on");
        assert!(main_path.exists(), "fixture missing: {main_path:?}");

        let source = std::fs::read_to_string(&main_path).expect("read main.on");
        let symbols = analyze(&source);
        let docs = oryn::DocTable::build(&source);
        let (_, types) = oryn::Chunk::check_file_with_types(&main_path, &source);

        // Cursor on `origin` in `math.vec2.Vec2.origin()` (line 41).
        // vec2.on has "// Public static constructor." directly above
        // this method, which should appear in the hover output.
        let needle = "origin()";
        let offset = source.find(needle).expect("needle not in main.on");
        let pos = offset_to_position(&source, offset);

        let result = hover(&source, pos, &symbols, &docs, &types, Some(&main_path))
            .expect("hover returned None for cross-file method");

        let out = match result.contents {
            HoverContents::Scalar(MarkedString::String(s)) => s,
            other => panic!("unexpected hover contents: {other:?}"),
        };

        assert!(out.contains("fn origin"), "missing signature — got: {out}");
        assert!(
            out.contains("Public static constructor"),
            "cross-file hover missing doc comment from vec2.on — got: {out}"
        );
    }
}
