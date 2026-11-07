//! Language server over stdio: diagnostics, hover, go to definition and completion.
//!
//! Every request rebuilds the project from disk plus the open buffers. That is slow on big
//! projects, which is fine for the proof of concept.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionResponse, CompletionTextEdit, Diagnostic,
    DiagnosticSeverity, DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    Hover, HoverContents, HoverProviderCapability, Location, MarkupContent,
    MarkupKind, OneOf, Position, PublishDiagnosticsParams, Range, ServerCapabilities,
    TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url,
};
use rein_core::ast::{Function, Import, Input};
use rein_core::diagnostic::{Diagnostic as ReinDiagnostic, Severity, Span};
use rein_core::lexer::KEYWORDS;
use rein_core::project::Project;
use serde_json::Value;

use crate::cli::description;

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
            trigger_characters: Some(vec![" ".into(), "\"".into(), "'".into(), "/".into(), ".".into()]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn main_loop(connection: &Connection) -> BoxResult<()> {
    let mut docs = Docs::new();
    // Every URI that has a diagnostics list from us. A file that leaves its project gets an empty one.
    let mut published = BTreeSet::new();
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
                handle_notification(connection, &mut docs, &mut published, note)?;
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

fn handle_notification(
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

/// Re-checks the projects that hold `changed` and every file published so far. Then it publishes.
/// A file gets diagnostics from the project its own path resolves to. A file that is no longer in
/// such a project gets an empty list, so its errors clear.
fn refresh(connection: &Connection, docs: &Docs, published: &mut BTreeSet<Url>, changed: &Url) -> BoxResult<()> {
    let Some(path) = doc_path(changed) else { return Ok(()) };
    let mut roots: BTreeSet<PathBuf> = published.iter().filter_map(doc_path).map(|p| Project::find_root(&p)).collect();
    roots.insert(Project::find_root(&path));
    let mut lists: BTreeMap<Url, Vec<Diagnostic>> = BTreeMap::new();
    for root in roots {
        let project = build_project(docs, &root);
        let diags = project.check();
        for (name, sf) in &project.files {
            if Project::find_root(&sf.path) != root {
                continue;
            }
            let Some(file_uri) = uri_for(docs, &sf.path) else { continue };
            let list: Vec<Diagnostic> = diags
                .get(name)
                .map(|ds| ds.iter().map(|d| to_lsp(&sf.text, d)).collect())
                .unwrap_or_default();
            lists.insert(file_uri, list);
        }
        // The client does not open rein.toml, so project errors go on an open file of the project.
        // Without one, they go on the file that changed.
        let extra: Vec<Diagnostic> = project.project_diagnostics().iter().map(|d| to_lsp("", d)).collect();
        if !extra.is_empty() {
            let open = lists.keys().find(|u| docs.contains_key(u)).cloned();
            lists.entry(open.unwrap_or_else(|| changed.clone())).or_default().extend(extra);
        }
    }
    for stale in published.iter().filter(|u| !lists.contains_key(*u)) {
        send_diagnostics(connection, stale.clone(), Vec::new())?;
    }
    for (file_uri, list) in &lists {
        send_diagnostics(connection, file_uri.clone(), list.clone())?;
    }
    *published = lists.into_keys().collect();
    Ok(())
}

fn send_diagnostics(connection: &Connection, uri: Url, diagnostics: Vec<Diagnostic>) -> BoxResult<()> {
    let params = PublishDiagnosticsParams { uri, diagnostics, version: None };
    let note = Notification::new("textDocument/publishDiagnostics".to_string(), params);
    connection.sender.send(Message::Notification(note))?;
    Ok(())
}

/// Project under `root`: files on disk, with open buffers replacing their disk copies.
/// A buffer in a nested project is left out, as its file is left out of this one.
fn build_project(docs: &Docs, root: &Path) -> Project {
    let mut by_path: BTreeMap<PathBuf, String> = BTreeMap::new();
    if let Ok(disk) = Project::load(root) {
        by_path.extend(disk.files.into_values().map(|sf| (sf.path, sf.text)));
    }
    by_path.extend(docs.iter().filter_map(|(uri, text)| {
        let path = doc_path(uri)?;
        (Project::find_root(&path) == root).then(|| (path, text.clone()))
    }));
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
    c.is_ascii_alphanumeric() || c == '_'
}

/// Start of the name that ends at or covers `offset`.
fn word_start(text: &str, offset: usize) -> usize {
    text[..offset]
        .char_indices()
        .rev()
        .find(|&(_, c)| !is_token_char(c))
        .map_or(0, |(i, c)| i + c.len_utf8())
}

/// End of the name that starts at or covers `offset`.
fn word_end(text: &str, offset: usize) -> usize {
    text[offset..]
        .char_indices()
        .find(|&(_, c)| !is_token_char(c))
        .map_or(text.len(), |(i, _)| offset + i)
}

/// A name under the cursor.
struct Token<'a> {
    text: &'a str,
}

/// The name at `offset`. Returns None inside a string literal.
fn token_at(text: &str, offset: usize) -> Option<Token<'_>> {
    let start = word_start(text, offset);
    let end = word_end(text, offset);
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let quotes = text[line_start..start].chars().filter(|&c| c == '"' || c == '\'').count();
    (start < end && quotes % 2 == 0).then(|| Token { text: &text[start..end] })
}

/// The text of the current line before `offset`.
fn line_before(text: &str, offset: usize) -> &str {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    &text[line_start..offset]
}

/// True when the cursor is inside the quotes of an import path, as in `from "./|`.
fn in_import_path(before: &str) -> bool {
    let Some(q) = before.rfind(['"', '\'']) else { return false };
    before[..q].trim_end().ends_with("from")
}

/// True when the cursor is after `return` and a space, where an imported name goes.
fn after_return(before: &str) -> bool {
    let without_word = before.trim_end_matches(is_token_char);
    let trimmed = without_word.trim_end();
    match trimmed.strip_suffix("return") {
        Some(head) => trimmed.len() < without_word.len() && !head.ends_with(is_token_char),
        None => false,
    }
}

/// The import whose quoted path holds `off`.
fn import_at(f: &Function, off: usize) -> Option<&Import> {
    f.imports.iter().find(|i| i.source.span.contains(off))
}

fn hover(docs: &Docs, p: &TextDocumentPositionParams) -> Option<Value> {
    let (project, name) = locate(docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let token = token_at(&sf.text, offset(&sf.text, p.position))?;
    let value = match project.import_targets(&name).get(token.text) {
        Some(target) => ref_markdown(target, &project.get(target)?.function),
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
    let off = offset(&sf.text, p.position);
    let location = if let Some(imp) = import_at(&sf.function, off) {
        let target = project.resolve(&name, &imp.source.value).ok()?;
        target_location(docs, &project, &target)?
    } else {
        let token = token_at(&sf.text, off)?;
        match project.import_targets(&name).get(token.text) {
            Some(target) => target_location(docs, &project, target)?,
            None => {
                let input = sf.function.input(token.text)?;
                Location::new(p.text_document.uri.clone(), range(&sf.text, input.name.span))
            }
        }
    };
    serde_json::to_value(location).ok()
}

/// The start of the file of function `target`.
fn target_location(docs: &Docs, project: &Project, target: &str) -> Option<Location> {
    let t = project.get(target)?;
    Some(Location::new(uri_for(docs, &t.path)?, Range::default()))
}

fn completion(docs: &Docs, p: &TextDocumentPositionParams) -> Option<Value> {
    let (project, name) = locate(docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let off = offset(&sf.text, p.position);
    let before = line_before(&sf.text, off);
    let items: Vec<CompletionItem> = if in_import_path(before) {
        import_path_completions(&sf.text, off, &project, &name)
    } else if after_return(before) {
        return_items(&project, &name)
    } else {
        let inputs = sf.function.inputs.iter().map(|i| CompletionItem {
            label: i.name.value.clone(),
            kind: Some(CompletionItemKind::VARIABLE),
            detail: Some(i.ty.ts_type()),
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

/// Import paths for the string the cursor is in. Each item replaces the whole string content, from the
/// opening quote to the closing quote or the cursor. Items must start with the text typed so far.
fn import_path_completions(text: &str, off: usize, project: &Project, from: &str) -> Vec<CompletionItem> {
    let before = line_before(text, off);
    let quote = off - before.len() + before.rfind(['"', '\'']).unwrap_or(0);
    let typed = &text[quote + 1..off];
    let line_rest = &text[off..];
    let line_rest = &line_rest[..line_rest.find('\n').unwrap_or(line_rest.len())];
    let end = line_rest.find(['"', '\'']).map_or(off, |i| off + i);
    let range = Range::new(position(text, quote + 1), position(text, end));
    import_path_items(project, from)
        .into_iter()
        .filter(|item| item.label.starts_with(typed))
        .map(|mut item| {
            item.text_edit = Some(CompletionTextEdit::Edit(TextEdit { range, new_text: item.label.clone() }));
            item
        })
        .collect()
}

/// Every other file of the project, as an import path from `from`.
fn import_path_items(project: &Project, from: &str) -> Vec<CompletionItem> {
    project
        .files
        .keys()
        .filter(|n| n.as_str() != from)
        .map(|n| CompletionItem {
            label: relative_import(from, n),
            kind: Some(CompletionItemKind::FILE),
            detail: project.get(n).map(|sf| description(&sf.function)),
            ..Default::default()
        })
        .collect()
}

/// The imported names of `name`.
fn return_items(project: &Project, name: &str) -> Vec<CompletionItem> {
    project
        .import_targets(name)
        .iter()
        .map(|(binding, target)| CompletionItem {
            label: binding.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: project.get(target).map(|sf| description(&sf.function)),
            ..Default::default()
        })
        .collect()
}

/// The import path from function `from` to function `to`, without `.rein`. Such as `./x` or `../x`.
fn relative_import(from: &str, to: &str) -> String {
    let from_dir: Vec<&str> = from.split('/').collect();
    let from_dir = &from_dir[..from_dir.len() - 1];
    let to_parts: Vec<&str> = to.split('/').collect();
    let common = from_dir.iter().zip(&to_parts[..to_parts.len() - 1]).take_while(|(a, b)| a == b).count();
    let ups = from_dir.len() - common;
    let prefix = if ups == 0 { "./".to_string() } else { "../".repeat(ups) };
    format!("{prefix}{}", to_parts[common..].join("/"))
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

/// `name: boolean` or `name: "a" | "b"`, as TypeScript writes it.
fn input_sig(input: &Input) -> String {
    format!("{}: {}", input.name.value, input.ty.ts_type())
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
    fn token_at_finds_import_and_parameter_names() {
        let text = "return mergePr;\n  if (docsOnly) {\n";
        let at_import = text.find("Pr").unwrap() + 1;
        assert_eq!(token_at(text, at_import).unwrap().text, "mergePr");
        let at_param = text.find("docsOnly").unwrap() + 2;
        assert_eq!(token_at(text, at_param).unwrap().text, "docsOnly");
    }

    #[test]
    fn token_inside_string_is_none() {
        let text = "import a from \"./ci/fix\";\n";
        let at = text.find("ci").unwrap() + 1;
        assert!(token_at(text, at).is_none());
        let text = "import a from './ci/fix';\n";
        let at = text.find("ci").unwrap() + 1;
        assert!(token_at(text, at).is_none());
    }

    #[test]
    fn import_path_and_return_contexts() {
        assert!(in_import_path("import a from \"./"));
        assert!(in_import_path("import a from '../"));
        assert!(!in_import_path("import a from \"./x\";"));
        assert!(after_return("  return "));
        assert!(after_return("return mer"));
        assert!(!after_return("return"));
        assert!(!after_return("myreturn "));
    }

    #[test]
    fn relative_import_paths() {
        assert_eq!(relative_import("ci/fix", "merge_pr"), "../merge_pr");
        assert_eq!(relative_import("ci/fix", "ci/deeper"), "./deeper");
        assert_eq!(relative_import("ci/sub/leaf", "top"), "../../top");
        assert_eq!(relative_import("main", "ci/fix"), "./ci/fix");
    }
}
