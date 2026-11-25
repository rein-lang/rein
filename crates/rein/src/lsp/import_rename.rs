//! Rename of an imported local name, and the import lines that a function's move changes. A local name renames only
//! its own file. A function that moves rewrites the path of each import line that names it, and its declared name
//! there when that changes.

use lsp_types::WorkspaceEdit;
use rein_core::ast::Function;
use rein_core::diagnostic::Span;
use rein_core::project::Project;
use rein_core::stdlib;

use super::import_references::local_uses;
use super::import_symbols::named_imports;
use super::occurrences::{declarations, name_span, target};
use super::rename::Edits;
use super::text::relative_import;
use super::workspace::Docs;

/// Renames the local name `old` of an import in `file` to `new`, and every use of it in that file. Without `as`, the
/// line becomes `{ declared as new }`. With `as`, the alias changes, and it goes when `new` is the declared name. A
/// default import changes its name in place.
pub fn rename_import_local(
    docs: &Docs,
    project: &Project,
    file: &str,
    old: &str,
    new: &str,
) -> Result<WorkspaceEdit, String> {
    let sf = project.get(file).ok_or_else(|| format!("cannot find `{file}`"))?;
    let f = &sf.function;
    let import = named_imports(f)
        .find(|i| i.local.value == old)
        .ok_or_else(|| format!("`{old}` is not an imported name in `{file}`"))?;
    let mut edits = Edits::default();
    if old == new {
        return Ok(edits.into_workspace_edit(docs, project, None));
    }
    local_clash(f, new)?;
    let declared = &import.name.value;
    if import.default {
        edits.add(project, file, import.local.span, new);
    } else if !import.aliased {
        edits.add(project, file, import.name.span, &format!("{declared} as {new}"));
    } else if new == declared {
        // Drop the `as`: from the end of the declared name to the end of the alias.
        edits.add(
            project,
            file,
            Span::new(import.name.span.end, import.local.span.end),
            "",
        );
    } else {
        edits.add(project, file, import.local.span, new);
    }
    for span in local_uses(&sf.text, f, import) {
        edits.add(project, file, span, new);
    }
    Ok(edits.into_workspace_edit(docs, project, None))
}

/// Edits the import lines that name function `old_target`, for its move to `new_target`. Each path is written again
/// from the importing file's place after the move. When the declared name changes, the name in braces changes with it.
/// Without `as`, the local name is that declared name, so its uses in that file change too.
pub fn move_imports(
    project: &Project,
    old_target: &str,
    new_target: &str,
    moved: &dyn Fn(&str) -> Option<String>,
    edits: &mut Edits,
) -> Result<(), String> {
    let old = old_target.rsplit('/').next().unwrap_or(old_target);
    let new = new_target.rsplit('/').next().unwrap_or(new_target);
    let declared_changes = old != new
        && project
            .get(old_target)
            .is_some_and(|sf| name_span(&sf.text).is_some_and(|s| sf.text.get(s.start..s.end) == Some(old)));
    for (from, sf) in &project.files {
        let from_after = moved(from).unwrap_or_else(|| from.clone());
        for import in named_imports(&sf.function) {
            if target(project, from, import.reference()).as_deref() != Some(old_target) {
                continue;
            }
            // The path token keeps its quotes, so the path is the span without them.
            let path = relative_import(&from_after, new_target);
            edits.add(
                project,
                from,
                Span::new(import.path.span.start + 1, import.path.span.end - 1),
                &path,
            );
            if !declared_changes || import.default || import.name.value != old {
                continue;
            }
            if !import.aliased {
                local_clash(&sf.function, new)?;
            }
            edits.add(project, from, import.name.span, new);
            if !import.aliased {
                for span in local_uses(&sf.text, &sf.function, import) {
                    edits.add(project, from, span, new);
                }
            }
        }
    }
    Ok(())
}

/// Refuses a local name `new` that the file already uses: another import, a parameter or answer, or a std name.
fn local_clash(f: &Function, new: &str) -> Result<(), String> {
    if named_imports(f).any(|i| i.local.value == new) || declarations(f).iter().any(|d| d.name.value == new) {
        return Err(format!("`{new}` is already a name in this file"));
    }
    if stdlib::names().contains(&new) {
        return Err(format!("`{new}` is a std name"));
    }
    Ok(())
}
