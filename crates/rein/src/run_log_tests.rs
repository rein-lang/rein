use super::*;

fn os(text: &str) -> Option<OsString> {
    Some(OsString::from(text))
}

#[test]
fn the_folder_follows_runs_dir_then_xdg_then_home_and_empty_runs_dir_turns_it_off() {
    assert_eq!(dir_from(os(""), os("/state"), os("/home/u")), None);
    assert_eq!(
        dir_from(os("/runs"), os("/state"), os("/home/u")),
        Some(PathBuf::from("/runs"))
    );
    assert_eq!(
        dir_from(None, os("/state"), os("/home/u")),
        Some(PathBuf::from("/state/rein/runs"))
    );
    assert_eq!(
        dir_from(None, os(""), os("/home/u")),
        Some(PathBuf::from("/home/u/.local/state/rein/runs"))
    );
    assert_eq!(dir_from(None, None, None), None);
}

#[cfg(unix)]
#[test]
fn the_log_folder_and_files_are_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let base = env::temp_dir().join(format!("rein-run-log-mode-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let folder = base.join("runs");
    let path = folder.join("step-mode.jsonl");
    make_folder(&folder);
    assert!(open(&path, true).is_some());
    assert_eq!(fs::metadata(&folder).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_session_key_keeps_only_safe_characters_and_falls_back_to_nosession() {
    assert_eq!(session_key(Some("abc_9-Z")), "abc_9-Z");
    assert_eq!(session_key(Some("../x y")), "xy");
    assert_eq!(session_key(Some("")), "nosession");
    assert_eq!(session_key(None), "nosession");
}
