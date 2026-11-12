use std::fs;
use std::path::{Path, PathBuf};

use super::{current, diagnostics, hash, render, LOCK_FILE};
use crate::diagnostic::Severity;
use crate::program_fixture::with_program;
use crate::project::Project;

const CALL: &str = "return @pr-handling/main(pr=pr);";

fn dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rein-lock-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn project_in(dir: &Path) -> Project {
    let mut p = with_program(CALL);
    p.root = dir.to_path_buf();
    p
}

fn source(text: &str) -> Project {
    Project::from_sources(PathBuf::from("/x"), vec![(PathBuf::from("/x/main.rein"), text.to_string())])
}

#[test]
fn hash_follows_the_content() {
    assert_eq!(hash(&source("a")), hash(&source("a")));
    assert_ne!(hash(&source("a")), hash(&source("b")));
    assert!(hash(&source("a")).starts_with("sha256:"));
    assert_eq!(hash(&source("a")).len(), "sha256:".len() + 64);
}

#[test]
fn hash_covers_the_file_path() {
    let moved = Project::from_sources(PathBuf::from("/x"), vec![(PathBuf::from("/x/other.rein"), "a".to_string())]);
    assert_ne!(hash(&source("a")), hash(&moved));
}

#[test]
fn no_lock_and_no_hand_off_is_silent() {
    let p = Project::from_sources(PathBuf::from("/nowhere"), vec![]);
    assert!(diagnostics(&p).is_empty());
}

#[test]
fn missing_lock_is_a_warning() {
    let d = diagnostics(&project_in(&dir("missing")));
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].severity, Severity::Warning);
    assert!(d[0].message.contains("rein lock"), "{}", d[0].message);
}

#[test]
fn matching_lock_is_clean() {
    let dir = dir("match");
    let p = project_in(&dir);
    fs::write(dir.join(LOCK_FILE), render(current(&p))).unwrap();
    assert!(diagnostics(&p).is_empty());
}

#[test]
fn changed_program_is_an_error_with_both_hashes() {
    let dir = dir("changed");
    let p = project_in(&dir);
    let mut entries = current(&p);
    entries.get_mut("pr-handling").unwrap().hash = "sha256:old".into();
    fs::write(dir.join(LOCK_FILE), render(entries)).unwrap();
    let d = diagnostics(&p);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].severity, Severity::Error);
    let message = &d[0].message;
    assert!(message.contains("`pr-handling`") && message.contains("sha256:old"), "{message}");
    assert!(message.contains(&current(&p)["pr-handling"].hash) && message.contains("rein lock"), "{message}");
}

#[test]
fn unpinned_call_is_an_error_and_unused_entry_a_warning() {
    let dir = dir("entries");
    let p = project_in(&dir);
    let mut other = current(&p);
    let pin = other.remove("pr-handling").unwrap();
    other.insert("gone".into(), pin);
    fs::write(dir.join(LOCK_FILE), render(other)).unwrap();
    let d = diagnostics(&p);
    assert_eq!(d.len(), 2);
    assert!(d.iter().any(|d| d.severity == Severity::Error && d.message.contains("is not in rein.lock")));
    assert!(d.iter().any(|d| d.severity == Severity::Warning && d.message.contains("`gone`")));
}

#[test]
fn broken_lock_is_an_error() {
    let dir = dir("broken");
    fs::write(dir.join(LOCK_FILE), "{").unwrap();
    let d = diagnostics(&project_in(&dir));
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].severity, Severity::Error);
}
