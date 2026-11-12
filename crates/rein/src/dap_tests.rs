use super::*;
use serde_json::{json, Value};

/// Frames each request as the client sends it: `Content-Length` header, then the JSON body.
fn framed(requests: &[Value]) -> Vec<u8> {
    let mut buf = Vec::new();
    for request in requests {
        wire::write(&mut buf, request).unwrap();
    }
    buf
}

/// Reads every framed message the server wrote.
fn read_all(bytes: &[u8]) -> Vec<Value> {
    let mut input = io::Cursor::new(bytes);
    let mut out = Vec::new();
    while let Some(message) = wire::read(&mut input).unwrap() {
        out.push(message);
    }
    out
}

fn request(seq: i64, command: &str, args: Value) -> Value {
    json!({"seq": seq, "type": "request", "command": command, "arguments": args})
}

#[test]
fn scripted_session_runs_pr_handling_to_its_instruction() {
    let program = format!(
        "{}/../../examples/rein/pr-handling/main.rein",
        env!("CARGO_MANIFEST_DIR")
    );
    let script = framed(&[
        request(1, "initialize", json!({"adapterID": "rein"})),
        request(
            2,
            "launch",
            json!({"program": program, "inputs": {"pr": "https://example.test/pull/1"}}),
        ),
        request(3, "configurationDone", json!({})),
        request(
            4,
            "rein/answer",
            json!({"key": "main.draft", "value": "no"}),
        ),
        request(
            5,
            "rein/answer",
            json!({"key": "main.task", "value": "merge"}),
        ),
        request(
            6,
            "rein/answer",
            json!({"key": "merge.ci", "value": "green"}),
        ),
        request(
            7,
            "rein/answer",
            json!({"key": "merge.conflicts", "value": "no"}),
        ),
        request(
            8,
            "rein/answer",
            json!({"key": "merge.review", "value": "approved"}),
        ),
        request(9, "disconnect", json!({})),
    ]);
    let mut out = Vec::new();
    assert_eq!(run(io::Cursor::new(script), &mut out), 0);
    let log = read_all(&out);

    let names: Vec<String> = log
        .iter()
        .map(|m| match m["type"].as_str() {
            Some("event") => format!("event {}", m["event"].as_str().unwrap()),
            _ => format!("response {}", m["command"].as_str().unwrap()),
        })
        .collect();
    assert!(names.contains(&"event rein/ask".to_string()), "{names:?}");
    let asks: Vec<&str> = log
        .iter()
        .filter(|m| m["event"] == "rein/ask")
        .map(|m| m["body"]["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        asks,
        [
            "main.draft",
            "main.task",
            "merge.ci",
            "merge.conflicts",
            "merge.review"
        ]
    );

    let output = log.iter().find(|m| m["event"] == "output").unwrap();
    assert!(output["body"]["output"]
        .as_str()
        .unwrap()
        .contains("Merge the pull request"));
    assert_eq!(log.last().unwrap()["command"], "disconnect");
    assert!(log.iter().any(|m| m["event"] == "terminated"));
}
