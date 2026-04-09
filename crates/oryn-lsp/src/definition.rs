use lsp_types::{GotoDefinitionResponse, Location, Position, Uri};

use crate::analysis::SymbolTable;
use crate::diagnostics::{position_to_offset, span_to_range};

/// Find the definition of the symbol under the cursor.
pub fn goto_definition(
    source: &str,
    pos: Position,
    uri: &Uri,
    symbol_table: &SymbolTable,
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

    None
}
