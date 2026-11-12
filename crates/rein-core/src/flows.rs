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
        "Rein flows are step-by-step agent flows. Each flow decides the next step for you; follow it instead of guessing.".to_string(),
        "Flows:".to_string(),
    ];
    for f in visible {
        lines.push(format!("- {} ({}): {}", f.name, f.scope.label(), f.description));
    }
    lines.push("How to use a flow:".to_string());
    lines.push(
        "When a task matches a flow, start it with the rein MCP tools (rein_start, then rein_answer until kind=done). Without them, run `rein step --flow <name> --json` and follow it until kind=done: answer each question with --answer <key>=<value> (or send the returned `answers` back via --stdin, with \"flow\" set and your new answer added); for kind=request, do the instruction first, then answer.".to_string(),
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

/// The program in `root`: a folder with a manifest or a `main.rein`. `None` for any other folder,
/// and for a hidden one. `package.json` wins over `rein.json` for `name` and `description`.
pub fn program(root: PathBuf, scope: Scope) -> Option<Flow> {
    let folder = root.file_name()?.to_str()?.to_string();
    let has_manifest = root.join(MANIFEST).is_file();
    if folder.starts_with('.') || !root.is_dir() || !(has_manifest || root.join(MAIN_FILE).is_file()) {
        return None;
    }
    // ponytail: a manifest that does not parse falls back to the defaults; `rein check` could report it.
    let manifest: Manifest = read_json(&root.join(MANIFEST)).unwrap_or_default();
    let package: PackageJson = read_json(&root.join(PACKAGE)).unwrap_or_default();
    let entry_file = manifest.entry.unwrap_or_else(|| MAIN_FILE.to_string());
    let entry = entry_file.strip_suffix(".rein").unwrap_or(&entry_file).to_string();
    let description = package
        .description
        .or(manifest.description)
        .unwrap_or_else(|| description_of(&root.join(&entry_file)));
    let name = package.name.or(manifest.name).unwrap_or(folder);
    let engines_rein = package.engines.and_then(|e| e.rein);
    Some(Flow { name, scope, root, description, entry, shadowed_by: None, version: package.version, engines_rein })
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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A unique empty temp directory.
    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rein-flows-{tag}-{}-{nanos}-{n}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes `rel` (relative to `base`) with `text`, creating folders.
    fn write(base: &Path, rel: &str, text: &str) {
        let p = base.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    /// A flow whose entry has the given description.
    fn flow_text(desc: &str) -> String {
        format!("/** {desc} */\nexport default function main() {{\n  return \"ok\";\n}}\n")
    }

    fn names(flows: &[Flow]) -> Vec<(String, Scope, Option<Scope>)> {
        flows.iter().map(|f| (f.name.clone(), f.scope, f.shadowed_by)).collect()
    }

    #[test]
    fn project_beats_user_and_user_beats_system() {
        let home = temp_dir("home");
        let sys = temp_dir("sys");
        let repo = temp_dir("repo");
        write(&repo, "rein/deploy/main.rein", &flow_text("project deploy"));
        write(&home, "rein/deploy/main.rein", &flow_text("user deploy"));
        write(&sys, "deploy/main.rein", &flow_text("system deploy"));

        let flows = discover(&repo, Some(&home.join(FLOW_DIR)), std::slice::from_ref(&sys));
        let deploy: Vec<_> = flows.iter().filter(|f| f.name == "deploy").collect();
        assert_eq!(deploy.len(), 3, "{flows:?}");
        assert_eq!(resolve(&flows, "deploy").unwrap().scope, Scope::Project);
        assert_eq!(resolve(&flows, "deploy").unwrap().description, "project deploy");
        assert_eq!(deploy[1].shadowed_by, Some(Scope::Project));
        assert_eq!(deploy[2].shadowed_by, Some(Scope::Project));
    }

    #[test]
    fn user_beats_system_when_no_project_flow() {
        let home = temp_dir("home2");
        let sys = temp_dir("sys2");
        let repo = temp_dir("repo2");
        write(&home, "rein/gate/main.rein", &flow_text("user gate"));
        write(&sys, "gate/main.rein", &flow_text("system gate"));
        let flows = discover(&repo, Some(&home.join(FLOW_DIR)), std::slice::from_ref(&sys));
        assert_eq!(resolve(&flows, "gate").unwrap().scope, Scope::User);
        assert_eq!(names(&flows).iter().find(|n| n.1 == Scope::System).unwrap().2, Some(Scope::User));
    }

    #[test]
    fn nearest_rein_wins_for_nested_repos() {
        let repo = temp_dir("nested");
        write(&repo, "rein/review/main.rein", &flow_text("outer review"));
        write(&repo, "service/rein/review/main.rein", &flow_text("inner review"));
        let cwd = repo.join("service");
        let flows = discover(&cwd, None, &[]);
        let winner = resolve(&flows, "review").unwrap();
        assert_eq!(winner.description, "inner review");
        assert_eq!(winner.root, cwd.join("rein/review"));
        let outer = flows.iter().find(|f| f.description == "outer review").unwrap();
        assert_eq!(outer.shadowed_by, Some(Scope::Project));
    }

    #[test]
    fn user_scope_under_home_is_not_also_a_project_scope() {
        let home = temp_dir("home3");
        write(&home, "rein/mine/main.rein", &flow_text("mine"));
        let cwd = home.join("code/app");
        fs::create_dir_all(&cwd).unwrap();
        let flows = discover(&cwd, Some(&home.join(FLOW_DIR)), &[]);
        assert_eq!(names(&flows), vec![("mine".to_string(), Scope::User, None)]);
    }

    #[test]
    fn folder_without_main_is_not_a_flow() {
        let repo = temp_dir("nomain");
        write(&repo, "rein/broken/other.rein", &flow_text("no entry"));
        write(&repo, "rein/ok/main.rein", &flow_text("has entry"));
        let flows = discover(&repo, None, &[]);
        assert_eq!(names(&flows), vec![("ok".to_string(), Scope::Project, None)]);
    }

    #[test]
    fn description_is_the_first_paragraph_of_the_doc_comment() {
        let repo = temp_dir("desc");
        write(
            &repo,
            "rein/merge-pr/main.rein",
            "/**\n * Decide whether a pull request\n * can be merged.\n *\n * @x later text\n */\nexport default function main() {\n  return \"ok\";\n}\n",
        );
        let flows = discover(&repo, None, &[]);
        assert_eq!(flows[0].description, "Decide whether a pull request can be merged.");
    }

    #[test]
    fn agent_context_lists_visible_flows_only() {
        let home = temp_dir("home4");
        let repo = temp_dir("repo4");
        write(&repo, "rein/merge-pr/main.rein", &flow_text("Decide a merge."));
        write(&home, "rein/merge-pr/main.rein", &flow_text("Shadowed copy."));
        write(&home, "rein/triage/main.rein", &flow_text("Triage an incident."));
        let flows = discover(&repo, Some(&home.join(FLOW_DIR)), &[]);
        let text = agent_context(&flows);
        assert!(text.contains("- merge-pr (project): Decide a merge."), "{text}");
        assert!(text.contains("- triage (user): Triage an incident."), "{text}");
        assert!(!text.contains("Shadowed copy"), "{text}");
        assert!(text.contains("rein step --flow <name> --json"), "{text}");
        assert!(text.lines().count() <= 25, "{text}");
    }

    #[test]
    fn agent_context_is_empty_without_flows() {
        let repo = temp_dir("empty");
        assert_eq!(agent_context(&discover(&repo, None, &[])), "");
    }

    #[test]
    fn visible_rein_folder_with_manifest_is_a_program() {
        let repo = temp_dir("manifest");
        write(&repo, "rein/app/rein.json", r#"{"name": "renamed", "description": "From the manifest.", "entry": "start.rein"}"#);
        write(&repo, "rein/app/start.rein", &flow_text("From the doc comment."));
        write(&repo, "rein/not-a-program/notes.txt", "no manifest, no main.rein");
        let flows = discover(&repo, None, &[]);
        assert_eq!(flows.len(), 1, "{flows:?}");
        let app = resolve(&flows, "renamed").unwrap();
        assert_eq!(app.scope, Scope::Project);
        assert_eq!(app.description, "From the manifest.");
        assert_eq!(app.entry, "start");
    }

    #[test]
    fn package_json_wins_over_rein_json_and_rein_json_is_the_fallback() {
        let repo = temp_dir("precedence");
        write(&repo, "rein/app/package.json", r#"{"name": "pkg-name", "version": "1.2.3", "description": "From package.json.", "engines": {"rein": "^0.1.0"}}"#);
        write(&repo, "rein/app/rein.json", r#"{"name": "rein-name", "description": "From rein.json.", "entry": "start.rein"}"#);
        write(&repo, "rein/app/start.rein", &flow_text("From the doc comment."));
        write(&repo, "rein/plain/package.json", r#"{"name": "plain-pkg"}"#);
        write(&repo, "rein/plain/rein.json", r#"{"description": "Fallback description."}"#);
        write(&repo, "rein/plain/main.rein", &flow_text("From the doc comment."));
        let flows = discover(&repo, None, &[]);
        let app = resolve(&flows, "pkg-name").unwrap();
        assert_eq!(app.description, "From package.json.");
        assert_eq!(app.version.as_deref(), Some("1.2.3"));
        assert_eq!(app.engines_rein.as_deref(), Some("^0.1.0"));
        assert_eq!(app.entry, "start");
        assert!(resolve(&flows, "rein-name").is_none());
        let plain = resolve(&flows, "plain-pkg").unwrap();
        assert_eq!(plain.description, "Fallback description.");
        assert_eq!(plain.version, None);
        assert_eq!(plain.engines_rein, None);
    }
}
