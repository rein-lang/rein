//! Drives `rein serve --mcp` over stdio, as Claude Code or Codex would, through support-ticket.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

struct Client {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    id: u64,
}

impl Client {
    fn notify(&mut self, method: &str) {
        writeln!(self.stdin, "{}", json!({"jsonrpc": "2.0", "method": method})).unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        writeln!(self.stdin, "{}", json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params})).unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"))
    }

    /// Calls a tool and returns its text content, parsed as JSON.
    fn call(&mut self, name: &str, args: Value) -> Value {
        let reply = self.request("tools/call", json!({"name": name, "arguments": args}));
        let text = reply["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("no text: {reply}"));
        serde_json::from_str(text).unwrap_or_else(|e| panic!("tool text is not JSON ({e}): {text}"))
    }
}

#[test]
fn mcp_host_walks_support_ticket_to_done_and_keeps_the_run_on_a_bad_answer() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").canonicalize().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rein"))
        .env("REIN_RUNS_DIR", "")
        .args(["serve", "--mcp"])
        .current_dir(&examples)
        .env("REIN_HOME", examples.join("no-user-programs"))
        .env("REIN_SYSTEM_DIRS", "")
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

    let programs = client.call("rein_programs", json!({}));
    let ticket = programs.as_array().unwrap().iter().find(|p| p["name"] == "support-ticket").unwrap();
    assert_eq!(ticket["first_step"]["key"], "main.vip");

    let first = client.call("rein_start", json!({"name": "support-ticket"}));
    assert_eq!(first["kind"], "question");
    let run = first["run"].as_str().unwrap().to_string();

    let bad = client.call("rein_answer", json!({"run_id": run, "value": "maybe"}));
    assert_eq!(bad["kind"], "error");
    let status = client.call("rein_status", json!({"run_id": run}));
    assert_eq!(status["key"], "main.vip", "a bad answer keeps the run waiting on the same key");

    let mut last = Value::Null;
    for value in ["no", "how_to", "yes", "yes"] {
        last = client.call("rein_answer", json!({"run_id": run, "value": value}));
    }
    assert_eq!(last["kind"], "done", "{last}");
    assert_eq!(last["instruction"], "Tell the customer: Glad the help article solved the problem. Reply to this ticket if it comes back. Then close the ticket as\nsolved.");
    assert_eq!(last["trace"], json!(["main", "faq"]));

    let stopped = client.call("rein_stop", json!({"run_id": run}));
    assert_eq!(stopped["stopped"], true);
    drop(client);
    child.wait().unwrap();
}
