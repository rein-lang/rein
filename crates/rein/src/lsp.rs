//! Language server over stdio. This file runs the message loop and sends each request to the module of
//! its feature.
//!
//! Every request rebuilds the project from disk plus the open buffers. That is slow on big
//! projects, which is fine for the proof of concept.

mod call_context;
mod call_hierarchy;
mod code_actions;
mod code_lens;
mod completion;
mod definition;
mod deprecation;
mod deprecation_hover;
mod diagnostics;
mod document_symbols;
mod hover;
mod import_completion;
mod occurrences;
mod references;
mod rename;
mod signature_help;
mod structs;
mod sync;
mod text;
mod workspace;
mod workspace_symbols;

use std::collections::BTreeSet;
use std::error::Error;

use lsp_server::{Connection, ErrorCode, Message, Request, Response};
use lsp_types::{
    CallHierarchyServerCapability, CodeActionProviderCapability, CodeLensOptions, CompletionOptions, HoverProviderCapability, InitializeParams,
    OneOf, RenameOptions, ServerCapabilities, SignatureHelpOptions, TextDocumentSyncCapability,
    TextDocumentSyncKind,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

use workspace::State;

type BoxResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub fn serve() -> u8 {
    match run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: language server: {e}");
            1
        }
    }
}

fn run() -> BoxResult<()> {
    let (connection, io_threads) = Connection::stdio();
    let init = connection.initialize(serde_json::to_value(capabilities())?)?;
    let init: InitializeParams = serde_json::from_value(init).unwrap_or_default();
    main_loop(&connection, &mut State::new(&init))?;
    // The writer thread ends only when every sender is gone, so drop the connection first.
    drop(connection);
    io_threads.join()?;
    Ok(())
}

fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        code_lens_provider: Some(CodeLensOptions { resolve_provider: Some(false) }),
        call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: Default::default(),
        })),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(
                [" ", "\"", "'", "/", ".", "(", ","].into_iter().map(String::from).collect(),
            ),
            ..Default::default()
        }),
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".into(), ",".into()]),
            retrigger_characters: Some(vec!["=".into()]),
            work_done_progress_options: Default::default(),
        }),
        ..Default::default()
    }
}

fn main_loop(connection: &Connection, state: &mut State) -> BoxResult<()> {
    // Every URI that has a diagnostics list from us. A file that leaves its project gets an empty one.
    let mut published = BTreeSet::new();
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                connection.sender.send(handle_request(state, req).into())?;
            }
            Message::Notification(note) => {
                if note.method == "exit" {
                    return Ok(());
                }
                sync::handle_notification(connection, &mut state.docs, &mut published, note)?;
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn handle_request(state: &State, req: Request) -> Response {
    let method = req.method.clone();
    match method.as_str() {
        "textDocument/hover" => reply(req, |p| Ok(hover::hover(state, &p))),
        "textDocument/definition" => reply(req, |p| Ok(definition::definition(state, &p))),
        "textDocument/completion" => reply(req, |p| Ok(completion::completion(state, &p))),
        "textDocument/references" => reply(req, |p| Ok(references::references(state, &p))),
        "textDocument/codeLens" => reply(req, |p| Ok(code_lens::code_lens(state, &p))),
        "textDocument/prepareCallHierarchy" => reply(req, |p| Ok(call_hierarchy::prepare(state, &p))),
        "callHierarchy/incomingCalls" => reply(req, |p| Ok(call_hierarchy::incoming(state, &p))),
        "callHierarchy/outgoingCalls" => reply(req, |p| Ok(call_hierarchy::outgoing(state, &p))),
        "textDocument/documentSymbol" => reply(req, |p| Ok(document_symbols::document_symbols(state, &p))),
        "workspace/symbol" => reply(req, |p| Ok(workspace_symbols::workspace_symbols(state, &p))),
        "textDocument/codeAction" => reply(req, |p| Ok(code_actions::code_actions(state, &p))),
        "textDocument/signatureHelp" => reply(req, |p| Ok(signature_help::signature_help(state, &p))),
        "textDocument/prepareRename" => reply(req, |p| rename::prepare(state, &p)),
        "textDocument/rename" => reply(req, |p| rename::rename(state, &p)),
        other => Response::new_err(req.id, ErrorCode::MethodNotFound as i32, format!("method not found: {other}")),
    }
}

/// Parses the params of `req`, runs `f`, and wraps its answer. An `Err` is a failed request with that message.
fn reply<P: DeserializeOwned, R: Serialize>(req: Request, f: impl FnOnce(P) -> Result<Option<R>, String>) -> Response {
    match serde_json::from_value::<P>(req.params) {
        Ok(p) => match f(p) {
            Ok(result) => Response::new_ok(req.id, result),
            Err(message) => Response::new_err(req.id, ErrorCode::RequestFailed as i32, message),
        },
        Err(e) => Response::new_err(req.id, ErrorCode::InvalidParams as i32, e.to_string()),
    }
}
