//! Runs the built `rein step` against the support-ticket example, as an agent or harness would.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rein/support-ticket")
        .canonicalize()
        .unwrap()
}

/// Runs `rein` with `args`, and `stdin` as its standard input when given.
fn rein(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rein"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut pipe = child.stdin.take().unwrap();
        if let Some(text) = stdin {
            pipe.write_all(text.as_bytes()).unwrap();
        }
    }
    child.wait_with_output().unwrap()
}

/// `rein step --root <example> --stdin --json` with `request` as the JSON request.
fn step_stdin(request: &Value) -> Output {
    let root = example();
    rein(
        &["step", "--root", root.to_str().unwrap(), "--stdin"],
        Some(&request.to_string()),
    )
}

/// `rein step --root <example> <extra>` with no stdin.
fn step_flags(extra: &[&str]) -> Output {
    let root = example();
    let mut args = vec!["step", "--root", root.to_str().unwrap()];
    args.extend_from_slice(extra);
    rein(&args, None)
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap()
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// The value a scripted agent gives for each key the walk asks.
fn answer_for(key: &str) -> &'static str {
    match key {
        "main.vip" => "no",
        "main.category" => "how_to",
        "faq.articleFound" => "yes",
        "faq.solved" => "yes",
        other => panic!("the walk asked an unexpected key `{other}`"),
    }
}

#[test]
fn stdin_walk_reaches_done_and_feeds_back_answers() {
    let mut answers: BTreeMap<String, String> = BTreeMap::new();
    let mut codes = Vec::new();
    let last = loop {
        let out = step_stdin(&json!({ "answers": answers }));
        codes.push(code(&out));
        let v = json(&out);
        assert_eq!(v["version"], 1);
        assert_eq!(
            v["answers"],
            json!(answers),
            "every response echoes the answers it was given"
        );
        match v["kind"].as_str().unwrap() {
            "question" | "request" => {
                let key = v["key"].as_str().unwrap().to_string();
                answers.insert(key.clone(), answer_for(&key).to_string());
            }
            "done" => break v,
            other => panic!("unexpected kind `{other}`: {v}"),
        }
    };
    assert_eq!(codes, [2, 2, 2, 3, 0]);
    assert_eq!(last["instruction"], "Tell the customer: Glad the help article solved the problem. Reply to this ticket if it comes back. Then close the ticket as\nsolved.");
    assert_eq!(last["trace"], json!(["main", "faq"]));
}

#[test]
fn stdin_walk_states_each_key_once() {
    let out = step_stdin(&json!({ "answers": {} }));
    let v = json(&out);
    assert_eq!(v["kind"], "question");
    assert_eq!(v["key"], "main.vip");
    assert_eq!(v["function"], "main");
    assert_eq!(v["type"], "boolean");
    assert_eq!(v["options"], Value::Null);
    assert_eq!(v["trace"], json!(["main"]));
}

#[test]
fn exit_code_is_two_for_a_question() {
    let out = step_flags(&["--json"]);
    assert_eq!(code(&out), 2);
    assert_eq!(json(&out)["kind"], "question");
}

#[test]
fn exit_code_is_zero_for_done() {
    let out = step_flags(&["--json", "--answer", "main.vip=yes"]);
    assert_eq!(code(&out), 0);
    let v = json(&out);
    assert_eq!(v["kind"], "done");
    assert_eq!(v["trace"], json!(["main", "escalate"]));
}

#[test]
fn exit_code_is_three_for_a_request() {
    let out = step_flags(&[
        "--json",
        "--answer",
        "main.vip=no",
        "--answer",
        "main.category=how_to",
        "--answer",
        "faq.articleFound=yes",
    ]);
    assert_eq!(code(&out), 3);
    let v = json(&out);
    assert_eq!(v["kind"], "request");
    assert_eq!(v["key"], "faq.solved");
}

#[test]
fn exit_code_is_one_for_an_error() {
    let out = step_flags(&["--json", "--answer", "main.vip=maybe"]);
    assert_eq!(code(&out), 1);
    let v = json(&out);
    assert_eq!(v["kind"], "error");
    assert_eq!(v["key"], "main.vip");
}

#[test]
fn text_output_shows_the_question_and_how_to_answer_it() {
    let out = step_flags(&[]);
    assert_eq!(code(&out), 2);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("question: main.vip: boolean"), "{text}");
    assert!(text.contains("--answer main.vip=<value>"), "{text}");
    assert!(text.contains("trace: main"), "{text}");
}

#[test]
fn text_output_shows_the_done_instruction_and_trace() {
    let out = step_flags(&["--answer", "main.vip=yes"]);
    assert_eq!(code(&out), 0);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Assign the ticket to the tier 2 queue"),
        "{text}"
    );
    assert!(text.contains("trace: main -> escalate"), "{text}");
}

#[test]
fn answer_flags_give_the_same_result_as_stdin() {
    let by_flags = step_flags(&[
        "--json",
        "--answer",
        "main.vip=no",
        "--answer",
        "main.category=how_to",
        "--answer",
        "faq.articleFound=yes",
        "--answer",
        "faq.solved=yes",
    ]);
    let by_stdin = step_stdin(&json!({
        "answers": {
            "main.vip": "no",
            "main.category": "how_to",
            "faq.articleFound": "yes",
            "faq.solved": "yes",
        }
    }));
    assert_eq!(code(&by_flags), 0);
    assert_eq!(code(&by_stdin), 0);
    assert_eq!(by_flags.stdout, by_stdin.stdout);
}

#[test]
fn stdin_with_bad_json_is_an_error_response() {
    let root = example();
    let out = rein(
        &["step", "--root", root.to_str().unwrap(), "--stdin"],
        Some("{not json"),
    );
    assert_eq!(code(&out), 1);
    let v = json(&out);
    assert_eq!(v["kind"], "error");
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .starts_with("invalid request"),
        "{v}"
    );
}

#[test]
fn stdin_cannot_be_combined_with_answer_flags() {
    let root = example();
    let out = rein(
        &[
            "step",
            "--root",
            root.to_str().unwrap(),
            "--stdin",
            "--answer",
            "main.vip=yes",
        ],
        Some("{}"),
    );
    assert_eq!(code(&out), 1);
    assert_eq!(json(&out)["kind"], "error");
}

#[test]
fn eval_help_points_agents_at_step() {
    let out = rein(&["eval", "--help"], None);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("rein step"), "{text}");
}

#[test]
fn run_json_lines_walks_to_done_and_survives_a_bad_answer() {
    let root = example();
    let mut lines = vec![json!({"value": "maybe"}).to_string()];
    for key in ["main.vip", "main.category", "faq.articleFound", "faq.solved"] {
        lines.push(json!({"key": key, "value": answer_for(key)}).to_string());
    }
    let input = lines.join("\n") + "\n";
    let out = rein(&["run", "--root", root.to_str().unwrap(), "--json"], Some(&input));
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stdout));
    let replies: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let kinds: Vec<&str> = replies.iter().map(|v| v["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["question", "error", "question", "question", "question", "request", "done"]);
    assert_eq!(replies[2]["key"], "main.vip", "the run asks the same key again after a bad answer");
    let last = replies.last().unwrap();
    assert_eq!(last["instruction"], "Tell the customer: Glad the help article solved the problem. Reply to this ticket if it comes back. Then close the ticket as\nsolved.");
    assert_eq!(last["trace"], json!(["main", "faq"]));
}
