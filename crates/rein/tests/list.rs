//! `rein list` takes `ls`-style flags and name globs. Each test gets a temp HOME, a user programs folder (REIN_HOME)
//! and a repo with a `rein/` folder, so no real user or system folder is read.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// The entry files' modification times are this many seconds after this epoch, plus the offset each test sets.
const BASE: u64 = 1_700_000_000;

struct Sandbox {
    base: PathBuf,
}

impl Sandbox {
    /// A fixture with five visible programs, and one user program shadowed by a project program of the same name.
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let raw = std::env::temp_dir().join(format!("rein-list-{tag}-{}-{nanos}-{n}", std::process::id()));
        fs::create_dir_all(raw.join("home")).unwrap();
        fs::create_dir_all(raw.join("repo/sub")).unwrap();
        // The child process reports its folder physically, so the expected paths are canonical too.
        let sb = Sandbox { base: fs::canonicalize(&raw).unwrap() };
        sb.lay_out();
        sb
    }

    fn programs(&self) -> PathBuf {
        self.base.join("programs")
    }

    fn repo(&self) -> PathBuf {
        self.base.join("repo")
    }

    fn lay_out(&self) {
        let user = self.programs();
        write(&user.join("merge-pr/main.rein"), &flow_text("Shadowed merge."));
        write(&user.join("triage/main.rein"), &flow_text("Triage an incident."));
        write(&user.join("pr-review/main.rein"), &flow_text("Unused."));
        write(&user.join("pr-review/package.json"), r#"{"name": "pr-review", "version": "1.2.0", "description": "Review a pull request."}"#);
        let rein = self.repo().join("rein");
        write(&rein.join("merge-pr/main.rein"), &flow_text("Decide a merge."));
        write(&rein.join("deploy/main.rein"), &flow_text("Deploy the app."));
        write(&rein.join("pr-fix/main.rein"), &flow_text("Unused."));
        write(&rein.join("pr-fix/package.json"), r#"{"name": "pr-fix", "version": "0.3.0", "description": "Fix a pull request."}"#);
        // Entry-file ages, oldest first: triage, the shadowed merge-pr, pr-review, project merge-pr, deploy, pr-fix.
        age(&user.join("triage/main.rein"), 10);
        age(&user.join("merge-pr/main.rein"), 20);
        age(&user.join("pr-review/main.rein"), 50);
        age(&rein.join("merge-pr/main.rein"), 100);
        age(&rein.join("deploy/main.rein"), 200);
        age(&rein.join("pr-fix/main.rein"), 300);
    }

    /// `rein list ...` run from a folder under the repo. Its stdout is a pipe, so it is not a terminal.
    fn rein(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rein"))
            .args(args)
            .current_dir(self.repo().join("sub"))
            .env("HOME", self.base.join("home"))
            .env("REIN_HOME", self.programs())
            .env("REIN_SYSTEM_DIRS", "")
            .env("TMPDIR", &self.base)
            .env_remove("POLYHOOK_CALLER")
            .env_remove("CLAUDE_CODE_VERSION")
            .output()
            .unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn age(path: &Path, offset: u64) {
    let when = UNIX_EPOCH + Duration::from_secs(BASE + offset);
    fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
}

fn flow_text(desc: &str) -> String {
    format!("/** {desc} */\nexport default function main() {{\n  return \"ok\";\n}}\n")
}

/// Stdout as text, after checking that the command exited 0.
fn ok_stdout(out: &Output) -> String {
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn json(out: &Output) -> Vec<Value> {
    serde_json::from_str(&ok_stdout(out)).unwrap()
}

#[test]
fn bare_list_prints_visible_names_one_per_line_without_shadowed_ones() {
    let sb = Sandbox::new("bare");
    assert_eq!(ok_stdout(&sb.rein(&["list"])), "deploy\nmerge-pr\npr-fix\npr-review\ntriage\n");
}

#[test]
fn long_format_aligns_scope_version_name_path_and_description() {
    let sb = Sandbox::new("long");
    let text = ok_stdout(&sb.rein(&["list", "-l"]));
    // scope, version, name, description, in name order. A package with no version shows `-`.
    let want = [
        ("project", "-", "deploy", "Deploy the app."),
        ("project", "-", "merge-pr", "Decide a merge."),
        ("project", "0.3.0", "pr-fix", "Fix a pull request."),
        ("user", "1.2.0", "pr-review", "Review a pull request."),
        ("user", "-", "triage", "Triage an incident."),
    ];
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), want.len(), "{text}");
    for (row, (scope, version, name, desc)) in rows.iter().zip(want) {
        let root = if scope == "project" { sb.repo().join("rein") } else { sb.programs() }.join(name);
        assert!(row.starts_with(&format!("{scope:<7}  {version:<5}  {name:<9}  {}", root.display())), "{row}");
        assert!(row.ends_with(desc), "{row}");
    }
}

#[test]
fn all_shows_the_shadowed_program_marked_and_long_format_marks_it_too() {
    let sb = Sandbox::new("all");
    let names = ok_stdout(&sb.rein(&["list", "-a"]));
    assert_eq!(names, "deploy\nmerge-pr\nmerge-pr (shadowed by project)\npr-fix\npr-review\ntriage\n");
    let long = ok_stdout(&sb.rein(&["list", "-la"]));
    assert_eq!(long.lines().count(), 6, "{long}");
    assert!(long.lines().any(|l| l.ends_with("Shadowed merge. (shadowed by project)")), "{long}");
    assert!(!ok_stdout(&sb.rein(&["list", "-l"])).contains("shadowed"));
}

#[test]
fn reverse_and_time_order_follow_ls() {
    let sb = Sandbox::new("order");
    assert_eq!(ok_stdout(&sb.rein(&["list", "-r"])), "triage\npr-review\npr-fix\nmerge-pr\ndeploy\n");
    // Newest entry file first.
    assert_eq!(ok_stdout(&sb.rein(&["list", "-t"])), "pr-fix\ndeploy\nmerge-pr\npr-review\ntriage\n");
    assert_eq!(ok_stdout(&sb.rein(&["list", "-tr"])), "triage\npr-review\nmerge-pr\ndeploy\npr-fix\n");
    // The shadowed copy is older than pr-review, so `-ta` places it between pr-review and triage.
    assert_eq!(
        ok_stdout(&sb.rein(&["list", "-ta"])),
        "pr-fix\ndeploy\nmerge-pr\npr-review\nmerge-pr (shadowed by project)\ntriage\n"
    );
}

#[test]
fn operands_are_shell_style_globs_on_the_name() {
    let sb = Sandbox::new("glob");
    assert_eq!(ok_stdout(&sb.rein(&["list", "pr-*"])), "pr-fix\npr-review\n");
    assert_eq!(ok_stdout(&sb.rein(&["list", "pr-*", "tri*"])), "pr-fix\npr-review\ntriage\n");
    assert_eq!(ok_stdout(&sb.rein(&["list", "-l", "merge-pr"])).lines().count(), 1);
}

#[test]
fn an_operand_that_matches_nothing_is_an_error_and_the_rest_still_print() {
    let sb = Sandbox::new("nomatch");
    let out = sb.rein(&["list", "nope", "pr-*"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "pr-fix\npr-review\n");
    assert!(String::from_utf8_lossy(&out.stderr).contains("error: no program matches `nope`"), "{out:?}");
}

#[test]
fn combined_short_flags_mean_the_same_as_separate_ones() {
    let sb = Sandbox::new("combined");
    assert_eq!(ok_stdout(&sb.rein(&["list", "-la"])), ok_stdout(&sb.rein(&["list", "-l", "-a"])));
    assert_eq!(ok_stdout(&sb.rein(&["list", "-lt"])), ok_stdout(&sb.rein(&["list", "-l", "-t"])));
    let plain = ok_stdout(&sb.rein(&["list", "-1"]));
    let reversed: Vec<&str> = plain.lines().rev().collect();
    assert_eq!(ok_stdout(&sb.rein(&["list", "-1r"])), format!("{}\n", reversed.join("\n")));
}

#[test]
fn json_honors_the_operands_all_and_reverse() {
    let sb = Sandbox::new("json");
    let all = json(&sb.rein(&["list", "--json"]));
    assert_eq!(all.len(), 5, "{all:?}");
    assert!(all.iter().all(|r| r.get("first_step").is_some()), "{all:?}");

    let globbed = json(&sb.rein(&["list", "--json", "pr-*"]));
    let names: Vec<&str> = globbed.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["pr-fix", "pr-review"]);

    // Both copies of merge-pr, in precedence order, reversed: the user copy comes first.
    let both = json(&sb.rein(&["list", "--json", "-a", "-r", "merge-pr"]));
    assert_eq!(both.len(), 2, "{both:?}");
    assert_eq!((both[0]["scope"].as_str(), both[0]["shadowed_by"].as_str()), (Some("user"), Some("project")));
    assert_eq!((both[1]["scope"].as_str(), both[1]["shadowed"].as_bool()), (Some("project"), Some(false)));
}
