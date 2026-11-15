//! Helpers shared by the integration tests. A test file that uses one of them has `mod common;`.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
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
