//! Command bodies for `check`, `run`, `describe` and `graph`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rein_core::ast::{Function, Input};
use rein_core::diagnostic::{line_col, Severity};
use rein_core::eval::{self, Outcome};
use rein_core::project::Project;
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

pub fn check(path: &Path) -> u8 {
    let root = Project::find_root(path);
    let project = match Project::load(&root) {
        Ok(p) => p,
        Err(e) => return fail(false, &format!("cannot read {}: {e}", root.display())),
    };
    let only = if path.is_file() { fs::canonicalize(path).ok() } else { None };
    let rows = rows(&project, only.as_deref());
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
        return fail(json, &format!("unknown function `{function}`"));
    };
    let errors: Vec<Row> = rows(&project, Some(sf.path.as_path()))
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
    match eval::run(&sf.function, &inputs) {
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
        return fail(false, &format!("unknown function `{function}`"));
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
    lines.extend(project.files.keys().map(|n| format!("    {}[\"{n}\"]", node_id(n))));
    lines.extend(
        project
            .references()
            .iter()
            .map(|(from, to)| format!("    {} --> {}", node_id(from), node_id(to))),
    );
    println!("{}", lines.join("\n"));
    OK
}

/// Description text of a function, or empty when it has none.
pub fn description(f: &Function) -> String {
    f.description.as_ref().map_or_else(String::new, |d| d.value.clone())
}

/// `bool`, or `choice: a, b` for a choice input.
pub fn type_label(ty: &rein_core::ast::InputType) -> String {
    match ty.options() {
        None => "bool".to_string(),
        Some(options) => format!("choice: {}", options.join(", ")),
    }
}

fn load(root: Option<PathBuf>) -> Result<Project, String> {
    let root = root.unwrap_or_else(|| Project::find_root(Path::new(".")));
    Project::load(&root).map_err(|e| format!("cannot read {}: {e}", root.display()))
}

/// Diagnostics of the project, optionally for one file only, sorted by file then position.
fn rows(project: &Project, only: Option<&Path>) -> Vec<Row> {
    let mut rows = Vec::new();
    for (name, diags) in project.check() {
        let Some(sf) = project.get(&name) else { continue };
        if only.is_some_and(|p| p != sf.path.as_path()) {
            continue;
        }
        let file = display_path(&sf.path);
        for d in diags {
            let (line, col) = line_col(&sf.text, d.span.start);
            let label = match d.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            rows.push(Row {
                line: format!("{file}:{}:{}: {label}: {}", line + 1, col + 1, d.message),
                start: d.span.start,
                severity: d.severity,
                file: file.clone(),
            });
        }
    }
    rows.sort_by(|a, b| (&a.file, a.start).cmp(&(&b.file, b.start)));
    rows
}

/// Path relative to the working directory when it lies under it.
fn display_path(path: &Path) -> String {
    let cwd = fs::canonicalize(".").unwrap_or_else(|_| PathBuf::from("."));
    path.strip_prefix(&cwd).unwrap_or(path).display().to_string()
}

fn parse_sets(sets: &[String]) -> Result<HashMap<String, String>, String> {
    sets.iter()
        .map(|s| {
            s.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| format!("invalid --set `{s}`; expected name=value"))
        })
        .collect()
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
        println!("@{target}");
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
    let kind = if input.ty.options().is_some() { "choice" } else { "bool" };
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

/// `name (bool|choice: a, b): description`.
fn input_line(input: &Input) -> String {
    format!("{} ({}): {}", input.name.value, type_label(&input.ty), input.description.value)
}

fn node_id(name: &str) -> String {
    name.replace('/', "__")
}
