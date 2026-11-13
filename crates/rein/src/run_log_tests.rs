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

#[test]
fn the_session_key_keeps_only_safe_characters_and_falls_back_to_nosession() {
    assert_eq!(session_key(Some("abc_9-Z")), "abc_9-Z");
    assert_eq!(session_key(Some("../x y")), "xy");
    assert_eq!(session_key(Some("")), "nosession");
    assert_eq!(session_key(None), "nosession");
}
