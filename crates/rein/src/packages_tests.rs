use super::{
    add_args, after_moves, bare_name, is_range, ls_remote_command, move_error, move_tag_pins, move_target, newest_tag,
    output_within, selected, spawn_error, tag_pin, tag_pins, unmoved_note, Pin, PACKAGE,
};
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[test]
fn reads_only_github_specs_pinned_to_a_v_tag() {
    assert_eq!(
        tag_pin("github:tupe12334/rein-review-handler#v0.3.0"),
        Some(("tupe12334/rein-review-handler", [0, 3, 0]))
    );
    for spec in [
        "github:tupe12334/rein-review-handler",
        "github:owner/repo#main",
        "github:owner/repo#0.3.0",
        "github:owner/repo#v0.3.0-rc1",
        "github:owner/repo/extra#v1.0.0",
        "github:owner/repo#v+1.0.0",
        "github:owner/repo#v1.+0.0",
        "github:/repo#v1.0.0",
        "file:../review-handler",
        "^0.3.0",
    ] {
        assert_eq!(tag_pin(spec), None, "{spec}");
    }
}

#[test]
fn picks_the_highest_release_tag_by_number() {
    let out = "a\trefs/tags/v0.3.0\nb\trefs/tags/v0.10.0\nc\trefs/tags/v0.4.0\nd\trefs/tags/v0.11.0-rc1\ne\trefs/tags/latest\nf\trefs/tags/v+9.0.0\n";
    assert_eq!(newest_tag(out, |_| true), Some(("v0.10.0".to_string(), [0, 10, 0])));
    assert_eq!(newest_tag("", |_| true), None);
}

#[test]
fn names_limit_the_programs_to_move() {
    assert!(selected(&[], "a"));
    assert!(selected(&["a"], "a"));
    assert!(!selected(&["a"], "b"));
    assert!(selected(&["a@latest"], "a"));
    assert!(selected(&["@s/p"], "@s/p"));
    assert!(!selected(&["a@latest"], "b"));
    assert_eq!(bare_name("a@1.2.3"), "a");
    assert_eq!(bare_name("a"), "a");
}

#[test]
fn a_version_range_selects_no_tag_move() {
    assert!(!selected(&["a@^1"], "a"), "a range is no tag move");
    assert!(!selected(&["@s/p@~1.2"], "@s/p"));
    assert!(!selected(&["a@1.2.3"], "a"));
    assert!(selected(&["a@^1", "b"], "b"), "the other names still count");
    assert!(is_range("a@^1"));
    assert!(is_range("@s/p@~1.2"));
    assert!(!is_range("a@latest"));
    assert!(!is_range("@s/p"));
    assert!(!is_range("a"));
}

#[test]
fn every_dependency_field_is_read_and_filtered_by_name() {
    let m = json!({
        "dependencies": {"a": "github:o/a#v0.3.0", "x": "^1.0.0"},
        "devDependencies": {"b": "github:o/b#v0.3.0"},
        "optionalDependencies": {"c": "github:o/c#v0.3.0"}
    });
    let mut all: Vec<(&str, &str)> = tag_pins(&m, &[]).iter().map(|p| (p.name, p.section)).collect();
    all.sort();
    assert_eq!(all, [("a", "dependencies"), ("b", "devDependencies"), ("c", "optionalDependencies")]);
    let some: Vec<&str> = tag_pins(&m, &["b@latest"]).iter().map(|p| p.name).collect();
    assert_eq!(some, ["b"]);
}

#[test]
fn the_pnpm_add_target_keeps_the_manifest_field() {
    let m = json!({
        "dependencies": {"@s/a": "github:o/a#v0.3.0"},
        "devDependencies": {"b": "github:o/b#v0.3.0"},
        "optionalDependencies": {"c": "github:o/c#v0.3.0"}
    });
    let pins = tag_pins(&m, &[]);
    assert_eq!(add_args(&pins[0], "v0.4.0"), ["add", "--ignore-scripts", "@s/a@github:o/a#v0.4.0"]);
    assert_eq!(add_args(&pins[1], "v0.4.0"), ["add", "--ignore-scripts", "--save-dev", "b@github:o/b#v0.4.0"]);
    assert_eq!(add_args(&pins[2], "v0.4.0"), ["add", "--ignore-scripts", "--save-optional", "c@github:o/c#v0.4.0"]);
}

#[test]
fn a_failed_update_names_the_programs_already_moved() {
    assert_eq!(after_moves("pnpm update exited with 1", &[]), "pnpm update exited with 1");
    assert_eq!(
        after_moves("pnpm update exited with 1", &["a".to_string(), "b".to_string()]),
        "pnpm update exited with 1. Already moved: a, b. They stay moved."
    );
}

#[test]
fn ls_remote_gives_up_on_a_stalled_transfer_by_itself() {
    let cmd = ls_remote_command("https://github.com/o/r");
    let env = |key: &str| {
        cmd.get_envs().find(|(k, _)| *k == key).and_then(|(_, v)| v).map(|v| v.to_string_lossy().into_owned())
    };
    assert_eq!(env("GIT_HTTP_LOW_SPEED_LIMIT").as_deref(), Some("1"));
    assert_eq!(env("GIT_HTTP_LOW_SPEED_TIME").as_deref(), Some("20"));
    assert_eq!(env("GIT_TERMINAL_PROMPT").as_deref(), Some("0"));
}

#[cfg(unix)]
#[test]
fn a_command_past_its_limit_is_killed() {
    let mut slow = Command::new("sleep");
    slow.arg("5");
    assert!(matches!(output_within(&mut slow, Duration::from_millis(100)), Ok(None)));
    let mut quick = Command::new("echo");
    quick.arg("hi");
    let out = output_within(&mut quick, Duration::from_secs(10)).unwrap().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hi\n");
}

#[test]
fn moves_only_to_a_higher_tag_of_the_same_major() {
    let out = "a\trefs/tags/v1.3.0\nb\trefs/tags/v1.4.1\nc\trefs/tags/v2.0.0\n";
    assert_eq!(move_target([1, 3, 0], out), Some("v1.4.1".to_string()));
    assert_eq!(move_target([1, 4, 1], out), None, "already the newest of its major");
    assert_eq!(move_target([2, 0, 0], out), None, "already the newest of its major");
    assert_eq!(move_target([1, 3, 0], "c\trefs/tags/v2.0.0\n"), None, "a new major is not taken");
    assert_eq!(move_target([1, 3, 0], "d\trefs/tags/main\n"), None, "no v tag");
}

#[test]
fn under_major_zero_a_pin_moves_only_within_its_minor() {
    let out = "a\trefs/tags/v0.3.0\nb\trefs/tags/v0.3.5\nc\trefs/tags/v0.4.0\nd\trefs/tags/v1.0.0\n";
    assert_eq!(move_target([0, 3, 0], out), Some("v0.3.5".to_string()));
    assert_eq!(move_target([0, 3, 5], out), None, "already the newest of its minor");
    assert_eq!(move_target([0, 2, 0], "c\trefs/tags/v0.4.0\n"), None, "a minor bump is breaking under major 0");
}

#[test]
fn a_pin_left_behind_a_newer_line_gets_a_note() {
    let pin = Pin { name: "a", repo: "o/a", current: [0, 3, 0], section: "dependencies" };
    let note = unmoved_note(&pin, "c\trefs/tags/v0.4.0\n").unwrap();
    assert!(note.starts_with("note: a stays on v0.3.0; o/a has v0.4.0, a new minor."), "{note}");
    let pin = Pin { name: "a", repo: "o/a", current: [1, 3, 0], section: "dependencies" };
    let note = unmoved_note(&pin, "c\trefs/tags/v2.0.0\n").unwrap();
    assert!(note.starts_with("note: a stays on v1.3.0; o/a has v2.0.0, a new major."), "{note}");
    assert_eq!(unmoved_note(&pin, "a\trefs/tags/v1.3.0\n"), None, "the newest of its line needs no note");
    let dev = Pin { name: "a", repo: "o/a", current: [1, 3, 0], section: "devDependencies" };
    assert_eq!(
        unmoved_note(&dev, "c\trefs/tags/v2.0.0\n").as_deref(),
        Some("note: a stays on v1.3.0; o/a has v2.0.0, a new major. To move, run: rein install -D github:o/a#v2.0.0")
    );
    assert_eq!(
        unmoved_note(&pin, "d\trefs/tags/main\n").as_deref(),
        Some("warning: a is not moved: o/a has no v<x.y.z> tag")
    );
}

#[test]
fn a_stopped_move_names_the_programs_already_moved() {
    assert_eq!(
        move_error("pnpm add github:o/a#v0.4.0 failed", &[]),
        "pnpm add github:o/a#v0.4.0 failed. Already moved: none. pnpm update did not run."
    );
    assert_eq!(
        move_error("pnpm add github:o/c#v0.4.0 failed", &["a", "b"]),
        "pnpm add github:o/c#v0.4.0 failed. Already moved: a, b. pnpm update did not run."
    );
}

#[test]
fn a_missing_pnpm_gets_the_install_hint() {
    let missing = io::Error::from(io::ErrorKind::NotFound);
    assert!(spawn_error("pnpm", &missing).starts_with("pnpm is not installed or not on PATH"));
    let denied = io::Error::from(io::ErrorKind::PermissionDenied);
    assert!(spawn_error("/x/pnpm", &denied).starts_with("cannot run /x/pnpm: "));
}

/// The `git ls-remote` output of the stub repos. No network. `o/c` fails, as an unreachable repo does.
#[cfg(unix)]
fn canned_tags(repo: &str) -> Result<String, String> {
    let tags = match repo {
        "o/a" => vec!["v0.3.0", "v0.3.2"],
        "o/b" => vec!["v2.0.0"],
        "o/c" => return Err("network down".to_string()),
        "o/d" => vec!["v0.4.0"],
        "o/f" => vec!["v1.2.0"],
        "o/g" => vec!["v1.0.0", "v1.2.0"],
        "o/h" => vec!["v1.0.0"],
        other => return Err(format!("no stub for {other}")),
    };
    Ok(tags.iter().map(|t| format!("x\trefs/tags/{t}\n")).collect())
}

/// A temp folder with the manifest, and a stub pnpm. The stub appends each call's arguments to `log` and fails on
/// any `f@github` argument. Returns the folder and the stub's path.
#[cfg(unix)]
fn stub_folder(label: &str, manifest: &Value) -> (PathBuf, String) {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("rein-move-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(PACKAGE), manifest.to_string()).unwrap();
    let stub = dir.join("pnpm");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}/log'\ncase \"$*\" in *f@github*) exit 1 ;; esac\nexit 0\n",
        dir.display()
    );
    fs::write(&stub, script).unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    (dir, stub.display().to_string())
}

#[cfg(unix)]
fn log_of(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("log")).unwrap_or_default().lines().map(String::from).collect()
}

#[cfg(unix)]
#[test]
fn move_tag_pins_runs_pnpm_add_for_each_move_and_skips_the_rest() {
    let m = json!({
        "dependencies": {"a": "github:o/a#v0.3.0", "c": "github:o/c#v0.3.0", "d": "github:o/d#v0.3.0"},
        "devDependencies": {"b": "github:o/b#v1.0.0", "g": "github:o/g#v1.0.0"},
        "optionalDependencies": {"h": "github:o/h#v1.0.0"}
    });
    let (dir, pnpm) = stub_folder("all", &m);
    let moved = move_tag_pins(&pnpm, &dir, &[], &canned_tags).unwrap();
    assert_eq!(moved, ["a", "g"]);
    assert_eq!(
        log_of(&dir),
        ["add --ignore-scripts a@github:o/a#v0.3.2", "add --ignore-scripts --save-dev g@github:o/g#v1.2.0"]
    );
    let moved = move_tag_pins(&pnpm, &dir, &["g"], &canned_tags).unwrap();
    assert_eq!(moved, ["g"], "a name limits the programs");
    assert_eq!(log_of(&dir).len(), 3);
    assert_eq!(move_tag_pins(&pnpm, &dir, &["a@^1"], &canned_tags).unwrap(), Vec::<String>::new());
    assert_eq!(log_of(&dir).len(), 3, "a range moves no tag pin");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn move_tag_pins_names_the_programs_already_moved_when_a_move_fails() {
    let m = json!({
        "dependencies": {"a": "github:o/a#v0.3.0"},
        "devDependencies": {"f": "github:o/f#v1.0.0"}
    });
    let (dir, pnpm) = stub_folder("fails", &m);
    let err = move_tag_pins(&pnpm, &dir, &[], &canned_tags).unwrap_err();
    assert_eq!(err, "pnpm add f@github:o/f#v1.2.0 failed. Already moved: a. pnpm update did not run.");
    assert_eq!(log_of(&dir).len(), 2, "the stop is at the first failed add");
    let err = move_tag_pins("/nonexistent/rein-stub-pnpm", &dir, &[], &canned_tags).unwrap_err();
    assert!(err.starts_with("pnpm is not installed or not on PATH"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}
