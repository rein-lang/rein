//! Runs a function against the inputs the agent has.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use crate::ast::{Arg, Binding, Call, Case, Expr, Function, Input, InputType, Piece, ReturnValue, Spanned, Stmt, TemplateLit, Value};
use crate::check::string_in_condition;

/// Result of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Text(String),
    /// The function to run next, by project path such as `ci/fix`. `args` are the values of its `string`
    /// parameters, by name, as written in `return @./target(name=value);`. They are empty for a plain return.
    Ref { target: String, args: Vec<(String, String)> },
    NeedInput(String),
    /// The agent must answer `response`, after doing `instruction`.
    Request { instruction: String, response: String },
}

/// Why evaluation stopped before a return.
enum Stop {
    Need(String),
    Error(String),
    Request { instruction: String, response: String },
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
    let env = Env { f, inputs, bindings, answers: RefCell::new(Vec::new()) };
    match exec(&env, &f.body) {
        Ok(Some(out)) => Ok(out),
        Ok(None) => Err("no return reached; every path must return".to_string()),
        Err(Stop::Need(name)) => Ok(Outcome::NeedInput(name)),
        Err(Stop::Error(msg)) => Err(msg),
        Err(Stop::Request { instruction, response }) => Ok(Outcome::Request { instruction, response }),
    }
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
            Stmt::Request { response, arg, .. } => {
                let instruction = expand_value(env, arg)?;
                let name = response.name.value.as_str();
                let Some(raw) = env.inputs.get(name) else {
                    return Err(Stop::Request { instruction, response: name.to_string() });
                };
                check_answer(response, raw)?;
                env.answers.borrow_mut().push(response);
                None
            }
            Stmt::Return(r) => Some(match &r.value {
                ReturnValue::Text(t) => Outcome::Text(t.clone()),
                ReturnValue::Ref(reference) => Outcome::Ref { target: target(env, &reference.value)?, args: Vec::new() },
                ReturnValue::Template(lit) => Outcome::Text(expand_template(env, lit)?),
                ReturnValue::Call(call) => match env.bindings.get(call.callee.key()) {
                    // A hand-off: the arguments are the values of the target's `string` parameters, by name.
                    Some(Binding::Function(path)) => Outcome::Ref { target: path.clone(), args: named_values(env, &call.args)? },
                    _ => Outcome::Text(expand_call(env, call)?),
                },
            }),
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
                    InputType::Text => return Err(Stop::Error(string_in_condition(&subject.value))),
                    InputType::Bool => return Err(bool_in_switch(&subject.value)),
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
                    InputType::Text => {}
                }
                out.push_str(provided(env, &n.value)?);
            }
        }
    }
    Ok(out)
}

/// The value of a `string` parameter of this function, for a bare name argument.
fn string_input(env: &Env, name: &str) -> Eval<String> {
    match declared(env, name)?.ty {
        InputType::Text => Ok(provided(env, name)?.to_string()),
        _ => Err(Stop::Error(format!("`{name}` is not a string input"))),
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
                out.push_str(arg.ok_or_else(|| Stop::Error(format!("unknown parameter `{}`", n.value)))?);
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
        Value::Name(n) => string_input(env, &n.value),
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
                InputType::Text => Err(Stop::Error(string_in_condition(&v.value))),
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
                    InputType::Text => Stop::Error(string_in_condition(&var.value)),
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
    let answer = env.answers.borrow().iter().rev().find(|i| i.name.value == name).copied();
    answer
        .or_else(|| env.f.input(name))
        .ok_or_else(|| Stop::Error(format!("unknown parameter `{name}`")))
}

/// Checks a value the agent gave for a `const` answer, as it checks a parameter.
fn check_answer(input: &Input, raw: &str) -> Eval<()> {
    let name = input.name.value.as_str();
    match &input.ty {
        InputType::Bool => parse_bool(name, raw).map(|_| ()),
        InputType::Choice(members) => check_member(name, members, raw),
        InputType::Text if raw.is_empty() => Err(Stop::Error(format!("the answer `{name}` is empty; give some text"))),
        InputType::Text => Ok(()),
    }
}

/// Checks one value the agent gives for a parameter or `const` answer. Returns the error text when it is not valid.
pub fn check_value(input: &Input, raw: &str) -> Result<(), String> {
    match check_answer(input, raw) {
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
