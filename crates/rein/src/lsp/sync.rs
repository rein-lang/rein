//! Document sync: keeps the open buffers current, then re-checks after each change.

use std::collections::BTreeSet;

use lsp_server::{Connection, Notification};
use lsp_types::{
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, Url,
};

use super::diagnostics::refresh;
use super::workspace::Docs;
use super::BoxResult;

pub fn handle_notification(
    connection: &Connection,
    docs: &mut Docs,
    published: &mut BTreeSet<Url>,
    note: Notification,
) -> BoxResult<()> {
    match note.method.as_str() {
        "textDocument/didOpen" => {
            let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(note.params) else {
                return Ok(());
            };
            let uri = p.text_document.uri;
            docs.insert(uri.clone(), p.text_document.text);
            refresh(connection, docs, published, &uri)
        }
        "textDocument/didChange" => {
            let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(note.params) else {
                return Ok(());
            };
            let uri = p.text_document.uri;
            // Full sync: the last change holds the whole text.
            if let Some(change) = p.content_changes.into_iter().last() {
                docs.insert(uri.clone(), change.text);
            }
            refresh(connection, docs, published, &uri)
        }
        "textDocument/didSave" => {
            let Ok(p) = serde_json::from_value::<DidSaveTextDocumentParams>(note.params) else {
                return Ok(());
            };
            refresh(connection, docs, published, &p.text_document.uri)
        }
        "textDocument/didClose" => {
            let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(note.params) else {
                return Ok(());
            };
            docs.remove(&p.text_document.uri);
            refresh(connection, docs, published, &p.text_document.uri)
        }
        "workspace/didChangeWatchedFiles" => {
            let Ok(p) = serde_json::from_value::<DidChangeWatchedFilesParams>(note.params) else {
                return Ok(());
            };
            for event in p.changes {
                refresh(connection, docs, published, &event.uri)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
