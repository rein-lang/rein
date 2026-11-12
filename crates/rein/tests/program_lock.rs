//! `rein lock` pins the programs a program hands off to. Programs come from a temp REIN_HOME.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

const TARGET: &str = "/** Handle a PR. */\nexport default function main(\n  /** The PR link. */\n  pr: string,\n) {\n  return `Handle ${pr}.`;\n}\n";

fn caller(call: &str) -> String {
    format!("/** Handler. */\nexport default function main(\n  /** The PR link. */\n  pr: string,\n) {{\n  return {call};\n}}\n")
}

fn sandbox(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("rein-lock-e2e-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    program(&home, "pr-handling", TARGET);
    program(&home, "handler", &caller("@pr-handling/main(pr=pr)"));
    home
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn lock_pins_a_hand_off_and_check_follows_it() {
    let home = sandbox("flow");
    let handler = home.join("handler");
    let dir = handler.to_str().unwrap();

    let out = rein(&home, &["check", dir]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("warning: no rein.lock"), "{}", text(&out.stdout));

    let out = rein(&home, &["lock", "handler"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let lock = fs::read_to_string(handler.join("rein.lock")).unwrap();
    assert!(lock.contains("\"pr-handling\"") && lock.contains("sha256:"), "{lock}");
    let out = rein(&home, &["check", dir]);
    assert!(out.status.success() && text(&out.stderr).contains("0 errors, 0 warnings"), "{}", text(&out.stderr));

    fs::write(home.join("pr-handling/main.rein"), TARGET.replace("Handle", "Take")).unwrap();
    let out = rein(&home, &["check", dir]);
    assert!(!out.status.success());
    let shown = text(&out.stdout);
    assert!(shown.contains("rein.lock") && shown.contains("program `pr-handling` changed") && shown.contains("rein lock"), "{shown}");
    let out = rein(&home, &["step", "handler", "--input", "pr=u/1", "--json"]);
    assert!(text(&out.stdout).contains("program `pr-handling` changed"), "{}", text(&out.stdout));

    let out = rein(&home, &["lock", "handler"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let out = rein(&home, &["check", dir]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let out = rein(&home, &["step", "handler", "--input", "pr=u/1", "--json"]);
    assert!(text(&out.stdout).contains("Take u/1."), "{}", text(&out.stdout));

    program(&home, "handler", &caller("`Done ${pr}.`"));
    let out = rein(&home, &["check", dir]);
    assert!(out.status.success());
    assert!(text(&out.stdout).contains("does not call"), "{}", text(&out.stdout));
    fs::remove_dir_all(&home).unwrap();
}
