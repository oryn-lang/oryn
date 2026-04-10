//! Signature help — the popup that shows `fn rgb(r: i32, g: i32, b: i32) -> Color`
//! with the current argument highlighted as the user types inside a call.
//!
//! The algorithm is a pure token walk: starting from the cursor we
//! scan backward tracking paren depth, counting top-level commas (the
//! active parameter index) until we hit the `(` that opens the
//! enclosing call. The identifier immediately before that `(` (plus
//! any leading `Module.Type.` chain) is the function being called;
//! we look it up in the local symbol table or via the cross-file
//! resolver and format the result into a [`SignatureHelp`].

use std::path::Path;

use lsp_types::{
    ParameterInformation, ParameterLabel, Position, SignatureHelp, SignatureInformation,
};

use crate::analysis::{SymbolInfo, SymbolKind, SymbolTable};
use crate::diagnostics::position_to_offset;
use crate::resolver;

/// Build a signature help response for the cursor position, if the
/// cursor is inside the argument list of a call expression.
pub fn signature_help(
    source: &str,
    pos: Position,
    symbols: &SymbolTable,
    file_path: Option<&Path>,
) -> Option<SignatureHelp> {
    let offset = position_to_offset(source, pos)?;
    let (tokens, _) = oryn::lex(source);

    let (open_paren_idx, active_param) = find_enclosing_call(&tokens, offset)?;

    // The function being called is the dotted chain ending right
    // before the opening paren. For `Color.rgb(…)` the chain is
    // `["Color", "rgb"]`; for a bare `add(…)` it's `["add"]`.
    let chain = collect_dotted_chain_before(&tokens, open_paren_idx)?;
    if chain.is_empty() {
        return None;
    }

    // Find the definition: first try the local symbol table, then
    // fall back to cross-file resolution via the existing resolver.
    let def = lookup_callable(&chain, symbols, source, file_path, &tokens, open_paren_idx)?;

    Some(SignatureHelp {
        signatures: vec![def_to_signature_info(&def)],
        active_signature: Some(0),
        active_parameter: Some(active_param),
    })
}

type Token = (oryn::Token, std::ops::Range<usize>);

/// Walk backward from the token at `offset`, tracking paren depth.
/// Returns `(open_paren_token_index, active_parameter_index)` when the
/// cursor is inside an unclosed `(...)`. Commas encountered at depth 0
/// count as argument separators.
fn find_enclosing_call(tokens: &[Token], offset: usize) -> Option<(usize, u32)> {
    // Start at the token whose span ends strictly before `offset`, or
    // the token the cursor sits on. Using `span.end <= offset` picks up
    // the previous token when the cursor is in a gap (e.g. whitespace).
    let mut idx = tokens.iter().rposition(|(_, span)| span.start <= offset)?;

    let mut depth: i32 = 0;
    let mut commas: u32 = 0;

    loop {
        match &tokens[idx].0 {
            oryn::Token::RightParen => depth += 1,
            oryn::Token::LeftParen => {
                if depth == 0 {
                    return Some((idx, commas));
                }
                depth -= 1;
            }
            oryn::Token::Comma if depth == 0 => commas += 1,
            _ => {}
        }
        if idx == 0 {
            return None;
        }
        idx -= 1;
    }
}

/// Given the index of a `(` token, collect the dotted identifier chain
/// immediately preceding it: `Color.rgb(` → `["Color", "rgb"]`.
/// Returns `None` if the token before the paren isn't an identifier.
fn collect_dotted_chain_before(tokens: &[Token], open_paren_idx: usize) -> Option<Vec<String>> {
    if open_paren_idx == 0 {
        return None;
    }
    let mut i = open_paren_idx - 1;

    // The token just before `(` must be an ident.
    let mut chain = match &tokens[i].0 {
        oryn::Token::Ident(name) => vec![name.clone()],
        _ => return None,
    };

    // Walk backward through `.ident` pairs.
    while i >= 2 {
        if !matches!(&tokens[i - 1].0, oryn::Token::Dot) {
            break;
        }
        match &tokens[i - 2].0 {
            oryn::Token::Ident(name) => chain.push(name.clone()),
            _ => break,
        }
        i -= 2;
    }

    chain.reverse();
    Some(chain)
}

/// Look up a callable in the local symbol table, falling back to the
/// cross-file resolver if `chain` starts with an imported module name.
fn lookup_callable(
    chain: &[String],
    symbols: &SymbolTable,
    source: &str,
    file_path: Option<&Path>,
    tokens: &[Token],
    open_paren_idx: usize,
) -> Option<SymbolInfo> {
    let target_name = chain.last()?.clone();

    // Single-segment chain: bare function name, check local table.
    if chain.len() == 1
        && let Some(def) = symbols
            .definitions
            .iter()
            .find(|d| d.name == target_name && d.kind == SymbolKind::Function)
    {
        return Some(clone_def(def));
    }

    // Dotted chain: use the cross-file resolver. It expects a cursor
    // offset pointing at the target ident, which is the token just
    // before the open paren.
    let path = file_path?;
    if open_paren_idx == 0 {
        return None;
    }
    let ident_span = &tokens[open_paren_idx - 1].1;
    let resolved = resolver::resolve_cross_file(source, ident_span.start, path)?;
    let def = &resolved.module_table.definitions[resolved.def_idx];
    if def.kind != SymbolKind::Function {
        return None;
    }
    Some(clone_def(def))
}

/// Hand-rolled clone because `SymbolInfo` doesn't derive `Clone`.
/// Only the fields signature help needs are copied.
fn clone_def(def: &SymbolInfo) -> SymbolInfo {
    SymbolInfo {
        name: def.name.clone(),
        name_span: def.name_span.clone(),
        full_span: def.full_span.clone(),
        kind: def.kind.clone(),
        params: def.params.clone(),
        type_name: def.type_name.clone(),
        return_type: def.return_type.clone(),
        scope_depth: def.scope_depth,
    }
}

fn def_to_signature_info(def: &SymbolInfo) -> SignatureInformation {
    let params = def.params.clone().unwrap_or_default();
    let ret = match &def.return_type {
        Some(rt) => format!(" -> {rt}"),
        None => String::new(),
    };
    let label = format!("fn {}({}){}", def.name, params.join(", "), ret);

    let parameters: Vec<ParameterInformation> = params
        .iter()
        .map(|p| ParameterInformation {
            label: ParameterLabel::Simple(p.clone()),
            documentation: None,
        })
        .collect();

    SignatureInformation {
        label,
        documentation: None,
        parameters: Some(parameters),
        active_parameter: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze;
    use crate::diagnostics::offset_to_position;

    fn help_at(source: &str, needle: &str) -> Option<SignatureHelp> {
        let offset = source.find(needle).expect("needle not in source");
        let symbols = analyze(source);
        let pos = offset_to_position(source, offset);
        signature_help(source, pos, &symbols, None)
    }

    #[test]
    fn signature_help_for_local_function_call_first_arg() {
        let source = "fn add(a: i32, b: i32) -> i32 {\nrn a + b\n}\nlet x = add(1, 2)";
        // Cursor right after the `(` — should be active_param 0.
        let help = help_at(source, "1, 2").expect("expected signature help");
        assert_eq!(help.active_parameter, Some(0));
        assert_eq!(help.signatures.len(), 1);
        assert!(help.signatures[0].label.contains("fn add"));
        assert!(help.signatures[0].label.contains("a: i32"));
    }

    #[test]
    fn signature_help_active_param_advances_after_comma() {
        let source = "fn add(a: i32, b: i32) -> i32 {\nrn a + b\n}\nlet x = add(1, 2)";
        // Cursor at the `2` (after the comma) — active_param should be 1.
        let help = help_at(source, "2)").expect("expected signature help");
        assert_eq!(help.active_parameter, Some(1));
    }

    #[test]
    fn signature_help_returns_none_outside_any_call() {
        let source = "fn add(a: i32, b: i32) -> i32 {\nrn a + b\n}\nlet x = 1";
        assert!(help_at(source, "x = 1").is_none());
    }

    #[test]
    fn signature_help_skips_nested_call_commas() {
        // The inner `add(1, 2)` has a comma at depth 1 which should
        // NOT count toward the outer `outer(…)` active parameter.
        let source = "
fn add(a: i32, b: i32) -> i32 { rn a + b }
fn outer(x: i32, y: i32) -> i32 { rn x + y }
let z = outer(add(1, 2), 3)
";
        let help = help_at(source, "3)").expect("expected signature help");
        assert_eq!(help.active_parameter, Some(1));
        assert!(help.signatures[0].label.contains("fn outer"));
    }

    /// End-to-end: signature help for a cross-file static method call
    /// on the playground. Cursor inside `math.colors.Color.rgb(255, 0, 0)`
    /// should return `rgb`'s signature with the right active param.
    #[test]
    fn signature_help_for_cross_file_static_method() {
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

        // Cursor just after `rgb(` — active_param 0.
        let needle = "rgb(255";
        let offset = source.find(needle).expect("needle not in main.on") + "rgb(".len();
        let pos = offset_to_position(&source, offset);

        let help = signature_help(&source, pos, &symbols, Some(&main_path))
            .expect("expected signature help");
        assert!(
            help.signatures[0].label.contains("fn rgb"),
            "got: {}",
            help.signatures[0].label
        );
        assert_eq!(help.active_parameter, Some(0));
    }
}
