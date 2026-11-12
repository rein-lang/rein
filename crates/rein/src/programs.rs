//! Programs as named apps: `rein show`, and the catalog the MCP host serves. `rein list` is in `list`.

use std::fs;

use rein_core::flows::{Flow, MANIFEST};
use rein_core::project::Project;
use rein_core::step::invalid_request;
use serde_json::Value;

use crate::cli::flow_json;
use crate::live_run::Run;
use crate::scopes;

/// Every visible program as JSON: the `record` fields plus `first_step`, the step a new run starts on.
pub fn catalog() -> Vec<Value> {
    scopes::discover_here().iter().map(|f| with_first_step(f, record(f))).collect()
}

/// One program as JSON: the `rein flows` fields. A package whose `engines.rein` does not hold for this rein also
/// has a `warning`.
pub fn record(f: &Flow) -> Value {
    let mut v = flow_json(f);
    if let Some(warning) = warning(f) {
        v["warning"] = Value::String(warning);
    }
    v
}

/// Adds `first_step` to a record: the step a new run of the program starts on. It runs the program's first
/// step, so only the JSON output asks for it.
pub fn with_first_step(f: &Flow, mut v: Value) -> Value {
    let first = match Project::load(&f.root) {
        Ok(project) => Run::start(project, f.entry.clone()).last().clone(),
        Err(e) => invalid_request(format!("cannot read {}: {e}", f.root.display())),
    };
    v["first_step"] = serde_json::to_value(first).unwrap_or_default();
    v
}

/// Why a program's `engines.rein` does not hold for this rein, if it does not.
pub fn warning(f: &Flow) -> Option<String> {
    f.engines_rein.as_deref().and_then(|range| engines_warning(range, RUNNING))
}

/// This rein's version. `engines.rein` ranges are checked against it.
const RUNNING: &str = env!("CARGO_PKG_VERSION");

/// A warning when `range` does not hold for `version`. `None` when it holds, or when `range_holds` cannot read it.
fn engines_warning(range: &str, version: &str) -> Option<String> {
    (range_holds(range, version) == Some(false)).then(|| format!("needs rein {range}; this is rein {version}"))
}

/// Whether `version` satisfies a caret range such as `^0.1.0`, or a plain `0.1.0`, which is read as a caret.
/// A caret keeps its first non-zero part: `^1.2.0` holds for 1.x at or above 1.2.0, and `^0.1.0` for 0.1.x at or
/// above 0.1.0. `None` when either side is not three numbers.
// ponytail: only caret and plain ranges are read. `>=`, `||` and spaced ranges are not checked; add the semver crate when they appear.
fn range_holds(range: &str, version: &str) -> Option<bool> {
    let want = triple(range.trim().trim_start_matches('^'))?;
    let have = triple(version)?;
    let lead = want.iter().position(|&p| p != 0).unwrap_or(2);
    Some(have[..=lead] == want[..=lead] && have >= want)
}

fn triple(version: &str) -> Option<[u64; 3]> {
    let parts: Vec<u64> = version.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    <[u64; 3]>::try_from(parts).ok()
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

#[cfg(test)]
#[path = "programs_tests.rs"]
mod tests;
