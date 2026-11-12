//! Program packages in `<user programs folder>/npm/`: discovery in `rein list`, and the pnpm commands. Each test
//! gets its own temp folders. The pnpm tests put a stub script in `REIN_PNPM`, so no network and no real pnpm run.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A temp folder with `home/`, `programs/` (the user programs folder) and `work/` (the command's cwd).
struct Sandbox {
    base: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("rein-packages-{tag}-{}-{nanos}-{n}", std::process::id()));
        fs::create_dir_all(base.join("home")).unwrap();
        fs::create_dir_all(base.join("work")).unwrap();
        Sandbox { base }
    }

    fn programs(&self) -> PathBuf {
        self.base.join("programs")
    }

    fn work(&self) -> PathBuf {
        self.base.join("work")
    }

    /// A `rein` command run from `cwd`. HOME, REIN_HOME and TMPDIR point into the sandbox.
    fn rein(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rein"));
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

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

const ENTRY: &str = "/** A program. */\nexport default function main() {\n  return \"ok\";\n}\n";

const ENTRY_WITH_QUESTION: &str = "/**\n * Plain tool.\n */\nexport default function main(\n  /** Is it urgent? */\n  urgent: boolean,\n) {\n  if (urgent) {\n    return \"yes\";\n  }\n  return \"no\";\n}\n";

/// Lays out one package the way pnpm does: the files under `node_modules/.pnpm/<store>/node_modules/<name>`,
/// and a symlink at `node_modules/<link>`. An npm alias has `link` different from `name`.
#[cfg(unix)]
fn pnpm_package(programs: &Path, store: &str, name: &str, link: &str, manifest: &str, entry: Option<&str>) {
    let modules = programs.join("npm/node_modules");
    let target = modules.join(".pnpm").join(store).join("node_modules").join(name);
    write(&target.join("package.json"), manifest);
    if let Some(text) = entry {
        write(&target.join("main.rein"), text);
    }
    let link_path = modules.join(link);
    fs::create_dir_all(link_path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &link_path).unwrap();
}

#[cfg(unix)]
#[test]
fn list_finds_packages_by_their_link_names() {
    let sb = Sandbox::new("list");
    let p = sb.programs();
    pnpm_package(&p, "plain-tool@1.0.0", "plain-tool", "plain-tool",
        r#"{"name": "plain-tool", "version": "1.0.0", "description": "A plain tool.", "engines": {"rein": "^0.1.0"}}"#,
        Some(ENTRY_WITH_QUESTION));
    pnpm_package(&p, "@scope+tool@2.0.0", "@scope/tool", "@scope/tool",
        r#"{"name": "@scope/tool", "version": "2.0.0", "description": "A scoped tool."}"#, Some(ENTRY));
    pnpm_package(&p, "pr@1.0.0", "pr", "pr-v1",
        r#"{"name": "pr", "version": "1.0.0", "description": "PR handling, version one.", "engines": {"rein": "^0.1.0"}}"#,
        Some(ENTRY));
    pnpm_package(&p, "pr@2.0.0", "pr", "pr-v2",
        r#"{"name": "pr", "version": "2.0.0", "description": "PR handling, version two.", "engines": {"rein": "^0.2.0"}}"#,
        Some(ENTRY));
    pnpm_package(&p, "lodash-like@4.0.0", "lodash-like", "lodash-like", r#"{"name": "lodash-like", "version": "4.0.0"}"#, None);

    let out = sb.rein(&sb.work(), &["list", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let list: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
    let names: Vec<&str> = list.iter().map(|f| f["name"].as_str().unwrap()).collect();
    // The non-Rein package and the `.pnpm` store are not programs. The two aliases are two programs.
    assert_eq!(names, ["@scope/tool", "plain-tool", "pr-v1", "pr-v2"]);

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
    let running = env!("CARGO_PKG_VERSION");
    assert_eq!(by("pr-v2")["description"], "PR handling, version two.");
    assert_eq!(by("pr-v2")["warning"], format!("needs rein ^0.2.0; this is rein {running}"));

    // A package runs by its link name, and `rein show` prints its source.
    let step = sb.rein(&sb.work(), &["step", "plain-tool", "--json"]).output().unwrap();
    assert_eq!(step.status.code(), Some(2));
    let step: Value = serde_json::from_slice(&step.stdout).unwrap();
    assert_eq!((step["kind"].as_str(), step["key"].as_str()), (Some("question"), Some("main.urgent")));
    let show = sb.rein(&sb.work(), &["show", "pr-v2"]).output().unwrap();
    assert_eq!(show.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&show.stdout).contains("// main.rein"));
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

/// The npm project folder as pnpm reports its cwd: canonical.
#[cfg(unix)]
fn npm_dir(sb: &Sandbox) -> String {
    fs::canonicalize(sb.programs().join("npm")).unwrap().display().to_string()
}

#[cfg(unix)]
#[test]
fn install_runs_pnpm_add_with_ignore_scripts_in_the_npm_folder() {
    let sb = Sandbox::new("install");
    let pnpm = stub_pnpm(&sb);
    let out = sb.rein(&sb.work(), &["install", "pr-handling@^1"]).env("REIN_PNPM", &pnpm).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(calls(&sb), [vec![npm_dir(&sb), "add".into(), "--ignore-scripts".into(), "pr-handling@^1".into()]]);

    // The first use made the folder's own manifest. It is private, so it cannot be published.
    let manifest: Value = serde_json::from_str(&fs::read_to_string(sb.programs().join("npm/package.json")).unwrap()).unwrap();
    assert_eq!(manifest["private"], true);
}

#[cfg(unix)]
#[test]
fn install_passes_every_source_to_pnpm_unchanged() {
    let sb = Sandbox::new("sources");
    let pnpm = stub_pnpm(&sb);
    for spec in ["github:owner/repo", "pr-v1@npm:pr-handling@^1", "file:../pr-handling"] {
        let out = sb.rein(&sb.work(), &["install", spec]).env("REIN_PNPM", &pnpm).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{spec}: {}", String::from_utf8_lossy(&out.stderr));
    }
    let args: Vec<String> = calls(&sb).iter().map(|c| c[3].clone()).collect();
    assert_eq!(args, ["github:owner/repo", "pr-v1@npm:pr-handling@^1", "file:../pr-handling"]);
}

#[cfg(unix)]
#[test]
fn uninstall_and_update_run_the_pnpm_commands() {
    let sb = Sandbox::new("remove");
    let pnpm = stub_pnpm(&sb);
    for args in [&["uninstall", "pr-v1"][..], &["update"], &["update", "pr-v2"]] {
        let out = sb.rein(&sb.work(), args).env("REIN_PNPM", &pnpm).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
    let npm = npm_dir(&sb);
    let expected = [
        vec![npm.clone(), "remove".into(), "pr-v1".into()],
        vec![npm.clone(), "update".into(), "--ignore-scripts".into()],
        vec![npm, "update".into(), "--ignore-scripts".into(), "pr-v2".into()],
    ];
    assert_eq!(calls(&sb), expected);
}

#[cfg(unix)]
#[test]
fn pnpm_exit_code_and_stderr_pass_through() {
    let sb = Sandbox::new("exit");
    let pnpm = stub_pnpm(&sb);
    let out = sb.rein(&sb.work(), &["uninstall", "missing-pkg"]).env("REIN_PNPM", &pnpm).output().unwrap();
    assert_eq!(out.status.code(), Some(7));
    assert!(String::from_utf8_lossy(&out.stderr).contains("stub: no such package"));
}

#[cfg(unix)]
#[test]
fn missing_pnpm_is_a_clear_error() {
    let sb = Sandbox::new("nopnpm");
    let out = sb.rein(&sb.work(), &["install", "pr-handling"]).env("REIN_PNPM", sb.base.join("no-such-pnpm")).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("pnpm is not installed"));
}

#[cfg(unix)]
#[test]
fn install_keeps_the_hook_and_the_folder_link_before_packages() {
    let sb = Sandbox::new("rule");
    let pnpm = stub_pnpm(&sb);

    // `claude-code` is the hook. It does not reach pnpm.
    let hook = sb.rein(&sb.work(), &["install", "claude-code", "--dry-run"]).env("REIN_PNPM", &pnpm).output().unwrap();
    assert_eq!(hook.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&hook.stdout).contains("rein hook --caller claude-code"));

    // An existing directory is linked. It does not reach pnpm.
    write(&sb.work().join("pr-handling/main.rein"), ENTRY);
    let link = sb.rein(&sb.work(), &["install", "pr-handling"]).env("REIN_PNPM", &pnpm).output().unwrap();
    assert_eq!(link.status.code(), Some(0), "{}", String::from_utf8_lossy(&link.stderr));
    assert!(sb.programs().join("pr-handling").symlink_metadata().is_ok());
    assert!(calls(&sb).is_empty());
}
