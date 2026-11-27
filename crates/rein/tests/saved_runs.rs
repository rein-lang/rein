//! `rein start`, `answer`, `status`, `stop` and `runs`: a run kept in a file, so each call sends only its new answer.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rein-saved-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Runs `rein` in the examples folder, with saved runs in `saved`.
fn rein(saved: &Path, args: &[&str]) -> (i32, Value) {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").canonicalize().unwrap();
    let out: Output = Command::new(env!("CARGO_BIN_EXE_rein"))
        .env("REIN_RUNS_DIR", "")
        .env("REIN_SAVED_RUNS_DIR", saved)
        .env("REIN_HOME", examples.join("no-user-programs"))
        .env("REIN_SYSTEM_DIRS", "")
        .current_dir(&examples)
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let value = serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"));
    (out.status.code().unwrap_or(-1), value)
}

#[test]
fn a_run_keeps_its_inputs_and_answers_between_calls() {
    let saved = scratch("keeps");
    let pr = "https://github.com/o/r/pull/1";

    let (code, start) = rein(&saved, &["start", "pr-handling", "--input", &format!("pr={pr}")]);
    assert_eq!(code, 2, "{start}");
    assert_eq!(start["key"], "main.draft", "{start}");
    let run = start["run"].as_str().unwrap().to_string();

    // The second call sends only its answer. The `pr` input is not sent again.
    let (code, next) = rein(&saved, &["answer", &run, "main.draft=no"]);
    assert_eq!(code, 2, "{next}");
    assert_eq!(next["key"], "main.task", "{next}");

    let (code, status) = rein(&saved, &["status", &run]);
    assert_eq!(code, 2, "{status}");
    assert_eq!(status["key"], "main.task", "{status}");
    assert_eq!(status["answers"]["main.draft"], "no", "{status}");

    let (code, listed) = rein(&saved, &["runs"]);
    assert_eq!(code, 0);
    assert_eq!(listed[0]["run"], run.as_str(), "{listed}");
    assert_eq!(listed[0]["program"], "pr-handling", "{listed}");

    let (code, stopped) = rein(&saved, &["stop", &run]);
    assert_eq!(code, 0, "{stopped}");
    assert_eq!(stopped["stopped"], true);
    let (code, gone) = rein(&saved, &["status", &run]);
    assert_eq!(code, 1, "{gone}");
    assert_eq!(gone["kind"], "error", "{gone}");
}

#[test]
fn a_wrong_answer_saves_nothing_and_the_run_waits_on_the_same_key() {
    let saved = scratch("wrong");
    let pr = "https://github.com/o/r/pull/2";
    let (_, start) = rein(&saved, &["start", "pr-handling", "--input", &format!("pr={pr}")]);
    let run = start["run"].as_str().unwrap().to_string();
    rein(&saved, &["answer", &run, "main.draft=no"]);

    let (code, bad) = rein(&saved, &["answer", &run, "main.task=bogus"]);
    assert_eq!(code, 1, "{bad}");
    assert_eq!(bad["kind"], "error", "{bad}");

    let (_, status) = rein(&saved, &["status", &run]);
    assert_eq!(status["key"], "main.task", "a bad answer must not move the run: {status}");
    assert!(status["answers"].get("main.task").is_none(), "{status}");
}

#[test]
fn a_run_id_that_leaves_the_folder_is_refused() {
    let saved = scratch("path");
    let (code, out) = rein(&saved, &["status", "../escape"]);
    assert_eq!(code, 1, "{out}");
    assert!(out["message"].as_str().unwrap().contains("invalid run id"), "{out}");
}
