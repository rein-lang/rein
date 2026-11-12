use super::*;

#[test]
fn utf16_columns_count_surrogate_pairs() {
    // "😀" is 4 bytes but 2 UTF-16 units.
    let text = "a😀 @x\n";
    let at = text.find('@').unwrap();
    assert_eq!(position(text, at), Position::new(0, 4));
    assert_eq!(offset(text, Position::new(0, 4)), at);
}

#[test]
fn utf16_column_resets_per_line() {
    let text = "é\nxé @y";
    let at = text.find('@').unwrap();
    assert_eq!(position(text, at), Position::new(1, 3));
    assert_eq!(offset(text, Position::new(1, 3)), at);
}

#[test]
fn token_at_finds_import_and_parameter_names() {
    let text = "return mergePr;\n  if (docsOnly) {\n";
    let at_import = text.find("Pr").unwrap() + 1;
    assert_eq!(token_at(text, at_import).unwrap(), "mergePr");
    let at_param = text.find("docsOnly").unwrap() + 2;
    assert_eq!(token_at(text, at_param).unwrap(), "docsOnly");
}

#[test]
fn token_inside_string_is_none() {
    let text = "import a from \"./ci/fix\";\n";
    let at = text.find("ci").unwrap() + 1;
    assert!(token_at(text, at).is_none());
    let text = "import a from './ci/fix';\n";
    let at = text.find("ci").unwrap() + 1;
    assert!(token_at(text, at).is_none());
}

#[test]
fn relative_import_paths() {
    assert_eq!(relative_import("ci/fix", "merge_pr"), "../merge_pr");
    assert_eq!(relative_import("ci/fix", "ci/deeper"), "./deeper");
    assert_eq!(relative_import("ci/sub/leaf", "top"), "../../top");
    assert_eq!(relative_import("main", "ci/fix"), "./ci/fix");
}
