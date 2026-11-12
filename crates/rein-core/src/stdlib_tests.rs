use super::*;
use crate::check::check_function;
use crate::diagnostic::Severity;

#[test]
fn names_are_sorted_and_include_request() {
    assert_eq!(names(), vec!["askUser", "delegate", "request"]);
}

#[test]
fn every_std_template_is_a_valid_template() {
    for name in names() {
        let Ok(Binding::Template { function, .. }) = item(name) else {
            continue;
        };
        let no_imports = |_: &str| Err("no imports".to_string());
        let errors: Vec<_> = check_function(function, name, &no_imports)
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:?}");
    }
}

#[test]
fn request_is_built_in_not_a_template() {
    assert!(matches!(item("request"), Ok(Binding::Request)));
    assert!(request_doc().contains("await request"));
}

#[test]
fn unknown_name_lists_what_std_has() {
    let err = item("nope").unwrap_err();
    assert_eq!(
        err,
        "`nope` is not in std; std has: askUser, delegate, request"
    );
}

#[test]
fn std_template_docs_come_from_the_file() {
    assert_eq!(
        doc("delegate"),
        "Hand a task to a subagent, with the model to use."
    );
    assert_eq!(
        doc("askUser"),
        "Ask the user a question and wait for the answer."
    );
}
