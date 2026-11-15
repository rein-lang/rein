//! The programs `rein watch` shows: the folder a run's program is, its functions, and the walk of a run drawn as
//! Mermaid, the same drawing `rein debug` shows.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use rein_core::graph::{self, Graph, NodeKind};
use rein_core::project::{Project, ENTRY};
use rein_core::step::StepResponse;
use serde_json::{Map, Value};

use crate::cli;

/// A program that loads: its project and the entry it starts from.
pub struct Loaded {
    pub project: Project,
    pub entry: String,
}

/// The programs of one poll, by folder or flow name. None when the program does not load. A program is loaded once
/// per poll, however many runs use it.
pub type Programs = HashMap<String, Option<Rc<Loaded>>>;

/// Whether a program value from the log is a path, not a name: absolute, dotted, with a `/`, or a `.rein` file.
pub fn is_path(program: &str) -> bool {
    Path::new(program).is_absolute() || program.contains('/') || program.starts_with('.') || program.ends_with(".rein")
}

/// The project folder a program path names. A path is resolved against the agent's cwd. For a `.rein` file, the
/// project is its folder. The folder must hold a main.rein, or nothing is loaded. None for a name.
pub fn resolve_folder(program: &str, cwd: &str) -> Option<PathBuf> {
    if !is_path(program) {
        return None;
    }
    let absolute = Path::new(program).is_absolute();
    if !absolute && cwd.is_empty() {
        return None;
    }
    let joined = if absolute { PathBuf::from(program) } else { Path::new(cwd).join(program) };
    let folder = if program.ends_with(".rein") { joined.parent()?.to_path_buf() } else { joined };
    if !folder.join("main.rein").is_file() {
        return None;
    }
    let mut out = PathBuf::new();
    for part in folder.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    Some(out)
}

/// The loaded program of a run: its folder, or its flow name. None when the run names no program or it does not
/// load.
pub fn program(programs: &mut Programs, folder: Option<&Path>, name: &str) -> Option<Rc<Loaded>> {
    let key = folder.map_or_else(|| name.to_string(), |f| f.display().to_string());
    // A path that resolves to no project is never loaded as a flow name.
    if key.is_empty() || (folder.is_none() && is_path(name)) {
        return None;
    }
    programs
        .entry(key)
        .or_insert_with(|| {
            let loaded = match folder {
                Some(dir) => cli::load_flow(Some(dir.to_path_buf()), None),
                None => cli::load_flow(None, Some(name.to_string())),
            };
            loaded.ok().map(|(project, entry)| Rc::new(Loaded { project, entry }))
        })
        .clone()
}

/// Whether the loaded program still has what a run used: each function on its trace, and an input or response for
/// each answer the run logged. A program changed since the run fails this, and its graph would show the wrong place.
pub fn matches_run(loaded: &Loaded, trace: &[String], answers: &Map<String, Value>) -> bool {
    let project = &loaded.project;
    trace.iter().all(|f| project.get(f).is_some())
        && answers.keys().all(|key| {
            let Some((function, name)) = rein_core::step::key_parts(key) else { return false };
            project
                .get(function)
                .is_some_and(|sf| sf.function.input(name).or_else(|| sf.function.response(name)).is_some())
        })
}

/// The functions of a program, sorted, without templates.
pub fn function_names(loaded: &Loaded) -> Vec<String> {
    let mut names: Vec<String> = loaded
        .project
        .functions()
        .filter(|(_, sf)| !sf.function.is_template())
        .map(|(n, _)| n.clone())
        .collect();
    names.sort();
    names
}

/// The graph of a run as Mermaid, with the walk marked: the nodes taken and the current one. The walk is the run's
/// last good step. Returns the path (the functions on `trace`, see `keep_path`) and the whole program. `entry` is the
/// one the run started from; the default entry is the program's.
pub fn walk_mermaid(loaded: &Loaded, step: &Value, entry: &str, trace: &[String]) -> (String, String) {
    let entry = if entry.is_empty() || entry == ENTRY { loaded.entry.as_str() } else { entry };
    let response = serde_json::from_value::<StepResponse>(step.clone()).ok();
    let mut g = match &response {
        Some(r) => graph::walk(&loaded.project, entry, r),
        None => graph::flow(&loaded.project, entry),
    };
    let full = graph::mermaid(&g);
    // With no trace yet, the path is the entry function alone.
    let mut on_path = if trace.is_empty() { vec![entry.to_string()] } else { trace.to_vec() };
    if let Some(current) = &g.current {
        if let Some(n) = g.nodes.iter().find(|n| n.id == current.node) {
            if !on_path.contains(&n.function) {
                on_path.push(n.function.clone());
            }
        }
    }
    keep_path(&mut g, &on_path);
    (graph::mermaid(&g), full)
}

/// Keeps the nodes of the functions on the path, and drops the edges that lose an end. Each function the path leaves
/// to keeps one node, its stub: it becomes a plain node labelled with the function name, so the reader sees the exit.
fn keep_path(g: &mut Graph, on_path: &[String]) {
    let kept: HashSet<String> =
        g.nodes.iter().filter(|n| on_path.contains(&n.function)).map(|n| n.id.clone()).collect();
    let mut stub_functions: HashSet<String> = HashSet::new();
    let mut stubs: HashSet<String> = HashSet::new();
    for e in g.edges.iter().filter(|e| kept.contains(&e.from) && !kept.contains(&e.to)) {
        if let Some(node) = g.nodes.iter().find(|n| n.id == e.to) {
            if stub_functions.insert(node.function.clone()) {
                stubs.insert(e.to.clone());
            }
        }
    }
    for node in g.nodes.iter_mut().filter(|n| stubs.contains(&n.id)) {
        node.kind = NodeKind::Function;
        node.label = node.function.clone();
        node.entry = false;
        node.name = None;
        node.ty = None;
    }
    let keep = |id: &String| kept.contains(id) || stubs.contains(id);
    g.nodes.retain(|n| keep(&n.id));
    g.edges.retain(|e| keep(&e.from) && keep(&e.to));
}
