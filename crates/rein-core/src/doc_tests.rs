use super::*;

fn doc(inner: &str) -> DocBlock {
    parse_doc(inner, 0)
}

#[test]
fn description_keeps_line_breaks_and_collapses_blank_runs() {
    let d = doc("\n * First line.\n * Second line.\n *\n *\n *\n * New paragraph.\n ");
    assert_eq!(
        d.description.value,
        "First line.\nSecond line.\n\nNew paragraph."
    );
    assert!(d.tags.is_empty());
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
        "Is the customer a VIP?\nCheck the account tier in the CRM."
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
