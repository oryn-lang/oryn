use lsp_types::{Location, Position, Uri};

use super::analysis::SymbolTable;
use super::diagnostics::{position_to_offset, span_to_range};

/// Find all references to the symbol under the cursor.
pub fn find_references(
    source: &str,
    pos: Position,
    uri: &Uri,
    include_declaration: bool,
    symbol_table: &SymbolTable,
) -> Vec<Location> {
    let offset = match position_to_offset(source, pos) {
        Some(o) => o,
        None => return Vec::new(),
    };

    // Figure out which definition the cursor is on (either directly
    // on a definition, or on a reference that resolves to one).
    let def_idx = find_definition_idx(offset, symbol_table);

    let def_idx = match def_idx {
        Some(idx) => idx,
        None => return Vec::new(),
    };

    let mut locations = Vec::new();

    // Include the definition itself if requested.
    if include_declaration {
        let def = &symbol_table.definitions[def_idx];
        locations.push(Location {
            uri: uri.clone(),
            range: span_to_range(source, def.name_span.clone()),
        });
    }

    // Collect all references that resolve to this definition.
    for reference in &symbol_table.references {
        if reference.definition_idx == Some(def_idx) {
            locations.push(Location {
                uri: uri.clone(),
                range: span_to_range(source, reference.name_span.clone()),
            });
        }
    }

    locations
}

/// Find the definition index for the symbol under the cursor.
fn find_definition_idx(offset: usize, symbol_table: &SymbolTable) -> Option<usize> {
    // Check if cursor is directly on a definition.
    for (i, def) in symbol_table.definitions.iter().enumerate() {
        if offset >= def.name_span.start && offset < def.name_span.end {
            return Some(i);
        }
    }

    // Check if cursor is on a reference.
    for reference in &symbol_table.references {
        if offset >= reference.name_span.start && offset < reference.name_span.end {
            return reference.definition_idx;
        }
    }

    None
}
