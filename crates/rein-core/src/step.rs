//! Stateless stepping of a whole flow, for agents and any other caller.
//!
//! Each call takes the project and every answer so far. It returns the next question, the next
//! request, the final instruction, or an error. Nothing is kept between calls. Stepping is pure:
//! no I/O, no clock, no randomness. Loading the project is I/O and stays outside this module.
//!
//! The types here are the wire protocol, version [`PROTOCOL_VERSION`]. A CLI, a daemon, or an MCP or
//! JSON-RPC server sends a [`StepRequest`] and returns a [`StepResponse`] unchanged. A loaded
//! [`Project`] is `Send + Sync`, so a daemon can load it once and step many requests.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::ast::{Function, Input, InputType, Struct};
use crate::diagnostic::{line_col, Severity};
use crate::eval::{self, Outcome};
use crate::project::{Project, ENTRY};

pub const PROTOCOL_VERSION: u32 = 1;

/// Most functions one call may visit. A flow that goes past it is treated as a reference cycle.
const HOP_LIMIT: usize = 64;

/// The answers so far, keyed by `<function path>.<name>`, such as `merge_pr.ci` or `answer/faq.solved`.
///
/// A struct answer is a JSON object. In memory it is the object's JSON text. On the wire it is a real JSON object.
pub type Answers = BTreeMap<String, String>;

/// Serde for [`Answers`]: a struct answer is a JSON object on the wire, every other answer a string.
mod answers_json {
    use super::Answers;
    use serde::{Deserialize, Deserializer, Serializer};
    use serde_json::Value;
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(answers: &Answers, s: S) -> Result<S::Ok, S::Error> {
        let wire: BTreeMap<&String, Value> = answers
            .iter()
            .map(|(k, v)| {
                let object = serde_json::from_str::<Value>(v).ok().filter(Value::is_object);
                (k, object.unwrap_or_else(|| Value::String(v.clone())))
            })
            .collect();
        serde::Serialize::serialize(&wire, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Answers, D::Error> {
        let wire = BTreeMap::<String, Value>::deserialize(d)?;
        Ok(wire.into_iter().map(|(k, v)| (k, if let Value::String(t) = v { t } else { v.to_string() })).collect())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRequest {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Function path to start from. Default `main`.
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default, with = "answers_json")]
    pub answers: Answers,
    /// Flow name to step, resolved by the caller through `flows::resolve`. Absent when the caller
    /// passes a root instead. The stepping itself ignores it: it runs on the project it is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    /// The values of the entry function's `string` parameters, by name. The agent is never asked for them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, String>,
}

fn default_version() -> u32 {
    PROTOCOL_VERSION
}

fn default_entry() -> String {
    ENTRY.to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepResponse {
    /// A parameter is needed. Answer `key`, then step again.
    Question {
        version: u32,
        key: String,
        function: String,
        name: String,
        r#type: String,
        options: Option<Vec<String>>,
        description: String,
        #[serde(with = "answers_json")]
        answers: Answers,
        trace: Vec<String>,
    },
    /// The agent must do `instruction`, then answer `response`, then step again.
    Request {
        version: u32,
        key: String,
        function: String,
        instruction: String,
        response: ResponseSpec,
        #[serde(with = "answers_json")]
        answers: Answers,
        trace: Vec<String>,
    },
    /// The flow ended. `instruction` is what the agent does next.
    Done {
        version: u32,
        instruction: String,
        #[serde(with = "answers_json")]
        answers: Answers,
        trace: Vec<String>,
    },
    Error {
        version: u32,
        message: String,
        /// The answer key the message is about, when there is one.
        key: Option<String>,
        diagnostics: Vec<StepDiagnostic>,
        #[serde(with = "answers_json")]
        answers: Answers,
        trace: Vec<String>,
    },
}

/// The answer a request asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseSpec {
    pub name: String,
    pub r#type: String,
    pub options: Option<Vec<String>>,
    pub description: String,
    /// The fields of a struct answer, when `type` is `struct`. The answer is a JSON object with exactly these fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<FieldSpec>>,
}

/// One field of a struct answer. `type` is `boolean`, `union` or `string`; a union lists its `options`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldSpec {
    pub name: String,
    pub r#type: String,
    pub options: Option<Vec<String>>,
    pub description: String,
}

/// One error from the checker. `file` is the project path, such as `ci/fix.rein`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepDiagnostic {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// Steps the flow from `req.entry` as far as the answers allow.
pub fn step(project: &Project, req: &StepRequest) -> StepResponse {
    let answers = req.answers.clone();
    if req.version != PROTOCOL_VERSION {
        return invalid_request(format!(
            "unsupported protocol version {}; this build speaks {PROTOCOL_VERSION}",
            req.version
        ));
    }
    let errors = error_diagnostics(project, &req.entry);
    if !errors.is_empty() {
        let message = format!(
            "project has {} error(s); run `rein check` and fix them",
            errors.len()
        );
        return failure(message, None, errors, answers, Vec::new());
    }
    if let Err((key, message)) = check_answers(project, &req.answers) {
        return failure(message, Some(key), Vec::new(), answers, Vec::new());
    }
    let Some(entry) = project.get(&req.entry) else {
        return failure(
            format!("unknown function `{}`", req.entry),
            None,
            Vec::new(),
            answers,
            Vec::new(),
        );
    };
    if entry.function.is_template() {
        let message = "templates are called from functions, not run";
        return failure(message, None, Vec::new(), answers, Vec::new());
    }
    if let Err(message) = check_entry_inputs(&entry.function, &req.inputs) {
        return failure(message, None, Vec::new(), answers, Vec::new());
    }

    let mut trace = Vec::new();
    let mut current = req.entry.clone();
    // The `string` values of the function being run. The entry gets them from the request; a hand-off
    // passes them on, bound to the target's `string` parameters by name.
    let mut strings: BTreeMap<String, String> = req.inputs.clone();
    for _ in 0..HOP_LIMIT {
        trace.push(current.clone());
        let Some(sf) = project.get(&current) else {
            return failure(
                format!("unknown function `{current}`"),
                None,
                Vec::new(),
                answers,
                trace,
            );
        };
        let mut inputs = inputs_of(&current, &req.answers);
        inputs.extend(strings.iter().map(|(k, v)| (k.clone(), v.clone())));
        let outcome = match eval::run(&sf.function, &inputs, &project.bindings(&current)) {
            Ok(outcome) => outcome,
            Err(message) => return failure(message, None, Vec::new(), answers, trace),
        };
        match outcome {
            // Auto-follow: the agent does not need to see a hop that asks nothing.
            Outcome::Ref { target, args } => {
                let target = project.qualify(&current, &target);
                let Some(next) = project.get(&target) else {
                    return failure(format!("unknown function `{target}`"), None, Vec::new(), answers, trace);
                };
                let mut given: BTreeMap<String, String> = args.into_iter().collect();
                let mut bound = BTreeMap::new();
                for input in next.function.text_inputs() {
                    let name = &input.name.value;
                    let Some(value) = given.remove(name) else {
                        return failure(format!("`{target}` is missing argument `{name}`"), None, Vec::new(), answers, trace);
                    };
                    bound.insert(name.clone(), value);
                }
                if let Some(extra) = given.keys().next() {
                    return failure(format!("`{target}` has no parameter `{extra}`"), None, Vec::new(), answers, trace);
                }
                strings = bound;
                current = target;
            }
            Outcome::Text(instruction) => {
                return StepResponse::Done {
                    version: PROTOCOL_VERSION,
                    instruction,
                    answers,
                    trace,
                };
            }
            Outcome::NeedInput(name) => {
                let Some(input) = sf.function.input(&name) else {
                    return failure(
                        format!("unknown input `{name}`"),
                        None,
                        Vec::new(),
                        answers,
                        trace,
                    );
                };
                // The agent is never asked for a `string` input. The entry's are checked up front, so this
                // only guards a value that did not reach the function.
                if input.ty == InputType::Text {
                    return failure(missing_input(&name), None, Vec::new(), answers, trace);
                }
                let (r#type, options) = type_of(input);
                return StepResponse::Question {
                    version: PROTOCOL_VERSION,
                    key: format!("{current}.{name}"),
                    function: current.clone(),
                    name,
                    r#type,
                    options,
                    description: input.description.value.clone(),
                    answers,
                    trace,
                };
            }
            Outcome::Request {
                instruction,
                response,
            } => {
                let Some(input) = sf.function.response(&response) else {
                    return failure(
                        format!("unknown answer `{response}`"),
                        None,
                        Vec::new(),
                        answers,
                        trace,
                    );
                };
                let (r#type, options) = type_of(input);
                let spec = ResponseSpec {
                    name: response.clone(),
                    r#type,
                    options,
                    description: input.description.value.clone(),
                    fields: sf.function.answer_struct(&response).and_then(|s| s.as_ref().ok()).map(field_specs),
                };
                return StepResponse::Request {
                    version: PROTOCOL_VERSION,
                    key: format!("{current}.{response}"),
                    function: current.clone(),
                    instruction,
                    response: spec,
                    answers,
                    trace,
                };
            }
        }
    }
    let message = format!("flow did not finish in {HOP_LIMIT} steps (reference cycle?)");
    failure(message, None, Vec::new(), answers, trace)
}

fn failure(
    message: impl Into<String>,
    key: Option<String>,
    diagnostics: Vec<StepDiagnostic>,
    answers: Answers,
    trace: Vec<String>,
) -> StepResponse {
    StepResponse::Error {
        version: PROTOCOL_VERSION,
        message: message.into(),
        key,
        diagnostics,
        answers,
        trace,
    }
}

/// The wire type name and the options of an input.
fn type_of(input: &Input) -> (String, Option<Vec<String>>) {
    let options = input
        .ty
        .options()
        .map(|o| o.into_iter().map(str::to_string).collect());
    let name = match input.ty {
        InputType::Bool => "boolean",
        InputType::Choice(_) => "union",
        InputType::Text => "string",
        InputType::Struct(_) => "struct",
    };
    (name.to_string(), options)
}

fn field_specs(s: &Struct) -> Vec<FieldSpec> {
    s.fields
        .iter()
        .map(|f| {
            let (r#type, options) = type_of(f);
            FieldSpec { name: f.name.value.clone(), r#type, options, description: f.description.value.clone() }
        })
        .collect()
}

/// Errors of every file and of the project, in a fixed order. Warnings are left out. A program is its entry file,
/// so the project-level error (no `main.rein`) counts only when the entry itself is missing.
fn error_diagnostics(project: &Project, entry: &str) -> Vec<StepDiagnostic> {
    let mut out = Vec::new();
    for (name, diags) in project.check() {
        let Some(sf) = project.get(&name) else {
            continue;
        };
        for d in diags.iter().filter(|d| d.severity == Severity::Error) {
            let (line, column) = line_col(&sf.text, d.span.start);
            let file = format!("{name}.rein");
            out.push(StepDiagnostic {
                file,
                line: line + 1,
                column: column + 1,
                message: d.message.clone(),
            });
        }
    }
    let project_errors = if project.get(entry).is_some() { Vec::new() } else { project.project_diagnostics() };
    for d in project_errors {
        let file = "main.rein".to_string();
        out.push(StepDiagnostic {
            file,
            line: 1,
            column: 1,
            message: d.message,
        });
    }
    out
}

/// The `(key, message)` of the first answer that names no parameter or answer, or has a bad value.
fn check_answers(project: &Project, answers: &Answers) -> Result<(), (String, String)> {
    for (key, raw) in answers {
        let Some((function, input)) = declared(project, key) else {
            return Err((key.clone(), format!("unknown key `{key}`")));
        };
        if input.ty == InputType::Text {
            return Err((
                key.clone(),
                format!("`{key}` is a string parameter; only a template takes one"),
            ));
        }
        eval::check_value(function, input, raw).map_err(|message| (key.clone(), message))?;
    }
    Ok(())
}

/// The parameter or `const` answer that `key` names. Names have no dots, so the last dot splits the key.
fn declared<'a>(project: &'a Project, key: &str) -> Option<(&'a Function, &'a Input)> {
    let (function, name) = key.rsplit_once('.')?;
    let f = &project.get(function)?.function;
    f.input(name).or_else(|| f.response(name)).map(|i| (f, i))
}

/// The error for a `string` input that was not given.
fn missing_input(name: &str) -> String {
    format!("missing input `{name}`; give it with --input {name}=<value>")
}

/// The entry's `string` inputs must all be given, and no other name may be. Checked before the flow runs.
fn check_entry_inputs(entry: &Function, given: &BTreeMap<String, String>) -> Result<(), String> {
    if let Some(name) = given.keys().find(|k| !entry.text_inputs().any(|i| i.name.value == **k)) {
        return Err(format!("unknown input `{name}`"));
    }
    match entry.text_inputs().find(|i| !given.contains_key(&i.name.value)) {
        Some(input) => Err(missing_input(&input.name.value)),
        None => Ok(()),
    }
}

/// The answers that belong to `function`, keyed by the bare name the evaluator uses.
pub(crate) fn inputs_of(function: &str, answers: &Answers) -> HashMap<String, String> {
    answers
        .iter()
        .filter_map(|(key, raw)| {
            let (owner, name) = key.rsplit_once('.')?;
            (owner == function).then(|| (name.to_string(), raw.clone()))
        })
        .collect()
}

/// A response for a request that cannot be stepped, such as bad JSON. It carries no answers.
pub fn invalid_request(message: impl Into<String>) -> StepResponse {
    StepResponse::Error {
        version: PROTOCOL_VERSION,
        message: message.into(),
        key: None,
        diagnostics: Vec::new(),
        answers: Answers::new(),
        trace: Vec::new(),
    }
}

#[cfg(test)]
#[path = "step_tests.rs"]
mod tests;
