//! Rename: a parameter, with `name=` in every call to its function, or a function, which renames its file and
//! updates every `@./` reference to it.

use std::collections::BTreeMap;

use lsp_types::{
    DocumentChangeOperation, DocumentChanges, OneOf, OptionalVersionedTextDocumentIdentifier, PrepareRenameResponse,
    RenameFile, RenameParams, ResourceOp, TextDocumentEdit, TextDocumentPositionParams, TextEdit, Url, WorkspaceEdit,
};
use rein_core::diagnostic::Span;
use rein_core::lexer::KEYWORDS;
use rein_core::project::Project;

use super::occurrences::{callers, declaration, name_span, param_uses, symbol_at, Symbol};
use super::text::{is_identifier, offset, range, relative_import};
use super::workspace::{locate, uri_for, Docs, State};

/// The span to rename and its current name. On a reference, only its last path segment, the file name, changes.
pub fn prepare(state: &State, p: &TextDocumentPositionParams) -> Result<Option<PrepareRenameResponse>, String> {
    let Some((project, name)) = locate(&state.docs, &p.text_document.uri) else { return Ok(None) };
    let Some(sf) = project.get(&name) else { return Ok(None) };
    let Some((symbol, span)) = symbol_at(&project, &name, offset(&sf.text, p.position)) else { return Ok(None) };
    let (span, placeholder) = match symbol {
        Symbol::Std(n) => return Err(std_error(&n)),
        Symbol::Param { name: n, .. } => (span, n),
        Symbol::Function(target) => {
            let base = basename(&target).to_string();
            let written = &sf.text[span.start..span.end];
            let span = if written.ends_with(&format!("/{base}")) { Span::new(span.end - base.len(), span.end) } else { span };
            (span, base)
        }
    };
    Ok(Some(PrepareRenameResponse::RangeWithPlaceholder { range: range(&sf.text, span), placeholder }))
}

pub fn rename(state: &State, p: &RenameParams) -> Result<Option<WorkspaceEdit>, String> {
    let pos = &p.text_document_position;
    let Some((project, name)) = locate(&state.docs, &pos.text_document.uri) else { return Ok(None) };
    let Some(sf) = project.get(&name) else { return Ok(None) };
    let Some((symbol, _)) = symbol_at(&project, &name, offset(&sf.text, pos.position)) else { return Ok(None) };
    let new = p.new_name.as_str();
    if !is_identifier(new) || KEYWORDS.contains(&new) {
        return Err(format!("`{new}` is not a valid name: use letters, digits and `_`, and start with a letter"));
    }
    match symbol {
        Symbol::Std(n) => Err(std_error(&n)),
        Symbol::Param { function, name: old } => rename_param(&state.docs, &project, &function, &old, new).map(Some),
        Symbol::Function(target) => rename_function(state, &project, &target, new).map(Some),
    }
}

fn std_error(n: &str) -> String {
    format!("`{n}` is a std item; it cannot be renamed")
}

/// The file name of a function, such as `refund` for `billing/refund`.
fn basename(function: &str) -> &str {
    function.rsplit('/').next().unwrap_or(function)
}

fn rename_param(docs: &Docs, project: &Project, function: &str, old: &str, new: &str) -> Result<WorkspaceEdit, String> {
    let f = &project.get(function).ok_or_else(|| format!("cannot find function `{function}`"))?.function;
    let decl = declaration(f, old).ok_or_else(|| format!("`{old}` is not declared in `{function}`"))?;
    if declaration(f, new).is_some() {
        return Err(format!("`{function}` already has `{new}`"));
    }
    let mut edits = Edits::default();
    edits.add(project, function, decl.name.span, new);
    for (file, span) in param_uses(project, function, old) {
        edits.add(project, &file, span, new);
    }
    Ok(edits.into_workspace_edit(docs, project, None))
}

/// Renames the file of `target` to `<new>.rein` in the same folder. Every reference to it changes its last
/// segment. The name after `export default function` changes too when it was the file name.
fn rename_function(state: &State, project: &Project, target: &str, new: &str) -> Result<WorkspaceEdit, String> {
    if !state.renames_files {
        return Err("this editor cannot rename files, so a function cannot be renamed".to_string());
    }
    let sf = project.get(target).ok_or_else(|| format!("cannot find function `{target}`"))?;
    let old = basename(target);
    let new_path = sf.path.with_file_name(format!("{new}.rein"));
    if new_path.exists() {
        return Err(format!("{} already exists", new_path.display()));
    }
    let new_target = match target.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/{new}"),
        None => new.to_string(),
    };
    let mut edits = Edits::default();
    for (caller, span) in callers(project, target) {
        let Some(text) = project.get(&caller).map(|c| &c.text) else { continue };
        if text[span.start..span.end].ends_with(&format!("/{old}")) {
            edits.add(project, &caller, Span::new(span.end - old.len(), span.end), new);
        } else {
            edits.add(project, &caller, span, &format!("@{}", relative_import(&caller, &new_target)));
        }
    }
    if let Some(span) = name_span(&sf.text).filter(|s| &sf.text[s.start..s.end] == old) {
        edits.add(project, target, span, new);
    }
    let old_uri = uri_for(&state.docs, &sf.path).ok_or("the file has no URI")?;
    let new_uri = Url::from_file_path(&new_path).map_err(|()| "the new path has no URI".to_string())?;
    Ok(edits.into_workspace_edit(&state.docs, project, Some((old_uri, new_uri))))
}

/// Text edits per function file.
#[derive(Default)]
struct Edits(BTreeMap<String, Vec<TextEdit>>);

impl Edits {
    fn add(&mut self, project: &Project, function: &str, span: Span, new_text: &str) {
        if let Some(sf) = project.get(function) {
            let edit = TextEdit { range: range(&sf.text, span), new_text: new_text.to_string() };
            self.0.entry(function.to_string()).or_default().push(edit);
        }
    }

    /// The text edits of each file, then the file rename, if any. The text edits use the old URIs, so they
    /// apply before the rename.
    fn into_workspace_edit(self, docs: &Docs, project: &Project, rename: Option<(Url, Url)>) -> WorkspaceEdit {
        let mut ops: Vec<DocumentChangeOperation> = self
            .0
            .into_iter()
            .filter_map(|(function, edits)| {
                let uri = uri_for(docs, &project.get(&function)?.path)?;
                Some(DocumentChangeOperation::Edit(TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier { uri, version: None },
                    edits: edits.into_iter().map(OneOf::Left).collect(),
                }))
            })
            .collect();
        if let Some((old_uri, new_uri)) = rename {
            let file = RenameFile { old_uri, new_uri, options: None, annotation_id: None };
            ops.push(DocumentChangeOperation::Op(ResourceOp::Rename(file)));
        }
        WorkspaceEdit { document_changes: Some(DocumentChanges::Operations(ops)), ..Default::default() }
    }
}
