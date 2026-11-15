//! The run log. Each rein process that steps a flow appends JSON Lines to a shared folder, so `rein watch` can show
//! every agent that uses rein. This module only writes. The folder rule and the line format are in language/interface.md in rein-lang/rein-lang.com.

use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rein_core::step::{StepRequest, StepResponse};
use serde_json::{json, Value};

/// The log folder: `REIN_RUNS_DIR`, else `$XDG_STATE_HOME/rein/runs`, else `~/.local/state/rein/runs`. `None` when
/// `REIN_RUNS_DIR` is set to the empty string, which turns logging off.
pub fn dir() -> Option<PathBuf> {
    dir_from(
        env::var_os("REIN_RUNS_DIR"),
        env::var_os("XDG_STATE_HOME"),
        env::var_os("HOME"),
    )
}

fn dir_from(
    runs: Option<OsString>,
    state: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    match runs {
        Some(runs) if runs.is_empty() => None,
        Some(runs) => Some(runs.into()),
        None => match state.filter(|s| !s.is_empty()) {
            Some(state) => Some(PathBuf::from(state).join("rein").join("runs")),
            None => home.map(|h| PathBuf::from(h).join(".local/state/rein/runs")),
        },
    }
}

/// The `CLAUDE_CODE_SESSION_ID` of this process, if set and not empty.
fn session() -> Option<String> {
    env::var("CLAUDE_CODE_SESSION_ID")
        .ok()
        .filter(|s| !s.is_empty())
}

/// The session part of a `step-<session>.jsonl` name: only `[A-Za-z0-9_-]`, else `nosession`.
fn session_key(session: Option<&str>) -> String {
    let key: String = session
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        .collect();
    if key.is_empty() {
        "nosession".into()
    } else {
        key
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn stamp(mut value: Value) -> Value {
    value["ts"] = now_ms().into();
    value
}

/// The first line of a file: which agent writes it.
fn header(transport: &str, pid: Option<u32>, session: Option<&str>) -> Value {
    json!({
        "ts": now_ms(),
        "event": "agent",
        "transport": transport,
        "pid": pid,
        "cwd": env::current_dir().ok().map(|d| d.display().to_string()),
        "session": session,
    })
}

/// Opens `path` for append. With `create_new` it fails when the file exists, so the caller knows it made the file.
fn open(path: &Path, create_new: bool) -> Option<File> {
    if let Some(folder) = path.parent() {
        make_folder(folder);
    }
    let mut options = OpenOptions::new();
    options.append(true).create(true).create_new(create_new);
    // The log holds answers and instructions, so only the user may read it.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path).ok()
}

/// Makes the log folder and any parents. On unix only the user may enter it.
fn make_folder(folder: &Path) {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    let _ = builder.create(folder);
}

/// Writes one line in one `write_all` call, so concurrent appends stay whole. Errors are ignored: a log failure
/// never changes the output of a command.
fn write_line(file: &mut File, value: &Value) {
    let mut text = value.to_string();
    text.push('\n');
    let _ = file.write_all(text.as_bytes());
}

/// The log of one process: `<transport>-<pid>-<start_ms>.jsonl`. The file is made at the first write, and the
/// header is its first line. An MCP server calls `announce` at start, so it has a file before any run.
// ponytail: no rotation. A long-running `rein serve --mcp` appends to one file until it exits.
pub struct Log {
    path: Option<PathBuf>,
    header: Option<Value>,
}

impl Log {
    /// The log of this process. `transport` is `mcp` or `run`. The start time is taken now, close enough to the start.
    pub fn process(transport: &str) -> Log {
        let pid = std::process::id();
        let session = session();
        let path = dir().map(|folder| folder.join(format!("{transport}-{pid}-{}.jsonl", now_ms())));
        Log {
            path,
            header: Some(header(transport, Some(pid), session.as_deref())),
        }
    }

    /// Writes the header now, so a process with no event yet still has its file.
    pub fn announce(&mut self) {
        self.append(None);
    }

    /// A run starts. `run` is `None` when the start failed and no run exists.
    pub fn start(
        &mut self,
        run: Option<&str>,
        program: Option<&str>,
        entry: &str,
        step: &StepResponse,
    ) {
        self.event(
            json!({"event": "start", "run": run, "program": program, "entry": entry, "step": step}),
        );
    }

    pub fn answer(&mut self, run: &str, key: &str, value: &str, step: &StepResponse) {
        self.event(
            json!({"event": "answer", "run": run, "key": key, "value": value, "step": step}),
        );
    }

    pub fn stop(&mut self, run: &str) {
        self.event(json!({"event": "stop", "run": run}));
    }

    /// Appends one event. The header goes first, on the first write only.
    fn event(&mut self, event: Value) {
        self.append(Some(event));
    }

    fn append(&mut self, event: Option<Value>) {
        let Some(path) = &self.path else { return };
        let Some(mut file) = open(path, false) else {
            return;
        };
        if let Some(header) = self.header.take() {
            write_line(&mut file, &header);
        }
        if let Some(event) = event {
            write_line(&mut file, &stamp(event));
        }
    }
}

/// One stateless `rein step`, in `step-<session>.jsonl`. The header is written only by the call that makes the file.
pub fn step(request: &StepRequest, program: Option<&str>, step: &StepResponse) {
    let Some(folder) = dir() else { return };
    let session = session();
    let path = folder.join(format!("step-{}.jsonl", session_key(session.as_deref())));
    let event =
        stamp(json!({"event": "step", "program": program, "request": request, "step": step}));
    match open(&path, true) {
        Some(mut file) => {
            write_line(&mut file, &header("step", None, session.as_deref()));
            write_line(&mut file, &event);
        }
        None => {
            if let Some(mut file) = open(&path, false) {
                write_line(&mut file, &event);
            }
        }
    }
}

#[cfg(test)]
#[path = "run_log_tests.rs"]
mod tests;
