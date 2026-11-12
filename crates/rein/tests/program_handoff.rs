//! A program hands off to another installed program by name. Programs come from a temp REIN_HOME.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn rein(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rein"))
        .args(args)
        .env("REIN_HOME", home)
        .env("REIN_SYSTEM_DIRS", home.join("none"))
        .output()
        .unwrap()
}

fn program(home: &Path, name: &str, main: &str) {
    let dir = home.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("main.rein"), main).unwrap();
    fs::write(dir.join("rein.json"), r#"{"entry":"main.rein"}"#).unwrap();
}

const TARGET: &str = "interface Input {\n  /** The PR link. */\n  pr: string;\n  /** Is it a draft? */\n  draft: boolean;\n}\n/** Handle a PR. */\nexport default function main({ pr, draft }: Input) {\n  if (draft) {\n    return `Skip ${pr}.`;\n  }\n  return `Handle ${pr}.`;\n}\n";

fn caller(call: &str) -> String {
    format!("/** Handler. */\nexport default function main(\n  /** The PR link. */\n  pr: string,\n) {{\n  return {call};\n}}\n")
}

fn sandbox(tag: &str, call: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("rein-handoff-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    program(&home, "pr-handling", TARGET);
    program(&home, "handler", &caller(call));
    home
}

#[test]
fn check_step_and_graph_follow_a_program_hand_off() {
    let home = sandbox("ok", "@pr-handling/main(pr=pr)");
    let handler = home.join("handler");
    let out = rein(&home, &["check", handler.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let out = rein(&home, &["step", "handler", "--input", "pr=u/1", "--json"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("@pr-handling/main.draft"), "{text}");

    let out = rein(&home, &["step", "handler", "--input", "pr=u/1", "--answer", "@pr-handling/main.draft=false", "--json"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Handle u/1."), "{text}");

    let out = rein(&home, &["graph", "handler", "--format", "json"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("\"kind\": \"program\"") && text.contains("\"handoff\""), "{text}");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn check_rejects_an_unknown_program() {
    let home = sandbox("unknown", "@pr-handlng/main(pr=pr)");
    let out = rein(&home, &["check", home.join("handler").to_str().unwrap()]);
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("unknown program `pr-handlng`"), "{text}");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn step_carries_a_struct_answer_field_into_a_program_hand_off() {
    let home = std::env::temp_dir().join(format!("rein-handoff-struct-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    program(&home, "pr-handling", TARGET);
    program(&home, "picker", "interface Pick {\n  /** The link of the pull request. */\n  pr: string;\n  /** Which task runs. */\n  task: \"review\" | \"merge\";\n}\n/** Picks a PR. */\nexport default async function main() {\n  /** The pull request of the next notification. */\n  const pick: Pick = await request(\"Pick the next pull request notification.\");\n  return @pr-handling/main(pr=pick.pr);\n}\n");
    let out = rein(&home, &["check", home.join("picker").to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));

    let out = rein(&home, &["step", "picker", "--json"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("\"type\":\"struct\"") && text.contains("\"fields\"") && text.contains("main.pick"), "{text}");

    let out = rein(&home, &["step", "picker", "--answer", "main.pick={\"pr\":\"\",\"task\":\"merge\"}", "--json"]);
    assert!(!out.status.success());
    let out = rein(&home, &["step", "picker", "--answer", "main.pick={\"pr\":\"https://x/pr/1 (draft)\",\"task\":\"merge\"}", "--json"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("@pr-handling/main.draft"), "{text}");

    let out = rein(&home, &["step", "picker", "--answer", "main.pick={\"pr\":\"https://x/pr/1\",\"task\":\"merge\"}", "--answer", "@pr-handling/main.draft=false", "--json"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Handle https://x/pr/1."), "{text}");

    // On stdin the answer is a real JSON object.
    let request = r#"{"entry":"main","answers":{"main.pick":{"pr":"https://x/pr/2","task":"review"},"@pr-handling/main.draft":"false"}}"#;
    let mut child = Command::new(env!("CARGO_BIN_EXE_rein"))
        .args(["step", "picker", "--stdin"])
        .env("REIN_HOME", &home)
        .env("REIN_SYSTEM_DIRS", home.join("none"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(request.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Handle https://x/pr/2.") && text.contains("\"main.pick\":{\"pr\""), "{text}");
    fs::remove_dir_all(&home).unwrap();
}
