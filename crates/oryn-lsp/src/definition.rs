use std::path::Path;

use lsp_types::{GotoDefinitionResponse, Location, Position, Uri};

use crate::analysis::SymbolTable;
use crate::diagnostics::{position_to_offset, span_to_range};

/// Find the definition of the symbol under the cursor.
pub fn goto_definition(
    source: &str,
    pos: Position,
    uri: &Uri,
    symbol_table: &SymbolTable,
    file_path: Option<&Path>,
) -> Option<GotoDefinitionResponse> {
    let offset = position_to_offset(source, pos)?;

    // If the cursor is on a reference, jump to its definition.
    for reference in &symbol_table.references {
        if offset >= reference.name_span.start && offset < reference.name_span.end {
            let def_idx = reference.definition_idx?;
            let def = &symbol_table.definitions[def_idx];

            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: span_to_range(source, def.name_span.clone()),
            }));
        }
    }

    // If the cursor is already on a definition, return its own location
    // (standard LSP self-reference behavior).
    for def in &symbol_table.definitions {
        if offset >= def.name_span.start && offset < def.name_span.end {
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: span_to_range(source, def.name_span.clone()),
            }));
        }
    }

    // If standard in-file lookup found nothing, try cross-file resolution.
    if let Some(path) = file_path {
        return try_cross_file_definition(source, offset, path);
    }

    None
}

/// Attempt to resolve a cross-file definition by walking the dotted access
/// chain around the cursor (e.g. `guard.Guard.spawn`), matching the first
/// segment against an `import` statement, and looking up the target symbol
/// in the imported module's symbol table.
fn try_cross_file_definition(
    source: &str,
    offset: usize,
    file_path: &Path,
) -> Option<GotoDefinitionResponse> {
    let (tokens, _) = oryn::lex(source);

    // Find the token index at the cursor offset.
    let cursor_idx = tokens
        .iter()
        .position(|(_, span)| offset >= span.start && offset < span.end)?;

    // The cursor must be on an identifier.
    let target_name = match &tokens[cursor_idx].0 {
        oryn::Token::Ident(name) => name.clone(),
        _ => return None,
    };

    // Walk backward through the token stream to collect the full dotted
    // chain (e.g. for `guard.Guard.spawn` we collect spawn, Guard, guard).
    let mut chain = vec![target_name.clone()];
    let mut i = cursor_idx;
    loop {
        if i == 0 {
            break;
        }
        i -= 1;
        // Skip newlines between dot and ident.
        while i > 0 && matches!(&tokens[i].0, oryn::Token::Newline) {
            i -= 1;
        }
        // Expect a dot.
        if !matches!(&tokens[i].0, oryn::Token::Dot) {
            break;
        }
        if i == 0 {
            break;
        }
        i -= 1;
        // Skip newlines between ident and dot.
        while i > 0 && matches!(&tokens[i].0, oryn::Token::Newline) {
            i -= 1;
        }
        // Expect an identifier.
        match &tokens[i].0 {
            oryn::Token::Ident(name) => chain.push(name.clone()),
            _ => break,
        }
    }
    chain.reverse();

    // We need at least two segments: module + something.
    if chain.len() < 2 {
        return None;
    }

    // Parse the file to find `import` statements.
    let (parsed_stmts, _) = oryn::parse(oryn::lex(source).0);

    // Find an import whose first path segment matches the first segment
    // of our chain (e.g. `import guard` matches chain `["guard", "Guard", "spawn"]`).
    let import_path = parsed_stmts.iter().find_map(|s| match &s.node {
        oryn::Statement::Import { path } if path.first().map(String::as_str) == Some(&chain[0]) => {
            Some(path.clone())
        }
        _ => None,
    })?;

    // Resolve the module file on disk.
    let parent = file_path.parent()?;
    let project_root = oryn::find_project_root(parent)?;
    let module_file = oryn::resolve_import(&project_root, &import_path);
    if !module_file.exists() {
        return None;
    }

    // Read and analyze the module.
    let module_source = std::fs::read_to_string(&module_file).ok()?;
    let module_table = crate::analysis::analyze(&module_source);

    // Build a file:// URI for the module.
    let module_uri = path_to_uri(&module_file)?;

    // The member segments are everything after the import path prefix.
    // e.g. chain = ["guard", "Guard", "spawn"], import_path = ["guard"]
    //   → member_segments = ["Guard", "spawn"]
    let member_segments = &chain[import_path.len()..];

    if member_segments.is_empty() {
        return None;
    }

    find_in_module(
        &module_source,
        &module_table,
        &module_uri,
        &target_name,
        member_segments,
    )
}

/// Search the module's symbol table for a definition matching `target_name`.
/// `member_segments` provides the full member access path after the import
/// prefix so we can scope method lookups to the correct type.
/// Build a `file://` URI from a filesystem path. Returns `None` if the
/// path cannot be canonicalized or encoded.
fn path_to_uri(path: &Path) -> Option<Uri> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let uri_string = format!("file://{}", canonical.display());
    uri_string.parse().ok()
}

fn find_in_module(
    module_source: &str,
    module_table: &SymbolTable,
    module_uri: &Uri,
    target_name: &str,
    member_segments: &[String],
) -> Option<GotoDefinitionResponse> {
    // First, check top-level definitions (scope_depth == 0).
    // This handles cases like clicking on `Guard` in `guard.Guard.spawn(...)`.
    for def in &module_table.definitions {
        if def.name == target_name && def.scope_depth == 0 {
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: module_uri.clone(),
                range: span_to_range(module_source, def.name_span.clone()),
            }));
        }
    }

    // If the cursor is on a method/member (e.g. `spawn` in `guard.Guard.spawn`),
    // and we know the parent type from member_segments, scope the search to
    // definitions nested inside that type.
    if member_segments.len() >= 2 {
        let type_name = &member_segments[0];

        // Find the type's definition to scope the method search.
        let type_def = module_table
            .definitions
            .iter()
            .find(|d| d.name == *type_name && d.scope_depth == 0);

        if let Some(type_def) = type_def {
            // Look for the method within the type's full span.
            for def in &module_table.definitions {
                if def.name == target_name
                    && def.scope_depth > 0
                    && def.full_span.start >= type_def.full_span.start
                    && def.full_span.end <= type_def.full_span.end
                {
                    return Some(GotoDefinitionResponse::Scalar(Location {
                        uri: module_uri.clone(),
                        range: span_to_range(module_source, def.name_span.clone()),
                    }));
                }
            }
        }
    }

    None
}
