//! Stateless stepping of a whole flow, for agents and any other caller.
//!
//! Each call takes the project and every answer so far. It returns the next question, the next
//! request, the final instruction, or an error. Nothing is kept between calls. Stepping is pure:
//! no I/O, no clock, no randomness. Loading the project is I/O and stays outside this module.
//!
//! The types here are the wire protocol, version [`PROTOCOL_VERSION`]. A CLI, a daemon, or an MCP or
//! JSON-RPC server sends a [`StepRequest`] and returns a [`StepResponse`] unchanged. A loaded
//! [`Project`] is `Send + Sync`, so a daemon can load it once and step many requests.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::ast::{walk_stmts, Awaited, Binding, Function, Input, InputType, ReturnValue, Stmt, Struct};
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
    /// True for a walk with stand-in input values, such as rein graph or the debug walk: format checks are skipped,
    /// because the values are not real.
    #[serde(skip)]
    pub walk: bool,
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
    run(project, req, &mut Vec::new())
}

/// The value that a walk gives an unset `string` or format input: `<name>`.
pub fn walk_value(input: &Input) -> String {
    format!("<{}>", input.name.value)
}

/// The owner of each function a step walks, in walk order, as [`scoped`] gives it: `main`, then `main.b>fn`.
/// `rein graph` reads a function's answers through the first of its owners. An unset `string` or format input of
/// `entry` is walked with [`walk_value`]: no condition reads an input, so the walk is the same as with the real inputs.
/// The walk sets [`StepRequest::walk`], so it checks no format. Each owner comes with the values passed to that visit,
/// so a walk reads a passed `boolean` as `rein step` does.
pub fn owners(project: &Project, entry: &str, answers: &Answers) -> Vec<(String, BTreeMap<String, String>)> {
    let inputs = project.get(entry).map_or_else(BTreeMap::new, |sf| {
        sf.function.text_inputs().map(|i| (i.name.value.clone(), walk_value(i))).collect()
    });
    let req = StepRequest { version: PROTOCOL_VERSION, entry: entry.to_string(), answers: answers.clone(), flow: None, inputs, walk: true };
    let mut owners = Vec::new();
    run(project, &req, &mut owners);
    owners
}

/// [`step`], pushing the owner of each function it walks, and its passed values, to `owners`.
fn run(project: &Project, req: &StepRequest, owners: &mut Vec<(String, BTreeMap<String, String>)>) -> StepResponse {
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
    if let Err((key, message)) = check_answers(project, &req.entry, &req.answers) {
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
    if let Err(message) = check_entry_inputs(&entry.function, &req.inputs, req.walk) {
        return failure(message, None, Vec::new(), answers, Vec::new());
    }

    let mut trace = Vec::new();
    let mut current = req.entry.clone();
    // The passed values of the function being run. The entry gets its `string` values from the request; a
    // hand-off passes a value for every parameter of the target, bound by name.
    let mut strings: BTreeMap<String, String> = req.inputs.clone();
    // The key prefix of the call being walked: empty at the top, else the caller's answer key, such as `main.b`.
    let mut scope = String::new();
    // The callers that wait in an `await`, innermost last: the function, its scope, and the answer the callee's
    // text fills.
    let mut callers: Vec<(String, String, String)> = Vec::new();
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
        // The owner of this visit's answer keys: `fn`, or `main.b>fn` inside the call `main.b`.
        let owner = scoped(&scope, &current);
        owners.push((owner.clone(), strings.clone()));
        let mut inputs = inputs_of(&owner, &req.answers);
        inputs.extend(strings.iter().map(|(k, v)| (k.clone(), v.clone())));
        let outcome = match eval::run(&sf.function, &inputs, &project.bindings(&current), req.walk) {
            Ok(outcome) => outcome,
            Err(message) => return failure(message, None, Vec::new(), answers, trace),
        };
        match outcome {
            // Auto-follow: the agent does not need to see a hop that asks nothing.
            Outcome::Ref { target, args } => match bind(project, &current, &target, args, req.walk) {
                Ok((target, bound)) => (current, strings) = (target, bound),
                Err(message) => return failure(message, None, Vec::new(), answers, trace),
            },
            // A call with return: walk the callee as a hand-off, and remember who waits for its text.
            Outcome::Call { target, args, response } => match bind(project, &current, &target, args, req.walk) {
                Ok((target, bound)) => {
                    // Each call gets its own keys, so a second call of `fn`, or a recursive call, asks again.
                    let caller = std::mem::replace(&mut current, target);
                    let caller_scope = std::mem::replace(&mut scope, format!("{owner}.{response}"));
                    callers.push((caller, caller_scope, response));
                    strings = bound;
                }
                Err(message) => return failure(message, None, Vec::new(), answers, trace),
            },
            Outcome::Text(text) => {
                // The callee of an `await` ended. Its text is a request whose answer is the caller's `const`.
                if let Some((caller, caller_scope, response)) = callers.pop() {
                    let caller_owner = scoped(&caller_scope, &caller);
                    let instruction = format!("{text}\n\nThen answer `{caller_owner}.{response}` with the result.");
                    return request(project, &caller, &caller_owner, &response, instruction, answers, trace);
                }
                let instruction = text;
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
                if input.ty.is_text() {
                    return failure(missing_input(&name), None, Vec::new(), answers, trace);
                }
                let (r#type, options) = type_of(input);
                return StepResponse::Question {
                    version: PROTOCOL_VERSION,
                    key: format!("{owner}.{name}"),
                    function: current.clone(),
                    name,
                    r#type,
                    options,
                    description: input.description.value.clone(),
                    answers,
                    trace,
                };
            }
            Outcome::Request { instruction, response } => {
                return request(project, &current, &owner, &response, instruction, answers, trace);
            }
        }
    }
    let message = format!("process did not finish in {HOP_LIMIT} steps (reference cycle?)");
    failure(message, None, Vec::new(), answers, trace)
}

/// The function a hand-off or a call from `from` goes to, and the values of all its parameters, bound by name
/// from `args`. Every `string` parameter is given. A `boolean` or union parameter that the caller did not pass is
/// asked as a question, until the checker makes passing it required (see `check::MISSING_TYPED_ARG`). A walk
/// (`walk`) checks no format of a passed value.
fn bind(
    project: &Project,
    from: &str,
    target: &str,
    args: Vec<(String, String)>,
    walk: bool,
) -> Result<(String, BTreeMap<String, String>), String> {
    let target = project.qualify(from, target);
    let Some(next) = project.get(&target) else {
        return Err(format!("unknown function `{target}`"));
    };
    let mut given: BTreeMap<String, String> = args.into_iter().collect();
    let mut bound = BTreeMap::new();
    for input in &next.function.inputs {
        let name = &input.name.value;
        match given.remove(name) {
            Some(value) => {
                if let (InputType::Format(format), false) = (&input.ty, walk) {
                    format.check(name, &value).map_err(|msg| format!("`{target}`: {msg}"))?;
                }
                _ = bound.insert(name.clone(), value);
            }
            None if input.ty.is_text() => return Err(format!("`{target}` is missing argument `{name}`")),
            None => {}
        }
    }
    if let Some(extra) = given.keys().next() {
        return Err(format!("`{target}` has no parameter `{extra}`"));
    }
    Ok((target, bound))
}

/// A request for the `const` answer `response` of `function`, after the agent does `instruction`. `owner` is the
/// function with its call scope, as [`scoped`] gives it; the key is `<owner>.<response>`.
fn request(
    project: &Project,
    function: &str,
    owner: &str,
    response: &str,
    instruction: String,
    answers: Answers,
    trace: Vec<String>,
) -> StepResponse {
    let Some(f) = project.get(function).map(|sf| &sf.function) else {
        return failure(format!("unknown function `{function}`"), None, Vec::new(), answers, trace);
    };
    let Some(input) = f.response(response) else {
        return failure(format!("unknown answer `{response}`"), None, Vec::new(), answers, trace);
    };
    let (r#type, options) = type_of(input);
    let spec = ResponseSpec {
        name: response.to_string(),
        r#type,
        options,
        description: input.description.value.clone(),
        fields: f.answer_struct(response).and_then(|s| s.as_ref().ok()).map(field_specs),
    };
    StepResponse::Request {
        version: PROTOCOL_VERSION,
        key: format!("{owner}.{response}"),
        function: function.to_string(),
        instruction,
        response: spec,
        answers,
        trace,
    }
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
pub fn type_of(input: &Input) -> (String, Option<Vec<String>>) {
    let options = input
        .ty
        .options()
        .map(|o| o.into_iter().map(str::to_string).collect());
    let name = match input.ty {
        InputType::Bool => "boolean",
        InputType::Choice(_) => "union",
        InputType::Text => "string",
        InputType::Format(f) => f.json_name(),
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
fn check_answers(project: &Project, entry: &str, answers: &Answers) -> Result<(), (String, String)> {
    for (key, raw) in answers {
        let Some((function, input)) = declared(project, entry, key) else {
            return Err((key.clone(), format!("unknown key `{key}`")));
        };
        if input.ty.is_text() {
            let message = match input.ty {
                InputType::Format(_) => {
                    format!("`{key}` is a parameter of type `{}`; only a caller passes one", input.ty.ts_type())
                }
                _ => format!("`{key}` is a string parameter; only a template takes one"),
            };
            return Err((key.clone(), message));
        }
        eval::check_value(function, input, raw).map_err(|message| (key.clone(), message))?;
    }
    Ok(())
}

/// The parameter or `const` answer that `key` names. Names have no dots, so the last dot splits the key. A key
/// inside a call, such as `main.b>fn.ok`, names `fn.ok`, and each part before a `>` must name a `const` answer.
fn declared<'a>(project: &'a Project, entry: &str, key: &str) -> Option<(&'a Function, &'a Input)> {
    let (owner, name) = key.rsplit_once('.')?;
    let f = &project.get(owner_function(project, entry, owner)?)?.function;
    f.input(name).or_else(|| f.response(name)).map(|i| (f, i))
}

/// The function that `owner` names, such as `fn` for `main.a>fn`, when each call in it is one a walk from `entry`
/// can make: the first part is a function that `entry` reaches through hand-offs, outside any call, and each part
/// before a `>` is `<function>.<const>`, an `await` whose target reaches the next part's function through
/// hand-offs. `None` for any other owner, so its keys are unknown. An owner with no `>` is any function, as before.
fn owner_function<'a>(project: &Project, entry: &str, owner: &'a str) -> Option<&'a str> {
    let mut reach: Option<BTreeSet<String>> = owner.contains(CALL).then(|| handoff_reach(project, entry.to_string()));
    let mut parts = owner.split(CALL).peekable();
    while let Some(part) = parts.next() {
        let last = parts.peek().is_none();
        let function = if last { part } else { part.rsplit_once('.')?.0 };
        if reach.as_ref().is_some_and(|r| !r.contains(function)) {
            return None;
        }
        if last {
            return Some(function);
        }
        reach = Some(call_reach(project, function, part.rsplit_once('.')?.1)?);
    }
    None
}

/// The functions a walk can be in during the call `<function>.<name>`: the target of that `await`, and each
/// function the target reaches through hand-offs. `None` when `name` is not an `await` of a function.
fn call_reach(project: &Project, function: &str, name: &str) -> Option<BTreeSet<String>> {
    let f = &project.get(function)?.function;
    let mut target = None;
    walk_stmts(&f.body, &mut |s| {
        if let Stmt::Request { response, awaited: Awaited::Call(value), .. } = s {
            if response.name.value == name {
                target = target.or(ref_key(value));
            }
        }
    });
    Some(handoff_reach(project, follow(project, function, target?)?))
}

/// `start`, and each function it reaches through hand-offs (returns, not awaits).
fn handoff_reach(project: &Project, start: String) -> BTreeSet<String> {
    let mut queue = vec![start];
    let mut seen = BTreeSet::new();
    while let Some(g) = queue.pop() {
        if !seen.insert(g.clone()) {
            continue;
        }
        let Some(sf) = project.get(&g) else { continue };
        walk_stmts(&sf.function.body, &mut |s| {
            if let Stmt::Return(r) = s {
                if let Some(next) = ref_key(&r.value).and_then(|k| follow(project, &g, k)) {
                    queue.push(next);
                }
            }
        });
    }
    seen
}

/// The reference key a returned or awaited value names, such as `./x` or `program/fn`. `None` for text.
fn ref_key(v: &ReturnValue) -> Option<&str> {
    match v {
        ReturnValue::Ref(r) => Some(&r.value),
        ReturnValue::Call(c) => Some(c.callee.key()),
        ReturnValue::Text(_) | ReturnValue::Template(_) => None,
    }
}

/// The function that the reference `key`, written in `from`, runs, named as the walk names it. `None` for a
/// template or a reference that does not resolve.
fn follow(project: &Project, from: &str, key: &str) -> Option<String> {
    match project.bindings(from).get(key)? {
        Binding::Function(path) => Some(project.qualify(from, path)),
        _ => None,
    }
}

/// Separates the caller's answer key from the callee in the key of a call with return: `main.b>fn.ok`.
pub const CALL: char = '>';

/// `function` inside the call `scope`: `main.b>fn`, or `fn` at the top, where `scope` is empty.
fn scoped(scope: &str, function: &str) -> String {
    if scope.is_empty() {
        function.to_string()
    } else {
        format!("{scope}{CALL}{function}")
    }
}

/// The function and the name an answer key names, without its call scope: `main.b>fn.ok` gives `("fn", "ok")`.
pub fn key_parts(key: &str) -> Option<(&str, &str)> {
    let (owner, name) = key.rsplit_once('.')?;
    Some((owner.rsplit_once(CALL).map_or(owner, |(_, f)| f), name))
}

/// The error for a `string` input that was not given.
fn missing_input(name: &str) -> String {
    format!("missing input `{name}`; give it with --input {name}=<value>")
}

/// The entry's `string` inputs must all be given, and no other name may be. Checked before the flow runs. A walk
/// (`walk`) checks no format of a given value.
fn check_entry_inputs(entry: &Function, given: &BTreeMap<String, String>, walk: bool) -> Result<(), String> {
    if let Some(name) = given.keys().find(|k| !entry.text_inputs().any(|i| i.name.value == **k)) {
        return Err(format!("unknown input `{name}`"));
    }
    if let Some(input) = entry.text_inputs().find(|i| !given.contains_key(&i.name.value)) {
        return Err(missing_input(&input.name.value));
    }
    if !walk {
        check_formats(entry, |name| given.get(name).map(String::as_str))?;
    }
    Ok(())
}

/// Checks each format input of `entry` that `value_of` gives a value to against its format. A name with no
/// value is skipped. `rein step` and `rein eval` both check their `string` inputs with it.
pub fn check_formats<'a>(entry: &Function, value_of: impl Fn(&str) -> Option<&'a str>) -> Result<(), String> {
    for input in entry.text_inputs() {
        if let (InputType::Format(format), Some(value)) = (&input.ty, value_of(&input.name.value)) {
            format.check(&input.name.value, value)?;
        }
    }
    Ok(())
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
