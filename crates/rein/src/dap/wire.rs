//! Framing and message shapes of the Debug Adapter Protocol: a `Content-Length` header block, then one JSON body.

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

/// Reads one framed message. `Ok(None)` at end of input.
pub fn read(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let Some(length) = length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing Content-Length header",
        ));
    };
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Writes one framed message and flushes.
pub fn write(out: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = message.to_string();
    write!(out, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    out.flush()
}

/// Numbers outgoing messages. Each response and event takes the next `seq`.
#[derive(Default)]
pub struct Sender {
    seq: i64,
}

impl Sender {
    /// A successful response to `request`, with `body`.
    pub fn ok(&mut self, request: &Value, body: Value) -> Value {
        self.response(request, true, None, body)
    }

    /// A failed response to `request`, with `message` as the reason.
    pub fn fail(&mut self, request: &Value, message: impl Into<String>) -> Value {
        self.response(request, false, Some(message.into()), json!({}))
    }

    /// An event with its `body`.
    pub fn event(&mut self, name: &str, body: Value) -> Value {
        self.seq += 1;
        json!({"seq": self.seq, "type": "event", "event": name, "body": body})
    }

    fn response(
        &mut self,
        request: &Value,
        success: bool,
        message: Option<String>,
        body: Value,
    ) -> Value {
        self.seq += 1;
        let mut out = json!({
            "seq": self.seq,
            "type": "response",
            "request_seq": request.get("seq").cloned().unwrap_or(json!(0)),
            "success": success,
            "command": request.get("command").cloned().unwrap_or(json!("")),
            "body": body,
        });
        if let Some(message) = message {
            out["message"] = json!(message);
        }
        out
    }
}

/// The `command` of a request, or `""` when the message has none.
pub fn command(request: &Value) -> &str {
    request.get("command").and_then(Value::as_str).unwrap_or("")
}

/// The `arguments` of a request, or an empty object.
pub fn arguments(request: &Value) -> Value {
    request
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}))
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
