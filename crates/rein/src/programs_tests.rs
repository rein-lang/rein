use super::range_holds;

#[test]
fn caret_and_plain_ranges_hold_like_semver() {
    assert_eq!(range_holds("^0.1.0", "0.1.0"), Some(true));
    assert_eq!(range_holds("^0.1.0", "0.1.3"), Some(true));
    assert_eq!(range_holds("0.1.0", "0.1.0"), Some(true));
    assert_eq!(range_holds("^0.2.0", "0.1.0"), Some(false));
    assert_eq!(range_holds("^0.1.2", "0.1.1"), Some(false));
    assert_eq!(range_holds("^1.0.0", "0.1.0"), Some(false));
    assert_eq!(range_holds("^1.0.0", "1.4.2"), Some(true));
    assert_eq!(range_holds(">=0.1.0 <0.2.0", "0.1.0"), None);
}
