use lsp_types::{DocumentSymbol, SymbolKind as LspSymbolKind};

use crate::analysis::{SymbolKind, SymbolTable};
use crate::diagnostics::span_to_range;

/// Build document symbols for the outline view. Returns top-level
/// function and variable definitions.
#[allow(deprecated)]
pub fn document_symbols(source: &str, symbol_table: &SymbolTable) -> Vec<DocumentSymbol> {
    symbol_table
        .definitions
        .iter()
        .filter(|def| {
            def.scope_depth == 0
                && matches!(
                    def.kind,
                    SymbolKind::Function | SymbolKind::Variable | SymbolKind::Object
                )
        })
        .map(|def| {
            let kind = match def.kind {
                SymbolKind::Function => LspSymbolKind::FUNCTION,
                SymbolKind::Variable => LspSymbolKind::VARIABLE,
                SymbolKind::Parameter => LspSymbolKind::VARIABLE,
                SymbolKind::Object => LspSymbolKind::STRUCT,
            };

            let detail = match &def.kind {
                SymbolKind::Function => def.params.as_ref().map(|p| format!("({})", p.join(", "))),
                _ => None,
            };

            // DocumentSymbol has a deprecated `deprecated` field that
            // must still be provided for the struct literal.
            DocumentSymbol {
                name: def.name.clone(),
                detail,
                kind,
                tags: None,
                deprecated: None,
                range: span_to_range(source, def.full_span.clone()),
                selection_range: span_to_range(source, def.name_span.clone()),
                children: None,
            }
        })
        .collect()
}
