use super::*;
use crate::diagnostic::{Severity, REMOVED_CODE};
use std::path::PathBuf;

/// A project of the given (name, source) files, all in `/proj`.
fn project_of(files: &[(&str, &str)]) -> Project {
    let root = PathBuf::from("/proj");
    let sources = files
        .iter()
        .map(|(n, t)| (root.join(format!("{n}.rein")), t.to_string()))
        .collect();
    Project::from_sources(root, sources)
}

fn main_with(imports: &str, body: &str) -> String {
    format!("{imports}\n/** Entry. */\nexport default function main() {{\n  {body}\n}}\n")
}

const FIX: &str = "/** Fix CI. */\nexport default function fix() {\n  return `Fix the CI.`;\n}\n";

/// The removed-form errors of `main`, by message.
fn removals(project: &Project) -> Vec<String> {
    project.check()["main"]
        .iter()
        .filter(|d| d.is_removed())
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn at_reference_to_a_function_gets_the_named_import_suggestion() {
    let project = project_of(&[("ci/fix", FIX), ("main", &main_with("", "return @./ci/fix;"))]);
    assert_eq!(
        removals(&project),
        vec![
            "`@./ci/fix` was removed in rein 0.2.0; write `import { fix } from \"./ci/fix\";` and use `fix`. Run `rein migrate imports` to change every file"
        ]
    );
}

#[test]
fn at_reference_error_is_an_error_with_the_removed_code() {
    let project = project_of(&[("ci/fix", FIX), ("main", &main_with("", "return @./ci/fix;"))]);
    let diag = project.check()["main"]
        .iter()
        .find(|d| d.is_removed())
        .cloned()
        .unwrap();
    assert_eq!(diag.severity, Severity::Error);
    assert_eq!(diag.code, Some(REMOVED_CODE));
}

#[test]
fn suggested_name_is_the_declared_name_not_the_last_segment() {
    let fix_ci = "/** Fix CI. */\nexport default function fixCi() {\n  return `Fix.`;\n}\n";
    let project = project_of(&[("ci/fix", fix_ci), ("main", &main_with("", "return @./ci/fix;"))]);
    let msgs = removals(&project);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(
        msgs[0].contains("write `import { fixCi } from \"./ci/fix\";` and use `fixCi`"),
        "{msgs:?}"
    );
}

#[test]
fn unresolved_reference_suggests_the_last_path_segment() {
    let project = project_of(&[("main", &main_with("", "return @./ci/missing;"))]);
    let msgs = removals(&project);
    assert!(
        msgs[0].contains("write `import { missing } from \"./ci/missing\";`"),
        "{msgs:?}"
    );
}

#[test]
fn parent_path_reference_keeps_its_path_in_the_suggestion() {
    let project = project_of(&[("ci/fix", FIX), ("ci/run/main", &main_with("", "return @../fix;"))]);
    let diags = project.check();
    let msgs: Vec<_> = diags["ci/run/main"]
        .iter()
        .filter(|d| d.is_removed())
        .map(|d| d.message.clone())
        .collect();
    assert!(msgs[0].contains("write `import { fix } from \"../fix\";`"), "{msgs:?}");
}

#[test]
fn reference_in_a_call_gets_an_error() {
    let tpl = "/** Template. */\nexport default function tpl(a: string) {\n  return `${a}`;\n}\n";
    let project = project_of(&[("tpl", tpl), ("main", &main_with("", "return @./tpl(a=\"x\");"))]);
    assert_eq!(removals(&project).len(), 1);
}

#[test]
fn await_of_a_reference_gets_an_error() {
    let tpl = "/** Template. */\nexport default function tpl(a: string) {\n  return `${a}`;\n}\n";
    let project = project_of(&[
        ("tpl", tpl),
        ("main", "/** Entry. */\nexport default async function main() {\n  /** Pick? */\n  const x: string = await @./tpl(a=\"x\");\n  return `${x}`;\n}\n"),
    ]);
    assert_eq!(removals(&project).len(), 1);
}

#[test]
fn struct_reference_suggests_a_struct_import_with_the_declared_name() {
    let message = "/** A message. */\nexport default interface Msg {\n  /** Opening? */\n  opening: string;\n}\n";
    let main = "/** Entry. */\nexport default function main({ opening }: @./message) {\n  return `${opening}`;\n}\n";
    let project = project_of(&[("message.struct", message), ("main", main)]);
    let msgs = removals(&project);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(msgs[0].starts_with("`@./message` was removed in rein 0.2.0"), "{msgs:?}");
    assert!(
        msgs[0].contains("write `import { Msg } from \"./message.struct\";` and use `Msg`"),
        "{msgs:?}"
    );
}

#[test]
fn unresolved_struct_reference_capitalizes_the_last_segment() {
    let main = "/** Entry. */\nexport default function main({ opening }: @./ticket) {\n  return `${opening}`;\n}\n";
    let project = project_of(&[("main", main)]);
    let msgs = removals(&project);
    assert!(
        msgs[0].contains("write `import { Ticket } from \"./ticket.struct\";`"),
        "{msgs:?}"
    );
}

#[test]
fn program_reference_suggests_the_camel_cased_program_name() {
    let project = project_of(&[("main", &main_with("", "return @pr-handling;"))]);
    let msgs = removals(&project);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(
        msgs[0].contains("write `import { prHandling } from \"pr-handling\";` and use `prHandling`"),
        "{msgs:?}"
    );
}

#[test]
fn program_entry_reference_suggests_the_bare_program_import() {
    let project = project_of(&[("main", &main_with("", "return @pr-handling/main;"))]);
    let msgs = removals(&project);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(
        msgs[0].contains("write `import { prHandling } from \"pr-handling\";` and use `prHandling`"),
        "{msgs:?}"
    );
}

#[test]
fn linked_program_entry_reference_suggests_the_bare_program_import() {
    let project = crate::program_fixture::with_main(main_with("", "return @pr-handling/main;"));
    let msgs = removals(&project);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(msgs[0].contains("from \"pr-handling\""), "{msgs:?}");
    assert!(!msgs[0].contains("from \"pr-handling/main\""), "{msgs:?}");
}

#[test]
fn named_import_use_is_not_a_removal() {
    let project = project_of(&[
        ("ci/fix", FIX),
        ("main", &main_with("import { fix } from \"./ci/fix\";", "return fix;")),
    ]);
    assert!(removals(&project).is_empty(), "{:?}", project.check()["main"]);
}

#[test]
fn at_inside_a_comment_or_doc_is_not_a_reference() {
    let main =
        "/** See @./ci/fix for the old form. */\nexport default function main() {\n  // @./ci/fix\n  return `go`;\n}\n";
    let project = project_of(&[("ci/fix", FIX), ("main", main)]);
    assert!(removals(&project).is_empty(), "{:?}", project.check()["main"]);
}

#[test]
fn default_import_gets_the_named_import_error() {
    let project = project_of(&[
        ("ci/fix", FIX),
        ("main", &main_with("import fix from \"./ci/fix\";", "return fix;")),
    ]);
    assert_eq!(
        removals(&project),
        vec!["default imports were removed in rein 0.2.0; write `import { fix } from \"./ci/fix\"`"]
    );
}

#[test]
fn default_import_suggests_the_declared_name() {
    let fix_ci = "/** Fix CI. */\nexport default function fixCi() {\n  return `Fix.`;\n}\n";
    let project = project_of(&[
        ("ci/fix", fix_ci),
        ("main", &main_with("import fix from \"./ci/fix\";", "return fix;")),
    ]);
    let msgs = removals(&project);
    assert!(
        msgs[0].contains("write `import { fixCi } from \"./ci/fix\"`"),
        "{msgs:?}"
    );
}

#[test]
fn removed_diagnostic_is_an_error_and_other_diagnostics_are_not() {
    let warning = Diagnostic::warning(Span::default(), "w");
    let error = Diagnostic::error(Span::default(), "e");
    let removed = Diagnostic::removed(Span::default(), "r");
    assert!(removed.is_removed());
    assert_eq!(removed.severity, Severity::Error);
    assert!(!warning.is_removed());
    assert!(!error.is_removed());
    assert_eq!(warning.code, None);
}

#[test]
fn removed_form_is_an_error_so_step_stops_on_it() {
    // `step` keeps only the errors of `Project::check`. A removed form must be an error for that to hold.
    let project = project_of(&[("ci/fix", FIX), ("main", &main_with("", "return @./ci/fix;"))]);
    let errors: Vec<_> = project.check()["main"]
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .cloned()
        .collect();
    assert!(errors.iter().any(|d| d.is_removed()), "{errors:?}");
}
