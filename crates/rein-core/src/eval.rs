//! Runs a function against the inputs the agent has.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use crate::ast::{Arg, Awaited, Binding, Call, Case, Expr, Function, Input, InputType, Piece, ReturnValue, Spanned, Stmt, Struct, TemplateLit, Value};
use crate::check::{string_in_condition, struct_in_use};

/// Result of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Text(String),
    /// The function to run next, by project path such as `ci/fix`. `args` are the values of its parameters,
    /// by name, as written in `return @./target(name=value);`. They are empty for a plain return.
    Ref { target: String, args: Vec<(String, String)> },
    NeedInput(String),
    /// The agent must answer `response`, after doing `instruction`.
    Request { instruction: String, response: String },
    /// `await @target(args)` with no answer yet: run `target` as a hand-off. Its final text is the request for
    /// `response`. `target` and `args` are as in [`Outcome::Ref`].
    Call { target: String, args: Vec<(String, String)>, response: String },
}

/// Why evaluation stopped before a return.
enum Stop {
    Need(String),
    Error(String),
    Request { instruction: String, response: String },
    Call { target: String, args: Vec<(String, String)>, response: String },
}

type Eval<T> = Result<T, Stop>;

struct Env<'a> {
    f: &'a Function,
    inputs: &'a HashMap<String, String>,
    /// Answers bound on the path so far, in order. Each block drops its own when it ends.
    answers: RefCell<Vec<&'a Input>>,
    /// What each import name refers to, from `Project::bindings`.
    bindings: &'a BTreeMap<String, Binding<'a>>,
}

pub fn run(
    f: &Function,
    inputs: &HashMap<String, String>,
    bindings: &BTreeMap<String, Binding<'_>>,
) -> Result<Outcome, String> {
    if f.is_template() {
        return Err("templates are called from functions, not run".to_string());
    }
    let mut unknown: Vec<&String> = inputs.keys().filter(|k| f.input(k).is_none() && f.response(k).is_none()).collect();
    unknown.sort();
    if let Some(name) = unknown.first() {
        return Err(format!("unknown input `{name}`"));
    }
    let expanded = with_struct_fields(f, inputs);
    let env = Env { f, inputs: &expanded, bindings, answers: RefCell::new(Vec::new()) };
    match exec(&env, &f.body) {
        Ok(Some(out)) => Ok(out),
        Ok(None) => Err("no return reached; every path must return".to_string()),
        Err(Stop::Need(name)) => Ok(Outcome::NeedInput(name)),
        Err(Stop::Error(msg)) => Err(msg),
        Err(Stop::Request { instruction, response }) => Ok(Outcome::Request { instruction, response }),
        Err(Stop::Call { target, args, response }) => Ok(Outcome::Call { target, args, response }),
    }
}

/// The inputs, plus one input per field of each struct answer given, named `answer.field`.
pub fn with_struct_fields(f: &Function, inputs: &HashMap<String, String>) -> HashMap<String, String> {
    let mut expanded = inputs.clone();
    for (name, found) in &f.answer_structs {
        if let (Ok(s), Some(raw)) = (found, inputs.get(name)) {
            expanded.extend(struct_values(name, s, raw).unwrap_or_default());
        }
    }
    expanded
}

/// Whether `cond` holds for `inputs` (from [`with_struct_fields`]), with every answer of `f` in scope.
/// `None` when a value it needs is missing or not valid. `rein graph` walks a flow with it.
pub fn holds(f: &Function, cond: &Expr, inputs: &HashMap<String, String>) -> Option<bool> {
    let bindings = BTreeMap::new();
    let answers = inputs.keys().filter_map(|k| f.response(k)).collect();
    let env = Env { f, inputs, bindings: &bindings, answers: RefCell::new(answers) };
    eval(&env, cond).ok()
}

/// Runs one block. The answers it binds are dropped when it ends.
fn exec<'a>(env: &Env<'a>, stmts: &'a [Stmt]) -> Eval<Option<Outcome>> {
    let mark = env.answers.borrow().len();
    let out = exec_stmts(env, stmts);
    env.answers.borrow_mut().truncate(mark);
    out
}

fn exec_stmts<'a>(env: &Env<'a>, stmts: &'a [Stmt]) -> Eval<Option<Outcome>> {
    for s in stmts {
        let out = match s {
            Stmt::Request { response, awaited, .. } => {
                let name = response.name.value.as_str();
                let stop = match awaited {
                    Awaited::Request { arg, .. } => Stop::Request { instruction: expand_value(env, arg)?, response: name.to_string() },
                    Awaited::Call(value) => match returned(env, value)? {
                        Outcome::Ref { target, args } => Stop::Call { target, args, response: name.to_string() },
                        _ => Stop::Error(format!("`{name}` awaits a template; only a function can be awaited")),
                    },
                };
                let Some(raw) = env.inputs.get(name) else {
                    return Err(stop);
                };
                check_answer(env.f, response, raw)?;
                env.answers.borrow_mut().push(response);
                None
            }
            Stmt::Return(r) => Some(returned(env, &r.value)?),
            Stmt::If { cond, then, .. } => {
                if eval(env, cond)? {
                    exec(env, then)?
                } else {
                    None
                }
            }
            Stmt::Switch { subject, cases, .. } => {
                let input = declared(env, &subject.value)?;
                let members = match &input.ty {
                    InputType::Choice(members) => members,
                    InputType::Text | InputType::Format(_) => {
                        return Err(Stop::Error(string_in_condition(&subject.value, &input.ty)));
                    }
                    InputType::Bool => return Err(bool_in_switch(&subject.value)),
                    InputType::Struct(_) => return Err(Stop::Error(struct_in_use(&subject.value))),
                };
                let raw = provided(env, &subject.value)?;
                check_member(&subject.value, members, raw)?;
                match find_case(cases, raw) {
                    Some(c) => exec(env, &c.body)?,
                    None => return Err(Stop::Error(format!("no case for `{raw}` in `{}`", subject.value))),
                }
            }
        };
        if out.is_some() {
            return Ok(out);
        }
    }
    Ok(None)
}

/// The outcome of a returned value, or of the target of an `await`.
fn returned(env: &Env, value: &ReturnValue) -> Eval<Outcome> {
    Ok(match value {
        ReturnValue::Text(t) => Outcome::Text(t.value.clone()),
        ReturnValue::Ref(reference) => Outcome::Ref { target: target(env, &reference.value)?, args: Vec::new() },
        ReturnValue::Template(lit) => Outcome::Text(expand_template(env, lit)?),
        ReturnValue::Call(call) => match env.bindings.get(call.callee.key()) {
            // A hand-off: the arguments are the values of the target's parameters, by name.
            Some(Binding::Function(path)) => Outcome::Ref { target: path.clone(), args: named_values(env, &call.args)? },
            _ => Outcome::Text(expand_call(env, call)?),
        },
    })
}

/// The case group whose labels include `raw`.
fn find_case<'a>(cases: &'a [Case], raw: &str) -> Option<&'a Case> {
    cases.iter().find(|c| c.values.iter().any(|v| v.value == raw))
}

/// Text of a template literal in a function. Each `${name}` is a union or a `string` parameter, read now.
fn expand_template(env: &Env, lit: &TemplateLit) -> Eval<String> {
    let mut out = String::new();
    for piece in &lit.pieces {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Name(n) => {
                let input = declared(env, &n.value)?;
                match &input.ty {
                    InputType::Bool => {
                        return Err(Stop::Error(format!("boolean parameter `{}` cannot be interpolated", n.value)));
                    }
                    InputType::Choice(members) => check_member(&n.value, members, provided(env, &n.value)?)?,
                    InputType::Struct(_) => return Err(Stop::Error(struct_in_use(&n.value))),
                    InputType::Text | InputType::Format(_) => {}
                }
                out.push_str(provided(env, &n.value)?);
            }
        }
    }
    Ok(out)
}

/// The value of a parameter or an answer of this function, or a `true`/`false` literal, for a bare name
/// argument. A `boolean` is passed as `true` or `false`, a union as its member.
fn arg_value(env: &Env, name: &str) -> Eval<String> {
    let input = match declared(env, name) {
        // A `true` or `false` literal. The checker allows it only for a `boolean` parameter, and no name is one.
        Err(_) if matches!(name, "true" | "false") => return Ok(name.to_string()),
        found => found?,
    };
    if let InputType::Struct(_) = input.ty {
        return Err(Stop::Error(struct_in_use(name)));
    }
    let raw = provided(env, name)?;
    match &input.ty {
        InputType::Bool => Ok(parse_bool(name, raw)?.to_string()),
        InputType::Choice(members) => check_member(name, members, raw).map(|()| raw.to_string()),
        InputType::Text | InputType::Format(_) | InputType::Struct(_) => Ok(raw.to_string()),
    }
}

/// The value of each named argument, in the order written. The checker has already rejected a positional one.
fn named_values(env: &Env, args: &[Arg]) -> Eval<Vec<(String, String)>> {
    args.iter()
        .map(|a| match &a.name {
            Some(name) => Ok((name.value.clone(), expand_value(env, &a.value)?)),
            None => Err(Stop::Error("an argument needs a name, like `name=value`".to_string())),
        })
        .collect()
}

/// Text of a template call: the template's own literal, with each parameter replaced by the argument of that name.
fn expand_call(env: &Env, call: &Call) -> Eval<String> {
    let name = call.callee.key();
    let Some(Binding::Template { function, .. }) = env.bindings.get(name) else {
        return Err(Stop::Error(format!("`{name}` is not an imported template")));
    };
    let args = named_values(env, &call.args)?;
    let lit = match function.body.as_slice() {
        [Stmt::Return(Spanned { value: ReturnValue::Template(lit), .. })] => lit,
        _ => return Err(Stop::Error(format!("`{name}` is not a template"))),
    };
    let mut out = String::new();
    for piece in &lit.pieces {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Name(n) => {
                let arg = args.iter().find(|(key, _)| *key == n.value).map(|(_, value)| value);
                let arg = arg.ok_or_else(|| Stop::Error(format!("unknown parameter `{}`", n.value)))?;
                if let Some(InputType::Format(format)) = function.input(&n.value).map(|i| &i.ty) {
                    format.check(&n.value, arg).map_err(Stop::Error)?;
                }
                out.push_str(arg);
            }
        }
    }
    Ok(out)
}

/// Text of a string, a template literal, or a template call.
fn expand_value(env: &Env, v: &Value) -> Eval<String> {
    match v {
        Value::Str(s) => Ok(s.value.clone()),
        Value::Template(lit) => expand_template(env, lit),
        Value::Name(n) => arg_value(env, &n.value),
        Value::Call(call) => expand_call(env, call),
    }
}

fn target(env: &Env, reference: &str) -> Eval<String> {
    match env.bindings.get(reference) {
        Some(Binding::Function(path)) => Ok(path.clone()),
        _ => Err(Stop::Error(format!("`@{reference}` is not a function"))),
    }
}

fn eval(env: &Env, e: &Expr) -> Eval<bool> {
    match e {
        Expr::Var(v) => {
            let input = declared(env, &v.value)?;
            match input.ty {
                InputType::Text | InputType::Format(_) => Err(Stop::Error(string_in_condition(&v.value, &input.ty))),
                InputType::Struct(_) => Err(Stop::Error(struct_in_use(&v.value))),
                InputType::Bool => parse_bool(&v.value, provided(env, &v.value)?),
                InputType::Choice(_) => Err(Stop::Error(format!(
                    "union parameter `{0}` needs a comparison, like `{0} === \"...\"`",
                    v.value
                ))),
            }
        }
        Expr::Compare { var, value, equal } => {
            let input = declared(env, &var.value)?;
            let InputType::Choice(members) = &input.ty else {
                return Err(match input.ty {
                    InputType::Text | InputType::Format(_) => Stop::Error(string_in_condition(&var.value, &input.ty)),
                    InputType::Struct(_) => Stop::Error(struct_in_use(&var.value)),
                    _ => bool_compared(&var.value),
                });
            };
            let raw = provided(env, &var.value)?;
            check_member(&var.value, members, raw)?;
            Ok((raw == value.value) == *equal)
        }
        Expr::Not(inner, _) => Ok(!eval(env, inner)?),
        // `&&` and `||` short-circuit, so the right side may stay unneeded.
        Expr::And(a, b) => Ok(eval(env, a)? && eval(env, b)?),
        Expr::Or(a, b) => Ok(eval(env, a)? || eval(env, b)?),
    }
}

fn declared<'a>(env: &Env<'a>, name: &str) -> Eval<&'a Input> {
    // `answer.field` is in scope when the struct answer is.
    let base = name.split_once('.').map_or(name, |(base, _)| base);
    let in_scope = env.answers.borrow().iter().any(|i| i.name.value == base);
    if base != name && in_scope {
        return env.f.response(name).ok_or_else(|| Stop::Error(format!("unknown parameter `{name}`")));
    }
    let answer = env.answers.borrow().iter().rev().find(|i| i.name.value == name).copied();
    answer
        .or_else(|| env.f.input(name))
        .ok_or_else(|| Stop::Error(format!("unknown parameter `{name}`")))
}

/// Checks a value the agent gave for a `const` answer, as it checks a parameter.
fn check_answer(f: &Function, input: &Input, raw: &str) -> Eval<()> {
    let name = input.name.value.as_str();
    match &input.ty {
        InputType::Bool => parse_bool(name, raw).map(|_| ()),
        InputType::Choice(members) => check_member(name, members, raw),
        InputType::Text => Ok(()),
        InputType::Format(format) => format.check(name, raw).map_err(Stop::Error),
        InputType::Struct(_) => match f.answer_struct(name) {
            Some(Ok(s)) => struct_values(name, s, raw).map(|_| ()).map_err(Stop::Error),
            Some(Err(msg)) => Err(Stop::Error(msg.clone())),
            None => Err(Stop::Error(format!("`{name}` is not a struct answer"))),
        },
    }
}

/// Checks a struct answer: a JSON object with every field of `s`, no other field, and a valid value in each.
/// Returns the value of each field as text, keyed `name.field`. A string is its text, a boolean is `true` or `false`.
pub fn struct_values(name: &str, s: &Struct, raw: &str) -> Result<Vec<(String, String)>, String> {
    let fields: Vec<&str> = s.fields.iter().map(|f| f.name.value.as_str()).collect();
    let example = fields.iter().map(|f| format!("\"{f}\": ...")).collect::<Vec<_>>().join(", ");
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str(raw) else {
        return Err(format!("the answer `{name}` must be a JSON object, like {{{example}}}"));
    };
    if let Some(extra) = map.keys().find(|k| !fields.contains(&k.as_str())) {
        return Err(format!("`{extra}` is not a field of `{}`; fields are {}", s.name.value, fields.join(", ")));
    }
    let mut out = Vec::new();
    for field in &s.fields {
        let key = format!("{name}.{}", field.name.value);
        let Some(value) = map.get(&field.name.value) else {
            return Err(format!("the answer `{name}` is missing field `{}`", field.name.value));
        };
        let text = match (&field.ty, value) {
            (InputType::Bool, serde_json::Value::Bool(b)) => b.to_string(),
            (InputType::Bool, _) => return Err(format!("field `{key}` must be true or false")),
            (InputType::Text | InputType::Format(_), serde_json::Value::String(t)) if t.is_empty() => {
                return Err(format!("field `{key}` is empty; give some text"));
            }
            (InputType::Text, serde_json::Value::String(t)) => t.clone(),
            (InputType::Format(format), serde_json::Value::String(t)) => {
                format.check(&key, t)?;
                t.clone()
            }
            (InputType::Choice(members), serde_json::Value::String(t)) => {
                if let Err(Stop::Error(m)) = check_member(&key, members, t) {
                    return Err(m);
                }
                t.clone()
            }
            (InputType::Text | InputType::Format(_) | InputType::Choice(_), _) => {
                return Err(format!("field `{key}` must be a string"));
            }
            (InputType::Struct(_), _) => return Err(format!("field `{key}` cannot be a struct")),
        };
        out.push((key, text));
    }
    Ok(out)
}

/// Checks one value the agent gives for a parameter or `const` answer. Returns the error text when it is not valid.
pub fn check_value(f: &Function, input: &Input, raw: &str) -> Result<(), String> {
    match check_answer(f, input, raw) {
        Err(Stop::Error(msg)) => Err(msg),
        _ => Ok(()),
    }
}

/// The raw value the agent gave. A missing value stops the run with `Need`.
fn provided<'a>(env: &Env<'a>, name: &str) -> Eval<&'a str> {
    match env.inputs.get(name) {
        Some(v) => Ok(v.as_str()),
        None => Err(Stop::Need(name.to_string())),
    }
}

fn parse_bool(name: &str, raw: &str) -> Eval<bool> {
    match raw.to_ascii_lowercase().as_str() {
        "true" | "yes" => Ok(true),
        "false" | "no" => Ok(false),
        _ => Err(Stop::Error(format!(
            "invalid value `{raw}` for `{name}`; expected true, false, yes or no"
        ))),
    }
}

fn check_member(name: &str, members: &[Spanned<String>], raw: &str) -> Eval<()> {
    if members.iter().any(|o| o.value == raw) {
        return Ok(());
    }
    let list = members.iter().map(|o| format!("`{}`", o.value)).collect::<Vec<_>>().join(", ");
    Err(Stop::Error(format!("invalid value `{raw}` for `{name}`; expected one of: {list}")))
}

fn bool_compared(name: &str) -> Stop {
    Stop::Error(format!("boolean parameter `{name}` cannot be compared; use `if ({name})` or `if (!{name})`"))
}

fn bool_in_switch(name: &str) -> Stop {
    Stop::Error(format!("boolean parameter `{name}` cannot be used in a switch; use `if ({name})` or `if (!{name})`"))
}

#[cfg(test)]
#[path = "eval_tests.rs"]
mod tests;
