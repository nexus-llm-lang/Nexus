use std::collections::HashMap;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{
    Completion, DocumentSymbolRequest, GotoDefinition, HoverRequest, References,
    Rename, Request as _,
};
use lsp_types::*;

use super::analysis;
use super::completion;
use super::hover;
use super::position::LineIndex;
use super::references;
use super::symbols;

use crate::lang::ast::Program;
use crate::lang::typecheck::TypeEnv;

struct DocumentState {
    text: String,
    line_index: LineIndex,
    env: Option<TypeEnv>,
    program: Option<Program>,
}

/// Key for document storage — wraps the URI string for HashMap lookup.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DocKey(String);

impl DocKey {
    fn from_uri(uri: &Uri) -> Self {
        DocKey(uri.as_str().to_string())
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (connection, io_threads) = Connection::stdio();

    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::FULL,
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: Default::default(),
        })),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    })?;

    let init_params = match connection.initialize(server_capabilities) {
        Ok(params) => params,
        Err(e) => {
            if e.channel_is_disconnected() {
                io_threads.join()?;
            }
            return Err(e.into());
        }
    };

    let _init: InitializeParams = serde_json::from_value(init_params)?;

    main_loop(&connection)?;
    io_threads.join()?;
    Ok(())
}

fn main_loop(
    connection: &Connection,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut documents: HashMap<DocKey, DocumentState> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                let resp = handle_request(&req, &documents);
                connection.sender.send(Message::Response(resp))?;
            }
            Message::Notification(not) => {
                handle_notification(connection, &mut documents, not)?;
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn handle_notification(
    connection: &Connection,
    documents: &mut HashMap<DocKey, DocumentState>,
    not: Notification,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams = serde_json::from_value(not.params)?;
            let uri = params.text_document.uri;
            let text = params.text_document.text;
            update_document(connection, documents, &uri, text)?;
        }
        DidChangeTextDocument::METHOD => {
            let params: DidChangeTextDocumentParams = serde_json::from_value(not.params)?;
            let uri = params.text_document.uri;
            if let Some(change) = params.content_changes.into_iter().last() {
                update_document(connection, documents, &uri, change.text)?;
            }
        }
        DidCloseTextDocument::METHOD => {
            let params: DidCloseTextDocumentParams = serde_json::from_value(not.params)?;
            let uri = &params.text_document.uri;
            documents.remove(&DocKey::from_uri(uri));
            let diag_params = PublishDiagnosticsParams {
                uri: params.text_document.uri,
                diagnostics: vec![],
                version: None,
            };
            connection.sender.send(Message::Notification(Notification {
                method: PublishDiagnostics::METHOD.to_string(),
                params: serde_json::to_value(diag_params)?,
            }))?;
        }
        _ => {}
    }
    Ok(())
}

fn update_document(
    connection: &Connection,
    documents: &mut HashMap<DocKey, DocumentState>,
    uri: &Uri,
    text: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let filename = uri_to_filename(uri);
    let result = analysis::analyze(&filename, &text);

    let lsp_diags: Vec<Diagnostic> = result
        .check
        .diagnostics
        .iter()
        .map(analysis::to_lsp_diagnostic)
        .collect();
    let diag_params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics: lsp_diags,
        version: None,
    };
    connection.sender.send(Message::Notification(Notification {
        method: PublishDiagnostics::METHOD.to_string(),
        params: serde_json::to_value(diag_params)?,
    }))?;

    documents.insert(
        DocKey::from_uri(uri),
        DocumentState {
            text,
            line_index: result.line_index,
            env: result.env,
            program: result.program,
        },
    );

    Ok(())
}

fn handle_request(req: &Request, documents: &HashMap<DocKey, DocumentState>) -> Response {
    match req.method.as_str() {
        HoverRequest::METHOD => {
            let params: HoverParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return error_response(req.id.clone(), e),
            };
            let uri = &params.text_document_position_params.text_document.uri;
            let pos = params.text_document_position_params.position;
            let key = DocKey::from_uri(uri);
            let result = documents.get(&key).and_then(|doc| {
                let offset = doc.line_index.position_to_offset(pos);
                let env = doc.env.as_ref()?;
                let program = doc.program.as_ref()?;
                let info = hover::hover_at(&doc.text, offset, program, env)?;
                Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: info,
                    }),
                    range: None,
                })
            });
            Response {
                id: req.id.clone(),
                result: Some(serde_json::to_value(result).unwrap()),
                error: None,
            }
        }
        DocumentSymbolRequest::METHOD => {
            let params: DocumentSymbolParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return error_response(req.id.clone(), e),
            };
            let key = DocKey::from_uri(&params.text_document.uri);
            let result = documents.get(&key).and_then(|doc| {
                let program = doc.program.as_ref()?;
                let syms = symbols::extract(program, &doc.line_index);
                Some(DocumentSymbolResponse::Nested(syms))
            });
            Response {
                id: req.id.clone(),
                result: Some(serde_json::to_value(result).unwrap()),
                error: None,
            }
        }
        GotoDefinition::METHOD => {
            let params: GotoDefinitionParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return error_response(req.id.clone(), e),
            };
            let uri = &params.text_document_position_params.text_document.uri;
            let pos = params.text_document_position_params.position;
            let key = DocKey::from_uri(uri);
            let result = documents.get(&key).and_then(|doc| {
                let program = doc.program.as_ref()?;
                let offset = doc.line_index.position_to_offset(pos);
                let span = hover::find_definition(program, &doc.text, offset)?;
                let range = doc.line_index.span_to_range(&span);
                Some(GotoDefinitionResponse::Scalar(Location {
                    uri: uri.clone(),
                    range,
                }))
            });
            Response {
                id: req.id.clone(),
                result: Some(serde_json::to_value(result).unwrap()),
                error: None,
            }
        }
        References::METHOD => {
            let params: ReferenceParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return error_response(req.id.clone(), e),
            };
            let uri = &params.text_document_position.text_document.uri;
            let pos = params.text_document_position.position;
            let key = DocKey::from_uri(uri);
            let result = documents.get(&key).and_then(|doc| {
                let offset = doc.line_index.position_to_offset(pos);
                let (_, _, spans) = references::find_all_references(&doc.text, offset)?;
                let locs: Vec<Location> = spans
                    .into_iter()
                    .map(|span| Location {
                        uri: uri.clone(),
                        range: doc.line_index.span_to_range(&span),
                    })
                    .collect();
                Some(locs)
            });
            Response {
                id: req.id.clone(),
                result: Some(serde_json::to_value(result).unwrap()),
                error: None,
            }
        }
        Rename::METHOD => {
            let params: RenameParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return error_response(req.id.clone(), e),
            };
            let uri = &params.text_document_position.text_document.uri;
            let pos = params.text_document_position.position;
            let new_name = params.new_name;
            let key = DocKey::from_uri(uri);
            let result = documents.get(&key).and_then(|doc| {
                let offset = doc.line_index.position_to_offset(pos);
                let (_, _, spans) = references::find_all_references(&doc.text, offset)?;
                let edits: Vec<TextEdit> = spans
                    .into_iter()
                    .map(|span| TextEdit {
                        range: doc.line_index.span_to_range(&span),
                        new_text: new_name.clone(),
                    })
                    .collect();
                let mut changes = HashMap::new();
                changes.insert(uri.clone(), edits);
                Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                })
            });
            Response {
                id: req.id.clone(),
                result: Some(serde_json::to_value(result).unwrap()),
                error: None,
            }
        }
        "textDocument/prepareRename" => {
            let params: TextDocumentPositionParams =
                match serde_json::from_value(req.params.clone()) {
                    Ok(p) => p,
                    Err(e) => return error_response(req.id.clone(), e),
                };
            let uri = &params.text_document.uri;
            let pos = params.position;
            let key = DocKey::from_uri(uri);
            let result = documents.get(&key).and_then(|doc| {
                let offset = doc.line_index.position_to_offset(pos);
                let (name, origin_span, _) =
                    references::find_all_references(&doc.text, offset)?;
                Some(PrepareRenameResponse::RangeWithPlaceholder {
                    range: doc.line_index.span_to_range(&origin_span),
                    placeholder: name,
                })
            });
            Response {
                id: req.id.clone(),
                result: Some(serde_json::to_value(result).unwrap()),
                error: None,
            }
        }
        Completion::METHOD => {
            let params: CompletionParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return error_response(req.id.clone(), e),
            };
            let uri = &params.text_document_position.text_document.uri;
            let key = DocKey::from_uri(uri);
            let items = documents
                .get(&key)
                .map(|doc| completion::completions(doc.env.as_ref()))
                .unwrap_or_default();
            let result = CompletionResponse::Array(items);
            Response {
                id: req.id.clone(),
                result: Some(serde_json::to_value(result).unwrap()),
                error: None,
            }
        }
        _ => Response {
            id: req.id.clone(),
            result: None,
            error: Some(lsp_server::ResponseError {
                code: lsp_server::ErrorCode::MethodNotFound as i32,
                message: format!("method not found: {}", req.method),
                data: None,
            }),
        },
    }
}

fn error_response(id: RequestId, err: impl std::fmt::Display) -> Response {
    Response {
        id,
        result: None,
        error: Some(lsp_server::ResponseError {
            code: lsp_server::ErrorCode::InvalidParams as i32,
            message: err.to_string(),
            data: None,
        }),
    }
}

fn uri_to_filename(uri: &Uri) -> String {
    let s = uri.as_str();
    if let Some(path) = s.strip_prefix("file://") {
        // URL-decode percent-encoded characters
        percent_decode(path)
    } else {
        s.to_string()
    }
}

fn percent_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().and_then(|c| hex_val(c));
            let lo = chars.next().and_then(|c| hex_val(c));
            if let (Some(h), Some(l)) = (hi, lo) {
                result.push((h << 4 | l) as char);
            }
        } else {
            result.push(b as char);
        }
    }
    result
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
