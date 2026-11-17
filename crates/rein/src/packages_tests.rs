use super::{move_target, newest_tag, selected, tag_pin};

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
    let out = "a\trefs/tags/v0.3.0\nb\trefs/tags/v0.10.0\nc\trefs/tags/v0.4.0\nd\trefs/tags/v0.11.0-rc1\ne\trefs/tags/latest\n";
    assert_eq!(newest_tag(out, None), Some(("v0.10.0".to_string(), [0, 10, 0])));
    assert_eq!(newest_tag("", None), None);
}

#[test]
fn names_limit_the_programs_to_move() {
    assert!(selected(&[], "a"));
    assert!(selected(&["a"], "a"));
    assert!(!selected(&["a"], "b"));
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
