//! Runs a function against the inputs the agent has.

use std::collections::{BTreeMap, HashMap};

use crate::ast::{Case, Expr, Function, Input, InputType, ReturnValue, Spanned, Stmt};

/// Result of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Text(String),
    /// The project path of the function to run next, such as `ci/fix`.
    Ref(String),
    NeedInput(String),
}

/// Why evaluation stopped before a return.
enum Stop {
    Need(String),
    Error(String),
}

type Eval<T> = Result<T, Stop>;

struct Env<'a> {
    f: &'a Function,
    inputs: &'a HashMap<String, String>,
    /// Import binding to project path, from `Project::import_targets`.
    targets: &'a BTreeMap<String, String>,
}

pub fn run(
    f: &Function,
    inputs: &HashMap<String, String>,
    targets: &BTreeMap<String, String>,
) -> Result<Outcome, String> {
    let mut unknown: Vec<&String> = inputs.keys().filter(|k| f.input(k).is_none()).collect();
    unknown.sort();
    if let Some(name) = unknown.first() {
        return Err(format!("unknown input `{name}`"));
    }
    let env = Env { f, inputs, targets };
    match exec(&env, &f.body) {
        Ok(Some(out)) => Ok(out),
        Ok(None) => Err("no return reached; every path must return".to_string()),
        Err(Stop::Need(name)) => Ok(Outcome::NeedInput(name)),
        Err(Stop::Error(msg)) => Err(msg),
    }
}

fn exec(env: &Env, stmts: &[Stmt]) -> Eval<Option<Outcome>> {
    for s in stmts {
        let out = match s {
            Stmt::Return(r) => Some(match &r.value {
                ReturnValue::Text(t) => Outcome::Text(t.clone()),
                ReturnValue::Ref(binding) => Outcome::Ref(target(env, binding)?),
            }),
            Stmt::If { cond, then, otherwise, .. } => {
                if eval(env, cond)? {
                    exec(env, then)?
                } else if let Some(o) = otherwise {
                    exec(env, o)?
                } else {
                    None
                }
            }
            Stmt::Switch { subject, cases, default, .. } => {
                let input = declared(env, &subject.value)?;
                let InputType::Choice(members) = &input.ty else {
                    return Err(bool_in_switch(&subject.value));
                };
                let raw = provided(env, &subject.value)?;
                check_member(&subject.value, members, raw)?;
                match find_case(cases, raw) {
                    Some(c) => exec(env, &c.body)?,
                    None => match default {
                        Some(d) => exec(env, d)?,
                        None => None,
                    },
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

fn target(env: &Env, binding: &str) -> Eval<String> {
    env.targets
        .get(binding)
        .cloned()
        .ok_or_else(|| Stop::Error(format!("`{binding}` is not an imported function")))
}

fn eval(env: &Env, e: &Expr) -> Eval<bool> {
    match e {
        Expr::Var(v) => {
            let input = declared(env, &v.value)?;
            match input.ty {
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
                return Err(bool_compared(&var.value));
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
    env.f.input(name).ok_or_else(|| Stop::Error(format!("unknown parameter `{name}`")))
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
    use crate::parser::parse;

    fn func(src: &str) -> Function {
        let (f, d) = parse(src);
        assert!(d.is_empty(), "{d:?}");
        f
    }

    fn inputs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn no_targets() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn run_plain(f: &Function, pairs: &[(&str, &str)]) -> Result<Outcome, String> {
        run(f, &inputs(pairs), &no_targets())
    }

    const TWO_BOOLS: &str = "/** d */\nexport default function f(\n  /** A */ a: boolean,\n  /** B */ b: boolean,\n) {\n";

    #[test]
    fn text_return() {
        let f = func("/** d */\nexport default function f() { return \"done\" }");
        assert_eq!(run_plain(&f, &[]), Ok(Outcome::Text("done".into())));
    }

    #[test]
    fn ref_return_resolves_the_import_to_its_project_path() {
        let f = func("import fixCi from \"./ci/fix\";\n/** d */\nexport default function f() { return fixCi }");
        let targets = BTreeMap::from([("fixCi".to_string(), "ci/fix".to_string())]);
        assert_eq!(run(&f, &inputs(&[]), &targets), Ok(Outcome::Ref("ci/fix".into())));
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
        let f = func("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } return \"2\" }");
        let err = run_plain(&f, &[("a", "maybe")]).unwrap_err();
        assert!(err.contains("expected true, false, yes or no"), "{err}");
    }

    #[test]
    fn invalid_union_value_is_error() {
        let f = func(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c === \"x\") { return \"1\" } return \"2\" }",
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
    fn switch_falls_through_to_default() {
        let f = func(
            "/** d */\nexport default function f(\n  /** C */ c: \"green\" | \"red\" | \"pending\",\n) {\n\
             switch (c) {\n  case \"green\":\n    return \"go\";\n  case \"red\":\n    return \"fix\";\n  default:\n    return \"wait\";\n}\n}",
        );
        assert_eq!(run_plain(&f, &[("c", "green")]), Ok(Outcome::Text("go".into())));
        assert_eq!(run_plain(&f, &[("c", "red")]), Ok(Outcome::Text("fix".into())));
        assert_eq!(run_plain(&f, &[("c", "pending")]), Ok(Outcome::Text("wait".into())));
        assert_eq!(run_plain(&f, &[]), Ok(Outcome::NeedInput("c".into())));
    }

    #[test]
    fn grouped_labels_run_one_body() {
        let f = func(
            "/** d */\nexport default function f(\n  /** C */ c: \"a\" | \"b\" | \"c\",\n) {\n switch (c) {\n  case \"a\":\n  case \"b\":\n    return \"ab\";\n  default:\n    return \"other\";\n}\n}",
        );
        assert_eq!(run_plain(&f, &[("c", "a")]), Ok(Outcome::Text("ab".into())));
        assert_eq!(run_plain(&f, &[("c", "b")]), Ok(Outcome::Text("ab".into())));
        assert_eq!(run_plain(&f, &[("c", "c")]), Ok(Outcome::Text("other".into())));
    }

    #[test]
    fn not_equal_comparison() {
        let f = func(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c !== \"x\") { return \"not x\" } return \"x\" }",
        );
        assert_eq!(run_plain(&f, &[("c", "y")]), Ok(Outcome::Text("not x".into())));
        assert_eq!(run_plain(&f, &[("c", "x")]), Ok(Outcome::Text("x".into())));
    }

    #[test]
    fn yes_no_and_case_insensitive_bools() {
        let f = func("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"t\" } return \"f\" }");
        assert_eq!(run_plain(&f, &[("a", "YES")]), Ok(Outcome::Text("t".into())));
        assert_eq!(run_plain(&f, &[("a", "No")]), Ok(Outcome::Text("f".into())));
        assert_eq!(run_plain(&f, &[("a", "True")]), Ok(Outcome::Text("t".into())));
    }
}
