//! `rein migrate imports`: changes every file of a project to named imports. The rewrite is in
//! `rein_core::migrate_imports`; this file reads and writes the project and prints the result.

use std::fs;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use rein_core::diagnostic::line_col;
use rein_core::migrate_imports::migrate_text;
use rein_core::project::Project;

use crate::scopes;

const OK: u8 = 0;
const FAILED: u8 = 1;

#[derive(Subcommand)]
pub enum MigrateCommand {
    /// Change every `@` reference and default import of the project to a named import. Changes every file.
    Imports {
        /// Project directory or .rein file. Default: the current project.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Change nothing. List the files that would change, and exit 1 if there are any.
        #[arg(long)]
        check: bool,
    },
}

pub fn run(command: MigrateCommand) -> u8 {
    match command {
        MigrateCommand::Imports { path, check } => imports(&path, check),
    }
}

/// Prints one line per changed file, then `N files changed`. With `check`, prints the files that would change.
fn imports(given: &Path, check: bool) -> u8 {
    if !given.exists() {
        eprintln!("error: no such file or directory: {}", given.display());
        return FAILED;
    }
    let scope = std::path::absolute(given).unwrap_or_else(|_| given.to_path_buf());
    let root = Project::find_root(&scope);
    let project = match scopes::load_linked(&root) {
        Ok(project) => project,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", root.display());
            return FAILED;
        }
    };
    let mut changed: Vec<(PathBuf, String)> = Vec::new();
    for (name, sf) in &project.files {
        let migration = migrate_text(&project, name, &sf.text);
        for warning in &migration.warnings {
            let (line, _) = line_col(&sf.text, warning.span.start);
            eprintln!("{}:{}: warning: {}", sf.path.display(), line + 1, warning.message);
        }
        if migration.text != sf.text {
            changed.push((sf.path.clone(), migration.text));
        }
    }
    if check {
        for (path, _) in &changed {
            println!("{}", path.display());
        }
        return if changed.is_empty() { OK } else { FAILED };
    }
    for (path, text) in &changed {
        if let Err(e) = fs::write(path, text) {
            eprintln!("error: cannot write {}: {e}", path.display());
            return FAILED;
        }
        println!("{}", path.display());
    }
    println!("{} files changed", changed.len());
    OK
}
