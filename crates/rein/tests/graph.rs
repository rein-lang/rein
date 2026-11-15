//! `rein graph` against the example programs: snapshots per program and format, DOT through Graphviz,
//! output files, and errors. Commands run from `examples/`, with no user or system programs.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

const EXAMPLES: [&str; 5] = ["incident-triage", "merge-pr", "pr-handling", "priority-work", "support-ticket"];

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").canonicalize().unwrap()
}

fn command(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rein"));
    c.env("REIN_RUNS_DIR", "");
    c.args(args)
        .current_dir(examples())
        .env("REIN_HOME", examples().join("no-user-programs"))
        .env("REIN_SYSTEM_DIRS", "");
    c
}

fn rein(args: &[&str]) -> Output {
    command(args).output().unwrap()
}

fn stdout(out: &Output) -> String {
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("rein-graph-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn mermaid_snapshot_per_example() {
    for name in EXAMPLES {
        insta::assert_snapshot!(format!("{name}.mmd"), stdout(&rein(&["graph", name])));
    }
}

#[test]
fn json_snapshot_per_example() {
    for name in EXAMPLES {
        let text = stdout(&rein(&["graph", name, "--format", "json"]));
        let v: Value = serde_json::from_str(&text).unwrap();
        let ids: BTreeSet<&str> = v["nodes"].as_array().unwrap().iter().map(|n| n["id"].as_str().unwrap()).collect();
        assert_eq!(ids.len(), v["nodes"].as_array().unwrap().len(), "{name}: ids are unique");
        for e in v["edges"].as_array().unwrap() {
            assert!(ids.contains(e["from"].as_str().unwrap()) && ids.contains(e["to"].as_str().unwrap()), "{name}: {e}");
        }
        insta::assert_snapshot!(format!("{name}.json"), text);
    }
}

#[test]
fn refs_view_is_one_node_per_function() {
    insta::assert_snapshot!("merge-pr.refs.mmd", stdout(&rein(&["graph", "merge-pr", "--refs"])));
}

#[test]
fn root_flag_matches_the_program_name() {
    let by_name = stdout(&rein(&["graph", "merge-pr"]));
    let by_root = stdout(&rein(&["graph", "--root", "rein/merge-pr"]));
    assert_eq!(by_name, by_root);
}

#[test]
fn dot_renders_with_graphviz() {
    if Command::new("dot").arg("-V").output().is_err() {
        eprintln!("skipped: Graphviz `dot` is not on PATH");
        return;
    }
    for name in EXAMPLES {
        let source = stdout(&rein(&["graph", name, "--format", "dot"]));
        let mut child = Command::new("dot").arg("-Tsvg").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(source.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("<svg"), "{name}");
    }
}

#[test]
fn svg_output_without_dot_names_the_tool() {
    let dir = temp_dir("nodot");
    let file = dir.join("g.svg");
    let out = command(&["graph", "merge-pr", "-o", file.to_str().unwrap()]).env("PATH", &dir).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Graphviz") && err.contains("`dot` is not on PATH"), "{err}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn output_file_gets_the_chosen_format() {
    let dir = temp_dir("out");
    let file = dir.join("g.json");
    let out = rein(&["graph", "merge-pr", "--format", "json", "-o", file.to_str().unwrap()]);
    assert_eq!(stdout(&out), "");
    let v: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(v["view"], "flow");
    assert_eq!(v["entry"], "main");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unknown_program_is_an_error() {
    let out = rein(&["graph", "no-such-program"]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown program `no-such-program`"), "{err}");
}

#[test]
fn ids_are_unique_and_labels_are_escaped() {
    let root = temp_dir("ids");
    fs::create_dir_all(root.join("ci")).unwrap();
    fs::write(root.join("main.rein"), "/** Entry. */\nexport default function main() {\n  return @./ci/fix;\n}\n").unwrap();
    fs::write(root.join("ci/fix.rein"), "/** F. */\nexport default function fix() {\n  return \"say \\\"f\\\"\";\n}\n").unwrap();
    fs::write(root.join("ci__fix.rein"), "/** C. */\nexport default function ci__fix() {\n  return `c`;\n}\n").unwrap();
    fs::write(root.join("my task.rein"), "/** M. */\nexport default function m() {\n  return `m`;\n}\n").unwrap();

    let text = stdout(&rein(&["graph", "--refs", "--root", root.to_str().unwrap()]));
    for label in ["f0[[\"ci/fix\"]]", "f1[[\"ci__fix\"]]", "f2([\"main\"])", "f3[[\"my task\"]]", "f2 --> f0"] {
        assert!(text.contains(label), "{text}");
    }
    let flow = stdout(&rein(&["graph", "--root", root.to_str().unwrap()]));
    assert!(flow.contains("f0_0(\"say #quot;f#quot;\")"), "{flow}");

    fs::remove_dir_all(&root).unwrap();
}

const PR_WALK: [&str; 6] = ["--input", "pr=https://x/pr/1", "--answer", "main.draft=no", "--answer", "main.task=merge"];

/// The Mermaid text of a walk, and a JSON summary of it: `path`, `current` and the edges taken.
fn walk_view(args: &[&str]) -> (String, String) {
    let mmd = stdout(&rein(args));
    let json = stdout(&rein(&[args, &["--format", "json"]].concat()));
    let v: Value = serde_json::from_str(&json).unwrap();
    let taken: Vec<String> = v["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["taken"] == true)
        .map(|e| format!("{} -> {}", e["from"].as_str().unwrap(), e["to"].as_str().unwrap()))
        .collect();
    let summary = serde_json::json!({"path": v["path"], "current": v["current"], "taken": taken});
    (mmd, serde_json::to_string_pretty(&summary).unwrap())
}

#[test]
fn walk_stops_at_the_next_question() {
    let (mmd, json) = walk_view(&[&["graph", "pr-handling"][..], &PR_WALK].concat());
    assert!(json.contains("\"key\": \"merge.ci\""), "{json}");
    insta::assert_snapshot!("pr-handling.walk-partial.mmd", mmd);
    insta::assert_snapshot!("pr-handling.walk-partial.json", json);
}

#[test]
fn walk_to_done_marks_the_outcome() {
    let more = ["--answer", "merge.ci=green", "--answer", "merge.conflicts=no", "--answer", "merge.review=approved"];
    let (mmd, json) = walk_view(&[&["graph", "pr-handling"][..], &PR_WALK, &more].concat());
    assert!(json.contains("\"kind\": \"done\""), "{json}");
    insta::assert_snapshot!("pr-handling.walk-done.mmd", mmd);
    insta::assert_snapshot!("pr-handling.walk-done.json", json);
}

#[test]
fn walk_with_an_invalid_answer_fails_like_step() {
    let bad = ["--input", "pr=https://x/pr/1", "--answer", "main.task=ship"];
    let graph = rein(&[&["graph", "pr-handling"][..], &bad].concat());
    let step = rein(&[&["step", "pr-handling"][..], &bad].concat());
    assert_eq!(graph.status.code(), Some(1));
    assert_eq!(graph.status.code(), step.status.code());
    let err = String::from_utf8_lossy(&graph.stderr).to_string();
    assert_eq!(err, String::from_utf8_lossy(&step.stderr));
    insta::assert_snapshot!("pr-handling.walk-invalid.stderr", err);
}

#[test]
fn html_page_has_the_diagram_the_pinned_mermaid_and_the_legend() {
    let dir = temp_dir("html");
    let file = dir.join("flow.html");
    let args = [&["graph", "pr-handling", "-o", file.to_str().unwrap()][..], &PR_WALK].concat();
    assert_eq!(stdout(&rein(&args)), "");
    let page = fs::read_to_string(&file).unwrap();
    let parts = [
        "https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs",
        "<pre class=\"mermaid\">",
        "flowchart TD",
        "classDef taken",
        "class=\"legend\"",
        "not taken",
        "<code>merge.ci</code>",
        "<code>--answer main.task=merge</code>",
        "<code>--input pr=https://x/pr/1</code>",
        "@media (prefers-color-scheme: dark)",
        "theme: dark ? \"dark\" : \"default\"",
    ];
    for part in parts {
        assert!(page.contains(part), "{part}\n{page}");
    }
    fs::remove_dir_all(&dir).unwrap();
}
