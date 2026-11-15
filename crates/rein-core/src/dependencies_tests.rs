use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A unique empty temp folder, with `package.json` holding `json` when given.
fn package(json: Option<&str>) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rein-deps-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    if let Some(json) = json {
        fs::write(dir.join(PACKAGE), json).unwrap();
    }
    dir
}

#[test]
fn no_package_json_has_no_limit() {
    assert_eq!(read(&package(None)), None);
}

#[test]
fn every_dependency_field_declares_a_program() {
    let dir = package(Some(
        r#"{"name": "a", "dependencies": {"b": "1"}, "devDependencies": {"c": "1"},
            "optionalDependencies": {"d": "1"}, "peerDependencies": {"e": "1"}}"#,
    ));
    let deps = read(&dir).unwrap();
    assert_eq!(deps.package, "a");
    for name in ["b", "c", "d", "e"] {
        assert_eq!(deps.undeclared(name), None, "{name}");
    }
}

#[test]
fn an_undeclared_program_names_the_package_and_the_fix() {
    let dir = package(Some(r#"{"name": "rein-github-notifications-handler"}"#));
    let reason = read(&dir).unwrap().undeclared("rein-dependabot-handler").unwrap();
    assert_eq!(
        reason,
        "program `rein-dependabot-handler` is not a dependency of `rein-github-notifications-handler`; add it to \
         `dependencies` in package.json, such as with `pnpm add rein-dependabot-handler`"
    );
}

#[test]
fn a_package_json_that_does_not_parse_declares_nothing() {
    let deps = read(&package(Some("{ not json"))).unwrap();
    assert!(deps.names.is_empty());
    assert!(deps.undeclared("b").is_some());
}
