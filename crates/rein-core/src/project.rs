//! A project: every `*.rein` file under a root directory, keyed by function name.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::ast::{walk_stmts, Function, ReturnValue, Stmt};
use crate::check::check_function;
use crate::diagnostic::{Diagnostic, Span};
use crate::parser::parse;

/// The function every project starts from. It is `main.rein` at the project root.
pub const ENTRY: &str = "main";

pub struct SourceFile {
    pub path: PathBuf,
    pub text: String,
    pub function: Function,
    pub parse_diagnostics: Vec<Diagnostic>,
}

pub struct Project {
    pub root: PathBuf,
    /// Key = function name.
    pub files: BTreeMap<String, SourceFile>,
}

impl Project {
    /// Nearest ancestor of `start` (or its parent, if `start` is a file) that
    /// has a `rein.toml`. Falls back to the start directory.
    pub fn find_root(start: &Path) -> PathBuf {
        let start = fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
        let base = if start.is_file() {
            start.parent().unwrap_or(start.as_path()).to_path_buf()
        } else {
            start
        };
        base.ancestors()
            .find(|d| d.join("rein.toml").is_file())
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
        Self { root, files }
    }

    /// Function name for a path under the root. Invalid names are returned as-is.
    pub fn name_for(&self, path: &Path) -> Option<String> {
        name_under(&self.root, path)
    }

    pub fn get(&self, name: &str) -> Option<&SourceFile> {
        self.files.get(name)
    }

    /// Diagnostics that belong to the project, not to one function. They point into `rein.toml`.
    pub fn project_diagnostics(&self) -> Vec<Diagnostic> {
        if self.files.contains_key(ENTRY) {
            return Vec::new();
        }
        vec![Diagnostic::error(Span::default(), "project has no main.rein; it is the agent's entry point")]
    }

    /// The project marker file, `rein.toml` in the root.
    pub fn marker_path(&self) -> PathBuf {
        self.root.join("rein.toml")
    }

    /// Resolves an import path written in function `from` to the project path of a function.
    /// The path starts with `./` or `../`, has no extension, and is relative to the importing file's directory.
    pub fn resolve(&self, from: &str, source: &str) -> Result<String, String> {
        if !(source.starts_with("./") || source.starts_with("../")) {
            return Err("import path must start with `./` or `../`".into());
        }
        if source.rsplit('/').next().is_some_and(|last| last.contains('.') && last != "..") {
            return Err("import path must have no extension".into());
        }
        // The importing file's directory, as name segments. `ci/fix` starts in `ci`.
        let mut parts: Vec<&str> = from.split('/').collect();
        parts.pop();
        for seg in source.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err(format!("import '{source}' leaves the project root"));
                    }
                }
                s => parts.push(s),
            }
        }
        let name = parts.join("/");
        if self.files.contains_key(&name) {
            Ok(name)
        } else {
            Err(format!("cannot find module '{source}'"))
        }
    }

    /// Binding name to project path, for every import of `name` that resolves.
    pub fn import_targets(&self, name: &str) -> BTreeMap<String, String> {
        let Some(sf) = self.files.get(name) else { return BTreeMap::new() };
        sf.function
            .imports
            .iter()
            .filter_map(|imp| Some((imp.binding.value.clone(), self.resolve(name, &imp.source.value).ok()?)))
            .collect()
    }

    /// Diagnostics per function, sorted by span: parse, single-file checks, cycles, and reachability.
    /// A file with a syntax error reports only that error. Its tree is partial.
    pub fn check(&self) -> BTreeMap<String, Vec<Diagnostic>> {
        let adj = self.adjacency();
        let reachable = self.reachable_from_entry(&adj);
        self.files
            .iter()
            .map(|(name, sf)| {
                if !sf.parse_diagnostics.is_empty() {
                    return (name.clone(), sf.parse_diagnostics.clone());
                }
                let mut diags = Vec::new();
                let resolve = |src: &str| self.resolve(name, src);
                diags.extend(check_function(&sf.function, name, &resolve));
                let targets = self.import_targets(name);
                diags.extend(self.cycle_warnings(name, &sf.function, &targets, &adj));
                if reachable.as_ref().is_some_and(|r| !r.contains(name)) {
                    let span = sf.function.doc.as_ref().map_or_else(Span::default, |d| d.span);
                    diags.push(Diagnostic::warning(span, format!("`{name}` is not reachable from main")));
                }
                diags.sort_by_key(|d| (d.span.start, d.span.end));
                (name.clone(), diags)
            })
            .collect()
    }

    /// Distinct `(from, to)` pairs for every return of an imported function that resolves.
    pub fn references(&self) -> Vec<(String, String)> {
        self.files
            .iter()
            .flat_map(|(name, sf)| {
                let targets = self.import_targets(name);
                ref_sites(&sf.function, &targets).into_iter().map(move |(to, _)| (name.clone(), to))
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Functions reachable from `main` through references. None when `main` is missing or has a syntax error.
    fn reachable_from_entry(&self, adj: &BTreeMap<String, BTreeSet<String>>) -> Option<BTreeSet<String>> {
        let entry = self.files.get(ENTRY)?;
        if !entry.parse_diagnostics.is_empty() {
            return None;
        }
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([ENTRY.to_string()]);
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
                let targets = self.import_targets(name);
                let edges = ref_sites(&sf.function, &targets).into_iter().map(|(t, _)| t).collect();
                (name.clone(), edges)
            })
            .collect()
    }

    /// One warning per return in `f` whose target can reach back to `name`.
    fn cycle_warnings(
        &self,
        name: &str,
        f: &Function,
        targets: &BTreeMap<String, String>,
        adj: &BTreeMap<String, BTreeSet<String>>,
    ) -> Vec<Diagnostic> {
        ref_sites(f, targets)
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
            if file_name.starts_with('.') || file_name == "target" || file_name == "node_modules" {
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

/// Returns of imported functions in `f`, as (target project path, span of the return).
fn ref_sites(f: &Function, targets: &BTreeMap<String, String>) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    walk_stmts(&f.body, &mut |s| {
        if let Stmt::Return(r) = s {
            if let ReturnValue::Ref(binding) = &r.value {
                if let Some(target) = targets.get(binding) {
                    out.push((target.clone(), s.span()));
                }
            }
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
mod tests {
    use super::*;
    use crate::diagnostic::Severity;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn has(diags: &[Diagnostic], severity: Severity, part: &str) -> bool {
        diags.iter().any(|d| d.severity == severity && d.message.contains(part))
    }

    /// Writes a small project into a unique temp dir and returns its path.
    fn temp_project(tag: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rein-core-{tag}-{}-{nanos}", std::process::id()));
        let write = |rel: &str, text: &str| {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        };
        write("rein.toml", "");
        write(
            "main.rein",
            "import mergePr from \"./merge_pr\";\n/** Entry. */\nexport default function main() {\n  return mergePr;\n}\n",
        );
        write(
            "merge_pr.rein",
            "import fixCi from \"./ci/fix\";\nimport nope from \"./nope\";\n/** Merge a pull request. */\nexport default function mergePr(\n  /** CI status? */\n  ci: \"green\" | \"red\",\n) {\n\
             switch (ci) {\n    case \"green\": return \"merge\";\n    case \"red\": return fixCi;\n    default: return nope;\n}\n}\n",
        );
        write(
            "ci/fix.rein",
            "import mergePr from \"../merge_pr\";\n/** Fix red CI. */\nexport default function fixCi() {\n  return mergePr;\n}\n",
        );
        write("bad-name.rein", "/** x */\nexport default function bad() {\n  return \"y\";\n}\n");
        fs::create_dir_all(root.join("nested/deeper")).unwrap();
        root
    }

    #[test]
    fn loads_checks_and_reports_cycles_and_unknown_imports() {
        let root = temp_project("load");
        let project = Project::load(&root).unwrap();
        let names: Vec<&str> = project.files.keys().map(String::as_str).collect();
        assert_eq!(names, ["bad-name", "ci/fix", "main", "merge_pr"]);

        let diags = project.check();
        let merge = &diags["merge_pr"];
        assert!(has(merge, Severity::Error, "cannot find module './nope'"), "{merge:?}");
        assert!(has(merge, Severity::Warning, "reference cycle: merge_pr -> ci/fix -> merge_pr"), "{merge:?}");
        let fix = &diags["ci/fix"];
        assert!(has(fix, Severity::Warning, "reference cycle: ci/fix -> merge_pr -> ci/fix"), "{fix:?}");
        assert!(has(&diags["bad-name"], Severity::Error, "invalid function file name"));
        assert!(has(&diags["bad-name"], Severity::Warning, "`bad-name` is not reachable from main"));
        assert!(!has(fix, Severity::Warning, "not reachable"), "{fix:?}");
        assert!(project.project_diagnostics().is_empty());

        assert_eq!(project.references().len(), 3);
        fs::remove_dir_all(&root).unwrap();
    }

    /// Diagnostics for one in-memory file named `f`, in a project with a valid `main`.
    fn check_one(src: &str) -> Vec<Diagnostic> {
        let root = PathBuf::from("/proj");
        let project = Project::from_sources(
            root.clone(),
            vec![
                (root.join("main.rein"), "import f from \"./f\";\n/** m */\nexport default function main() { return f }\n".to_string()),
                (root.join("f.rein"), src.to_string()),
            ],
        );
        project.check().remove("f").unwrap()
    }

    #[test]
    fn syntax_error_reports_only_the_syntax_error() {
        let diags = check_one("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } return )");
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn lex_error_reports_only_the_syntax_error() {
        let diags = check_one("/** abc");
        assert_eq!(diags.len(), 1, "{diags:?}");
    }

    #[test]
    fn syntax_error_file_gets_no_cycle_warning() {
        let root = PathBuf::from("/proj");
        let project = Project::from_sources(
            root.clone(),
            vec![
                (root.join("main.rein"), "import f from \"./f\";\n/** m */\nexport default function main() { return f }\n".to_string()),
                (root.join("f.rein"), "import g from \"./g\";\n/** d */\nexport default function f() { return g }\nreturn )".to_string()),
                (root.join("g.rein"), "import f from \"./f\";\n/** d */\nexport default function g() { return f }\n".to_string()),
            ],
        );
        let diags = project.check();
        assert_eq!(diags["f"].len(), 1, "{:?}", diags["f"]);
    }

    #[test]
    fn unreadable_file_is_kept_with_read_error() {
        let root = temp_project("badutf8");
        fs::write(root.join("broken.rein"), [0xff, 0xfe, 0x00]).unwrap();
        let project = Project::load(&root).unwrap();
        let sf = project.get("broken").expect("unreadable file is still loaded");
        assert!(sf.text.is_empty());
        assert_eq!(sf.parse_diagnostics.len(), 1, "{:?}", sf.parse_diagnostics);
        assert!(sf.parse_diagnostics[0].message.starts_with("cannot read file: "));
        assert_eq!(sf.parse_diagnostics[0].span, Span::default());
        assert_eq!(project.check()["broken"].len(), 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn follows_symlinks_and_survives_symlink_loops() {
        let root = temp_project("links");
        fs::write(root.join("real.rein"), "/** d */\nexport default function real() { return \"x\" }\n").unwrap();
        std::os::unix::fs::symlink(root.join("real.rein"), root.join("alias.rein")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
        let project = Project::load(&root).unwrap();
        assert!(project.get("alias").is_some());
        assert!(project.get("real").is_some());
        assert!(project.files.keys().all(|k| !k.starts_with("loop/")));
        fs::remove_dir_all(&root).unwrap();
    }

    /// A project of `main` plus the given (name, source) files, all in `/proj`.
    fn project_of(files: &[(&str, &str)]) -> Project {
        let root = PathBuf::from("/proj");
        let mut sources = vec![(root.join("main.rein"), "/** m */\nexport default function main() { return \"m\" }\n".to_string())];
        sources.extend(files.iter().map(|(n, t)| (root.join(format!("{n}.rein")), t.to_string())));
        Project::from_sources(root, sources)
    }

    #[test]
    fn cycle_warns_on_every_return_on_the_cycle() {
        let project = project_of(&[
            ("main", "import a from \"./a\";\n/** m */\nexport default function main() { return a }\n"),
            ("a", "import b from \"./b\";\n/** d */\nexport default function a() { return b }\n"),
            ("b", "import c from \"./c\";\n/** d */\nexport default function b() { return c }\n"),
            ("c", "import a from \"./a\";\n/** d */\nexport default function c() { return a }\n"),
            ("self", "import me from \"./self\";\n/** d */\nexport default function me() { return me }\n"),
        ]);
        let diags = project.check();
        for name in ["a", "b", "c"] {
            assert!(has(&diags[name], Severity::Warning, "reference cycle"), "{name}: {:?}", diags[name]);
        }
        let cycle_count: usize = ["a", "b", "c"]
            .iter()
            .map(|n| diags[*n].iter().filter(|d| d.message.contains("reference cycle")).count())
            .sum();
        assert_eq!(cycle_count, 3);
        assert!(has(&diags["self"], Severity::Warning, "reference cycle: self -> self"), "{:?}", diags["self"]);
    }

    #[test]
    fn missing_main_is_a_project_error() {
        let root = PathBuf::from("/proj");
        let project = Project::from_sources(root.clone(), vec![(root.join("f.rein"), "/** d */\nexport default function f() { return \"x\" }".into())]);
        let errs = project.project_diagnostics();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].severity, Severity::Error);
        assert_eq!(errs[0].message, "project has no main.rein; it is the agent's entry point");
        assert_eq!(project.marker_path(), root.join("rein.toml"));
        // Without main, nothing is reported as unreachable.
        assert!(!has(&project.check()["f"], Severity::Warning, "not reachable"));
    }

    #[test]
    fn unreachable_function_is_a_warning() {
        let project = project_of(&[
            ("main", "import used from \"./used\";\n/** m */\nexport default function main() { return used }\n"),
            ("used", "import leaf from \"./deep/leaf\";\n/** u */\nexport default function used() { return leaf }\n"),
            ("deep/leaf", "/** l */\nexport default function leaf() { return \"ok\" }\n"),
            ("orphan", "/** o */\nexport default function orphan() { return \"alone\" }\n"),
        ]);
        let diags = project.check();
        assert!(has(&diags["orphan"], Severity::Warning, "`orphan` is not reachable from main"));
        assert!(!has(&diags["deep/leaf"], Severity::Warning, "not reachable"));
        assert!(!has(&diags["used"], Severity::Warning, "not reachable"));
        assert!(project.project_diagnostics().is_empty());
    }

    #[test]
    fn main_with_syntax_error_skips_reachability() {
        let root = PathBuf::from("/proj");
        let project = Project::from_sources(
            root.clone(),
            vec![
                (root.join("main.rein"), "/** m */\nexport default function main() { return )".to_string()),
                (root.join("orphan.rein"), "/** o */\nexport default function orphan() { return \"alone\" }\n".to_string()),
            ],
        );
        assert!(!has(&project.check()["orphan"], Severity::Warning, "not reachable"));
    }

    #[test]
    fn import_paths_resolve_relative_to_the_importing_file() {
        let project = project_of(&[
            ("ci/fix", ""),
            ("ci/deeper", ""),
            ("merge_pr", ""),
            ("ci/sub/leaf", ""),
            ("top", ""),
        ]);
        assert_eq!(project.resolve("ci/fix", "../merge_pr").unwrap(), "merge_pr");
        assert_eq!(project.resolve("ci/fix", "./deeper").unwrap(), "ci/deeper");
        assert_eq!(project.resolve("ci/sub/leaf", "../../top").unwrap(), "top");
        assert_eq!(project.resolve("ci/sub/leaf", "./../deeper").unwrap(), "ci/deeper");
        assert_eq!(project.resolve("main", "./top").unwrap(), "top");
    }

    #[test]
    fn import_path_errors() {
        let project = project_of(&[("ci/fix", ""), ("top", "")]);
        assert_eq!(project.resolve("ci/fix", "./missing").unwrap_err(), "cannot find module './missing'");
        assert_eq!(project.resolve("ci/fix", "top").unwrap_err(), "import path must start with `./` or `../`");
        assert_eq!(project.resolve("ci/fix", "./top.rein").unwrap_err(), "import path must have no extension");
        assert_eq!(project.resolve("ci/fix", "./top.js").unwrap_err(), "import path must have no extension");
        // From ci/fix the directory is `ci`; two steps up leaves the root.
        assert_eq!(project.resolve("ci/fix", "../../top").unwrap_err(), "import '../../top' leaves the project root");
        assert_eq!(project.resolve("top", "../top").unwrap_err(), "import '../top' leaves the project root");
    }

    #[test]
    fn import_outside_root_is_reported_by_the_checker() {
        let project = project_of(&[
            ("ci/fix", "import x from \"../../outside\";\n/** d */\nexport default function fix() { return x }\n"),
        ]);
        let diags = project.check();
        assert!(has(&diags["ci/fix"], Severity::Error, "leaves the project root"), "{:?}", diags["ci/fix"]);
    }

    #[test]
    fn unused_import_is_a_warning_from_the_checker() {
        let project = project_of(&[("a", "import m from \"./main\";\n/** d */\nexport default function a() { return \"x\" }\n")]);
        assert!(has(&project.check()["a"], Severity::Warning, "unused import `m`"));
    }

    #[test]
    fn find_root_walks_up_to_marker() {
        let root = fs::canonicalize(temp_project("root")).unwrap();
        assert_eq!(Project::find_root(&root.join("nested/deeper")), root);
        assert_eq!(Project::find_root(&root.join("ci/fix.rein")), root);
        fs::remove_dir_all(&root).unwrap();
    }
}
