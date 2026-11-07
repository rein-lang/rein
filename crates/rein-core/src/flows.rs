//! Flow discovery: scoped `.rein/` folders, each holding one flow per subfolder.
//!
//! A flow is a directory that holds `main.rein`. Flows live in three scopes, like `.claude`:
//! project (`.rein/` in the working directory or any ancestor), user (`<home>/.rein/`), and system
//! (each directory in the system list, such as `/etc/rein/`). On a name clash the closest scope wins:
//! project, then user, then system. Inside the project scope, the nearest `.rein/` wins.
//!
//! Discovery takes its inputs as arguments and reads no environment variables, so tests can pass
//! temp directories.

use std::fs;
use std::path::{Path, PathBuf};

use crate::parser::parse;

/// The file that marks a flow root and holds its entry function.
pub const MAIN_FILE: &str = "main.rein";

/// The folder name of the scoped flow folder.
pub const FLOW_DIR: &str = ".rein";

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
    /// The folder name, such as `merge-pr`.
    pub name: String,
    pub scope: Scope,
    /// The flow root: the folder that holds `main.rein`.
    pub root: PathBuf,
    /// The doc comment of `main.rein`, flattened to its first paragraph. Empty when there is none.
    pub description: String,
    /// The scope of the flow that wins this name, when this one is shadowed.
    pub shadowed_by: Option<Scope>,
}

/// Every flow visible from `cwd`, in precedence order, with shadowed flows marked.
///
/// `cwd` must be absolute. `user_dir` is the user scope folder, normally `~/.rein`. It is not also a
/// project scope when it is an ancestor of `cwd`. `system_dirs` are checked in order; the first one
/// wins a clash. A folder under a scoped `.rein/` that has no `main.rein` is not a flow and is skipped.
pub fn discover(cwd: &Path, user_dir: Option<&Path>, system_dirs: &[PathBuf]) -> Vec<Flow> {
    let mut found = Vec::new();
    for dir in cwd.ancestors() {
        let scoped = dir.join(FLOW_DIR);
        if user_dir == Some(scoped.as_path()) {
            continue;
        }
        if scoped.is_dir() {
            found.extend(flows_in(&scoped, Scope::Project));
        }
    }
    if let Some(user) = user_dir {
        found.extend(flows_in(user, Scope::User));
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
        "When a task matches a flow, run `rein step --flow <name> --json` and follow it until kind=done: answer each question with --answer <key>=<value> (or send the returned `answers` back via --stdin, with \"flow\" set and your new answer added); for kind=request, do the instruction first, then answer.".to_string(),
    );
    lines.join("\n")
}

/// Flows directly under `folder`: each subfolder that holds `main.rein`. Sorted by name.
fn flows_in(folder: &Path, scope: Scope) -> Vec<Flow> {
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    let mut flows: Vec<Flow> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join(MAIN_FILE).is_file())
        .filter_map(|root| {
            let name = root.file_name()?.to_str()?.to_string();
            if name.starts_with('.') {
                return None;
            }
            let description = description_of(&root.join(MAIN_FILE));
            Some(Flow { name, scope, root, description, shadowed_by: None })
        })
        .collect();
    flows.sort_by(|a, b| a.name.cmp(&b.name));
    flows
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
        write(&repo, ".rein/deploy/main.rein", &flow_text("project deploy"));
        write(&home, ".rein/deploy/main.rein", &flow_text("user deploy"));
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
        write(&home, ".rein/gate/main.rein", &flow_text("user gate"));
        write(&sys, "gate/main.rein", &flow_text("system gate"));
        let flows = discover(&repo, Some(&home.join(FLOW_DIR)), std::slice::from_ref(&sys));
        assert_eq!(resolve(&flows, "gate").unwrap().scope, Scope::User);
        assert_eq!(names(&flows).iter().find(|n| n.1 == Scope::System).unwrap().2, Some(Scope::User));
    }

    #[test]
    fn nearest_rein_wins_for_nested_repos() {
        let repo = temp_dir("nested");
        write(&repo, ".rein/review/main.rein", &flow_text("outer review"));
        write(&repo, "service/.rein/review/main.rein", &flow_text("inner review"));
        let cwd = repo.join("service");
        let flows = discover(&cwd, None, &[]);
        let winner = resolve(&flows, "review").unwrap();
        assert_eq!(winner.description, "inner review");
        assert_eq!(winner.root, cwd.join(".rein/review"));
        let outer = flows.iter().find(|f| f.description == "outer review").unwrap();
        assert_eq!(outer.shadowed_by, Some(Scope::Project));
    }

    #[test]
    fn user_scope_under_home_is_not_also_a_project_scope() {
        let home = temp_dir("home3");
        write(&home, ".rein/mine/main.rein", &flow_text("mine"));
        let cwd = home.join("code/app");
        fs::create_dir_all(&cwd).unwrap();
        let flows = discover(&cwd, Some(&home.join(FLOW_DIR)), &[]);
        assert_eq!(names(&flows), vec![("mine".to_string(), Scope::User, None)]);
    }

    #[test]
    fn folder_without_main_is_not_a_flow() {
        let repo = temp_dir("nomain");
        write(&repo, ".rein/broken/other.rein", &flow_text("no entry"));
        write(&repo, ".rein/ok/main.rein", &flow_text("has entry"));
        let flows = discover(&repo, None, &[]);
        assert_eq!(names(&flows), vec![("ok".to_string(), Scope::Project, None)]);
    }

    #[test]
    fn description_is_the_first_paragraph_of_the_doc_comment() {
        let repo = temp_dir("desc");
        write(
            &repo,
            ".rein/merge-pr/main.rein",
            "/**\n * Decide whether a pull request\n * can be merged.\n *\n * @x later text\n */\nexport default function main() {\n  return \"ok\";\n}\n",
        );
        let flows = discover(&repo, None, &[]);
        assert_eq!(flows[0].description, "Decide whether a pull request can be merged.");
    }

    #[test]
    fn agent_context_lists_visible_flows_only() {
        let home = temp_dir("home4");
        let repo = temp_dir("repo4");
        write(&repo, ".rein/merge-pr/main.rein", &flow_text("Decide a merge."));
        write(&home, ".rein/merge-pr/main.rein", &flow_text("Shadowed copy."));
        write(&home, ".rein/triage/main.rein", &flow_text("Triage an incident."));
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
}
