//! Quick fixes for some diagnostics: add a missing doc comment, name a positional argument, and move a function
//! that one other function calls into that caller's folder.

use std::collections::HashMap;

use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams, CodeActionResponse, Diagnostic, Range,
    TextEdit, WorkspaceEdit,
};
use rein_core::ast::Function;
use rein_core::lexer::{lex, Tok};
use rein_core::project::Project;

use super::occurrences::calls;
use super::rename::move_function;
use super::text::{offset, position};
use super::workspace::{locate, State};

pub fn code_actions(state: &State, p: &CodeActionParams) -> Option<CodeActionResponse> {
    let (project, name) = locate(&state.docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let actions = p
        .context
        .diagnostics
        .iter()
        .filter_map(|d| {
            let (title, edit) = match move_fix(state, &project, &name, d) {
                Some(moved) => moved,
                None => {
                    let (title, edit) = fix(&sf.text, &sf.function, d)?;
                    let changes = HashMap::from([(p.text_document.uri.clone(), vec![edit])]);
                    (title, WorkspaceEdit { changes: Some(changes), ..Default::default() })
                }
            };
            Some(CodeActionOrCommand::CodeAction(CodeAction {
                title,
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![d.clone()]),
                edit: Some(edit),
                is_preferred: Some(true),
                ..Default::default()
            }))
        })
        .collect();
    Some(actions)
}

/// Moves function `name` into the folder of its only caller, as the warning says: the file moves and every
/// reference follows. None when the editor cannot rename files.
fn move_fix(state: &State, project: &Project, name: &str, d: &Diagnostic) -> Option<(String, WorkspaceEdit)> {
    if !d.message.contains("so the folders show the process") {
        return None;
    }
    let home = project.nest_target(name)?;
    let edit = move_function(state, project, name, &home).ok()?;
    Some((format!("Move to {home}.rein"), edit))
}

/// The fix for one diagnostic, found by its message: a title and one text edit.
fn fix(text: &str, f: &Function, d: &Diagnostic) -> Option<(String, TextEdit)> {
    let insert = |at: usize, new_text: &str| TextEdit {
        range: Range::new(position(text, at), position(text, at)),
        new_text: new_text.to_string(),
    };
    let msg = d.message.as_str();
    // Swap the quote kind. ponytail: only plain text converts; text with a quote, a backtick, `$` or `\` keeps
    // its form, and the author fixes it by hand.
    let swap = |open: &str, close: &str| -> Option<TextEdit> {
        let (start, end) = (offset(text, d.range.start), offset(text, d.range.end));
        let inner = text.get(start + 1..end.checked_sub(1)?)?;
        (!inner.contains(['"', '\'', '`', '$', '\\', '\n'])).then(|| TextEdit { range: d.range, new_text: format!("{open}{inner}{close}") })
    };
    if msg == rein_core::check::TEXT_QUOTES {
        return swap("`", "`").map(|e| ("Use backticks for text".to_string(), e));
    }
    if msg.starts_with("a tag uses quotes: ") {
        return swap("\"", "\"").map(|e| ("Use quotes for a tag".to_string(), e));
    }
    if msg.starts_with("missing doc comment on ") {
        let (_, export) = lex(text).ok()?.into_iter().find(|(t, _)| *t == Tok::Export)?;
        let doc = "/**\n * TODO: say what this function decides.\n */\n";
        return Some(("Add a doc comment".to_string(), insert(export.start, doc)));
    }
    // The name the message is about, between its first pair of backticks.
    let named = msg.split('`').nth(1)?;
    if msg.starts_with("parameter `") && msg.contains("needs a doc comment") {
        let input = f.input(named).filter(|i| i.doc.is_none())?;
        let doc = "/** TODO: ask the question. */ ";
        return Some((format!("Add a doc comment to `{named}`"), insert(input.name.span.start, doc)));
    }
    if msg.starts_with("missing argument `") {
        let start = offset(text, d.range.start);
        let positional = calls(f).into_iter().flat_map(|c| &c.args).any(|a| a.name.is_none() && a.span.start == start);
        return positional.then(|| (format!("Name the argument `{named}=`"), insert(start, &format!("{named}="))));
    }
    None
}
