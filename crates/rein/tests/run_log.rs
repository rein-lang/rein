//! The run log: `rein run` and `rein step` append JSON Lines to `REIN_RUNS_DIR`, and write nothing when it is empty.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rein/support-ticket")
        .canonicalize()
        .unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rein-run-log-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `rein` with `args` and `env`. The log folder is set by the caller.
fn rein(args: &[&str], stdin: Option<&str>, env: &[(&str, &str)]) -> (u32, Output) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rein"));
    cmd.args(args)
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    let pid = child.id();
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
    (pid, child.wait_with_output().unwrap())
}

fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}")))
        .collect()
}

fn only_file(dir: &Path) -> PathBuf {
    let files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    files[0].clone()
}

#[test]
fn rein_run_writes_header_start_answers_and_stop() {
    let base = scratch("run");
    let runs = base.join("runs");
    let root = example();
    let env = [
        ("REIN_RUNS_DIR", runs.to_str().unwrap()),
        ("CLAUDE_CODE_SESSION_ID", "sess-run"),
        ("REIN_HOME", base.to_str().unwrap()),
        ("REIN_SYSTEM_DIRS", ""),
    ];
    let (pid, out) = rein(
        &["run", "--root", root.to_str().unwrap()],
        Some("no\nhow_to\nyes\nyes\n"),
        &env,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let file = only_file(&runs);
    let name = file.file_name().unwrap().to_str().unwrap().to_string();
    let start_ms = name
        .strip_prefix(&format!("run-{pid}-"))
        .and_then(|r| r.strip_suffix(".jsonl"))
        .unwrap_or_else(|| panic!("name {name}"));
    assert!(start_ms.parse::<u64>().is_ok(), "{name}");

    let log = lines(&file);
    assert_eq!(log.len(), 7, "{log:?}");
    assert_eq!(log[0]["event"], "agent");
    assert_eq!(log[0]["transport"], "run");
    assert_eq!(log[0]["pid"], pid);
    assert!(log[0]["cwd"].is_string(), "{}", log[0]);
    assert_eq!(log[0]["session"], "sess-run");
    assert!(log[0]["ts"].is_u64(), "{}", log[0]);

    assert_eq!(log[1]["event"], "start");
    assert_eq!(log[1]["run"], "run-1");
    assert_eq!(log[1]["program"], Value::Null);
    assert_eq!(log[1]["entry"], "main");
    assert_eq!(log[1]["step"]["kind"], "question");
    assert_eq!(log[1]["step"]["key"], "main.vip");

    assert_eq!(log[2]["event"], "answer");
    assert_eq!(log[2]["run"], "run-1");
    assert_eq!(log[2]["key"], "main.vip");
    assert_eq!(log[2]["value"], "no");
    assert_eq!(log[2]["step"]["kind"], "question");
    assert_eq!(log[3]["value"], "how_to");
    assert_eq!(log[6]["event"], "stop");
    assert_eq!(log[6]["run"], "run-1");
    assert_eq!(log[5]["step"]["kind"], "done");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn empty_runs_dir_turns_logging_off_and_writes_nothing() {
    let base = scratch("off");
    let root = example();
    let state = base.join("state");
    let home = base.join("home");
    let env = [
        ("REIN_RUNS_DIR", ""),
        ("XDG_STATE_HOME", state.to_str().unwrap()),
        ("HOME", home.to_str().unwrap()),
        ("REIN_HOME", base.to_str().unwrap()),
        ("REIN_SYSTEM_DIRS", ""),
    ];
    let (_, run) = rein(
        &["run", "--root", root.to_str().unwrap()],
        Some("no\nhow_to\nyes\nyes\n"),
        &env,
    );
    assert!(run.status.success());
    let (_, step) = rein(
        &[
            "step",
            "--root",
            root.to_str().unwrap(),
            "--answer",
            "main.vip=no",
        ],
        None,
        &env,
    );
    assert_eq!(step.status.code(), Some(2));
    assert!(
        !state.exists(),
        "logging off must not create {}",
        state.display()
    );
    assert!(
        !home.exists(),
        "logging off must not create {}",
        home.display()
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn rein_step_appends_one_step_line_per_call_and_one_header() {
    let base = scratch("step");
    let runs = base.join("runs");
    let root = example();
    let root_str = root.to_str().unwrap();
    let env = [
        ("REIN_RUNS_DIR", runs.to_str().unwrap()),
        ("CLAUDE_CODE_SESSION_ID", "sess-1"),
        ("REIN_SYSTEM_DIRS", ""),
    ];
    rein(
        &["step", "--root", root_str, "--answer", "main.vip=no"],
        None,
        &env,
    );
    rein(
        &[
            "step",
            "--root",
            root_str,
            "--answer",
            "main.vip=no",
            "--answer",
            "main.category=how_to",
        ],
        None,
        &env,
    );

    let file = runs.join("step-sess-1.jsonl");
    assert_eq!(only_file(&runs), file);
    let log = lines(&file);
    assert_eq!(log.len(), 3, "{log:?}");
    assert_eq!(log[0]["event"], "agent");
    assert_eq!(log[0]["transport"], "step");
    assert_eq!(log[0]["pid"], Value::Null);
    assert_eq!(log[0]["session"], "sess-1");
    assert_eq!(log[1]["event"], "step");
    assert_eq!(log[1]["program"], root_str);
    assert_eq!(
        log[1]["request"]["answers"],
        serde_json::json!({"main.vip": "no"})
    );
    assert_eq!(log[1]["step"]["kind"], "question");
    assert_eq!(log[2]["event"], "step");
    assert_eq!(log[2]["request"]["answers"]["main.category"], "how_to");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn serve_mcp_writes_its_header_at_start_even_with_no_run() {
    let base = scratch("mcp");
    let runs = base.join("runs");
    let env = [
        ("REIN_RUNS_DIR", runs.to_str().unwrap()),
        ("CLAUDE_CODE_SESSION_ID", "sess-mcp"),
    ];
    // Closing stdin before `initialize` makes the server exit with an error. The header is written before that.
    let (pid, _) = rein(&["serve", "--mcp"], Some(""), &env);

    let file = only_file(&runs);
    let name = file.file_name().unwrap().to_str().unwrap().to_string();
    assert!(name.starts_with(&format!("mcp-{pid}-")), "{name}");
    let log = lines(&file);
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0]["event"], "agent");
    assert_eq!(log[0]["transport"], "mcp");
    assert_eq!(log[0]["pid"], pid);
    assert_eq!(log[0]["session"], "sess-mcp");
    fs::remove_dir_all(&base).unwrap();
}
