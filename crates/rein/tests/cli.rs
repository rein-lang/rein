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
fn run_without_input_asks_for_it() {
    let out = rein(&["run", "merge_pr", "--root", "examples/merge-pr", "--json"]);
    assert_eq!(code(&out), 2);
    let v = json(&out);
    assert_eq!(v["kind"], "need_input");
    assert_eq!(v["input"]["name"], "docs_only");
    assert_eq!(v["input"]["type"], "bool");
}

#[test]
fn run_with_docs_only_returns_approved() {
    let out = rein(&["run", "merge_pr", "--root", "examples/merge-pr", "--set", "docs_only=yes", "--json"]);
    assert_eq!(code(&out), 0);
    assert_eq!(json(&out), serde_json::json!({"kind": "text", "value": "approved"}));
}

#[test]
fn run_red_ci_returns_ref() {
    let out = rein(&[
        "run", "merge_pr", "--root", "examples/merge-pr", "--set", "docs_only=no", "--set", "ci=red", "--json",
    ]);
    assert_eq!(code(&out), 0);
    let v = json(&out);
    assert_eq!(v["kind"], "ref");
    assert_eq!(v["function"], "ci/fix");
    assert_eq!(v["inputs"][0]["name"], "fix_pushed");
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
fn check_reports_errors_in_broken_project() {
    let root = temp_dir("broken");
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::write(root.join("bad.rein"), "description \"x\"\nreturn @missing\n").unwrap();

    let out = rein(&["check", root.to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("error:"), "{stdout}");
    assert!(stdout.contains("unknown function reference `@missing`"), "{stdout}");

    let out = rein(&["check", root.join("bad.rein").to_str().unwrap()]);
    assert_eq!(code(&out), 1);

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn run_refuses_function_with_errors() {
    let root = temp_dir("refuse");
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::write(root.join("bad.rein"), "description \"x\"\nreturn @missing\n").unwrap();

    let out = rein(&["run", "bad", "--root", root.to_str().unwrap(), "--json"]);
    assert_eq!(code(&out), 1);
    assert_eq!(json(&out)["kind"], "error");

    fs::remove_dir_all(&root).unwrap();
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("rein-cli-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}
