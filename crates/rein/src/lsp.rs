//! Language server over stdio: diagnostics, hover, go to definition and completion.
//!
//! Every request rebuilds the project from disk plus the open buffers. That is slow on big
//! projects, which is fine for the proof of concept.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionResponse, Diagnostic,
    DiagnosticSeverity, DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    FileChangeType, Hover, HoverContents, HoverProviderCapability, Location, MarkupContent,
    MarkupKind, OneOf, Position, PublishDiagnosticsParams, Range, ServerCapabilities,
    TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use rein_core::ast::{Function, Input};
use rein_core::diagnostic::{Diagnostic as ReinDiagnostic, Severity, Span};
use rein_core::lexer::KEYWORDS;
use rein_core::project::Project;
use serde_json::Value;

use crate::cli::{description, type_label};

/// Open editor buffers by document URI. They replace the files on disk.
type Docs = HashMap<Url, String>;
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
    connection.initialize(serde_json::to_value(capabilities())?)?;
    main_loop(&connection)?;
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
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec!["@".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn main_loop(connection: &Connection) -> BoxResult<()> {
    let mut docs = Docs::new();
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                connection.sender.send(handle_request(&docs, req).into())?;
            }
            Message::Notification(note) => {
                if note.method == "exit" {
                    return Ok(());
                }
                handle_notification(connection, &mut docs, note)?;
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn handle_request(docs: &Docs, req: Request) -> Response {
    let handler: fn(&Docs, &TextDocumentPositionParams) -> Option<Value> = match req.method.as_str() {
        "textDocument/hover" => hover,
        "textDocument/definition" => definition,
        "textDocument/completion" => completion,
        other => {
            return Response::new_err(
                req.id,
                ErrorCode::MethodNotFound as i32,
                format!("method not found: {other}"),
            );
        }
    };
    match serde_json::from_value::<TextDocumentPositionParams>(req.params) {
        Ok(p) => Response::new_ok(req.id, handler(docs, &p).unwrap_or(Value::Null)),
        Err(e) => Response::new_err(req.id, ErrorCode::InvalidParams as i32, e.to_string()),
    }
}

fn handle_notification(connection: &Connection, docs: &mut Docs, note: Notification) -> BoxResult<()> {
    match note.method.as_str() {
        "textDocument/didOpen" => {
            let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(note.params) else {
                return Ok(());
            };
            let uri = p.text_document.uri;
            docs.insert(uri.clone(), p.text_document.text);
            publish_for(connection, docs, &uri)
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
            publish_for(connection, docs, &uri)
        }
        "textDocument/didSave" => {
            let Ok(p) = serde_json::from_value::<DidSaveTextDocumentParams>(note.params) else {
                return Ok(());
            };
            publish_for(connection, docs, &p.text_document.uri)
        }
        "textDocument/didClose" => {
            let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(note.params) else {
                return Ok(());
            };
            docs.remove(&p.text_document.uri);
            publish_for(connection, docs, &p.text_document.uri)
        }
        "workspace/didChangeWatchedFiles" => {
            let Ok(p) = serde_json::from_value::<DidChangeWatchedFilesParams>(note.params) else {
                return Ok(());
            };
            for event in p.changes {
                if event.typ == FileChangeType::DELETED {
                    send_diagnostics(connection, event.uri.clone(), Vec::new())?;
                }
                publish_for(connection, docs, &event.uri)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Re-checks the project that holds `uri`, then publishes diagnostics for every file in it.
/// Clean files get an empty list, so fixed errors clear.
fn publish_for(connection: &Connection, docs: &Docs, uri: &Url) -> BoxResult<()> {
    let Some(path) = doc_path(uri) else { return Ok(()) };
    let project = build_project(docs, &Project::find_root(&path));
    let diags = project.check();
    for (name, sf) in &project.files {
        let Some(file_uri) = uri_for(docs, &sf.path) else { continue };
        let list: Vec<Diagnostic> = diags
            .get(name)
            .map(|ds| ds.iter().map(|d| to_lsp(&sf.text, d)).collect())
            .unwrap_or_default();
        send_diagnostics(connection, file_uri, list)?;
    }
    Ok(())
}

fn send_diagnostics(connection: &Connection, uri: Url, diagnostics: Vec<Diagnostic>) -> BoxResult<()> {
    let params = PublishDiagnosticsParams { uri, diagnostics, version: None };
    let note = Notification::new("textDocument/publishDiagnostics".to_string(), params);
    connection.sender.send(Message::Notification(note))?;
    Ok(())
}

/// Project under `root`: files on disk, with open buffers replacing their disk copies.
fn build_project(docs: &Docs, root: &Path) -> Project {
    let mut by_path: BTreeMap<PathBuf, String> = BTreeMap::new();
    if let Ok(disk) = Project::load(root) {
        by_path.extend(disk.files.into_values().map(|sf| (sf.path, sf.text)));
    }
    by_path.extend(docs.iter().filter_map(|(uri, text)| Some((doc_path(uri)?, text.clone()))));
    Project::from_sources(root.to_path_buf(), by_path.into_iter().collect())
}

/// The project of `uri` and the function name of that file.
fn locate(docs: &Docs, uri: &Url) -> Option<(Project, String)> {
    let path = doc_path(uri)?;
    let project = build_project(docs, &Project::find_root(&path));
    let name = project.name_for(&path)?;
    project.get(&name)?;
    Some((project, name))
}

/// The URI the client uses for `path`. For an open document, that is the URI the client sent.
fn uri_for(docs: &Docs, path: &Path) -> Option<Url> {
    docs.keys()
        .find(|u| doc_path(u).as_deref() == Some(path))
        .cloned()
        .or_else(|| Url::from_file_path(path).ok())
}

fn doc_path(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok().map(canonical)
}

/// Resolves symlinks in the directory part, so a file that does not exist yet still matches
/// the canonical project root.
fn canonical(path: PathBuf) -> PathBuf {
    if let (Some(dir), Some(name)) = (path.parent(), path.file_name()) {
        if let Ok(dir) = fs::canonicalize(dir) {
            return dir.join(name);
        }
    }
    path
}

fn to_lsp(text: &str, d: &ReinDiagnostic) -> Diagnostic {
    Diagnostic {
        range: range(text, d.span),
        severity: Some(match d.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        source: Some("rein".to_string()),
        message: d.message.clone(),
        ..Default::default()
    }
}

fn range(text: &str, span: Span) -> Range {
    Range::new(position(text, span.start), position(text, span.end))
}

/// LSP position for a byte offset. The column counts UTF-16 units.
fn position(text: &str, offset: usize) -> Position {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line = text[..offset].matches('\n').count();
    let character = text[line_start..offset].encode_utf16().count();
    Position::new(line as u32, character as u32)
}

/// Byte offset for an LSP position. A column past the end of its line clamps to the line end.
fn offset(text: &str, pos: Position) -> usize {
    let mut line_start = 0;
    for _ in 0..pos.line {
        match text[line_start..].find('\n') {
            Some(i) => line_start += i + 1,
            None => return text.len(),
        }
    }
    let mut units = 0u32;
    for (i, c) in text[line_start..].char_indices() {
        if c == '\n' || units >= pos.character {
            return line_start + i;
        }
        units += c.len_utf16() as u32;
    }
    text.len()
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '@')
}

/// Start of the token run that ends at or covers `offset`.
fn word_start(text: &str, offset: usize) -> usize {
    text[..offset]
        .char_indices()
        .rev()
        .find(|&(_, c)| !is_token_char(c))
        .map_or(0, |(i, c)| i + c.len_utf8())
}

/// End of the token run that starts at or covers `offset`.
fn word_end(text: &str, offset: usize) -> usize {
    text[offset..]
        .char_indices()
        .find(|&(_, c)| !is_token_char(c))
        .map_or(text.len(), |(i, _)| offset + i)
}

/// A token under the cursor: `@ref`, or a plain name such as an input.
struct Token<'a> {
    text: &'a str,
}

/// The token at `offset`. Returns None inside a string literal.
fn token_at(text: &str, offset: usize) -> Option<Token<'_>> {
    let start = word_start(text, offset);
    let end = word_end(text, offset);
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let inside_string = text[line_start..start].matches('"').count() % 2 == 1;
    (start < end && !inside_string).then(|| Token { text: &text[start..end] })
}

fn hover(docs: &Docs, p: &TextDocumentPositionParams) -> Option<Value> {
    let (project, name) = locate(docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let token = token_at(&sf.text, offset(&sf.text, p.position))?;
    let value = match token.text.strip_prefix('@') {
        Some(target) => match project.get(target) {
            Some(t) => ref_markdown(target, &t.function),
            None => format!("**{target}**\n\nunknown function"),
        },
        None => {
            let input = sf.function.input(token.text)?;
            format!("`{}`\n\n{}", input_sig(input), input.description.value)
        }
    };
    markdown_hover(value)
}

fn definition(docs: &Docs, p: &TextDocumentPositionParams) -> Option<Value> {
    let (project, name) = locate(docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let token = token_at(&sf.text, offset(&sf.text, p.position))?;
    let location = match token.text.strip_prefix('@') {
        Some(target) => {
            let t = project.get(target)?;
            Location::new(uri_for(docs, &t.path)?, Range::default())
        }
        None => {
            let input = sf.function.input(token.text)?;
            Location::new(p.text_document.uri.clone(), range(&sf.text, input.name.span))
        }
    };
    serde_json::to_value(location).ok()
}

fn completion(docs: &Docs, p: &TextDocumentPositionParams) -> Option<Value> {
    let (project, name) = locate(docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let off = offset(&sf.text, p.position);
    let items: Vec<CompletionItem> = if sf.text[word_start(&sf.text, off)..off].starts_with('@') {
        project
            .files
            .iter()
            .map(|(n, f)| CompletionItem {
                label: n.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(description(&f.function)),
                ..Default::default()
            })
            .collect()
    } else {
        let inputs = sf.function.inputs.iter().map(|i| CompletionItem {
            label: i.name.value.clone(),
            kind: Some(CompletionItemKind::VARIABLE),
            detail: Some(type_label(&i.ty)),
            ..Default::default()
        });
        let keywords = KEYWORDS.iter().map(|k| CompletionItem {
            label: k.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        });
        inputs.chain(keywords).collect()
    };
    serde_json::to_value(CompletionResponse::Array(items)).ok()
}

fn markdown_hover(value: String) -> Option<Value> {
    let hover = Hover {
        contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }),
        range: None,
    };
    serde_json::to_value(hover).ok()
}

fn ref_markdown(name: &str, f: &Function) -> String {
    let mut out = format!("**{name}**\n\n{}", description(f));
    for input in &f.inputs {
        out.push_str(&format!("\n\n- `{}`: {}", input_sig(input), input.description.value));
    }
    out
}

/// `name: bool` or `name: choice("a", "b")`.
fn input_sig(input: &Input) -> String {
    let ty = match input.ty.options() {
        None => "bool".to_string(),
        Some(options) => {
            let quoted: Vec<String> = options.iter().map(|o| format!("\"{o}\"")).collect();
            format!("choice({})", quoted.join(", "))
        }
    };
    format!("{}: {ty}", input.name.value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_columns_count_surrogate_pairs() {
        // "😀" is 4 bytes but 2 UTF-16 units.
        let text = "a😀 @x\n";
        let at = text.find('@').unwrap();
        assert_eq!(position(text, at), Position::new(0, 4));
        assert_eq!(offset(text, Position::new(0, 4)), at);
    }

    #[test]
    fn utf16_column_resets_per_line() {
        let text = "é\nxé @y";
        let at = text.find('@').unwrap();
        assert_eq!(position(text, at), Position::new(1, 3));
        assert_eq!(offset(text, Position::new(1, 3)), at);
    }

    #[test]
    fn token_at_finds_ref_and_input_names() {
        let text = "input docs_only: bool \"q\"\nreturn @ci/fix\n";
        let at_ci = text.find("ci").unwrap() + 1;
        assert_eq!(token_at(text, at_ci).unwrap().text, "@ci/fix");
        let at_input = text.find("docs_only").unwrap() + 2;
        assert_eq!(token_at(text, at_input).unwrap().text, "docs_only");
    }

    #[test]
    fn token_inside_string_is_none() {
        let text = "return \"see @ci/fix\"\n";
        let at_ci = text.find("ci").unwrap() + 1;
        assert!(token_at(text, at_ci).is_none());
    }
}
