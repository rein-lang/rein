//! Quick fixes for a removed `@` reference or default import: convert that one form, or convert every form in the file.

use std::collections::HashMap;

use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Diagnostic, NumberOrString, Range, TextEdit, Url, WorkspaceEdit,
};
use rein_core::diagnostic::{Span, REMOVED_CODE};
use rein_core::migrate_imports::{migrate_one, migrate_text};
use rein_core::project::Project;

use super::text::{offset, position};

/// The two quick fixes for each deprecation in `diags`. Each one replaces the whole text of file `name`, whose text is
/// `text`. A fix that changes nothing is left out.
pub fn deprecation_actions(
    project: &Project,
    name: &str,
    text: &str,
    uri: &Url,
    diags: &[Diagnostic],
) -> Vec<CodeActionOrCommand> {
    let removed = Some(NumberOrString::String(REMOVED_CODE.to_string()));
    let mut out = Vec::new();
    for d in diags.iter().filter(|d| d.code == removed) {
        let span = Span::new(offset(text, d.range.start), offset(text, d.range.end));
        let one = migrate_one(project, name, text, span).text;
        let all = migrate_text(project, name, text).text;
        for (title, new_text) in [("Convert to named import", one), ("Convert all in this file", all)] {
            if new_text != text {
                out.push(action(title, d, uri, text, new_text));
            }
        }
    }
    out
}

/// A quick fix that replaces the whole document with `new_text`.
fn action(title: &str, d: &Diagnostic, uri: &Url, text: &str, new_text: String) -> CodeActionOrCommand {
    let whole = Range::new(position(text, 0), position(text, text.len()));
    let changes = HashMap::from([(uri.clone(), vec![TextEdit { range: whole, new_text }])]);
    CodeActionOrCommand::CodeAction(CodeAction {
        title: title.to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![d.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        ..Default::default()
    })
}
