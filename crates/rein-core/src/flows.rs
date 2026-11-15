//! Program discovery: scoped program folders, each holding one program per subfolder.
//!
//! A program is a directory with a `rein.json` manifest or a `main.rein`. Programs live in three
//! scopes: project (a visible `rein/` folder in the working directory or any ancestor), user (the
//! user programs folder, `~/.config/rein/programs`), and system (each directory in the system list,
//! such as `/etc/rein/`). On a name clash the closest scope wins: project, then user, then system.
//! Inside the project scope, the nearest folder wins.
//!
//! Discovery takes its inputs as arguments and reads no environment variables, so tests can pass
//! temp directories.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::parser::parse;

/// The file that marks a flow root and holds its entry function.
pub const MAIN_FILE: &str = "main.rein";

/// The program manifest: `{"name": "...", "description": "...", "entry": "main.rein"}`. Each field is optional.
pub const MANIFEST: &str = "rein.json";

/// The folder name of the project scope folder.
pub const FLOW_DIR: &str = "rein";

/// The folder under the user programs folder that holds the pnpm project of installed packages.
pub const NPM_DIR: &str = "npm";

/// The package manifest. Name, version, description and `engines.rein` come from it.
pub const PACKAGE: &str = "package.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    Project,
    User,
    System,
}

impl Scope {
    /// The word the CLI prints and the JSON carries.
    pub fn label(self) -> &'static str {
        match self {
            Scope::Project => "project",
            Scope::User => "user",
            Scope::System => "system",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    /// The manifest name, else the folder name, such as `merge-pr`.
    pub name: String,
    pub scope: Scope,
    /// The flow root: the folder that holds `main.rein`.
    pub root: PathBuf,
    /// The doc comment of `main.rein`, flattened to its first paragraph. Empty when there is none.
    pub description: String,
    /// The entry function path: the manifest `entry` without `.rein`. Default `main`.
    pub entry: String,
    /// The scope of the flow that wins this name, when this one is shadowed.
    pub shadowed_by: Option<Scope>,
    /// The package version from `package.json`, if any.
    pub version: Option<String>,
    /// The Rein version range from `package.json` `engines.rein`, if any.
    pub engines_rein: Option<String>,
}

/// Every flow visible from `cwd`, in precedence order, with shadowed flows marked.
///
/// `cwd` must be absolute. `user_dir` is the user scope folder, normally `~/.config/rein/programs`. It is not also a
/// project scope when it is an ancestor of `cwd`. `system_dirs` are checked in order; the first one
/// wins a clash. A folder under a scoped `rein/` that has no `main.rein` is not a flow and is skipped.
pub fn discover(cwd: &Path, user_dir: Option<&Path>, system_dirs: &[PathBuf]) -> Vec<Flow> {
    let mut found = Vec::new();
    for dir in cwd.ancestors() {
        let scoped = dir.join(FLOW_DIR);
        if user_dir == Some(scoped.as_path()) {
            continue;
        }
        if scoped.is_dir() {
            found.extend(flows_in(&scoped, Scope::Project));
            found.extend(npm_programs(&scoped.join(NPM_DIR), Scope::Project));
        }
    }
    if let Some(user) = user_dir {
        found.extend(flows_in(user, Scope::User));
        found.extend(npm_programs(&user.join(NPM_DIR), Scope::User));
    }
    for dir in system_dirs {
        found.extend(flows_in(dir, Scope::System));
    }
    mark_shadowed(found)
}

/// The visible flow called `name`: the first one in precedence order that is not shadowed.
pub fn resolve<'a>(flows: &'a [Flow], name: &str) -> Option<&'a Flow> {
    flows.iter().find(|f| f.name == name && f.shadowed_by.is_none())
}

/// A block for an agent's context: what Rein flows are, the visible flows, and how to use them.
/// Empty when no flow is visible.
pub fn agent_context(flows: &[Flow]) -> String {
    let visible: Vec<&Flow> = flows.iter().filter(|f| f.shadowed_by.is_none()).collect();
    if visible.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "Rein processes are step-by-step agent processes. Each process decides the next step for you; follow it instead of guessing.".to_string(),
        "Processes:".to_string(),
    ];
    for f in visible {
        lines.push(format!("- {} ({}): {}", f.name, f.scope.label(), f.description));
    }
    lines.push("How to use a process:".to_string());
    lines.push(
        "When a task matches a process, start it with the rein MCP tools (rein_start, then rein_answer until kind=done). Without them, run `rein step --flow <name> --json` and follow it until kind=done: answer each question with --answer <key>=<value> (or send the returned `answers` back via --stdin, with \"flow\" set and your new answer added); for kind=request, do the instruction first, then answer.".to_string(),
    );
    lines.join("\n")
}

/// Programs directly under `folder`, sorted by name.
fn flows_in(folder: &Path, scope: Scope) -> Vec<Flow> {
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    let mut flows: Vec<Flow> = entries
        .filter_map(Result::ok)
        // `npm` is the package project of its scope, so it is never a program.
        .filter(|e| e.file_name().to_str() != Some(NPM_DIR))
        .filter_map(|e| program(e.path(), scope))
        .collect();
    flows.sort_by(|a, b| a.name.cmp(&b.name));
    flows
}

/// The Rein-only manifest fields. `name` and `description` are the fallback when there is no `package.json`.
#[derive(Default, Deserialize)]
struct Manifest {
    name: Option<String>,
    description: Option<String>,
    entry: Option<String>,
}

#[derive(Default, Deserialize)]
struct PackageJson {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    engines: Option<Engines>,
}

#[derive(Deserialize)]
struct Engines {
    rein: Option<String>,
}

/// Packages installed in `npm_dir/node_modules`. pnpm links each direct dependency at its top level, so each
/// top-level entry is one program, named by its link name. An npm alias such as `pr-v1` is a program of its own.
/// A scope folder such as `@scope` holds `@scope/name` programs. Dot entries (`.pnpm`) are skipped.
fn npm_programs(npm_dir: &Path, scope: Scope) -> Vec<Flow> {
    let modules = npm_dir.join("node_modules");
    let Ok(entries) = fs::read_dir(&modules) else { return Vec::new() };
    let mut names = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if file_name.starts_with('.') {
            continue;
        }
        if !file_name.starts_with('@') {
            names.push(file_name);
            continue;
        }
        let Ok(scoped) = fs::read_dir(entry.path()) else { continue };
        for pkg in scoped.filter_map(Result::ok) {
            let pkg_name = pkg.file_name().to_string_lossy().into_owned();
            if !pkg_name.starts_with('.') {
                names.push(format!("{file_name}/{pkg_name}"));
            }
        }
    }
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            // A package without a Rein entry is not a program. `program` returns None for it.
            let mut flow = program(modules.join(&name), scope)?;
            flow.name = name;
            Some(flow)
        })
        .collect()
}

/// Whether `dir` is a program root: a folder, not a hidden one, that holds a `rein.json` or a `main.rein`.
pub fn is_program_dir(dir: &Path) -> bool {
    let hidden = dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
    !hidden && dir.is_dir() && (dir.join(MANIFEST).is_file() || dir.join(MAIN_FILE).is_file())
}

/// The program in `root`: a folder with a manifest or a `main.rein`. `None` for any other folder,
/// and for a hidden one. `package.json` wins over `rein.json` for `name` and `description`.
pub fn program(root: PathBuf, scope: Scope) -> Option<Flow> {
    let folder = root.file_name()?.to_str()?.to_string();
    if !is_program_dir(&root) {
        return None;
    }
    // ponytail: a manifest that does not parse falls back to the defaults; `rein check` could report it.
    let manifest: Manifest = read_json(&root.join(MANIFEST)).unwrap_or_default();
    let package: PackageJson = read_json(&root.join(PACKAGE)).unwrap_or_default();
    let entry_file = manifest.entry.unwrap_or_else(|| MAIN_FILE.to_string());
    let entry = entry_name(&entry_file);
    let description = package
        .description
        .or(manifest.description)
        .unwrap_or_else(|| description_of(&root.join(&entry_file)));
    let name = package.name.or(manifest.name).unwrap_or(folder);
    let engines_rein = package.engines.and_then(|e| e.rein);
    Some(Flow { name, scope, root, description, entry, shadowed_by: None, version: package.version, engines_rein })
}

/// The function name of an entry file, such as `main` for `main.rein`. `rein.json` names the file.
pub(crate) fn entry_name(file: &str) -> String {
    file.strip_suffix(".rein").unwrap_or(file).to_string()
}

/// The JSON file at `path`, or None when it is missing or not valid.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// First paragraph of the doc comment of the file, with its whitespace collapsed to single spaces.
fn description_of(main: &Path) -> String {
    let Ok(text) = fs::read_to_string(main) else { return String::new() };
    let (function, _) = parse(&text);
    let raw = function.description.map(|d| d.value).unwrap_or_default();
    let first = raw.split("\n\n").next().unwrap_or("");
    first.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Marks each flow whose name an earlier entry already has. The first entry of a name stays visible.
fn mark_shadowed(flows: Vec<Flow>) -> Vec<Flow> {
    let mut winners: Vec<(String, Scope)> = Vec::new();
    flows
        .into_iter()
        .map(|mut f| {
            if let Some((_, by)) = winners.iter().find(|(n, _)| *n == f.name) {
                f.shadowed_by = Some(*by);
            } else {
                winners.push((f.name.clone(), f.scope));
            }
            f
        })
        .collect()
}

#[cfg(test)]
#[path = "flows_tests.rs"]
mod tests;
