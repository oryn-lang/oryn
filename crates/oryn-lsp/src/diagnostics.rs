use std::path::PathBuf;

use lsp_server::{Connection, Message, Notification as ServerNotification};
use lsp_types::notification::{Notification, PublishDiagnostics};
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, PublishDiagnosticsParams, Range, Uri};

/// Runs the full compile pipeline on `source` and sends any errors to the
/// client as diagnostics. An empty error list clears previous diagnostics.
///
/// When the document URI points to a file on disk, we use [`oryn::Chunk::check_file`]
/// so the project's `package.on` is found and `import` statements resolve
/// against sibling files. This is what makes cross-module references like
/// `math.vec2.Vec2` show up as valid in the LSP. When the URI isn't a
/// file URI (untitled buffers, etc.) we fall back to single-file checking.
pub fn publish_diagnostics(connection: &Connection, uri: Uri, source: &str) {
    let errors = match uri_to_path(&uri) {
        Some(path) => oryn::Chunk::check_file(&path, source),
        None => oryn::Chunk::check(source),
    };

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
                oryn::OrynError::Module { .. } => return None,
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

/// Convert an `lsp_types::Uri` to a filesystem path. Returns `None` for
/// non-file URIs (e.g. `untitled:` scratch buffers) so callers can fall
/// back to single-file analysis.
pub(crate) fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let s = uri.as_str();
    let path = s.strip_prefix("file://")?;
    // URL-decode percent escapes — most editors send `%20` for spaces, etc.
    let decoded = percent_decode(path);
    Some(PathBuf::from(decoded))
}

/// Minimal percent-decoder for file URIs. Handles `%XX` byte escapes;
/// invalid escapes pass through unchanged. We don't pull in a full URL
/// crate because the LSP only ever encodes spaces and a handful of
/// other ASCII punctuation in real-world editor URIs.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
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
