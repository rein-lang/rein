//! A project: every `*.rein` file under a root directory, keyed by function name.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::ast::{is_struct, walk_stmts, Binding, Callee, Function, Input, ReturnValue, Stmt, Struct, StructRef, Value, STRUCT_SUFFIX};
use crate::check::{check_function, check_length, check_line_length, Kind};
use crate::diagnostic::{Diagnostic, Span};
use crate::parser::parse;

/// The function every project starts from. It is `main.rein` at the project root.
pub const ENTRY: &str = "main";

/// The file that marks a flow root. Same name as [`flows::MAIN_FILE`].
const MAIN_FILE: &str = crate::flows::MAIN_FILE;

pub struct SourceFile {
    pub path: PathBuf,
    pub text: String,
    pub function: Function,
    pub parse_diagnostics: Vec<Diagnostic>,
}

/// Gives the root and entry function of a program by name, or why there is none.
pub type FindProgram = dyn Fn(&str) -> Result<(PathBuf, String), String>;

/// Another installed program that this project hands off to, with the entry function a hand-off starts at.
pub struct Linked {
    pub project: Box<Project>,
    pub entry: String,
}

pub struct Project {
    pub root: PathBuf,
    /// The programs named by `@program/function`, by name. Filled by [`Project::link_programs`]. A name that did
    /// not load holds the reason. Their functions are reached as `name:function`, such as `pr-handling:main`.
    pub programs: BTreeMap<String, Result<Linked, String>>,
    /// Key = function name.
    pub files: BTreeMap<String, SourceFile>,
    /// The most lines a function may have: `maxFunctionLines` in `rein.json`, else the default.
    pub max_function_lines: u64,
    /// The most characters a line may hold: `maxLineLength` in `rein.json`, else the default.
    pub max_line_length: u64,
    /// Errors in `rein.json`. They apply to every file of the project.
    manifest_diagnostics: Vec<Diagnostic>,
    /// The entry function: `entry` in `rein.json` without `.rein`, else [`ENTRY`].
    entry: String,
}

/// The `rein.json` field that sets the line limit of a function.
const MAX_LINES_FIELD: &str = "maxFunctionLines";

/// The `rein.json` field that sets the most characters a line may hold.
const MAX_LINE_LENGTH_FIELD: &str = "maxLineLength";

/// The limits from `root/rein.json`: the function line limit, the line length limit, and an error for each field
/// that is not a positive integer. A missing or unreadable manifest, or one that is not JSON, gives the defaults,
/// as the program reader does.
fn read_limits(root: &Path) -> (u64, u64, Vec<Diagnostic>) {
    let manifest = fs::read_to_string(root.join(crate::flows::MANIFEST))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let mut errors = Vec::new();
    let mut limit = |field: &str, default: u64| {
        let Some(v) = manifest.as_ref().and_then(|m| m.get(field)) else { return default };
        v.as_u64().filter(|n| *n > 0).unwrap_or_else(|| {
            errors.push(Diagnostic::error(
                Span::default(),
                format!("`{field}` in {} must be a positive integer, such as {default}; found {v}", crate::flows::MANIFEST),
            ));
            default
        })
    };
    let lines = limit(MAX_LINES_FIELD, crate::check::DEFAULT_MAX_FUNCTION_LINES);
    let length = limit(MAX_LINE_LENGTH_FIELD, crate::check::DEFAULT_MAX_LINE_LENGTH);
    (lines, length, errors)
}

/// The entry function of the program at `root`: `entry` in `rein.json` without `.rein`, else [`ENTRY`].
fn read_entry(root: &Path) -> String {
    fs::read_to_string(root.join(crate::flows::MANIFEST))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|m| Some(crate::flows::entry_name(m.get("entry")?.as_str()?)))
        .unwrap_or_else(|| ENTRY.to_string())
}

impl Project {
    /// Nearest ancestor of `start` (or its parent, if `start` is a file) that
    /// holds a `main.rein`. A flow folder in `rein/<flow>/` is such a directory. Falls back to the start directory.
    /// The walk uses the absolute path, not the symlink target, so a linked file belongs to the
    /// project that holds the link.
    pub fn find_root(start: &Path) -> PathBuf {
        let start = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
        let base = if start.is_file() {
            start.parent().unwrap_or(start.as_path()).to_path_buf()
        } else {
            start
        };
        base.ancestors()
            .find(|d| d.join(MAIN_FILE).is_file())
            .map_or_else(|| base.clone(), Path::to_path_buf)
    }

    /// Reads every `*.rein` file under `root`. Skips hidden dirs, `target/` and `node_modules/`.
    /// Follows symlinks. A file that cannot be read is kept with an error diagnostic.
    pub fn load(root: &Path) -> io::Result<Self> {
        let mut sources = Vec::new();
        collect(root, &mut sources, &mut BTreeSet::new())?;
        Ok(Self::build(root.to_path_buf(), sources))
    }

    /// Builds a project from (path, text) pairs. The LSP uses this to overlay open buffers.
    /// Files outside `root` are ignored. A later pair for the same name replaces an earlier one.
    pub fn from_sources(root: PathBuf, sources: Vec<(PathBuf, String)>) -> Self {
        Self::build(root, sources.into_iter().map(|(p, t)| (p, Ok(t))).collect())
    }

    fn build(root: PathBuf, sources: Vec<(PathBuf, io::Result<String>)>) -> Self {
        let mut files = BTreeMap::new();
        for (path, read) in sources {
            let Some(name) = name_under(&root, &path) else { continue };
            let (text, function, parse_diagnostics) = match read {
                Ok(text) => {
                    let (function, parse_diagnostics) = parse(&text);
                    (text, function, parse_diagnostics)
                }
                Err(e) => (
                    String::new(),
                    Function::default(),
                    vec![Diagnostic::error(Span::default(), format!("cannot read file: {e}"))],
                ),
            };
            files.insert(name, SourceFile { path, text, function, parse_diagnostics });
        }
        let (max_function_lines, max_line_length, manifest_diagnostics) = read_limits(&root);
        let entry = read_entry(&root);
        let mut project = Self {
            root,
            files,
            programs: BTreeMap::new(),
            max_function_lines,
            max_line_length,
            manifest_diagnostics,
            entry,
        };
        project.bind_structs();
        project
    }

    /// Binds each `{ ... }: @./name` parameter to the `export default interface` of `name.struct.rein`.
    fn bind_structs(&mut self) {
        // `(file, answer name or None for the parameter, struct)`.
        let mut found: Vec<(String, Option<String>, Result<Struct, String>)> = Vec::new();
        for (name, sf) in &self.files {
            let f = &sf.function;
            let param = f.struct_param().map(|p| (None, p.ty.clone()));
            let answers = f.answer_struct_refs().into_iter().map(|(a, r)| (Some(a), r));
            for (answer, r) in param.into_iter().chain(answers) {
                if let StructRef::Ref(r) = &r {
                    found.push((name.clone(), answer, self.struct_for(name, &r.value).map(|(_, s)| s.clone())));
                }
            }
        }
        for (name, answer, s) in found {
            if let Some(sf) = self.files.get_mut(&name) {
                match answer {
                    None => sf.function.bind_struct(s),
                    Some(a) => sf.function.bind_answer_struct(&a, s),
                }
            }
        }
    }

    /// The struct that the parameter type `@<source>`, written in `from`, names: the `export default interface`
    /// of `<source>.struct.rein`. Returns the project path of that file, such as `ticket.struct`, and the struct.
    pub fn struct_for(&self, from: &str, source: &str) -> Result<(String, &Struct), String> {
        let name = format!("{}{STRUCT_SUFFIX}", join(from, source)?);
        let base = source.rsplit('/').next().unwrap_or(source);
        self.files
            .get(&name)
            .and_then(|sf| sf.function.exported_struct())
            .map(|s| (name.clone(), s))
            .ok_or_else(|| format!("cannot find struct `@{source}`; a shared struct is a `{base}.struct.rein` file"))
    }

    /// The function files. A `.struct.rein` file holds a struct, so it is left out.
    pub fn functions(&self) -> impl Iterator<Item = (&String, &SourceFile)> {
        self.files.iter().filter(|(n, _)| !is_struct(n))
    }

    /// Function name for a path under the root. Invalid names are returned as-is.
    pub fn name_for(&self, path: &Path) -> Option<String> {
        name_under(&self.root, path)
    }

    /// A function by project path. `@program/path` is a function inside a linked program.
    pub fn get(&self, name: &str) -> Option<&SourceFile> {
        match name.strip_prefix('@') {
            Some(inner) => {
                let (program, path) = inner.split_once('/')?;
                self.linked(program)?.project.get(path)
            }
            None => self.files.get(name),
        }
    }

    fn linked(&self, program: &str) -> Option<&Linked> {
        self.programs.get(program)?.as_ref().ok()
    }

    /// Loads each program that a `@program` or `@program/function` names. `find` gives the root and entry of a
    /// program by name, as the scopes resolve it. The CLI passes it, because scopes live on the machine, not here.
    pub fn link_programs(&mut self, find: &FindProgram) {
        let names: BTreeSet<String> = self
            .files
            .values()
            .flat_map(|sf| keys(&sf.function))
            .filter(|k| is_program_key(k))
            .map(|k| k.split('/').next().unwrap_or_default().to_string())
            .collect();
        for name in names {
            let loaded = find(&name).and_then(|(root, entry)| {
                let project = Project::load(&root).map_err(|e| format!("cannot read program `{name}`: {e}"))?;
                Ok(Linked { project: Box::new(project), entry })
            });
            self.programs.insert(name, loaded);
        }
    }

    /// The function to run after a hand-off from `from` to `target`. A target in another program is already
    /// `@program/path`. Inside a linked program (`from` is `@program/path`), a local target stays in that program.
    pub fn qualify(&self, from: &str, target: &str) -> String {
        match from.strip_prefix('@').and_then(|f| f.split_once('/')) {
            Some((program, _)) if !target.starts_with('@') => format!("@{program}/{target}"),
            _ => target.to_string(),
        }
    }

    /// Diagnostics that belong to the project, not to one function. They point at `main.rein`.
    pub fn project_diagnostics(&self) -> Vec<Diagnostic> {
        if self.files.contains_key(ENTRY) {
            return Vec::new();
        }
        vec![Diagnostic::error(Span::default(), "project has no main.rein; it is the agent's entry point")]
    }

    /// Errors in `rein.json`, such as a `maxFunctionLines` or a `maxLineLength` that is not a positive integer.
    pub fn manifest_diagnostics(&self) -> &[Diagnostic] {
        &self.manifest_diagnostics
    }

    /// The entry file, `main.rein` in the root. Project errors point here.
    pub fn marker_path(&self) -> PathBuf {
        self.root.join(MAIN_FILE)
    }

    /// Resolves an import path written in function `from` to the project path of a function.
    /// The path starts with `./` or `../`, has no extension, and is relative to the importing file's directory.
    pub fn resolve(&self, from: &str, source: &str) -> Result<String, String> {
        let name = join(from, source)?;
        if self.files.contains_key(&name) {
            Ok(name)
        } else {
            Err(format!("cannot find function `@{source}`"))
        }
    }

    /// What each callee and reference of `name` refers to, keyed by [`Callee::key`]. Unresolved ones are left out.
    pub fn bindings(&self, name: &str) -> BTreeMap<String, Binding<'_>> {
        if let Some((program, path)) = name.strip_prefix('@').and_then(|n| n.split_once('/')) {
            return self.linked(program).map_or_else(BTreeMap::new, |l| l.project.bindings(path));
        }
        let Some(sf) = self.files.get(name) else { return BTreeMap::new() };
        keys(&sf.function)
            .into_iter()
            .filter_map(|key| Some((key.clone(), self.binding_for(name, &key).ok()?)))
            .collect()
    }

    /// What a callee or reference written in function `from` refers to, or why it does not resolve.
    /// A std name (no `./` or `../`) is a std item. Anything else is a path relative to `from`.
    pub fn binding_for(&self, from: &str, key: &str) -> Result<Binding<'_>, String> {
        if is_program_key(key) {
            return self.program_binding(key);
        }
        if !(key.starts_with("./") || key.starts_with("../")) {
            return crate::stdlib::item(key);
        }
        let target = self.resolve(from, key)?;
        let function = &self.files.get(&target).ok_or_else(|| format!("cannot find function `@{key}`"))?.function;
        Ok(if function.is_template() {
            Binding::Template { path: Some(target), function }
        } else {
            Binding::Function(target)
        })
    }

    /// What `@program` or `@program/function` names: the function in the linked program, entry when bare.
    fn program_binding(&self, key: &str) -> Result<Binding<'_>, String> {
        let (program, path) = key.split_once('/').map_or((key, None), |(p, f)| (p, Some(f)));
        let linked = match self.programs.get(program) {
            Some(Ok(linked)) => linked,
            Some(Err(reason)) => return Err(reason.clone()),
            None => return Err(unknown_program(program)),
        };
        let path = path.unwrap_or(&linked.entry);
        let function = &linked
            .project
            .files
            .get(path)
            .ok_or_else(|| format!("cannot find function `{path}` in program `{program}`"))?
            .function;
        let target = format!("@{program}/{path}");
        Ok(if function.is_template() { Binding::Template { path: Some(target), function } } else { Binding::Function(target) })
    }

    /// What a reference written in function `from` names, for the checker. A function reports the names of its
    /// `string` parameters, and a template the names of all its parameters, so a call can bind them by name.
    fn kind_for(&self, from: &str, key: &str) -> Result<Kind, String> {
        Ok(match self.binding_for(from, key)? {
            Binding::Function(path) => Kind::Function {
                strings: self.get(&path).map_or_else(Vec::new, |sf| param_names(sf.function.text_inputs())),
            },
            Binding::Template { function, .. } => Kind::Template { params: param_names(function.inputs.iter()) },
            Binding::Request => Kind::Request,
        })
    }

    /// Diagnostics per function, sorted by span: parse, single-file checks, cycles, and reachability.
    /// A file with a syntax error reports only that error. Its tree is partial.
    pub fn check(&self) -> BTreeMap<String, Vec<Diagnostic>> {
        let adj = self.adjacency();
        let reachable = self.reachable_from_entry(&adj);
        let placements = self.placements(&adj, reachable.as_ref());
        self.files
            .iter()
            .map(|(name, sf)| {
                if !sf.parse_diagnostics.is_empty() {
                    return (name.clone(), sf.parse_diagnostics.clone());
                }
                let mut diags = Vec::new();
                let resolve = |path: &str| self.kind_for(name, path);
                diags.extend(check_function(&sf.function, name, &resolve));
                if !is_struct(name) {
                    diags.extend(check_length(&sf.function, name, &sf.text, self.max_function_lines));
                }
                diags.extend(check_line_length(&sf.text, self.max_line_length));
                let bindings = self.bindings(name);
                diags.extend(self.cycle_warnings(name, &sf.function, &bindings, &adj));
                if !is_struct(name) && reachable.as_ref().is_some_and(|r| !r.contains(name)) {
                    let span = sf.function.doc.as_ref().map_or_else(Span::default, |d| d.span);
                    let entry = &self.entry;
                    diags.push(Diagnostic::warning(span, format!("`{name}` is not reachable from {entry}")));
                }
                if let Some(placement) = placements.get(name) {
                    let span = sf.function.doc.as_ref().map_or_else(Span::default, |d| d.span);
                    diags.push(Diagnostic::warning(span, placement.message(name)));
                }
                diags.sort_by_key(|d| (d.span.start, d.span.end));
                (name.clone(), diags)
            })
            .collect()
    }

    /// Where function `name` belongs when one other function calls it: in a folder named after that caller,
    /// next to the caller's file. Callees of the entry stay in the entry's folder, the program root.
    /// None when `name` is a struct, the entry, not reachable from the entry, has no caller or more than one,
    /// is already in place, or cannot move there because the path is taken or another function belongs there too.
    pub fn nest_target(&self, name: &str) -> Option<String> {
        let adj = self.adjacency();
        let reachable = self.reachable_from_entry(&adj);
        match self.placements(&adj, reachable.as_ref()).remove(name)? {
            Placement { home, clash, .. } if clash.is_empty() => Some(home),
            _ => None,
        }
    }

    /// The nesting warning of each function that is not where the rule puts it. A function not reachable from
    /// the entry is left out: the reachability warning covers it, and unreachable functions that call each
    /// other would each be told to move into the other's folder. A target path that another file already has
    /// is left out too, since the move cannot happen. Two functions with one target name each other.
    fn placements(
        &self,
        adj: &BTreeMap<String, BTreeSet<String>>,
        reachable: Option<&BTreeSet<String>>,
    ) -> BTreeMap<String, Placement> {
        let callers = callers_of(adj);
        let mut out: BTreeMap<String, Placement> = BTreeMap::new();
        for (name, _) in self.functions() {
            if reachable.is_some_and(|r| !r.contains(name)) {
                continue;
            }
            let Some((caller, home)) = self.misplaced(name, &callers) else { continue };
            if self.files.contains_key(&home) {
                continue;
            }
            out.insert(name.clone(), Placement { caller: caller.to_string(), home, clash: Vec::new() });
        }
        let homes: Vec<(String, String)> = out.iter().map(|(n, p)| (n.clone(), p.home.clone())).collect();
        for (name, placement) in &mut out {
            placement.clash =
                homes.iter().filter(|(n, h)| n != name && *h == placement.home).map(|(n, _)| n.clone()).collect();
        }
        out
    }

    /// The only caller of `name` and the project path `name` should have, when it is not there yet.
    fn misplaced<'a>(&self, name: &str, callers: &'a BTreeMap<String, BTreeSet<String>>) -> Option<(&'a str, String)> {
        if is_struct(name) || name == self.entry {
            return None;
        }
        let mut only = callers.get(name)?.iter();
        let (Some(caller), None) = (only.next(), only.next()) else { return None };
        let base = name.rsplit('/').next().unwrap_or(name);
        let folder = if *caller == self.entry { caller.rsplit_once('/').map(|(d, _)| d) } else { Some(caller.as_str()) };
        let home = folder.map_or_else(|| base.to_string(), |d| format!("{d}/{base}"));
        (home != name).then_some((caller.as_str(), home))
    }

    /// Distinct `(function, reference)` pairs for every `@program` or `@program/function` in the project.
    pub fn program_refs(&self) -> Vec<(String, String)> {
        let pairs = self.files.iter().flat_map(|(name, sf)| {
            keys(&sf.function).into_iter().filter(|k| is_program_key(k)).map(move |k| (name.clone(), k))
        });
        pairs.collect()
    }

    /// Distinct `(from, to)` pairs for every return of an imported function that resolves.
    pub fn references(&self) -> Vec<(String, String)> {
        self.files
            .iter()
            .flat_map(|(name, sf)| {
                let bindings = self.bindings(name);
                ref_sites(&sf.function, &bindings).into_iter().map(move |(to, _)| (name.clone(), to))
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Functions reachable from the entry (`main`, or `entry` in `rein.json`) through references. None when the
    /// entry is missing or has a syntax error.
    fn reachable_from_entry(&self, adj: &BTreeMap<String, BTreeSet<String>>) -> Option<BTreeSet<String>> {
        let entry = self.files.get(&self.entry)?;
        if !entry.parse_diagnostics.is_empty() {
            return None;
        }
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([self.entry.clone()]);
        while let Some(name) = queue.pop_front() {
            if seen.insert(name.clone()) {
                queue.extend(adj.get(&name).into_iter().flatten().cloned());
            }
        }
        Some(seen)
    }

    /// Edges between functions: each resolved import that is returned.
    fn adjacency(&self) -> BTreeMap<String, BTreeSet<String>> {
        self.files
            .iter()
            .map(|(name, sf)| {
                let bindings = self.bindings(name);
                let edges = ref_sites(&sf.function, &bindings).into_iter().map(|(t, _)| t).collect();
                (name.clone(), edges)
            })
            .collect()
    }

    /// One warning per return in `f` whose target can reach back to `name`.
    fn cycle_warnings(
        &self,
        name: &str,
        f: &Function,
        bindings: &BTreeMap<String, Binding<'_>>,
        adj: &BTreeMap<String, BTreeSet<String>>,
    ) -> Vec<Diagnostic> {
        ref_sites(f, bindings)
            .into_iter()
            .filter_map(|(t, span)| {
                let back = path_between(adj, &t, name)?;
                let chain: Vec<&str> =
                    std::iter::once(name).chain(back.iter().map(String::as_str)).collect();
                Some(Diagnostic::warning(
                    span,
                    format!("reference cycle: {}; Rein has no loops yet", chain.join(" -> ")),
                ))
            })
            .collect()
    }
}

/// Where the nesting rule puts a function that is not there yet.
struct Placement {
    /// The only caller.
    caller: String,
    /// The project path the function belongs at.
    home: String,
    /// Other functions that belong at the same path. Then none of them can move until one is renamed.
    clash: Vec<String>,
}

impl Placement {
    /// The warning for function `name`.
    fn message(&self, name: &str) -> String {
        let Placement { caller, home, clash } = self;
        if clash.is_empty() {
            return format!(
                "`{name}` is only called by `{caller}`; move it to `{home}.rein` so the folders show the flow. \
                 The move changes its answer keys and breaks `@<program>/{name}` references from other programs"
            );
        }
        let others: Vec<String> = clash.iter().map(|c| format!("`{c}`")).collect();
        format!(
            "`{name}` is only called by `{caller}`, but `{home}.rein` is also the place for {}; rename one of them, \
             then move each into the folder of its caller",
            others.join(", ")
        )
    }
}

/// The callers of each function, from the edges. A function that calls itself is not its own caller.
fn callers_of(adj: &BTreeMap<String, BTreeSet<String>>) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (from, tos) in adj {
        for to in tos.iter().filter(|t| *t != from) {
            out.entry(to.clone()).or_default().insert(from.clone());
        }
    }
    out
}

/// The project path that `@<source>`, written in function `from`, points to. The path starts with `./` or
/// `../`, has no extension, is relative to the folder of `from`, and stays inside the root.
fn join(from: &str, source: &str) -> Result<String, String> {
    if !(source.starts_with("./") || source.starts_with("../")) {
        return Err(format!("references are relative: write `@./{source}`"));
    }
    if source.rsplit('/').next().is_some_and(|last| last.contains('.') && last != "..") {
        return Err("references have no extension".into());
    }
    // The importing file's directory, as name segments. `ci/fix` starts in `ci`.
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for seg in source.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!("`@{source}` leaves the project root"));
                }
            }
            s => parts.push(s),
        }
    }
    Ok(parts.join("/"))
}

/// Walks `dir`. `seen` holds canonical directories already visited, so a symlink loop ends.
/// Unreadable subdirectories are skipped. Only an unreadable `dir` itself is an error.
fn collect(
    dir: &Path,
    out: &mut Vec<(PathBuf, io::Result<String>)>,
    seen: &mut BTreeSet<PathBuf>,
) -> io::Result<()> {
    if !seen.insert(fs::canonicalize(dir)?) {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().into_owned();
        // `metadata` follows symlinks. A broken link fails here and is skipped.
        let Ok(meta) = fs::metadata(&path) else { continue };
        if meta.is_dir() {
            // A subdirectory with its own main.rein is another flow. Skip it.
            if file_name.starts_with('.') || file_name == "target" || file_name == "node_modules" || path.join(MAIN_FILE).is_file() {
                continue;
            }
            let _ = collect(&path, out, seen);
        } else if meta.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rein") {
            let read = fs::read_to_string(&path);
            out.push((path, read));
        }
    }
    Ok(())
}

/// Whether a reference key names another program: it is not a relative path and not a std name.
fn is_program_key(key: &str) -> bool {
    !(key.starts_with("./") || key.starts_with("../")) && crate::stdlib::item(key).is_err()
}

/// Why a `@program` has no program behind it.
fn unknown_program(name: &str) -> String {
    format!("cannot find program `{name}`; run `rein list` to list programs")
}

/// The names of some parameters, in order.
fn param_names<'a>(inputs: impl Iterator<Item = &'a Input>) -> Vec<String> {
    inputs.map(|i| i.name.value.clone()).collect()
}

/// `ci/fix.rein` under `root` gives `ci/fix`.
fn name_under(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    if rel.extension()?.to_str()? != "rein" {
        return None;
    }
    let parts: Vec<String> = rel
        .with_extension("")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("/"))
}

/// Every callee and reference key used in `f`, sorted and without repeats.
fn keys(f: &Function) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk_stmts(&f.body, &mut |s| match s {
        Stmt::Return(r) => match &r.value {
            ReturnValue::Ref(reference) => {
                out.insert(reference.value.clone());
            }
            ReturnValue::Call(call) => out.extend(call.calls().into_iter().map(|c| c.callee.key().to_string())),
            ReturnValue::Text(_) | ReturnValue::Template(_) => {}
        },
        Stmt::Request { callee, arg, .. } => {
            out.insert(callee.value.clone());
            if let Value::Call(call) = arg {
                out.extend(call.calls().into_iter().map(|c| c.callee.key().to_string()));
            }
        }
        _ => {}
    });
    out
}

/// Edges from `f` to project functions: each return of a reference, and each call of a reference.
/// Returns (target project path, span of the statement). Std names are not edges.
fn ref_sites(f: &Function, bindings: &BTreeMap<String, Binding<'_>>) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    walk_stmts(&f.body, &mut |s| {
        let mut push = |key: &str| match bindings.get(key) {
            // A program is another project, not a node of this one.
            Some(Binding::Function(path)) if !path.starts_with('@') => out.push((path.clone(), s.span())),
            Some(Binding::Template { path: Some(path), .. }) if !path.starts_with('@') => out.push((path.clone(), s.span())),
            _ => {}
        };
        let callee_is_ref = |c: &crate::ast::Call| matches!(c.callee, Callee::Ref(_));
        match s {
            Stmt::Return(r) => match &r.value {
                ReturnValue::Ref(reference) => push(&reference.value),
                ReturnValue::Call(call) => call.calls().into_iter().filter(|c| callee_is_ref(c)).for_each(|c| push(c.callee.key())),
                _ => {}
            },
            Stmt::Request { arg: Value::Call(call), .. } => {
                call.calls().into_iter().filter(|c| callee_is_ref(c)).for_each(|c| push(c.callee.key()));
            }
            _ => {}
        }
    });
    out
}

/// Shortest chain of functions from `from` to `to`, both included.
fn path_between(adj: &BTreeMap<String, BTreeSet<String>>, from: &str, to: &str) -> Option<Vec<String>> {
    let mut prev: BTreeMap<&str, &str> = BTreeMap::new();
    let mut seen: BTreeSet<&str> = BTreeSet::from([from]);
    let mut queue: VecDeque<&str> = VecDeque::from([from]);
    while let Some(node) = queue.pop_front() {
        if node == to {
            let mut path = vec![node.to_string()];
            let mut cur = node;
            while let Some(&p) = prev.get(cur) {
                path.push(p.to_string());
                cur = p;
            }
            path.reverse();
            return Some(path);
        }
        for next in adj.get(node).into_iter().flatten().map(String::as_str) {
            if seen.insert(next) {
                prev.insert(next, node);
                queue.push_back(next);
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;
