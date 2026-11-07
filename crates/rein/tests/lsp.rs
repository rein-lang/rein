//! Drives `rein lsp` over stdio with framed JSON-RPC messages.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Kills the server if a test panics before it exits.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<Value>,
}

impl Server {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rein"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, messages) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(msg) = read_frame(&mut reader) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        Self { child, stdin: Some(stdin), messages }
    }

    fn send(&mut self, msg: Value) {
        let body = msg.to_string();
        let stdin = self.stdin.as_mut().unwrap();
        write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        stdin.flush().unwrap();
    }

    /// Waits for the next message that matches `pred`. Other messages are skipped.
    fn wait_for(&self, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let left = deadline.checked_duration_since(Instant::now()).expect("timed out waiting for message");
            let msg = self.messages.recv_timeout(left).expect("timed out waiting for message");
            if pred(&msg) {
                return msg;
            }
        }
    }

    fn response(&self, id: i64) -> Value {
        self.wait_for(|m| m["id"] == id)
    }

    fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.response(id)
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn open(&mut self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri, "languageId": "rein", "version": 1, "text": text}}),
        );
    }

    /// Waits for diagnostics published for `uri` that satisfy `pred`.
    fn wait_diagnostics(&self, uri: &str, pred: impl Fn(&Vec<Value>) -> bool) -> Vec<Value> {
        let msg = self.wait_for(|m| {
            m["method"] == "textDocument/publishDiagnostics"
                && m["params"]["uri"] == uri
                && pred(m["params"]["diagnostics"].as_array().unwrap())
        });
        msg["params"]["diagnostics"].as_array().unwrap().clone()
    }

    /// Shuts the server down and checks it exits with status 0.
    fn shutdown(&mut self) {
        let response = self.request(900, "shutdown", Value::Null);
        assert!(response["error"].is_null(), "{response}");
        self.notify("exit", Value::Null);
        // A client closes stdin after exit. The server's reader thread waits for that EOF.
        drop(self.stdin.take());
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "server exited with {status}");
                return;
            }
            assert!(Instant::now() < deadline, "server did not exit");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Reads one `Content-Length` framed JSON message. None at end of stream.
fn read_frame(reader: &mut BufReader<impl Read>) -> Option<Value> {
    let mut len = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length: ") {
            len = v.parse::<usize>().ok();
        }
    }
    let mut body = vec![0; len?];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Project with a valid target function. Returns its root.
fn temp_project(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rein-lsp-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("target.rein"),
        "/** Target function does X. */\nexport default function target() {\n  return \"done\";\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("main.rein"),
        "/** Entry. */\nexport default function main() {\n  return @./target;\n}\n",
    )
    .unwrap();
    root
}

fn uri_of(root: &Path, file: &str) -> String {
    format!("file://{}", root.join(file).display())
}

fn initialize(server: &mut Server) {
    let init = server.request(1, "initialize", json!({"capabilities": {}, "processId": null, "rootUri": null}));
    assert!(init["result"]["capabilities"]["hoverProvider"].as_bool().unwrap());
    server.notify("initialized", json!({}));
}

fn has_message(diags: &[Value], part: &str) -> bool {
    diags.iter().any(|d| d["message"].as_str().is_some_and(|m| m.contains(part)))
}

/// Text of the file that `start` imports `target` into. Line 4 returns the import.
const START: &str = "/** Start. */\nexport default function start() {\n  return @./target;\n}\n";

#[test]
fn diagnostics_hover_and_shutdown() {
    let root = temp_project("smoke");
    let mut server = Server::start();
    initialize(&mut server);

    // An import that does not resolve: the error arrives as a diagnostic.
    let caller = uri_of(&root, "caller.rein");
    server.open(&caller, "/** Caller. */\nexport default function caller() {\n  return @./nope;\n}\n");
    let diags = server.wait_diagnostics(&caller, |d| has_message(d, "cannot find function `@./nope`"));
    let error = diags.iter().find(|d| d["severity"] == 1).expect("error diagnostic");
    assert!(error["message"].as_str().unwrap().contains("cannot find function"));
    assert_eq!(error["source"], "rein");

    // Hover on a returned import shows the target's description.
    let start = uri_of(&root, "start.rein");
    server.open(&start, START);
    let hover = server.request(
        2,
        "textDocument/hover",
        json!({"textDocument": {"uri": start}, "position": {"line": 2, "character": 11}}),
    );
    let text = hover["result"]["contents"]["value"].as_str().unwrap();
    assert!(text.contains("Target function does X."), "{text}");

    // An unknown request gets MethodNotFound, not a crash.
    let unknown = server.request(3, "rein/unknown", json!({}));
    assert_eq!(unknown["error"]["code"], -32601);

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn fixing_one_file_clears_errors_in_another() {
    let root = temp_project("clear");
    let mut server = Server::start();
    initialize(&mut server);

    let start = uri_of(&root, "start.rein");
    server.open(&start, &START.replace("./target", "./fresh").replace("target", "fresh"));
    server.wait_diagnostics(&start, |d| has_message(d, "cannot find function `@./fresh`"));

    // Creating `fresh` in an open buffer must clear the error in `start`.
    server.open(&uri_of(&root, "fresh.rein"), "/** Fresh. */\nexport default function fresh() {\n  return \"ok\";\n}\n");
    let cleared = server.wait_diagnostics(&start, |d| !has_message(d, "cannot find module"));
    assert!(!has_message(&cleared, "cannot find module"));

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn hover_on_parameter_shows_its_type_and_doc() {
    let root = temp_project("param");
    let mut server = Server::start();
    initialize(&mut server);

    let start = uri_of(&root, "start.rein");
    server.open(
        &start,
        "/** Start. */\nexport default function start(\n  /** Does it change docs? */\n  docsOnly: boolean,\n  /** Which CI status? */\n  ci: \"green\" | \"red\",\n) {\n  if (docsOnly) {\n    return \"x\";\n  } else {\n    return \"y\";\n  }\n}\n",
    );
    let hover = server.request(
        2,
        "textDocument/hover",
        json!({"textDocument": {"uri": start}, "position": {"line": 3, "character": 4}}),
    );
    let text = hover["result"]["contents"]["value"].as_str().unwrap();
    assert!(text.contains("docsOnly: boolean"), "{text}");
    assert!(text.contains("Does it change docs?"), "{text}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn definition_and_completion_follow_references() {
    let root = temp_project("nav");
    let mut server = Server::start();
    initialize(&mut server);

    let start = uri_of(&root, "start.rein");
    server.open(&start, START);

    // Definition on the reference goes to the target file.
    let def = server.request(
        2,
        "textDocument/definition",
        json!({"textDocument": {"uri": start}, "position": {"line": 2, "character": 11}}),
    );
    assert!(def["result"]["uri"].as_str().unwrap().ends_with("/target.rein"), "{def}");

    // Typing `@./` after `return ` offers the project functions, relative to this file.
    let items = server.request(
        4,
        "textDocument/completion",
        json!({"textDocument": {"uri": start}, "position": {"line": 2, "character": 12}}),
    );
    let labels: Vec<&str> = items["result"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"@./target") && labels.contains(&"@./main"), "{labels:?}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn missing_main_is_reported_on_an_open_file() {
    let root = temp_project("nomain");
    fs::remove_file(root.join("main.rein")).unwrap();
    let mut server = Server::start();
    initialize(&mut server);

    let target = uri_of(&root, "target.rein");
    server.open(&target, "/** Target function does X. */\nexport default function target() {\n  return \"done\";\n}\n");
    let diags = server.wait_diagnostics(&target, |d| has_message(d, "project has no main.rein"));
    assert!(has_message(&diags, "project has no main.rein"));

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn closing_a_deleted_file_clears_its_diagnostics() {
    let root = temp_project("deleted");
    let caller_text = "/** Caller. */\nexport default function caller() {\n  return @./nope;\n}\n";
    fs::write(root.join("caller.rein"), caller_text).unwrap();
    let mut server = Server::start();
    initialize(&mut server);

    let caller = uri_of(&root, "caller.rein");
    server.open(&caller, caller_text);
    server.wait_diagnostics(&caller, |d| has_message(d, "cannot find function `@./nope`"));

    // The file goes away while its buffer is still open, then the editor closes the buffer.
    fs::remove_file(root.join("caller.rein")).unwrap();
    server.notify("textDocument/didClose", json!({"textDocument": {"uri": caller}}));
    let cleared = server.wait_diagnostics(&caller, |d| d.is_empty());
    assert!(cleared.is_empty());

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn deleting_a_marker_rechecks_open_files_under_the_new_root() {
    let root = temp_project("marker");
    let sub = root.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("main.rein"), "/** Sub. */\nexport default function sub() {\n  return @./x;\n}\n").unwrap();
    fs::write(root.join("outside.rein"), "/** Outside. */\nexport default function outside() {\n  return \"o\";\n}\n").unwrap();
    let x_text = "/** X. */\nexport default function x() {\n  return @../outside;\n}\n";
    fs::write(sub.join("x.rein"), x_text).unwrap();
    let mut server = Server::start();
    initialize(&mut server);

    // Under sub's own marker, `../outside` leaves the project root.
    let x = uri_of(&root, "sub/x.rein");
    server.open(&x, x_text);
    server.wait_diagnostics(&x, |d| has_message(d, "leaves the project root"));

    // Without the marker, sub belongs to the project of the root, and the import resolves.
    fs::remove_file(sub.join("main.rein")).unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes": [{"uri": uri_of(&root, "sub/main.rein"), "type": 3}]}),
    );
    let rechecked = server.wait_diagnostics(&x, |d| !has_message(d, "leaves the project root"));
    assert!(!has_message(&rechecked, "leaves the project root"));

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_nested_project_is_not_checked_as_part_of_the_outer_one() {
    let root = temp_project("nested");
    let sub = root.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("main.rein"), "/** Sub. */\nexport default function sub() {\n  return @./x;\n}\n").unwrap();
    let x_text = "/** X. */\nexport default function x() {\n  return @./nope;\n}\n";
    fs::write(sub.join("x.rein"), x_text).unwrap();
    let mut server = Server::start();
    initialize(&mut server);

    let x = uri_of(&root, "sub/x.rein");
    server.open(&x, x_text);
    server.wait_diagnostics(&x, |d| has_message(d, "cannot find function `@./nope`"));

    // Opening main makes the outer project check again. It must not add sub's file as an unreachable one.
    let main = uri_of(&root, "main.rein");
    server.open(&main, "/** Entry. */\nexport default function main() {\n  return @./target;\n}\n");
    let next = server.wait_diagnostics(&x, |_| true);
    assert!(has_message(&next, "cannot find function `@./nope`"), "{next:?}");
    assert!(!has_message(&next, "not reachable"), "{next:?}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn reference_completion_replaces_the_whole_token() {
    let root = temp_project("complete");
    fs::create_dir_all(root.join("ci")).unwrap();
    fs::write(root.join("ci/fix.rein"), "/** Fix. */\nexport default function fix() {\n  return \"f\";\n}\n").unwrap();
    let mut server = Server::start();
    initialize(&mut server);

    // The cursor sits after `@./ci/`. The reference token runs from the `@` (column 9) to column 17.
    let start = uri_of(&root, "start.rein");
    server.open(&start, "/** Start. */\nexport default function start() {\n  return @./ci/fix;\n}\n");
    let items = server.request(
        2,
        "textDocument/completion",
        json!({"textDocument": {"uri": start}, "position": {"line": 2, "character": 15}}),
    );
    let items = items["result"].as_array().unwrap();
    let labels: Vec<&str> = items.iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["@./ci/fix"], "{items:?}");
    let edit = &items[0]["textEdit"];
    assert_eq!(edit["newText"], "@./ci/fix", "{items:?}");
    assert_eq!(edit["range"]["start"]["character"], 9, "{items:?}");
    assert_eq!(edit["range"]["end"]["character"], 18, "{items:?}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn hover_and_completion_know_the_std_library() {
    let root = temp_project("std");
    let mut server = Server::start();
    initialize(&mut server);

    // std names need no import: hover on `delegate` shows its doc, and completion lists the std names.
    let start = uri_of(&root, "start.rein");
    server.open(
        &start,
        "/** Start. */\nexport default function start(\n  /** C */ c: \"x\" | \"y\",\n) {\n  if (c === \"x\") {\n    return delegate(\"a\", \"b\");\n  }\n  return \"z\";\n}\n",
    );
    let hover = server.request(
        2,
        "textDocument/hover",
        json!({"textDocument": {"uri": start}, "position": {"line": 5, "character": 12}}),
    );
    let text = hover["result"]["contents"]["value"].as_str().unwrap();
    assert!(text.contains("Hand a task to a subagent"), "{text}");

    let items = server.request(
        3,
        "textDocument/completion",
        json!({"textDocument": {"uri": start}, "position": {"line": 5, "character": 4}}),
    );
    let labels: Vec<&str> = items["result"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
    for name in ["askUser", "delegate", "request"] {
        assert!(labels.contains(&name), "{labels:?}");
    }

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}
