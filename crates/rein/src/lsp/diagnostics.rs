//! Diagnostics: re-checks the projects that hold a changed file, then publishes the checker's errors and warnings.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use lsp_server::{Connection, Message, Notification};
use lsp_types::{Diagnostic, DiagnosticSeverity, PublishDiagnosticsParams, Url};
use rein_core::diagnostic::{Diagnostic as ReinDiagnostic, Severity};
use rein_core::project::Project;

use super::definition::std_dir;
use super::text::range;
use super::workspace::{build_project, doc_path, uri_for, Docs};
use super::BoxResult;

/// Re-checks the projects that hold `changed` and every file published so far. Then it publishes.
/// A file gets diagnostics from the project its own path resolves to. A file that is no longer in
/// such a project gets an empty list, so its errors clear. A std source opened by go to definition gets none.
pub fn refresh(connection: &Connection, docs: &Docs, published: &mut BTreeSet<Url>, changed: &Url) -> BoxResult<()> {
    let Some(path) = doc_path(changed) else { return Ok(()) };
    if path.starts_with(std_dir()) {
        return Ok(());
    }
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
        // The client does not open main.rein, so project errors go on an open file of the project.
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
