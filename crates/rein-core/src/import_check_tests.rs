use super::*;
use crate::diagnostic::Severity;
use crate::program_fixture::with_main;
use crate::step::Answers;
use crate::step::{step, StepRequest, StepResponse, PROTOCOL_VERSION};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn has(diags: &[Diagnostic], severity: Severity, part: &str) -> bool {
    diags.iter().any(|d| d.severity == severity && d.message.contains(part))
}

/// A project of `main` plus the given (name, source) files, all in `/proj`.
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

#[test]
fn wrong_name_message_names_the_declared_name() {
    let project = project_of(&[
        (
            "ci/fix",
            "/** Fix CI. */\nexport default function fixCi() {\n  return `Fix.`;\n}\n",
        ),
        ("main", &main_with("import { fix } from \"./ci/fix\";", "return fix;")),
    ]);
    let diags = project.check();
    assert!(
        has(
            &diags["main"],
            Severity::Error,
            "`./ci/fix` declares `fixCi`, not `fix`; write `import { fixCi } from \"./ci/fix\"`"
        ),
        "{:?}",
        diags["main"]
    );
}

#[test]
fn right_name_with_as_has_no_name_error() {
    let project = project_of(&[
        ("ci/fix", FIX),
        (
            "main",
            &main_with("import { fix as ciFix } from \"./ci/fix\";", "return ciFix;"),
        ),
    ]);
    let diags = project.check();
    assert!(!has(&diags["main"], Severity::Error, "declares"), "{:?}", diags["main"]);
}

#[test]
fn missing_target_is_an_error_at_the_import() {
    let project = project_of(&[("main", &main_with("import { fix } from \"./ci/nope\";", "return `x`;"))]);
    let diags = project.check();
    assert!(
        has(&diags["main"], Severity::Error, "cannot find function `@./ci/nope`"),
        "{:?}",
        diags["main"]
    );
}

#[test]
fn struct_import_must_name_the_declared_struct() {
    let project = project_of(&[
        ("message.struct", "/** A message. */\nexport default interface Msg {\n  /** Opening? */\n  opening: string;\n}\n"),
        (
            "main",
            "import { Message } from \"./message.struct\";\n/** Entry. */\nexport default function main({ opening }: Message) {\n  return `${opening}`;\n}\n",
        ),
    ]);
    let diags = project.check();
    assert!(
        has(
            &diags["main"],
            Severity::Error,
            "`./message.struct` declares `Msg`, not `Message`; write `import { Msg } from \"./message.struct\"`"
        ),
        "{:?}",
        diags["main"]
    );
}

#[test]
fn struct_import_with_the_declared_name_is_clean() {
    let project = project_of(&[
        ("message.struct", "/** A message. */\nexport default interface Message {\n  /** Opening? */\n  opening: string;\n}\n"),
        (
            "main",
            "import { Message } from \"./message.struct\";\n/** Entry. */\nexport default function main({ opening }: Message) {\n  return `${opening}`;\n}\n",
        ),
    ]);
    let diags = project.check();
    assert!(!has(&diags["main"], Severity::Error, "declares"), "{:?}", diags["main"]);
    assert!(
        diags["main"].iter().all(|d| d.severity != Severity::Error),
        "{:?}",
        diags["main"]
    );
}

#[test]
fn unused_import_is_a_warning() {
    let project = project_of(&[
        ("ci/fix", FIX),
        ("main", &main_with("import { fix } from \"./ci/fix\";", "return `x`;")),
    ]);
    let diags = project.check();
    assert!(
        has(&diags["main"], Severity::Warning, "`fix` is imported but not used"),
        "{:?}",
        diags["main"]
    );
    assert!(!has(&diags["main"], Severity::Error, "imported but not used"));
}

#[test]
fn as_with_the_same_name_is_a_warning() {
    let project = project_of(&[
        ("ci/fix", FIX),
        (
            "main",
            &main_with("import { fix as fix } from \"./ci/fix\";", "return fix;"),
        ),
    ]);
    let diags = project.check();
    assert!(
        has(
            &diags["main"],
            Severity::Warning,
            "`fix as fix` keeps the name; write `import { fix } from \"./ci/fix\"`"
        ),
        "{:?}",
        diags["main"]
    );
}

#[test]
fn default_import_with_the_declared_name_is_an_error_for_the_removal() {
    let project = project_of(&[
        ("ci/fix", FIX),
        ("main", &main_with("import fix from \"./ci/fix\";", "return fix;")),
    ]);
    let diags = project.check();
    // Rein 0.2.0 removed default imports. Their only diagnostic is the removal error.
    let removals: Vec<_> = diags["main"].iter().filter(|d| d.is_removed()).collect();
    assert_eq!(removals.len(), 1, "{:?}", diags["main"]);
    assert_eq!(removals[0].severity, Severity::Error);
    assert!(
        !has(&diags["main"], Severity::Warning, "is imported but not used"),
        "{:?}",
        diags["main"]
    );
}

#[test]
fn default_import_with_another_declared_name_is_an_error() {
    let project = project_of(&[
        (
            "ci/fix",
            "/** d */\nexport default function fixCi() {\n  return `x`;\n}\n",
        ),
        ("main", &main_with("import fix from \"./ci/fix\";", "return fix;")),
    ]);
    let diags = project.check();
    assert!(
        has(&diags["main"], Severity::Error, "declares `fixCi`, not `fix`"),
        "{:?}",
        diags["main"]
    );
}

#[test]
fn program_import_must_name_the_entry_function() {
    let ok = with_main(program_main(
        "import { main as handle } from \"pr-handling\";",
        "return handle(pr=pr);",
    ));
    let diags = ok.check();
    assert!(
        diags["main"].iter().all(|d| d.severity != Severity::Error),
        "{:?}",
        diags["main"]
    );

    let bad = with_main(program_main(
        "import { start } from \"pr-handling\";",
        "return start(pr=pr);",
    ));
    let diags = bad.check();
    assert!(
        has(
            &diags["main"],
            Severity::Error,
            "`pr-handling` declares `main`, not `start`"
        ),
        "{:?}",
        diags["main"]
    );
}

/// The removed `@` form of the project: every hand-off written with `@`. Each one is an error now.
fn by_reference() -> Vec<(&'static str, String)> {
    vec![
        (
            "main",
            "/** Entry. */\nexport default function main(\n  /** Which path? */\n  path: \"fix\" | \"tpl\" | \"wait\",\n) {\n  switch (path) {\n    case \"fix\":\n      return @./ci/fix;\n    case \"tpl\":\n      return @./tpl(a=`x`);\n    case \"wait\":\n      return @./wait;\n  }\n}\n".into(),
        ),
        ("ci/fix", FIX.into()),
        ("tpl", "/** Template. */\nexport default function tpl(\n  /** A? */\n  a: string,\n) {\n  return `Result ${a}`;\n}\n".into()),
        ("merge", "/** Merge. */\nexport default function merge() {\n  return `Merge the PR.`;\n}\n".into()),
        (
            "wait",
            "/** Wait. */\nexport default async function wait() {\n  /** Merged? */\n  const merged: boolean = await @./merge;\n  if (merged) {\n    return `Merged.`;\n  }\n  return `Not merged.`;\n}\n".into(),
        ),
        ("message.struct", "/** A message. */\nexport default interface Message {\n  /** Opening? */\n  opening: string;\n  /** Next step? */\n  next_step: string;\n}\n".into()),
        ("tell", "/** Tell. */\nexport default function tell({ opening, next_step }: @./message) {\n  return `${opening} ${next_step}`;\n}\n".into()),
    ]
}

/// The same project, every hand-off written as an import.
fn by_import() -> Vec<(&'static str, String)> {
    vec![
        (
            "main",
            "import { fix } from \"./ci/fix\";\nimport { tpl } from \"./tpl\";\nimport { wait } from \"./wait\";\n\n/** Entry. */\nexport default function main(\n  /** Which path? */\n  path: \"fix\" | \"tpl\" | \"wait\",\n) {\n  switch (path) {\n    case \"fix\":\n      return fix;\n    case \"tpl\":\n      return tpl(a=`x`);\n    case \"wait\":\n      return wait;\n  }\n}\n".into(),
        ),
        ("ci/fix", FIX.into()),
        ("tpl", "/** Template. */\nexport default function tpl(\n  /** A? */\n  a: string,\n) {\n  return `Result ${a}`;\n}\n".into()),
        ("merge", "/** Merge. */\nexport default function merge() {\n  return `Merge the PR.`;\n}\n".into()),
        (
            "wait",
            "import { merge } from \"./merge\";\n\n/** Wait. */\nexport default async function wait() {\n  /** Merged? */\n  const merged: boolean = await merge;\n  if (merged) {\n    return `Merged.`;\n  }\n  return `Not merged.`;\n}\n".into(),
        ),
        ("message.struct", "/** A message. */\nexport default interface Message {\n  /** Opening? */\n  opening: string;\n  /** Next step? */\n  next_step: string;\n}\n".into()),
        ("tell", "import { Message } from \"./message.struct\";\n\n/** Tell. */\nexport default function tell({ opening, next_step }: Message) {\n  return `${opening} ${next_step}`;\n}\n".into()),
    ]
}

fn build(files: Vec<(&'static str, String)>) -> Project {
    let root = PathBuf::from("/proj");
    let sources = files
        .into_iter()
        .map(|(n, t)| (root.join(format!("{n}.rein")), t))
        .collect();
    Project::from_sources(root, sources)
}

fn answers(pairs: &[(&str, &str)]) -> Answers {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn step_of(project: &Project, pairs: &[(&str, &str)]) -> StepResponse {
    step(
        project,
        &StepRequest {
            version: PROTOCOL_VERSION,
            entry: "main".into(),
            answers: answers(pairs),
            flow: None,
            inputs: BTreeMap::new(),
        },
    )
}

#[test]
fn import_project_finishes_through_the_named_import() {
    let imports = build(by_import());
    // The `fix` path finishes through `ci/fix`, so the named import reaches the callee.
    let done = step_of(&imports, &[("main.path", "fix")]);
    assert!(
        matches!(&done, StepResponse::Done { trace, .. } if trace.iter().any(|t| t == "ci/fix")),
        "{done:?}"
    );
}

#[test]
fn import_of_a_struct_reads_its_fields() {
    let imports = build(by_import());
    let fields: Vec<String> = imports
        .get("tell")
        .unwrap()
        .function
        .inputs
        .iter()
        .map(|i| i.name.value.clone())
        .collect();
    assert_eq!(fields, vec!["opening".to_string(), "next_step".to_string()]);
}

#[test]
fn reference_form_is_refused_after_the_removal() {
    let refs = build(by_reference());
    assert!(refs.check()["main"].iter().any(|d| d.is_removed()), "{:?}", refs.check()["main"]);
    assert!(matches!(step_of(&refs, &[]), StepResponse::Error { .. }));
    let by_ref = with_main(program_main("", "return @pr-handling(pr=pr);"));
    assert!(by_ref.check()["main"].iter().any(|d| d.is_removed()), "{:?}", by_ref.check()["main"]);
}

/// A handler `main(pr)` with the given imports before it.
fn program_main(imports: &str, body: &str) -> String {
    format!("{imports}\n/** Handler. */\nexport default function main(\n  /** The PR link. */\n  pr: string,\n) {{\n  {body}\n}}\n")
}

#[test]
fn import_that_leaves_the_project_root_is_an_error_at_the_import() {
    let project = project_of(&[(
        "main",
        &main_with("import { x } from \"../../outside\";", "return `x`;"),
    )]);
    let diags = project.check();
    assert!(
        has(&diags["main"], Severity::Error, "leaves the project root"),
        "{:?}",
        diags["main"]
    );
}

const REVIEW: &str = "/** Review. */\nexport default function review() {\n  return `Review.`;\n}\n";
const INPUT: &str = "interface Input {\n  /** Review? */\n  review: string;\n}\n";

#[test]
fn import_named_like_a_destructured_field_is_an_error() {
    let main = format!(
        "import {{ review }} from \"./review\";\n{INPUT}\n/** Entry. */\nexport default function main({{ review }}: Input) {{\n  return review;\n}}\n"
    );
    let project = project_of(&[("review", REVIEW), ("main", &main)]);
    let diags = project.check();
    assert!(
        has(
            &diags["main"],
            Severity::Error,
            "import `review` clashes with the parameter `review`, which is an error; give the import another name, like `import { review as openReview } from \"./review\"`"
        ),
        "{:?}",
        diags["main"]
    );
    let clashes = diags["main"].iter().filter(|d| d.message.contains("clashes")).count();
    assert_eq!(clashes, 1, "{:?}", diags["main"]);
}

#[test]
fn import_named_like_a_scalar_parameter_is_an_error() {
    let main = "import { review } from \"./review\";\n\n/** Entry. */\nexport default function main(/** Review? */ review: string) {\n  return review;\n}\n";
    let project = project_of(&[("review", REVIEW), ("main", main)]);
    let diags = project.check();
    assert!(
        has(&diags["main"], Severity::Error, "import `review` clashes with the parameter `review`"),
        "{:?}",
        diags["main"]
    );
}

#[test]
fn import_named_like_a_const_is_an_error() {
    let main = "import { review } from \"./review\";\n\n/** Entry. */\nexport default async function main() {\n  /** Review? */\n  const review: boolean = await request(\"Review it.\");\n  if (review) {\n    return `yes`;\n  }\n  return `no`;\n}\n";
    let project = project_of(&[("review", REVIEW), ("main", main)]);
    let diags = project.check();
    assert!(
        has(&diags["main"], Severity::Error, "import `review` clashes with the const `review`"),
        "{:?}",
        diags["main"]
    );
}

#[test]
fn aliased_import_of_a_destructured_field_name_is_clean() {
    let main = format!(
        "import {{ review as openReview }} from \"./review\";\n{INPUT}\n/** Entry. */\nexport default function main({{ review }}: Input) {{\n  return openReview;\n}}\n"
    );
    let project = project_of(&[("review", REVIEW), ("main", &main)]);
    let diags = project.check();
    assert!(!has(&diags["main"], Severity::Error, "clashes"), "{:?}", diags["main"]);
    assert!(
        diags["main"].iter().all(|d| d.severity != Severity::Error),
        "{:?}",
        diags["main"]
    );
}
