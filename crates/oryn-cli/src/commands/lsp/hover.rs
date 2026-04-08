use lsp_types::{Hover, HoverContents, MarkedString, Position};

use super::analysis::{SymbolKind, SymbolTable};
use super::diagnostics::{position_to_offset, span_to_range};

/// Build hover info for the token at the given cursor position.
/// Uses the symbol table for rich info on identifiers, falls back
/// to token-level descriptions for keywords and operators.
pub fn hover(source: &str, pos: Position, symbol_table: &SymbolTable) -> Option<Hover> {
    let offset = position_to_offset(source, pos)?;
    let (tokens, _) = oryn::lex(source);

    let (token, span) = tokens
        .into_iter()
        .find(|(_, span)| offset >= span.start && offset < span.end)?;

    let contents = match &token {
        oryn::Token::Ident(name) => hover_ident(name, offset, symbol_table),
        oryn::Token::Int(n) => Some(format!("`{n}` - i32 literal")),
        oryn::Token::Float(n) => Some(format!("`{n}` - f32 literal")),
        oryn::Token::String(s) => Some(format!("`\"{s}\"` - String literal")),
        oryn::Token::True => Some("`true` - bool literal".to_string()),
        oryn::Token::False => Some("`false` - bool literal".to_string()),
        oryn::Token::Let => Some("`let` - declare a mutable variable".to_string()),
        oryn::Token::Val => Some("`val` - declare an immutable variable".to_string()),
        oryn::Token::Obj => Some("`obj` - declare an object type".to_string()),
        oryn::Token::Use => Some("`use` - compose fields and methods from another type".to_string()),
        oryn::Token::Fn => Some("`fn` - declare a function".to_string()),
        oryn::Token::Rn => Some("`rn` - return a value from a function".to_string()),
        oryn::Token::If => Some("`if` - conditional branch".to_string()),
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
        _ => None,
    }?;

    Some(Hover {
        contents: HoverContents::Scalar(MarkedString::String(contents)),
        range: Some(span_to_range(source, span)),
    })
}

/// Build hover info for an identifier by looking it up in the symbol table.
fn hover_ident(name: &str, offset: usize, table: &SymbolTable) -> Option<String> {
    // Check if the cursor is on a definition.
    for def in &table.definitions {
        if offset >= def.name_span.start && offset < def.name_span.end {
            return Some(format_definition(def));
        }
    }

    // Check if it's a reference that resolves to a definition.
    for reference in &table.references {
        if offset >= reference.name_span.start && offset < reference.name_span.end {
            if let Some(def_idx) = reference.definition_idx {
                return Some(format_definition(&table.definitions[def_idx]));
            }
            return Some(format!("`{name}` - unresolved identifier"));
        }
    }

    Some(format!("`{name}` - identifier"))
}

fn format_definition(def: &super::analysis::SymbolInfo) -> String {
    match def.kind {
        SymbolKind::Function => {
            let params = def
                .params
                .as_ref()
                .map(|p| p.join(", "))
                .unwrap_or_default();
            let ret = match &def.return_type {
                Some(rt) => format!(" -> {rt}"),
                None => String::new(),
            };
            format!("```oryn\nfn {}({}){}\n```", def.name, params, ret)
        }
        SymbolKind::Variable => {
            let type_str = match &def.type_name {
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
    }
}
