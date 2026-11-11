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
mod tests {
    use super::*;
    use crate::ast::Binding;
    use crate::parser::parse;

    fn func(src: &str) -> Function {
        let (f, d) = parse(src);
        assert!(d.is_empty(), "{d:?}");
        f
    }

    fn inputs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn no_bindings() -> BTreeMap<String, Binding<'static>> {
        BTreeMap::new()
    }

    fn run_plain(f: &Function, pairs: &[(&str, &str)]) -> Result<Outcome, String> {
        run(f, &inputs(pairs), &no_bindings())
    }

    fn std_binding(name: &str) -> (String, Binding<'static>) {
        (name.to_string(), crate::stdlib::item(name).unwrap())
    }

    const TWO_BOOLS: &str = "/** d */\nexport default function f(\n  /** A */ a: boolean,\n  /** B */ b: boolean,\n) {\n";

    #[test]
    fn text_return() {
        let f = func("/** d */\nexport default function f() { return \"done\" }");
        assert_eq!(run_plain(&f, &[]), Ok(Outcome::Text("done".into())));
    }

    #[test]
    fn ref_return_resolves_the_import_to_its_project_path() {
        let f = func("/** d */\nexport default function f() { return @./ci/fix }");
        let bindings = BTreeMap::from([("./ci/fix".to_string(), Binding::Function("ci/fix".into()))]);
        assert_eq!(
            run(&f, &inputs(&[]), &bindings),
            Ok(Outcome::Ref { target: "ci/fix".into(), args: vec![] })
        );
    }

    #[test]
    fn and_does_not_need_right_side_when_left_is_false() {
        let f = func(&format!("{TWO_BOOLS}  if (a && b) {{ return \"both\" }}\n  return \"neither\"\n}}"));
        assert_eq!(run_plain(&f, &[("a", "false")]), Ok(Outcome::Text("neither".into())));
        assert_eq!(run_plain(&f, &[("a", "true")]), Ok(Outcome::NeedInput("b".into())));
    }

    #[test]
    fn or_short_circuits() {
        let f = func(&format!("{TWO_BOOLS}  if (a || b) {{ return \"either\" }}\n  return \"none\"\n}}"));
        assert_eq!(run_plain(&f, &[("a", "true")]), Ok(Outcome::Text("either".into())));
    }

    #[test]
    fn invalid_bool_value_is_error() {
        let f = func("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } return \"2\"  }");
        let err = run_plain(&f, &[("a", "maybe")]).unwrap_err();
        assert!(err.contains("expected true, false, yes or no"), "{err}");
    }

    #[test]
    fn invalid_union_value_is_error() {
        let f = func(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c === \"x\") { return \"1\" } return \"2\"  }",
        );
        let err = run_plain(&f, &[("c", "purple")]).unwrap_err();
        assert!(err.contains("expected one of: `x`, `y`"), "{err}");
    }

    #[test]
    fn unknown_input_is_error() {
        let f = func("/** d */\nexport default function f() { return \"x\" }");
        assert_eq!(run_plain(&f, &[("zzz", "1")]), Err("unknown input `zzz`".into()));
    }

    #[test]
    fn switch_runs_the_matching_case() {
        let f = func(
            "/** d */\nexport default function f(\n  /** C */ c: \"green\" | \"red\" | \"pending\",\n) {\n\
             switch (c) {\n  case \"green\":\n    return \"go\";\n  case \"red\":\n    return \"fix\";\n  case \"pending\":\n    return \"wait\";\n}\n}",
        );
        assert_eq!(run_plain(&f, &[("c", "green")]), Ok(Outcome::Text("go".into())));
        assert_eq!(run_plain(&f, &[("c", "red")]), Ok(Outcome::Text("fix".into())));
        assert_eq!(run_plain(&f, &[("c", "pending")]), Ok(Outcome::Text("wait".into())));
        assert_eq!(run_plain(&f, &[]), Ok(Outcome::NeedInput("c".into())));
    }

    #[test]
    fn grouped_labels_run_one_body() {
        let f = func(
            "/** d */\nexport default function f(\n  /** C */ c: \"a\" | \"b\" | \"c\",\n) {\n switch (c) {\n  case \"a\":\n  case \"b\":\n    return \"ab\";\n  case \"c\":\n    return \"other\";\n}\n}",
        );
        assert_eq!(run_plain(&f, &[("c", "a")]), Ok(Outcome::Text("ab".into())));
        assert_eq!(run_plain(&f, &[("c", "b")]), Ok(Outcome::Text("ab".into())));
        assert_eq!(run_plain(&f, &[("c", "c")]), Ok(Outcome::Text("other".into())));
    }

    #[test]
    fn not_equal_comparison() {
        let f = func(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c !== \"x\") { return \"not x\" } return \"x\"  }",
        );
        assert_eq!(run_plain(&f, &[("c", "y")]), Ok(Outcome::Text("not x".into())));
        assert_eq!(run_plain(&f, &[("c", "x")]), Ok(Outcome::Text("x".into())));
    }

    #[test]
    fn yes_no_and_case_insensitive_bools() {
        let f = func("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"t\" } return \"f\"  }");
        assert_eq!(run_plain(&f, &[("a", "YES")]), Ok(Outcome::Text("t".into())));
        assert_eq!(run_plain(&f, &[("a", "No")]), Ok(Outcome::Text("f".into())));
        assert_eq!(run_plain(&f, &[("a", "True")]), Ok(Outcome::Text("t".into())));
    }

    #[test]
    fn std_template_call_expands_to_text() {
        let f = func(r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  if (c === "x") {
    return delegate(model="use opus model", task=`code review ${c}`);
  } return delegate(model="use haiku model", task="code review the pr"); 
}"#);
        let bindings = BTreeMap::from([std_binding("delegate")]);
        assert_eq!(
            run(&f, &inputs(&[("c", "x")]), &bindings),
            Ok(Outcome::Text("Delegate this task to a subagent.\nModel: use opus model\nTask: code review x".into()))
        );
        assert_eq!(
            run(&f, &inputs(&[("c", "y")]), &bindings),
            Ok(Outcome::Text("Delegate this task to a subagent.\nModel: use haiku model\nTask: code review the pr".into()))
        );
    }

    #[test]
    fn user_template_call_expands_its_own_params() {
        let tpl = func(r#"/** Pick. */
export default function tpl(
  /** First. */ a: string,
  /** Second. */ b: string,
) {
  return `${a} then ${b}`;
}"#);
        let f = func(r#"/** d */
export default function f() {
  return @./tpl(a="one", b="two");
}"#);
        let bindings = BTreeMap::from([(
            "./tpl".to_string(),
            Binding::Template { path: Some("tpl".into()), function: Box::leak(Box::new(tpl)) },
        )]);
        assert_eq!(run(&f, &inputs(&[]), &bindings), Ok(Outcome::Text("one then two".into())));
    }

    #[test]
    fn template_literal_return_expands_the_union_value() {
        let f = func("/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) {\n  return `Pick ${c}`;\n}");
        assert_eq!(run_plain(&f, &[("c", "y")]), Ok(Outcome::Text("Pick y".into())));
        assert_eq!(run_plain(&f, &[]), Ok(Outcome::NeedInput("c".into())));
    }

    #[test]
    fn running_a_template_is_an_error() {
        let f = func("/** d */\nexport default function t(\n  /** A */ a: string,\n) { return `${a}` }");
        assert_eq!(run_plain(&f, &[]), Err("templates are called from functions, not run".into()));
    }


    #[test]
    fn request_without_an_answer_stops_for_it() {
        let f = func(r#"/** d */
export default async function f() {
  /** Did it work? */
  const ok: boolean = await request("Do the thing.");
  if (ok) { return "y" }
  return "n"
}"#);
        assert_eq!(
            run_plain(&f, &[]),
            Ok(Outcome::Request { instruction: "Do the thing.".into(), response: "ok".into() })
        );
        assert_eq!(run_plain(&f, &[("ok", "yes")]), Ok(Outcome::Text("y".into())));
        assert_eq!(run_plain(&f, &[("ok", "no")]), Ok(Outcome::Text("n".into())));
    }

    #[test]
    fn request_instruction_can_be_a_std_template_call() {
        let f = func(r#"/** d */
export default async function f() {
  /** Did it work? */
  const ok: boolean = await request(delegate(model="use opus model", task="code review the pr"));
  return "y"
}"#);
        let bindings = BTreeMap::from([std_binding("delegate")]);
        assert_eq!(
            run(&f, &inputs(&[]), &bindings),
            Ok(Outcome::Request {
                instruction: "Delegate this task to a subagent.\nModel: use opus model\nTask: code review the pr".into(),
                response: "ok".into(),
            })
        );
    }

    #[test]
    fn request_instruction_that_interpolates_a_union_needs_it_first() {
        let f = func(r#"/** d */
export default async function f(
  /** C */ c: "x" | "y",
) {
  /** Did it work? */
  const ok: boolean = await request(`fix ${c}`);
  return "y"
}"#);
        assert_eq!(run_plain(&f, &[]), Ok(Outcome::NeedInput("c".into())));
        assert_eq!(
            run_plain(&f, &[("c", "x")]),
            Ok(Outcome::Request { instruction: "fix x".into(), response: "ok".into() })
        );
    }

    #[test]
    fn request_answer_is_validated_when_used() {
        let f = func(r#"/** d */
export default async function f() {
  /** Did it work? */
  const ok: boolean = await request("Do it.");
  return "y"
}"#);
        let err = run_plain(&f, &[("ok", "maybe")]).unwrap_err();
        assert!(err.contains("invalid value `maybe` for `ok`"), "{err}");
    }

    #[test]
    fn answer_is_a_parameter_for_the_rest_of_its_block() {
        let f = func(r#"/** d */
export default async function f(
  /** C */ c: "x" | "y",
) {
  if (c === "x") {
    /** Did it work? */
    const ok: boolean = await request("fix it");
    if (ok) { return "fixed" }
    return "not fixed"
  }
  return "other"
}"#);
        assert_eq!(run_plain(&f, &[("c", "y")]), Ok(Outcome::Text("other".into())));
        assert_eq!(
            run_plain(&f, &[("c", "x")]),
            Ok(Outcome::Request { instruction: "fix it".into(), response: "ok".into() })
        );
        assert_eq!(run_plain(&f, &[("c", "x"), ("ok", "no")]), Ok(Outcome::Text("not fixed".into())));
    }

    #[test]
    fn hand_off_returns_the_string_arguments() {
        let f = func(r#"/** d */
export default function f(
  /** Link */ pr: string,
) {
  return @./merge(pr=pr);
}"#);
        let bindings = BTreeMap::from([("./merge".to_string(), Binding::Function("merge".into()))]);
        assert_eq!(
            run(&f, &inputs(&[("pr", "https://example.com/pr/1")]), &bindings),
            Ok(Outcome::Ref { target: "merge".into(), args: vec![("pr".into(), "https://example.com/pr/1".into())] })
        );
    }

    #[test]
    fn string_input_is_interpolated_in_a_template_literal() {
        // A union makes this a normal function, so `pr` is an input and not a template parameter.
        let f = func("/** d */\nexport default function f(\n  /** Link */ pr: string,\n  /** C */ c: \"x\" | \"y\",\n) {\n  if (c === \"x\") { return `Review ${pr}` }\n  return `Other ${pr}`;\n}");
        assert_eq!(run_plain(&f, &[("c", "x"), ("pr", "L")]), Ok(Outcome::Text("Review L".into())));
        assert_eq!(run_plain(&f, &[("c", "y"), ("pr", "L")]), Ok(Outcome::Text("Other L".into())));
        assert_eq!(run_plain(&f, &[("c", "x")]), Ok(Outcome::NeedInput("pr".into())));
    }

    #[test]
    fn string_input_is_a_bare_argument_of_a_std_template() {
        let f = func("/** d */\nexport default function f(\n  /** Link */ pr: string,\n) {\n  return delegate(model=\"use opus model\", task=pr);\n}");
        let bindings = BTreeMap::from([std_binding("delegate")]);
        assert_eq!(
            run(&f, &inputs(&[("pr", "L")]), &bindings),
            Ok(Outcome::Text("Delegate this task to a subagent.\nModel: use opus model\nTask: L".into()))
        );
    }

    #[test]
    fn string_input_in_a_condition_is_an_error() {
        let f = func("/** d */\nexport default function f(\n  /** Link */ pr: string,\n) {\n  if (pr) { return \"a\" }\n  return \"b\"\n}");
        assert_eq!(run_plain(&f, &[("pr", "x")]), Err("`pr` is a string input; a condition cannot read it".into()));
    }
}
