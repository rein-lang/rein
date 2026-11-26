//! Helpers shared by the integration tests. A test file that uses one of them has `mod common;`.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, ChildStdout};

use serde_json::{json, Value};

/// A JSON-RPC client over the stdio of `rein serve --mcp`.
pub struct Client {
    pub stdin: ChildStdin,
    pub stdout: BufReader<ChildStdout>,
    pub id: u64,
}

impl Client {
    pub fn notify(&mut self, method: &str) {
        writeln!(self.stdin, "{}", json!({"jsonrpc": "2.0", "method": method})).unwrap();
    }

    pub fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params})
        )
        .unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"))
    }

    /// Calls a tool and returns its text content, parsed as JSON.
    pub fn call(&mut self, name: &str, args: Value) -> Value {
        let reply = self.request("tools/call", json!({"name": name, "arguments": args}));
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no text: {reply}"));
        serde_json::from_str(text).unwrap_or_else(|e| panic!("tool text is not JSON ({e}): {text}"))
    }
}

/// The folder the test sandboxes go in. Rein takes the nearest `rein/` folder above the working directory, and
/// otherwise the nearest `.git` folder; `rein check` and `rein lsp` take the nearest `main.rein` as the project root. So
/// the folder and each of its parents must hold none of them. Tries the cargo target temp folder, then
/// `$HOME/.cache/rein-test` (absolute `HOME` only), then the shared temp folder.
pub fn sandbox_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .map(|h| h.join(".cache/rein-test"));
    [Some(PathBuf::from(env!("CARGO_TARGET_TMPDIR"))), home, Some(std::env::temp_dir())]
        .into_iter()
        .flatten()
        .find(|root| {
            root.ancestors()
                .all(|d| !d.join("rein").is_dir() && !d.join(".git").exists() && !d.join("main.rein").exists())
        })
        .expect(
            "no clean folder: none of the target temp, $HOME/.cache/rein-test or the temp folder is free of `rein/`, \
             `.git` and `main.rein`",
        )
}
