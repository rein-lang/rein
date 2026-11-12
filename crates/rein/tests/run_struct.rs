//! `rein run` lists the fields of a struct answer and takes the JSON object as the answer line.

use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

const PICKER: &str = "interface Pick {\n  /** The link of the pull request. */\n  pr: string;\n  /** Which task runs. */\n  task: \"review\" | \"merge\";\n}\n/** Picks a PR. */\nexport default async function main() {\n  /** The pull request of the next notification. */\n  const pick: Pick = await request(`Pick the next pull request notification.`);\n  return `Do ${pick.task} on ${pick.pr}.`;\n}\n";

#[test]
fn run_lists_struct_fields_rejects_a_bad_object_and_takes_a_good_one() {
    let dir = std::env::temp_dir().join(format!("rein-run-struct-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("main.rein"), PICKER).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rein"))
        .args(["run", "--root", dir.to_str().unwrap()])
        .env("REIN_HOME", dir.join("home"))
        .env("REIN_SYSTEM_DIRS", dir.join("none"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{\"pr\":\"u/1\"}\n{\"pr\":\"u/1\",\"task\":\"merge\"}\n").unwrap();
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("  pr: string - The link of the pull request."), "{text}");
    assert!(text.contains("  task: \"review\" | \"merge\" - Which task runs."), "{text}");
    assert!(err.contains("error:"), "{err}");
    assert!(text.contains("Do merge on u/1."), "{text}");
    assert!(out.status.success(), "{text}{err}");
    fs::remove_dir_all(&dir).unwrap();
}
