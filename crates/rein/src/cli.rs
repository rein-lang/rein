//! Command bodies for `check`, `run`, `describe` and `graph`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use rein_core::ast::{Function, Input};
use rein_core::diagnostic::{line_col, Severity};
use rein_core::eval::{self, Outcome};
use rein_core::project::{Project, ENTRY};
use serde_json::{json, Value};

const OK: u8 = 0;
const FAILED: u8 = 1;
const NEED_INPUT: u8 = 2;

/// One printed diagnostic line, kept with its sort keys.
struct Row {
    file: String,
    start: usize,
    severity: Severity,
    line: String,
}

pub fn check(given: &Path) -> u8 {
    if !given.exists() {
        return fail(false, &format!("no such file or directory: {}", given.display()));
    }
    if given.is_file() && given.extension().and_then(|e| e.to_str()) != Some("rein") {
        return fail(false, &format!("not a .rein file: {}", given.display()));
    }
    let scope = absolute(given);
    let root = Project::find_root(&scope);
    let project = match Project::load(&root) {
        Ok(p) => p,
        Err(e) => return fail(false, &format!("cannot read {}: {e}", root.display())),
    };
    let rows = rows(&project, &scope);
    let errors = rows.iter().filter(|r| r.severity == Severity::Error).count();
    let warnings = rows.len() - errors;
    for r in &rows {
        println!("{}", r.line);
    }
    eprintln!("{errors} errors, {warnings} warnings");
    if errors > 0 {
        FAILED
    } else {
        OK
    }
}

pub fn run(function: &str, root: Option<PathBuf>, sets: &[String], json: bool) -> u8 {
    let project = match load(root) {
        Ok(p) => p,
        Err(msg) => return fail(json, &msg),
    };
    let Some(sf) = project.get(function) else {
        return fail(json, &unknown_function(&project, function));
    };
    let errors: Vec<Row> = rows(&project, &sf.path)
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
    let targets = project.import_targets(function);
    match eval::run(&sf.function, &inputs, &targets) {
        Err(msg) => fail(json, &msg),
        Ok(Outcome::Text(value)) => {
            if json {
                print_json(json!({"kind": "text", "value": value}));
            } else {
                println!("{value}");
            }
            OK
        }
        Ok(Outcome::Ref(target)) => show_ref(&project, &target, json),
        Ok(Outcome::NeedInput(name)) => need_input(&sf.function, &name, json),
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

pub fn graph(root: Option<PathBuf>) -> u8 {
    let project = match load(root) {
        Ok(p) => p,
        Err(msg) => return fail(false, &msg),
    };
    let mut lines = vec!["flowchart TD".to_string()];
    // Ids are positional, so no two names can collide. The name is the label.
    let ids: BTreeMap<&str, String> = project.files.keys().enumerate().map(|(i, n)| (n.as_str(), format!("n{i}"))).collect();
    // The entry point is drawn as a stadium, so it stands out.
    for (name, id) in &ids {
        let label = name.replace('"', "#quot;");
        if *name == ENTRY {
            lines.push(format!("    {id}([\"{label}\"])"));
        } else {
            lines.push(format!("    {id}[\"{label}\"]"));
        }
    }
    lines.extend(
        project
            .references()
            .iter()
            .map(|(from, to)| format!("    {} --> {}", ids[from.as_str()], ids[to.as_str()])),
    );
    println!("{}", lines.join("\n"));
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
    Project::load(&root).map_err(|e| format!("cannot read {}: {e}", root.display()))
}

/// Diagnostics under `scope`: one file, one directory, or the whole project. Sorted by file then position.
/// Project-level diagnostics (such as a missing `main.rein`) show only when `scope` is the project root.
fn rows(project: &Project, scope: &Path) -> Vec<Row> {
    let mut rows = Vec::new();
    for (name, diags) in project.check() {
        let Some(sf) = project.get(&name) else { continue };
        if !sf.path.starts_with(scope) {
            continue;
        }
        let file = display_path(&sf.path);
        for d in diags {
            let (line, col) = line_col(&sf.text, d.span.start);
            rows.push(row(&file, line, col, &d));
        }
    }
    if scope == project.root {
        let file = display_path(&project.marker_path());
        rows.extend(project.project_diagnostics().iter().map(|d| row(&file, 0, 0, d)));
    }
    rows.sort_by(|a, b| (&a.file, a.start).cmp(&(&b.file, b.start)));
    rows
}

/// One printed row. `line` and `col` are 0-based.
fn row(file: &str, line: usize, col: usize, d: &rein_core::diagnostic::Diagnostic) -> Row {
    let label = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    Row {
        line: format!("{file}:{}:{}: {label}: {}", line + 1, col + 1, d.message),
        start: d.span.start,
        severity: d.severity,
        file: file.to_string(),
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
    let kind = if input.ty.options().is_some() { "union" } else { "boolean" };
    let mut v = json!({
        "name": input.name.value,
        "type": kind,
        "description": input.description.value,
    });
    if let Some(options) = input.ty.options() {
        v["options"] = json!(options);
    }
    v
}

/// `name: type - description`, with the type as TypeScript writes it.
fn input_line(input: &Input) -> String {
    format!("{}: {} - {}", input.name.value, input.ty.ts_type(), input.description.value)
}
