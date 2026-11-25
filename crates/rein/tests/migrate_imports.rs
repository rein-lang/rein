//! Runs the built `rein migrate imports` on inline projects with `@` references. The migrated project must check the same
//! as before, run the same steps with the same answers, and a second run must change nothing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

#[path = "fixtures/support_ticket.rs"]
mod support_ticket;
use support_ticket::SUPPORT_TICKET;

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

/// A fresh copy of the support-ticket program with its `@` references, in a temp folder that the test removes.
fn support_ticket_project(tag: &str) -> PathBuf {
    let root = temp_dir(tag).join("support-ticket");
    for (file, text) in SUPPORT_TICKET {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    root
}

/// A project with `@` references, written inline into a fresh temp folder that the test removes. Its root is returned.
fn at_project(tag: &str) -> PathBuf {
    let root = temp_dir(tag).join("project");
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(
        root.join("main.rein"),
        "/** Entry. */\nexport default function main(\n  /** The link. */\n  pr: string,\n) {\n  return @./fix(pr=pr);\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("fix.rein"),
        "/** Fix. */\nexport default function fix(\n  /** The link. */\n  pr: string,\n) {\n  return `fixed ${pr}`;\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("sub/next.rein"),
        "/** Next. */\nexport default function next() {\n  return @../fix(pr=`x`);\n}\n",
    )
    .unwrap();
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
/// The messages of the check, without line numbers. A deprecation is left out: migrating removes it, and the test
/// checks what else the migration changes.
fn check_summary(root: &Path) -> (i32, String) {
    let out = rein(&["check", root.to_str().unwrap()]);
    let messages: Vec<String> = text(&out.stdout)
        .lines()
        .filter(|l| !l.contains("is deprecated and becomes") && !l.contains("default imports are deprecated"))
        .map(|l| l.split_once(": ").map_or(l, |(_, rest)| rest).to_string())
        .collect();
    (code(&out), messages.join("\n"))
}

/// The first answer that a scripted caller gives to a question or request: the first option, else a boolean or text.
fn scripted(options: &Value, kind: Option<&str>) -> String {
    match options.as_array().and_then(|o| o.first()).and_then(Value::as_str) {
        Some(first) => first.to_string(),
        None if kind == Some("boolean") => "false".to_string(),
        None => "x".to_string(),
    }
}

/// Steps the program at `root` until it stops asking. Returns each response as printed, and the keys answered, in
/// order. The `preset` answers are given first. The rest are scripted the same way every time, so two projects that
/// run the same steps give the same lines and keys.
fn trace(root: &Path, inputs: &[&str], preset: &[(&str, &str)]) -> (Vec<String>, Vec<String>) {
    let mut answers: Vec<(String, String)> = preset.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
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
    let keys = answers.into_iter().map(|(key, _)| key).collect();
    (lines, keys)
}

#[test]
fn migrate_removes_every_at_reference_and_check_is_unchanged() {
    let root = at_project("check");
    let (before_code, before) = check_summary(&root);
    assert_eq!(before_code, 0, "before: {before}");

    let out = migrate(&root, false);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let last = text(&out.stdout).lines().last().unwrap_or_default().to_string();
    assert!(last.ends_with("files changed"), "{last}");

    for (path, body) in rein_files(&root) {
        assert!(!body.contains('@'), "{} still has `@`", path.display());
    }
    let (after_code, after) = check_summary(&root);
    assert_eq!(after_code, 0, "after: {after}");
    assert_eq!(after, before, "the check summary changed");

    let again = migrate(&root, false);
    assert_eq!(text(&again.stdout), "0 files changed\n", "second run must change nothing");
    assert_eq!(code(&migrate(&root, true)), 0, "--check after migrate");
    fs::remove_dir_all(root.parent().unwrap()).unwrap();
}

#[test]
fn check_lists_changed_files_and_exits_1_without_writing() {
    let root = at_project("check-flag");
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
    // The `how_to` path reaches the `await request(...)` in faq.rein, then the template call to close_ticket.
    // The default answers take the `billing` path, which asks a refund question first.
    let how_to = [
        ("main.vip", "false"),
        ("main.category", "how_to"),
        ("faq.article_found", "true"),
        ("faq.solved", "true"),
    ];
    let billing: [(&str, &str); 0] = [];
    for (tag, preset) in [("how-to", &how_to[..]), ("billing", &billing[..])] {
        let root = support_ticket_project(tag);
        let (before, before_keys) = trace(&root, &[], preset);
        assert!(
            before.last().is_some_and(|line| line.contains("\"kind\":\"done\"")),
            "{tag}: the script must reach done: {before:#?}"
        );

        assert_eq!(code(&migrate(&root, false)), 0, "{tag}");
        for (path, body) in rein_files(&root) {
            assert!(!body.contains('@'), "{tag}: {} still has `@`", path.display());
        }
        let (after, after_keys) = trace(&root, &[], preset);
        assert_eq!(after, before, "{tag}: the step trace changed");
        assert_eq!(after_keys, before_keys, "{tag}: the answer keys changed");
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
