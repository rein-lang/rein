//! `rein hook`: the agent hook entry point. Reads one event on stdin, answers on stdout.
//!
//! Injection works in two steps, because polyhook 0.2.0 cannot write a `SessionStart` context
//! response in Claude Code's format (it names the event `UserPromptSubmit` or `BeforeAgent`):
//! - `SessionStart` marks the session as pending in a state folder, and writes nothing.
//! - The first `UserPromptSubmit` of a pending session answers with the agent context. polyhook writes
//!   that response in the right format.
//!
//! Every other event is a no-op: no stdout, exit 0. Polyhook's approve response on `PreToolUse` means
//! "allow", which would skip the user's permission prompt. Any error is a no-op too, with the reason on
//! stderr. A hook must never block the agent.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use polyhook::{HookEventEvent, HookResponse};
use serde_json::Value;

use crate::scopes;

/// The folder that holds one state file per session.
fn state_dir() -> PathBuf {
    std::env::temp_dir().join("rein-sessions")
}

/// The session id as a file name. Only letters, digits, `-` and `_` are kept, so no path escapes.
fn session_key(raw: &Value) -> Option<String> {
    let id = raw.get("session_id")?.as_str()?;
    let key: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (!key.is_empty()).then_some(key)
}

/// Marks the session pending. The next prompt gets the context.
fn mark_pending(dir: &Path, key: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join(key), "pending")
}

/// True once per pending session: it flips the state to delivered and reports true.
fn take_pending(dir: &Path, key: &str) -> io::Result<bool> {
    let path = dir.join(key);
    if fs::read_to_string(&path).ok().as_deref() != Some("pending") {
        return Ok(false);
    }
    fs::write(&path, "delivered")?;
    Ok(true)
}

/// Runs the hook. `caller` sets polyhook's caller (`claude-code`), so the response uses its format.
pub fn run(caller: Option<&str>) -> u8 {
    if let Some(name) = caller {
        // Edition 2021: setting a variable is safe here, before polyhook reads it.
        std::env::set_var("POLYHOOK_CALLER", name);
    }
    let mut bytes = Vec::new();
    if let Err(e) = io::stdin().read_to_end(&mut bytes) {
        eprintln!("rein hook: cannot read stdin: {e}");
        return 0;
    }
    let raw: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rein hook: payload is not JSON: {e}");
            return 0;
        }
    };
    let event = match polyhook::read_from(&mut bytes.as_slice()) {
        Ok(ev) => ev,
        Err(e) => {
            eprintln!("rein hook: polyhook cannot read the event: {e}");
            return 0;
        }
    };
    let name = raw.get("hook_event_name").and_then(Value::as_str).unwrap_or("");
    let key = session_key(&raw);
    let dir = state_dir();

    if name == "SessionStart" {
        if let Some(key) = &key {
            if let Err(e) = mark_pending(&dir, key) {
                eprintln!("rein hook: cannot record the session: {e}");
            }
        }
        return 0;
    }
    if event.event != HookEventEvent::PromptSubmit {
        return 0;
    }
    let Some(key) = key else { return 0 };
    match take_pending(&dir, &key) {
        Ok(true) => {}
        Ok(false) => return 0,
        Err(e) => {
            eprintln!("rein hook: cannot read the session state: {e}");
            return 0;
        }
    }
    let cwd = raw.get("cwd").and_then(Value::as_str).map(PathBuf::from);
    let text = context_for(cwd.as_deref());
    if text.is_empty() {
        return 0;
    }
    if let Err(e) = polyhook::respond_to(&mut io::stdout(), &HookResponse::context(&text)) {
        eprintln!("rein hook: cannot write the response: {e}");
    }
    0
}

/// The agent context for the flows visible from `cwd`, or the current directory. Empty when none.
fn context_for(cwd: Option<&Path>) -> String {
    let dir = match cwd {
        Some(c) => std::path::absolute(c).unwrap_or_else(|_| c.to_path_buf()),
        None => std::env::current_dir().unwrap_or_default(),
    };
    let found = rein_core::flows::discover(&dir, scopes::user_dir().as_deref(), &scopes::system_dirs());
    rein_core::flows::agent_context(&found)
}
