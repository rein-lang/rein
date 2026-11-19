use super::*;
use crate::diagnostic::{Severity, DEPRECATED_CODE};
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

/// The deprecation warnings of `main`, by message.
fn deprecations(project: &Project) -> Vec<String> {
    project.check()["main"]
        .iter()
        .filter(|d| d.is_deprecated())
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn removal_date_is_one_constant() {
    assert_eq!(IMPORT_REMOVAL_DATE, "2026-12-19");
}

#[test]
fn at_reference_to_a_function_gets_the_named_import_suggestion() {
    let project = project_of(&[("ci/fix", FIX), ("main", &main_with("", "return @./ci/fix;"))]);
    assert_eq!(
        deprecations(&project),
        vec![
            "`@./ci/fix` is deprecated and becomes an error on 2026-12-19; write `import { fix } from \"./ci/fix\";` and use `fix`. Run `rein migrate imports` to change every file"
        ]
    );
}

#[test]
fn at_reference_warning_is_a_warning_with_the_deprecated_code() {
    let project = project_of(&[("ci/fix", FIX), ("main", &main_with("", "return @./ci/fix;"))]);
    let diag = project.check()["main"]
        .iter()
        .find(|d| d.is_deprecated())
        .cloned()
        .unwrap();
    assert_eq!(diag.severity, Severity::Warning);
    assert_eq!(diag.code, Some(DEPRECATED_CODE));
}

#[test]
fn suggested_name_is_the_declared_name_not_the_last_segment() {
    let fix_ci = "/** Fix CI. */\nexport default function fixCi() {\n  return `Fix.`;\n}\n";
    let project = project_of(&[("ci/fix", fix_ci), ("main", &main_with("", "return @./ci/fix;"))]);
    let msgs = deprecations(&project);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(
        msgs[0].contains("write `import { fixCi } from \"./ci/fix\";` and use `fixCi`"),
        "{msgs:?}"
    );
}

#[test]
fn unresolved_reference_suggests_the_last_path_segment() {
    let project = project_of(&[("main", &main_with("", "return @./ci/missing;"))]);
    let msgs = deprecations(&project);
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
        .filter(|d| d.is_deprecated())
        .map(|d| d.message.clone())
        .collect();
    assert!(msgs[0].contains("write `import { fix } from \"../fix\";`"), "{msgs:?}");
}

#[test]
fn reference_in_a_call_gets_a_warning() {
    let tpl = "/** Template. */\nexport default function tpl(a: string) {\n  return `${a}`;\n}\n";
    let project = project_of(&[("tpl", tpl), ("main", &main_with("", "return @./tpl(a=\"x\");"))]);
    assert_eq!(deprecations(&project).len(), 1);
}

#[test]
fn await_of_a_reference_gets_a_warning() {
    let tpl = "/** Template. */\nexport default function tpl(a: string) {\n  return `${a}`;\n}\n";
    let project = project_of(&[
        ("tpl", tpl),
        ("main", "/** Entry. */\nexport default async function main() {\n  /** Pick? */\n  const x: string = await @./tpl(a=\"x\");\n  return `${x}`;\n}\n"),
    ]);
    assert_eq!(deprecations(&project).len(), 1);
}

#[test]
fn struct_reference_suggests_a_struct_import_with_the_declared_name() {
    let message = "/** A message. */\nexport default interface Msg {\n  /** Opening? */\n  opening: string;\n}\n";
    let main = "/** Entry. */\nexport default function main({ opening }: @./message) {\n  return `${opening}`;\n}\n";
    let project = project_of(&[("message.struct", message), ("main", main)]);
    let msgs = deprecations(&project);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(msgs[0].starts_with("`@./message` is deprecated"), "{msgs:?}");
    assert!(
        msgs[0].contains("write `import { Msg } from \"./message.struct\";` and use `Msg`"),
        "{msgs:?}"
    );
}

#[test]
fn unresolved_struct_reference_capitalizes_the_last_segment() {
    let main = "/** Entry. */\nexport default function main({ opening }: @./ticket) {\n  return `${opening}`;\n}\n";
    let project = project_of(&[("main", main)]);
    let msgs = deprecations(&project);
    assert!(
        msgs[0].contains("write `import { Ticket } from \"./ticket.struct\";`"),
        "{msgs:?}"
    );
}

#[test]
fn program_reference_suggests_the_camel_cased_program_name() {
    let project = project_of(&[("main", &main_with("", "return @pr-handling;"))]);
    let msgs = deprecations(&project);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(
        msgs[0].contains("write `import { prHandling } from \"pr-handling\";` and use `prHandling`"),
        "{msgs:?}"
    );
}

#[test]
fn named_import_use_is_not_a_deprecation() {
    let project = project_of(&[
        ("ci/fix", FIX),
        ("main", &main_with("import { fix } from \"./ci/fix\";", "return fix;")),
    ]);
    assert!(deprecations(&project).is_empty(), "{:?}", project.check()["main"]);
}

#[test]
fn at_inside_a_comment_or_doc_is_not_a_reference() {
    let main =
        "/** See @./ci/fix for the old form. */\nexport default function main() {\n  // @./ci/fix\n  return `go`;\n}\n";
    let project = project_of(&[("ci/fix", FIX), ("main", main)]);
    assert!(deprecations(&project).is_empty(), "{:?}", project.check()["main"]);
}

#[test]
fn default_import_gets_the_named_import_warning() {
    let project = project_of(&[
        ("ci/fix", FIX),
        ("main", &main_with("import fix from \"./ci/fix\";", "return fix;")),
    ]);
    assert_eq!(
        deprecations(&project),
        vec![
            "default imports are deprecated and become an error on 2026-12-19; write `import { fix } from \"./ci/fix\"`"
        ]
    );
}

#[test]
fn default_import_suggests_the_declared_name() {
    let fix_ci = "/** Fix CI. */\nexport default function fixCi() {\n  return `Fix.`;\n}\n";
    let project = project_of(&[
        ("ci/fix", fix_ci),
        ("main", &main_with("import fix from \"./ci/fix\";", "return fix;")),
    ]);
    let msgs = deprecations(&project);
    assert!(
        msgs[0].contains("write `import { fixCi } from \"./ci/fix\"`"),
        "{msgs:?}"
    );
}

#[test]
fn deprecated_diagnostic_is_a_warning_and_other_diagnostics_are_not() {
    let warning = Diagnostic::warning(Span::default(), "w");
    let error = Diagnostic::error(Span::default(), "e");
    let deprecated = Diagnostic::deprecated(Span::default(), "d");
    assert!(deprecated.is_deprecated());
    assert_eq!(deprecated.severity, Severity::Warning);
    assert!(!warning.is_deprecated());
    assert!(!error.is_deprecated());
    assert_eq!(warning.code, None);
}

#[test]
fn deprecation_is_not_an_error_so_step_leaves_it_out() {
    // `step` keeps only the errors of `Project::check`. A deprecation must stay a warning for that to hold.
    let project = project_of(&[("ci/fix", FIX), ("main", &main_with("", "return @./ci/fix;"))]);
    let errors: Vec<_> = project.check()["main"]
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .cloned()
        .collect();
    assert!(errors.iter().all(|d| !d.is_deprecated()), "{errors:?}");
}
