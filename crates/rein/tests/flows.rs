//! Flow scopes, `step --flow`, `agent-context`, the agent hook, and `install` of a package. Every run gets temp
//! HOME, REIN_HOME, REIN_SYSTEM_DIRS and TMPDIR, so no real user or system folder is read or written.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A unique empty temp directory.
fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rein-cli-flows-{tag}-{}-{nanos}-{n}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(base: &Path, rel: &str, text: &str) {
    let p = base.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn flow_text(desc: &str) -> String {
    format!("/** {desc} */\nexport default function main() {{\n  return `ok`;\n}}\n")
}

/// Sandbox: a home with its own user programs folder, an empty system list, and a private TMPDIR.
struct Sandbox {
    home: PathBuf,
    tmp: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let root = temp_dir(tag);
        let home = root.join("home");
        let tmp = root.join("tmp");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&tmp).unwrap();
        Sandbox { home, tmp }
    }

    fn user_flows(&self) -> PathBuf {
        self.home.join(".config/rein/programs")
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rein"));
        cmd.args(args)
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("REIN_HOME", self.user_flows())
            .env("REIN_SYSTEM_DIRS", "")
            .env("TMPDIR", &self.tmp)
            .env_remove("POLYHOOK_CALLER")
            .env_remove("CLAUDE_CODE_VERSION");
        cmd
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().unwrap()
    }

    /// Runs with `stdin` piped in.
    fn run_with_stdin(&self, cwd: &Path, args: &[&str], stdin: &str) -> Output {
        let mut child = self
            .command(cwd, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn json_out(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", stdout(out)))
}

/// A repo with one project flow `merge-pr` (the real example copy is used by other tests).
fn repo_with_flow(sb: &Sandbox, tag: &str) -> PathBuf {
    let repo = temp_dir(tag);
    let _ = sb;
    write(&repo, "rein/merge-pr/main.rein", &flow_text("Decide a merge."));
    repo
}

#[test]
fn flows_json_lists_visible_flows_with_scope_and_shadowing() {
    let sb = Sandbox::new("json");
    let repo = repo_with_flow(&sb, "json-repo");
    write(&sb.user_flows(), "merge-pr/main.rein", &flow_text("Shadowed."));
    write(&sb.user_flows(), "my-review/main.rein", &flow_text("My review."));

    // Shadowed programs are hidden unless `-a` asks for them.
    let visible = json_out(&sb.run(&repo, &["flows", "--json"]));
    assert_eq!(visible.as_array().unwrap().len(), 2, "{visible}");

    let out = sb.run(&repo, &["flows", "--json", "-a"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let list = json_out(&out);
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 3, "{list}");
    let merge_project = rows.iter().find(|r| r["name"] == "merge-pr" && r["scope"] == "project").unwrap();
    assert_eq!(merge_project["description"], "Decide a merge.");
    assert_eq!(merge_project["shadowed"], false);
    let merge_user = rows.iter().find(|r| r["name"] == "merge-pr" && r["scope"] == "user").unwrap();
    assert_eq!(merge_user["shadowed"], true);
    assert_eq!(merge_user["shadowed_by"], "project");
    let mine = rows.iter().find(|r| r["name"] == "my-review").unwrap();
    assert_eq!(mine["scope"], "user");
    assert!(mine["path"].as_str().unwrap().ends_with("my-review"), "{mine}");
}

#[test]
fn flows_text_lists_the_examples_from_their_folder() {
    let sb = Sandbox::new("examples");
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").canonicalize().unwrap();
    let out = sb.run(&examples, &["flows"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    for name in ["merge-pr", "incident-triage", "support-ticket"] {
        assert!(text.lines().any(|l| l == name), "{text}");
    }
    let long = stdout(&sb.run(&examples, &["flows", "-l"]));
    assert!(long.lines().any(|l| l.starts_with("project ") && l.contains(" merge-pr ")), "{long}");
}

#[test]
fn step_with_flow_runs_the_named_flow() {
    let sb = Sandbox::new("stepflow");
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").canonicalize().unwrap();
    let out = sb.run(&examples, &["step", "--flow", "support-ticket", "--json"]);
    assert_eq!(code(&out), 2, "{}", stdout(&out));
    let v = json_out(&out);
    assert_eq!(v["kind"], "question");
    assert_eq!(v["key"], "main.vip");
}

#[test]
fn step_with_unknown_flow_is_an_error_that_names_it() {
    let sb = Sandbox::new("unknownflow");
    let out = sb.run(&sb.home, &["step", "--flow", "nope", "--json"]);
    assert_eq!(code(&out), 1);
    let v = json_out(&out);
    assert_eq!(v["kind"], "error");
    assert!(v["message"].as_str().unwrap().contains("unknown program `nope`"), "{v}");
}

#[test]
fn step_flow_and_root_are_mutually_exclusive() {
    let sb = Sandbox::new("exclusive");
    let out = sb.run(&sb.home, &["step", "--flow", "a", "--root", "b"]);
    assert_eq!(code(&out), 2, "clap rejects the pair");
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot be used with"), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn stdin_request_with_flow_field_steps_that_flow() {
    let sb = Sandbox::new("stdinflow");
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").canonicalize().unwrap();
    let req = json!({"version": 1, "flow": "support-ticket", "answers": {"main.vip": "no"}}).to_string();
    let out = sb.run_with_stdin(&examples, &["step", "--stdin"], &req);
    assert_eq!(code(&out), 2, "{}", stdout(&out));
    let v = json_out(&out);
    assert_eq!(v["key"], "main.category");
}

#[test]
fn agent_context_text_names_flows_and_explains_use() {
    let sb = Sandbox::new("ctxtext");
    let repo = repo_with_flow(&sb, "ctx-repo");
    let out = sb.run(&repo, &["agent-context"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(text.contains("- merge-pr (project): Decide a merge."), "{text}");
    assert!(text.contains("rein step --flow <name> --json"), "{text}");
    assert!(text.contains("kind=done"), "{text}");
    assert!(text.contains("--answer <key>=<value>"), "{text}");
    assert!(text.lines().count() <= 25, "{text}");
}

#[test]
fn agent_context_json_carries_the_flows_and_the_text() {
    let sb = Sandbox::new("ctxjson");
    let repo = repo_with_flow(&sb, "ctxj-repo");
    let out = sb.run(&repo, &["agent-context", "--format", "json"]);
    assert_eq!(code(&out), 0);
    let v = json_out(&out);
    assert_eq!(v["flows"][0]["name"], "merge-pr");
    assert!(v["text"].as_str().unwrap().contains("merge-pr (project)"), "{v}");
}

#[test]
fn agent_context_prints_nothing_without_flows() {
    let sb = Sandbox::new("ctxnone");
    let empty = temp_dir("ctxnone-repo");
    let out = sb.run(&empty, &["agent-context"]);
    assert_eq!(code(&out), 0);
    assert!(out.stdout.is_empty(), "{}", stdout(&out));
}

/// Claude Code's SessionStart payload, as the agent sends it.
fn session_start(session: &str, cwd: &Path) -> String {
    json!({
        "session_id": session,
        "transcript_path": "/tmp/t.jsonl",
        "cwd": cwd,
        "hook_event_name": "SessionStart",
        "source": "startup"
    })
    .to_string()
}

fn prompt_submit(session: &str, cwd: &Path) -> String {
    json!({
        "session_id": session,
        "transcript_path": "/tmp/t.jsonl",
        "cwd": cwd,
        "hook_event_name": "UserPromptSubmit",
        "prompt": "merge the PR"
    })
    .to_string()
}

#[test]
fn hook_session_start_is_a_no_op_and_the_first_prompt_gets_the_context() {
    let sb = Sandbox::new("hookflow");
    let repo = repo_with_flow(&sb, "hook-repo");
    let args = ["hook", "--caller", "claude-code"];

    let start = sb.run_with_stdin(&repo, &args, &session_start("sess-a", &repo));
    assert_eq!(code(&start), 0, "{}", String::from_utf8_lossy(&start.stderr));
    assert!(start.stdout.is_empty(), "session start writes nothing: {}", stdout(&start));

    let first = sb.run_with_stdin(&repo, &args, &prompt_submit("sess-a", &repo));
    assert_eq!(code(&first), 0);
    let v = json_out(&first);
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit", "{v}");
    let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
    assert!(ctx.contains("merge-pr (project): Decide a merge."), "{ctx}");

    let second = sb.run_with_stdin(&repo, &args, &prompt_submit("sess-a", &repo));
    assert_eq!(code(&second), 0);
    assert!(second.stdout.is_empty(), "context is given once per session: {}", stdout(&second));
}

#[test]
fn hook_unrelated_event_is_a_no_op() {
    let sb = Sandbox::new("hookpre");
    let repo = repo_with_flow(&sb, "hookpre-repo");
    let pre = json!({
        "session_id": "s",
        "cwd": repo,
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "ls"}
    })
    .to_string();
    let out = sb.run_with_stdin(&repo, &["hook", "--caller", "claude-code"], &pre);
    assert_eq!(code(&out), 0);
    assert!(out.stdout.is_empty(), "no allow decision for PreToolUse: {}", stdout(&out));
}

#[test]
fn hook_malformed_payload_is_a_no_op_with_stderr() {
    let sb = Sandbox::new("hookbad");
    let out = sb.run_with_stdin(&sb.home, &["hook"], "{not json");
    assert_eq!(code(&out), 0);
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("rein hook:"), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn install_other_name_is_a_package_for_pnpm() {
    // `-g` installs for the user, so it runs in the user folder. A missing pnpm stops it before any network use.
    let sb = Sandbox::new("install-other");
    let repo = temp_dir("install-other-repo");
    let out = sb.command(&repo, &["install", "-g", "cursor"]).env("REIN_PNPM", sb.tmp.join("no-pnpm")).output().unwrap();
    assert_eq!(code(&out), 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("pnpm is not installed"));
}

/// A program with no `rein/` folder, no manifest and no `main.rein`: the file is the program. It refs `@./ci/fix`.
fn write_file_program(base: &Path) -> PathBuf {
    write(
        base,
        "x/review.rein",
        "/**\n * Review a failing change.\n */\nexport default function review(\n  /** Is CI green? */\n  green: boolean,\n) {\n  if (green) {\n    return `approved`;\n  }\n  return @./ci/fix;\n}\n",
    );
    write(
        base,
        "x/ci/fix.rein",
        "/**\n * Fix the failing CI.\n */\nexport default function fix(\n  /** Is the fix pushed? */\n  pushed: boolean,\n) {\n  if (pushed) {\n    return `rerun CI`;\n  }\n  return `push the fix`;\n}\n",
    );
    base.join("x/review.rein")
}

#[test]
fn step_runs_a_program_from_its_entry_file() {
    let sb = Sandbox::new("filestep");
    let file = write_file_program(&sb.home);
    let path = file.to_str().unwrap();
    // Run from a folder with no `rein/`: the file alone decides the project root.
    let out = sb.run(&sb.home, &["step", path, "--json"]);
    assert_eq!(code(&out), 2, "{}", stdout(&out));
    let v = json_out(&out);
    assert_eq!(v["kind"], "question");
    assert_eq!(v["key"], "review.green");

    let out = sb.run(&sb.home, &["step", path, "--answer", "review.green=no", "--json"]);
    assert_eq!(code(&out), 2, "{}", stdout(&out));
    let v = json_out(&out);
    assert_eq!(v["key"], "ci/fix.pushed");
    assert_eq!(v["function"], "ci/fix");

    let out = sb.run(&sb.home, &["step", path, "--answer", "review.green=no", "--answer", "ci/fix.pushed=yes", "--json"]);
    assert_eq!(code(&out), 0, "{}", stdout(&out));
    let v = json_out(&out);
    assert_eq!(v["kind"], "done");
    assert_eq!(v["instruction"], "rerun CI");
    assert_eq!(v["trace"], json!(["review", "ci/fix"]));
}

#[test]
fn run_takes_a_program_file_path() {
    let sb = Sandbox::new("filerun");
    let file = write_file_program(&sb.home);
    let out = sb.run_with_stdin(
        &sb.home,
        &["run", file.to_str().unwrap(), "--json"],
        "{\"value\": \"no\"}\n{\"value\": \"yes\"}\n",
    );
    assert_eq!(code(&out), 0, "{}", stdout(&out));
    let last = stdout(&out).lines().last().unwrap().to_string();
    let v: Value = serde_json::from_str(&last).unwrap();
    assert_eq!(v["kind"], "done", "{}", stdout(&out));
}

#[test]
fn check_takes_a_program_file_path() {
    let sb = Sandbox::new("filecheck");
    let file = write_file_program(&sb.home);
    let out = sb.run(&sb.home, &["check", file.to_str().unwrap()]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn step_program_file_that_does_not_exist_is_a_clear_error() {
    let sb = Sandbox::new("missingfile");
    let out = sb.run(&sb.home, &["step", "nope/review.rein", "--json"]);
    assert_eq!(code(&out), 1);
    let v = json_out(&out);
    assert_eq!(v["kind"], "error");
    assert_eq!(v["message"], "no such file: nope/review.rein", "{v}");
}

#[test]
fn step_program_name_still_resolves_through_the_catalog() {
    let sb = Sandbox::new("namestep");
    write(&sb.user_flows(), "my-gate/main.rein", &flow_text("A user gate."));
    let out = sb.run(&sb.home, &["step", "my-gate", "--json"]);
    assert_eq!(code(&out), 0, "{}", stdout(&out));
    assert_eq!(json_out(&out)["kind"], "done");
}
