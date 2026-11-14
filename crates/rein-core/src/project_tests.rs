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

/// Cycle warnings of `files`, in a project whose `main` returns `@./a`.
fn cycle_warnings_of(files: &[(&str, &str)]) -> BTreeMap<String, Vec<Diagnostic>> {
    let mut all = vec![("main", "/** m */\nexport default function main() { return @./a }\n")];
    all.extend_from_slice(files);
    project_of(&all).check()
}

fn has_cycle_warning(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.message.contains("reference cycle"))
}

const SELF_AWAIT: &str = "/** d */\nexport default async function a() {\n  /** Again? */\n  const again: boolean = await @./a;\n  return `done`;\n}\n";
const GUARDED_SELF_AWAIT: &str = "/** Recurse until the agent says stop. */\nexport default async function a(\n  /** Keep going? */\n  more: boolean,\n) {\n  if (!more) {\n    return `stopped`;\n  }\n  /** Again? */\n  const again: boolean = await @./a;\n  return `done`;\n}\n";
const AWAIT_B: &str = "/** d */\nexport default async function a() {\n  /** Ok? */\n  const ok: boolean = await @./b;\n  return `a`;\n}\n";
const AWAIT_A: &str = "/** d */\nexport default async function b() {\n  /** Ok? */\n  const ok: boolean = await @./a;\n  return `b`;\n}\n";
const GUARDED_AWAIT_B: &str = "/** d */\nexport default async function a(\n  /** A? */\n  go: boolean,\n) {\n  if (!go) {\n    return `a stop`;\n  }\n  /** Ok? */\n  const ok: boolean = await @./b;\n  return `a`;\n}\n";
const GUARDED_AWAIT_A: &str = "/** d */\nexport default async function b(\n  /** B? */\n  go: boolean,\n) {\n  if (!go) {\n    return `b stop`;\n  }\n  /** Ok? */\n  const ok: boolean = await @./a;\n  return `b`;\n}\n";

#[test]
fn hand_off_self_cycle_still_warns() {
    // c1
    let diags = cycle_warnings_of(&[("a", "/** d */\nexport default function a() { return @./a }\n")]);
    assert!(
        has(&diags["a"], Severity::Warning, "reference cycle: a -> a; Rein has no loops yet"),
        "{:?}",
        diags["a"]
    );
}

#[test]
fn hand_off_two_cycle_warns() {
    // c2
    let diags = cycle_warnings_of(&[
        ("a", "/** d */\nexport default function a() { return @./b }\n"),
        ("b", "/** d */\nexport default function b() { return @./a }\n"),
    ]);
    assert!(has_cycle_warning(&diags["a"]), "{:?}", diags["a"]);
    assert!(has_cycle_warning(&diags["b"]), "{:?}", diags["b"]);
}

#[test]
fn self_await_without_a_question_warns() {
    // c3: the await is the first step, so every level asks the same key.
    let diags = cycle_warnings_of(&[("a", SELF_AWAIT)]);
    assert!(
        has(&diags["a"], Severity::Warning, "reference cycle: a -> a"),
        "{:?}",
        diags["a"]
    );
}

#[test]
fn self_await_after_a_question_is_not_warned() {
    // c10: the parameter is asked before the await, at each level.
    let diags = cycle_warnings_of(&[("a", GUARDED_SELF_AWAIT)]);
    assert!(!has_cycle_warning(&diags["a"]), "{:?}", diags["a"]);
}

#[test]
fn mutual_await_without_a_question_warns() {
    // c4
    let diags = cycle_warnings_of(&[("a", AWAIT_B), ("b", AWAIT_A)]);
    assert!(has_cycle_warning(&diags["a"]), "{:?}", diags["a"]);
    assert!(has_cycle_warning(&diags["b"]), "{:?}", diags["b"]);
}

#[test]
fn mutual_await_after_questions_is_not_warned() {
    // c11
    let diags = cycle_warnings_of(&[("a", GUARDED_AWAIT_B), ("b", GUARDED_AWAIT_A)]);
    assert!(!has_cycle_warning(&diags["a"]), "{:?}", diags["a"]);
    assert!(!has_cycle_warning(&diags["b"]), "{:?}", diags["b"]);
}

#[test]
fn hand_off_then_await_without_a_question_warns() {
    // c5: a returns b, and b awaits a with no question before it.
    let diags = cycle_warnings_of(&[
        ("a", "/** d */\nexport default function a() { return @./b }\n"),
        ("b", AWAIT_A),
    ]);
    assert!(has_cycle_warning(&diags["a"]), "{:?}", diags["a"]);
    assert!(has_cycle_warning(&diags["b"]), "{:?}", diags["b"]);
}

#[test]
fn hand_off_cycle_with_an_await_elsewhere_warns() {
    // c6: the await is not on the cycle, and the two returns replay the same answers.
    let diags = cycle_warnings_of(&[
        ("a", "/** d */\nexport default async function a() {\n  /** Ok? */\n  const ok: boolean = await @./c;\n  return @./b;\n}\n"),
        ("b", "/** d */\nexport default function b() { return @./a }\n"),
        ("c", "/** d */\nexport default function c() { return `c` }\n"),
    ]);
    assert!(has_cycle_warning(&diags["a"]), "{:?}", diags["a"]);
    assert!(has_cycle_warning(&diags["b"]), "{:?}", diags["b"]);
}

#[test]
fn await_without_a_cycle_is_not_warned() {
    // c7
    let diags = cycle_warnings_of(&[("a", "/** d */\nexport default function a() { return `f` }\n")]);
    assert!(!has_cycle_warning(&diags["a"]), "{:?}", diags["a"]);
}

#[test]
fn await_answer_does_not_count_as_a_question_before_its_own_edge() {
    // c8: `ok` is the answer to the await, so it comes after the edge.
    let diags = cycle_warnings_of(&[
        ("a", AWAIT_B),
        ("b", "/** d */\nexport default function b() { return @./a }\n"),
    ]);
    assert!(has_cycle_warning(&diags["a"]), "{:?}", diags["a"]);
    assert!(has_cycle_warning(&diags["b"]), "{:?}", diags["b"]);
}

#[test]
fn mixed_cycle_without_a_question_warns() {
    // c9: one await edge, but no question before any edge.
    let diags = cycle_warnings_of(&[
        ("a", "/** d */\nexport default function a() { return @./b }\n"),
        ("b", "/** d */\nexport default async function b() {\n  /** Ok? */\n  const ok: boolean = await @./c;\n  return `b`;\n}\n"),
        ("c", "/** d */\nexport default function c() { return @./a }\n"),
    ]);
    for name in ["a", "b", "c"] {
        assert!(has_cycle_warning(&diags[name]), "{name}: {:?}", diags[name]);
    }
}

#[test]
fn request_answer_before_the_await_is_a_question() {
    // A `request` answer above the await asks a new key at each level.
    let diags = cycle_warnings_of(&[(
        "a",
        "/** d */\nexport default async function a() {\n  /** Checked? */\n  const checked: boolean = await request(`Check the thing.`);\n  /** Again? */\n  const again: boolean = await @./a;\n  return `done`;\n}\n",
    )]);
    assert!(!has_cycle_warning(&diags["a"]), "{:?}", diags["a"]);
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
    assert_eq!(errors(&missing), ["`@pr-handling/main` takes 3 arguments, got 0; pass `pr`, `draft` and `task`"]);
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

#[test]
fn a_missing_blank_line_around_an_if_is_a_warning_not_an_error() {
    let text = "/** Long. */\nexport default function long(/** A? */ a: boolean) {\n  if (a) {\n    return `yes`;\n  }\n  return `no`;\n}\n";
    let project = project_of(&[("long", text)]);
    let diags = &project.check()["long"];
    assert!(has(diags, Severity::Warning, "add an empty line after the `if` block"), "{diags:?}");
    assert!(!diags.iter().any(|d| d.severity == Severity::Error), "{diags:?}");
}

/// A function named `name` with a boolean question `a`: it returns `yes` when `a` holds, else `no`.
fn branch(name: &str, yes: &str, no: &str) -> String {
    format!(
        "/** {name} */\nexport default function {name}(\n  /** A? */\n  a: boolean,\n) {{\n  if (a) {{\n    return {yes};\n  }}\n  return {no};\n}}\n"
    )
}

#[test]
fn a_function_with_one_caller_belongs_in_the_caller_folder() {
    let leaf = |body: &str| format!("/** l */\nexport default function leaf() {{ return {body} }}\n");
    let ticket = "/** A ticket. */\nexport default interface Ticket {\n  /** The ticket id. */\n  id: string;\n}\n";
    let files = [
        ("main", branch("main", "@./merge", "@./shared")),
        ("merge", branch("merge", "@./steps/fix_ci", "@./merge/done")),
        ("merge/done", leaf("@../shared")),
        ("steps/fix_ci", leaf("`fixed`")),
        ("shared", leaf("`shared`")),
        ("ticket.struct", ticket.to_string()),
    ];
    let files: Vec<(&str, &str)> = files.iter().map(|(n, t)| (*n, t.as_str())).collect();
    let project = project_of(&files);
    let diags = project.check();
    let warning = "`steps/fix_ci` is only called by `merge`; move it to `merge/fix_ci.rein` so the folders show the flow";
    assert!(has(&diags["steps/fix_ci"], Severity::Warning, warning), "{:?}", diags["steps/fix_ci"]);
    assert_eq!(project.nest_target("steps/fix_ci").as_deref(), Some("merge/fix_ci"));
    // A callee of the entry stays at the root, a callee already in its caller's folder is in place, a function
    // with two callers is shared, and a struct is not a function.
    for name in ["main", "merge", "merge/done", "shared", "ticket.struct"] {
        assert!(!has(&diags[name], Severity::Warning, "folders show the flow"), "{name}: {:?}", diags[name]);
        assert_eq!(project.nest_target(name), None, "{name}");
    }
}

#[test]
fn a_function_called_only_by_the_entry_belongs_at_the_root() {
    let project = project_of(&[
        ("main", "/** m */\nexport default function main() { return @./security/review }\n"),
        ("security/review", "/** r */\nexport default function review() { return `ok` }\n"),
    ]);
    let warning = "`security/review` is only called by `main`; move it to `review.rein` so the folders show the flow";
    assert!(has(&project.check()["security/review"], Severity::Warning, warning));
    assert_eq!(project.nest_target("security/review").as_deref(), Some("review"));
}

#[test]
fn the_nesting_warning_says_a_move_changes_keys_and_program_references() {
    let project = project_of(&[
        ("main", "/** m */\nexport default function main() { return @./a }\n"),
        ("a", "/** a */\nexport default function a() { return @./steps/leaf }\n"),
        ("steps/leaf", "/** l */\nexport default function leaf() { return `ok` }\n"),
    ]);
    let tail = "The move changes its answer keys and breaks `@<program>/steps/leaf` references from other programs";
    assert!(has(&project.check()["steps/leaf"], Severity::Warning, tail), "{:?}", project.check()["steps/leaf"]);
}

#[test]
fn two_callees_with_one_target_name_each_other_and_get_no_move() {
    let project = project_of(&[
        ("main", "/** m */\nexport default function main() { return @./a }\n"),
        ("a", &branch("a", "@./x/leaf", "@./y/leaf")),
        ("x/leaf", "/** l */\nexport default function leaf() { return `x` }\n"),
        ("y/leaf", "/** l */\nexport default function leaf() { return `y` }\n"),
    ]);
    let diags = project.check();
    let clash = "`x/leaf` is only called by `a`, but `a/leaf.rein` is also the place for `y/leaf`; rename one of them";
    assert!(has(&diags["x/leaf"], Severity::Warning, clash), "{:?}", diags["x/leaf"]);
    assert!(has(&diags["y/leaf"], Severity::Warning, "is also the place for `x/leaf`"), "{:?}", diags["y/leaf"]);
    for name in ["x/leaf", "y/leaf"] {
        assert!(!has(&diags[name], Severity::Warning, "folders show the flow"), "{name}: {:?}", diags[name]);
        assert_eq!(project.nest_target(name), None, "{name}");
    }
}

#[test]
fn a_target_path_that_another_file_has_gives_no_warning() {
    // `x/leaf` belongs at `a/leaf`, but `a/leaf` is another function, so `x/leaf` cannot move there.
    let taken = project_of(&[
        ("main", "/** m */\nexport default function main() { return @./a }\n"),
        ("a", &branch("a", "@./x/leaf", "@./a/leaf")),
        ("a/leaf", "/** l */\nexport default function leaf() { return @../y/leaf }\n"),
        ("x/leaf", "/** l */\nexport default function leaf() { return `x` }\n"),
        ("y/leaf", "/** l */\nexport default function leaf() { return `y` }\n"),
    ]);
    let diags = taken.check();
    assert!(!has(&diags["x/leaf"], Severity::Warning, "only called by"), "{:?}", diags["x/leaf"]);
    assert_eq!(taken.nest_target("x/leaf"), None);
    assert_eq!(taken.nest_target("y/leaf").as_deref(), Some("a/leaf/leaf"));
}

#[test]
fn a_caller_inside_the_function_folder_gives_no_nesting_warning() {
    // `x` has one caller, `x/y`, which sits in `x/`. Moving `x` to `x/y/x` would move `x/y` along, so the
    // warning would come back one level deeper each time.
    let project = project_of(&[
        ("main", "/** m */\nexport default function main() { return @./x/y }\n"),
        ("x/y", "/** y */\nexport default function y() { return @../x }\n"),
        ("x", "/** x */\nexport default function x() { return `x` }\n"),
    ]);
    let diags = project.check();
    assert!(!has(&diags["x"], Severity::Warning, "only called by"), "{:?}", diags["x"]);
    assert_eq!(project.nest_target("x"), None);
}

#[test]
fn a_function_whose_folder_holds_the_entry_gives_no_nesting_warning() {
    // `app` belongs at `y/app`, but moving its folder would move the entry `app/start` and leave `rein.json`
    // naming a file that is gone.
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rein-core-entry-folder-{}-{nanos}", std::process::id()));
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    write("rein.json", r#"{"entry": "app/start.rein"}"#);
    write("app/start.rein", "/** s */\nexport default function start() { return @../y }\n");
    write("y.rein", "/** y */\nexport default function y() { return @./app }\n");
    write("app.rein", "/** a */\nexport default function app() { return `a` }\n");
    let project = Project::load(&root).unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(project.entry(), "app/start");
    assert!(!has(&project.check()["app"], Severity::Warning, "only called by"), "{:?}", project.check()["app"]);
    assert_eq!(project.nest_target("app"), None);
}

#[test]
fn a_path_the_loader_skips_is_never_a_nesting_target() {
    assert!(loadable_path("merge/fix"));
    assert!(loadable_path("main"));
    for path in ["target/pick", "node_modules/pick", ".hidden/pick", "a/target/b", "foo/main"] {
        assert!(!loadable_path(path), "{path}");
    }
    // `pick` belongs in `target/`, a folder the loader skips, so it would drop out of the project.
    let project = project_of(&[
        ("main", "/** m */\nexport default function main() { return @./target }\n"),
        ("target", "/** t */\nexport default function target() { return @./pick }\n"),
        ("pick", "/** p */\nexport default function pick() { return `p` }\n"),
    ]);
    assert!(!has(&project.check()["pick"], Severity::Warning, "only called by"), "{:?}", project.check()["pick"]);
    assert_eq!(project.nest_target("pick"), None);
}

#[test]
fn a_folder_with_its_own_main_is_never_a_nesting_target() {
    // `pick` belongs in `x/`, but `x/main.rein` makes `x/` another flow that the loader skips.
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rein-core-nested-flow-{}-{nanos}", std::process::id()));
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    write("main.rein", "/** m */\nexport default function main() { return @./x }\n");
    write("x.rein", "/** x */\nexport default function x() { return @./pick }\n");
    write("pick.rein", "/** p */\nexport default function pick() { return `p` }\n");
    write("x/main.rein", "/** n */\nexport default function main() { return `n` }\n");
    let project = Project::load(&root).unwrap();
    assert!(project.loads("pick") && !project.loads("x/pick") && project.loads("y/pick"));
    let (diags, target) = (project.check(), project.nest_target("pick"));
    fs::remove_dir_all(&root).unwrap();
    assert!(!has(&diags["pick"], Severity::Warning, "only called by"), "{:?}", diags["pick"]);
    assert_eq!(target, None);
}

#[test]
fn the_root_main_never_gets_a_nesting_warning() {
    // With the entry `start`, `main` is a plain function called only by `foo`. It still stays at the root: the
    // project needs it, and `foo/main.rein` would make `foo/` another flow.
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rein-core-root-main-{}-{nanos}", std::process::id()));
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    write("rein.json", r#"{"entry": "start.rein"}"#);
    write("start.rein", "/** s */\nexport default function start() { return @./foo }\n");
    write("foo.rein", "/** f */\nexport default function foo() { return @./main }\n");
    write("main.rein", "/** m */\nexport default function main() { return `m` }\n");
    let project = Project::load(&root).unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert!(!has(&project.check()["main"], Severity::Warning, "only called by"), "{:?}", project.check()["main"]);
    assert_eq!(project.nest_target("main"), None);
}

#[test]
fn unreachable_functions_get_no_nesting_warning() {
    // `u` and `v` only call each other. Each has one caller, but neither is reachable from main.
    let project = project_of(&[
        ("u", "/** u */\nexport default function u() { return @./v }\n"),
        ("v", "/** v */\nexport default function v() { return @./u }\n"),
    ]);
    let diags = project.check();
    for name in ["u", "v"] {
        assert!(has(&diags[name], Severity::Warning, "is not reachable from main"), "{name}: {:?}", diags[name]);
        assert!(!has(&diags[name], Severity::Warning, "only called by"), "{name}: {:?}", diags[name]);
        assert_eq!(project.nest_target(name), None, "{name}");
    }
}

#[test]
fn unknown_reachability_gives_no_nesting_warning() {
    // `u` and `v` only call each other. Reachability from main is unknown when main has a syntax error, and
    // also when main is missing. Neither case may tell `u` and `v` to move into each other's folder.
    let broken = project_of(&[
        ("main", "/** m */\nexport default function main() { return @./u"),
        ("u", "/** u */\nexport default function u() { return @./v }\n"),
        ("v", "/** v */\nexport default function v() { return @./u }\n"),
    ]);
    let root = PathBuf::from("/proj");
    let missing = Project::from_sources(
        root.clone(),
        vec![
            (root.join("u.rein"), "/** u */\nexport default function u() { return @./v }\n".to_string()),
            (root.join("v.rein"), "/** v */\nexport default function v() { return @./u }\n".to_string()),
        ],
    );
    for project in [&broken, &missing] {
        let diags = project.check();
        for name in ["u", "v"] {
            assert!(!has(&diags[name], Severity::Warning, "only called by"), "{name}: {:?}", diags[name]);
            assert_eq!(project.nest_target(name), None, "{name}");
        }
    }
}

#[test]
fn reachability_starts_at_the_entry_from_the_manifest() {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rein-core-entry-{}-{nanos}", std::process::id()));
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    write("rein.json", r#"{"entry": "flow.rein"}"#);
    write("main.rein", "/** m */\nexport default function main() { return `m` }\n");
    write("flow.rein", "/** f */\nexport default function flow() { return @./step }\n");
    write("step.rein", "/** s */\nexport default function step() { return `s` }\n");
    let project = Project::load(&root).unwrap();
    fs::remove_dir_all(&root).unwrap();
    let diags = project.check();
    // `step` is reached from `flow`, the entry, and only `flow` calls it, so it stays at the root.
    assert!(diags["step"].is_empty(), "{:?}", diags["step"]);
    assert!(diags["flow"].is_empty(), "{:?}", diags["flow"]);
    assert!(has(&diags["main"], Severity::Warning, "`main` is not reachable from flow"), "{:?}", diags["main"]);
}

#[test]
fn program_hand_off_passes_every_parameter() {
    let warnings = |p: &Project| -> Vec<String> {
        let all = &p.check()["main"];
        all.iter().filter(|d| d.severity == Severity::Warning).map(|d| d.message.clone()).collect()
    };
    let partial = crate::program_fixture::with_program("return @pr-handling/main(pr=pr);");
    assert_eq!(
        warnings(&partial),
        [
            "missing argument `draft` in call to `@pr-handling/main`; write `draft=value`",
            "missing argument `task` in call to `@pr-handling/main`; write `task=value`",
        ]
    );
    let main = "/** Handler. */\nexport default async function main(\n  /** The PR link. */\n  pr: string,\n) {\n  /** Draft? */\n  const draft: boolean = await request(`Read the PR.`);\n  /** Task? */\n  const task: \"review\" | \"merge\" = await request(`Pick a task.`);\n  return @pr-handling/main(pr=pr, draft=draft, task=task);\n}\n";
    let full = crate::program_fixture::with_main(main.to_string());
    assert!(full.check()["main"].is_empty(), "{:?}", full.check()["main"]);
}
