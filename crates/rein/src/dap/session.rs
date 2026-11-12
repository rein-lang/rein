//! One debug session: a program walked by the step engine, with breakpoints, stops and answers.
//!
//! The step engine is stateless. A walk runs `rein_core::step::step` from the entry with every answer so far. It
//! returns at the first question, request or end it reaches, and follows hand-offs on the way. The answers and the
//! breakpoints live here, in the adapter. Nothing is evaluated outside the engine.
//!
//! The walk is synchronous. It runs until it stops, asks, or ends, then returns. A `rein/ask` event waits for a
//! `rein/answer` request. The answer resumes the walk.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rein_core::project::Project;
use rein_core::step::{step, Answers, StepRequest, StepResponse, PROTOCOL_VERSION};
use serde_json::{json, Value};

use super::lines;
use super::wire::{arguments, command, Sender};
use crate::scopes;

const THREAD: i64 = 1;
// Variable references of the three scopes.
const INPUTS: i64 = 1;
const ANSWERS: i64 = 2;
const CURRENT: i64 = 3;

/// How a walk treats the first node it reaches.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Stop at the first node, with reason `entry`.
    Entry,
    /// Run to the next question, request, breakpoint or end.
    Continue,
    /// Stop at the next question or request, with reason `step`.
    Next,
}

impl Mode {
    /// The mode a walk resumes in after an answer. The entry stop is over, so the walk runs on.
    fn after_answer(self) -> Mode {
        if self == Mode::Entry {
            Mode::Continue
        } else {
            self
        }
    }
}

/// What the walk waits for: an answer to a question or request, or a `string` input.
#[derive(Clone)]
enum Waiting {
    Node { key: String, mode: Mode },
    Input { name: String, mode: Mode },
}

/// One stack frame: a function, its file, and the line of the node the walk is at.
struct Frame {
    function: String,
    file: PathBuf,
    line: usize,
}

/// Where a walk arrived: its trace, the line of the node, the event body, and what it waits for.
struct Arrival {
    trace: Vec<String>,
    line: usize,
    body: Value,
    waiting: Option<Waiting>,
}

#[derive(Default)]
pub struct Session {
    sender: Sender,
    out: Vec<Value>,
    done: bool,
    project: Option<Rc<Project>>,
    entry: String,
    /// The `string` inputs of the entry function, by name.
    inputs: BTreeMap<String, String>,
    answers: Answers,
    stop_on_entry: bool,
    /// Run without the debugger: no breakpoints and no stops. Questions are still asked.
    no_debug: bool,
    breakpoints: BTreeMap<PathBuf, BTreeSet<usize>>,
    waiting: Option<Waiting>,
    frames: Vec<Frame>,
    /// The variables of the `Current step` scope.
    current: Vec<(String, String)>,
    /// How many trace entries the last arrival passed. Hand-offs past this count are new.
    seen: usize,
}

impl Session {
    /// Whether the client ended the session. The server exits after the reply.
    pub fn finished(&self) -> bool {
        self.done
    }

    /// Handles one client request. Returns every message to send back, in order: the response, then events.
    pub fn handle(&mut self, request: &Value) -> Vec<Value> {
        let args = arguments(request);
        match command(request) {
            "initialize" => {
                self.reply(request, json!({"supportsConfigurationDoneRequest": true}));
                let event = self.sender.event("initialized", json!({}));
                self.out.push(event);
            }
            "launch" => match self.launch(&args) {
                Ok(()) => self.reply(request, json!({})),
                Err(message) => self.refuse(request, message),
            },
            "setBreakpoints" => {
                let body = self.set_breakpoints(&args);
                self.reply(request, body);
            }
            "configurationDone" => {
                self.reply(request, json!({}));
                let mode = if self.stop_on_entry {
                    Mode::Entry
                } else {
                    Mode::Continue
                };
                self.resume(mode, false);
            }
            "threads" => self.reply(
                request,
                json!({"threads": [{"id": THREAD, "name": "rein"}]}),
            ),
            "stackTrace" => {
                let body = self.stack_trace();
                self.reply(request, body);
            }
            "scopes" => {
                let scopes = json!({"scopes": [scope("Inputs", INPUTS), scope("Answers", ANSWERS), scope("Current step", CURRENT)]});
                self.reply(request, scopes);
            }
            "variables" => {
                let reference = args
                    .get("variablesReference")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let body = self.variables(reference);
                self.reply(request, body);
            }
            "continue" => {
                self.reply(request, json!({"allThreadsContinued": true}));
                self.walk(Mode::Continue, true);
            }
            "next" | "stepIn" | "stepOut" => {
                self.reply(request, json!({}));
                self.walk(Mode::Next, true);
            }
            "rein/answer" => match self.accept(&args) {
                Ok(mode) => {
                    self.reply(request, json!({}));
                    self.resume(mode, false);
                }
                Err(message) => self.refuse(request, message),
            },
            "disconnect" | "terminate" => {
                self.reply(request, json!({}));
                self.done = true;
            }
            "evaluate" => self.refuse(request, "evaluate is not supported"),
            other => self.refuse(request, format!("unsupported request `{other}`")),
        }
        std::mem::take(&mut self.out)
    }

    fn reply(&mut self, request: &Value, body: Value) {
        let message = self.sender.ok(request, body);
        self.out.push(message);
    }

    fn refuse(&mut self, request: &Value, message: impl Into<String>) {
        let message = self.sender.fail(request, message);
        self.out.push(message);
    }

    fn launch(&mut self, args: &Value) -> Result<(), String> {
        let program = args
            .get("program")
            .and_then(Value::as_str)
            .ok_or("launch needs `program`")?;
        let (root, default_entry) = scopes::locate(program)?;
        let project = scopes::load_linked(&root)
            .map_err(|e| format!("cannot read {}: {e}", root.display()))?;
        self.entry = args
            .get("entry")
            .and_then(Value::as_str)
            .map_or(default_entry, str::to_string);
        self.inputs = pairs(args.get("inputs"));
        self.answers = pairs(args.get("answers"));
        self.stop_on_entry = args
            .get("stopOnEntry")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.no_debug = args
            .get("noDebug")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.project = Some(Rc::new(project));
        self.seen = 1;
        Ok(())
    }

    fn set_breakpoints(&mut self, args: &Value) -> Value {
        let path = args
            .pointer("/source/path")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let lines: Vec<usize> = args
            .get("breakpoints")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|b| b.get("line")?.as_u64())
                    .map(|l| l as usize)
                    .collect()
            })
            .unwrap_or_default();
        self.breakpoints
            .insert(canonical(Path::new(path)), lines.iter().copied().collect());
        let verified: Vec<Value> = lines
            .iter()
            .map(|line| json!({"verified": true, "line": line}))
            .collect();
        json!({"breakpoints": verified})
    }

    /// Validates and stores an answer. Returns the mode to resume in. Nothing is stored when the answer is refused.
    fn accept(&mut self, args: &Value) -> Result<Mode, String> {
        let key = args
            .get("key")
            .and_then(Value::as_str)
            .ok_or("rein/answer needs `key`")?
            .to_string();
        let value = args.get("value").map(text).unwrap_or_default();
        match self.waiting.clone() {
            Some(Waiting::Node { key: waiting, mode }) if waiting == key => {
                let previous = self.answers.insert(key.clone(), value);
                if let Some(message) = self.refusal(&key) {
                    match previous {
                        Some(old) => self.answers.insert(key, old),
                        None => self.answers.remove(&key),
                    };
                    return Err(message);
                }
                self.waiting = None;
                Ok(mode)
            }
            Some(Waiting::Input { name, mode }) if name == key => {
                self.inputs.insert(name, value);
                self.waiting = None;
                Ok(mode)
            }
            _ => Err(format!("no question is waiting for `{key}`")),
        }
    }

    /// The step engine's error for `key`, if it refuses the answers so far.
    fn refusal(&self, key: &str) -> Option<String> {
        let project = self.project.as_ref()?;
        match step(project, &self.request()) {
            StepResponse::Error {
                message,
                key: Some(k),
                ..
            } if k == key => Some(message),
            _ => None,
        }
    }

    /// Asks for the next missing `string` input, or walks the program.
    fn resume(&mut self, mode: Mode, skip: bool) {
        if let Some((name, description)) = self.missing_input() {
            self.waiting = Some(Waiting::Input {
                name: name.clone(),
                mode,
            });
            let body = json!({"key": name, "kind": "input", "name": name, "type": "string", "question": description});
            let event = self.sender.event("rein/ask", body);
            self.out.push(event);
            return;
        }
        self.walk(mode, skip);
    }

    /// The first `string` input of the entry that has no value yet, with its description.
    fn missing_input(&self) -> Option<(String, String)> {
        let project = self.project.as_ref()?;
        let function = &project.get(&self.entry)?.function;
        function
            .text_inputs()
            .find(|i| !self.inputs.contains_key(&i.name.value))
            .map(|i| (i.name.value.clone(), i.description.value.clone()))
    }

    /// Runs the step engine once and records where it arrived. `skip` leaves out a breakpoint on the node the
    /// walk stopped at last time, so a continue does not stop there again.
    fn walk(&mut self, mode: Mode, skip: bool) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let arrival = match step(&project, &self.request()) {
            StepResponse::Error { message, .. } => {
                let output = self.sender.event(
                    "output",
                    json!({"category": "stderr", "output": format!("{message}\n")}),
                );
                self.out.push(output);
                self.end();
                return;
            }
            StepResponse::Done {
                instruction, trace, ..
            } => {
                let line = trace
                    .last()
                    .map_or(1, |f| lines::done(&project, f, &instruction));
                let body = json!({"kind": "done", "instruction": instruction});
                Arrival {
                    trace,
                    line,
                    body,
                    waiting: None,
                }
            }
            StepResponse::Question {
                key,
                function,
                name,
                r#type,
                options,
                description,
                trace,
                ..
            } => {
                let line = project
                    .get(&function)
                    .map_or(1, |sf| lines::question(sf, &name));
                let body = json!({"key": key, "kind": "question", "name": name, "type": r#type, "options": options, "question": description, "function": function});
                let waiting = Waiting::Node {
                    key,
                    mode: mode.after_answer(),
                };
                Arrival {
                    trace,
                    line,
                    body,
                    waiting: Some(waiting),
                }
            }
            StepResponse::Request {
                key,
                function,
                instruction,
                response,
                trace,
                ..
            } => {
                let line = project
                    .get(&function)
                    .map_or(1, |sf| lines::request(sf, &response.name));
                let body = json!({"key": key, "kind": "request", "name": response.name, "type": response.r#type, "options": response.options, "question": response.description, "instruction": instruction, "fields": response.fields});
                let waiting = Waiting::Node {
                    key,
                    mode: mode.after_answer(),
                };
                Arrival {
                    trace,
                    line,
                    body,
                    waiting: Some(waiting),
                }
            }
        };
        self.arrive(&project, arrival, mode, skip);
    }

    fn arrive(&mut self, project: &Project, at: Arrival, mode: Mode, skip: bool) {
        let file = at
            .trace
            .last()
            .and_then(|f| project.get(f))
            .map(|sf| sf.path.clone())
            .unwrap_or_default();
        let hit = !skip && !self.no_debug && self.hit(project, &at.trace, &file, at.line);
        self.frames = frames(project, &at.trace, at.line);
        self.current = current(&at.body, &file, at.line);
        self.seen = at.trace.len();
        self.waiting = at.waiting.clone();
        let Some(_) = at.waiting else {
            if hit {
                self.stop("breakpoint");
            } else {
                let instruction = at.body.get("instruction").map(text).unwrap_or_default();
                let output = self.sender.event(
                    "output",
                    json!({"category": "console", "output": format!("{instruction}\n")}),
                );
                self.out.push(output);
                self.end();
            }
            return;
        };
        if hit {
            self.stop("breakpoint");
        } else if mode != Mode::Continue && !skip {
            self.stop(if mode == Mode::Entry { "entry" } else { "step" });
        } else {
            if !skip {
                self.stop("pause");
            }
            let ask = self.sender.event("rein/ask", at.body);
            self.out.push(ask);
        }
    }

    /// Whether a breakpoint is set on the node at `line` of `file`, or on a hand-off the walk passed since the last
    /// arrival. A hand-off stop is reported at the node the walk reaches after it.
    fn hit(&self, project: &Project, trace: &[String], file: &Path, line: usize) -> bool {
        if self.breakpoint_at(file, line) {
            return true;
        }
        (self.seen.max(1)..trace.len()).any(|i| {
            let caller = &trace[i - 1];
            let Some(sf) = project.get(caller) else {
                return false;
            };
            self.breakpoint_at(&sf.path, lines::handoff(project, caller, &trace[i]))
        })
    }

    fn breakpoint_at(&self, file: &Path, line: usize) -> bool {
        self.breakpoints
            .get(&canonical(file))
            .is_some_and(|lines| lines.contains(&line))
    }

    fn stop(&mut self, reason: &str) {
        // ponytail: with no_debug the client gets no stopped event, so VS Code shows the program as running.
        if self.no_debug {
            return;
        }
        let event = self.sender.event(
            "stopped",
            json!({"reason": reason, "threadId": THREAD, "allThreadsStopped": true}),
        );
        self.out.push(event);
    }

    fn end(&mut self) {
        self.waiting = None;
        let event = self.sender.event("terminated", json!({}));
        self.out.push(event);
    }

    fn stack_trace(&self) -> Value {
        let frames: Vec<Value> = self
            .frames
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let name = f
                    .file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                json!({
                    "id": i + 1,
                    "name": f.function,
                    "source": {"name": name, "path": f.file.display().to_string()},
                    "line": f.line,
                    "column": 1,
                })
            })
            .collect();
        json!({"stackFrames": frames, "totalFrames": self.frames.len()})
    }

    fn variables(&self, reference: i64) -> Value {
        let pairs: Vec<(String, String)> = match reference {
            INPUTS => self
                .inputs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            ANSWERS => self
                .answers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            CURRENT => self.current.clone(),
            _ => Vec::new(),
        };
        let variables: Vec<Value> = pairs
            .into_iter()
            .map(|(name, value)| json!({"name": name, "value": value, "variablesReference": 0}))
            .collect();
        json!({"variables": variables})
    }

    fn request(&self) -> StepRequest {
        StepRequest {
            version: PROTOCOL_VERSION,
            entry: self.entry.clone(),
            answers: self.answers.clone(),
            flow: None,
            inputs: self.inputs.clone(),
        }
    }
}

fn scope(name: &str, reference: i64) -> Value {
    json!({"name": name, "variablesReference": reference, "expensive": false})
}

/// The stack, top frame first. Each caller's line is its hand-off to the frame above it.
fn frames(project: &Project, trace: &[String], line: usize) -> Vec<Frame> {
    let mut out = Vec::new();
    for i in (0..trace.len()).rev() {
        let Some(sf) = project.get(&trace[i]) else {
            continue;
        };
        let at = if i + 1 == trace.len() {
            line
        } else {
            lines::handoff(project, &trace[i], &trace[i + 1])
        };
        out.push(Frame {
            function: trace[i].clone(),
            file: sf.path.clone(),
            line: at,
        });
    }
    out
}

/// The `Current step` variables: the event body, then the file and line of the node.
fn current(body: &Value, file: &Path, line: usize) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = body
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), text(v))).collect())
        .unwrap_or_default();
    vars.push(("file".to_string(), file.display().to_string()));
    vars.push(("line".to_string(), line.to_string()));
    vars
}

/// A JSON value as the text the user sees. Strings are unquoted, lists joined, null is empty.
fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Array(items) => items.iter().map(text).collect::<Vec<_>>().join(" | "),
        other => other.to_string(),
    }
}

/// The entries of a JSON object as text pairs. Missing or non-object values give no pairs.
fn pairs(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|o| o.iter().map(|(k, v)| (k.clone(), text(v))).collect())
        .unwrap_or_default()
}

/// The path with symlinks resolved, or as given when it does not exist. Breakpoints match on this form.
fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
