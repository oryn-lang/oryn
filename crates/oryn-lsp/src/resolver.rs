//! Shared cross-file symbol resolution used by both goto-definition
//! and hover. Walks a dotted chain backward from the cursor (e.g.
//! `math.vec2.Vec2`), matches the head against an `import` statement,
//! resolves the module file on disk, and locates the target symbol in
//! the imported module's symbol table.

use std::path::Path;

use lsp_types::Uri;

use crate::analysis::{SymbolKind, SymbolTable};

/// A symbol resolved across module boundaries. Owns the imported
/// module's source text and symbol table so callers can format hover
/// output against them or build a `Location` for goto-definition.
pub struct ResolvedSymbol {
    pub module_source: String,
    pub module_table: SymbolTable,
    pub module_uri: Uri,
    /// Index into `module_table.definitions`.
    pub def_idx: usize,
}

/// Attempt to resolve the identifier at `offset` across module
/// boundaries. Returns `None` when the cursor isn't on a dotted chain,
/// the first segment isn't an imported module, the module file cannot
/// be read, or no matching symbol exists.
pub fn resolve_cross_file(source: &str, offset: usize, file_path: &Path) -> Option<ResolvedSymbol> {
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
        while i > 0 && matches!(&tokens[i].0, oryn::Token::Newline) {
            i -= 1;
        }
        if !matches!(&tokens[i].0, oryn::Token::Dot) {
            break;
        }
        if i == 0 {
            break;
        }
        i -= 1;
        while i > 0 && matches!(&tokens[i].0, oryn::Token::Newline) {
            i -= 1;
        }
        match &tokens[i].0 {
            oryn::Token::Ident(name) => chain.push(name.clone()),
            _ => break,
        }
    }
    chain.reverse();

    // Need at least module + something.
    if chain.len() < 2 {
        return None;
    }

    // Parse the file to find `import` statements.
    let (parsed_stmts, _) = oryn::parse(tokens);

    // Find an import whose leading segments match the prefix of our chain.
    // Prefer the longest match so nested imports like `import math.vec2`
    // bind more specifically than `import math`.
    let import_path = parsed_stmts
        .iter()
        .filter_map(|s| match &s.node {
            oryn::Statement::Import { path } if path_matches_prefix(path, &chain) => {
                Some(path.clone())
            }
            _ => None,
        })
        .max_by_key(|p| p.len())?;

    // Resolve the module file on disk.
    let parent = file_path.parent()?;
    let project_root = oryn::find_project_root(parent)?;
    let module_file = oryn::resolve_import(&project_root, &import_path);
    if !module_file.exists() {
        return None;
    }

    let module_source = std::fs::read_to_string(&module_file).ok()?;
    let module_table = crate::analysis::analyze(&module_source);
    let module_uri = path_to_uri(&module_file)?;

    // Member segments are everything after the import path prefix.
    let member_segments = &chain[import_path.len()..];
    if member_segments.is_empty() {
        return None;
    }

    let def_idx = find_def_in_module(&module_table, &target_name, member_segments)?;

    Some(ResolvedSymbol {
        module_source,
        module_table,
        module_uri,
        def_idx,
    })
}

/// True if every segment of `import_path` matches the corresponding
/// segment at the start of `chain`.
fn path_matches_prefix(import_path: &[String], chain: &[String]) -> bool {
    import_path.len() <= chain.len() && import_path.iter().zip(chain.iter()).all(|(a, b)| a == b)
}

/// Search the module's symbol table for the index of the definition
/// that matches `target_name`, preferring top-level symbols and
/// scoping to the parent type when a dotted member access is known.
fn find_def_in_module(
    module_table: &SymbolTable,
    target_name: &str,
    member_segments: &[String],
) -> Option<usize> {
    // First, check top-level definitions (scope_depth == 0). Ignore
    // module re-exports (SymbolKind::Module) since those wouldn't have
    // a meaningful hover/definition target in the imported file.
    for (idx, def) in module_table.definitions.iter().enumerate() {
        if def.name == target_name && def.scope_depth == 0 && def.kind != SymbolKind::Module {
            return Some(idx);
        }
    }

    // For nested members (e.g. `guard.Guard.spawn`), scope the search
    // to the parent type's span.
    if member_segments.len() >= 2 {
        let type_name = &member_segments[0];
        let type_def = module_table
            .definitions
            .iter()
            .find(|d| d.name == *type_name && d.scope_depth == 0)?;

        for (idx, def) in module_table.definitions.iter().enumerate() {
            if def.name == target_name
                && def.scope_depth > 0
                && def.full_span.start >= type_def.full_span.start
                && def.full_span.end <= type_def.full_span.end
            {
                return Some(idx);
            }
        }
    }

    None
}

/// Build a `file://` URI from a filesystem path. Returns `None` if the
/// path cannot be encoded.
fn path_to_uri(path: &Path) -> Option<Uri> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let uri_string = format!("file://{}", canonical.display());
    uri_string.parse().ok()
}
