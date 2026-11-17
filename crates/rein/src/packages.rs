//! Program packages: `rein install`, `rein uninstall` and `rein update`. Each passes its arguments to pnpm, in the npm
//! folder of the scope. `-g` or `--global` picks the user programs folder and is not passed on. Without it, the
//! folder is the repo's `rein/npm/`. The npm folder and its `package.json` are made on first use.

use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rein_core::flows::{NPM_DIR, PACKAGE};
use serde_json::Value;

use crate::programs::triple;
use crate::scopes;

const FAILED: u8 = 1;

/// The project manifest. It is private, so `npm publish` refuses it. pnpm adds the dependencies.
const PROJECT_MANIFEST: &str = "{\n  \"name\": \"rein-programs\",\n  \"private\": true\n}\n";

/// Runs `pnpm <verb> <args>` in the npm folder of the scope the arguments name. `verb` is `add`, `remove` or
/// `update`. `add` and `update` get `--ignore-scripts` first, unless the arguments already set it (for example
/// `--ignore-scripts=false`). pnpm's output goes straight to this terminal, and its exit code is ours.
/// `REIN_PNPM` names another pnpm binary.
pub fn run(verb: &str, args: &[String]) -> u8 {
    let global = args.iter().any(|a| is_global(a));
    let rest: Vec<&str> = args.iter().map(String::as_str).filter(|a| !is_global(a)).collect();
    let dir = match npm_dir(global) {
        Ok(dir) => dir,
        Err(message) => return fail(&message),
    };
    let mut pnpm_args = vec![verb];
    if verb != "remove" && !rest.iter().any(|a| a.starts_with("--ignore-scripts")) {
        pnpm_args.push("--ignore-scripts");
    }
    pnpm_args.extend(rest.clone());
    let bin = env::var("REIN_PNPM").unwrap_or_else(|_| "pnpm".to_string());
    let mut moved: Vec<String> = Vec::new();
    if verb == "update" {
        match move_tag_pins(&bin, &dir, &package_names(&rest), &git_tags) {
            Ok(done) => moved = done,
            Err(message) => return fail(&message),
        }
    }
    let code = match Command::new(&bin).args(&pnpm_args).current_dir(&dir).status() {
        Ok(status) => status.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(FAILED),
        Err(e) => return fail(&after_moves(&spawn_error(&bin, &e), &moved)),
    };
    if code != 0 && !moved.is_empty() {
        eprintln!("error: {}", after_moves(&format!("pnpm update exited with {code}"), &moved));
    }
    code
}

fn is_global(arg: &str) -> bool {
    arg == "-g" || arg == "--global"
}

/// The package names among the arguments. A flag is no name, and neither is a flag's value such as the `0` of
/// `--depth 0`. A name starts with a letter, a digit or `@`, and has a letter in it.
fn package_names<'a>(args: &[&'a str]) -> Vec<&'a str> {
    args.iter().copied().filter(|a| is_package_name(a)).collect()
}

fn is_package_name(arg: &str) -> bool {
    arg.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '@') && arg.contains(|c: char| c.is_ascii_alphabetic())
}

/// `<user programs folder>/npm/` for `-g`, else `<project rein folder>/npm/`. Both are created with their
/// `package.json` when missing.
fn npm_dir(global: bool) -> Result<PathBuf, String> {
    let base = if global {
        scopes::user_dir().ok_or("no user programs folder; set HOME or REIN_HOME")?
    } else {
        let cwd = env::current_dir().map_err(|e| format!("cannot read the working directory: {e}"))?;
        scopes::project_rein_dir(&cwd)?
    };
    let dir = base.join(NPM_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let manifest = dir.join(PACKAGE);
    if !manifest.exists() {
        write_manifest(&manifest)?;
    }
    Ok(dir)
}

fn write_manifest(path: &Path) -> Result<(), String> {
    fs::write(path, PROJECT_MANIFEST).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// `rein update` moves each tag-pinned GitHub program to the newest `v<x.y.z>` tag of its line (its major, or under
/// major 0 its minor). `pnpm update` keeps a git tag fixed, so it cannot do this. `names` limits the programs, as
/// `pnpm update` does. No names means all of them. A program whose tags cannot be read is skipped with a warning.
/// `tags_of` gives a repo's `git ls-remote` output. Returns the names of the programs moved. `pnpm update` then runs
/// as before.
fn move_tag_pins(
    bin: &str,
    dir: &Path,
    names: &[&str],
    tags_of: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Vec<String>, String> {
    let path = dir.join(PACKAGE);
    let text = fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let manifest: Value = serde_json::from_str(&text).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    for arg in names.iter().filter(|arg| is_range(arg)) {
        eprintln!("note: {arg} names a version range, so rein update moves no tag pin for it");
    }
    let mut moved: Vec<&str> = Vec::new();
    for pin in tag_pins(&manifest, names) {
        let tags = match tags_of(pin.repo) {
            Ok(tags) => tags,
            Err(message) => {
                eprintln!("warning: {} is not moved: {message}", pin.name);
                continue;
            }
        };
        let Some(tag) = move_target(pin.current, &tags) else {
            if let Some(note) = unmoved_note(&pin, &tags) {
                eprintln!("{note}");
            }
            continue;
        };
        let status = match Command::new(bin).args(add_args(&pin, &tag)).current_dir(dir).status() {
            Ok(status) => status,
            Err(e) => return Err(move_error(&spawn_error(bin, &e), &moved)),
        };
        if !status.success() {
            return Err(move_error(&format!("pnpm add {} failed", spec(&pin, &tag)), &moved));
        }
        moved.push(pin.name);
        for note in move_notes(&pin, &tag, &tags) {
            eprintln!("{note}");
        }
    }
    Ok(moved.into_iter().map(String::from).collect())
}

/// The notes after a pin moves to `tag`: the move itself, and a note when a newer line still exists past `tag`.
fn move_notes(pin: &Pin, tag: &str, ls_remote: &str) -> Vec<String> {
    let mut notes = vec![format!("note: {} moved to {tag}", pin.name)];
    if let Some(now) = version(tag) {
        notes.extend(unmoved_note(&Pin { current: now, ..*pin }, ls_remote));
    }
    notes
}

/// The manifest fields that can hold a dependency. `rein install -D` and `-O` write to the last two.
const SECTIONS: [&str; 3] = ["dependencies", "devDependencies", "optionalDependencies"];

/// A program pinned to a GitHub tag: its name in the manifest, its repo, its version, and the field it is in.
struct Pin<'a> {
    name: &'a str,
    repo: &'a str,
    current: [u64; 3],
    section: &'static str,
}

/// The tag-pinned programs that `rein update` may move. `names` limits them, as in `selected`.
fn tag_pins<'a>(manifest: &'a Value, names: &[&str]) -> Vec<Pin<'a>> {
    let mut pins = Vec::new();
    for section in SECTIONS {
        let Some(deps) = manifest[section].as_object() else {
            continue;
        };
        for (name, spec) in deps {
            if !selected(names, name) {
                continue;
            }
            if let Some((repo, current)) = tag_pin(spec.as_str().unwrap_or_default()) {
                pins.push(Pin { name: name.as_str(), repo, current, section });
            }
        }
    }
    pins
}

/// The `pnpm add` arguments that move a pin to `tag`. The save flag keeps the program in its manifest field.
fn add_args(pin: &Pin, tag: &str) -> Vec<String> {
    let mut args = vec!["add".to_string(), "--ignore-scripts".to_string()];
    match pin.section {
        "devDependencies" => args.push("--save-dev".to_string()),
        "optionalDependencies" => args.push("--save-optional".to_string()),
        _ => {}
    }
    args.push(spec(pin, tag));
    args
}

/// The `<key>@github:<owner>/<repo>#<tag>` spec. The key is the manifest name, so `pnpm add` keeps that entry and
/// does not add one named after the repo's package.
fn spec(pin: &Pin, tag: &str) -> String {
    format!("{}@github:{}#{tag}", pin.name, pin.repo)
}

/// The repo and version of a `github:<owner>/<repo>#v<x.y.z>` spec. `None` for any other spec.
fn tag_pin(spec: &str) -> Option<(&str, [u64; 3])> {
    let (repo, tag) = spec.strip_prefix("github:")?.split_once('#')?;
    let valid = repo.matches('/').count() == 1 && repo.split('/').all(|part| !part.is_empty());
    valid.then_some((repo, version(tag)?))
}

/// `v<x.y.z>` as numbers. Pre-releases and other tags give `None`. Each part is digits only, so `v+1.0.0` is no tag.
fn version(tag: &str) -> Option<[u64; 3]> {
    let parts = tag.strip_prefix('v')?;
    if parts.bytes().all(|b| b == b'.' || b.is_ascii_digit()) {
        triple(parts)
    } else {
        None
    }
}

/// Whether `rein update` moves the program `name`, given the names on the command line. No names means all. A
/// version range in the names selects no tag move, see `is_range`.
fn selected(names: &[&str], name: &str) -> bool {
    names.is_empty() || names.iter().any(|arg| bare_name(arg) == name && !is_range(arg))
}

/// The package name in an argument such as `name@latest` or `@scope/name@^1`: the text before the `@` that starts
/// the version or tag. The `@` that starts a scope stays.
fn bare_name(arg: &str) -> &str {
    split_version(arg).0
}

/// Whether an argument names a version range such as `name@^1`, not a tag. `pnpm update` applies a range to the
/// program's spec, and a tag pin has no spec to apply it to, so the range selects no tag move. `name@latest` is no
/// range.
fn is_range(arg: &str) -> bool {
    split_version(arg).1.is_some_and(|v| v != "latest")
}

/// The name and the version or tag part of an argument. The version part is `None` when no `@` follows the name.
fn split_version(arg: &str) -> (&str, Option<&str>) {
    match arg.get(1..).and_then(|rest| rest.find('@')) {
        Some(i) => (&arg[..i + 1], Some(&arg[i + 2..])),
        None => (arg, None),
    }
}

/// The note or warning for a pin that stays where it is, when a newer tag exists that `move_target` does not take.
/// `None` when the pin is the newest of its line, or the note is not needed.
fn unmoved_note(pin: &Pin, ls_remote: &str) -> Option<String> {
    match newest_tag(ls_remote, |_| true) {
        None => Some(format!("warning: {} is not moved: {} has no v<x.y.z> tag", pin.name, pin.repo)),
        Some((top, v)) if v > pin.current && !same_line(pin.current, v) => {
            let kind = if v[0] != pin.current[0] { "a new major" } else { "a new minor" };
            Some(format!(
                "note: {} stays on {}; {} has {top}, {kind}. To move, run: {}",
                pin.name,
                tag_text(pin.current),
                pin.repo,
                install_command(pin, &top)
            ))
        }
        Some(_) => None,
    }
}

/// The `rein install` command that moves a pin to `top`. A pin in `devDependencies` or `optionalDependencies` keeps
/// its field with `-D` or `-O`, as `rein install` takes them.
fn install_command(pin: &Pin, top: &str) -> String {
    let flag = match pin.section {
        "devDependencies" => "-D ",
        "optionalDependencies" => "-O ",
        _ => "",
    };
    format!("rein install {flag}github:{}#{top}", pin.repo)
}

/// `v<x.y.z>` for numbers.
fn tag_text(v: [u64; 3]) -> String {
    format!("v{}.{}.{}", v[0], v[1], v[2])
}

/// The tag a pin moves to: the highest `v<x.y.z>` tag on the pin's line, when it is higher than the pin. A higher
/// major is never taken, because it can break the program. Under major 0 a higher minor is not taken either, since
/// semver allows it to break the program.
fn move_target(current: [u64; 3], ls_remote: &str) -> Option<String> {
    let (tag, newest) = newest_tag(ls_remote, |v| same_line(current, v))?;
    (newest > current).then_some(tag)
}

/// Whether `v` is on the line that a pin at `current` may move within: its major, or under major 0 its minor.
fn same_line(current: [u64; 3], v: [u64; 3]) -> bool {
    if current[0] == 0 {
        v[0] == 0 && v[1] == current[1]
    } else {
        v[0] == current[0]
    }
}

/// The highest `v<x.y.z>` tag in `git ls-remote --tags --refs` output, with its tag name and numbers. Only the
/// tags that `keep` accepts count.
fn newest_tag(ls_remote: &str, keep: impl Fn([u64; 3]) -> bool) -> Option<(String, [u64; 3])> {
    ls_remote
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1)?.strip_prefix("refs/tags/"))
        .filter_map(|tag| Some((tag.to_string(), version(tag)?)))
        .filter(|(_, v)| keep(*v))
        .max_by_key(|(_, v)| *v)
}

/// How long one `git ls-remote` may run. A stalled connection then fails that program, and the update goes on.
const LS_REMOTE_LIMIT: Duration = Duration::from_secs(30);

/// The tags of a GitHub repo, as `git ls-remote` prints them.
fn git_tags(repo: &str) -> Result<String, String> {
    let url = format!("https://github.com/{repo}");
    let mut cmd = ls_remote_command(&url);
    let out = match output_within(&mut cmd, LS_REMOTE_LIMIT) {
        Ok(Some(out)) => out,
        Ok(None) => return Err(format!("git ls-remote {url} timed out after {} s", LS_REMOTE_LIMIT.as_secs())),
        Err(e) => return Err(format!("cannot run git: {e}")),
    };
    if !out.status.success() {
        return Err(format!("git ls-remote {url} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The `git ls-remote` command for `url`. Prompts are off, so a private repo fails. The HTTP helper of git runs on
/// after git is killed at the time limit, so git also gives up when a transfer stays under 1 byte a second for 20 s.
fn ls_remote_command(url: &str) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(["ls-remote", "--tags", "--refs", url])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_HTTP_LOW_SPEED_LIMIT", "1")
        .env("GIT_HTTP_LOW_SPEED_TIME", "20");
    cmd
}

/// Runs `cmd` and collects its output. `Ok(None)` when it runs past `limit`: the child is then killed.
fn output_within(cmd: &mut Command, limit: Duration) -> io::Result<Option<Output>> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = thread::spawn({
        let pipe = child.stdout.take();
        move || drain(pipe)
    });
    let stderr = thread::spawn({
        let pipe = child.stderr.take();
        move || drain(pipe)
    });
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait()? {
            let stdout = stdout.join().unwrap_or_default();
            let stderr = stderr.join().unwrap_or_default();
            return Ok(Some(Output { status, stdout, stderr }));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Reads a child's pipe to its end. A missing pipe gives no bytes.
fn drain(pipe: Option<impl Read>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut bytes);
    }
    bytes
}

fn fail(message: &str) -> u8 {
    eprintln!("error: {message}");
    FAILED
}

/// The error when a move stops. `why` says what failed. `moved` names the programs already moved, which stay moved.
fn move_error(why: &str, moved: &[&str]) -> String {
    let done = if moved.is_empty() { "none".to_string() } else { moved.join(", ") };
    format!("{why}. Already moved: {done}. pnpm update did not run.")
}

/// The error when `pnpm update` cannot run or fails after some programs moved. With no moves, `why` stands alone.
fn after_moves(why: &str, moved: &[String]) -> String {
    if moved.is_empty() {
        return why.to_string();
    }
    format!("{why}. Already moved: {}. They stay moved.", moved.join(", "))
}

/// The message when pnpm cannot start. A missing pnpm gets the install hint.
fn spawn_error(bin: &str, e: &io::Error) -> String {
    if e.kind() == io::ErrorKind::NotFound {
        "pnpm is not installed or not on PATH; install pnpm (https://pnpm.io) or set REIN_PNPM to its path".to_string()
    } else {
        format!("cannot run {bin}: {e}")
    }
}

#[cfg(test)]
#[path = "packages_tests.rs"]
mod tests;
