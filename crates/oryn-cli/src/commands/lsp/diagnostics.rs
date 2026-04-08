use lsp_server::{Connection, Message, Notification as ServerNotification};
use lsp_types::notification::{Notification, PublishDiagnostics};
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, PublishDiagnosticsParams, Range, Uri};

/// Runs the lexer and parser on `source` and sends any errors to the
/// client as diagnostics. An empty error list clears previous diagnostics.
pub fn publish_diagnostics(connection: &Connection, uri: Uri, source: &str) {
    let errors = oryn::Chunk::check(source);

    let diagnostics: Vec<Diagnostic> = errors
        .iter()
        .filter_map(|error| {
            let (span, message) = match error {
                oryn::OrynError::Lexer { span } => {
                    (span.clone(), "unexpected character".to_string())
                }
                oryn::OrynError::Parser { span, message } => (span.clone(), message.clone()),
                oryn::OrynError::Compiler { span, message } => (span.clone(), message.clone()),
                oryn::OrynError::Runtime(_) => return None,
            };
            Some(Diagnostic {
                range: span_to_range(source, span),
                severity: Some(DiagnosticSeverity::ERROR),
                message,
                ..Default::default()
            })
        })
        .collect();

    let params = PublishDiagnosticsParams {
        uri,
        diagnostics,
        version: None,
    };

    let notification = ServerNotification::new(
        <PublishDiagnostics as Notification>::METHOD.to_owned(),
        params,
    );

    let _ = connection.sender.send(Message::Notification(notification));
}

/// Converts a byte-offset range into an LSP Range (line/column pairs).
/// LSP positions are 0-indexed. Since Oryn only has ASCII, bytes = columns.
pub fn span_to_range(source: &str, span: std::ops::Range<usize>) -> Range {
    let start = offset_to_position(source, span.start);
    let end = offset_to_position(source, span.end);

    Range { start, end }
}

pub fn offset_to_position(source: &str, offset: usize) -> Position {
    let before = &source[..offset.min(source.len())];
    let line = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let character = (offset - line_start) as u32;

    Position { line, character }
}

pub fn position_to_offset(source: &str, pos: Position) -> Option<usize> {
    let mut current_line = 0u32;
    let mut line_start = 0usize;

    for (i, ch) in source.char_indices() {
        if current_line == pos.line {
            let offset = line_start + pos.character as usize;

            return Some(offset.min(source.len()));
        }

        if ch == '\n' {
            current_line += 1;
            line_start = i + 1;
        }
    }

    if current_line == pos.line {
        Some((line_start + pos.character as usize).min(source.len()))
    } else {
        None
    }
}
