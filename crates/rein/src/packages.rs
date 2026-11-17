//! Program packages: `rein install`, `rein uninstall` and `rein update`. Each passes its arguments to pnpm, in the npm
//! folder of the scope. `-g` or `--global` picks the user programs folder and is not passed on. Without it, the
//! folder is the repo's `rein/npm/`. The npm folder and its `package.json` are made on first use.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use rein_core::flows::{NPM_DIR, PACKAGE};
use serde_json::Value;

use crate::programs::triple;
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
    pnpm_args.extend(rest.clone());
    let bin = env::var("REIN_PNPM").unwrap_or_else(|_| "pnpm".to_string());
    if verb == "update" {
        let names: Vec<&str> = rest.iter().copied().filter(|a| !a.starts_with('-')).collect();
        if let Err(message) = move_tag_pins(&bin, &dir, &names) {
            return fail(&message);
        }
    }
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

/// `rein update` moves each tag-pinned GitHub program to the newest `v<x.y.z>` tag of its major. `pnpm update`
/// keeps a git tag fixed, so it cannot do this. `names` limits the programs, as `pnpm update` does. No names means
/// all of them.
/// `pnpm update` then runs as before.
fn move_tag_pins(bin: &str, dir: &Path, names: &[&str]) -> Result<(), String> {
    let path = dir.join(PACKAGE);
    let text = fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let manifest: Value = serde_json::from_str(&text).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let Some(deps) = manifest["dependencies"].as_object() else {
        return Ok(());
    };
    for (name, spec) in deps {
        if !names.is_empty() && !names.contains(&name.as_str()) {
            continue;
        }
        let Some((repo, current)) = tag_pin(spec.as_str().unwrap_or_default()) else {
            continue;
        };
        let tags = git_tags(repo)?;
        let Some(tag) = move_target(current, &tags) else {
            if let Some((top, _)) = newest_tag(&tags, None).filter(|(_, v)| v[0] > current[0]) {
                let major = current[0];
                eprintln!("note: {name} stays on major {major}; {repo} has {top}. Install that major by hand to move.");
            }
            continue;
        };
        let target = format!("github:{repo}#{tag}");
        let status = Command::new(bin)
            .args(["add", "--ignore-scripts", &target])
            .current_dir(dir)
            .status()
            .map_err(|e| format!("cannot run {bin}: {e}"))?;
        if !status.success() {
            return Err(format!("pnpm add {target} failed"));
        }
    }
    Ok(())
}

/// The repo and version of a `github:<owner>/<repo>#v<x.y.z>` spec. `None` for any other spec.
fn tag_pin(spec: &str) -> Option<(&str, [u64; 3])> {
    let (repo, tag) = spec.strip_prefix("github:")?.split_once('#')?;
    let valid = repo.matches('/').count() == 1 && repo.split('/').all(|part| !part.is_empty());
    valid.then_some((repo, version(tag)?))
}

/// `v<x.y.z>` as numbers. Pre-releases and other tags give `None`.
fn version(tag: &str) -> Option<[u64; 3]> {
    triple(tag.strip_prefix('v')?)
}

/// The tag a pin moves to: the highest `v<x.y.z>` tag of the pin's major, when it is higher than the pin.
/// A higher major is never taken, because it can break the program.
fn move_target(current: [u64; 3], ls_remote: &str) -> Option<String> {
    let (tag, newest) = newest_tag(ls_remote, Some(current[0]))?;
    (newest > current).then_some(tag)
}

/// The highest `v<x.y.z>` tag in `git ls-remote --tags --refs` output, with its tag name and numbers. `major`
/// limits the tags to one major version.
fn newest_tag(ls_remote: &str, major: Option<u64>) -> Option<(String, [u64; 3])> {
    ls_remote
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1)?.strip_prefix("refs/tags/"))
        .filter_map(|tag| Some((tag.to_string(), version(tag)?)))
        .filter(|(_, v)| major.is_none_or(|m| v[0] == m))
        .max_by_key(|(_, v)| *v)
}

/// The tags of a GitHub repo, as `git ls-remote` prints them.
fn git_tags(repo: &str) -> Result<String, String> {
    let url = format!("https://github.com/{repo}");
    let out = Command::new("git")
        .args(["ls-remote", "--tags", "--refs", &url])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git ls-remote {url} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn fail(message: &str) -> u8 {
    eprintln!("error: {message}");
    FAILED
}

#[cfg(test)]
#[path = "packages_tests.rs"]
mod tests;
