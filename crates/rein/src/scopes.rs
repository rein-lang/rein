//! Where the flow scopes live on this machine. The environment is read here, not in `rein-core`.
//!
//! `REIN_HOME` replaces the user programs folder (`~/.config/rein/programs`). `REIN_SYSTEM_DIRS` replaces the system
//! folders, as a colon-separated list. Both exist for tests and daemons.

use std::env;
use std::path::{Path, PathBuf};

use rein_core::flows::{self, Flow};

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

/// The visible program called `name`.
pub fn find(name: &str) -> Result<Flow, String> {
    let all = discover_here();
    match flows::resolve(&all, name) {
        Some(flow) => Ok(flow.clone()),
        None => Err(format!("unknown program `{name}`; run `rein list` to list programs")),
    }
}
