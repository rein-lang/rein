//! Completion on an import line. Between `{` and `}` it offers the name the target declares. After `from "` it
//! offers the project paths (`./`, `../`, `.struct` files) and the installed program names.

use lsp_types::{CompletionItem, CompletionItemKind, CompletionTextEdit, Range, TextEdit};
use rein_core::ast::{is_struct, Binding, STRUCT_SUFFIX};
use rein_core::project::Project;

use super::text::{line_before, position, relative_import};

/// The items for the import line at `off`, or `None` when the cursor is not on an import name or path.
pub fn import_completion(text: &str, off: usize, project: &Project, from: &str) -> Option<Vec<CompletionItem>> {
    let before = line_before(text, off);
    if !before.trim_start().starts_with("import") {
        return None;
    }
    if let Some(q) = before.rfind('"') {
        if !before[q + 1..].contains('"') && before[..q].trim_end().ends_with("from") {
            return Some(path_completions(text, off, q, project, from));
        }
    }
    if before.contains('{') && !before.contains('}') {
        return name_completions(text, off, project, from);
    }
    None
}

/// The paths that go after `from "`. The path starts after the quote at `q` of the line before `off`. Each item
/// replaces the whole path, up to the closing quote.
fn path_completions(text: &str, off: usize, q: usize, project: &Project, from: &str) -> Vec<CompletionItem> {
    let start = off - line_before(text, off).len() + q + 1;
    let rest = &text[off..];
    let end = off + rest.find(['"', '\n']).unwrap_or(rest.len());
    let typed = &text[start..off];
    let range = Range::new(position(text, start), position(text, end));
    let files = project.files.keys().filter(|n| n.as_str() != from).map(|n| {
        let base = n.strip_suffix(STRUCT_SUFFIX).unwrap_or(n);
        let label = if is_struct(n) {
            format!("{}{STRUCT_SUFFIX}", relative_import(from, base))
        } else {
            relative_import(from, base)
        };
        (label, CompletionItemKind::FILE)
    });
    // A `./` or `../` path names a file of the project. Any other path names an installed program.
    let programs = if typed.starts_with('.') {
        Vec::new()
    } else {
        let visible = crate::scopes::discover_from(&project.root);
        visible
            .iter()
            .filter(|f| f.shadowed_by.is_none())
            .map(|f| (f.name.clone(), CompletionItemKind::MODULE))
            .collect()
    };
    files
        .chain(programs)
        .filter(|(label, _)| label.starts_with(typed))
        .map(|(label, kind)| CompletionItem {
            label: label.clone(),
            kind: Some(kind),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: label })),
            ..Default::default()
        })
        .collect()
}

/// Between the braces, the name that the target of the line's `from "..."` declares.
fn name_completions(text: &str, off: usize, project: &Project, from: &str) -> Option<Vec<CompletionItem>> {
    let line_start = text[..off].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[off..].find('\n').map_or(text.len(), |i| off + i);
    let line = &text[line_start..line_end];
    let path = &line[line.find("from \"")? + "from \"".len()..];
    let path = &path[..path.find('"')?];
    let declared = declared_name(project, from, path)?;
    Some(vec![CompletionItem {
        label: declared,
        kind: Some(CompletionItemKind::FUNCTION),
        detail: Some(format!("from \"{path}\"")),
        ..Default::default()
    }])
}

/// The name that the target of an import path declares. A `.struct` path names the struct.
fn declared_name(project: &Project, from: &str, path: &str) -> Option<String> {
    if let Some(base) = path.strip_suffix(STRUCT_SUFFIX) {
        return project.struct_for(from, base).ok().map(|(_, s)| s.name.value.clone());
    }
    match project.binding_for(from, path).ok()? {
        Binding::Function(target) => project.get(&target)?.function.name.as_ref().map(|n| n.value.clone()),
        Binding::Template { path: Some(target), .. } => {
            project.get(&target)?.function.name.as_ref().map(|n| n.value.clone())
        }
        _ => None,
    }
}
