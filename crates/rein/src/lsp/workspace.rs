//! The open buffers, and the projects built from them and the files on disk. Paths, URIs and locations.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use lsp_types::{InitializeParams, Location, ResourceOperationKind, Url};
use rein_core::diagnostic::Span;
use rein_core::project::Project;

use super::text::range;

/// Open editor buffers by document URI. They replace the files on disk.
pub type Docs = HashMap<Url, String>;

/// What the server keeps between messages.
#[derive(Default)]
pub struct State {
    pub docs: Docs,
    /// Workspace folders from `initialize`. Workspace symbols search the programs under them.
    pub folders: Vec<PathBuf>,
    /// The client applies file renames in a workspace edit, so a function can be renamed.
    pub renames_files: bool,
}

impl State {
    pub fn new(init: &InitializeParams) -> Self {
        #[allow(deprecated)] // Older clients send only `rootUri`.
        let root = init.root_uri.iter();
        let folders = init
            .workspace_folders
            .iter()
            .flatten()
            .map(|f| &f.uri)
            .chain(root)
            .filter_map(|u| fs::canonicalize(u.to_file_path().ok()?).ok())
            .collect();
        let renames_files = init
            .capabilities
            .workspace
            .as_ref()
            .and_then(|w| w.workspace_edit.as_ref())
            .and_then(|e| e.resource_operations.as_ref())
            .is_some_and(|ops| ops.contains(&ResourceOperationKind::Rename));
        Self { docs: Docs::new(), folders, renames_files }
    }
}

/// Project under `root`: files on disk, with open buffers replacing their disk copies.
/// A buffer the loader would skip, such as one in a nested project, is left out, as its file is left out of this one.
pub fn build_project(docs: &Docs, root: &Path) -> Project {
    let mut by_path: BTreeMap<PathBuf, String> = BTreeMap::new();
    if let Ok(disk) = Project::load(root) {
        by_path.extend(disk.files.into_values().map(|sf| (sf.path, sf.text)));
    }
    by_path.extend(docs.iter().filter_map(|(uri, text)| {
        let path = Project::loaded_path(root, &doc_path(uri)?)?;
        Some((path, text.clone()))
    }));
    let mut project = Project::from_sources(root.to_path_buf(), by_path.into_iter().collect());
    // The scopes are searched only when a `@program` reference names a program. Scopes resolve from the project root,
    // as `rein check` run there does. The server's own directory is unrelated.
    if !project.program_refs().is_empty() {
        let visible = crate::scopes::discover_from(root);
        // Open buffers of other programs replace their files on disk, as they do in this project.
        let open: Vec<(PathBuf, String)> =
            docs.iter().filter_map(|(uri, text)| Some((doc_path(uri)?, text.clone()))).collect();
        project.link_programs_with(&move |name| crate::scopes::find_in(&visible, name), &open);
    }
    project
}

/// The project of `uri` and the function name of that file.
pub fn locate(docs: &Docs, uri: &Url) -> Option<(Project, String)> {
    let path = doc_path(uri)?;
    let project = build_project(docs, &Project::find_root(&path));
    let name = project.name_for(&path)?;
    project.get(&name)?;
    Some((project, name))
}

/// Where `span` is in the file of function `name`.
pub fn location(docs: &Docs, project: &Project, name: &str, span: Span) -> Option<Location> {
    let sf = project.get(name)?;
    Some(Location::new(uri_for(docs, &sf.path)?, range(&sf.text, span)))
}

/// The URI the client uses for `path`. For an open document, that is the URI the client sent.
pub fn uri_for(docs: &Docs, path: &Path) -> Option<Url> {
    docs.keys()
        .find(|u| doc_path(u).as_deref() == Some(path))
        .cloned()
        .or_else(|| Url::from_file_path(path).ok())
}

pub fn doc_path(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok().map(canonical)
}

/// Resolves symlinks in the directory part, so a file that does not exist yet still matches
/// the canonical project root.
pub fn canonical(path: PathBuf) -> PathBuf {
    if let (Some(dir), Some(name)) = (path.parent(), path.file_name()) {
        if let Ok(dir) = fs::canonicalize(dir) {
            return dir.join(name);
        }
    }
    path
}
