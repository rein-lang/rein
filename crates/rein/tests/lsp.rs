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
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::write(root.join("target.rein"), "description \"Target function does X.\"\nreturn \"done\"\n").unwrap();
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

#[test]
fn diagnostics_hover_and_shutdown() {
    let root = temp_project("smoke");
    let mut server = Server::start();
    initialize(&mut server);

    // Unknown @ref: the error arrives as a diagnostic.
    let caller = uri_of(&root, "caller.rein");
    server.open(&caller, "description \"Caller.\"\nreturn @nope\n");
    let diags = server.wait_diagnostics(&caller, |d| has_message(d, "`@nope`"));
    assert_eq!(diags[0]["severity"], 1);
    assert_eq!(diags[0]["source"], "rein");

    // Hover on a valid @ref shows the target's description.
    let start = uri_of(&root, "start.rein");
    server.open(&start, "description \"Start.\"\nreturn @target\n");
    let hover = server.request(
        2,
        "textDocument/hover",
        json!({"textDocument": {"uri": start}, "position": {"line": 1, "character": 9}}),
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
    server.open(&start, "description \"Start.\"\nreturn @fresh\n");
    server.wait_diagnostics(&start, |d| has_message(d, "`@fresh`"));

    // Creating `fresh` in an open buffer must clear the error in `start`.
    server.open(&uri_of(&root, "fresh.rein"), "description \"Fresh.\"\nreturn \"ok\"\n");
    let cleared = server.wait_diagnostics(&start, |d| d.is_empty());
    assert!(cleared.is_empty());

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}
