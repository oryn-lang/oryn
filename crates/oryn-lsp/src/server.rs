use std::collections::HashMap;

use lsp_server::{Connection, Message, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification,
};
use lsp_types::request::{
    DocumentSymbolRequest, GotoDefinition, HoverRequest, References, Request,
};
use lsp_types::{
    HoverProviderCapability, OneOf, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind,
};
use tracing::{debug, error, info};

use crate::analysis::SymbolTable;
use crate::{definition, diagnostics, hover, references, symbols};

/// Per-document state: source text and cached symbol table.
struct Document {
    source: String,
    symbols: SymbolTable,
}

impl Document {
    fn new(source: String) -> Self {
        let symbols = crate::analysis::analyze(&source);
        Self { source, symbols }
    }

    fn update(&mut self, source: String) {
        self.symbols = crate::analysis::analyze(&source);
        self.source = source;
    }
}

pub fn run() {
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
        document_symbol_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
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

fn main_loop(connection: &Connection) {
    let mut documents: HashMap<String, Document> = HashMap::new();

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

                    let result = documents
                        .get(uri)
                        .and_then(|doc| hover::hover(&doc.source, pos, &doc.symbols));

                    let resp = Response::new_ok(req.id, &result);
                    let _ = connection.sender.send(Message::Response(resp));
                } else if req.method == <DocumentSymbolRequest as Request>::METHOD {
                    debug!("document symbol request");

                    let params: lsp_types::DocumentSymbolParams =
                        match serde_json::from_value(req.params.clone()) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("failed to parse document symbol params: {e}");
                                continue;
                            }
                        };

                    let uri = params.text_document.uri.as_str();

                    let result = documents.get(uri).map(|doc| {
                        lsp_types::DocumentSymbolResponse::Nested(symbols::document_symbols(
                            &doc.source,
                            &doc.symbols,
                        ))
                    });

                    let resp = Response::new_ok(req.id, &result);
                    let _ = connection.sender.send(Message::Response(resp));
                } else if req.method == <GotoDefinition as Request>::METHOD {
                    debug!("goto definition request");

                    let params: lsp_types::GotoDefinitionParams =
                        match serde_json::from_value(req.params.clone()) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("failed to parse goto definition params: {e}");
                                continue;
                            }
                        };

                    let uri_str = params
                        .text_document_position_params
                        .text_document
                        .uri
                        .as_str()
                        .to_owned();
                    let uri = params.text_document_position_params.text_document.uri;
                    let pos = params.text_document_position_params.position;

                    let file_path = crate::diagnostics::uri_to_path(&uri);
                    let result = documents.get(&uri_str).and_then(|doc| {
                        definition::goto_definition(
                            &doc.source,
                            pos,
                            &uri,
                            &doc.symbols,
                            file_path.as_deref(),
                        )
                    });

                    let resp = Response::new_ok(req.id, &result);
                    let _ = connection.sender.send(Message::Response(resp));
                } else if req.method == <References as Request>::METHOD {
                    debug!("references request");

                    let params: lsp_types::ReferenceParams =
                        match serde_json::from_value(req.params.clone()) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("failed to parse references params: {e}");
                                continue;
                            }
                        };

                    let uri_str = params
                        .text_document_position
                        .text_document
                        .uri
                        .as_str()
                        .to_owned();
                    let uri = params.text_document_position.text_document.uri;
                    let pos = params.text_document_position.position;
                    let include_declaration = params.context.include_declaration;

                    let result = documents.get(&uri_str).map(|doc| {
                        references::find_references(
                            &doc.source,
                            pos,
                            &uri,
                            include_declaration,
                            &doc.symbols,
                        )
                    });

                    let resp = Response::new_ok(req.id, &result);
                    let _ = connection.sender.send(Message::Response(resp));
                }
            }
            Message::Notification(note) => match note.method.as_str() {
                <DidOpenTextDocument as Notification>::METHOD => {
                    let params: lsp_types::DidOpenTextDocumentParams =
                        match serde_json::from_value(note.params) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("failed to parse didOpen params: {e}");
                                continue;
                            }
                        };
                    let uri = params.text_document.uri;
                    let source = params.text_document.text;

                    diagnostics::publish_diagnostics(connection, uri.clone(), &source);
                    documents.insert(uri.as_str().to_owned(), Document::new(source));
                }
                <DidChangeTextDocument as Notification>::METHOD => {
                    let params: lsp_types::DidChangeTextDocumentParams =
                        match serde_json::from_value(note.params) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("failed to parse didChange params: {e}");
                                continue;
                            }
                        };
                    let uri = params.text_document.uri;

                    if let Some(change) = params.content_changes.into_iter().next() {
                        diagnostics::publish_diagnostics(connection, uri.clone(), &change.text);

                        if let Some(doc) = documents.get_mut(uri.as_str()) {
                            doc.update(change.text);
                        } else {
                            documents.insert(uri.as_str().to_owned(), Document::new(change.text));
                        }
                    }
                }
                <DidCloseTextDocument as Notification>::METHOD => {
                    let params: lsp_types::DidCloseTextDocumentParams =
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
            },
            Message::Response(_) => {}
        }
    }
}
