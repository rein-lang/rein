use super::*;
use std::fs;
use std::path::PathBuf;

/// Loads an example project from `examples/rein`, by its folder name.
fn project(name: &str) -> Project {
    let root = PathBuf::from(format!(
        "{}/../../examples/rein/{name}",
        env!("CARGO_MANIFEST_DIR")
    ));
    Project::load(&root).unwrap()
}

/// The 1-based line of the first line of `file` in `name` that contains `needle`.
fn line_with(name: &str, file: &str, needle: &str) -> usize {
    let path = format!(
        "{}/../../examples/rein/{name}/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = fs::read_to_string(path).unwrap();
    text.lines()
        .position(|l| l.contains(needle))
        .map(|i| i + 1)
        .unwrap()
}

#[test]
fn question_is_the_line_that_first_tests_it() {
    let p = project("pr-handling");
    let main = p.get("main").unwrap();
    assert_eq!(
        question(main, "draft"),
        line_with("pr-handling", "main.rein", "if (draft)")
    );
    assert_eq!(
        question(main, "task"),
        line_with("pr-handling", "main.rein", "switch (task)")
    );
}

#[test]
fn handoff_is_the_return_that_calls_the_target() {
    let p = project("pr-handling");
    assert_eq!(
        handoff(&p, "main", "merge"),
        line_with("pr-handling", "main.rein", "@./merge(pr=pr)")
    );
}

#[test]
fn done_is_the_return_that_gives_the_text() {
    let p = project("pr-handling");
    let text = "Merge the pull request https://example.test/pull/1. Then delete its branch.";
    let expected = line_with("pr-handling", "merge.rein", "Merge the pull request");
    assert_eq!(done(&p, "merge", text), expected);
}

#[test]
fn request_is_the_line_of_the_await() {
    let p = project("support-ticket");
    let faq = p.get("faq").unwrap();
    assert_eq!(
        request(faq, "solved"),
        line_with("support-ticket", "faq.rein", "await request")
    );
}
