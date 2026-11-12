//! `rein lock`: writes `rein.lock` from the programs a project hands off to now.

use std::fs;
use std::path::{Path, PathBuf};

use rein_core::lock::{self, LOCK_FILE};
use rein_core::project::Project;

use crate::scopes;

pub fn run(program: Option<String>, root: Option<PathBuf>) -> u8 {
    match write(program, root) {
        Ok(message) => {
            println!("{message}");
            0
        }
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

fn write(program: Option<String>, root: Option<PathBuf>) -> Result<String, String> {
    let root = match program {
        Some(p) => scopes::locate(&p)?.0,
        None => root.unwrap_or_else(|| Project::find_root(Path::new("."))),
    };
    let project = scopes::load_linked(&root).map_err(|e| format!("cannot read {}: {e}", root.display()))?;
    if let Some((name, reason)) = project.programs.iter().find_map(|(n, l)| l.as_ref().err().map(|r| (n, r))) {
        return Err(format!("cannot lock `{name}`: {reason}"));
    }
    let entries = lock::current(&project);
    let file = root.join(LOCK_FILE);
    if entries.is_empty() {
        if !file.exists() {
            return Ok("this program hands off to no program; nothing to lock".to_string());
        }
        fs::remove_file(&file).map_err(|e| format!("cannot remove {}: {e}", file.display()))?;
        return Ok(format!("removed {}: this program hands off to no program", file.display()));
    }
    let count = entries.len();
    fs::write(&file, lock::render(entries)).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    Ok(format!("wrote {} ({count} program(s))", file.display()))
}
