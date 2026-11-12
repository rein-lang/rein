use super::fuzzy;

#[test]
fn fuzzy_matches_characters_in_order() {
    assert!(fuzzy("bref", "billing/refund"));
    assert!(fuzzy("", "main"));
    assert!(!fuzzy("fb", "billing/refund"));
}
