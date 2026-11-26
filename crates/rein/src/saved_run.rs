//! Saved runs: a run kept in a file between calls, so a caller sends only its new answers. The file holds the
//! program, its root and entry, the inputs, and the answers so far. It never holds a step: each call steps the
//! flow again with `rein_core::step`, so a step is never stale.

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rein_core::project::Project;
use rein_core::step::{invalid_request, step, Answers, StepRequest, StepResponse, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cli::{exit_code, load_flow};
use crate::scopes;

const ENDED: &str = "the run has ended; start a new one";

/// One saved run, as its file holds it.
#[derive(Serialize, Deserialize)]
struct Saved {
    run: String,
    program: String,
    root: PathBuf,
    entry: String,
    #[serde(default)]
    inputs: BTreeMap<String, String>,
    #[serde(default)]
    answers: Answers,
}

/// Starts a run with its inputs. Returns the run id (none when the start fails), the entry, and the first step.
// ponytail: an error step saves nothing, so a failed start has no run id to retry with.
pub fn start(program: &str, inputs: BTreeMap<String, String>) -> (Option<String>, String, StepResponse) {
    let (root, entry) = match scopes::locate(program) {
        Ok(found) => found,
        Err(message) => return (None, String::new(), invalid_request(message)),
    };
    let project = match load_flow(Some(root.clone()), None) {
        Ok((project, _)) => project,
        Err(message) => return (None, entry, invalid_request(message)),
    };
    let saved = Saved { run: new_id(), program: program.to_string(), root, entry: entry.clone(), inputs, answers: Answers::new() };
    let first = evaluate(&project, &saved);
    if matches!(first, StepResponse::Error { .. }) {
        return (None, entry, first);
    }
    match write(&saved) {
        Ok(()) => (Some(saved.run), entry, first),
        Err(message) => (None, entry, invalid_request(message)),
    }
}

/// The step a run waits on, computed now from the saved inputs and answers.
pub fn status(run: &str) -> StepResponse {
    match open(run) {
        Ok((saved, project)) => evaluate(&project, &saved),
        Err(message) => invalid_request(message),
    }
}

/// Adds answers to a run and steps it. An answer that fails leaves the file as it was, and returns the error.
pub fn answer(run: &str, given: BTreeMap<String, String>) -> StepResponse {
    let (mut saved, project) = match open(run) {
        Ok(found) => found,
        Err(message) => return invalid_request(message),
    };
    if evaluate(&project, &saved).asked().is_none() {
        return invalid_request(ENDED);
    }
    if let Some(key) = given.keys().find(|key| saved.answers.contains_key(*key)) {
        return invalid_request(format!("answer `{key}` given twice"));
    }
    saved.answers.extend(given);
    let next = evaluate(&project, &saved);
    if matches!(next, StepResponse::Error { .. }) {
        return next;
    }
    match write(&saved) {
        Ok(()) => next,
        Err(message) => invalid_request(message),
    }
}

/// Answers the key the run waits on. Returns that key, and the next step. The key is empty when the run has no
/// key to answer.
pub fn answer_asked(run: &str, value: String) -> (String, StepResponse) {
    let current = status(run);
    match current.asked() {
        Some(key) => {
            let key = key.to_string();
            let next = answer(run, BTreeMap::from([(key.clone(), value)]));
            (key, next)
        }
        None if matches!(current, StepResponse::Error { .. }) => (String::new(), current),
        None => (String::new(), invalid_request(ENDED)),
    }
}

/// Deletes a run and its file.
pub fn stop(run: &str) -> Result<(), String> {
    fs::remove_file(file(run)?).map_err(|_| unknown(run))
}

/// Every saved run, with the step it waits on and its program. A file that does not open is skipped.
pub fn list() -> Vec<Value> {
    let Ok(folder) = dir() else { return Vec::new() };
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    let mut runs: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|name| name.strip_suffix(".json").map(str::to_string))
        .collect();
    runs.sort();
    runs.iter()
        .filter_map(|run| {
            let (saved, project) = open(run).ok()?;
            let mut v = value(Some(run), &evaluate(&project, &saved));
            v["program"] = saved.program.into();
            Some(v)
        })
        .collect()
}

/// The step as JSON, with the run id added as `run`.
pub fn value(run: Option<&str>, step: &StepResponse) -> Value {
    let mut v = serde_json::to_value(step).unwrap_or_default();
    if let (Some(id), Some(map)) = (run, v.as_object_mut()) {
        map.insert("run".into(), id.into());
    }
    v
}

/// `rein start PROGRAM --input name=value ...`: prints the run id and the first step as JSON.
pub fn start_cli(program: &str, inputs: &[String]) -> u8 {
    match pairs(inputs, "--input", "name=value") {
        Err(message) => print(None, &invalid_request(message)),
        Ok(inputs) => {
            let (run, _, first) = start(program, inputs);
            print(run.as_deref(), &first)
        }
    }
}

/// `rein status RUN`: prints the step the run waits on as JSON.
pub fn status_cli(run: &str) -> u8 {
    print(Some(run), &status(run))
}

/// `rein answer RUN key=value ...`: prints the next step as JSON.
pub fn answer_cli(run: &str, answers: &[String]) -> u8 {
    match pairs(answers, "answer", "key=value") {
        Err(message) => print(Some(run), &invalid_request(message)),
        Ok(given) => print(Some(run), &answer(run, given)),
    }
}

/// `rein stop RUN`: deletes the run.
pub fn stop_cli(run: &str) -> u8 {
    match stop(run) {
        Ok(()) => {
            println!("{}", json!({"run": run, "stopped": true}));
            0
        }
        Err(message) => print(Some(run), &invalid_request(message)),
    }
}

fn print(run: Option<&str>, step: &StepResponse) -> u8 {
    println!("{}", value(run, step));
    exit_code(step)
}

/// `name=value` items, one per flag. A name that repeats is an error.
fn pairs(items: &[String], flag: &str, shape: &str) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    for item in items {
        let (name, value) = item.split_once('=').ok_or_else(|| format!("invalid {flag} `{item}`; expected {shape}"))?;
        if map.insert(name.to_string(), value.to_string()).is_some() {
            return Err(format!("`{name}` given twice"));
        }
    }
    Ok(map)
}

fn evaluate(project: &Project, saved: &Saved) -> StepResponse {
    let request = StepRequest {
        version: PROTOCOL_VERSION,
        entry: saved.entry.clone(),
        answers: saved.answers.clone(),
        flow: None,
        inputs: saved.inputs.clone(),
        walk: false,
    };
    step(project, &request)
}

/// The saved run and its project, loaded from the root it was started in.
fn open(run: &str) -> Result<(Saved, Project), String> {
    let saved = read(run)?;
    let (project, _) = load_flow(Some(saved.root.clone()), None)?;
    Ok((saved, project))
}

fn read(run: &str) -> Result<Saved, String> {
    let text = fs::read_to_string(file(run)?).map_err(|_| unknown(run))?;
    serde_json::from_str(&text).map_err(|e| format!("cannot read run `{run}`: {e}"))
}

fn write(saved: &Saved) -> Result<(), String> {
    let folder = dir()?;
    make_folder(&folder)?;
    let tmp = folder.join(format!("{}.tmp", saved.run));
    let text = serde_json::to_string(saved).map_err(|e| e.to_string())?;
    open_private(&tmp)?
        .write_all(text.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, file(&saved.run)?).map_err(|e| format!("cannot save run `{}`: {e}", saved.run))
}

/// Opens a file for writing only by the user. The answers may hold private values.
fn open_private(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn make_folder(folder: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(folder).map_err(|e| format!("cannot make {}: {e}", folder.display()))
}

/// The file of a run id. Only letters, digits and `-` are allowed, so an id cannot leave the folder.
fn file(run: &str) -> Result<PathBuf, String> {
    let valid = !run.is_empty() && run.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return Err(format!("invalid run id `{run}`"));
    }
    Ok(dir()?.join(format!("{run}.json")))
}

fn unknown(run: &str) -> String {
    format!("unknown run `{run}`; rein_list or `rein runs` shows the saved runs")
}

/// `REIN_SAVED_RUNS_DIR`, else `$XDG_STATE_HOME/rein/saved-runs`, else `~/.local/state/rein/saved-runs`.
fn dir() -> Result<PathBuf, String> {
    dir_from(env::var_os("REIN_SAVED_RUNS_DIR"), env::var_os("XDG_STATE_HOME"), env::var_os("HOME"))
}

fn dir_from(saved: Option<OsString>, state: Option<OsString>, home: Option<OsString>) -> Result<PathBuf, String> {
    if let Some(saved) = saved.filter(|s| !s.is_empty()) {
        return Ok(saved.into());
    }
    if let Some(state) = state.filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(state).join("rein").join("saved-runs"));
    }
    home.filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".local/state/rein/saved-runs"))
        .ok_or_else(|| "no home folder: set REIN_SAVED_RUNS_DIR".to_string())
}

static COUNT: AtomicUsize = AtomicUsize::new(0);

/// A run id unique across processes: time, process id, and a counter for this process.
fn new_id() -> String {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    format!("run-{ms:x}-{}-{}", process::id(), COUNT.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
#[path = "saved_run_tests.rs"]
mod tests;
