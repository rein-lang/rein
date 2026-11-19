//! Runs the built `rein migrate imports` on copies of the examples. The migrated project must check the same as before,
//! run the same steps, and a second run must change nothing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

const EXAMPLES: [&str; 5] = [
    "merge-pr",
    "incident-triage",
    "support-ticket",
    "pr-handling",
    "priority-work",
];

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rein")
        .canonicalize()
        .unwrap()
}

fn rein(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rein"))
        .env("REIN_RUNS_DIR", "")
        .args(args)
        .current_dir(examples_root())
        .output()
        .unwrap()
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("rein-migrate-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

/// A fresh copy of the example `name`, in a temp folder that the test removes.
fn copy_example(name: &str, tag: &str) -> PathBuf {
    let dir = temp_dir(tag);
    let root = dir.join(name);
    copy_tree(&examples_root().join(name), &root);
    root
}

/// Every `.rein` file under `dir`, with its text.
fn rein_files(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(rein_files(&path));
        } else if path.extension().is_some_and(|e| e == "rein") {
            out.push((path.clone(), fs::read_to_string(&path).unwrap()));
        }
    }
    out
}

fn migrate(root: &Path, check: bool) -> Output {
    let root = root.to_str().unwrap();
    if check {
        rein(&["migrate", "imports", "--check", root])
    } else {
        rein(&["migrate", "imports", root])
    }
}

/// The last line of `rein check`'s stderr, such as `0 errors, 2 warnings`.
fn check_summary(root: &Path) -> (i32, String) {
    let out = rein(&["check", root.to_str().unwrap()]);
    let summary = text(&out.stderr).lines().last().unwrap_or_default().to_string();
    (code(&out), summary)
}

/// The first answer that a scripted caller gives to a question or request: the first option, else a boolean or text.
fn scripted(options: &Value, kind: Option<&str>) -> String {
    match options.as_array().and_then(|o| o.first()).and_then(Value::as_str) {
        Some(first) => first.to_string(),
        None if kind == Some("boolean") => "false".to_string(),
        None => "x".to_string(),
    }
}

/// Steps the program at `root` until it stops asking, and returns each response as printed. The answers are scripted
/// the same way every time, so two projects that run the same steps give the same lines.
fn trace(root: &Path, inputs: &[&str]) -> Vec<String> {
    let mut answers: Vec<(String, String)> = Vec::new();
    let mut lines = Vec::new();
    for _ in 0..40 {
        let mut args = vec![
            "step".to_string(),
            "--root".into(),
            root.display().to_string(),
            "--json".into(),
        ];
        for input in inputs {
            args.push("--input".into());
            args.push(input.to_string());
        }
        for (key, value) in &answers {
            args.push("--answer".into());
            args.push(format!("{key}={value}"));
        }
        let out = Command::new(env!("CARGO_BIN_EXE_rein"))
            .env("REIN_RUNS_DIR", "")
            .args(&args)
            .output()
            .unwrap();
        let line = text(&out.stdout);
        lines.push(line.trim_end().to_string());
        let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("step output is not JSON ({e}): {line}"));
        let (key, value) = match v["kind"].as_str() {
            Some("question") => (v["key"].as_str().unwrap(), scripted(&v["options"], v["type"].as_str())),
            Some("request") => (
                v["key"].as_str().unwrap(),
                scripted(&v["response"]["options"], v["response"]["type"].as_str()),
            ),
            _ => break,
        };
        if answers.iter().any(|(k, _)| k == key) {
            break;
        }
        answers.push((key.to_string(), value));
    }
    lines
}

#[test]
fn migrate_removes_every_at_reference_and_check_is_unchanged() {
    for name in EXAMPLES {
        let root = copy_example(name, "check");
        let (before_code, before) = check_summary(&root);
        assert_eq!(before_code, 0, "{name} before: {before}");

        let out = migrate(&root, false);
        assert_eq!(code(&out), 0, "{name}: {}", text(&out.stderr));
        let last = text(&out.stdout).lines().last().unwrap_or_default().to_string();
        assert!(last.ends_with("files changed"), "{name}: {last}");

        for (path, body) in rein_files(&root) {
            assert!(!body.contains('@'), "{} still has `@`", path.display());
        }
        let (after_code, after) = check_summary(&root);
        assert_eq!(after_code, 0, "{name} after: {after}");
        assert_eq!(after, before, "{name}: the check summary changed");

        let again = migrate(&root, false);
        assert_eq!(
            text(&again.stdout),
            "0 files changed\n",
            "{name}: second run must change nothing"
        );
        assert_eq!(code(&migrate(&root, true)), 0, "{name}: --check after migrate");
        fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }
}

#[test]
fn check_lists_changed_files_and_exits_1_without_writing() {
    let root = copy_example("support-ticket", "check-flag");
    let before = rein_files(&root);

    let out = migrate(&root, true);
    assert_eq!(code(&out), 1, "--check must exit 1 when files would change");
    let listed = text(&out.stdout);
    assert!(listed.contains("main.rein"), "{listed}");
    assert_eq!(rein_files(&root), before, "--check must not write");

    migrate(&root, false);
    let clean = migrate(&root, true);
    assert_eq!(code(&clean), 0, "--check must exit 0 once nothing would change");
    assert_eq!(text(&clean.stdout), "");
    fs::remove_dir_all(root.parent().unwrap()).unwrap();
}

#[test]
fn migrated_project_runs_the_same_steps() {
    let cases: [(&str, &[&str]); 4] = [
        ("support-ticket", &[]),
        ("merge-pr", &[]),
        ("incident-triage", &[]),
        ("pr-handling", &["pr=https://example.test/pull/1"]),
    ];
    for (name, inputs) in cases {
        let root = copy_example(name, "trace");
        let before = trace(&root, inputs);
        assert!(before.len() >= 2, "{name}: the script must reach a step: {before:?}");

        assert_eq!(code(&migrate(&root, false)), 0, "{name}");
        let after = trace(&root, inputs);
        assert_eq!(after, before, "{name}: the step trace changed");
        fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }
}

#[test]
fn an_unresolved_reference_is_a_warning_with_its_line() {
    let dir = temp_dir("warn");
    let root = dir.join("project");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("main.rein"),
        "export default function main() {\n  return @nosuch/fix;\n}\n",
    )
    .unwrap();

    let out = migrate(&root, false);
    assert_eq!(code(&out), 0);
    assert_eq!(text(&out.stdout), "0 files changed\n");
    let warnings = text(&out.stderr);
    assert!(
        warnings.contains("main.rein:2: warning: `@nosuch/fix` is not migrated"),
        "{warnings}"
    );
    fs::remove_dir_all(dir).unwrap();
}
