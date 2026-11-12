//! Completion: reference paths after `@./`, argument names inside a call, union members after `===` or `case`,
//! and elsewhere the parameter names, std names and keywords.

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, Documentation, Range,
    TextDocumentPositionParams, TextEdit,
};
use rein_core::ast::{is_struct, Function, STRUCT_SUFFIX};
use rein_core::lexer::KEYWORDS;
use rein_core::project::Project;

use super::call_context::{call_at, callee_params};
use super::hover::input_sig;
use super::occurrences::declaration;
use super::text::{is_path_char, is_token_char, line_before, offset, position, relative_import};
use super::workspace::{locate, State};

pub fn completion(state: &State, p: &TextDocumentPositionParams) -> Option<CompletionResponse> {
    let (project, name) = locate(&state.docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let off = offset(&sf.text, p.position);
    let items = ref_completions(&sf.text, off, &project, &name)
        .or_else(|| arg_name_completions(&sf.text, off, &project, &name))
        .or_else(|| option_completions(&sf.text, off, &sf.function))
        .unwrap_or_else(|| {
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
        });
    Some(CompletionResponse::Array(items))
}

/// Completion of a reference path: the project functions, or after `}: ` the shared structs, relative to this file, that match what is typed.
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
    // After `}: ` a parameter type goes, so the shared structs are offered. Elsewhere, the functions.
    let in_type = before[..at].trim_end().ends_with(':');
    let kind = if in_type { CompletionItemKind::STRUCT } else { CompletionItemKind::FUNCTION };
    let items = project
        .files
        .keys()
        .filter(|n| n.as_str() != from && is_struct(n) == in_type)
        .map(|n| format!("@{}", relative_import(from, n.strip_suffix(STRUCT_SUFFIX).unwrap_or(n))))
        .filter(|label| label.starts_with(typed))
        .map(|label| CompletionItem {
            label: label.clone(),
            kind: Some(kind),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: label })),
            ..Default::default()
        })
        .collect();
    Some(items)
}

/// Inside call parentheses, where an argument name goes: the callee's parameters not given yet, as `name=`.
fn arg_name_completions(text: &str, off: usize, project: &Project, from: &str) -> Option<Vec<CompletionItem>> {
    let call = call_at(text, off)?;
    if !call.at_name {
        return None;
    }
    let params = callee_params(project, from, &call.callee)?;
    let items = params
        .into_iter()
        .filter(|i| !call.given.contains(&i.name.value))
        .map(|i| CompletionItem {
            label: format!("{}=", i.name.value),
            kind: Some(CompletionItemKind::FIELD),
            detail: Some(input_sig(i)),
            documentation: Some(Documentation::String(i.description.value.clone())),
            ..Default::default()
        })
        .collect();
    Some(items)
}

/// After `x === ` or `case `: the members of the union `x`, or of the switch subject, as string literals. Each
/// item replaces the literal under the cursor.
fn option_completions(text: &str, off: usize, f: &Function) -> Option<Vec<CompletionItem>> {
    let before = line_before(text, off);
    let mut head = before.trim_end_matches(is_token_char);
    let mut start = off - (before.len() - head.len());
    if let Some(h) = head.strip_suffix(['"', '\'']) {
        head = h;
        start -= 1;
    }
    let head = head.trim_end();
    // `===` before `==`, so `x ===` is not read as `x =` plus `==`.
    let op = ["===", "!==", "==", "!="].iter().find_map(|op| head.strip_suffix(op));
    let var = if let Some(h) = op {
        let h = h.trim_end();
        &h[h.trim_end_matches(is_token_char).len()..]
    } else if head.trim_start() == "case" {
        switch_subject(&text[..off])?
    } else {
        return None;
    };
    let options = declaration(f, var)?.ty.options()?;
    let rest = &text[off..];
    let mut end = off + (rest.len() - rest.trim_start_matches(is_token_char).len());
    if text[end..].starts_with(['"', '\'']) {
        end += 1;
    }
    let range = Range::new(position(text, start), position(text, end));
    let items = options
        .into_iter()
        .map(|o| {
            let label = format!("\"{o}\"");
            CompletionItem {
                label: label.clone(),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: label })),
                ..Default::default()
            }
        })
        .collect();
    Some(items)
}

/// The name in the last `switch (name)` of `text`.
fn switch_subject(text: &str) -> Option<&str> {
    let rest = text[text.rfind("switch")? + "switch".len()..].trim_start().strip_prefix('(')?.trim_start();
    let end = rest.find(|c: char| !is_token_char(c))?;
    Some(&rest[..end])
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
