//! A program hands off to another installed program by name. Programs come from a temp REIN_HOME.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn rein(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rein"))
        .env("REIN_RUNS_DIR", "")
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
    program(&home, "picker", "interface Pick {\n  /** The link of the pull request. */\n  pr: string;\n  /** Which task runs. */\n  task: \"review\" | \"merge\";\n}\n/** Picks a PR. */\nexport default async function main() {\n  /** The pull request of the next notification. */\n  const pick: Pick = await request(`Pick the next pull request notification.`);\n  return @pr-handling/main(pr=pick.pr);\n}\n");
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
        .env("REIN_RUNS_DIR", "")
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

/// Installs `a`, which asks one question and hands off to `b`, and `b`, which asks one question and returns
/// `b_call`, and `c`, which is done at once. Returns the REIN_HOME.
fn chain(tag: &str, b_call: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("rein-handoff-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    let ask = |name: &str, call: &str| {
        format!("/** {name}. */\nexport default function main(\n  /** Go on? */\n  go: boolean,\n) {{\n  if (go) {{\n    return {call};\n  }}\n  return `stop`;\n}}\n")
    };
    program(&home, "a", &ask("A", "@b/main"));
    program(&home, "b", &ask("B", b_call));
    program(&home, "c", "/** C. */\nexport default function main() {\n  return `done c`;\n}\n");
    home
}

/// The first step of `a` is an error that names `missing`, and `rein check` and `rein run` report the same.
fn assert_first_step_names(home: &Path, missing: &str) {
    let out = rein(home, &["step", "a", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("\"kind\":\"error\"") && text.contains(missing), "{text}");
    let out = rein(home, &["check", home.join("a").to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains(missing), "{out:?}");
    let out = rein(home, &["run", "a"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains(missing), "{out:?}");
}

#[test]
fn a_chain_of_hand_offs_with_every_program_installed_runs() {
    let home = chain("present", "@c/main");
    let out = rein(&home, &["check", home.join("a").to_str().unwrap()]);
    assert!(out.status.success(), "{out:?}");
    let out = rein(&home, &["step", "a", "--json"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let out = rein(&home, &["step", "a", "--answer", "main.go=true", "--answer", "@b/main.go=true", "--json"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("done c") && text.contains("@c/main"), "{text}");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn first_step_names_a_program_missing_behind_another_program() {
    let home = chain("transitive", "@d/main");
    assert_first_step_names(&home, "`@b/main` hands off to `@d/main`: unknown program `d`; run `rein list` to list programs, or `rein install -g d` to install it");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn first_step_names_a_function_missing_in_an_installed_program() {
    let home = chain("nofn", "@c/other");
    assert_first_step_names(&home, "`@b/main` hands off to `@c/other`: cannot find function `other` in program `c`");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn first_step_names_a_program_missing_from_the_entry() {
    let home = chain("direct", "@c/main");
    fs::remove_dir_all(home.join("b")).unwrap();
    assert_first_step_names(&home, "unknown program `b`; run `rein list` to list programs, or `rein install -g b` to install it");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_package_reaches_only_the_programs_its_package_json_declares() {
    let home = sandbox("deps", "@pr-handling/main(pr=pr)");
    let handler = home.join("handler");
    fs::write(handler.join("package.json"), r#"{"name": "handler"}"#).unwrap();
    let out = rein(&home, &["check", handler.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("program `pr-handling` is not a dependency of `handler`"), "{text}");

    let out = rein(&home, &["step", "handler", "--input", "pr=u/1", "--json"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("is not a dependency of `handler`"), "{text}");

    fs::write(handler.join("package.json"), r#"{"name": "handler", "dependencies": {"pr-handling": "1"}}"#).unwrap();
    let out = rein(&home, &["check", handler.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
}
