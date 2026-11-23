//! Command bodies for `check`, `eval`, `step` and `describe`. The live `rein run` is in `live_run`. `rein graph` is in `graph`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rein_core::ast::{Function, Input, InputType};
use rein_core::deprecation::IMPORT_REMOVAL_DATE;
use rein_core::diagnostic::{line_col, Diagnostic, Severity};
use rein_core::eval::{self, Outcome};
use rein_core::project::{Project, ENTRY};

use crate::run_log;
use crate::scopes;
use rein_core::step::{invalid_request, step as step_flow, StepRequest, StepResponse, PROTOCOL_VERSION};
use serde_json::{json, Value};

const OK: u8 = 0;
const FAILED: u8 = 1;
const NEED_INPUT: u8 = 2;
const REQUEST: u8 = 3;

/// One printed diagnostic line, kept with its sort keys.
struct Row {
    file: String,
    start: usize,
    severity: Severity,
    line: String,
    /// A deprecation warning, see `Diagnostic::is_deprecated`.
    deprecated: bool,
}

pub fn check(given: &Path, deny_deprecated: bool) -> u8 {
    if !given.exists() {
        return fail(false, &format!("no such file or directory: {}", given.display()));
    }
    if given.is_file() && given.extension().and_then(|e| e.to_str()) != Some("rein") {
        return fail(false, &format!("not a .rein file: {}", given.display()));
    }
    let scope = absolute(given);
    let root = Project::find_root(&scope);
    let project = match scopes::load_linked(&root) {
        Ok(p) => p,
        Err(e) => return fail(false, &format!("cannot read {}: {e}", root.display())),
    };
    let rows = rows(&project, &scope, deny_deprecated);
    let errors = rows.iter().filter(|r| r.severity == Severity::Error).count();
    let warnings = rows.len() - errors;
    let deprecated = rows.iter().filter(|r| r.deprecated).count();
    for r in &rows {
        println!("{}", r.line);
    }
    eprintln!("{errors} errors, {warnings} warnings");
    if deprecated > 0 {
        eprintln!("{deprecated} deprecated imports and references; they become errors on {IMPORT_REMOVAL_DATE}");
    }
    if errors > 0 {
        FAILED
    } else {
        OK
    }
}

pub fn eval(function: &str, root: Option<PathBuf>, sets: &[String], json: bool) -> u8 {
    let project = match load(root) {
        Ok(p) => p,
        Err(msg) => return fail(json, &msg),
    };
    let Some(sf) = project.get(function) else {
        return fail(json, &unknown_function(&project, function));
    };
    let errors: Vec<Row> = rows(&project, &sf.path, false)
        .into_iter()
        .filter(|r| r.severity == Severity::Error)
        .collect();
    if !errors.is_empty() {
        let message = format!("`{function}` has {} error(s); run `rein check` and fix them", errors.len());
        if json {
            let diagnostics: Vec<&str> = errors.iter().map(|r| r.line.as_str()).collect();
            print_json(json!({"kind": "error", "message": message, "diagnostics": diagnostics}));
        } else {
            for r in &errors {
                println!("{}", r.line);
            }
            eprintln!("error: {message}");
        }
        return FAILED;
    }
    let inputs = match parse_sets(sets) {
        Ok(i) => i,
        Err(msg) => return fail(json, &msg),
    };
    let bindings = project.bindings(function);
    match eval::run(&sf.function, &inputs, &bindings, false) {
        Err(msg) => fail(json, &msg),
        Ok(Outcome::Request { instruction, response }) => request(&sf.function, &instruction, &response, json),
        Ok(Outcome::Text(value)) => {
            if json {
                print_json(json!({"kind": "text", "value": value}));
            } else {
                println!("{value}");
            }
            OK
        }
        Ok(Outcome::Ref { target, .. }) => show_ref(&project, &target, json),
        Ok(Outcome::NeedInput(name)) => need_input(&sf.function, &name, json),
        Ok(Outcome::Call { target, response, .. }) => {
            let instruction = format!("Run `{target}` to its end. Its final instruction is the result.");
            request(&sf.function, &instruction, &response, json)
        }
    }
}

/// Steps the whole flow once. The request comes from flags, or from stdin as JSON. This is only an adapter:
/// `rein_core::step::step` decides everything. A flow name, from `--flow` or the stdin `flow` field, is
/// resolved through the scoped `rein/` folders.
pub fn step(entry: Option<String>, root: Option<PathBuf>, flow: Option<String>, answers: &[String], inputs: &[String], from_stdin: bool, json: bool) -> u8 {
    let built = build_request(entry, answers, inputs, from_stdin);
    let flow = flow.or_else(|| built.as_ref().ok().and_then(|r| r.flow.clone()));
    let program = flow.clone().or_else(|| root.as_ref().map(|r| r.display().to_string()));
    let (request, response) = match built {
        Err(message) => (empty_request(), invalid_request(message)),
        Ok(mut request) => {
            let response = match load_flow(root, flow) {
                Ok((project, entry)) => {
                    if request.entry == ENTRY {
                        request.entry = entry;
                    }
                    step_flow(&project, &request)
                }
                Err(message) => invalid_request(message),
            };
            (request, response)
        }
    };
    run_log::step(&request, program.as_deref(), &response);
    print_step(&response, json || from_stdin);
    exit_code(&response)
}

/// The request that a failed `rein step` logs, when the request itself could not be read.
fn empty_request() -> StepRequest {
    StepRequest { version: PROTOCOL_VERSION, entry: ENTRY.to_string(), answers: Default::default(), flow: None, inputs: Default::default(), walk: false }
}

/// The project and its entry function for `--root DIR`, a program name or `.rein` path, or the nearest root from
/// here. A root's entry is `entry` in `rein.json`, else `main`, as `rein check` uses it. A name's entry comes from
/// the program manifest. A file's entry is its name.
pub(crate) fn load_flow(root: Option<PathBuf>, flow: Option<String>) -> Result<(Project, String), String> {
    match (root, flow) {
        (Some(_), Some(_)) => Err("--flow and --root are mutually exclusive".to_string()),
        (None, Some(name)) => {
            let (root, entry) = scopes::locate(&name)?;
            Ok((load(Some(root))?, entry))
        }
        (root, None) => {
            let project = load(root)?;
            let entry = project.entry().to_string();
            Ok((project, entry))
        }
    }
}

/// The exit code of a step response (spec section 11).
pub(crate) fn exit_code(response: &StepResponse) -> u8 {
    match response {
        StepResponse::Done { .. } => OK,
        StepResponse::Question { .. } => NEED_INPUT,
        StepResponse::Request { .. } => REQUEST,
        StepResponse::Error { .. } => FAILED,
    }
}

/// The request from stdin (JSON, all of it) or from `--entry` and `--answer key=value` flags.
pub(crate) fn build_request(entry: Option<String>, answers: &[String], inputs: &[String], from_stdin: bool) -> Result<StepRequest, String> {
    if from_stdin {
        if entry.is_some() || !answers.is_empty() || !inputs.is_empty() {
            return Err("--stdin reads the whole request; do not combine it with --entry, --answer or --input".into());
        }
        let text = io::read_to_string(io::stdin()).map_err(|e| format!("cannot read stdin: {e}"))?;
        return serde_json::from_str(&text).map_err(|e| format!("invalid request JSON: {e}"));
    }
    let mut map = BTreeMap::new();
    for a in answers {
        let (key, value) = a.split_once('=').ok_or_else(|| format!("invalid --answer `{a}`; expected key=value"))?;
        if map.insert(key.to_string(), value.to_string()).is_some() {
            return Err(format!("answer `{key}` given twice"));
        }
    }
    let mut given = BTreeMap::new();
    for i in inputs {
        let (name, value) = i.split_once('=').ok_or_else(|| format!("invalid --input `{i}`; expected name=value"))?;
        if given.insert(name.to_string(), value.to_string()).is_some() {
            return Err(format!("input `{name}` set twice"));
        }
    }
    Ok(StepRequest {
        version: PROTOCOL_VERSION,
        entry: entry.unwrap_or_else(|| ENTRY.to_string()),
        answers: map,
        flow: None,
        inputs: given,
        walk: false,
    })
}

pub(crate) fn flow_json(f: &rein_core::flows::Flow) -> Value {
    json!({
        "name": f.name,
        "scope": f.scope.label(),
        "path": f.root.display().to_string(),
        "description": f.description,
        "version": f.version,
        "entry": f.entry,
        "shadowed": f.shadowed_by.is_some(),
        "shadowed_by": f.shadowed_by.map(|s| s.label()),
    })
}

/// `rein agent-context`: the block for an agent's context. Prints nothing when no flow is visible.
pub fn agent_context(as_json: bool) -> u8 {
    let found = scopes::discover_here();
    let text = rein_core::flows::agent_context(&found);
    if text.is_empty() {
        return OK;
    }
    if as_json {
        let visible: Vec<Value> = found.iter().filter(|f| f.shadowed_by.is_none()).map(flow_json).collect();
        print_json(json!({"flows": visible, "text": text}));
    } else {
        println!("{text}");
    }
    OK
}

pub(crate) fn print_step(response: &StepResponse, as_json: bool) {
    if as_json {
        println!("{}", serde_json::to_string(response).unwrap_or_default());
        return;
    }
    match response {
        StepResponse::Question { key, r#type, options, description, answers, trace, .. } => {
            println!("question: {key}: {} - {description}", shown_type(r#type, options));
            println!("hint: rerun with --answer {key}=<value>");
            print_answers(answers);
            println!("trace: {}", trace.join(" -> "));
        }
        StepResponse::Request { key, instruction, response, answers, trace, .. } => {
            println!("request: {instruction}");
            println!("then answer: {key}: {} - {}", shown_type(&response.r#type, &response.options), response.description);
            let fields = response.fields.as_deref().unwrap_or_default();
            for line in field_lines(fields) {
                println!("{line}");
            }
            if fields.is_empty() {
                println!("hint: rerun with --answer {key}=<value>");
            } else {
                let shape: Vec<String> = fields.iter().map(|f| format!("\"{}\":...", f.name)).collect();
                println!("hint: rerun with --answer {key}='{{{}}}'", shape.join(","));
            }
            print_answers(answers);
            println!("trace: {}", trace.join(" -> "));
        }
        StepResponse::Done { instruction, answers, trace, .. } => {
            println!("{instruction}");
            print_answers(answers);
            println!("trace: {}", trace.join(" -> "));
        }
        StepResponse::Error { message, diagnostics, .. } => {
            for d in diagnostics {
                println!("{}:{}:{}: error: {}", d.file, d.line, d.column, d.message);
            }
            eprintln!("error: {message}");
        }
    }
}

/// The answers so far, as `--answer` flags to repeat. Nothing is printed when there are none.
fn print_answers(answers: &BTreeMap<String, String>) {
    if answers.is_empty() {
        return;
    }
    // A struct answer is JSON text, so it needs quotes in a shell.
    let flags: Vec<String> = answers
        .iter()
        .map(|(k, v)| if v.starts_with('{') { format!("--answer {k}='{v}'") } else { format!("--answer {k}={v}") })
        .collect();
    println!("answers: {}", flags.join(" "));
}

/// One indented line per field of a struct answer: `  name: type - description`.
pub(crate) fn field_lines(fields: &[rein_core::step::FieldSpec]) -> Vec<String> {
    fields.iter().map(|f| format!("  {}: {} - {}", f.name, shown_type(&f.r#type, &f.options), f.description)).collect()
}

/// The type as TypeScript writes it: `boolean`, or the union such as `"green" | "red"`.
pub(crate) fn shown_type(kind: &str, options: &Option<Vec<String>>) -> String {
    match options {
        Some(list) => list.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join(" | "),
        None => kind.to_string(),
    }
}

pub fn describe(function: &str, root: Option<PathBuf>) -> u8 {
    let project = match load(root) {
        Ok(p) => p,
        Err(msg) => return fail(false, &msg),
    };
    let Some(sf) = project.get(function) else {
        return fail(false, &unknown_function(&project, function));
    };
    let schema = json!({
        "name": function,
        "description": description(&sf.function),
        "inputs": inputs_json(&sf.function),
    });
    println!("{}", serde_json::to_string_pretty(&schema).unwrap_or_default());
    OK
}

/// Error text for a function name that is not in the project. A missing `main` gets the project error.
fn unknown_function(project: &Project, function: &str) -> String {
    match project.project_diagnostics().first() {
        Some(d) if function == ENTRY => d.message.clone(),
        _ => format!("unknown function `{function}`"),
    }
}

/// Description text of a function, or empty when it has none.
pub fn description(f: &Function) -> String {
    f.description.as_ref().map_or_else(String::new, |d| d.value.clone())
}

fn load(root: Option<PathBuf>) -> Result<Project, String> {
    let root = root.unwrap_or_else(|| Project::find_root(Path::new(".")));
    scopes::load_linked(&root).map_err(|e| format!("cannot read {}: {e}", root.display()))
}

/// Diagnostics under `scope`: one file, one directory, or the whole project. Sorted by file then position.
/// Project-level diagnostics (such as a missing `main.rein`) show only when `scope` is the project root.
/// With `deny_deprecated`, each deprecation is an error.
fn rows(project: &Project, scope: &Path, deny_deprecated: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    for (name, diags) in project.check() {
        let Some(sf) = project.get(&name) else { continue };
        if !sf.path.starts_with(scope) {
            continue;
        }
        let file = display_path(&sf.path);
        for d in diags {
            let (line, col) = line_col(&sf.text, d.span.start);
            rows.push(row(&file, line, col, &deny(d, deny_deprecated)));
        }
    }
    if scope == project.root {
        let file = display_path(&project.marker_path());
        rows.extend(project.project_diagnostics().iter().map(|d| row(&file, 0, 0, d)));
    }
    // A bad manifest applies to every file, so it shows for any scope.
    let manifest = display_path(&project.root.join(rein_core::flows::MANIFEST));
    rows.extend(project.manifest_diagnostics().iter().map(|d| row(&manifest, 0, 0, d)));
    rows.sort_by(|a, b| (&a.file, a.start).cmp(&(&b.file, b.start)));
    rows
}

/// The diagnostic with a deprecation made an error, when `deny` is set. Other diagnostics stay as they are.
fn deny(d: Diagnostic, deny: bool) -> Diagnostic {
    if deny && d.is_deprecated() {
        Diagnostic { severity: Severity::Error, ..d }
    } else {
        d
    }
}

/// One printed row. `line` and `col` are 0-based. `deprecated` is read before a deny changes the severity.
fn row(file: &str, line: usize, col: usize, d: &Diagnostic) -> Row {
    let label = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    Row {
        line: format!("{file}:{}:{}: {label}: {}", line + 1, col + 1, d.message),
        start: d.span.start,
        severity: d.severity,
        file: file.to_string(),
        deprecated: d.is_deprecated(),
    }
}

/// The path made absolute, without resolving symlinks. Project lookup and file matching use it.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Path relative to the working directory when it lies under it.
fn display_path(path: &Path) -> String {
    let cwd = fs::canonicalize(".").unwrap_or_else(|_| PathBuf::from("."));
    path.strip_prefix(&cwd).unwrap_or(path).display().to_string()
}

fn parse_sets(sets: &[String]) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    for s in sets {
        let (k, v) = s.split_once('=').ok_or_else(|| format!("invalid --set `{s}`; expected name=value"))?;
        if out.insert(k.to_string(), v.to_string()).is_some() {
            return Err(format!("input `{k}` set twice"));
        }
    }
    Ok(out)
}

fn show_ref(project: &Project, target: &str, json: bool) -> u8 {
    let Some(t) = project.get(target) else {
        return fail(json, &format!("unknown function `@{target}`"));
    };
    let desc = description(&t.function);
    if json {
        print_json(json!({
            "kind": "ref",
            "function": target,
            "description": desc,
            "inputs": inputs_json(&t.function),
        }));
    } else {
        println!("next: {target}");
        println!("  {desc}");
        for input in &t.function.inputs {
            println!("  {}", input_line(input));
        }
    }
    OK
}

fn need_input(f: &Function, name: &str, json: bool) -> u8 {
    let Some(input) = f.input(name) else {
        return fail(json, &format!("unknown input `{name}`"));
    };
    if json {
        print_json(json!({"kind": "need_input", "input": input_json(input)}));
    } else {
        println!("need input {}", input_line(input));
        println!("hint: rerun with --set {}=<value>", input.name.value);
    }
    NEED_INPUT
}

/// The agent must do `instruction`, then answer `name`.
fn request(f: &Function, instruction: &str, name: &str, json: bool) -> u8 {
    let Some(input) = f.response(name) else {
        return fail(json, &format!("unknown answer `{name}`"));
    };
    if json {
        print_json(json!({"kind": "request", "instruction": instruction, "response": input_json(input)}));
    } else {
        println!("request: {instruction}");
        println!("then answer: {}", input_line(input));
        println!("hint: rerun with --set {}=<value>", input.name.value);
    }
    REQUEST
}

fn fail(json: bool, message: &str) -> u8 {
    if json {
        print_json(json!({"kind": "error", "message": message}));
    } else {
        eprintln!("error: {message}");
    }
    FAILED
}

fn print_json(value: Value) {
    println!("{value}");
}

fn inputs_json(f: &Function) -> Vec<Value> {
    f.inputs.iter().map(input_json).collect()
}

fn input_json(input: &Input) -> Value {
    let kind = match &input.ty {
        InputType::Bool => "boolean",
        InputType::Choice(_) => "union",
        InputType::Text => "string",
        InputType::Format(f) => f.json_name(),
        InputType::Struct(_) => "struct",
    };
    let mut v = json!({
        "name": input.name.value,
        "type": kind,
        "description": input.description.value,
    });
    if let Some(options) = input.ty.options() {
        v["options"] = json!(options);
    }
    if let Some(schema) = input.ty.json_schema() {
        v["schema"] = schema;
    }
    v
}

/// `name: type - description`, with the type as TypeScript writes it.
fn input_line(input: &Input) -> String {
    format!("{}: {} - {}", input.name.value, input.ty.ts_type(), input.description.value)
}
