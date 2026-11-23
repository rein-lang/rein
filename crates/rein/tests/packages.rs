//! `rein install`, `uninstall` and `update`: the arguments reach pnpm, in the npm folder of the scope, and project-scope
//! discovery. Each test gets its own temp folders. The pnpm tests put a stub script in `REIN_PNPM`, so no pnpm runs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A temp folder with `home/`, `programs/` (the user programs folder), `work/` (outside any repository) and `repo/`
/// (a git repository with no `rein/` yet).
struct Sandbox {
    base: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let base = sandbox_root().join(format!("rein-packages-{tag}-{}-{nanos}-{n}", std::process::id()));
        fs::create_dir_all(base.join("home")).unwrap();
        fs::create_dir_all(base.join("work")).unwrap();
        fs::create_dir_all(base.join("repo/.git")).unwrap();
        fs::create_dir_all(base.join("repo/sub")).unwrap();
        Sandbox { base }
    }

    fn programs(&self) -> PathBuf {
        self.base.join("programs")
    }

    fn work(&self) -> PathBuf {
        self.base.join("work")
    }

    fn repo(&self) -> PathBuf {
        self.base.join("repo")
    }

    /// A `rein` command run from `cwd`. HOME, REIN_HOME and TMPDIR point into the sandbox.
    fn rein(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rein"));
        cmd.env("REIN_RUNS_DIR", "");
        cmd.args(args)
            .current_dir(cwd)
            .env("HOME", self.base.join("home"))
            .env("REIN_HOME", self.programs())
            .env("REIN_SYSTEM_DIRS", "")
            .env("TMPDIR", &self.base)
            .env_remove("POLYHOOK_CALLER")
            .env_remove("CLAUDE_CODE_VERSION");
        cmd
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// The folder the sandboxes go in. Rein takes the nearest `rein/` folder above the working directory, and otherwise the
/// nearest `.git` folder. So the folder and each of its parents must hold neither. The shared temp folder and a
/// checkout can both hold one.
fn sandbox_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .map(|h| h.join(".cache/rein-test"));
    [Some(PathBuf::from(env!("CARGO_TARGET_TMPDIR"))), home]
        .into_iter()
        .flatten()
        .find(|root| root.ancestors().all(|d| !d.join("rein").is_dir() && !d.join(".git").exists()))
        .expect("no clean folder: CARGO_TARGET_TMPDIR and $HOME/.cache/rein-test both sit under a `rein/` or `.git`")
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

const ENTRY: &str = "/** A program. */\nexport default function main() {\n  return `ok`;\n}\n";

const ENTRY_WITH_QUESTION: &str = "/**\n * Plain tool.\n */\nexport default function main(\n  /** Is it urgent? */\n  urgent: boolean,\n) {\n  if (urgent) {\n    return `yes`;\n  }\n  return `no`;\n}\n";

/// Lays out one package the way pnpm does under `<base>/npm`: the files under `node_modules/.pnpm/<store>/node_modules/
/// <name>`, and a symlink at `node_modules/<link>`. An npm alias has `link` different from `name`.
fn pnpm_package(base: &Path, store: &str, name: &str, link: &str, manifest: &str, entry: Option<&str>) {
    let modules = base.join("npm/node_modules");
    let target = modules.join(".pnpm").join(store).join("node_modules").join(name);
    write(&target.join("package.json"), manifest);
    if let Some(text) = entry {
        write(&target.join("main.rein"), text);
    }
    let link_path = modules.join(link);
    fs::create_dir_all(link_path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &link_path).unwrap();
}

/// Packages used by the discovery tests, under `base/npm`.
fn lay_out_packages(base: &Path) {
    pnpm_package(base, "plain-tool@1.0.0", "plain-tool", "plain-tool",
        r#"{"name": "plain-tool", "version": "1.0.0", "description": "A plain tool.", "engines": {"rein": "^0.1.0"}}"#,
        Some(ENTRY_WITH_QUESTION));
    pnpm_package(base, "@scope+tool@2.0.0", "@scope/tool", "@scope/tool",
        r#"{"name": "@scope/tool", "version": "2.0.0", "description": "A scoped tool."}"#, Some(ENTRY));
    pnpm_package(base, "pr@1.0.0", "pr", "pr-v1",
        r#"{"name": "pr", "version": "1.0.0", "description": "PR handling, version one.", "engines": {"rein": "^0.1.0"}}"#,
        Some(ENTRY));
    pnpm_package(base, "pr@2.0.0", "pr", "pr-v2",
        r#"{"name": "pr", "version": "2.0.0", "description": "PR handling, version two.", "engines": {"rein": "^0.2.0"}}"#,
        Some(ENTRY));
    pnpm_package(base, "lodash-like@4.0.0", "lodash-like", "lodash-like", r#"{"name": "lodash-like", "version": "4.0.0"}"#, None);
}

fn names(list: &[Value]) -> Vec<&str> {
    list.iter().map(|f| f["name"].as_str().unwrap()).collect()
}

fn rein_list(sb: &Sandbox, cwd: &Path) -> Vec<Value> {
    let out = sb.rein(cwd, &["list", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

#[cfg(unix)]
#[test]
fn list_finds_user_packages_by_their_link_names() {
    let sb = Sandbox::new("list-user");
    lay_out_packages(&sb.programs());
    let list = rein_list(&sb, &sb.work());
    // The non-Rein package and the `.pnpm` store are not programs. The two aliases are two programs.
    assert_eq!(names(&list), ["@scope/tool", "plain-tool", "pr-v1", "pr-v2"]);

    let by = |name: &str| list.iter().find(|f| f["name"] == name).unwrap();
    assert!(list.iter().all(|f| f["scope"] == "user" && f["shadowed"] == false), "{list:?}");
    assert_eq!(by("plain-tool")["version"], "1.0.0");
    assert_eq!(by("plain-tool")["description"], "A plain tool.");
    assert!(by("plain-tool")["path"].as_str().unwrap().ends_with("npm/node_modules/plain-tool"));
    assert_eq!(by("plain-tool")["first_step"]["description"], "Is it urgent?");
    assert!(by("plain-tool").get("warning").is_none());
    assert_eq!(by("@scope/tool")["version"], "2.0.0");
    assert_eq!(by("pr-v1")["description"], "PR handling, version one.");
    assert!(by("pr-v1").get("warning").is_none());
    assert_eq!(by("pr-v2")["description"], "PR handling, version two.");
    assert_eq!(by("pr-v2")["warning"], format!("needs rein ^0.2.0; this is rein {}", env!("CARGO_PKG_VERSION")));

    // A package runs by its link name, and `rein show` prints its source.
    let step = sb.rein(&sb.work(), &["step", "plain-tool", "--json"]).output().unwrap();
    assert_eq!(step.status.code(), Some(2));
    let step: Value = serde_json::from_slice(&step.stdout).unwrap();
    assert_eq!((step["kind"].as_str(), step["key"].as_str()), (Some("question"), Some("main.urgent")));
    let show = sb.rein(&sb.work(), &["show", "pr-v2"]).output().unwrap();
    assert_eq!(show.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&show.stdout).contains("// main.rein"));
}

#[cfg(unix)]
#[test]
fn list_finds_repo_packages_in_the_project_rein_npm_folder() {
    let sb = Sandbox::new("list-project");
    lay_out_packages(&sb.repo().join("rein"));
    // `npm` is the package project of its scope. A folder named `npm` with a flow in it is not a program.
    write(&sb.repo().join("rein/npm/main.rein"), ENTRY);
    let list = rein_list(&sb, &sb.repo().join("sub"));
    assert_eq!(names(&list), ["@scope/tool", "plain-tool", "pr-v1", "pr-v2"]);
    assert!(list.iter().all(|f| f["scope"] == "project"), "{list:?}");
}

/// A stub pnpm. It logs each call as its cwd, then one line per argument, then `END`. `remove missing-pkg` fails.
#[cfg(unix)]
fn stub_pnpm(sb: &Sandbox) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = sb.base.join("pnpm-stub");
    let log = sb.base.join("calls.log");
    let script = format!(
        "#!/bin/sh\n{{ pwd -P; for a in \"$@\"; do echo \"$a\"; done; echo END; }} >> '{}'\n\
         case \"$1 $2\" in\n  \"remove missing-pkg\") echo 'stub: no such package' >&2; exit 7 ;;\nesac\n",
        log.display()
    );
    write(&path, &script);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Each logged pnpm call: its cwd, then its arguments.
#[cfg(unix)]
fn calls(sb: &Sandbox) -> Vec<Vec<String>> {
    let log = fs::read_to_string(sb.base.join("calls.log")).unwrap_or_default();
    log.split("END\n").filter(|c| !c.is_empty()).map(|c| c.lines().map(str::to_string).collect()).collect()
}

/// A folder as pnpm reports its cwd: canonical.
#[cfg(unix)]
fn canon(path: &Path) -> String {
    fs::canonicalize(path).unwrap().display().to_string()
}

/// Shorthand for one expected pnpm call: its cwd, then its arguments.
#[cfg(unix)]
fn call(cwd: &Path, args: &[&str]) -> Vec<String> {
    std::iter::once(canon(cwd)).chain(args.iter().map(|a| a.to_string())).collect()
}

#[cfg(unix)]
#[test]
fn install_without_global_runs_in_the_repo_rein_npm_folder() {
    let sb = Sandbox::new("repo-install");
    let pnpm = stub_pnpm(&sb);
    let out = sb.rein(&sb.repo().join("sub"), &["install", "pr-handling@^1"]).env("REIN_PNPM", &pnpm).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    // No `rein/` existed, so it is made at the git root. The user folder is not touched.
    let npm = sb.repo().join("rein/npm");
    assert_eq!(calls(&sb), [call(&npm, &["add", "--ignore-scripts", "pr-handling@^1"])]);
    let manifest: Value = serde_json::from_str(&fs::read_to_string(npm.join("package.json")).unwrap()).unwrap();
    assert_eq!(manifest["private"], true);
    assert!(!sb.programs().exists());
}

#[cfg(unix)]
#[test]
fn global_flag_runs_in_the_user_folder_and_is_not_passed_on() {
    let sb = Sandbox::new("global");
    let pnpm = stub_pnpm(&sb);
    let user = sb.programs().join("npm");
    let out = sb.rein(&sb.repo(), &["install", "pr-handling@^2", "-g"]).env("REIN_PNPM", &pnpm).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let out = sb.rein(&sb.repo(), &["uninstall", "--global", "pr-v1"]).env("REIN_PNPM", &pnpm).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        calls(&sb),
        [call(&user, &["add", "--ignore-scripts", "pr-handling@^2"]), call(&user, &["remove", "pr-v1"])]
    );
    assert!(!sb.repo().join("rein/npm").exists());
}

#[cfg(unix)]
#[test]
fn outside_a_repository_without_global_is_an_error() {
    let sb = Sandbox::new("no-repo");
    let pnpm = stub_pnpm(&sb);
    let out = sb.rein(&sb.work(), &["install", "pr-handling"]).env("REIN_PNPM", &pnpm).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not in a repository; use -g to install for the user"));
    assert!(calls(&sb).is_empty());
}

#[cfg(unix)]
#[test]
fn every_argument_reaches_pnpm_unchanged() {
    let sb = Sandbox::new("passthrough");
    let pnpm = stub_pnpm(&sb);
    let npm = sb.repo().join("rein/npm");
    let runs: [&[&str]; 6] = [
        &["install", "--save-exact", "pr-handling@1"],
        &["install", "github:owner/repo"],
        &["install", "file:../pr-handling", "./my-program"],
        &["install", "pr-v1@npm:pr-handling@^1"],
        &["update", "--latest"],
        &["install", "--ignore-scripts=false", "file:../x"],
    ];
    for args in runs {
        let out = sb.rein(&sb.repo().join("sub"), args).env("REIN_PNPM", &pnpm).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
    assert_eq!(
        calls(&sb),
        [
            call(&npm, &["add", "--ignore-scripts", "--save-exact", "pr-handling@1"]),
            call(&npm, &["add", "--ignore-scripts", "github:owner/repo"]),
            call(&npm, &["add", "--ignore-scripts", "file:../pr-handling", "./my-program"]),
            call(&npm, &["add", "--ignore-scripts", "pr-v1@npm:pr-handling@^1"]),
            call(&npm, &["update", "--ignore-scripts", "--latest"]),
            // The user's own `--ignore-scripts=false` replaces the default, so only one flag reaches pnpm.
            call(&npm, &["add", "--ignore-scripts=false", "file:../x"]),
        ]
    );
}

#[cfg(unix)]
#[test]
fn uninstall_and_update_pass_their_arguments() {
    let sb = Sandbox::new("remove");
    let pnpm = stub_pnpm(&sb);
    let npm = sb.repo().join("rein/npm");
    for args in [&["uninstall", "pr-v1", "--silent"][..], &["update"], &["update", "pr-v2"]] {
        let out = sb.rein(&sb.repo(), args).env("REIN_PNPM", &pnpm).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
    assert_eq!(
        calls(&sb),
        [
            call(&npm, &["remove", "pr-v1", "--silent"]),
            call(&npm, &["update", "--ignore-scripts"]),
            call(&npm, &["update", "--ignore-scripts", "pr-v2"]),
        ]
    );
}

#[cfg(unix)]
#[test]
fn pnpm_exit_code_and_stderr_pass_through() {
    let sb = Sandbox::new("exit");
    let pnpm = stub_pnpm(&sb);
    let out = sb.rein(&sb.work(), &["uninstall", "-g", "missing-pkg"]).env("REIN_PNPM", &pnpm).output().unwrap();
    assert_eq!(out.status.code(), Some(7));
    assert!(String::from_utf8_lossy(&out.stderr).contains("stub: no such package"));
}

#[cfg(unix)]
#[test]
fn missing_pnpm_is_a_clear_error() {
    let sb = Sandbox::new("nopnpm");
    let out = sb
        .rein(&sb.repo(), &["install", "pr-handling"])
        .env("REIN_PNPM", sb.base.join("no-such-pnpm"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("pnpm is not installed"));
}

#[test]
fn install_needs_arguments() {
    let sb = Sandbox::new("noargs");
    let out = sb.rein(&sb.repo(), &["install"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}
