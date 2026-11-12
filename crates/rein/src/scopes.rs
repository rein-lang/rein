//! Where the flow scopes live on this machine. The environment is read here, not in `rein-core`.
//!
//! `REIN_HOME` replaces the user programs folder (`~/.config/rein/programs`). `REIN_SYSTEM_DIRS` replaces the system
//! folders, as a colon-separated list. Both exist for tests and daemons.

use std::env;
use std::path::{Path, PathBuf};

use rein_core::flows::{self, Flow, FLOW_DIR};
use rein_core::project::Project;

/// The user programs folder: `REIN_HOME`, else `$XDG_CONFIG_HOME/rein/programs`, else
/// `~/.config/rein/programs`.
pub fn user_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("REIN_HOME") {
        return Some(PathBuf::from(dir));
    }
    let config = env::var_os("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("rein").join("programs"))
}

/// The `rein/` folder that repo-bound installs use: the nearest one from `cwd` up, as project scope finds it. Without
/// one, `rein/` at the git repository root (the nearest folder with `.git`). The folder is not created here.
pub fn project_rein_dir(cwd: &Path) -> Result<PathBuf, String> {
    if let Some(dir) = cwd.ancestors().map(|d| d.join(FLOW_DIR)).find(|d| d.is_dir()) {
        return Ok(dir);
    }
    cwd.ancestors()
        .find(|d| d.join(".git").exists())
        .map(|root| root.join(FLOW_DIR))
        .ok_or_else(|| "not in a repository; use -g to install for the user".to_string())
}

/// The system scope folders, in precedence order: `REIN_SYSTEM_DIRS`, else the platform defaults.
pub fn system_dirs() -> Vec<PathBuf> {
    if let Some(list) = env::var_os("REIN_SYSTEM_DIRS") {
        return env::split_paths(&list).collect();
    }
    let mut dirs = vec![PathBuf::from("/etc/rein")];
    if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/Library/Application Support/Rein"));
    }
    dirs
}

/// Every flow visible from the current directory.
pub fn discover_here() -> Vec<Flow> {
    let cwd = std::path::absolute(Path::new(".")).unwrap_or_else(|_| PathBuf::from("."));
    discover_from(&cwd)
}

/// Every flow visible from `cwd`. The language server passes the project root, not its own working directory.
pub fn discover_from(cwd: &Path) -> Vec<Flow> {
    flows::discover(cwd, user_dir().as_deref(), &system_dirs())
}

/// The visible program called `name`.
pub fn find(name: &str) -> Result<Flow, String> {
    find_in(&discover_here(), name)
}

/// The program called `name` among `all`, as [`find`] picks it.
pub fn find_in(all: &[Flow], name: &str) -> Result<Flow, String> {
    match flows::resolve(all, name) {
        Some(flow) => Ok(flow.clone()),
        None => Err(format!("unknown program `{name}`; run `rein list` to list programs")),
    }
}

/// Loads the project at `root`, and the programs its `@program` references name, found as `rein list` finds them.
pub fn load_linked(root: &Path) -> std::io::Result<Project> {
    let mut project = Project::load(root)?;
    project.link_programs(&|name| find(name).map(|f| (f.root, f.entry)));
    Ok(project)
}

/// The project root and entry function of a program named on the command line. A `.rein` path, or any
/// path to an existing file, is the program itself: its folder is the root and its name is the entry, with
/// no manifest or scope. Anything else is a visible program name (see `rein list`).
pub fn locate(program: &str) -> Result<(PathBuf, String), String> {
    let path = Path::new(program);
    if !program.ends_with(".rein") && !path.is_file() {
        let flow = find(program)?;
        return Ok((flow.root, flow.entry));
    }
    let file = std::path::absolute(path).map_err(|e| format!("cannot read {program}: {e}"))?;
    if !file.is_file() {
        return Err(format!("no such file: {program}"));
    }
    if file.extension().and_then(|e| e.to_str()) != Some("rein") {
        return Err(format!("not a .rein file: {program}"));
    }
    let root = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let entry = file.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    Ok((root, entry))
}
