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
        Self::start_with(&[])
    }

    /// Starts the server with extra environment variables, such as a temp `REIN_HOME`.
    fn start_with(env: &[(&str, &Path)]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rein"))
            .env("REIN_RUNS_DIR", "")
            .arg("lsp")
            .envs(env.iter().copied())
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

    /// Sends a request, which the server answers after the notifications sent before it. Returns one list per
    /// publish for `uri` that came first, so a publish with no diagnostics is seen too.
    fn publishes_for(&mut self, uri: &str) -> Vec<Vec<Value>> {
        const SYNC: i64 = 800;
        self.send(json!({
            "jsonrpc": "2.0", "id": SYNC, "method": "textDocument/documentSymbol",
            "params": {"textDocument": {"uri": uri}}
        }));
        let deadline = Instant::now() + TIMEOUT;
        let mut seen = Vec::new();
        loop {
            let left = deadline.checked_duration_since(Instant::now()).expect("timed out waiting for the server");
            let msg = self.messages.recv_timeout(left).expect("timed out waiting for the server");
            if msg["id"] == SYNC {
                return seen;
            }
            if msg["method"] == "textDocument/publishDiagnostics" && msg["params"]["uri"] == uri {
                seen.push(msg["params"]["diagnostics"].as_array().unwrap().clone());
            }
        }
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
        "/** Target function does X. */\nexport default function target() {\n  return `done`;\n}\n",
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
fn a_function_over_the_line_limit_is_an_error_on_its_header() {
    let root = temp_project("long");
    let mut server = Server::start();
    initialize(&mut server);

    // 41 lines from `export default` to `}`: the header, 38 comment lines, the return and the brace.
    let long = uri_of(&root, "long.rein");
    let text = format!("/** Long. */\nexport default function long() {{\n{}  return `done`;\n}}\n", "  // a step\n".repeat(38));
    server.open(&long, &text);
    let diags = server.wait_diagnostics(&long, |d| has_message(d, "has 41 lines; the limit is 40."));
    let error = diags.iter().find(|d| d["severity"] == 1).expect("error diagnostic");
    assert_eq!(error["range"]["start"], json!({"line": 1, "character": 0}));
    assert_eq!(error["range"]["end"]["line"], 1);
    assert_eq!(error["source"], "rein");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_line_over_the_length_limit_is_an_error_on_its_overflow() {
    let root = temp_project("width");
    let mut server = Server::start();
    initialize(&mut server);

    // Line 3 is `  return "` + 69 letters + `";`: 81 characters, so the overflow is the `;`.
    let wide = uri_of(&root, "wide.rein");
    let text = format!("/** Wide. */\nexport default function wide() {{\n  return `{}`;\n}}\n", "a".repeat(69));
    server.open(&wide, &text);
    let diags = server.wait_diagnostics(&wide, |d| has_message(d, "line has 81 characters; the limit is 80."));
    let error = diags
        .iter()
        .find(|d| d["message"].as_str().is_some_and(|m| m.starts_with("line has 81 characters")))
        .expect("line length error");
    assert_eq!(error["severity"], 1);
    assert_eq!(error["range"]["start"], json!({"line": 2, "character": 80}));
    assert_eq!(error["range"]["end"], json!({"line": 2, "character": 81}));
    assert_eq!(error["source"], "rein");

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
    server.open(&uri_of(&root, "fresh.rein"), "/** Fresh. */\nexport default function fresh() {\n  return `ok`;\n}\n");
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
        "interface Input {\n  /** Does it change docs? */\n  docs_only: boolean;\n  /** Which CI status? */\n  ci: \"green\" | \"red\";\n}\n\n/** Start. */\nexport default function start({ docs_only, ci }: Input) {\n  if (docs_only) {\n    return `x`;\n  } else {\n    return `y`;\n  }\n}\n",
    );
    let hover = server.request(
        2,
        "textDocument/hover",
        json!({"textDocument": {"uri": start}, "position": {"line": 2, "character": 4}}),
    );
    let text = hover["result"]["contents"]["value"].as_str().unwrap();
    assert!(text.contains("docs_only: boolean"), "{text}");
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
    server.open(&target, "/** Target function does X. */\nexport default function target() {\n  return `done`;\n}\n");
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
    fs::write(root.join("outside.rein"), "/** Outside. */\nexport default function outside() {\n  return `o`;\n}\n").unwrap();
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
fn a_file_in_a_skipped_folder_gets_a_hint_why() {
    let root = temp_project("skipped-hint");
    let target_dir = root.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    let text = "/** Y. */\nexport default function y() {\n  return `done`;\n}\n";
    fs::write(target_dir.join("y.rein"), text).unwrap();
    let mut server = Server::start();
    initialize(&mut server);

    // The loader skips `target/`, so the file has no language features. The hint says why, at the top of the file.
    let y = uri_of(&root, "target/y.rein");
    server.open(&y, text);
    let diags = server.wait_diagnostics(&y, |d| has_message(d, "does not load files in `target/`"));
    let hint = diags.iter().find(|d| d["severity"] == 4).expect("hint diagnostic");
    assert!(hint["message"].as_str().unwrap().contains("move this file"), "{hint}");
    assert_eq!(hint["range"], json!({"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}));

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

/// How many skipped-folder hints a publish holds.
fn hint_count(diags: &[Value]) -> usize {
    diags.iter().filter(|d| d["severity"] == 4 && d["message"].as_str().is_some_and(|m| m.contains("does not load"))).count()
}

/// Opens `file` under `root` with `text`, then checks that no publish for it carries the skipped-folder hint.
fn assert_no_skipped_hint(server: &mut Server, root: &Path, file: &str, text: &str) {
    let uri = uri_of(root, file);
    server.open(&uri, text);
    let published = server.publishes_for(&uri);
    assert!(published.iter().all(|d| hint_count(d) == 0), "{published:?}");
}

#[test]
fn a_skipped_hint_clears_when_its_file_closes() {
    let root = temp_project("hint-close");
    let target_dir = root.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    let text = "/** Y. */\nexport default function y() {\n  return `done`;\n}\n";
    fs::write(target_dir.join("y.rein"), text).unwrap();
    let mut server = Server::start();
    initialize(&mut server);

    let y = uri_of(&root, "target/y.rein");
    server.open(&y, text);
    server.wait_diagnostics(&y, |d| has_message(d, "does not load files in `target/`"));

    // The buffer closes, so the file is no longer open. Its hint goes with it, as an empty publish.
    server.notify("textDocument/didClose", json!({"textDocument": {"uri": y}}));
    let cleared = server.wait_diagnostics(&y, |d| d.is_empty());
    assert!(cleared.is_empty());

    // Opening another file does not publish the closed file again.
    let main = uri_of(&root, "main.rein");
    server.open(&main, &fs::read_to_string(root.join("main.rein")).unwrap());
    let later = server.publishes_for(&y);
    assert!(later.is_empty(), "{later:?}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_skipped_hint_is_published_once_per_publish() {
    let root = temp_project("hint-once");
    let target_dir = root.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    let text = "/** Y. */\nexport default function y() {\n  return `done`;\n}\n";
    fs::write(target_dir.join("y.rein"), text).unwrap();
    let mut server = Server::start();
    initialize(&mut server);

    let y = uri_of(&root, "target/y.rein");
    server.open(&y, text);
    let first = server.wait_diagnostics(&y, |d| has_message(d, "does not load"));
    assert_eq!(hint_count(&first), 1, "{first:?}");

    let edited = text.replace("done", "gone");
    server.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": y, "version": 2}, "contentChanges": [{"text": edited}]}),
    );
    let again = server.wait_diagnostics(&y, |d| has_message(d, "does not load"));
    assert_eq!(hint_count(&again), 1, "{again:?}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_program_installed_under_node_modules_gets_no_hint() {
    let root = temp_project("installed");
    let pkg = root.join("node_modules/foo");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("rein.json"), r#"{"entry":"start.rein"}"#).unwrap();
    let text = "/** Start. */\nexport default function start() {\n  return `s`;\n}\n";
    fs::write(pkg.join("start.rein"), text).unwrap();
    let mut server = Server::start();
    initialize(&mut server);

    assert_no_skipped_hint(&mut server, &root, "node_modules/foo/start.rein", text);

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_program_installed_by_pnpm_gets_no_hint() {
    let root = temp_project("pnpm");
    let pkg = root.join("node_modules/.pnpm/foo@1.0.0/node_modules/foo");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("rein.json"), r#"{"entry":"start.rein"}"#).unwrap();
    let text = "/** Start. */\nexport default function start() {\n  return `s`;\n}\n";
    fs::write(pkg.join("start.rein"), text).unwrap();
    let mut server = Server::start();
    initialize(&mut server);

    assert_no_skipped_hint(&mut server, &root, "node_modules/.pnpm/foo@1.0.0/node_modules/foo/start.rein", text);

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
    fs::write(root.join("ci/fix.rein"), "/** Fix. */\nexport default function fix() {\n  return `f`;\n}\n").unwrap();
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
        "/** Start. */\nexport default function start(\n  /** C */ c: \"x\" | \"y\",\n) {\n  if (c === \"x\") {\n    return delegate(model=`a`, task=`b`);\n  }\n  return `z`;\n}\n",
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
    for name in ["agentSetup", "askUser", "delegate", "request"] {
        assert!(labels.contains(&name), "{labels:?}");
    }

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

/// The real example program `examples/rein/<name>`, as a canonical path.
fn example(name: &str) -> PathBuf {
    fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/rein").join(name)).unwrap()
}

/// Opens `file` of the project at `root` with its text on disk. Returns its URI.
fn open_file(server: &mut Server, root: &Path, file: &str) -> String {
    let uri = uri_of(root, file);
    server.open(&uri, &fs::read_to_string(root.join(file)).unwrap());
    uri
}

type Spot = (String, u64, u64, u64, u64);

fn spot(file: &str, start_line: u64, start_col: u64, end_line: u64, end_col: u64) -> Spot {
    (file.to_string(), start_line, start_col, end_line, end_col)
}

/// One location as (file under `root`, start line, start column, end line, end column).
fn spot_of(root: &Path, location: &Value) -> Spot {
    let prefix = format!("file://{}/", root.display());
    let r = &location["range"];
    let n = |v: &Value| v.as_u64().unwrap();
    (
        location["uri"].as_str().unwrap().strip_prefix(&prefix).unwrap().to_string(),
        n(&r["start"]["line"]),
        n(&r["start"]["character"]),
        n(&r["end"]["line"]),
        n(&r["end"]["character"]),
    )
}

/// Each location of a list, sorted.
fn spots(root: &Path, locations: &Value) -> Vec<Spot> {
    let mut out: Vec<Spot> = locations.as_array().unwrap().iter().map(|l| spot_of(root, l)).collect();
    out.sort();
    out
}

fn at(uri: &str, line: u64, character: u64) -> Value {
    json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": character}})
}

/// Every `@./escalate` in examples/rein/support-ticket.
fn escalate_calls() -> Vec<Spot> {
    vec![
        spot("faq.rein", 8, 11, 8, 22),
        spot("faq.rein", 24, 9, 24, 20),
        spot("main.rein", 12, 11, 12, 22),
        spot("main.rein", 19, 13, 19, 24),
        spot("refund.rein", 8, 11, 8, 22),
    ]
}

#[test]
fn references_list_every_call_of_a_function() {
    let root = example("support-ticket");
    let mut server = Server::start();
    initialize(&mut server);
    let escalate = open_file(&mut server, &root, "escalate.rein");
    let main = open_file(&mut server, &root, "main.rein");

    // From the function's own name, from anywhere in its file, and from a reference to it.
    for (id, uri, line, character) in [(2, &escalate, 3, 26), (3, &escalate, 4, 20), (4, &main, 12, 15)] {
        let mut params = at(uri, line, character);
        params["context"] = json!({"includeDeclaration": false});
        let refs = server.request(id, "textDocument/references", params);
        assert_eq!(spots(&root, &refs["result"]), escalate_calls(), "{refs}");
    }

    // With the declaration, the function's own name is in the list too.
    let mut params = at(&main, 12, 15);
    params["context"] = json!({"includeDeclaration": true});
    let refs = server.request(5, "textDocument/references", params);
    let mut expected = escalate_calls();
    expected.push(spot("escalate.rein", 3, 24, 3, 32));
    expected.sort();
    assert_eq!(spots(&root, &refs["result"]), expected, "{refs}");

    server.shutdown();
}

#[test]
fn references_of_a_parameter_include_named_arguments() {
    let root = example("support-ticket");
    let mut server = Server::start();
    initialize(&mut server);
    let refund = open_file(&mut server, &root, "refund.rein");

    // From `opening=` in the call: the argument, the struct field, each destructuring, and each `${opening}` use.
    let mut params = at(&refund, 12, 6);
    params["context"] = json!({"includeDeclaration": true});
    let refs = server.request(2, "textDocument/references", params);
    assert_eq!(
        spots(&root, &refs["result"]),
        vec![
            spot("faq.rein", 19, 6, 19, 13),
            spot("faq/close_ticket.rein", 3, 38, 3, 45),
            spot("faq/close_ticket.rein", 4, 31, 4, 38),
            spot("message.struct.rein", 5, 2, 5, 9),
            spot("refund.rein", 12, 4, 12, 11),
            spot("refund/tell_customer.rein", 3, 39, 3, 46),
            spot("refund/tell_customer.rein", 4, 12, 4, 19),
        ],
        "{refs}"
    );

    server.shutdown();
}

#[test]
fn code_lens_counts_references_and_carries_them() {
    let root = example("support-ticket");
    let mut server = Server::start();
    initialize(&mut server);
    let escalate = open_file(&mut server, &root, "escalate.rein");
    let main = open_file(&mut server, &root, "main.rein");

    let lenses = server.request(2, "textDocument/codeLens", json!({"textDocument": {"uri": escalate}}));
    let lenses = lenses["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 1, "{lenses:?}");
    let lens = &lenses[0];
    assert_eq!(lens["range"], json!({"start": {"line": 3, "character": 24}, "end": {"line": 3, "character": 32}}));
    assert_eq!(lens["command"]["title"], "5 references");
    assert_eq!(lens["command"]["command"], "rein.showReferences");
    let args = &lens["command"]["arguments"];
    assert_eq!(args[0], escalate);
    assert_eq!(args[1], json!({"line": 3, "character": 24}));
    assert_eq!(spots(&root, &args[2]), escalate_calls());

    let lenses = server.request(3, "textDocument/codeLens", json!({"textDocument": {"uri": main}}));
    assert_eq!(lenses["result"][0]["command"]["title"], "0 references", "{lenses}");

    server.shutdown();
}

#[test]
fn definition_goes_to_functions_parameters_and_std() {
    let root = example("support-ticket");
    let mut server = Server::start();
    initialize(&mut server);
    let refund = open_file(&mut server, &root, "refund.rein");

    // A reference goes to the name of the function it names.
    let def = server.request(2, "textDocument/definition", at(&refund, 11, 20));
    assert_eq!(spot_of(&root, &def["result"]), spot("refund/tell_customer.rein", 3, 24, 3, 36), "{def}");
    // `opening=` in a call goes to that field of the struct.
    let def = server.request(3, "textDocument/definition", at(&refund, 12, 6));
    assert_eq!(spot_of(&root, &def["result"]), spot("message.struct.rein", 5, 2, 5, 9), "{def}");
    // A parameter used in the body goes to its declaration.
    let def = server.request(4, "textDocument/definition", at(&refund, 7, 8));
    assert_eq!(spot_of(&root, &def["result"]), spot("refund.rein", 5, 2, 5, 19), "{def}");

    // A std template goes to its source. `request` is built in and has none.
    let tmp = temp_project("stddef");
    let start = uri_of(&tmp, "start.rein");
    server.open(
        &start,
        "/** Start. */\nexport default async function start() {\n  /** Ok? */\n  const ok: boolean = await request(`a`);\n  return delegate(model=`a`, task=`b`);\n}\n",
    );
    let def = server.request(5, "textDocument/definition", at(&start, 4, 12));
    assert!(def["result"]["uri"].as_str().unwrap().ends_with("/rein-std/delegate.rein"), "{def}");
    assert_eq!(def["result"]["range"], json!({"start": {"line": 10, "character": 24}, "end": {"line": 10, "character": 32}}));
    let def = server.request(6, "textDocument/definition", at(&start, 3, 30));
    assert!(def["result"].is_null(), "{def}");

    server.shutdown();
    fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn hover_shows_function_docs_and_callee_parameters() {
    let root = example("support-ticket");
    let mut server = Server::start();
    initialize(&mut server);
    let escalate = open_file(&mut server, &root, "escalate.rein");
    let refund = open_file(&mut server, &root, "refund.rein");

    // On the function's own name: its doc comment.
    let hover = server.request(2, "textDocument/hover", at(&escalate, 3, 26));
    let text = hover["result"]["contents"]["value"].as_str().unwrap();
    assert!(text.contains("Hand the ticket to a human support engineer."), "{text}");
    assert_eq!(hover["result"]["range"], json!({"start": {"line": 3, "character": 24}, "end": {"line": 3, "character": 32}}));

    // On `opening=` in a call: the callee's parameter type and question.
    let hover = server.request(3, "textDocument/hover", at(&refund, 12, 6));
    let text = hover["result"]["contents"]["value"].as_str().unwrap();
    assert!(text.contains("opening: string"), "{text}");
    assert!(text.contains("The first part of the sentence, such as the action taken."), "{text}");

    server.shutdown();
}

/// The names and reference spans of call hierarchy calls, under key `item` ("from" or "to").
fn calls_of(root: &Path, result: &Value, item: &str) -> Vec<(String, Vec<Spot>)> {
    result
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let uri = c[item]["uri"].clone();
            let ranges = c["fromRanges"].as_array().unwrap();
            let spots = ranges.iter().map(|r| spot_of(root, &json!({"uri": uri, "range": r}))).collect();
            (c[item]["name"].as_str().unwrap().to_string(), spots)
        })
        .collect()
}

#[test]
fn call_hierarchy_lists_incoming_and_outgoing_calls() {
    let root = example("support-ticket");
    let mut server = Server::start();
    initialize(&mut server);
    let escalate = open_file(&mut server, &root, "escalate.rein");
    let main = open_file(&mut server, &root, "main.rein");

    let items = server.request(2, "textDocument/prepareCallHierarchy", at(&escalate, 3, 26));
    let item = items["result"][0].clone();
    assert_eq!(item["name"], "escalate");
    assert_eq!(item["detail"], "Hand the ticket to a human support engineer.");
    assert_eq!(spot_of(&root, &json!({"uri": item["uri"], "range": item["selectionRange"]})), spot("escalate.rein", 3, 24, 3, 32));

    let incoming = server.request(3, "callHierarchy/incomingCalls", json!({"item": item}));
    assert_eq!(
        calls_of(&root, &incoming["result"], "from"),
        vec![
            ("faq".to_string(), vec![spot("faq.rein", 8, 11, 8, 22), spot("faq.rein", 24, 9, 24, 20)]),
            ("main".to_string(), vec![spot("main.rein", 12, 11, 12, 22), spot("main.rein", 19, 13, 19, 24)]),
            ("refund".to_string(), vec![spot("refund.rein", 8, 11, 8, 22)]),
        ],
        "{incoming}"
    );

    let items = server.request(4, "textDocument/prepareCallHierarchy", at(&main, 10, 25));
    let outgoing = server.request(5, "callHierarchy/outgoingCalls", json!({"item": items["result"][0]}));
    let names: Vec<String> = calls_of(&root, &outgoing["result"], "to").into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["escalate", "faq", "refund"], "{outgoing}");
    // Outgoing ranges are in the caller's file.
    let ranges = &outgoing["result"][0]["fromRanges"];
    assert_eq!(ranges[0], json!({"start": {"line": 12, "character": 11}, "end": {"line": 12, "character": 22}}));
    assert_eq!(ranges[1], json!({"start": {"line": 19, "character": 13}, "end": {"line": 19, "character": 24}}));

    server.shutdown();
}

#[test]
fn document_symbols_outline_the_function_and_its_names() {
    let root = example("support-ticket");
    let mut server = Server::start();
    initialize(&mut server);
    let faq = open_file(&mut server, &root, "faq.rein");

    let symbols = server.request(2, "textDocument/documentSymbol", json!({"textDocument": {"uri": faq}}));
    let function = &symbols["result"][0];
    assert_eq!(function["name"], "faq");
    assert_eq!(function["kind"], 12);
    assert_eq!(spot_of(&root, &json!({"uri": faq, "range": function["selectionRange"]})), spot("faq.rein", 3, 30, 3, 39));
    let children: Vec<(String, String, Spot)> = function["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let at = spot_of(&root, &json!({"uri": faq, "range": c["selectionRange"]}));
            (c["name"].as_str().unwrap().to_string(), c["detail"].as_str().unwrap().to_string(), at)
        })
        .collect();
    assert_eq!(
        children,
        vec![
            ("article_found".to_string(), "boolean".to_string(), spot("faq.rein", 5, 2, 5, 15)),
            ("solved".to_string(), "boolean".to_string(), spot("faq.rein", 12, 8, 12, 14)),
        ]
    );

    server.shutdown();
}

#[test]
fn workspace_symbols_search_every_program() {
    let examples = example("");
    let mut server = Server::start();
    let folders = json!([{"uri": format!("file://{}", examples.display()), "name": "examples"}]);
    let init = server.request(1, "initialize", json!({"capabilities": {}, "processId": null, "workspaceFolders": folders}));
    assert!(init["result"]["capabilities"]["workspaceSymbolProvider"].as_bool().unwrap());
    server.notify("initialized", json!({}));

    let found = server.request(2, "workspace/symbol", json!({"query": "refund"}));
    let found = found["result"].as_array().unwrap();
    // The query also matches `refund/tell_customer`, which is inside the `refund` folder.
    assert_eq!(found.len(), 2, "{found:?}");
    let refund = found.iter().find(|s| s["name"] == "refund").expect("refund symbol");
    assert_eq!(refund["containerName"], "support-ticket");
    let root = example("support-ticket");
    assert_eq!(spot_of(&root, &refund["location"]), spot("refund.rein", 3, 24, 3, 30));
    let tell = found.iter().find(|s| s["name"] == "refund/tell_customer").expect("tell_customer symbol");
    assert_eq!(spot_of(&root, &tell["location"]), spot("refund/tell_customer.rein", 3, 24, 3, 36));

    // An empty query lists every function of the five example programs.
    let all = server.request(3, "workspace/symbol", json!({"query": ""}));
    let all_symbols = all["result"].as_array().unwrap();
    assert_eq!(all_symbols.len(), 26, "{all}");
    assert!(
        all_symbols.iter().any(|s| s["name"] == "message" && s["kind"] == 23 && s["containerName"] == "support-ticket"),
        "{all}"
    );

    server.shutdown();
}

/// Starts a server whose client can rename files in a workspace edit.
fn start_with_file_renames() -> Server {
    let mut server = Server::start();
    let caps = json!({"workspace": {"workspaceEdit": {"documentChanges": true, "resourceOperations": ["create", "rename", "delete"]}}});
    let init = server.request(1, "initialize", json!({"capabilities": caps, "processId": null, "rootUri": null}));
    assert!(init["result"]["capabilities"]["renameProvider"]["prepareProvider"].as_bool().unwrap());
    server.notify("initialized", json!({}));
    server
}

/// Each text edit of a workspace edit as (location, new text), sorted, and the file renames as (old, new).
type EditSummary = (Vec<(Spot, String)>, Vec<(String, String)>);

fn edits_of(root: &Path, edit: &Value) -> EditSummary {
    let mut texts = Vec::new();
    let mut renames = Vec::new();
    let prefix = format!("file://{}/", root.display());
    let file = |v: &Value| v.as_str().unwrap().strip_prefix(&prefix).unwrap().to_string();
    for change in edit["documentChanges"].as_array().unwrap() {
        if change["kind"] == "rename" {
            renames.push((file(&change["oldUri"]), file(&change["newUri"])));
            continue;
        }
        for e in change["edits"].as_array().unwrap() {
            let location = json!({"uri": change["textDocument"]["uri"], "range": e["range"]});
            texts.push((spot_of(root, &location), e["newText"].as_str().unwrap().to_string()));
        }
    }
    texts.sort();
    (texts, renames)
}

#[test]
fn rename_a_parameter_updates_named_arguments() {
    let root = example("support-ticket");
    let mut server = start_with_file_renames();
    let refund = open_file(&mut server, &root, "refund.rein");

    let prepared = server.request(2, "textDocument/prepareRename", at(&refund, 12, 6));
    assert_eq!(prepared["result"]["placeholder"], "opening");
    assert_eq!(prepared["result"]["range"], json!({"start": {"line": 12, "character": 4}, "end": {"line": 12, "character": 11}}));

    let mut params = at(&refund, 12, 6);
    params["newName"] = json!("lead");
    let edit = server.request(3, "textDocument/rename", params);
    let lead = |s: Spot| (s, "lead".to_string());
    assert_eq!(
        edits_of(&root, &edit["result"]),
        (
            vec![
                lead(spot("faq.rein", 19, 6, 19, 13)),
                lead(spot("faq/close_ticket.rein", 3, 38, 3, 45)),
                lead(spot("faq/close_ticket.rein", 4, 31, 4, 38)),
                lead(spot("message.struct.rein", 5, 2, 5, 9)),
                lead(spot("refund.rein", 12, 4, 12, 11)),
                lead(spot("refund/tell_customer.rein", 3, 39, 3, 46)),
                lead(spot("refund/tell_customer.rein", 4, 12, 4, 19)),
            ],
            vec![]
        ),
        "{edit}"
    );

    // A name that is not an identifier fails the request.
    let mut params = at(&refund, 12, 6);
    params["newName"] = json!("2x");
    let failed = server.request(4, "textDocument/rename", params);
    assert_eq!(failed["error"]["code"], -32803, "{failed}");

    server.shutdown();
}

#[test]
fn rename_a_function_renames_its_file_and_references() {
    let root = example("support-ticket");
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    // On a reference, only the file name part is renamed.
    let prepared = server.request(2, "textDocument/prepareRename", at(&main, 12, 15));
    assert_eq!(prepared["result"]["placeholder"], "escalate");
    assert_eq!(prepared["result"]["range"], json!({"start": {"line": 12, "character": 14}, "end": {"line": 12, "character": 22}}));

    let mut params = at(&main, 12, 15);
    params["newName"] = json!("handoff");
    let edit = server.request(3, "textDocument/rename", params);
    let handoff = |s: Spot| (s, "handoff".to_string());
    assert_eq!(
        edits_of(&root, &edit["result"]),
        (
            vec![
                handoff(spot("escalate.rein", 3, 24, 3, 32)),
                handoff(spot("faq.rein", 8, 14, 8, 22)),
                handoff(spot("faq.rein", 24, 12, 24, 20)),
                handoff(spot("main.rein", 12, 14, 12, 22)),
                handoff(spot("main.rein", 19, 16, 19, 24)),
                handoff(spot("refund.rein", 8, 14, 8, 22)),
            ],
            vec![("escalate.rein".to_string(), "handoff.rein".to_string())]
        ),
        "{edit}"
    );
    // The file rename comes after every text edit, so the edits apply to the old file names.
    let changes = edit["result"]["documentChanges"].as_array().unwrap();
    assert_eq!(changes.last().unwrap()["kind"], "rename");

    server.shutdown();
}

#[test]
fn rename_a_function_renames_the_folder_of_its_nested_functions() {
    let root = fs::canonicalize(temp_project("rename-folder")).unwrap();
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("main.rein", "/** Entry. */\nexport default function main() {\n  return @./a;\n}\n");
    write("a.rein", "/** A. */\nexport default function a() {\n  return @./a/b;\n}\n");
    write("a/b.rein", "/** B. */\nexport default function b() {\n  return @../target;\n}\n");
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("c");
    let edit = server.request(2, "textDocument/rename", params);
    assert_eq!(
        edits_of(&root, &edit["result"]),
        (
            vec![
                (spot("a.rein", 1, 24, 1, 25), "c".to_string()),
                (spot("a.rein", 2, 9, 2, 15), "@./c/b".to_string()),
                (spot("main.rein", 2, 12, 2, 13), "c".to_string()),
            ],
            vec![
                ("a.rein".to_string(), "c.rein".to_string()),
                ("a/b.rein".to_string(), "c/b.rein".to_string()),
            ]
        ),
        "{edit}"
    );

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn rename_refuses_to_move_a_folder_that_holds_the_entry() {
    let root = fs::canonicalize(temp_project("rename-entry-folder")).unwrap();
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("rein.json", r#"{"entry": "app/start.rein"}"#);
    write("app/start.rein", "/** Start. */\nexport default function start() {\n  return @../app;\n}\n");
    write("app.rein", "/** App. */\nexport default function app() {\n  return `a`;\n}\n");
    let mut server = start_with_file_renames();
    let start = open_file(&mut server, &root, "app/start.rein");

    let mut params = at(&start, 2, 13);
    params["newName"] = json!("web");
    let edit = server.request(2, "textDocument/rename", params);
    let message = edit["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("holds the entry `app/start`"), "{edit}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn rename_refuses_to_move_a_folder_into_a_path_the_project_skips() {
    let root = fs::canonicalize(temp_project("rename-skipped-folder")).unwrap();
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("main.rein", "/** Entry. */\nexport default function main() {\n  return @./a;\n}\n");
    write("a.rein", "/** A. */\nexport default function a() {\n  return @./a/b;\n}\n");
    write("a/b.rein", "/** B. */\nexport default function b() {\n  return `b`;\n}\n");
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("node_modules");
    let edit = server.request(2, "textDocument/rename", params);
    let message = edit["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("`node_modules/b` is a path the project does not load"), "{edit}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn rename_refuses_to_move_a_folder_into_another_flow() {
    let root = fs::canonicalize(temp_project("rename-nested-flow")).unwrap();
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("main.rein", "/** Entry. */\nexport default function main() {\n  return @./a;\n}\n");
    write("a.rein", "/** A. */\nexport default function a() {\n  return @./a/b;\n}\n");
    write("a/b.rein", "/** B. */\nexport default function b() {\n  return `b`;\n}\n");
    write("tools/main.rein", "/** Tools. */\nexport default function main() {\n  return `t`;\n}\n");
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("tools");
    let edit = server.request(2, "textDocument/rename", params);
    let message = edit["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("`tools/b` is a path the project does not load"), "{edit}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

/// A project where `a` holds `a/b`, which calls `../target`. Returns its root, canonical.
fn folder_project(tag: &str) -> PathBuf {
    let root = fs::canonicalize(temp_project(tag)).unwrap();
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("main.rein", "/** Entry. */\nexport default function main() {\n  return @./a;\n}\n");
    write("a.rein", "/** A. */\nexport default function a() {\n  return @./a/b;\n}\n");
    write("a/b.rein", "/** B. */\nexport default function b() {\n  return `b`;\n}\n");
    root
}

#[test]
fn rename_refuses_to_leave_a_file_of_the_folder_behind() {
    let root = folder_project("rename-left-behind");
    fs::write(root.join("a/notes.md"), "Notes.\n").unwrap();
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("c");
    let edit = server.request(2, "textDocument/rename", params);
    let message = edit["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("`a/notes.md` is not a `.rein` file of the project"), "{edit}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn rename_refuses_to_move_through_a_symlinked_folder() {
    let root = folder_project("rename-symlink");
    fs::create_dir_all(root.join("elsewhere")).unwrap();
    fs::write(root.join("elsewhere/x.md"), "X.\n").unwrap();
    std::os::unix::fs::symlink(root.join("elsewhere"), root.join("a/linked")).unwrap();
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("c");
    let edit = server.request(2, "textDocument/rename", params);
    let message = edit["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("`a/linked` is a symbolic link"), "{edit}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn rename_refuses_to_move_a_folder_holding_a_hidden_file_symlink() {
    let root = folder_project("rename-hidden-file-symlink");
    fs::write(root.join("shared.env"), "KEY=1\n").unwrap();
    std::os::unix::fs::symlink("../shared.env", root.join("a/.env")).unwrap();
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("c");
    let edit = server.request(2, "textDocument/rename", params);
    let message = edit["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("`a/.env` is a symbolic link"), "{edit}");
    // Nothing moved: the folder and its link are where they were.
    assert!(root.join("a/b.rein").is_file(), "{edit}");
    assert!(fs::symlink_metadata(root.join("a/.env")).unwrap().file_type().is_symlink(), "{edit}");
    assert!(!root.join("c").exists(), "{edit}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn rename_keeps_a_broken_reference_that_keeps_its_meaning() {
    let root = folder_project("rename-broken-same-meaning");
    // `a` drafts `@./todo`, and `a/b` refers to `@../nope`. Neither exists, and neither names a function after the
    // rename to `c`: `todo` and `nope` are at the root in both places.
    fs::write(root.join("a.rein"), "/** A. */\nexport default function a() {\n  return @./todo;\n}\n").unwrap();
    fs::write(root.join("a/b.rein"), "/** B. */\nexport default function b() {\n  return @../nope;\n}\n").unwrap();
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("c");
    let edit = server.request(2, "textDocument/rename", params);
    assert_eq!(
        edits_of(&root, &edit["result"]),
        (
            vec![
                (spot("a.rein", 1, 24, 1, 25), "c".to_string()),
                (spot("main.rein", 2, 12, 2, 13), "c".to_string()),
            ],
            vec![
                ("a.rein".to_string(), "c.rein".to_string()),
                ("a/b.rein".to_string(), "c/b.rein".to_string()),
            ]
        ),
        "{edit}"
    );

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn rename_refuses_a_broken_reference_that_would_resolve_after_the_move() {
    let root = folder_project("rename-broken-ref");
    // `a/b` refers to `../c`, which does not exist yet. Renaming `a` to `c` creates `c.rein`, and from `c/b` the
    // same text names it.
    fs::write(root.join("a/b.rein"), "/** B. */\nexport default function b() {\n  return @../c;\n}\n").unwrap();
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("c");
    let edit = server.request(2, "textDocument/rename", params);
    let message = edit["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("`@../c` in `a/b` does not resolve, but after the move it would resolve to `c`"),
        "{edit}"
    );

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn rename_refuses_a_broken_reference_in_a_file_that_stays_put_that_would_resolve_after_the_move() {
    let root = folder_project("rename-broken-stays-put");
    // `x` stays at the root and refers to `@./c`, which does not exist. Renaming `a` to `c` creates `c.rein`, and
    // the same text in `x` names it, although `x` does not move.
    fs::write(root.join("x.rein"), "/** X. */\nexport default function x() {\n  return @./c;\n}\n").unwrap();
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("c");
    let edit = server.request(2, "textDocument/rename", params);
    let message = edit["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("`@./c` in `x` does not resolve, but after the move it would resolve to `c`"),
        "{edit}"
    );

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

/// `steps/fix_ci` is only called by `flow/merge`, so the quick fix moves it to `flow/merge/fix_ci.rein`, one folder
/// deeper. It refers to `@../../nope`, which leaves the root from `steps` but names `nope` from `flow/merge`.
fn escaping_reference_project(tag: &str, nope: bool) -> (PathBuf, Value) {
    let root = fs::canonicalize(temp_project(tag)).unwrap();
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("main.rein", "/** Entry. */\nexport default function main() {\n  return @./flow/merge;\n}\n");
    write("flow/merge.rein", "/** Merge. */\nexport default function merge() {\n  return @../steps/fix_ci;\n}\n");
    write("steps/fix_ci.rein", "/** Fix CI. */\nexport default function fixCi() {\n  return @../../nope;\n}\n");
    if nope {
        write("nope.rein", "/** Nope. */\nexport default function nope() {\n  return `n`;\n}\n");
    }
    let mut server = start_with_file_renames();
    let fix_ci = open_file(&mut server, &root, "steps/fix_ci.rein");

    let part = "`steps/fix_ci` is only called by `flow/merge`; move it to `flow/merge/fix_ci.rein`";
    let diags = server.wait_diagnostics(&fix_ci, |d| has_message(d, part));
    let diag = diags.iter().find(|d| d["message"].as_str().unwrap().contains(part)).unwrap().clone();
    let params = json!({"textDocument": {"uri": fix_ci}, "range": diag["range"], "context": {"diagnostics": [diag]}});
    let actions = server.request(2, "textDocument/codeAction", params);
    server.shutdown();
    (root, actions)
}

#[test]
fn quick_fix_moves_a_function_deeper_when_its_escaping_reference_still_names_nothing() {
    // From `steps/fix_ci`, `@../../nope` leaves the root. From `flow/merge/fix_ci` it names `nope`, which is not a
    // file, so the move changes nothing about the reference and is allowed.
    let (root, actions) = escaping_reference_project("escape-allowed", false);
    let action = &actions["result"][0];
    assert_eq!(action["title"], "Move to flow/merge/fix_ci.rein", "{actions}");
    assert_eq!(
        edits_of(&root, &action["edit"]),
        (
            vec![(spot("flow/merge.rein", 2, 9, 2, 25), "@./merge/fix_ci".to_string())],
            vec![("steps/fix_ci.rein".to_string(), "flow/merge/fix_ci.rein".to_string())]
        ),
        "{actions}"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn quick_fix_refuses_a_move_that_makes_an_escaping_reference_name_a_file() {
    // With `nope.rein` at the root, `@../../nope` from `flow/merge/fix_ci` names it after the move.
    let (root, actions) = escaping_reference_project("escape-refused", true);
    let offered = actions["result"].as_array().unwrap().iter().any(|a| a["title"] == "Move to flow/merge/fix_ci.rein");
    assert!(!offered, "{actions}");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn rename_a_function_to_a_name_that_differs_only_in_case() {
    let root = fs::canonicalize(temp_project("rename-case")).unwrap();
    // Only a case-insensitive file system (macOS, Windows, or a casefold folder) finds `TARGET.rein` in this folder.
    if !root.join("TARGET.rein").exists() {
        eprintln!("skipped: the temp dir is case-sensitive");
        fs::remove_dir_all(&root).unwrap();
        return;
    }
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("Target");
    let edit = server.request(2, "textDocument/rename", params);
    assert_eq!(
        edits_of(&root, &edit["result"]),
        (
            vec![
                (spot("main.rein", 2, 12, 2, 18), "Target".to_string()),
                (spot("target.rein", 1, 24, 1, 30), "Target".to_string()),
            ],
            vec![("target.rein".to_string(), "Target.rein".to_string())]
        ),
        "{edit}"
    );

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn rename_refuses_a_case_only_rename_of_a_function_with_a_folder() {
    // Decided from the names and the folder on disk, so this runs on any file system.
    let root = folder_project("rename-case-folder");
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("A");
    let edit = server.request(2, "textDocument/rename", params);
    assert_eq!(
        edit["error"]["message"],
        "a rename that only changes case cannot move the folder `a/`; rename it in two steps through another name",
        "{edit}"
    );

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn rename_refuses_a_destination_that_is_a_hard_link_of_the_source() {
    let root = fs::canonicalize(temp_project("rename-hard-link")).unwrap();
    fs::write(root.join("a.rein"), "/** A. */\nexport default function a() {\n  return `a`;\n}\n").unwrap();
    fs::write(root.join("main.rein"), "/** Entry. */\nexport default function main() {\n  return @./a;\n}\n").unwrap();
    fs::hard_link(root.join("a.rein"), root.join("c.rein")).unwrap();
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("c");
    let edit = server.request(2, "textDocument/rename", params);
    let message = edit["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("already exists"), "{edit}");
    // Nothing moved: both names still exist.
    assert!(root.join("a.rein").exists() && root.join("c.rein").exists());

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn rename_refuses_a_destination_that_is_a_symlink_to_the_source() {
    let root = fs::canonicalize(temp_project("rename-symlink-dest")).unwrap();
    fs::write(root.join("a.rein"), "/** A. */\nexport default function a() {\n  return `a`;\n}\n").unwrap();
    fs::write(root.join("main.rein"), "/** Entry. */\nexport default function main() {\n  return @./a;\n}\n").unwrap();
    std::os::unix::fs::symlink(root.join("a.rein"), root.join("c.rein")).unwrap();
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("c");
    let edit = server.request(2, "textDocument/rename", params);
    let message = edit["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("already exists"), "{edit}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn rename_moves_the_editor_leftovers_of_the_folder_with_it() {
    let root = folder_project("rename-leftovers");
    fs::write(root.join("a/.DS_Store"), "").unwrap();
    fs::write(root.join("a/Thumbs.db"), "").unwrap();
    fs::write(root.join("a/b.rein~"), "").unwrap();
    let mut server = start_with_file_renames();
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 2, 12);
    params["newName"] = json!("c");
    let edit = server.request(2, "textDocument/rename", params);
    assert_eq!(
        edits_of(&root, &edit["result"]),
        (
            vec![
                (spot("a.rein", 1, 24, 1, 25), "c".to_string()),
                (spot("a.rein", 2, 9, 2, 15), "@./c/b".to_string()),
                (spot("main.rein", 2, 12, 2, 13), "c".to_string()),
            ],
            vec![
                ("a.rein".to_string(), "c.rein".to_string()),
                ("a/b.rein".to_string(), "c/b.rein".to_string()),
                ("a/.DS_Store".to_string(), "c/.DS_Store".to_string()),
                ("a/Thumbs.db".to_string(), "c/Thumbs.db".to_string()),
                ("a/b.rein~".to_string(), "c/b.rein~".to_string()),
            ]
        ),
        "{edit}"
    );

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn rename_a_function_needs_a_client_that_renames_files() {
    let root = example("support-ticket");
    let mut server = Server::start();
    initialize(&mut server);
    let main = open_file(&mut server, &root, "main.rein");

    let mut params = at(&main, 12, 15);
    params["newName"] = json!("handoff");
    let failed = server.request(2, "textDocument/rename", params);
    assert_eq!(failed["error"]["message"], "this editor cannot rename files, so a function cannot be renamed", "{failed}");

    server.shutdown();
}

/// The labels of a completion list.
fn labels(items: &Value) -> Vec<String> {
    items["result"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap().to_string()).collect()
}

/// A buffer in support-ticket that calls the template with one argument given. Line 2, column 46 is after `, `.
const PARTIAL_CALL: &str =
    "/** R. */\nexport default function r() {\n  return @./refund/tell_customer(opening=`x`, );\n}\n";

#[test]
fn completion_offers_argument_names_and_union_members() {
    let root = example("support-ticket");
    let mut server = Server::start();
    initialize(&mut server);

    // Inside the call, after one named argument: the parameter not given yet.
    let partial = uri_of(&root, "partial.rein");
    server.open(&partial, PARTIAL_CALL);
    let items = server.request(2, "textDocument/completion", at(&partial, 2, 46));
    assert_eq!(labels(&items), ["next_step="], "{items}");
    assert_eq!(items["result"][0]["detail"], "next_step: string");

    // After `case "`: the members of the switch subject. Each replaces the whole literal.
    let main = open_file(&mut server, &root, "main.rein");
    let items = server.request(3, "textDocument/completion", at(&main, 16, 10));
    assert_eq!(labels(&items), ["\"billing\"", "\"account\"", "\"how_to\""], "{items}");
    let range = json!({"start": {"line": 16, "character": 9}, "end": {"line": 16, "character": 18}});
    assert_eq!(items["result"][0]["textEdit"]["range"], range);

    // After `c === "`: the members of `c`.
    let tmp = temp_project("options");
    let start = uri_of(&tmp, "start.rein");
    server.open(
        &start,
        "/** Start. */\nexport default function start(\n  /** C */ c: \"x\" | \"y\",\n) {\n  if (c === \"x\") {\n    return `a`;\n  }\n  return `z`;\n}\n",
    );
    let items = server.request(4, "textDocument/completion", at(&start, 4, 13));
    assert_eq!(labels(&items), ["\"x\"", "\"y\""], "{items}");
    let range = json!({"start": {"line": 4, "character": 12}, "end": {"line": 4, "character": 15}});
    assert_eq!(items["result"][0]["textEdit"]["range"], range);

    server.shutdown();
    fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn double_equals_has_no_diagnostic_and_completes_members() {
    let tmp = temp_project("double-equals");
    let mut server = Server::start();
    initialize(&mut server);

    // `==` is `===`: no diagnostic, and completion after `c == "` lists the members of `c`.
    let start = uri_of(&tmp, "start.rein");
    server.open(
        &start,
        "/** Start. */\nexport default function start(\n  /** C */ c: \"x\" | \"y\",\n) {\n  if (c == \"x\") {\n    return `a`;\n  }\n  return `z`;\n}\n",
    );
    // The server publishes only files with problems. Messages sent before the response are the ones it sent for the open.
    server.send(json!({"jsonrpc": "2.0", "id": 2, "method": "textDocument/completion", "params": at(&start, 4, 12)}));
    let mut before = Vec::new();
    let items = loop {
        let msg = server.messages.recv_timeout(TIMEOUT).expect("completion response");
        if msg["id"] == 2 {
            break msg;
        }
        before.push(msg);
    };
    let problems: Vec<&Value> = before
        .iter()
        .filter(|m| m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == start.as_str())
        .filter(|m| m["params"]["diagnostics"].as_array().is_some_and(|d| d.iter().any(|x| x["severity"] == 1)))
        .collect();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(labels(&items), ["\"x\"", "\"y\""], "{items}");

    server.shutdown();
    fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn signature_help_shows_the_callee_parameters() {
    let root = example("support-ticket");
    let mut server = Server::start();
    initialize(&mut server);
    let partial = uri_of(&root, "partial.rein");
    server.open(&partial, PARTIAL_CALL);

    let help = server.request(2, "textDocument/signatureHelp", at(&partial, 2, 46));
    let signature = &help["result"]["signatures"][0];
    assert_eq!(signature["label"], "@./refund/tell_customer(opening: string, next_step: string)");
    assert_eq!(signature["parameters"][0]["label"], json!([24, 39]));
    assert_eq!(signature["parameters"][1]["label"], json!([41, 58]));
    assert_eq!(signature["parameters"][1]["documentation"], "The next step the customer can expect.");
    assert_eq!(help["result"]["activeParameter"], 1, "{help}");

    // In the first argument, the first parameter is active.
    let help = server.request(3, "textDocument/signatureHelp", at(&partial, 2, 36));
    assert_eq!(help["result"]["activeParameter"], 0, "{help}");

    server.shutdown();
}

/// The quick fix for the first diagnostic of `uri` that contains `part`: its title and its one edit.
fn quick_fix(server: &mut Server, id: i64, uri: &str, part: &str) -> (String, Value) {
    let diags = server.wait_diagnostics(uri, |d| has_message(d, part));
    let diag = diags.iter().find(|d| d["message"].as_str().unwrap().contains(part)).unwrap().clone();
    let params = json!({"textDocument": {"uri": uri}, "range": diag["range"], "context": {"diagnostics": [diag]}});
    let actions = server.request(id, "textDocument/codeAction", params);
    let action = &actions["result"][0];
    assert_eq!(action["kind"], "quickfix", "{actions}");
    let edits = action["edit"]["changes"][uri].as_array().unwrap();
    assert_eq!(edits.len(), 1, "{actions}");
    (action["title"].as_str().unwrap().to_string(), edits[0].clone())
}

fn insert_at(line: u64, character: u64, text: &str) -> Value {
    let at = json!({"line": line, "character": character});
    json!({"range": {"start": at, "end": at}, "newText": text})
}

#[test]
fn quick_fixes_add_doc_comments_and_name_arguments() {
    let root = temp_project("fixes");
    let mut server = Server::start();
    initialize(&mut server);

    let undocumented = uri_of(&root, "undocumented.rein");
    server.open(&undocumented, "export default function undocumented() {\n  return `x`;\n}\n");
    let (title, edit) = quick_fix(&mut server, 2, &undocumented, "missing doc comment");
    assert_eq!(title, "Add a doc comment");
    assert_eq!(edit, insert_at(0, 0, "/**\n * TODO: say what this function decides.\n */\n"));

    let param = uri_of(&root, "param.rein");
    server.open(
        &param,
        "/** P. */\nexport default function param(\n  c: \"x\" | \"y\",\n) {\n  if (c === \"x\") {\n    return `a`;\n  }\n  return `b`;\n}\n",
    );
    let (title, edit) = quick_fix(&mut server, 3, &param, "parameter `c` needs a doc comment");
    assert_eq!(title, "Add a doc comment to `c`");
    assert_eq!(edit, insert_at(2, 2, "/** TODO: ask the question. */ "));

    let positional = uri_of(&root, "positional.rein");
    server.open(&positional, "/** P. */\nexport default function positional() {\n  return delegate(\"a\", task=`b`);\n}\n");
    let (title, edit) = quick_fix(&mut server, 4, &positional, "missing argument `model`");
    assert_eq!(title, "Name the argument `model=`");
    assert_eq!(edit, insert_at(2, 18, "model="));

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn quick_fix_moves_a_function_into_the_folder_of_its_only_caller() {
    // `temp_project` writes `target.rein`. Here only `steps/fix_ci` calls it, so it is a nested relative reference.
    let root = fs::canonicalize(temp_project("nest")).unwrap();
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("main.rein", "/** Entry. */\nexport default function main() {\n  return @./flow/merge;\n}\n");
    write("flow/merge.rein", "/** Merge. */\nexport default function merge() {\n  return @../steps/fix_ci;\n}\n");
    write("steps/fix_ci.rein", "/** Fix CI. */\nexport default function fixCi() {\n  return @../target;\n}\n");
    let mut server = start_with_file_renames();
    let fix_ci = open_file(&mut server, &root, "steps/fix_ci.rein");

    let part = "`steps/fix_ci` is only called by `flow/merge`; move it to `flow/merge/fix_ci.rein`";
    let diags = server.wait_diagnostics(&fix_ci, |d| has_message(d, part));
    let diag = diags.iter().find(|d| d["message"].as_str().unwrap().contains(part)).unwrap().clone();
    assert_eq!(diag["severity"], 2, "{diag}");
    let params = json!({"textDocument": {"uri": fix_ci}, "range": diag["range"], "context": {"diagnostics": [diag]}});
    let actions = server.request(2, "textDocument/codeAction", params);
    let action = &actions["result"][0];
    assert_eq!(action["title"], "Move to flow/merge/fix_ci.rein", "{actions}");
    assert_eq!(
        edits_of(&root, &action["edit"]),
        (
            vec![
                (spot("flow/merge.rein", 2, 9, 2, 25), "@./merge/fix_ci".to_string()),
                (spot("steps/fix_ci.rein", 2, 9, 2, 19), "@../../target".to_string()),
            ],
            vec![("steps/fix_ci.rein".to_string(), "flow/merge/fix_ci.rein".to_string())]
        ),
        "{actions}"
    );

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn quick_fix_leaves_out_a_name_edit_that_changes_nothing() {
    // `fix_ci` keeps its name when it moves, so writing its function name again changes nothing and is not sent.
    let root = fs::canonicalize(temp_project("nest-same-name")).unwrap();
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("main.rein", "/** Entry. */\nexport default function main() {\n  return @./flow/merge;\n}\n");
    write("flow/merge.rein", "/** Merge. */\nexport default function merge() {\n  return @../steps/fix_ci;\n}\n");
    write("steps/fix_ci.rein", "/** Fix CI. */\nexport default function fix_ci() {\n  return @../target;\n}\n");
    let mut server = start_with_file_renames();
    let fix_ci = open_file(&mut server, &root, "steps/fix_ci.rein");

    let part = "`steps/fix_ci` is only called by `flow/merge`; move it to `flow/merge/fix_ci.rein`";
    let diags = server.wait_diagnostics(&fix_ci, |d| has_message(d, part));
    let diag = diags.iter().find(|d| d["message"].as_str().unwrap().contains(part)).unwrap().clone();
    let params = json!({"textDocument": {"uri": fix_ci}, "range": diag["range"], "context": {"diagnostics": [diag]}});
    let actions = server.request(2, "textDocument/codeAction", params);
    let action = &actions["result"][0];
    assert_eq!(action["title"], "Move to flow/merge/fix_ci.rein", "{actions}");
    assert_eq!(
        edits_of(&root, &action["edit"]),
        (
            vec![
                (spot("flow/merge.rein", 2, 9, 2, 25), "@./merge/fix_ci".to_string()),
                (spot("steps/fix_ci.rein", 2, 9, 2, 19), "@../../target".to_string()),
            ],
            vec![("steps/fix_ci.rein".to_string(), "flow/merge/fix_ci.rein".to_string())]
        ),
        "{actions}"
    );

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn quick_fix_moves_the_folder_of_nested_functions_with_the_file() {
    // `steps/fix_ci/retry` is already in the folder of its only caller. When `steps/fix_ci` moves, it moves too.
    let root = fs::canonicalize(temp_project("nest-folder")).unwrap();
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("main.rein", "/** Entry. */\nexport default function main() {\n  return @./flow/merge;\n}\n");
    write("flow/merge.rein", "/** Merge. */\nexport default function merge() {\n  return @../steps/fix_ci;\n}\n");
    write("steps/fix_ci.rein", "/** Fix CI. */\nexport default function fixCi() {\n  return @./fix_ci/retry;\n}\n");
    write("steps/fix_ci/retry.rein", "/** Retry. */\nexport default function retry() {\n  return @../../target;\n}\n");
    let mut server = start_with_file_renames();
    let fix_ci = open_file(&mut server, &root, "steps/fix_ci.rein");

    let part = "move it to `flow/merge/fix_ci.rein`";
    let diags = server.wait_diagnostics(&fix_ci, |d| has_message(d, part));
    let diag = diags.iter().find(|d| d["message"].as_str().unwrap().contains(part)).unwrap().clone();
    let params = json!({"textDocument": {"uri": fix_ci}, "range": diag["range"], "context": {"diagnostics": [diag]}});
    let actions = server.request(2, "textDocument/codeAction", params);
    let action = &actions["result"][0];
    assert_eq!(action["title"], "Move to flow/merge/fix_ci.rein", "{actions}");
    // The reference from `steps/fix_ci` into its own folder stays as it is.
    assert_eq!(
        edits_of(&root, &action["edit"]),
        (
            vec![
                (spot("flow/merge.rein", 2, 9, 2, 25), "@./merge/fix_ci".to_string()),
                (spot("steps/fix_ci/retry.rein", 2, 9, 2, 22), "@../../../target".to_string()),
            ],
            vec![
                ("steps/fix_ci.rein".to_string(), "flow/merge/fix_ci.rein".to_string()),
                ("steps/fix_ci/retry.rein".to_string(), "flow/merge/fix_ci/retry.rein".to_string()),
            ]
        ),
        "{actions}"
    );

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn structs_hover_define_reference_rename_and_list() {
    // Canonical, like `example()`: the server reports renamed files under the canonical path.
    let root = fs::canonicalize(temp_project("structs")).unwrap();
    let ticket_text = "/** A support ticket. */\nexport default interface Ticket {\n  /** The ticket title. */\n  title: string;\n  /** Is the customer a VIP? */\n  vip: boolean;\n}\n";
    let a_text = "/** A. */\nexport default function a({ title, vip }: @./ticket) {\n  if (vip) {\n    return `VIP: ${title}`;\n  }\n  return `Answer ${title}`;\n}\n";
    let b_text = "/** B. */\nexport default function b({ title, vip }: @./ticket) {\n  if (vip) {\n    return `x`;\n  }\n  return @./a(title=title);\n}\n";
    fs::write(root.join("ticket.struct.rein"), ticket_text).unwrap();
    fs::write(root.join("a.rein"), a_text).unwrap();
    fs::write(root.join("b.rein"), b_text).unwrap();
    let mut server = start_with_file_renames();
    let ticket = uri_of(&root, "ticket.struct.rein");
    let a = uri_of(&root, "a.rein");
    let b = uri_of(&root, "b.rein");
    server.open(&ticket, ticket_text);
    server.open(&a, a_text);
    server.open(&b, b_text);

    // Hover on the struct name in a parameter type: the struct's doc and its fields.
    let hover = server.request(2, "textDocument/hover", at(&a, 1, 45));
    let text = hover["result"]["contents"]["value"].as_str().unwrap();
    for part in ["interface Ticket", "A support ticket.", "title: string", "vip: boolean"] {
        assert!(text.contains(part), "{text}");
    }

    // Definition on the struct name goes to the struct file.
    let def = server.request(3, "textDocument/definition", at(&a, 1, 45));
    assert!(def["result"]["uri"].as_str().unwrap().ends_with("/ticket.struct.rein"), "{def}");
    assert_eq!(def["result"]["range"]["start"]["line"], 1, "{def}");

    // Definition on `title=` in a call goes to the field of the struct.
    let def = server.request(4, "textDocument/definition", at(&b, 5, 15));
    assert!(def["result"]["uri"].as_str().unwrap().ends_with("/ticket.struct.rein"), "{def}");
    assert_eq!(spot_of(&root, &def["result"]), spot("ticket.struct.rein", 3, 2, 3, 7), "{def}");

    // References of the struct name, without the declaration.
    let mut params = at(&ticket, 1, 27);
    params["context"] = json!({"includeDeclaration": false});
    let refs = server.request(5, "textDocument/references", params);
    assert_eq!(spots(&root, &refs["result"]), vec![spot("a.rein", 1, 42, 1, 51), spot("b.rein", 1, 42, 1, 51)], "{refs}");

    // Document symbols: the struct, with its fields as children.
    let symbols = server.request(6, "textDocument/documentSymbol", json!({"textDocument": {"uri": ticket}}));
    let symbols = symbols["result"].as_array().unwrap();
    assert_eq!(symbols.len(), 1, "{symbols:?}");
    assert_eq!(symbols[0]["name"], "Ticket");
    assert_eq!(symbols[0]["kind"], 23);
    let children: Vec<&str> = symbols[0]["children"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(children, ["title", "vip"]);

    // Completion after `@./` in a new buffer lists the struct file and not the functions.
    let c = uri_of(&root, "c.rein");
    server.open(&c, "/** C. */\nexport default function c({ title }: @./");
    let items = server.request(7, "textDocument/completion", at(&c, 1, 40));
    let names = labels(&items);
    assert!(names.contains(&"@./ticket".to_string()) && !names.contains(&"@./a".to_string()), "{names:?}");

    // Rename the struct: its file is renamed, and each reference changes its last path segment.
    let mut params = at(&a, 1, 45);
    params["newName"] = json!("issue");
    let edit = server.request(8, "textDocument/rename", params);
    let issue = |s: Spot| (s, "issue".to_string());
    assert_eq!(
        edits_of(&root, &edit["result"]),
        (
            vec![issue(spot("a.rein", 1, 45, 1, 51)), issue(spot("b.rein", 1, 45, 1, 51))],
            vec![("ticket.struct.rein".to_string(), "issue.struct.rein".to_string())]
        ),
        "{edit}"
    );

    // Rename a field from its declaration: the destructuring, the body uses and the named argument all change.
    let mut params = at(&ticket, 3, 3);
    params["newName"] = json!("subject");
    let edit = server.request(9, "textDocument/rename", params);
    let subject = |s: Spot| (s, "subject".to_string());
    assert_eq!(
        edits_of(&root, &edit["result"]),
        (
            vec![
                subject(spot("a.rein", 1, 28, 1, 33)),
                subject(spot("a.rein", 3, 19, 3, 24)),
                subject(spot("a.rein", 5, 19, 5, 24)),
                subject(spot("b.rein", 1, 28, 1, 33)),
                subject(spot("b.rein", 5, 14, 5, 19)),
                subject(spot("b.rein", 5, 20, 5, 25)),
                subject(spot("ticket.struct.rein", 3, 2, 3, 7)),
            ],
            vec![]
        ),
        "{edit}"
    );

    // Two parameters are reported, and the message says to use a struct.
    let d = uri_of(&root, "d.rein");
    server.open(&d, "/** D. */\nexport default function d(\n  /** A? */ a: boolean,\n  /** B? */ b: boolean,\n) {\n  if (a && b) { return `x` }\n  return `y`\n}\n");
    server.wait_diagnostics(&d, |diags| has_message(diags, "a function takes one parameter; put `a` and `b` in a struct"));

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
}

/// A program installed in the user scope. Its entry takes the PR link and asks whether it is a draft.
const PR_HANDLING: &str = "interface Input {\n  /** The PR link. */\n  pr: string;\n  /** Is it a draft? */\n  draft: boolean;\n}\n/** Handle a PR. */\nexport default function main({ pr, draft }: Input) {\n  if (draft) {\n    return `Skip ${pr}.`;\n  }\n  return @./review(pr=pr);\n}\n";

/// Hands off to the installed program. Line 5 holds the reference: `@` at column 9, `(` at column 26.
const HANDOFF: &str = "/** Hand off. */\nexport default function handoff(\n  /** The PR link. */\n  pr: string,\n) {\n  return @pr-handling/main(pr=pr);\n}\n";

#[test]
fn program_references_resolve_through_the_scopes() {
    let root = temp_project("progref");
    let home = root.with_extension("home");
    let program = home.join("pr-handling");
    fs::create_dir_all(&program).unwrap();
    fs::write(program.join("main.rein"), PR_HANDLING).unwrap();
    fs::write(program.join("review.rein"), "/** Review. */\nexport default function review(\n  /** The PR link. */\n  pr: string,\n) {\n  return `Review ${pr}.`;\n}\n").unwrap();
    fs::write(program.join("rein.json"), r#"{"entry":"main.rein"}"#).unwrap();
    let mut server = Server::start_with(&[("REIN_HOME", &home), ("REIN_SYSTEM_DIRS", &home.join("none"))]);
    initialize(&mut server);
    let handoff = uri_of(&root, "handoff.rein");
    server.open(&handoff, HANDOFF);

    // Definition goes to the entry of the installed program, at its function name.
    let def = server.request(2, "textDocument/definition", at(&handoff, 5, 12));
    assert!(def["result"]["uri"].as_str().unwrap().ends_with("/pr-handling/main.rein"), "{def}");
    assert_eq!(def["result"]["range"]["start"], json!({"line": 7, "character": 24}), "{def}");

    // Hover names the program and its scope, then the function's doc and inputs.
    let hover = server.request(3, "textDocument/hover", at(&handoff, 5, 12));
    let text = hover["result"]["contents"]["value"].as_str().unwrap();
    for part in ["program `pr-handling`", "user scope", "**@pr-handling/main**", "Handle a PR.", "`pr: string`", "`draft: boolean`"] {
        assert!(text.contains(part), "{part} not in {text}");
    }

    // After `@`, the installed programs. After `@pr-handling/`, its functions.
    let items = server.request(4, "textDocument/completion", at(&handoff, 5, 10));
    assert!(labels(&items).contains(&"@pr-handling".to_string()), "{items}");
    let items = server.request(5, "textDocument/completion", at(&handoff, 5, 22));
    assert_eq!(labels(&items), ["@pr-handling/main", "@pr-handling/review"], "{items}");

    // Signature help shows the string inputs the hand-off passes.
    let help = server.request(6, "textDocument/signatureHelp", at(&handoff, 5, 27));
    assert_eq!(help["result"]["signatures"][0]["label"], "@pr-handling/main(pr: string)", "{help}");

    // References to the program function in this program.
    let mut params = at(&handoff, 5, 12);
    params["context"] = json!({"includeDeclaration": false});
    let refs = server.request(7, "textDocument/references", params);
    assert_eq!(spots(&root, &refs["result"]), [spot("handoff.rein", 5, 9, 5, 26)], "{refs}");

    // Rename does not cross programs.
    let mut params = at(&handoff, 5, 12);
    params["newName"] = json!("start");
    let rename = server.request(8, "textDocument/rename", params);
    assert!(rename["error"]["message"].as_str().unwrap().contains("another program"), "{rename}");

    // A program that is not installed is an error, as `rein check` reports it.
    let lost = uri_of(&root, "lost.rein");
    server.open(&lost, "/** Lost. */\nexport default function lost() {\n  return @no-such-program;\n}\n");
    server.wait_diagnostics(&lost, |diags| has_message(diags, "unknown program `no-such-program`"));

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&home).unwrap();
}

/// Installs `pr-handling` in the user scope of `home`, as `program_references_resolve_through_the_scopes` does.
fn install_pr_handling(home: &Path) -> PathBuf {
    let program = home.join("pr-handling");
    fs::create_dir_all(&program).unwrap();
    fs::write(program.join("main.rein"), PR_HANDLING).unwrap();
    fs::write(program.join("review.rein"), "/** Review. */\nexport default function review(\n  /** The PR link. */\n  pr: string,\n) {\n  return `Review ${pr}.`;\n}\n").unwrap();
    fs::write(program.join("rein.json"), r#"{"entry":"main.rein"}"#).unwrap();
    program
}

#[test]
fn a_project_without_program_refs_checks_and_completes_without_programs() {
    let root = temp_project("norefs");
    let home = root.with_extension("home");
    install_pr_handling(&home);
    let mut server = Server::start_with(&[("REIN_HOME", &home), ("REIN_SYSTEM_DIRS", &home.join("none"))]);
    initialize(&mut server);

    // The file references only a local function, so the checker finds nothing wrong.
    let main = uri_of(&root, "main.rein");
    server.open(&main, &fs::read_to_string(root.join("main.rein")).unwrap());
    let diags = server.wait_diagnostics(&main, |d| d.is_empty());
    assert!(diags.is_empty(), "{diags:?}");

    // After `@./` the local functions only. The installed program is not offered.
    let items = server.request(2, "textDocument/completion", at(&main, 2, 12));
    let names = labels(&items);
    assert_eq!(names, ["@./target"], "{items}");

    // After a bare `@` the installed programs are still offered next to the local ones.
    let items = server.request(3, "textDocument/completion", at(&main, 2, 10));
    assert!(labels(&items).contains(&"@pr-handling".to_string()), "{items}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn an_unsaved_buffer_of_an_installed_program_shows_in_the_hand_off() {
    let root = temp_project("overlay");
    let home = root.with_extension("home");
    let program = install_pr_handling(&home);
    let mut server = Server::start_with(&[("REIN_HOME", &home), ("REIN_SYSTEM_DIRS", &home.join("none"))]);
    initialize(&mut server);
    let handoff = uri_of(&root, "handoff.rein");
    server.open(&handoff, HANDOFF);

    // Open in the editor, the installed program's entry has an edit that is not saved.
    let edited = PR_HANDLING.replace("Handle a PR.", "Handle a PR, unsaved.");
    server.open(&uri_of(&program, "main.rein"), &edited);

    // The hand-off's hover shows the buffer's doc comment, not the file on disk.
    let hover = server.request(2, "textDocument/hover", at(&handoff, 5, 12));
    let text = hover["result"]["contents"]["value"].as_str().unwrap();
    assert!(text.contains("Handle a PR, unsaved."), "{text}");
    assert!(text.contains("program `pr-handling`"), "{text}");

    // Completion after `@pr-handling/` still lists the program's functions.
    let items = server.request(3, "textDocument/completion", at(&handoff, 5, 22));
    assert_eq!(labels(&items), ["@pr-handling/main", "@pr-handling/review"], "{items}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&home).unwrap();
}

#[cfg(unix)]
#[test]
fn an_unsaved_buffer_of_a_program_reached_through_a_symlink_shows_in_the_hand_off() {
    let root = temp_project("overlay-link");
    let home = root.with_extension("home");
    let store = root.with_extension("store");
    // pnpm links `node_modules/<name>` to the package folder, so the program's folder is reached through a symlink.
    let program = install_pr_handling(&store);
    fs::create_dir_all(&home).unwrap();
    std::os::unix::fs::symlink(&program, home.join("pr-handling")).unwrap();
    let mut server = Server::start_with(&[("REIN_HOME", &home), ("REIN_SYSTEM_DIRS", &home.join("none"))]);
    initialize(&mut server);
    let handoff = uri_of(&root, "handoff.rein");
    server.open(&handoff, HANDOFF);

    let edited = PR_HANDLING.replace("Handle a PR.", "Handle a PR, unsaved.");
    server.open(&uri_of(&home.join("pr-handling"), "main.rein"), &edited);

    // The hand-off shows the unsaved buffer, not the copy on disk.
    let hover = server.request(2, "textDocument/hover", at(&handoff, 5, 12));
    let text = hover["result"]["contents"]["value"].as_str().unwrap();
    assert!(text.contains("Handle a PR, unsaved."), "{text}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&home).unwrap();
    fs::remove_dir_all(&store).unwrap();
}

#[test]
fn an_unsaved_buffer_the_loader_skips_is_not_a_function_of_the_program() {
    let root = temp_project("overlay-skip");
    let home = root.with_extension("home");
    let program = install_pr_handling(&home);
    // `sub` is a nested flow, as it has its own main.rein. `.cache` is hidden, and `target` is build output.
    fs::create_dir_all(program.join("sub")).unwrap();
    fs::write(program.join("sub/main.rein"), PR_HANDLING).unwrap();
    fs::create_dir_all(program.join(".cache")).unwrap();
    fs::create_dir_all(program.join("target")).unwrap();
    fs::create_dir_all(program.join("helpers")).unwrap();
    let mut server = Server::start_with(&[("REIN_HOME", &home), ("REIN_SYSTEM_DIRS", &home.join("none"))]);
    initialize(&mut server);
    let handoff = uri_of(&root, "handoff.rein");
    server.open(&handoff, HANDOFF);

    let stray = "/** Stray. */\nexport default function stray() {\n  return `stray`;\n}\n";
    for file in ["sub/extra.rein", ".cache/extra.rein", "target/extra.rein"] {
        server.open(&uri_of(&program, file), stray);
    }
    // A folder the loader does load still shows its open buffer as a function.
    server.open(&uri_of(&program, "helpers/extra.rein"), stray);

    // Completion after `@pr-handling/` lists what the loader finds on disk, plus the open buffer that it would load.
    let items = server.request(3, "textDocument/completion", at(&handoff, 5, 22));
    assert_eq!(labels(&items), ["@pr-handling/helpers/extra", "@pr-handling/main", "@pr-handling/review"], "{items}");

    server.shutdown();
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&home).unwrap();
}
