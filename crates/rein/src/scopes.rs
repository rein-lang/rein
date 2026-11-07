//! Where the flow scopes live on this machine. The environment is read here, not in `rein-core`.
//!
//! `REIN_HOME` replaces the user scope folder (`~/.rein`). `REIN_SYSTEM_DIRS` replaces the system
//! folders, as a colon-separated list. Both exist for tests and daemons.

use std::env;
use std::path::{Path, PathBuf};

use rein_core::flows::{self, Flow};

/// The user scope folder: `REIN_HOME`, else `$HOME/.rein`.
pub fn user_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("REIN_HOME") {
        return Some(PathBuf::from(dir));
    }
    env::var_os("HOME").map(|h| PathBuf::from(h).join(flows::FLOW_DIR))
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
    flows::discover(&cwd, user_dir().as_deref(), &system_dirs())
}

/// The root folder of the visible flow called `name`.
pub fn flow_root(name: &str) -> Result<PathBuf, String> {
    let all = discover_here();
    match flows::resolve(&all, name) {
        Some(flow) => Ok(flow.root.clone()),
        None => Err(format!("unknown flow `{name}`; run `rein flows` to list flows")),
    }
}
