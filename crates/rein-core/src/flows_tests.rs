use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A unique empty temp directory.
fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "rein-flows-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes `rel` (relative to `base`) with `text`, creating folders.
fn write(base: &Path, rel: &str, text: &str) {
    let p = base.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// A flow whose entry has the given description.
fn flow_text(desc: &str) -> String {
    format!("/** {desc} */\nexport default function main() {{\n  return `ok`;\n}}\n")
}

fn names(flows: &[Flow]) -> Vec<(String, Scope, Option<Scope>)> {
    flows
        .iter()
        .map(|f| (f.name.clone(), f.scope, f.shadowed_by))
        .collect()
}

#[test]
fn project_beats_user_and_user_beats_system() {
    let home = temp_dir("home");
    let sys = temp_dir("sys");
    let repo = temp_dir("repo");
    write(&repo, "rein/deploy/main.rein", &flow_text("project deploy"));
    write(&home, "rein/deploy/main.rein", &flow_text("user deploy"));
    write(&sys, "deploy/main.rein", &flow_text("system deploy"));

    let flows = discover(
        &repo,
        Some(&home.join(FLOW_DIR)),
        std::slice::from_ref(&sys),
    );
    let deploy: Vec<_> = flows.iter().filter(|f| f.name == "deploy").collect();
    assert_eq!(deploy.len(), 3, "{flows:?}");
    assert_eq!(resolve(&flows, "deploy").unwrap().scope, Scope::Project);
    assert_eq!(
        resolve(&flows, "deploy").unwrap().description,
        "project deploy"
    );
    assert_eq!(deploy[1].shadowed_by, Some(Scope::Project));
    assert_eq!(deploy[2].shadowed_by, Some(Scope::Project));
}

#[test]
fn user_beats_system_when_no_project_flow() {
    let home = temp_dir("home2");
    let sys = temp_dir("sys2");
    let repo = temp_dir("repo2");
    write(&home, "rein/gate/main.rein", &flow_text("user gate"));
    write(&sys, "gate/main.rein", &flow_text("system gate"));
    let flows = discover(
        &repo,
        Some(&home.join(FLOW_DIR)),
        std::slice::from_ref(&sys),
    );
    assert_eq!(resolve(&flows, "gate").unwrap().scope, Scope::User);
    assert_eq!(
        names(&flows)
            .iter()
            .find(|n| n.1 == Scope::System)
            .unwrap()
            .2,
        Some(Scope::User)
    );
}

#[test]
fn nearest_rein_wins_for_nested_repos() {
    let repo = temp_dir("nested");
    write(&repo, "rein/review/main.rein", &flow_text("outer review"));
    write(
        &repo,
        "service/rein/review/main.rein",
        &flow_text("inner review"),
    );
    let cwd = repo.join("service");
    let flows = discover(&cwd, None, &[]);
    let winner = resolve(&flows, "review").unwrap();
    assert_eq!(winner.description, "inner review");
    assert_eq!(winner.root, cwd.join("rein/review"));
    let outer = flows
        .iter()
        .find(|f| f.description == "outer review")
        .unwrap();
    assert_eq!(outer.shadowed_by, Some(Scope::Project));
}

#[test]
fn user_scope_under_home_is_not_also_a_project_scope() {
    let home = temp_dir("home3");
    write(&home, "rein/mine/main.rein", &flow_text("mine"));
    let cwd = home.join("code/app");
    fs::create_dir_all(&cwd).unwrap();
    let flows = discover(&cwd, Some(&home.join(FLOW_DIR)), &[]);
    assert_eq!(names(&flows), vec![("mine".to_string(), Scope::User, None)]);
}

#[test]
fn folder_without_main_is_not_a_flow() {
    let repo = temp_dir("nomain");
    write(&repo, "rein/broken/other.rein", &flow_text("no entry"));
    write(&repo, "rein/ok/main.rein", &flow_text("has entry"));
    let flows = discover(&repo, None, &[]);
    assert_eq!(
        names(&flows),
        vec![("ok".to_string(), Scope::Project, None)]
    );
}

#[test]
fn description_is_the_first_paragraph_of_the_doc_comment() {
    let repo = temp_dir("desc");
    write(
        &repo,
        "rein/merge-pr/main.rein",
        "/**\n * Decide whether a pull request\n * can be merged.\n *\n * @x later text\n */\nexport default function main() {\n  return `ok`;\n}\n",
    );
    let flows = discover(&repo, None, &[]);
    assert_eq!(
        flows[0].description,
        "Decide whether a pull request can be merged."
    );
}

#[test]
fn agent_context_lists_visible_flows_only() {
    let home = temp_dir("home4");
    let repo = temp_dir("repo4");
    write(
        &repo,
        "rein/merge-pr/main.rein",
        &flow_text("Decide a merge."),
    );
    write(
        &home,
        "rein/merge-pr/main.rein",
        &flow_text("Shadowed copy."),
    );
    write(
        &home,
        "rein/triage/main.rein",
        &flow_text("Triage an incident."),
    );
    let flows = discover(&repo, Some(&home.join(FLOW_DIR)), &[]);
    let text = agent_context(&flows);
    assert!(
        text.contains("- merge-pr (project): Decide a merge."),
        "{text}"
    );
    assert!(
        text.contains("- triage (user): Triage an incident."),
        "{text}"
    );
    assert!(!text.contains("Shadowed copy"), "{text}");
    assert!(text.contains("rein step --flow <name> --json"), "{text}");
    assert!(text.lines().count() <= 25, "{text}");
}

#[test]
fn agent_context_is_empty_without_flows() {
    let repo = temp_dir("empty");
    assert_eq!(agent_context(&discover(&repo, None, &[])), "");
}

#[test]
fn visible_rein_folder_with_manifest_is_a_program() {
    let repo = temp_dir("manifest");
    write(
        &repo,
        "rein/app/rein.json",
        r#"{"name": "renamed", "description": "From the manifest.", "entry": "start.rein"}"#,
    );
    write(
        &repo,
        "rein/app/start.rein",
        &flow_text("From the doc comment."),
    );
    write(
        &repo,
        "rein/not-a-program/notes.txt",
        "no manifest, no main.rein",
    );
    let flows = discover(&repo, None, &[]);
    assert_eq!(flows.len(), 1, "{flows:?}");
    let app = resolve(&flows, "renamed").unwrap();
    assert_eq!(app.scope, Scope::Project);
    assert_eq!(app.description, "From the manifest.");
    assert_eq!(app.entry, "start");
}

#[test]
fn package_json_wins_over_rein_json_and_rein_json_is_the_fallback() {
    let repo = temp_dir("precedence");
    write(
        &repo,
        "rein/app/package.json",
        r#"{"name": "pkg-name", "version": "1.2.3", "description": "From package.json.", "engines": {"rein": "^0.1.0"}}"#,
    );
    write(
        &repo,
        "rein/app/rein.json",
        r#"{"name": "rein-name", "description": "From rein.json.", "entry": "start.rein"}"#,
    );
    write(
        &repo,
        "rein/app/start.rein",
        &flow_text("From the doc comment."),
    );
    write(&repo, "rein/plain/package.json", r#"{"name": "plain-pkg"}"#);
    write(
        &repo,
        "rein/plain/rein.json",
        r#"{"description": "Fallback description."}"#,
    );
    write(
        &repo,
        "rein/plain/main.rein",
        &flow_text("From the doc comment."),
    );
    let flows = discover(&repo, None, &[]);
    let app = resolve(&flows, "pkg-name").unwrap();
    assert_eq!(app.description, "From package.json.");
    assert_eq!(app.version.as_deref(), Some("1.2.3"));
    assert_eq!(app.engines_rein.as_deref(), Some("^0.1.0"));
    assert_eq!(app.entry, "start");
    assert!(resolve(&flows, "rein-name").is_none());
    let plain = resolve(&flows, "plain-pkg").unwrap();
    assert_eq!(plain.description, "Fallback description.");
    assert_eq!(plain.version, None);
    assert_eq!(plain.engines_rein, None);
}
