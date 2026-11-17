use super::{
    add_args, after_moves, bare_name, move_error, move_target, newest_tag, output_within, selected, spawn_error,
    tag_pin, tag_pins,
};
use serde_json::json;
use std::io;
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
    assert_eq!(newest_tag(out, None), Some(("v0.10.0".to_string(), [0, 10, 0])));
    assert_eq!(newest_tag("", None), None);
}

#[test]
fn names_limit_the_programs_to_move() {
    assert!(selected(&[], "a"));
    assert!(selected(&["a"], "a"));
    assert!(!selected(&["a"], "b"));
    assert!(selected(&["a@latest"], "a"));
    assert!(selected(&["@s/p@^1"], "@s/p"));
    assert!(selected(&["@s/p"], "@s/p"));
    assert!(!selected(&["a@latest"], "b"));
    assert_eq!(bare_name("a@1.2.3"), "a");
    assert_eq!(bare_name("a"), "a");
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
        "dependencies": {"a": "github:o/a#v0.3.0"},
        "devDependencies": {"b": "github:o/b#v0.3.0"},
        "optionalDependencies": {"c": "github:o/c#v0.3.0"}
    });
    let pins = tag_pins(&m, &[]);
    assert_eq!(add_args(&pins[0], "v0.4.0"), ["add", "--ignore-scripts", "github:o/a#v0.4.0"]);
    assert_eq!(add_args(&pins[1], "v0.4.0"), ["add", "--ignore-scripts", "--save-dev", "github:o/b#v0.4.0"]);
    assert_eq!(add_args(&pins[2], "v0.4.0"), ["add", "--ignore-scripts", "--save-optional", "github:o/c#v0.4.0"]);
}

#[test]
fn a_failed_update_names_the_programs_already_moved() {
    assert_eq!(after_moves("pnpm update exited with 1", &[]), "pnpm update exited with 1");
    assert_eq!(
        after_moves("pnpm update exited with 1", &["a".to_string(), "b".to_string()]),
        "pnpm update exited with 1. Already moved: a, b. They stay moved."
    );
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
    let out = "a\trefs/tags/v0.3.0\nb\trefs/tags/v0.4.1\nc\trefs/tags/v1.0.0\n";
    assert_eq!(move_target([0, 3, 0], out), Some("v0.4.1".to_string()));
    assert_eq!(move_target([0, 4, 1], out), None, "already the newest of its major");
    assert_eq!(move_target([1, 0, 0], out), None, "already the newest of its major");
    assert_eq!(move_target([0, 3, 0], "c\trefs/tags/v1.0.0\n"), None, "a new major is not taken");
    assert_eq!(move_target([0, 3, 0], "d\trefs/tags/main\n"), None, "no v tag");
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
