//! Completion: reference paths after `@./`, and elsewhere the parameter names, std names and keywords.

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, Range, TextDocumentPositionParams,
    TextEdit,
};
use rein_core::lexer::KEYWORDS;
use rein_core::project::Project;

use super::text::{is_path_char, line_before, offset, position, relative_import};
use super::workspace::{locate, State};

pub fn completion(state: &State, p: &TextDocumentPositionParams) -> Option<CompletionResponse> {
    let (project, name) = locate(&state.docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let off = offset(&sf.text, p.position);
    let items: Vec<CompletionItem> = if let Some(items) = ref_completions(&sf.text, off, &project, &name) {
        items
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
        inputs.chain(std_completions()).chain(keywords).collect()
    };
    Some(CompletionResponse::Array(items))
}

/// Completion of a reference path: the project functions, relative to this file, that match what is typed.
/// Each item replaces the whole reference token, from its `@` to the end of its path.
fn ref_completions(text: &str, off: usize, project: &Project, from: &str) -> Option<Vec<CompletionItem>> {
    let before = line_before(text, off);
    let at = before.rfind('@')?;
    let typed = &before[at..];
    if !typed[1..].chars().all(is_path_char) {
        return None;
    }
    let at_abs = off - before.len() + at;
    let end = text[off..].find(|c: char| !is_path_char(c)).map_or(text.len(), |i| off + i);
    let range = Range::new(position(text, at_abs), position(text, end));
    let items = project
        .files
        .keys()
        .filter(|n| n.as_str() != from)
        .map(|n| format!("@{}", relative_import(from, n)))
        .filter(|label| label.starts_with(typed))
        .map(|label| CompletionItem {
            label: label.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: label })),
            ..Default::default()
        })
        .collect();
    Some(items)
}

/// Each std name, for completion. Its detail is its description.
fn std_completions() -> Vec<CompletionItem> {
    rein_core::stdlib::names()
        .into_iter()
        .map(|name| CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(rein_core::stdlib::doc(name)).filter(|d| !d.is_empty()),
            ..Default::default()
        })
        .collect()
}
