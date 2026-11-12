//! Program packages: `rein install`, `rein uninstall` and `rein update`. Each passes its arguments to pnpm, in the npm
//! folder of the scope. `-g` or `--global` picks the user programs folder and is not passed on. Without it, the
//! folder is the repo's `rein/npm/`. The npm folder and its `package.json` are made on first use.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use rein_core::flows::{NPM_DIR, PACKAGE};

use crate::scopes;

const FAILED: u8 = 1;

/// The project manifest. It is private, so `npm publish` refuses it. pnpm adds the dependencies.
const PROJECT_MANIFEST: &str = "{\n  \"name\": \"rein-programs\",\n  \"private\": true\n}\n";

/// Runs `pnpm <verb> <args>` in the npm folder of the scope the arguments name. `verb` is `add`, `remove` or
/// `update`. `add` and `update` get `--ignore-scripts` first, unless the arguments already set it (for example
/// `--ignore-scripts=false`). pnpm's output goes straight to this terminal, and its exit code is ours.
/// `REIN_PNPM` names another pnpm binary.
pub fn run(verb: &str, args: &[String]) -> u8 {
    let global = args.iter().any(|a| is_global(a));
    let rest: Vec<&str> = args.iter().map(String::as_str).filter(|a| !is_global(a)).collect();
    let dir = match npm_dir(global) {
        Ok(dir) => dir,
        Err(message) => return fail(&message),
    };
    let mut pnpm_args = vec![verb];
    if verb != "remove" && !rest.iter().any(|a| a.starts_with("--ignore-scripts")) {
        pnpm_args.push("--ignore-scripts");
    }
    pnpm_args.extend(rest);
    let bin = env::var("REIN_PNPM").unwrap_or_else(|_| "pnpm".to_string());
    match Command::new(&bin).args(&pnpm_args).current_dir(&dir).status() {
        Ok(status) => status.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(FAILED),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fail("pnpm is not installed or not on PATH; install pnpm (https://pnpm.io) or set REIN_PNPM to its path")
        }
        Err(e) => fail(&format!("cannot run {bin}: {e}")),
    }
}

fn is_global(arg: &str) -> bool {
    arg == "-g" || arg == "--global"
}

/// `<user programs folder>/npm/` for `-g`, else `<project rein folder>/npm/`. Both are created with their
/// `package.json` when missing.
fn npm_dir(global: bool) -> Result<PathBuf, String> {
    let base = if global {
        scopes::user_dir().ok_or("no user programs folder; set HOME or REIN_HOME")?
    } else {
        let cwd = env::current_dir().map_err(|e| format!("cannot read the working directory: {e}"))?;
        scopes::project_rein_dir(&cwd)?
    };
    let dir = base.join(NPM_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let manifest = dir.join(PACKAGE);
    if !manifest.exists() {
        write_manifest(&manifest)?;
    }
    Ok(dir)
}

fn write_manifest(path: &Path) -> Result<(), String> {
    fs::write(path, PROJECT_MANIFEST).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn fail(message: &str) -> u8 {
    eprintln!("error: {message}");
    FAILED
}
