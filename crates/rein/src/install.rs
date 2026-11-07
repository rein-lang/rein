//! `rein install <tool>`: writes the tool's hook config so it runs `rein hook`.
//!
//! Only `claude-code` is supported. polyhook's `tools.toml` gives docs links for the other tools but
//! no config file location, so the others print "not supported yet".
//! The merge keeps every other key and every other hook. A second run adds nothing.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The command the agent runs. `--caller` tells polyhook which agent sends the event.
pub const HOOK_COMMAND: &str = "rein hook --caller claude-code";

/// The events that run `rein hook`: the session start marks it, and the first prompt gets the context.
const EVENTS: [&str; 2] = ["SessionStart", "UserPromptSubmit"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Project,
    User,
}

const OK: u8 = 0;
const FAILED: u8 = 1;

pub fn run(tool: &str, target: Target, dry_run: bool) -> u8 {
    if tool != "claude-code" {
        eprintln!("error: install for `{tool}` is not supported yet; only claude-code is");
        return FAILED;
    }
    let path = match settings_path(target) {
        Ok(p) => p,
        Err(msg) => return fail(&msg),
    };
    let existing = match read_settings(&path) {
        Ok(v) => v,
        Err(msg) => return fail(&msg),
    };
    let merged = match merge(existing, HOOK_COMMAND) {
        Ok(v) => v,
        Err(msg) => return fail(&format!("{}: {msg}", path.display())),
    };
    let text = format!("{}\n", serde_json::to_string_pretty(&merged).unwrap_or_default());
    if dry_run {
        print!("{text}");
        return OK;
    }
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return fail(&format!("cannot create {}: {e}", parent.display()));
        }
    }
    if let Err(e) = fs::write(&path, &text) {
        return fail(&format!("cannot write {}: {e}", path.display()));
    }
    println!("installed {HOOK_COMMAND} in {}", path.display());
    OK
}

fn fail(message: &str) -> u8 {
    eprintln!("error: {message}");
    FAILED
}

/// `.claude/settings.json` under the working directory, or under `$HOME` for the user target.
fn settings_path(target: Target) -> Result<PathBuf, String> {
    match target {
        Target::Project => Ok(PathBuf::from(".claude").join("settings.json")),
        Target::User => env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".claude").join("settings.json"))
            .ok_or_else(|| "HOME is not set".to_string()),
    }
}

/// The settings object at `path`, or an empty object when the file does not exist yet.
fn read_settings(path: &Path) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{} is not valid JSON: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// Adds the hook command under each event that lacks it. Other keys and hooks are kept as they are.
fn merge(mut settings: Value, command: &str) -> Result<Value, String> {
    let root = settings.as_object_mut().ok_or("settings must be a JSON object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("`hooks` must be a JSON object")?;
    for event in EVENTS {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("`hooks.{event}` must be a JSON array"))?;
        if !groups.iter().any(|g| has_command(g, command)) {
            groups.push(json!({ "hooks": [{ "type": "command", "command": command }] }));
        }
    }
    Ok(settings)
}

/// True when the matcher group already runs `command`.
fn has_command(group: &Value, command: &str) -> bool {
    let Some(Value::Array(handlers)) = group.get("hooks") else { return false };
    handlers.iter().any(|h| h.get("command").and_then(Value::as_str) == Some(command))
}
