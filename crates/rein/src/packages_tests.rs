use super::{newest_tag, tag_pin};

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
    assert_eq!(newest_tag(out), Some(("v0.10.0".to_string(), [0, 10, 0])));
    assert_eq!(newest_tag(""), None);
}
