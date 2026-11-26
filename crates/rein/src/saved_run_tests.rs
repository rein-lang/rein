use std::ffi::OsString;
use std::path::PathBuf;

use super::{dir_from, file};

fn os(s: &str) -> Option<OsString> {
    Some(OsString::from(s))
}

#[test]
fn the_folder_is_the_override_then_xdg_state_then_home() {
    assert_eq!(dir_from(os("/x"), os("/s"), os("/h")).unwrap(), PathBuf::from("/x"));
    assert_eq!(dir_from(None, os("/s"), os("/h")).unwrap(), PathBuf::from("/s/rein/saved-runs"));
    assert_eq!(dir_from(os(""), None, os("/h")).unwrap(), PathBuf::from("/h/.local/state/rein/saved-runs"));
}

#[test]
fn with_no_folder_at_all_the_error_names_the_override() {
    let err = dir_from(None, None, None).unwrap_err();
    assert!(err.contains("REIN_SAVED_RUNS_DIR"), "{err}");
}

#[test]
fn a_run_id_with_a_path_separator_is_refused() {
    assert!(file("../etc").is_err());
    assert!(file("run/x").is_err());
    assert!(file("").is_err());
}

#[test]
fn a_run_id_from_new_id_is_accepted() {
    let id = super::new_id();
    assert!(file(&id).is_ok(), "{id}");
}
