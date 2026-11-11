//! Workspace symbols: every function of every program under the workspace folders, matched by name.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use lsp_types::{OneOf, SymbolKind, WorkspaceSymbol, WorkspaceSymbolParams, WorkspaceSymbolResponse};
use rein_core::project::Project;

use super::definition::std_dir;
use super::occurrences::header_span;
use super::workspace::{build_project, doc_path, location, State};

/// The programs under the workspace folders and those of the open files. Each function's name is its project
/// path, such as `billing/refund`, and its container is the program folder.
pub fn workspace_symbols(state: &State, p: &WorkspaceSymbolParams) -> Option<WorkspaceSymbolResponse> {
    let mut roots = BTreeSet::new();
    for folder in &state.folders {
        program_roots(folder, &mut roots);
    }
    let open = state.docs.keys().filter_map(doc_path).filter(|p| !p.starts_with(std_dir()));
    roots.extend(open.map(|p| Project::find_root(&p)));
    let query = p.query.to_lowercase();
    let mut out = Vec::new();
    for root in roots {
        let project = build_project(&state.docs, &root);
        let program = root.file_name().map(|n| n.to_string_lossy().into_owned());
        for name in project.files.keys().filter(|n| fuzzy(&query, &n.to_lowercase())) {
            let Some(at) = location(&state.docs, &project, name, header_span(&project, name)) else { continue };
            out.push(WorkspaceSymbol {
                name: name.clone(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                container_name: program.clone(),
                location: OneOf::Left(at),
                data: None,
            });
        }
    }
    Some(WorkspaceSymbolResponse::Nested(out))
}

/// Every directory under `dir` that holds a `main.rein`, `dir` included. Skips hidden dirs, `target/` and
/// `node_modules/`.
fn program_roots(dir: &Path, out: &mut BTreeSet<PathBuf>) {
    if dir.join("main.rein").is_file() {
        out.insert(dir.to_path_buf());
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // `file_type` does not follow symlinks, so a link loop cannot recurse.
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if is_dir && !name.starts_with('.') && name != "target" && name != "node_modules" {
            program_roots(&entry.path(), out);
        }
    }
}

/// Each character of `query` appears in `name`, in order. An empty query matches every name.
fn fuzzy(query: &str, name: &str) -> bool {
    let mut chars = name.chars();
    query.chars().all(|q| chars.any(|c| c == q))
}

#[cfg(test)]
mod tests {
    use super::fuzzy;

    #[test]
    fn fuzzy_matches_characters_in_order() {
        assert!(fuzzy("bref", "billing/refund"));
        assert!(fuzzy("", "main"));
        assert!(!fuzzy("fb", "billing/refund"));
    }
}
