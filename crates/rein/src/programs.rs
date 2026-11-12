//! Programs as named apps: `rein list` and `rein show`. The MCP host uses the same functions.

use std::fs;

use rein_core::flows::MANIFEST;
use rein_core::project::Project;
use rein_core::step::invalid_request;
use serde_json::Value;

use crate::cli::flow_json;
use crate::live_run::Run;
use crate::scopes;

/// Every visible program as JSON: the `rein flows` fields plus `first_step`, the step a new run starts on.
/// A package whose `engines.rein` does not hold for this rein also has a `warning`.
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
            if let Some(warning) = f.engines_rein.as_deref().and_then(|range| engines_warning(range, RUNNING)) {
                v["warning"] = Value::String(warning);
            }
            v
        })
        .collect()
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
        let warning = p["warning"].as_str().map_or(String::new(), |w| format!(" (warning: {w})"));
        println!("{}\t{}\t{}\t{}\tfirst: {first}{shadow}{warning}", text(p, "name"), text(p, "scope"), text(p, "path"), text(p, "description"));
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

#[cfg(test)]
mod tests {
    use super::range_holds;

    #[test]
    fn caret_and_plain_ranges_hold_like_semver() {
        assert_eq!(range_holds("^0.1.0", "0.1.0"), Some(true));
        assert_eq!(range_holds("^0.1.0", "0.1.3"), Some(true));
        assert_eq!(range_holds("0.1.0", "0.1.0"), Some(true));
        assert_eq!(range_holds("^0.2.0", "0.1.0"), Some(false));
        assert_eq!(range_holds("^0.1.2", "0.1.1"), Some(false));
        assert_eq!(range_holds("^1.0.0", "0.1.0"), Some(false));
        assert_eq!(range_holds("^1.0.0", "1.4.2"), Some(true));
        assert_eq!(range_holds(">=0.1.0 <0.2.0", "0.1.0"), None);
    }
}
