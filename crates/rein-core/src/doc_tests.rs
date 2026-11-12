use super::*;

fn doc(inner: &str) -> DocBlock {
    parse_doc(inner, 0)
}

#[test]
fn description_joins_lines_and_collapses_blank_runs_to_a_paragraph_break() {
    let d = doc("\n * First line.\n * Second line.\n *\n *\n *\n * New paragraph.\n ");
    assert_eq!(
        d.description.value,
        "First line. Second line.\n\nNew paragraph."
    );
    assert!(d.tags.is_empty());
}

#[test]
fn field_doc_over_two_lines_keeps_both_lines() {
    let d = doc("\n * Does the pull request wait for another pull request or change that is\n * not merged yet?\n ");
    assert_eq!(
        d.description.value,
        "Does the pull request wait for another pull request or change that is not merged yet?"
    );
}

#[test]
fn doc_over_three_lines_keeps_all_three() {
    let d = doc("\n * One\n * two\n * three.\n ");
    assert_eq!(d.description.value, "One two three.");
}

#[test]
fn lines_without_a_leading_star_are_joined_too() {
    let d = doc("\n  Is the customer a VIP\n  and paying?\n ");
    assert_eq!(d.description.value, "Is the customer a VIP and paying?");
}

#[test]
fn ragged_indent_is_trimmed_and_inner_spaces_are_kept() {
    let d = doc("\n *   Padded   start\n *     end   \n ");
    assert_eq!(d.description.value, "Padded   start end");
}

#[test]
fn single_line_block_is_the_description() {
    assert_eq!(
        doc(" Page the engineer. ").description.value,
        "Page the engineer."
    );
    assert_eq!(doc("").description.value, "");
}

#[test]
fn multi_line_parameter_doc_is_one_description() {
    let d = doc("\n * Is the customer a VIP?\n * Check the account tier in the CRM.\n ");
    assert_eq!(
        d.description.value,
        "Is the customer a VIP? Check the account tier in the CRM."
    );
}

#[test]
fn tags_are_recorded_by_name_and_stay_out_of_the_description() {
    let d = doc("\n * Desc.\n * @returns Something.\n *   more of returns.\n * @example foo\n ");
    assert_eq!(d.description.value, "Desc.");
    let names: Vec<&str> = d.tags.iter().map(|t| t.value.as_str()).collect();
    assert_eq!(names, ["returns", "example"]);
}

#[test]
fn param_tag_is_recorded_so_the_checker_can_reject_it() {
    let d = doc("\n * Desc.\n * @param ci What is CI?\n ");
    assert_eq!(d.tags[0].value, "param");
    assert_eq!(d.description.value, "Desc.");
}

#[test]
fn tag_span_covers_the_at_sign_and_name() {
    let inner = "\n * @param ci Question?\n ";
    let d = parse_doc(inner, 10);
    let tag = &d.tags[0];
    assert_eq!(&inner[tag.span.start - 10..tag.span.end - 10], "@param");
}

#[test]
fn spans_point_into_the_block() {
    // Block text starts at offset 10. The description line is " * Question?".
    let inner = "\n * Question?\n ";
    let d = parse_doc(inner, 10);
    let text = &d.description.span;
    assert_eq!(&inner[text.start - 10..text.end - 10], "Question?");
}
