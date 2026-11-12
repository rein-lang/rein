//! `rein.lock`: the content hash of each program a project hands off to, so an edit to a called program is seen
//! before a run. See `docs/spec.md`, "The lock file".

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::diagnostic::{Diagnostic, Span};
use crate::flows::MANIFEST;
use crate::project::Project;

/// The lock file name, next to `rein.json` in the project root.
pub const LOCK_FILE: &str = "rein.lock";

/// The pin of one called program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// `sha256:` and 64 hex digits. See [`hash`].
    pub hash: String,
    /// `version` from the program's `package.json`, as a label for people. Not checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct LockFile {
    programs: BTreeMap<String, Entry>,
}

/// The hash of a program: SHA-256 over `rein.json` (when it exists) and every `.rein` file the project reader
/// loads, taken in order of relative path (`/` separators, byte order). Each file adds its path, a NUL byte, its
/// length as 8 big-endian bytes, and its bytes. The result is `sha256:<hex>`.
pub fn hash(program: &Project) -> String {
    let mut files: Vec<(String, Vec<u8>)> =
        program.files.iter().map(|(name, sf)| (format!("{name}.rein"), sf.text.clone().into_bytes())).collect();
    if let Ok(manifest) = fs::read(program.root.join(MANIFEST)) {
        files.push((MANIFEST.to_string(), manifest));
    }
    files.sort();
    let mut sha = Sha256::new();
    for (path, bytes) in &files {
        sha.update(path.as_bytes());
        sha.update([0]);
        sha.update((bytes.len() as u64).to_be_bytes());
        sha.update(bytes);
    }
    let hex: String = sha.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// The pin of each program the project hands off to, by name. A program that did not load is left out.
pub fn current(project: &Project) -> BTreeMap<String, Entry> {
    let mut out = BTreeMap::new();
    for (name, linked) in &project.programs {
        let Ok(linked) = linked else { continue };
        out.insert(name.clone(), Entry { hash: hash(&linked.project), version: package_version(&linked.project.root) });
    }
    out
}

fn package_version(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("version")?.as_str().map(str::to_string)
}

/// The text of `rein.lock` for `entries`.
pub fn render(entries: BTreeMap<String, Entry>) -> String {
    let mut text = serde_json::to_string_pretty(&LockFile { programs: entries }).unwrap_or_default();
    text.push('\n');
    text
}

/// Diagnostics of the lock for a project whose programs are linked. They are shown on `rein.lock`.
///
/// - No lock, and the project hands off to a program: a warning.
/// - A called program with no entry, or an entry with another hash: an error with the fix.
/// - An entry for a program the project does not call: a warning.
/// - A lock that is not valid: an error.
pub fn diagnostics(project: &Project) -> Vec<Diagnostic> {
    let warn = |m: String| Diagnostic::warning(Span::default(), m);
    let error = |m: String| Diagnostic::error(Span::default(), m);
    let called = current(project);
    let text = match fs::read_to_string(project.root.join(LOCK_FILE)) {
        Ok(text) => text,
        Err(_) if project.programs.is_empty() => return Vec::new(),
        Err(_) => return vec![warn(format!("no {LOCK_FILE}; run `rein lock` to pin the programs this one hands off to"))],
    };
    let locked = match serde_json::from_str::<LockFile>(&text) {
        Ok(file) => file.programs,
        Err(e) => return vec![error(format!("cannot read {LOCK_FILE}: {e}; run `rein lock` to write it again"))],
    };
    let mut out = Vec::new();
    for (name, now) in &called {
        match locked.get(name) {
            None => out.push(error(format!("program `{name}` is not in {LOCK_FILE}; run `rein lock` to pin it"))),
            Some(pin) if pin.hash != now.hash => out.push(error(format!(
                "program `{name}` changed: {LOCK_FILE} has {}, the installed program is {}; run `rein lock` to accept it",
                pin.hash, now.hash
            ))),
            Some(_) => {}
        }
    }
    for name in locked.keys().filter(|n| !project.programs.contains_key(*n)) {
        out.push(warn(format!("{LOCK_FILE} has `{name}`, which this program does not call; run `rein lock` to remove it")));
    }
    out
}

#[cfg(test)]
#[path = "lock_tests.rs"]
mod tests;
