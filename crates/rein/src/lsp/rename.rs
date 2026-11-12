//! Rename: a parameter, with `name=` in every call to its function; a field, in its struct and in every function
//! that takes it; a function or a shared struct, which renames its file and updates every `@./` reference to it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use lsp_types::{
    DocumentChangeOperation, DocumentChanges, OneOf, OptionalVersionedTextDocumentIdentifier, PrepareRenameResponse,
    RenameFile, RenameParams, ResourceOp, TextDocumentEdit, TextDocumentPositionParams, TextEdit, Url, WorkspaceEdit,
};
use rein_core::ast::{is_struct, StructRef, STRUCT_SUFFIX};
use rein_core::diagnostic::Span;
use rein_core::flows::MAIN_FILE;
use rein_core::lexer::KEYWORDS;
use rein_core::project::{join, skipped_folder, Project};

use super::occurrences::{declaration, name_span, param_uses, refs, symbol_at, Symbol};
use super::structs::{base as struct_base, field_sites, struct_decl, type_sites};
use super::text::{is_identifier, offset, range, relative_import};
use super::workspace::{locate, uri_for, Docs, State};

/// The span to rename and its current name. On a reference, only its last path segment, the file name, changes.
pub fn prepare(state: &State, p: &TextDocumentPositionParams) -> Result<Option<PrepareRenameResponse>, String> {
    let Some((project, name)) = locate(&state.docs, &p.text_document.uri) else { return Ok(None) };
    let Some(sf) = project.get(&name) else { return Ok(None) };
    let Some((symbol, span)) = symbol_at(&project, &name, offset(&sf.text, p.position)) else { return Ok(None) };
    if let Some(program) = other_program(&symbol) {
        return Err(program_error(program));
    }
    let (span, placeholder) = match symbol {
        Symbol::Std(n) => return Err(std_error(&n)),
        Symbol::Param { name: n, .. } => (span, n),
        Symbol::Field { name: n, .. } => (span, n),
        Symbol::Struct { name: n, file } if !is_struct(&file) => (span, n),
        Symbol::Struct { file, .. } => {
            let base = struct_base(&file).to_string();
            let written = &sf.text[span.start..span.end];
            let span = if written.ends_with(&format!("/{base}")) { Span::new(span.end - base.len(), span.end) } else { span };
            (span, base)
        }
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
    if let Some(program) = other_program(&symbol) {
        return Err(program_error(program));
    }
    let new = p.new_name.as_str();
    if !is_identifier(new) || KEYWORDS.contains(&new) {
        return Err(format!("`{new}` is not a valid name: use letters, digits and `_`, and start with a letter"));
    }
    match symbol {
        Symbol::Std(n) => Err(std_error(&n)),
        Symbol::Param { function, name: old } => rename_param(&state.docs, &project, &function, &old, new).map(Some),
        Symbol::Field { file, strukt, name: old } => rename_field(&state.docs, &project, &file, &strukt, &old, new).map(Some),
        Symbol::Struct { file, name: old } => rename_struct(state, &project, &file, &old, new).map(Some),
        Symbol::Function(target) => rename_function(state, &project, &target, new).map(Some),
    }
}

fn std_error(n: &str) -> String {
    format!("`{n}` is a std item; it cannot be renamed")
}

/// The program that holds `symbol`, when it is another installed program (its path is `@program/...`).
fn other_program(symbol: &Symbol) -> Option<&str> {
    let path = match symbol {
        Symbol::Function(f) | Symbol::Param { function: f, .. } => f,
        Symbol::Struct { file, .. } | Symbol::Field { file, .. } => file,
        Symbol::Std(_) => return None,
    };
    path.strip_prefix('@')?.split('/').next()
}

fn program_error(program: &str) -> String {
    format!("`{program}` is another program; rename does not cross programs")
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

/// Renames a field in its `interface`, in `{ }` of each function that takes the struct, in their bodies, and
/// `name=` in each call to them.
fn rename_field(docs: &Docs, project: &Project, file: &str, strukt: &str, old: &str, new: &str) -> Result<WorkspaceEdit, String> {
    let s = struct_decl(project, file, strukt).ok_or_else(|| format!("cannot find struct `{strukt}`"))?;
    if s.field(new).is_some() {
        return Err(format!("`{strukt}` already has `{new}`"));
    }
    let mut edits = Edits::default();
    if let Some(field) = s.field(old) {
        edits.add(project, file, field.name.span, new);
    }
    for (f, span) in field_sites(project, file, strukt, old) {
        edits.add(project, &f, span, new);
    }
    Ok(edits.into_workspace_edit(docs, project, None))
}

/// An inline struct renames its `interface` and the parameter type. A shared struct renames its file to
/// `<new>.struct.rein`, the last segment of each `@./` type, and its `interface` name when that was the file name.
fn rename_struct(state: &State, project: &Project, file: &str, old: &str, new: &str) -> Result<WorkspaceEdit, String> {
    let s = struct_decl(project, file, old).ok_or_else(|| format!("cannot find struct `{old}`"))?;
    let mut edits = Edits::default();
    if !is_struct(file) {
        if struct_decl(project, file, new).is_some() {
            return Err(format!("`{file}` already has a struct `{new}`"));
        }
        edits.add(project, file, s.name.span, new);
        for (f, span) in type_sites(project, file, old) {
            edits.add(project, &f, span, new);
        }
        return Ok(edits.into_workspace_edit(&state.docs, project, None));
    }
    if !state.renames_files {
        return Err("this editor cannot rename files, so a shared struct cannot be renamed".to_string());
    }
    let sf = project.get(file).ok_or_else(|| format!("cannot find `{file}`"))?;
    let base = struct_base(file);
    let new_path = sf.path.with_file_name(format!("{new}.struct.rein"));
    if new_path.exists() {
        return Err(format!("{} already exists", new_path.display()));
    }
    for (f, span) in type_sites(project, file, old) {
        edits.add(project, &f, Span::new(span.end - base.len(), span.end), new);
    }
    if s.name.value == base {
        edits.add(project, file, s.name.span, new);
    }
    let old_uri = uri_for(&state.docs, &sf.path).ok_or("the file has no URI")?;
    let new_uri = Url::from_file_path(&new_path).map_err(|()| "the new path has no URI".to_string())?;
    Ok(edits.into_workspace_edit(&state.docs, project, Some((old_uri, new_uri))))
}

/// Renames the file of `target` to `<new>.rein` in the same folder.
fn rename_function(state: &State, project: &Project, target: &str, new: &str) -> Result<WorkspaceEdit, String> {
    let new_target = match target.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/{new}"),
        None => new.to_string(),
    };
    move_function(state, project, target, &new_target)
}

/// Moves the file of `target` to the project path `new_target`, with the folder of the functions nested under
/// it (`<target>/`), so they stay in their caller's folder. Every reference to a moved file is rewritten, and
/// every relative reference inside a moved file whose path changes. A reference that only changes in its last
/// segment, as in a rename in the same folder, changes only that segment. A reference inside a moved file that
/// does not resolve keeps its target, and is refused only when the move would make it resolve to a file. The
/// editor and OS leftovers in the folder move with it. The name after `export default function` changes when it
/// was the file name.
pub fn move_function(state: &State, project: &Project, target: &str, new_target: &str) -> Result<WorkspaceEdit, String> {
    if !state.renames_files {
        return Err("this editor cannot rename files, so a function cannot be renamed".to_string());
    }
    let sf = project.get(target).ok_or_else(|| format!("cannot find function `{target}`"))?;
    let (old, new) = (basename(target), basename(new_target));
    // Each moved file, by its old project path: the target, and every file in its folder.
    let nested = format!("{target}/");
    let moved = |name: &str| -> Option<String> {
        if name == target {
            return Some(new_target.to_string());
        }
        name.strip_prefix(&nested).map(|rest| format!("{new_target}/{rest}"))
    };
    let entry = project.entry();
    if entry != target && entry.starts_with(&nested) {
        return Err(format!(
            "the folder `{nested}` holds the entry `{entry}`; moving it would leave the program without its entry"
        ));
    }
    let leftovers = check_folder(project, target)?;
    // The project names after the move: the ones a reference in a moved file may resolve to.
    let after: BTreeSet<String> = project.files.keys().map(|name| moved(name).unwrap_or_else(|| name.clone())).collect();
    let mut renames = Vec::new();
    for (name, file) in &project.files {
        let Some(to) = moved(name) else { continue };
        if !project.loads(&to) {
            return Err(format!(
                "`{to}` is a path the project does not load (a hidden, `target` or `node_modules` folder, a folder \
                 with its own `main.rein`, or a `main` below the root); moving `{name}` there would drop it from \
                 the program"
            ));
        }
        let new_path = project.root.join(format!("{to}.rein"));
        if new_path.exists() {
            return Err(format!("{} already exists", new_path.display()));
        }
        let old_uri = uri_for(&state.docs, &file.path).ok_or("the file has no URI")?;
        let new_uri = Url::from_file_path(&new_path).map_err(|()| "the new path has no URI".to_string())?;
        renames.push((old_uri, new_uri));
    }
    let folder = project.root.join(target);
    for path in leftovers {
        let rest = path.strip_prefix(&folder).map_err(|_| format!("`{}` is not in the folder", path.display()))?;
        let new_path = project.root.join(new_target).join(rest);
        if new_path.exists() {
            return Err(format!("{} already exists", new_path.display()));
        }
        let old_uri = Url::from_file_path(&path).map_err(|()| "the file has no URI".to_string())?;
        let new_uri = Url::from_file_path(&new_path).map_err(|()| "the new path has no URI".to_string())?;
        renames.push((old_uri, new_uri));
    }
    let mut edits = Edits::default();
    for (from, file) in &project.files {
        let new_from = moved(from);
        for (span, value, suffix) in references(project, from) {
            let Ok(path_now) = join(from, &value) else {
                // The reference leaves the project root or is not a path. A moved file may now reach a path.
                if let Some(new_from) = &new_from {
                    if let Ok(path) = join(new_from, &value) {
                        return Err(format!(
                            "`@{value}` in `{from}` leaves the project root, but after the move it would name \
                             `{path}`, so the move would change its meaning; fix it first"
                        ));
                    }
                }
                continue;
            };
            let now = format!("{path_now}{suffix}");
            let resolved = project.files.contains_key(&now);
            let new_to = moved(&now);
            // An unresolved reference in a file that stays put is not this move's business.
            if new_from.is_none() && (!resolved || new_to.is_none()) {
                continue;
            }
            let to_after = new_to.unwrap_or_else(|| now.clone());
            if !resolved && after.contains(&to_after) {
                return Err(format!(
                    "`@{value}` in `{from}` does not resolve, but after the move it would resolve to `{}`, so the move \
                     would change its meaning; fix it first",
                    shown_name(&to_after)
                ));
            }
            let from_now = new_from.as_deref().unwrap_or(from);
            let path = relative_import(from_now, to_after.strip_suffix(suffix).unwrap_or(&to_after));
            let written = &file.text[span.start..span.end];
            let written = written.strip_prefix('@').unwrap_or(written);
            if written == path {
                continue;
            }
            let (head, tail) = path.rsplit_once('/').unwrap_or(("", &path));
            match written.rsplit_once('/') {
                Some((h, t)) if h == head => edits.add(project, from, Span::new(span.end - t.len(), span.end), tail),
                _ => edits.add(project, from, span, &format!("@{path}")),
            }
        }
    }
    if let Some(span) = name_span(&sf.text).filter(|s| &sf.text[s.start..s.end] == old) {
        edits.add(project, target, span, new);
    }
    Ok(edits.into_workspace_edit(&state.docs, project, renames))
}

/// Refuses a move that would leave something of the folder `<root>/<target>` behind, or that would go through a
/// symbolic link. The `.rein` files the project loads move with their folder, and so do the editor and OS
/// leftovers (see [`is_leftover`]), which are returned sorted. Anything else in it blocks the move: a file that is
/// not `.rein` (such as `notes.md`), a folder the loader skips, or a folder with its own `main.rein`. Any link in
/// it refuses the move, a link to a file or to a folder alike, and this is checked before the leftover rule: a
/// link to a file would otherwise move as a leftover and be left pointing at nothing.
fn check_folder(project: &Project, target: &str) -> Result<Vec<PathBuf>, String> {
    let folder = project.root.join(target);
    let Ok(meta) = fs::symlink_metadata(&folder) else { return Ok(Vec::new()) };
    if meta.file_type().is_symlink() {
        return Err(symlink_error(&shown(project, &folder)));
    }
    if !meta.is_dir() {
        return Ok(Vec::new());
    }
    let loaded: BTreeSet<PathBuf> = project.files.values().filter_map(|f| fs::canonicalize(&f.path).ok()).collect();
    let mut leftovers = Vec::new();
    left_behind(project, &folder, &loaded, &mut leftovers)?;
    leftovers.sort();
    Ok(leftovers)
}

/// Editor and OS leftovers: a hidden file, `Thumbs.db`, `desktop.ini`, or a backup or swap file (`*~`, `*.swp`,
/// `*.swo`). They do not block a move, and the move renames them with their folder. A hidden folder is not one.
fn is_leftover(name: &str) -> bool {
    name.starts_with('.')
        || matches!(name, "Thumbs.db" | "desktop.ini")
        || name.ends_with('~')
        || name.ends_with(".swp")
        || name.ends_with(".swo")
}

/// The first thing under `dir` that a move of its functions would not carry, or that the move would only carry as a
/// leftover, added to `leftovers`: see [`check_folder`].
fn left_behind(project: &Project, dir: &Path, loaded: &BTreeSet<PathBuf>, leftovers: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read `{}`: {e}", shown(project, dir)))?;
    for entry in entries {
        let path = entry.map_err(|e| format!("cannot read `{}`: {e}", shown(project, dir)))?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        // A link is refused whatever it points to, before any other rule: `symlink_metadata` does not follow it.
        let is_link = fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
        if is_link {
            return Err(symlink_error(&shown(project, &path)));
        }
        let is_dir = path.is_dir();
        if is_dir {
            if skipped_folder(&name) || path.join(MAIN_FILE).is_file() {
                return Err(format!(
                    "`{}` is a folder the project does not load (a hidden, `target` or `node_modules` folder, or one \
                     with its own `main.rein`), so the move would leave it behind; move or remove it first",
                    shown(project, &path)
                ));
            }
            left_behind(project, &path, loaded, leftovers)?;
        } else if fs::canonicalize(&path).is_ok_and(|p| loaded.contains(&p)) {
            // A `.rein` file of the project: the move carries it.
        } else if is_leftover(&name) {
            leftovers.push(path);
        } else {
            return Err(format!(
                "`{}` is not a `.rein` file of the project, so the move would leave it behind; move or remove it first",
                shown(project, &path)
            ));
        }
    }
    Ok(())
}

fn symlink_error(path: &str) -> String {
    format!(
        "`{path}` is a symbolic link; a move would carry the link to a new place, where its target no longer \
         resolves, or take files out of the folder it points to. Move or remove the link first"
    )
}

/// The path of `path` under the project root, as the messages show it, such as `ci/fix/notes.md`.
fn shown(project: &Project, path: &Path) -> String {
    path.strip_prefix(&project.root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// Each reference written in function `name`: its span, the path as written (such as `./fix`), and the suffix that
/// the project name of the file it names has beyond that path. A shared struct's file is `<path>.struct.rein`, so
/// its suffix is `.struct`. Whether the reference resolves is left to the caller.
fn references(project: &Project, name: &str) -> Vec<(Span, String, &'static str)> {
    let Some(sf) = project.get(name) else { return Vec::new() };
    let f = &sf.function;
    let functions = refs(f).into_iter().map(|(r, _)| (r.span, r.value.clone(), ""));
    let types = f.struct_param().map(|p| p.ty.clone()).into_iter().chain(f.answer_struct_refs().into_iter().map(|(_, t)| t));
    let structs = types.filter_map(|t| match t {
        StructRef::Ref(r) => Some((r.span, r.value.clone(), STRUCT_SUFFIX)),
        StructRef::Local(_) => None,
    });
    functions.chain(structs).collect()
}

/// A project name as a reference names it: a shared struct's file, `<name>.struct.rein`, else the name itself.
fn shown_name(name: &str) -> String {
    if is_struct(name) {
        format!("{name}.rein")
    } else {
        name.to_string()
    }
}

/// Text edits per function file.
#[derive(Default)]
struct Edits(BTreeMap<String, Vec<TextEdit>>);

impl Edits {
    fn add(&mut self, project: &Project, function: &str, span: Span, new_text: &str) {
        let Some(sf) = project.get(function) else { return };
        // An edit that writes the text already there changes nothing, so it is left out.
        if sf.text.get(span.start..span.end) == Some(new_text) {
            return;
        }
        let edit = TextEdit { range: range(&sf.text, span), new_text: new_text.to_string() };
        self.0.entry(function.to_string()).or_default().push(edit);
    }

    /// The text edits of each file, then the file renames. The text edits use the old URIs, so they apply before
    /// the renames.
    fn into_workspace_edit(self, docs: &Docs, project: &Project, renames: impl IntoIterator<Item = (Url, Url)>) -> WorkspaceEdit {
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
        for (old_uri, new_uri) in renames {
            let file = RenameFile { old_uri, new_uri, options: None, annotation_id: None };
            ops.push(DocumentChangeOperation::Op(ResourceOp::Rename(file)));
        }
        WorkspaceEdit { document_changes: Some(DocumentChanges::Operations(ops)), ..Default::default() }
    }
}
