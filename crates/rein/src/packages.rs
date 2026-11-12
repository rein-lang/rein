//! Program packages: `rein install <package>`, `rein uninstall` and `rein update`. Each one runs pnpm in
//! `<user programs folder>/npm/`. The folder and its `package.json` are made on first use.

use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;

use rein_core::flows::{NPM_DIR, PACKAGE};

use crate::scopes;

const FAILED: u8 = 1;

/// The project manifest. It is private, so `npm publish` refuses it. pnpm adds the dependencies.
const PROJECT_MANIFEST: &str = "{\n  \"name\": \"rein-programs\",\n  \"private\": true\n}\n";

/// `pnpm add --ignore-scripts <spec>`. The spec goes to pnpm unchanged.
pub fn install(spec: &str) -> u8 {
    pnpm(&["add", "--ignore-scripts", spec])
}

/// `pnpm remove <name>`.
pub fn uninstall(name: &str) -> u8 {
    pnpm(&["remove", name])
}

/// `pnpm update --ignore-scripts [<name>]`. Without a name, every direct dependency moves within its range.
pub fn update(name: Option<&str>) -> u8 {
    let mut args = vec!["update", "--ignore-scripts"];
    args.extend(name);
    pnpm(&args)
}

/// Runs pnpm in the npm project. pnpm writes straight to this terminal, so its stderr passes through.
/// The exit code of pnpm is the exit code of Rein. `REIN_PNPM` names another pnpm binary.
fn pnpm(args: &[&str]) -> u8 {
    let dir = match project_dir() {
        Ok(dir) => dir,
        Err(message) => return fail(&message),
    };
    let bin = env::var("REIN_PNPM").unwrap_or_else(|_| "pnpm".to_string());
    match Command::new(&bin).args(args).current_dir(&dir).status() {
        Ok(status) => status.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(FAILED),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fail("pnpm is not installed or not on PATH; install pnpm (https://pnpm.io) or set REIN_PNPM to its path")
        }
        Err(e) => fail(&format!("cannot run {bin}: {e}")),
    }
}

/// `<user programs folder>/npm/`, created with its `package.json` if they are missing.
fn project_dir() -> Result<PathBuf, String> {
    let dir = scopes::user_dir().ok_or("no user programs folder; set HOME or REIN_HOME")?.join(NPM_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let manifest = dir.join(PACKAGE);
    if !manifest.exists() {
        fs::write(&manifest, PROJECT_MANIFEST).map_err(|e| format!("cannot write {}: {e}", manifest.display()))?;
    }
    Ok(dir)
}

fn fail(message: &str) -> u8 {
    eprintln!("error: {message}");
    FAILED
}
