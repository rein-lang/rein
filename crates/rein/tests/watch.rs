//! The writer and the reader together: `rein step` writes the run log, and `rein watch` reads it back over HTTP. A
//! contract mismatch between them fails here.

mod common;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};

use common::Client;
use serde_json::{json, Value};

/// A running `rein watch`. It is killed when the test ends, even when an assertion fails.
struct Watch {
    child: Child,
    // Kept open, so the server never writes to a closed pipe.
    _out: BufReader<ChildStdout>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/rein/pr-handling")
}

fn folder(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rein-watch-it-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// One `rein step` of the example program, with the run log in `runs`.
fn step(runs: &Path, answers: &[&str]) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rein"));
    cmd.arg("step")
        .arg(example().join("main.rein"))
        .arg("--input")
        .arg("pr=https://x/pr/1");
    for answer in answers {
        cmd.arg("--answer").arg(answer);
    }
    // The exit code of a waiting step is not zero, so the output is read, not the status.
    cmd.env("REIN_RUNS_DIR", runs)
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .output()
        .unwrap();
}

/// `rein watch` on a free port, reading `runs`. Its URL line tells the port.
fn watch(runs: &Path) -> (Watch, u16) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rein"))
        .args(["watch", "--no-open", "--port", "0"])
        .env("REIN_RUNS_DIR", runs)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let port = line
        .split("127.0.0.1:")
        .nth(1)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    (Watch { child, _out: out }, port)
}

/// The body of a GET with a local Host header.
fn get(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response.split_once("\r\n\r\n").unwrap().1.to_string()
}

#[test]
fn rein_step_runs_are_read_back_by_rein_watch() {
    let runs = folder("roundtrip");
    step(&runs, &["main.draft=no"]);
    step(&runs, &["main.draft=no", "main.task=merge"]);
    let (_watch, port) = watch(&runs);

    let agents: Value = serde_json::from_str(&get(port, "/api/agents")).unwrap();
    assert_eq!(agents.as_array().unwrap().len(), 1, "{agents}");
    let agent = &agents[0];
    assert_eq!(agent["transport"], "step", "{agent}");
    let runs_seen = agent["runs"].as_array().unwrap();
    assert_eq!(runs_seen.len(), 1, "the extending call continues the run: {agent}");
    let run = &runs_seen[0];
    // A `.rein` program path names its project folder, which the page shows.
    assert_eq!(run["program"], "pr-handling");
    assert_eq!(run["status"], "waiting");
    assert_eq!(run["step"]["key"], "merge.ci");
    let main = run["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "main")
        .unwrap();
    assert_eq!(
        main["answers"],
        serde_json::json!({"draft": "no", "task": "merge"}),
        "{run}"
    );
    fs::remove_dir_all(&runs).unwrap();
}

/// A running `rein serve --mcp`. It is killed when the test ends, even when an assertion fails.
struct Mcp {
    child: Child,
    client: Client,
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A `rein serve --mcp` that writes its run log to `runs`, after the MCP handshake.
fn serve_mcp(runs: &Path) -> Mcp {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rein"))
        .args(["serve", "--mcp"])
        .current_dir(&examples)
        .env("REIN_RUNS_DIR", runs)
        .env("REIN_SAVED_RUNS_DIR", runs.with_extension("saved"))
        .env("REIN_HOME", examples.join("no-user-programs"))
        .env("REIN_SYSTEM_DIRS", "")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut client = Client {
        stdin: child.stdin.take().unwrap(),
        stdout: BufReader::new(child.stdout.take().unwrap()),
        id: 0,
    };
    let init = client.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "rein", "{init}");
    client.notify("notifications/initialized");
    Mcp { child, client }
}

fn merge_pr() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rein/merge-pr/main.rein")
        .canonicalize()
        .unwrap()
}

/// The agents that `rein watch` on `port` lists.
fn api_agents(port: u16) -> Vec<Value> {
    let body: Value = serde_json::from_str(&get(port, "/api/agents")).unwrap();
    body.as_array().unwrap().clone()
}

// The MCP server writes each event before it answers the call, so the log is complete when the reply arrives.
// No polling is needed, and `rein watch` reads the log on each request.
#[test]
fn rein_serve_mcp_runs_are_read_back_by_rein_watch() {
    let runs = folder("mcp-writer");
    let mut mcp = serve_mcp(&runs);
    let first = mcp
        .client
        .call("rein_start", json!({"name": merge_pr().to_str().unwrap()}));
    assert_eq!(first["kind"], "question", "{first}");
    let run_id = first["run"].as_str().unwrap().to_string();
    assert_eq!(first["key"], "main.task", "{first}");
    let answered = mcp
        .client
        .call("rein_answer", json!({"run_id": run_id, "value": "merge"}));
    assert_eq!(answered["key"], "merge_pr.docs_only", "{answered}");

    let (_watch, port) = watch(&runs);
    let agents = api_agents(port);
    assert_eq!(agents.len(), 1, "{agents:?}");
    let agent = &agents[0];
    assert_eq!(agent["transport"], "mcp", "{agent}");
    assert_eq!(agent["alive"], true, "{agent}");
    assert_eq!(agent["pid"], mcp.child.id(), "{agent}");
    let runs_seen = agent["runs"].as_array().unwrap();
    assert_eq!(runs_seen.len(), 1, "{agent}");
    let run = &runs_seen[0];
    assert_eq!(run["run"], run_id.as_str(), "{run}");
    assert_eq!(run["status"], "waiting", "{run}");
    assert_eq!(run["step"]["key"], "merge_pr.docs_only", "{run}");
    assert_eq!(run["trace"], json!(["main", "merge_pr"]), "{run}");
    assert!(run["mermaid"].is_string(), "{run}");
    let main = run["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "main")
        .unwrap();
    assert_eq!(main["answers"], json!({"task": "merge"}), "{run}");
    fs::remove_dir_all(&runs).unwrap();
}

#[test]
fn rein_serve_mcp_is_listed_before_any_run() {
    let runs = folder("mcp-idle");
    let mcp = serve_mcp(&runs);
    let (_watch, port) = watch(&runs);
    let agents = api_agents(port);
    assert_eq!(agents.len(), 1, "{agents:?}");
    assert_eq!(agents[0]["transport"], "mcp", "{}", agents[0]);
    assert_eq!(agents[0]["pid"], mcp.child.id(), "{}", agents[0]);
    assert_eq!(agents[0]["runs"], json!([]), "{}", agents[0]);
    fs::remove_dir_all(&runs).unwrap();
}

#[test]
fn rein_watch_refuses_to_run_with_the_run_log_off() {
    let out = Command::new(env!("CARGO_BIN_EXE_rein"))
        .args(["watch", "--no-open", "--port", "0"])
        .env("REIN_RUNS_DIR", "")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("the run log is off: REIN_RUNS_DIR is empty"));
}
