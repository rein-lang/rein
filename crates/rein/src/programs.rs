//! Programs as named apps: `rein list`, `rein show`, and `rein install <path>`. The MCP host uses
//! the same functions.

use std::fs;
use std::path::{Path, PathBuf};

use rein_core::flows::{self, Scope, MANIFEST};
use rein_core::project::Project;
use rein_core::step::{invalid_request, StepResponse};
use serde_json::Value;

use crate::cli::flow_json;
use crate::live_run::Run;
use crate::scopes;

/// Every visible program as JSON: the `rein flows` fields plus `first_step`, the step a new run starts on.
pub fn catalog() -> Vec<Value> {
    scopes::discover_here()
        .iter()
        .map(|f| {
            let mut v = flow_json(f);
            let first = match Project::load(&f.root) {
                Ok(project) => Run::start(project, f.entry.clone()).last().clone(),
                Err(e) => invalid_request(format!("cannot read {}: {e}", f.root.display())),
            };
            v["first_step"] = serde_json::to_value(first).unwrap_or_default();
            v
        })
        .collect()
}

/// `rein list`: name, scope, path, description, and the first question of each program.
pub fn list(json: bool) -> u8 {
    let all = catalog();
    if json {
        println!("{}", Value::Array(all));
        return 0;
    }
    let text = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
    for p in &all {
        let step = &p["first_step"];
        let first = ["description", "instruction", "message"].iter().map(|k| text(step, k)).find(|s| !s.is_empty()).unwrap_or_default();
        let shadow = p["shadowed_by"].as_str().map_or(String::new(), |by| format!(" (shadowed by {by})"));
        println!("{}\t{}\t{}\t{}\tfirst: {first}{shadow}", text(p, "name"), text(p, "scope"), text(p, "path"), text(p, "description"));
    }
    0
}

/// The source of a program: the manifest, then each `.rein` file with the entry first. Each file
/// starts with a `// <path>` line.
pub fn source(name: &str) -> Result<String, String> {
    let flow = scopes::find(name)?;
    let project = Project::load(&flow.root).map_err(|e| format!("cannot read {}: {e}", flow.root.display()))?;
    let mut out = String::new();
    if let Ok(manifest) = fs::read_to_string(flow.root.join(MANIFEST)) {
        out.push_str(&format!("// {MANIFEST}\n{}\n", manifest.trim_end()));
    }
    let mut files: Vec<_> = project.files.iter().collect();
    files.sort_by_key(|(n, _)| **n != flow.entry);
    for (n, sf) in files {
        out.push_str(&format!("\n// {n}.rein\n{}", sf.text));
    }
    Ok(out)
}

/// `rein show <name>`: prints the program source.
pub fn show(name: &str) -> u8 {
    match source(name) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

/// `rein install <path>`: links a program folder into the user programs folder under its name.
pub fn install(path: &Path) -> u8 {
    match link(path) {
        Ok((name, target)) => {
            println!("installed {name} -> {}", target.display());
            0
        }
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

fn link(path: &Path) -> Result<(String, PathBuf), String> {
    let root = fs::canonicalize(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let flow = flows::program(root.clone(), Scope::User)
        .ok_or_else(|| format!("{} is not a program: it has no {MANIFEST} or main.rein", root.display()))?;
    // A trust boundary: the name becomes a path under the user folder.
    if flow.name.is_empty() || !flow.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("invalid program name `{}`", flow.name));
    }
    let project = Project::load(&root).map_err(|e| format!("cannot read {}: {e}", root.display()))?;
    if let StepResponse::Error { message, .. } = Run::start(project, flow.entry.clone()).last() {
        return Err(format!("{} cannot start: {message}", flow.name));
    }
    let dir = scopes::user_dir().ok_or("no user programs folder; set HOME or REIN_HOME")?;
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let target = dir.join(&flow.name);
    if target.symlink_metadata().is_ok() {
        return Err(format!("{} already exists", target.display()));
    }
    symlink(&root, &target).map_err(|e| format!("cannot link {}: {e}", target.display()))?;
    Ok((flow.name, target))
}

// ponytail: a link keeps the program live while its author edits it; copy instead if installs must be frozen.
#[cfg(unix)]
fn symlink(from: &Path, to: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(from, to)
}

#[cfg(not(unix))]
fn symlink(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("install needs symlinks; only Unix is supported"))
}
