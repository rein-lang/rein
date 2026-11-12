//! Rename: a parameter, with `name=` in every call to its function; a field, in its struct and in every function
//! that takes it; a function or a shared struct, which renames its file and updates every `@./` reference to it.

use std::collections::BTreeMap;

use lsp_types::{
    DocumentChangeOperation, DocumentChanges, OneOf, OptionalVersionedTextDocumentIdentifier, PrepareRenameResponse,
    RenameFile, RenameParams, ResourceOp, TextDocumentEdit, TextDocumentPositionParams, TextEdit, Url, WorkspaceEdit,
};
use rein_core::ast::{is_struct, StructRef, STRUCT_SUFFIX};
use rein_core::diagnostic::Span;
use rein_core::lexer::KEYWORDS;
use rein_core::project::Project;

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
/// segment, as in a rename in the same folder, changes only that segment. The name after
/// `export default function` changes when it was the file name.
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
    let mut edits = Edits::default();
    for (from, file) in &project.files {
        let new_from = moved(from);
        for (span, to, suffix) in outgoing(project, from) {
            let new_to = moved(&to);
            if new_from.is_none() && new_to.is_none() {
                continue;
            }
            let (from_now, to_now) = (new_from.as_deref().unwrap_or(from), new_to.as_deref().unwrap_or(&to));
            let path = relative_import(from_now, to_now.strip_suffix(suffix).unwrap_or(to_now));
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

/// Each relative reference written in function `name` that resolves: its span, the project path of the file it
/// names, and the suffix that path has beyond the written reference. A struct type names `<name>.struct.rein`
/// without `.struct`, as the reference is written.
fn outgoing(project: &Project, name: &str) -> Vec<(Span, String, &'static str)> {
    let Some(sf) = project.get(name) else { return Vec::new() };
    let f = &sf.function;
    let functions = refs(f).into_iter().filter_map(|(r, _)| Some((r.span, project.resolve(name, &r.value).ok()?, "")));
    let types = f.struct_param().map(|p| p.ty.clone()).into_iter().chain(f.answer_struct_refs().into_iter().map(|(_, t)| t));
    let structs = types.filter_map(|t| match t {
        StructRef::Ref(r) => {
            let (file, _) = project.struct_for(name, &r.value).ok()?;
            Some((r.span, file, STRUCT_SUFFIX))
        }
        StructRef::Local(_) => None,
    });
    functions.chain(structs).collect()
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
