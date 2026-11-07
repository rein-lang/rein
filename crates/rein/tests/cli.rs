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
    assert_eq!(v["inputs"][0]["type"], "boolean");
    assert_eq!(v["inputs"][1]["type"], "union");
}

#[test]
fn run_merge_pr_without_input_asks_for_it() {
    let out = rein(&["run", "merge_pr", "--root", "examples/merge-pr", "--json"]);
    assert_eq!(code(&out), 2);
    let v = json(&out);
    assert_eq!(v["kind"], "need_input");
    assert_eq!(v["input"]["name"], "docsOnly");
    assert_eq!(v["input"]["type"], "boolean");
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
    assert_eq!(v["inputs"][1]["type"], "union");
    assert_eq!(v["inputs"][1]["options"], serde_json::json!(["green", "red", "pending"]));
}

#[test]
fn graph_draws_main_as_entry() {
    let out = rein(&["graph", "--root", "examples/merge-pr"]);
    assert_eq!(code(&out), 0);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("[\"main\"])"), "{text}");
    assert!(text.contains("[\"merge_pr\"]"), "{text}");
    assert!(text.contains("[\"ci/fix\"]"), "{text}");
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

#[test]
fn check_missing_path_is_an_error() {
    let root = temp_dir("missing");
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::write(root.join("main.rein"), "/** Entry. */\nexport default function main() {\n  return \"ok\";\n}\n").unwrap();
    let missing = root.join("nothere.rein");

    let out = rein(&["check", missing.to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(&format!("error: no such file or directory: {}", missing.display())), "{stderr}");

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn check_directory_inside_project_reports_only_its_files() {
    let root = temp_dir("subdir");
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::create_dir_all(root.join("ci")).unwrap();
    fs::write(root.join("main.rein"), "/** Entry. */\nexport default function main() {\n  return \"ok\";\n}\n").unwrap();
    fs::write(root.join("ci/fix.rein"), "import gone from \"./gone\";\n/** F. */\nexport default function fix() {\n  return gone;\n}\n").unwrap();
    fs::write(root.join("other.rein"), "import nope from \"./nope\";\n/** O. */\nexport default function other() {\n  return nope;\n}\n").unwrap();

    let out = rein(&["check", root.join("ci").to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ci/fix.rein") && stdout.contains("cannot find module './gone'"), "{stdout}");
    assert!(!stdout.contains("other.rein"), "{stdout}");

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn check_rejects_a_non_rein_file() {
    let root = temp_dir("notrein");
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::write(root.join("notes.txt"), "hello").unwrap();

    let out = rein(&["check", root.join("notes.txt").to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("error: not a .rein file"));

    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn check_symlinked_file_is_checked_in_the_project_of_the_link() {
    let a = temp_dir("link-a");
    let b = temp_dir("link-b");
    fs::write(a.join("rein.toml"), "").unwrap();
    fs::write(a.join("main.rein"), "/** Entry. */\nexport default function main() {\n  return \"ok\";\n}\n").unwrap();
    fs::write(b.join("rein.toml"), "").unwrap();
    fs::write(b.join("real.rein"), "/** Real. */\nexport default function real() {\n  return \"x\";\n}\n").unwrap();
    std::os::unix::fs::symlink(b.join("real.rein"), a.join("link.rein")).unwrap();

    let out = rein(&["check", a.join("link.rein").to_str().unwrap()]);
    assert_eq!(code(&out), 0);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("`link` is not reachable from main"), "{stdout}");
    assert!(!stdout.contains("real"), "{stdout}");

    fs::remove_dir_all(&a).unwrap();
    fs::remove_dir_all(&b).unwrap();
}

#[test]
fn graph_ids_are_unique_and_labels_are_escaped() {
    let root = temp_dir("graph");
    fs::write(root.join("rein.toml"), "").unwrap();
    fs::create_dir_all(root.join("ci")).unwrap();
    fs::write(
        root.join("main.rein"),
        "import a from \"./ci/fix\";\nimport b from \"./ci__fix\";\nimport c from \"./my task\";\n/** Entry. */\nexport default function main() {\n  return a;\n}\n",
    )
    .unwrap();
    fs::write(root.join("ci/fix.rein"), "/** F. */\nexport default function fix() {\n  return \"f\";\n}\n").unwrap();
    fs::write(root.join("ci__fix.rein"), "/** C. */\nexport default function ci__fix() {\n  return \"c\";\n}\n").unwrap();
    fs::write(root.join("my task.rein"), "/** M. */\nexport default function m() {\n  return \"m\";\n}\n").unwrap();

    let out = rein(&["graph", "--root", root.to_str().unwrap()]);
    assert_eq!(code(&out), 0);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    for label in ["[\"ci/fix\"]", "[\"ci__fix\"]", "[\"my task\"]", "([\"main\"])"] {
        assert!(text.contains(label), "{text}");
    }
    let ids: Vec<String> = text
        .lines()
        .filter(|l| l.contains("[\"") && !l.contains("-->"))
        .map(|l| l.trim().split(['[', '(']).next().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 4, "{text}");
    let unique: std::collections::BTreeSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "{text}");
    assert!(ids.iter().all(|id| id.chars().all(|c| c.is_ascii_alphanumeric())), "{text}");

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn set_twice_is_an_error() {
    let out = rein(&["run", "merge_pr", "--root", "examples/merge-pr", "--set", "docsOnly=yes", "--set", "docsOnly=no"]);
    assert_eq!(code(&out), 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("error: input `docsOnly` set twice"));
}

#[test]
fn text_output_uses_typescript_types() {
    let out = rein(&["run", "merge_pr", "--root", "examples/merge-pr"]);
    assert_eq!(code(&out), 2);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).lines().next().unwrap(),
        "need input docsOnly: boolean - Does the PR change only documentation files?"
    );

    let out = rein(&["run", "merge_pr", "--root", "examples/merge-pr", "--set", "docsOnly=no"]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).lines().next().unwrap(),
        "need input ci: \"green\" | \"red\" | \"pending\" - What is the CI status of the PR?"
    );

    let out = rein(&["run", "--root", "examples/merge-pr", "--set", "task=merge"]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "next: merge_pr\n  Decide whether a pull request can be merged.\n  docsOnly: boolean - Does the PR change only documentation files?\n  ci: \"green\" | \"red\" | \"pending\" - What is the CI status of the PR?\n"
    );
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("rein-cli-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}
