//! Inlay type hints. Renders the dim `: Color` that editors show
//! next to `let red =` when the user didn't write an explicit
//! annotation. Re-uses the [`oryn::TypeMap`] populated during
//! compilation — no extra type inference happens here.

use lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, Range};

use crate::analysis::{SymbolKind, SymbolTable};
use crate::diagnostics::offset_to_position;

/// Build inlay type hints for every unannotated `let`/`val` binding
/// whose type the compiler could infer. Filters out hints outside the
/// editor viewport (the `range` param) so we never send more data
/// than the editor will render.
pub fn inlay_hints(
    source: &str,
    viewport: &Range,
    symbols: &SymbolTable,
    types: &oryn::TypeMap,
) -> Vec<InlayHint> {
    let mut out = Vec::new();

    for def in &symbols.definitions {
        // Only emit for unannotated let/val bindings where the compiler
        // successfully inferred a type.
        if def.kind != SymbolKind::Variable || def.type_name.is_some() {
            continue;
        }
        let Some(ty) = types.get(&def.full_span) else {
            continue;
        };

        // Anchor at the end of the variable name: `let red▸: Color`.
        let pos = offset_to_position(source, def.name_span.end);
        if pos < viewport.start || pos > viewport.end {
            continue;
        }

        out.push(InlayHint {
            position: pos,
            label: InlayHintLabel::String(format!(": {ty}")),
            kind: Some(InlayHintKind::TYPE),
            text_edits: None,
            tooltip: None,
            padding_left: Some(false),
            padding_right: Some(false),
            data: None,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze;
    use lsp_types::Position;

    fn full_viewport() -> Range {
        Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: u32::MAX,
                character: u32::MAX,
            },
        }
    }

    fn hints_for(source: &str) -> Vec<InlayHint> {
        let symbols = analyze(source);
        let (_, types) = oryn::Chunk::check_with_types(source);
        inlay_hints(source, &full_viewport(), &symbols, &types)
    }

    #[test]
    fn inlay_hint_for_unannotated_let() {
        let hints = hints_for("let x = 5");
        assert_eq!(hints.len(), 1);
        match &hints[0].label {
            InlayHintLabel::String(s) => assert_eq!(s, ": int"),
            other => panic!("unexpected label: {other:?}"),
        }
        assert_eq!(hints[0].kind, Some(InlayHintKind::TYPE));
    }

    #[test]
    fn no_inlay_hint_for_annotated_let() {
        let hints = hints_for("let x: int = 5");
        assert!(hints.is_empty(), "got: {hints:?}");
    }

    #[test]
    fn inlay_hint_for_obj_literal() {
        let source = "struct Point {\nx: int\ny: int\n}\nlet p = Point { x: 1, y: 2 }";
        let hints = hints_for(source);
        assert_eq!(hints.len(), 1);
        match &hints[0].label {
            InlayHintLabel::String(s) => assert_eq!(s, ": Point"),
            other => panic!("unexpected label: {other:?}"),
        }
    }

    #[test]
    fn inlay_hint_for_inferred_list_type() {
        let hints = hints_for("let xs = [1, 2, 3]");
        assert_eq!(hints.len(), 1);
        match &hints[0].label {
            InlayHintLabel::String(s) => assert_eq!(s, ": [int]"),
            other => panic!("unexpected label: {other:?}"),
        }
    }

    #[test]
    fn inlay_hint_for_inferred_nested_list_type() {
        let hints = hints_for("let grid = [[1, 2], [3, 4]]");
        assert_eq!(hints.len(), 1);
        match &hints[0].label {
            InlayHintLabel::String(s) => assert_eq!(s, ": [[int]]"),
            other => panic!("unexpected label: {other:?}"),
        }
    }

    #[test]
    fn inlay_hint_for_list_of_strings() {
        let hints = hints_for("let names = [\"a\", \"b\"]");
        assert_eq!(hints.len(), 1);
        match &hints[0].label {
            InlayHintLabel::String(s) => assert_eq!(s, ": [string]"),
            other => panic!("unexpected label: {other:?}"),
        }
    }

    #[test]
    fn inlay_hint_for_inferred_map_type() {
        let hints = hints_for("let stats = {\"hp\": 10}");
        assert_eq!(hints.len(), 1);
        match &hints[0].label {
            InlayHintLabel::String(s) => assert_eq!(s, ": {string: int}"),
            other => panic!("unexpected label: {other:?}"),
        }
    }

    #[test]
    fn inlay_hint_hidden_outside_viewport() {
        let source = "let x = 1\nlet y = 2\nlet z = 3";
        let symbols = analyze(source);
        let (_, types) = oryn::Chunk::check_with_types(source);

        // Only line 1 (the `let y = 2` line) is in the viewport.
        let viewport = Range {
            start: Position {
                line: 1,
                character: 0,
            },
            end: Position {
                line: 1,
                character: 100,
            },
        };
        let hints = inlay_hints(source, &viewport, &symbols, &types);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].position.line, 1);
    }
}
