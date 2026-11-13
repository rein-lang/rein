//! The programs `rein watch` shows: the folder a run's program is, its functions, and the walk of a run drawn as
//! Mermaid, the same drawing `rein debug` shows.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use rein_core::graph;
use rein_core::project::{Project, ENTRY};
use rein_core::step::StepResponse;
use serde_json::Value;

use crate::cli;

/// A program that loads: its project and the entry it starts from.
pub struct Loaded {
    pub project: Project,
    pub entry: String,
}

/// The programs of one poll, by folder or flow name. None when the program does not load. A program is loaded once
/// per poll, however many runs use it.
pub type Programs = HashMap<String, Option<Rc<Loaded>>>;

/// The folder a program path names: an absolute path, or a path relative to the agent's cwd (`.`, `./x`, `x/y` with a
/// main.rein in it). None for a flow name.
pub fn resolve_folder(program: &str, cwd: &str) -> Option<PathBuf> {
    let absolute = Path::new(program).is_absolute();
    if !absolute && cwd.is_empty() {
        return None;
    }
    let joined = if absolute { PathBuf::from(program) } else { Path::new(cwd).join(program) };
    let dotted = program == "." || program == ".." || program.starts_with("./") || program.starts_with("../");
    let folder = dotted || ((absolute || program.contains('/')) && joined.join("main.rein").is_file());
    if !folder {
        return None;
    }
    let mut out = PathBuf::new();
    for part in joined.components() {
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
    if key.is_empty() {
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

/// The graph of a program as Mermaid, with the walk of a run marked: the nodes taken and the current one. The walk
/// is the run's last good step. With no step, the plain graph. `entry` is the one the run started from; the default
/// entry is the program's.
pub fn walk_mermaid(loaded: &Loaded, step: &Value, entry: &str) -> String {
    let entry = if entry.is_empty() || entry == ENTRY { loaded.entry.as_str() } else { entry };
    let response = serde_json::from_value::<StepResponse>(step.clone()).ok();
    let g = match &response {
        Some(r) => graph::walk(&loaded.project, entry, r),
        None => graph::flow(&loaded.project, entry),
    };
    graph::mermaid(&g)
}
