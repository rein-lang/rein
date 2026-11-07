//! Runs the built `rein` binary against the examples and a broken temp project.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Repository root: the examples live here, and commands run from here.
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn rein(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rein"))
        .args(args)
        .current_dir(repo())
        .output()
        .unwrap()
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap()
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)))
}

#[test]
fn check_examples_have_no_errors() {
    for example in ["examples/merge-pr", "examples/incident-triage", "examples/support-ticket"] {
        let out = rein(&["check", example]);
        assert_eq!(code(&out), 0, "{example}: {}", String::from_utf8_lossy(&out.stdout));
    }
}

#[test]
fn run_without_function_runs_main() {
    let out = rein(&["run", "--root", "examples/merge-pr", "--json"]);
    assert_eq!(code(&out), 2);
    let v = json(&out);
    assert_eq!(v["kind"], "need_input");
    assert_eq!(v["input"]["name"], "task");
    assert_eq!(v["input"]["options"], serde_json::json!(["merge", "security review"]));
}

#[test]
fn run_merge_task_returns_ref_to_merge_pr() {
    let out = rein(&["run", "--root", "examples/merge-pr", "--set", "task=merge", "--json"]);
    assert_eq!(code(&out), 0);
    let v = json(&out);
    assert_eq!(v["kind"], "ref");
    assert_eq!(v["function"], "merge_pr");
    assert_eq!(v["description"], "Decide whether a pull request can be merged.");
    assert_eq!(v["inputs"][0]["name"], "docsOnly");
    assert_eq!(v["inputs"][0]["description"], "Does the PR change only documentation files?");
}

#[test]
fn run_merge_pr_without_input_asks_for_it() {
    let out = rein(&["run", "merge_pr", "--root", "examples/merge-pr", "--json"]);
    assert_eq!(code(&out), 2);
    let v = json(&out);
    assert_eq!(v["kind"], "need_input");
    assert_eq!(v["input"]["name"], "docsOnly");
    assert_eq!(v["input"]["type"], "bool");
}

#[test]
fn run_with_docs_only_returns_approved() {
    let out = rein(&["run", "merge_pr", "--root", "examples/merge-pr", "--set", "docsOnly=yes", "--json"]);
    assert_eq!(code(&out), 0);
    assert_eq!(json(&out), serde_json::json!({"kind": "text", "value": "approved"}));
}

#[test]
fn run_red_ci_returns_ref_to_the_project_path() {
    let out = rein(&[
        "run", "merge_pr", "--root", "examples/merge-pr", "--set", "docsOnly=no", "--set", "ci=red", "--json",
    ]);
    assert_eq!(code(&out), 0);
    let v = json(&out);
    assert_eq!(v["kind"], "ref");
    assert_eq!(v["function"], "ci/fix");
    assert_eq!(v["inputs"][0]["name"], "fixPushed");
}

#[test]
fn describe_defaults_to_main() {
    let out = rein(&["describe", "--root", "examples/merge-pr"]);
    assert_eq!(code(&out), 0);
    let v = json(&out);
    assert_eq!(v["name"], "main");
    assert_eq!(v["inputs"][0]["name"], "task");
}

#[test]
fn describe_prints_schema() {
    let out = rein(&["describe", "merge_pr", "--root", "examples/merge-pr"]);
    assert_eq!(code(&out), 0);
    let v = json(&out);
    assert_eq!(v["name"], "merge_pr");
    assert_eq!(v["inputs"].as_array().unwrap().len(), 2);
    assert_eq!(v["inputs"][1]["options"], serde_json::json!(["green", "red", "pending"]));
}

#[test]
fn graph_draws_main_as_entry() {
    let out = rein(&["graph", "--root", "examples/merge-pr"]);
    assert_eq!(code(&out), 0);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("main([\"main\"])"), "{text}");
    assert!(text.contains("main --> merge_pr"), "{text}");
    assert!(text.contains("merge_pr --> ci__fix"), "{text}");
}

#[test]
fn check_reports_errors_in_broken_project() {
    let root = temp_dir("broken");
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::write(root.join("main.rein"), "import bad from \"./bad\";\n/** Entry. */\nexport default function main() {\n  return bad;\n}\n").unwrap();
    fs::write(root.join("bad.rein"), "import gone from \"./gone\";\n/** x */\nexport default function bad() {\n  return gone;\n}\n").unwrap();

    let out = rein(&["check", root.to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("error: cannot find module './gone'"), "{stdout}");

    let out = rein(&["check", root.join("bad.rein").to_str().unwrap()]);
    assert_eq!(code(&out), 1);

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn syntax_error_is_reported_with_its_position() {
    let root = temp_dir("syntax");
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::write(root.join("main.rein"), "/** Entry. */\nexport default function main() {\n  for (;;) { }\n}\n").unwrap();

    let out = rein(&["check", root.to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("main.rein:3:3: error: Rein has no loops yet"), "{stdout}");

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn run_refuses_function_with_errors() {
    let root = temp_dir("refuse");
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::write(root.join("main.rein"), "/** Entry. */\nexport default function main() {\n  return \"ok\";\n}\n").unwrap();
    fs::write(root.join("bad.rein"), "import gone from \"./gone\";\n/** x */\nexport default function bad() {\n  return gone;\n}\n").unwrap();

    let out = rein(&["run", "bad", "--root", root.to_str().unwrap(), "--json"]);
    assert_eq!(code(&out), 1);
    assert_eq!(json(&out)["kind"], "error");

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn missing_main_is_a_project_error() {
    let root = temp_dir("nomain");
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::write(root.join("other.rein"), "/** Other. */\nexport default function other() {\n  return \"x\";\n}\n").unwrap();

    let out = rein(&["check", root.to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let marker = root.join("rein.toml").display().to_string();
    let expected = format!("{marker}:1:1: error: project has no main.rein; it is the agent's entry point");
    assert!(stdout.contains(&expected) || stdout.contains("rein.toml:1:1: error: project has no main.rein"), "{stdout}");

    let out = rein(&["run", "--root", root.to_str().unwrap(), "--json"]);
    assert_eq!(code(&out), 1);
    assert_eq!(json(&out)["message"], "project has no main.rein; it is the agent's entry point");

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn unreachable_function_is_a_warning() {
    let root = temp_dir("orphan");
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::write(root.join("main.rein"), "/** Entry. */\nexport default function main() {\n  return \"ok\";\n}\n").unwrap();
    fs::write(root.join("orphan.rein"), "/** Orphan. */\nexport default function orphan() {\n  return \"x\";\n}\n").unwrap();

    let out = rein(&["check", root.to_str().unwrap()]);
    assert_eq!(code(&out), 0);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("warning: `orphan` is not reachable from main"), "{stdout}");

    fs::remove_dir_all(&root).unwrap();
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("rein-cli-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}
