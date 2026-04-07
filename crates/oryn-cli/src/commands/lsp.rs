use std::collections::HashMap;

use lsp_server::{Connection, Message, Notification as ServerNotification, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification,
    PublishDiagnostics,
};
use lsp_types::request::{HoverRequest, Request};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, Hover, HoverContents, HoverProviderCapability, MarkedString,
    Position, PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, Uri,
};
use tracing::{debug, error, info};

pub fn run() {
    // LSP communicates over stdio, so tracing logs go to a file via
    // ORYN_LOG env var (e.g. ORYN_LOG=debug).
    let log_file = std::fs::File::create("/tmp/oryn-lsp.log").ok();

    if let Some(file) = log_file {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_env("ORYN_LOG")
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .with_writer(file)
            .with_ansi(false)
            .init();
    }

    info!("starting oryn lsp");

    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        ..Default::default()
    };

    let server_capabilities = match serde_json::to_value(capabilities) {
        Ok(v) => v,
        Err(e) => {
            error!("failed to serialize capabilities: {e}");

            return;
        }
    };

    if let Err(e) = connection.initialize(server_capabilities) {
        error!("initialization failed: {e}");

        return;
    }

    info!("initialized");

    main_loop(&connection);

    if let Err(e) = io_threads.join() {
        error!("io thread error: {e}");
    }

    info!("shut down");
}

// Keeps a copy of each open document's source text so we can re-analyze
// on every change without hitting the filesystem.
fn main_loop(connection: &Connection) {
    // Keyed by URI string rather than `Uri` directly because `Uri` has
    // interior mutability, which clippy flags for HashMap keys.
    let mut documents: HashMap<String, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                match connection.handle_shutdown(&req) {
                    Ok(true) => {
                        info!("shutdown requested");

                        break;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        error!("shutdown error: {e}");

                        break;
                    }
                }

                if req.method == <HoverRequest as Request>::METHOD {
                    debug!("hover request");

                    let params: lsp_types::HoverParams =
                        match serde_json::from_value(req.params.clone()) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("failed to parse hover params: {e}");
                                continue;
                            }
                        };
                    let uri = params
                        .text_document_position_params
                        .text_document
                        .uri
                        .as_str();
                    let pos = params.text_document_position_params.position;

                    let result = documents.get(uri).and_then(|source| hover(source, pos));

                    let resp = Response::new_ok(req.id, &result);
                    let _ = connection.sender.send(Message::Response(resp));
                }
            }
            Message::Notification(note) => {
                match note.method.as_str() {
                    <DidOpenTextDocument as Notification>::METHOD => {
                        let params: DidOpenTextDocumentParams =
                            match serde_json::from_value(note.params) {
                                Ok(p) => p,
                                Err(e) => {
                                    error!("failed to parse didOpen params: {e}");

                                    continue;
                                }
                            };
                        let uri = params.text_document.uri;
                        documents
                            .insert(uri.as_str().to_owned(), params.text_document.text.clone());
                        publish_diagnostics(connection, uri, &params.text_document.text);
                    }
                    <DidChangeTextDocument as Notification>::METHOD => {
                        let params: DidChangeTextDocumentParams =
                            match serde_json::from_value(note.params) {
                                Ok(p) => p,
                                Err(e) => {
                                    error!("failed to parse didChange params: {e}");

                                    continue;
                                }
                            };
                        let uri = params.text_document.uri;

                        // We use `TextDocumentSyncKind::FULL`, so the first
                        // content change always contains the entire document.
                        if let Some(change) = params.content_changes.into_iter().next() {
                            documents.insert(uri.as_str().to_owned(), change.text.clone());
                            publish_diagnostics(connection, uri, &change.text);
                        }
                    }
                    <DidCloseTextDocument as Notification>::METHOD => {
                        let params: DidCloseTextDocumentParams =
                            match serde_json::from_value(note.params) {
                                Ok(p) => p,
                                Err(e) => {
                                    error!("failed to parse didClose params: {e}");

                                    continue;
                                }
                            };
                        documents.remove(params.text_document.uri.as_str());
                    }
                    _ => {}
                }
            }
            Message::Response(_) => {}
        }
    }
}

// Runs the lexer and parser on `source` and sends any errors to the
// client as diagnostics. Passing an empty vec clears previous diagnostics
// when the user fixes all errors.
fn publish_diagnostics(connection: &Connection, uri: Uri, source: &str) {
    let errors = oryn::Chunk::check(source);

    let diagnostics: Vec<Diagnostic> = errors
        .iter()
        .filter_map(|error| {
            let (span, message) = match error {
                oryn::OrynError::Lexer { span } => {
                    (span.clone(), "unexpected character".to_string())
                }
                oryn::OrynError::Parser { span, message } => (span.clone(), message.clone()),
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

// Finds the token under the cursor and returns a description. Since we
// only have `Int` types right now, this is pretty basic, but it'll grow
// as the language does.
fn hover(source: &str, pos: Position) -> Option<Hover> {
    let offset = position_to_offset(source, pos)?;
    let (tokens, _) = oryn::lex(source);

    let (token, span) = tokens
        .into_iter()
        .find(|(_, span)| offset >= span.start && offset < span.end)?;

    let contents = match &token {
        oryn::Token::Int(n) => format!("`{n}`: Int literal"),
        oryn::Token::Ident(name) => format!("`{name}`: identifier"),
        oryn::Token::Let => "`let`: variable binding".to_string(),
        oryn::Token::Plus => "`+`: addition operator".to_string(),
        oryn::Token::Minus => "`-`: subtraction operator".to_string(),
        oryn::Token::Multiply => "`*`: multiplication operator".to_string(),
        oryn::Token::Divide => "`/`: division operator".to_string(),
        oryn::Token::Equals => "`=`: assignment operator".to_string(),
        _ => return None,
    };

    Some(Hover {
        contents: HoverContents::Scalar(MarkedString::String(contents)),
        range: Some(span_to_range(source, span)),
    })
}

// Converts a byte offset range into an LSP `Range` (line/column pairs).
// LSP positions are 0-indexed lines and UTF-16 code unit offsets, but
// since oryn only has ASCII we can treat bytes as columns.
fn span_to_range(source: &str, span: std::ops::Range<usize>) -> Range {
    let start = offset_to_position(source, span.start);
    let end = offset_to_position(source, span.end);

    Range { start, end }
}

fn offset_to_position(source: &str, offset: usize) -> Position {
    let before = &source[..offset.min(source.len())];
    let line = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let character = (offset - line_start) as u32;

    Position { line, character }
}

fn position_to_offset(source: &str, pos: Position) -> Option<usize> {
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
