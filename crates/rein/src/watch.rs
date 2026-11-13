//! `rein watch`: a local page that lists the agents using rein now. For each agent: its runs, the program each runs,
//! its functions with their answers, and the step each run waits on. It reads the run logs the agents write, and
//! writes nothing.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Args;
use serde_json::{json, Map, Value};

use crate::watch_program::{self as program_of, Programs};
use crate::{graph as graph_cmd, http, run_log};

/// A step-file agent with no event for this long has ended.
const STEP_IDLE_MS: u64 = 10 * 60 * 1000;

/// How many errors of an agent the page keeps.
const MAX_ERRORS: usize = 5;

/// Flags of `rein watch`.
#[derive(Args)]
pub struct WatchArgs {
    /// Port on 127.0.0.1. Default: a free port.
    #[arg(long, default_value_t = 0)]
    port: u16,
    /// Do not open the page in the browser.
    #[arg(long)]
    no_open: bool,
}

pub fn run(a: &WatchArgs) -> u8 {
    match serve(a) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

fn serve(a: &WatchArgs) -> Result<(), String> {
    let Some(dir) = run_log::dir() else {
        return Err("the run log is off: REIN_RUNS_DIR is empty".to_string());
    };
    let listener =
        TcpListener::bind(("127.0.0.1", a.port)).map_err(|e| format!("cannot listen on 127.0.0.1:{}: {e}", a.port))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let url = format!("http://127.0.0.1:{port}/");
    println!("rein watch: {url} (Ctrl-C stops it)");
    if !a.no_open {
        graph_cmd::open(&url);
    }
    for stream in listener.incoming().flatten() {
        if let Err(e) = http::answer(stream, port, |method, path, _| respond(&dir, method, path)) {
            eprintln!("warning: {e}");
        }
    }
    Ok(())
}

/// The page, and the agents as JSON. Any other route is not found.
fn respond(dir: &Path, method: &str, path: &str) -> http::Reply {
    match (method, path) {
        ("GET", "/") => ("200 OK", "text/html", page()),
        ("GET", "/api/agents") => ("200 OK", "application/json", agents(dir, now_ms()).to_string()),
        _ => ("404 Not Found", "text/plain", "not found\n".to_string()),
    }
}

/// The page: its style, its script and the Mermaid build the script loads, inlined into one document.
fn page() -> String {
    include_str!("watch_page.html")
        .replace("__CSS__", include_str!("watch_page.css"))
        .replace("__SCRIPT__", &include_str!("watch_page.js").replace("__MERMAID__", graph_cmd::MERMAID))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Every agent whose log is in `dir`, newest first. `now` decides which step-file agents are still alive.
fn agents(dir: &Path, now: u64) -> Value {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    let mut programs = Programs::new();
    let mut list: Vec<Value> = paths.iter().filter_map(|p| agent(p, now, &mut programs)).collect();
    list.sort_by_key(|a| std::cmp::Reverse(a["last_ts"].as_u64().unwrap_or(0)));
    Value::Array(list)
}

/// One agent: the header of its log, its runs, and whether it is alive. None when the log has no header.
fn agent(path: &Path, now: u64, programs: &mut Programs) -> Option<Value> {
    let id = path.file_stem()?.to_str()?.to_string();
    let text = fs::read_to_string(path).ok()?;
    // A line that does not parse, or is not an object, is skipped.
    let lines: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(Value::is_object)
        .collect();
    let header = lines.iter().find(|v| v["event"] == "agent")?;
    let transport = header["transport"].as_str().unwrap_or_default().to_string();
    let pid = header["pid"].as_u64();
    let last_ts = lines.iter().map(ts_of).max().unwrap_or(0);
    let step = transport == "step";
    let alive = if step {
        now.saturating_sub(last_ts) < STEP_IDLE_MS
    } else {
        // ponytail: pid 0 would signal this process group, so it never counts as alive.
        pid.is_some_and(|p| p > 0 && process_alive(p))
    };
    let mut runs = if step { step_runs(&lines) } else { event_runs(&lines) };
    runs.sort_by_key(|r| std::cmp::Reverse(r.last_ts));
    let cwd = header["cwd"].as_str().unwrap_or_default();
    // Only the newest run is open on the page, so only its graph is built.
    let runs: Vec<Value> = runs.iter().enumerate().map(|(i, r)| r.to_json(programs, cwd, i == 0)).collect();
    // A start with no run id made no run. Its error is kept here, the last few.
    let errors: Vec<Value> = lines
        .iter()
        .filter(|v| v["event"] == "start" && v["run"].is_null() && v["step"]["kind"] == "error")
        .filter_map(|v| {
            // The log is written by other processes: a message that is not text is left out.
            let message = v["step"]["message"].as_str()?;
            Some(json!({"ts": ts_of(v), "program": string_or_null(&v["program"]), "message": message}))
        })
        .collect();
    let errors = errors[errors.len().saturating_sub(MAX_ERRORS)..].to_vec();
    Some(json!({
        "id": id, "transport": transport, "pid": pid, "cwd": string_or_null(&header["cwd"]),
        "session": string_or_null(&header["session"]),
        "alive": alive, "last_ts": last_ts, "runs": runs, "errors": errors,
    }))
}

/// Whether a process with this id is running. ponytail: one `kill -0` per agent per poll, fine for a few agents;
/// use libc::kill if the count grows.
fn process_alive(pid: u64) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The value when it is a string, else null. The log is written by other processes, so a field may hold anything.
fn string_or_null(v: &Value) -> Value {
    v.as_str().map_or(Value::Null, |s| json!(s))
}

fn ts_of(v: &Value) -> u64 {
    v["ts"].as_u64().unwrap_or(0)
}

/// One run, read from the log: a `start` and its answers and stop, or one group of stateless `step` calls.
struct Run {
    id: String,
    program: Value,
    entry: Value,
    started_ts: u64,
    last_ts: u64,
    /// The last step that is not an error, or null. An error leaves the run where it was.
    step: Value,
    /// The message of the last error step, or none.
    error: Option<String>,
    stopped: bool,
    answers: Map<String, Value>,
    inputs: Value,
    events: Vec<Value>,
}

impl Run {
    fn new(id: String, ts: u64) -> Run {
        Run {
            id,
            program: Value::Null,
            entry: Value::Null,
            started_ts: ts,
            last_ts: ts,
            step: Value::Null,
            error: None,
            stopped: false,
            answers: Map::new(),
            inputs: json!({}),
            events: Vec::new(),
        }
    }

    fn see_step(&mut self, step: &Value) {
        if !step.is_object() {
            return;
        }
        if step["kind"] == "error" {
            self.error = step["message"].as_str().map(str::to_string);
        } else {
            self.step = step.clone();
        }
    }

    /// `error` when no step has been good yet and the last step was an error.
    fn status(&self) -> &'static str {
        if self.stopped {
            "stopped"
        } else if self.step["kind"] == "done" {
            "done"
        } else if self.step.is_null() && self.error.is_some() {
            "error"
        } else {
            "waiting"
        }
    }

    /// The run as JSON. Only the newest run of an agent (`newest`) loads its program: it gets the graph and the
    /// changed-program check. Older runs list their functions from the trace and the answers.
    fn to_json(&self, programs: &mut Programs, cwd: &str, newest: bool) -> Value {
        let trace = self.step["trace"].as_array().cloned().unwrap_or_default();
        let folder = self.program.as_str().and_then(|p| program_of::resolve_folder(p, cwd));
        let program = folder
            .as_ref()
            .and_then(|f| f.file_name())
            .map_or_else(|| string_or_null(&self.program), |n| json!(n.to_string_lossy()));
        let name = self.program.as_str().unwrap_or_default();
        let loaded = if newest { program_of::program(programs, folder.as_deref(), name) } else { None };
        let names: Vec<String> = trace.iter().filter_map(Value::as_str).map(str::to_string).collect();
        // A program changed since the run gives no graph, rather than a wrong one.
        let changed = loaded.as_deref().is_some_and(|l| !program_of::matches_run(l, &names, &self.answers));
        let (mermaid, mermaid_full) = match (&loaded, newest) {
            (Some(l), true) if !changed => {
                let (path, full) = program_of::walk_mermaid(l, &self.step, self.entry.as_str().unwrap_or_default(), &names);
                (Some(path), Some(full))
            }
            _ => (None, None),
        };
        let functions = self.functions(loaded.as_deref().map(program_of::function_names), &trace);
        json!({
            "run": self.id, "program": program, "entry": self.entry, "status": self.status(),
            "started_ts": self.started_ts, "last_ts": self.last_ts, "step": self.step, "error": self.error,
            "trace": trace, "inputs": self.inputs, "functions": functions, "mermaid": mermaid, "mermaid_full": mermaid_full, "program_changed": changed,
            "events": self.events,
        })
    }

    /// The functions of the program, with `visited` from the trace and the answers that start with each name. A
    /// program that does not load lists the functions seen in the trace and the answers.
    fn functions(&self, loaded: Option<Vec<String>>, trace: &[Value]) -> Value {
        let trace: Vec<&str> = trace.iter().filter_map(Value::as_str).collect();
        let names: Vec<String> = loaded.unwrap_or_else(|| {
            let mut names: Vec<String> = Vec::new();
            let seen = trace.iter().map(|t| t.to_string());
            let answered = self
                .answers
                .keys()
                .filter_map(|k| rein_core::step::key_parts(k).map(|(f, _)| f.to_string()));
            for name in seen.chain(answered) {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            names
        });
        Value::Array(
            names
                .iter()
                .map(|name| {
                    let answers: Map<String, Value> = self
                        .answers
                        .iter()
                        .filter_map(|(k, v)| {
                            let (f, n) = rein_core::step::key_parts(k)?;
                            (f == name.as_str()).then(|| (n.to_string(), v.clone()))
                        })
                        .collect();
                    json!({"name": name, "visited": trace.contains(&name.as_str()), "answers": answers})
                })
                .collect(),
        )
    }
}

/// The run with this id. A new id adds a run at the end.
fn run_named<'a>(runs: &'a mut Vec<Run>, id: &str, ts: u64) -> &'a mut Run {
    let i = match runs.iter().position(|r| r.id == id) {
        Some(i) => i,
        None => {
            runs.push(Run::new(id.to_string(), ts));
            runs.len() - 1
        }
    };
    &mut runs[i]
}

/// The runs of an `mcp` or `run` log: events grouped by run id, in order of first appearance.
fn event_runs(lines: &[Value]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for v in lines {
        let (Some(id), Some(event)) = (v["run"].as_str(), v["event"].as_str()) else {
            continue;
        };
        if !matches!(event, "start" | "answer" | "stop") {
            continue;
        }
        let run = run_named(&mut runs, id, ts_of(v));
        run.last_ts = ts_of(v);
        run.events.push(v.clone());
        match event {
            "start" => {
                run.program = v["program"].clone();
                run.entry = v["entry"].clone();
                run.see_step(&v["step"]);
            }
            "answer" => {
                // A value the flow rejected (an error step about the same key) is not an answer.
                let rejected = v["step"]["kind"] == "error" && v["step"]["key"] == v["key"];
                if let (Some(key), false) = (v["key"].as_str(), rejected) {
                    run.answers.insert(key.to_string(), v["value"].clone());
                }
                run.see_step(&v["step"]);
            }
            _ => run.stopped = true,
        }
    }
    runs
}

/// The runs of a `step` log. A call continues the run before it when its program and entry match and its answers
/// extend that call's answers. Any other call starts a new run.
fn step_runs(lines: &[Value]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut previous: Option<(Value, Value, Map<String, Value>)> = None;
    for v in lines.iter().filter(|v| v["event"] == "step") {
        let request = &v["request"];
        let answers: Map<String, Value> = request["answers"].as_object().cloned().unwrap_or_default();
        let (program, entry) = (v["program"].clone(), request["entry"].clone());
        let continues = previous.as_ref().is_some_and(|(p, e, before)| {
            *p == program && *e == entry && before.iter().all(|(k, val)| answers.get(k) == Some(val))
        });
        if !continues {
            let mut run = Run::new(format!("step-{}", runs.len() + 1), ts_of(v));
            run.program = program.clone();
            run.entry = entry.clone();
            runs.push(run);
        }
        let Some(run) = runs.last_mut() else { continue };
        run.last_ts = ts_of(v);
        run.answers = answers.clone();
        if request["inputs"].is_object() {
            run.inputs = request["inputs"].clone();
        }
        run.events.push(v.clone());
        run.see_step(&v["step"]);
        previous = Some((program, entry, answers));
    }
    runs
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
