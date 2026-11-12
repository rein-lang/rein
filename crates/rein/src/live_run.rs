//! A live run: a flow held in memory while an agent or a person answers it. `rein run` drives one
//! over stdio. `rein serve --mcp` holds many. Each step comes from `rein_core::step`, so the
//! evaluator is not duplicated here.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use rein_core::project::Project;
use rein_core::step::{invalid_request, step, Answers, StepRequest, StepResponse, PROTOCOL_VERSION};
use serde_json::Value;

use crate::cli::{exit_code, field_lines, load_flow, print_step, shown_type};

/// One live run. It keeps its answers and the step it waits on.
pub struct Run {
    project: Project,
    request: StepRequest,
    last: StepResponse,
}

impl Run {
    pub fn start(project: Project, entry: String) -> Run {
        let request = StepRequest { version: PROTOCOL_VERSION, entry, answers: Answers::new(), flow: None, inputs: Default::default() };
        let last = step(&project, &request);
        Run { project, request, last }
    }

    /// The step the run waits on, or the step it ended with.
    pub fn last(&self) -> &StepResponse {
        &self.last
    }

    /// The key the run waits on. `None` when the run ended.
    pub fn asked(&self) -> Option<&str> {
        match &self.last {
            StepResponse::Question { key, .. } | StepResponse::Request { key, .. } => Some(key),
            _ => None,
        }
    }

    /// Answers `key` (default: the asked key) and steps. An error step leaves the run as it was.
    pub fn answer(&mut self, key: Option<String>, value: String) -> StepResponse {
        let Some(asked) = self.asked() else {
            return invalid_request("the run has ended; start a new one");
        };
        let key = key.unwrap_or_else(|| asked.to_string());
        if self.request.answers.contains_key(&key) {
            return invalid_request(format!("answer `{key}` given twice"));
        }
        let mut tried = self.request.clone();
        tried.answers.insert(key, value);
        let next = step(&self.project, &tried);
        if !matches!(next, StepResponse::Error { .. }) {
            self.request = tried;
            self.last = next.clone();
        }
        next
    }
}

/// `rein run`: writes one step per line and reads one answer per line, until the flow ends.
pub fn run(flow: Option<String>, root: Option<PathBuf>, entry: Option<String>, json: bool) -> u8 {
    let (project, default_entry) = match load_flow(root, flow) {
        Ok(found) => found,
        Err(message) => {
            let response = invalid_request(message);
            show(&response, json);
            return exit_code(&response);
        }
    };
    let mut run = Run::start(project, entry.unwrap_or(default_entry));
    let mut lines = io::stdin().lock().lines();
    loop {
        show(run.last(), json);
        if run.asked().is_none() {
            return exit_code(run.last());
        }
        // End of input leaves the run waiting. The exit code says for what, as with `rein step`.
        let Some(Ok(line)) = lines.next() else {
            return exit_code(run.last());
        };
        let next = match parse_answer(&line, json) {
            Err(message) => invalid_request(message),
            Ok((key, value)) => run.answer(key, value),
        };
        // A bad answer does not end the run: show the error, then ask the same step again.
        if matches!(next, StepResponse::Error { .. }) {
            show(&next, json);
        }
    }
}

/// One answer line. JSON mode takes `{"key": "...", "value": "..."}`, where `key` is optional.
/// Text mode takes the bare value for the asked key.
fn parse_answer(line: &str, json: bool) -> Result<(Option<String>, String), String> {
    if !json {
        let value = line.trim();
        if value.is_empty() {
            return Err("type a value, then press Enter".into());
        }
        return Ok((None, value.to_string()));
    }
    let v: Value = serde_json::from_str(line).map_err(|e| format!("invalid answer JSON: {e}"))?;
    let key = v.get("key").and_then(Value::as_str).map(str::to_string);
    let value = match v.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        // A struct answer: the object is the value, as the JSON text `rein step --answer` takes.
        Some(object @ Value::Object(_)) => object.to_string(),
        _ => return Err("an answer line needs a string, boolean or object `value`".into()),
    };
    Ok((key, value))
}

/// One step: a JSON line, or text with a `> ` prompt when an answer is due.
fn show(response: &StepResponse, json: bool) {
    if json {
        println!("{}", serde_json::to_string(response).unwrap_or_default());
        return;
    }
    let Some(lines) = prompt_lines(response) else {
        return print_step(response, false);
    };
    for line in &lines {
        println!("{line}");
    }
    if matches!(response, StepResponse::Done { .. }) {
        return;
    }
    print!("> ");
    let _ = io::stdout().flush();
}

/// The text lines of one step. `None` for an error step, which `print_step` shows.
fn prompt_lines(response: &StepResponse) -> Option<Vec<String>> {
    Some(match response {
        StepResponse::Question { key, r#type, options, description, .. } => {
            vec![format!("question: {key}: {} - {description}", shown_type(r#type, options))]
        }
        StepResponse::Request { key, instruction, response, .. } => {
            let mut lines = vec![
                format!("request: {instruction}"),
                format!("then answer: {key}: {} - {}", shown_type(&response.r#type, &response.options), response.description),
            ];
            lines.extend(field_lines(response.fields.as_deref().unwrap_or_default()));
            lines
        }
        StepResponse::Done { instruction, trace, self_improve, .. } => {
            let mut lines = vec![instruction.clone()];
            lines.extend(self_improve.iter().map(|text| format!("self improve: {text}")));
            lines.push(format!("trace: {}", trace.join(" -> ")));
            lines
        }
        StepResponse::Error { .. } => return None,
    })
}

#[cfg(test)]
#[path = "live_run_tests.rs"]
mod tests;
