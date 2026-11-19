use std::path::PathBuf;

use super::*;
use crate::program_fixture::with_main;

const FIX: &str = "export default function fix() {\n  return `ok`;\n}\n";
const MESSAGE: &str = "export default interface Message {\n  /** Opening. */\n  opening: string;\n}\n";

/// A project with `main.rein` as `main` and each `(name, text)` as one more file. Names are project paths.
fn project(main: &str, others: &[(&str, &str)]) -> Project {
    let mut sources = vec![(PathBuf::from("/p/main.rein"), main.to_string())];
    for (name, text) in others {
        sources.push((PathBuf::from(format!("/p/{name}.rein")), text.to_string()));
    }
    Project::from_sources(PathBuf::from("/p"), sources)
}

fn migrate(main: &str, others: &[(&str, &str)]) -> Migration {
    migrate_text(&project(main, others), "main", main)
}

#[test]
fn function_reference_becomes_a_named_import_above_the_doc() {
    let main = "/** Go. */\nexport default function main() {\n  return @./ci/fix;\n}\n";
    let out = migrate(main, &[("ci/fix", FIX)]);
    assert_eq!(
        out.text,
        "import { fix } from \"./ci/fix\";\n\n/** Go. */\nexport default function main() {\n  return fix;\n}\n"
    );
    assert!(out.warnings.is_empty());
}

#[test]
fn struct_reference_names_the_struct_in_a_parameter_type() {
    let main = "export default function tell({ opening }: @./message) {\n  return opening;\n}\n";
    let out = migrate(main, &[("message.struct", MESSAGE)]);
    assert_eq!(
        out.text,
        "import { Message } from \"./message.struct\";\n\nexport default function tell({ opening }: Message) {\n  return opening;\n}\n"
    );
}

#[test]
fn default_import_keeps_the_declared_name_and_renames_its_uses() {
    let main = "import ciFix from \"./ci/fix\";\n\n/** Go. */\nexport default function main() {\n  return ciFix;\n}\n";
    let out = migrate(main, &[("ci/fix", FIX)]);
    assert_eq!(
        out.text,
        "import { fix } from \"./ci/fix\";\n\n/** Go. */\nexport default function main() {\n  return fix;\n}\n"
    );
}

#[test]
fn a_clashing_name_gets_an_alias_from_the_path() {
    let main = "export default function main() {\n  return @./a/fix;\n  return @./b/fix;\n}\n";
    let out = migrate(main, &[("a/fix", FIX), ("b/fix", FIX)]);
    assert_eq!(
        out.text,
        "import { fix } from \"./a/fix\";\nimport { fix as bFix } from \"./b/fix\";\n\nexport default function main() {\n  return fix;\n  return bFix;\n}\n"
    );
}

#[test]
fn a_reference_that_clashes_with_the_own_function_gets_an_alias() {
    let main = "export default function fix() {\n  return @./ci/fix;\n}\n";
    let out = migrate(main, &[("ci/fix", FIX)]);
    assert_eq!(
        out.text,
        "import { fix as ciFix } from \"./ci/fix\";\n\nexport default function fix() {\n  return ciFix;\n}\n"
    );
}

#[test]
fn strings_and_comments_keep_their_at_signs() {
    let main = "// see @./ci/fix\n/** Go. */\nexport default function main() {\n  return \"@./ci/fix\";\n}\n";
    let out = migrate(main, &[("ci/fix", FIX)]);
    assert_eq!(out.text, main);
    assert!(out.warnings.is_empty());
}

#[test]
fn a_second_run_changes_nothing() {
    let main = "/** Go. */\nexport default function main() {\n  return @./ci/fix;\n}\n";
    let others = [("ci/fix", FIX)];
    let once = migrate(main, &others);
    let twice = migrate(&once.text, &others);
    assert_eq!(twice.text, once.text);
}

#[test]
fn a_named_import_that_exists_is_reused_without_a_new_line() {
    let main =
        "import { fix } from \"./ci/fix\";\n\n/** Go. */\nexport default function main() {\n  return @./ci/fix;\n}\n";
    let out = migrate(main, &[("ci/fix", FIX)]);
    assert_eq!(
        out.text,
        "import { fix } from \"./ci/fix\";\n\n/** Go. */\nexport default function main() {\n  return fix;\n}\n"
    );
}

#[test]
fn a_new_import_goes_after_the_existing_imports() {
    let main =
        "import { other } from \"./other\";\n\n/** Go. */\nexport default function main() {\n  return @./ci/fix;\n}\n";
    let out = migrate(main, &[("ci/fix", FIX)]);
    assert_eq!(
        out.text,
        "import { other } from \"./other\";\nimport { fix } from \"./ci/fix\";\n\n/** Go. */\nexport default function main() {\n  return fix;\n}\n"
    );
}

#[test]
fn an_unresolved_reference_stays_with_a_warning() {
    let main = "export default function main() {\n  return @./missing;\n}\n";
    let out = migrate(main, &[]);
    assert_eq!(out.text, main);
    assert_eq!(out.warnings.len(), 1);
    assert!(
        out.warnings[0].message.contains("`@./missing` is not migrated"),
        "{:?}",
        out.warnings
    );
}

#[test]
fn a_program_reference_imports_the_entry_by_its_declared_name() {
    let main = "export default function main() {\n  return @pr-handling(pr=pr);\n}\n";
    let project = with_main(main.to_string());
    let out = migrate_text(&project, "main", main);
    assert_eq!(
        out.text,
        "import { main as prHandling } from \"pr-handling\";\n\nexport default function main() {\n  return prHandling(pr=pr);\n}\n"
    );
}

#[test]
fn a_bad_import_line_leaves_the_file_alone() {
    let main = "import * as all from \"./ci/fix\";\nexport default function main() {\n  return @./ci/fix;\n}\n";
    let out = migrate(main, &[("ci/fix", FIX)]);
    assert_eq!(out.text, main);
    assert_eq!(out.warnings.len(), 1);
}
