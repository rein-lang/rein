use super::*;
use crate::diagnostic::Severity;
use std::time::{SystemTime, UNIX_EPOCH};

fn has(diags: &[Diagnostic], severity: Severity, part: &str) -> bool {
    diags
        .iter()
        .any(|d| d.severity == severity && d.message.contains(part))
}

/// Writes a small project into a unique temp dir and returns its path.
fn temp_project(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rein-core-{tag}-{}-{nanos}", std::process::id()));
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    write(
        "main.rein",
        "/** Entry. */\nexport default function main() {\n  return @./merge_pr;\n}\n",
    );
    write(
        "merge_pr.rein",
        "/** Merge a pull request. */\nexport default function mergePr(\n  /** CI status? */\n  ci: \"green\" | \"red\" | \"pending\",\n) {\n\
         switch (ci) {\n    case \"green\": return `merge`;\n    case \"red\": return @./ci/fix;\n    case \"pending\": return @./nope;\n}\n}\n",
    );
    write(
        "ci/fix.rein",
        "/** Fix red CI. */\nexport default function fixCi() {\n  return @../merge_pr;\n}\n",
    );
    write(
        "bad-name.rein",
        "/** x */\nexport default function bad() {\n  return `y`;\n}\n",
    );
    fs::create_dir_all(root.join("nested/deeper")).unwrap();
    root
}

#[test]
fn loads_checks_and_reports_cycles_and_unknown_imports() {
    let root = temp_project("load");
    let project = Project::load(&root).unwrap();
    let names: Vec<&str> = project.files.keys().map(String::as_str).collect();
    assert_eq!(names, ["bad-name", "ci/fix", "main", "merge_pr"]);

    let diags = project.check();
    let merge = &diags["merge_pr"];
    assert!(
        has(merge, Severity::Error, "cannot find function `@./nope`"),
        "{merge:?}"
    );
    assert!(
        has(
            merge,
            Severity::Warning,
            "reference cycle: merge_pr -> ci/fix -> merge_pr"
        ),
        "{merge:?}"
    );
    let fix = &diags["ci/fix"];
    assert!(
        has(
            fix,
            Severity::Warning,
            "reference cycle: ci/fix -> merge_pr -> ci/fix"
        ),
        "{fix:?}"
    );
    assert!(has(
        &diags["bad-name"],
        Severity::Error,
        "invalid function file name"
    ));
    assert!(has(
        &diags["bad-name"],
        Severity::Warning,
        "`bad-name` is not reachable from main"
    ));
    assert!(!has(fix, Severity::Warning, "not reachable"), "{fix:?}");
    assert!(project.project_diagnostics().is_empty());

    assert_eq!(project.references().len(), 3);
    fs::remove_dir_all(&root).unwrap();
}

/// Diagnostics for one in-memory file named `f`, in a project with a valid `main`.
fn check_one(src: &str) -> Vec<Diagnostic> {
    let root = PathBuf::from("/proj");
    let project = Project::from_sources(
        root.clone(),
        vec![
            (
                root.join("main.rein"),
                "/** m */\nexport default function main() { return @./f }\n".to_string(),
            ),
            (root.join("f.rein"), src.to_string()),
        ],
    );
    project.check().remove("f").unwrap()
}

#[test]
fn template_only_used_in_a_nested_call_is_bound_and_reachable() {
    let root = PathBuf::from("/proj");
    let project = Project::from_sources(
        root.clone(),
        vec![
            (root.join("main.rein"), "/** m */\nexport default function main() { return delegate(model=@./tpl(a=`x`), task=`task`) }\n".to_string()),
            (root.join("tpl.rein"), "/** t */\nexport default function tpl(\n  /** A */ a: string,\n) { return `Hi ${a}` }\n".to_string()),
        ],
    );
    assert!(project.bindings("main").contains_key("./tpl"));
    let diags = project.check();
    assert!(
        !has(&diags["tpl"], Severity::Warning, "not reachable"),
        "{:?}",
        diags["tpl"]
    );
}

#[test]
fn syntax_error_reports_only_the_syntax_error() {
    let diags = check_one("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return `1` } return )");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].severity, Severity::Error);
}

#[test]
fn lex_error_reports_only_the_syntax_error() {
    let diags = check_one("/** abc");
    assert_eq!(diags.len(), 1, "{diags:?}");
}

#[test]
fn syntax_error_file_gets_no_cycle_warning() {
    let root = PathBuf::from("/proj");
    let project = Project::from_sources(
        root.clone(),
        vec![
            (
                root.join("main.rein"),
                "/** m */\nexport default function main() { return @./f }\n".to_string(),
            ),
            (
                root.join("f.rein"),
                "/** d */\nexport default function f() { return @./g }\nreturn )".to_string(),
            ),
            (
                root.join("g.rein"),
                "/** d */\nexport default function g() { return @./f }\n".to_string(),
            ),
        ],
    );
    let diags = project.check();
    assert_eq!(diags["f"].len(), 1, "{:?}", diags["f"]);
}

#[test]
fn unreadable_file_is_kept_with_read_error() {
    let root = temp_project("badutf8");
    fs::write(root.join("broken.rein"), [0xff, 0xfe, 0x00]).unwrap();
    let project = Project::load(&root).unwrap();
    let sf = project
        .get("broken")
        .expect("unreadable file is still loaded");
    assert!(sf.text.is_empty());
    assert_eq!(sf.parse_diagnostics.len(), 1, "{:?}", sf.parse_diagnostics);
    assert!(sf.parse_diagnostics[0]
        .message
        .starts_with("cannot read file: "));
    assert_eq!(sf.parse_diagnostics[0].span, Span::default());
    assert_eq!(project.check()["broken"].len(), 1);
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn follows_symlinks_and_survives_symlink_loops() {
    let root = temp_project("links");
    fs::write(
        root.join("real.rein"),
        "/** d */\nexport default function real() { return `x` }\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(root.join("real.rein"), root.join("alias.rein")).unwrap();
    std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
    let project = Project::load(&root).unwrap();
    assert!(project.get("alias").is_some());
    assert!(project.get("real").is_some());
    assert!(project.files.keys().all(|k| !k.starts_with("loop/")));
    fs::remove_dir_all(&root).unwrap();
}

/// A project of `main` plus the given (name, source) files, all in `/proj`.
fn project_of(files: &[(&str, &str)]) -> Project {
    let root = PathBuf::from("/proj");
    let mut sources = vec![(
        root.join("main.rein"),
        "/** m */\nexport default function main() { return `m` }\n".to_string(),
    )];
    sources.extend(
        files
            .iter()
            .map(|(n, t)| (root.join(format!("{n}.rein")), t.to_string())),
    );
    Project::from_sources(root, sources)
}

#[test]
fn cycle_warns_on_every_return_on_the_cycle() {
    let project = project_of(&[
        (
            "main",
            "/** m */\nexport default function main() { return @./a }\n",
        ),
        (
            "a",
            "/** d */\nexport default function a() { return @./b }\n",
        ),
        (
            "b",
            "/** d */\nexport default function b() { return @./c }\n",
        ),
        (
            "c",
            "/** d */\nexport default function c() { return @./a }\n",
        ),
        (
            "self",
            "/** d */\nexport default function me() { return @./self }\n",
        ),
    ]);
    let diags = project.check();
    for name in ["a", "b", "c"] {
        assert!(
            has(&diags[name], Severity::Warning, "reference cycle"),
            "{name}: {:?}",
            diags[name]
        );
    }
    let cycle_count: usize = ["a", "b", "c"]
        .iter()
        .map(|n| {
            diags[*n]
                .iter()
                .filter(|d| d.message.contains("reference cycle"))
                .count()
        })
        .sum();
    assert_eq!(cycle_count, 3);
    assert!(
        has(
            &diags["self"],
            Severity::Warning,
            "reference cycle: self -> self"
        ),
        "{:?}",
        diags["self"]
    );
}

#[test]
fn missing_main_is_a_project_error() {
    let root = PathBuf::from("/proj");
    let project = Project::from_sources(
        root.clone(),
        vec![(
            root.join("f.rein"),
            "/** d */\nexport default function f() { return `x` }".into(),
        )],
    );
    let errs = project.project_diagnostics();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].severity, Severity::Error);
    assert_eq!(
        errs[0].message,
        "project has no main.rein; it is the agent's entry point"
    );
    assert_eq!(project.marker_path(), root.join("main.rein"));
    // Without main, nothing is reported as unreachable.
    assert!(!has(
        &project.check()["f"],
        Severity::Warning,
        "not reachable"
    ));
}

#[test]
fn unreachable_function_is_a_warning() {
    let project = project_of(&[
        (
            "main",
            "/** m */\nexport default function main() { return @./used }\n",
        ),
        (
            "used",
            "/** u */\nexport default function used() { return @./deep/leaf }\n",
        ),
        (
            "deep/leaf",
            "/** l */\nexport default function leaf() { return `ok` }\n",
        ),
        (
            "orphan",
            "/** o */\nexport default function orphan() { return `alone` }\n",
        ),
    ]);
    let diags = project.check();
    assert!(has(
        &diags["orphan"],
        Severity::Warning,
        "`orphan` is not reachable from main"
    ));
    assert!(!has(
        &diags["deep/leaf"],
        Severity::Warning,
        "not reachable"
    ));
    assert!(!has(&diags["used"], Severity::Warning, "not reachable"));
    assert!(project.project_diagnostics().is_empty());
}

#[test]
fn main_with_syntax_error_skips_reachability() {
    let root = PathBuf::from("/proj");
    let project = Project::from_sources(
        root.clone(),
        vec![
            (
                root.join("main.rein"),
                "/** m */\nexport default function main() { return )".to_string(),
            ),
            (
                root.join("orphan.rein"),
                "/** o */\nexport default function orphan() { return `alone` }\n".to_string(),
            ),
        ],
    );
    assert!(!has(
        &project.check()["orphan"],
        Severity::Warning,
        "not reachable"
    ));
}

#[test]
fn import_paths_resolve_relative_to_the_importing_file() {
    let project = project_of(&[
        ("ci/fix", ""),
        ("ci/deeper", ""),
        ("merge_pr", ""),
        ("ci/sub/leaf", ""),
        ("top", ""),
    ]);
    assert_eq!(
        project.resolve("ci/fix", "../merge_pr").unwrap(),
        "merge_pr"
    );
    assert_eq!(project.resolve("ci/fix", "./deeper").unwrap(), "ci/deeper");
    assert_eq!(project.resolve("ci/sub/leaf", "../../top").unwrap(), "top");
    assert_eq!(
        project.resolve("ci/sub/leaf", "./../deeper").unwrap(),
        "ci/deeper"
    );
    assert_eq!(project.resolve("main", "./top").unwrap(), "top");
}

#[test]
fn import_path_errors() {
    let project = project_of(&[("ci/fix", ""), ("top", "")]);
    assert_eq!(
        project.resolve("ci/fix", "./missing").unwrap_err(),
        "cannot find function `@./missing`"
    );
    assert_eq!(
        project.resolve("ci/fix", "top").unwrap_err(),
        "references are relative: write `@./top`"
    );
    assert_eq!(
        project.resolve("ci/fix", "./top.rein").unwrap_err(),
        "references have no extension"
    );
    assert_eq!(
        project.resolve("ci/fix", "./top.js").unwrap_err(),
        "references have no extension"
    );
    // From ci/fix the directory is `ci`; two steps up leaves the root.
    assert_eq!(
        project.resolve("ci/fix", "../../top").unwrap_err(),
        "`@../../top` leaves the project root"
    );
    assert_eq!(
        project.resolve("top", "../top").unwrap_err(),
        "`@../top` leaves the project root"
    );
}

#[test]
fn import_outside_root_is_reported_by_the_checker() {
    let project = project_of(&[(
        "ci/fix",
        "/** d */\nexport default function fix() { return @../../outside }\n",
    )]);
    let diags = project.check();
    assert!(
        has(&diags["ci/fix"], Severity::Error, "leaves the project root"),
        "{:?}",
        diags["ci/fix"]
    );
}

#[test]
fn subdirectory_with_main_is_another_flow() {
    let root = temp_project("nested");
    fs::write(
        root.join("nested/main.rein"),
        "/** n */\nexport default function n() { return `n` }\n",
    )
    .unwrap();
    fs::write(
        root.join("nested/x.rein"),
        "/** x */\nexport default function x() { return `x` }\n",
    )
    .unwrap();
    let project = Project::load(&root).unwrap();
    assert!(
        project.get("nested/x").is_none(),
        "{:?}",
        project.files.keys().collect::<Vec<_>>()
    );
    assert!(project.get("ci/fix").is_some());
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn find_root_walks_from_the_link_not_its_target() {
    let a = temp_project("link-a");
    let b = temp_project("link-b");
    std::os::unix::fs::symlink(b.join("ci/fix.rein"), a.join("link.rein")).unwrap();
    assert_eq!(Project::find_root(&a.join("link.rein")), a);
    fs::remove_dir_all(&a).unwrap();
    fs::remove_dir_all(&b).unwrap();
}

#[test]
fn find_root_walks_up_to_main() {
    let root = fs::canonicalize(temp_project("root")).unwrap();
    assert_eq!(Project::find_root(&root.join("nested/deeper")), root);
    assert_eq!(Project::find_root(&root.join("ci/fix.rein")), root);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn template_called_from_main_is_reachable_and_drawn() {
    let project = project_of(&[
        ("main", "/** m */\nexport default function main(\n  /** C */ c: \"x\" | \"y\",\n) {\n  if (c === \"x\") { return @./tpl(a=`a`, b=`b`) } return @./tpl(b=`d`, a=`c`) \n}\n"),
        ("tpl", "/** t */\nexport default function tpl(\n  /** A */ a: string,\n  /** B */ b: string,\n) { return `${a} ${b}` }\n"),
    ]);
    let diags = project.check();
    assert!(
        !has(&diags["tpl"], Severity::Warning, "not reachable"),
        "{:?}",
        diags["tpl"]
    );
    assert!(project
        .references()
        .contains(&("main".to_string(), "tpl".to_string())));
}

#[test]
fn a_shared_struct_binds_in_every_function_that_names_it() {
    let ticket = "/** A ticket. */\nexport default interface Ticket {\n  /** Title. */\n  title: string;\n  /** VIP? */\n  vip: boolean;\n}\n";
    let a = "/** A. */\nexport default function a({ title, vip }: @./ticket) {\n  if (vip) { return `VIP ${title}` }\n  return `Plain ${title}`\n}\n";
    let b = "/** B. */\nexport default function b({ vip, title }: @../ticket) {\n  if (vip) { return `x` }\n  return @../a(title=title)\n}\n";
    let m = "/** m */\nexport default function main() { return @./sub/b(title=`t`) }\n";
    let project = project_of(&[
        ("ticket.struct", ticket),
        ("a", a),
        ("sub/b", b),
        ("main", m),
    ]);

    let input_names = |name: &str| -> Vec<String> {
        project
            .get(name)
            .unwrap()
            .function
            .inputs
            .iter()
            .map(|i| i.name.value.clone())
            .collect()
    };
    assert_eq!(input_names("a"), ["title", "vip"]);
    assert_eq!(input_names("sub/b"), ["title", "vip"]);

    let diags = project.check();
    for (name, ds) in &diags {
        assert!(
            !ds.iter().any(|d| d.severity == Severity::Error),
            "{name} has an error: {ds:?}"
        );
    }
    assert!(
        !has(&diags["ticket.struct"], Severity::Warning, "not reachable"),
        "{:?}",
        diags["ticket.struct"]
    );

    let fns: Vec<&str> = project.functions().map(|(n, _)| n.as_str()).collect();
    assert!(fns.contains(&"a"), "{fns:?}");
    assert!(!fns.contains(&"ticket.struct"), "{fns:?}");
}

#[test]
fn a_missing_shared_struct_is_an_error() {
    let project = project_of(&[(
        "f",
        "/** f */\nexport default function f({ x }: @./nope) { return `x` }\n",
    )]);
    assert!(has(
        &project.check()["f"],
        Severity::Error,
        "cannot find struct `@./nope`; a shared struct is a `nope.struct.rein` file"
    ));
}

/// A function file of exactly `lines` lines: the `export default` line, comment lines, a return, and `}`.
fn function_of(lines: usize) -> String {
    let mut text = String::from("/** Long. */\nexport default function long() {\n");
    for _ in 0..lines - 3 {
        text.push_str("  // a step\n");
    }
    text.push_str("  return `done`;\n}\n");
    text
}

/// Line-limit errors of the function `long` in a project of `main` and `long`.
fn length_errors(text: &str) -> Vec<Diagnostic> {
    project_of(&[("long", text)])
        .check()
        .remove("long")
        .unwrap()
        .into_iter()
        .filter(|d| d.message.contains(" lines; the limit is "))
        .collect()
}

#[test]
fn exactly_40_lines_passes_and_the_doc_comment_does_not_count() {
    assert!(length_errors(&function_of(40)).is_empty());
    let with_doc = format!("/**\n * One.\n * Two.\n * Three.\n */\n{}", function_of(40));
    assert!(length_errors(&with_doc).is_empty());
}

#[test]
fn forty_one_lines_fails_on_the_header() {
    let text = function_of(41);
    let errors = length_errors(&text);
    assert_eq!(errors.len(), 1, "{errors:?}");
    let d = &errors[0];
    assert_eq!(d.severity, Severity::Error);
    assert_eq!(
        d.message,
        "function `long` has 41 lines; the limit is 40. Move part of it into its own function and call it with `@./name`"
    );
    assert_eq!(
        &text[d.span.start..d.span.end],
        "export default function long"
    );
}

/// A flow root in a temp dir: `main.rein`, the given `long.rein`, and an optional `rein.json`.
fn flow_of(tag: &str, manifest: Option<&str>, long: &str) -> Project {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rein-core-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("main.rein"),
        "/** m */\nexport default function main() { return @./long }\n",
    )
    .unwrap();
    fs::write(root.join("long.rein"), long).unwrap();
    if let Some(m) = manifest {
        fs::write(root.join("rein.json"), m).unwrap();
    }
    let project = Project::load(&root).unwrap();
    fs::remove_dir_all(&root).unwrap();
    project
}

/// A flow root in a temp dir: `main.rein`, a `long.rein` of `lines` lines, and an optional `rein.json`.
fn flow_with_manifest(tag: &str, manifest: Option<&str>, lines: usize) -> Project {
    flow_of(tag, manifest, &function_of(lines))
}

#[test]
fn manifest_sets_the_line_limit() {
    let project = flow_with_manifest("limit-ok", Some(r#"{"maxFunctionLines": 60}"#), 41);
    assert_eq!(project.max_function_lines, 60);
    assert!(project.manifest_diagnostics().is_empty());
    assert!(length_lines(&project).is_empty());

    let project = flow_with_manifest("limit-over", Some(r#"{"maxFunctionLines": 60}"#), 61);
    let errors = length_lines(&project);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("function `long` has 61 lines; the limit is 60."),
        "{errors:?}"
    );
}

#[test]
fn no_manifest_uses_the_default_limit() {
    let project = flow_with_manifest("limit-none", None, 41);
    assert_eq!(project.max_function_lines, 40);
    assert_eq!(length_lines(&project).len(), 1);
}

#[test]
fn invalid_line_limit_is_a_manifest_error_and_the_default_applies() {
    for value in ["0", "-1", "1.5", "\"60\"", "null"] {
        let manifest = format!(r#"{{"maxFunctionLines": {value}}}"#);
        let project = flow_with_manifest("limit-bad", Some(&manifest), 41);
        assert_eq!(project.max_function_lines, 40, "{value}");
        let errors = project.manifest_diagnostics();
        assert_eq!(errors.len(), 1, "{value}: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Error);
        assert!(
            errors[0].message.starts_with(
                "`maxFunctionLines` in rein.json must be a positive integer, such as 40; found "
            ),
            "{value}: {}",
            errors[0].message
        );
    }
}

/// The line-limit error texts of the project's functions.
fn length_lines(project: &Project) -> Vec<String> {
    project
        .check()
        .into_values()
        .flatten()
        .map(|d| d.message)
        .filter(|m| m.contains(" lines; the limit is "))
        .collect()
}

#[test]
fn struct_files_are_not_checked_for_length() {
    let mut text = String::from("/** Ticket. */\nexport default interface Ticket {\n  /** Category? */\n  category: boolean;\n");
    text.push_str(&"\n".repeat(60));
    text.push_str("}\n");
    let project = project_of(&[("ticket.struct", text.as_str())]);
    assert!(!project.check()["ticket.struct"]
        .iter()
        .any(|d| d.message.contains(" lines; the limit is ")));
}

fn errors(project: &Project) -> Vec<String> {
    let all = &project.check()["main"];
    all.iter().filter(|d| d.severity == Severity::Error).map(|d| d.message.clone()).collect()
}

#[test]
fn program_hand_off_with_string_args_is_clean() {
    let p = crate::program_fixture::with_program("return @pr-handling/main(pr=pr);");
    assert_eq!(errors(&p), Vec::<String>::new());
    let bare = crate::program_fixture::with_program("return @pr-handling(pr=pr);");
    assert_eq!(errors(&bare), Vec::<String>::new());
}

#[test]
fn unknown_program_is_an_error_naming_it() {
    let mut p = crate::program_fixture::with_program("return @pr-handlng/main(pr=pr);");
    p.link_programs(&|name| Err(format!("unknown program `{name}`; run `rein list` to list programs")));
    assert_eq!(errors(&p), ["unknown program `pr-handlng`; run `rein list` to list programs"]);
    let unlinked = Project::from_sources(
        PathBuf::from("/p"),
        vec![(PathBuf::from("/p/main.rein"), "/** M. */\nexport default function main() {\n  return @nope;\n}\n".into())],
    );
    assert_eq!(errors(&unlinked), ["cannot find program `nope`; run `rein list` to list programs"]);
}

#[test]
fn program_hand_off_checks_argument_names_and_function() {
    let p = crate::program_fixture::with_program("return @pr-handling/main(url=pr);");
    let msgs = errors(&p);
    assert!(msgs.iter().any(|m| m.contains("`url` is not a parameter of `@pr-handling/main`; parameters are pr")), "{msgs:?}");
    let missing = crate::program_fixture::with_program("return @pr-handling/main;");
    assert_eq!(errors(&missing), ["`@pr-handling/main` takes 1 argument, got 0"]);
    let nofn = crate::program_fixture::with_program("return @pr-handling/other(pr=pr);");
    assert_eq!(errors(&nofn), ["cannot find function `other` in program `pr-handling`"]);
}

#[test]
fn program_references_are_not_in_project_edges() {
    let p = crate::program_fixture::with_program("return @pr-handling/main(pr=pr);");
    assert!(p.references().is_empty());
    assert_eq!(p.program_refs(), [("main".to_string(), "pr-handling/main".to_string())]);
}

/// A function `long` whose return line has `width` characters. The string repeats `fill`.
fn function_with_line(width: usize, fill: char) -> String {
    // `  return "` is 10 characters and `";` is 2.
    let text: String = std::iter::repeat_n(fill, width - 12).collect();
    format!("/** Long. */\nexport default function long() {{\n  return `{text}`;\n}}\n")
}

const WIDTH_81: &str = "line has 81 characters; the limit is 80. Break the line; a long text can be a template (backticks) that spans lines";

/// Line-length errors of the file `long` in a project of `main` and `long`.
fn width_errors(text: &str) -> Vec<Diagnostic> {
    project_of(&[("long", text)])
        .check()
        .remove("long")
        .unwrap()
        .into_iter()
        .filter(|d| d.message.contains(" characters; the limit is "))
        .collect()
}

/// The line-length error texts of the project's files.
fn width_lines(project: &Project) -> Vec<String> {
    project
        .check()
        .into_values()
        .flatten()
        .map(|d| d.message)
        .filter(|m| m.contains(" characters; the limit is "))
        .collect()
}

#[test]
fn exactly_80_characters_pass_and_81_fail_on_the_overflow() {
    assert!(width_errors(&function_with_line(80, 'a')).is_empty());
    let text = function_with_line(81, 'a');
    let errors = width_errors(&text);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].severity, Severity::Error);
    assert_eq!(errors[0].message, WIDTH_81);
    assert_eq!(&text[errors[0].span.start..errors[0].span.end], ";");
}

#[test]
fn line_length_counts_characters_not_bytes() {
    // `é` is 2 bytes, so the 80-character line is 149 bytes.
    assert!(width_errors(&function_with_line(80, 'é')).is_empty());
    let text = function_with_line(81, 'é');
    let errors = width_errors(&text);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].message, WIDTH_81);
    assert_eq!(&text[errors[0].span.start..errors[0].span.end], ";");
    // A tab is one character.
    assert!(width_errors(&function_with_line(80, '\t')).is_empty());
}

#[test]
fn manifest_sets_the_line_length_limit() {
    let manifest = Some(r#"{"maxLineLength": 120}"#);
    let project = flow_of("width-ok", manifest, &function_with_line(120, 'a'));
    assert_eq!(project.max_line_length, 120);
    assert!(project.manifest_diagnostics().is_empty());
    assert!(width_lines(&project).is_empty());

    let project = flow_of("width-over", manifest, &function_with_line(121, 'a'));
    let errors = width_lines(&project);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("line has 121 characters; the limit is 120. Break the line"), "{errors:?}");
}

#[test]
fn invalid_line_length_is_a_manifest_error_and_the_default_applies() {
    for value in ["0", "-1", "1.5", "\"120\"", "null"] {
        let manifest = format!(r#"{{"maxLineLength": {value}}}"#);
        let project = flow_of("width-bad", Some(&manifest), &function_with_line(81, 'a'));
        assert_eq!(project.max_line_length, 80, "{value}");
        let errors = project.manifest_diagnostics();
        assert_eq!(errors.len(), 1, "{value}: {errors:?}");
        assert!(
            errors[0].message.starts_with("`maxLineLength` in rein.json must be a positive integer, such as 80; found "),
            "{value}: {}",
            errors[0].message
        );
        assert_eq!(width_lines(&project), [WIDTH_81], "{value}");
    }
}

#[test]
fn one_manifest_sets_both_limits() {
    let manifest = Some(r#"{"maxFunctionLines": 60, "maxLineLength": 0}"#);
    let project = flow_of("both", manifest, &function_of(41));
    assert_eq!(project.max_function_lines, 60);
    assert_eq!(project.max_line_length, 80);
    assert_eq!(project.manifest_diagnostics().len(), 1);
}

#[test]
fn struct_files_are_checked_for_line_length() {
    let doc = "x".repeat(70);
    let text = format!("/** Ticket. */\nexport default interface Ticket {{\n  /** {doc} */\n  category: boolean;\n}}\n");
    let project = project_of(&[("ticket.struct", text.as_str())]);
    let errors: Vec<_> = project.check()["ticket.struct"].iter().map(|d| d.message.clone()).collect();
    assert!(!errors.iter().any(|m| m.contains(" characters; the limit is ")), "{errors:?}");
    let text = text.replace(&doc, &"x".repeat(72));
    let project = project_of(&[("ticket.struct", text.as_str())]);
    assert!(project.check()["ticket.struct"].iter().any(|d| d.message.starts_with("line has 81 characters")));
}
