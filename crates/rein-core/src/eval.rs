//! Runs a function against the inputs the agent has.

use std::collections::HashMap;

use crate::ast::{Expr, Function, Input, InputType, ReturnValue, Spanned, Stmt};

/// Result of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Text(String),
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
}

pub fn run(f: &Function, inputs: &HashMap<String, String>) -> Result<Outcome, String> {
    let mut unknown: Vec<&String> = inputs.keys().filter(|k| f.input(k).is_none()).collect();
    unknown.sort();
    if let Some(name) = unknown.first() {
        return Err(format!("unknown input `{name}`"));
    }
    let env = Env { f, inputs };
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
                ReturnValue::Ref(n) => Outcome::Ref(n.clone()),
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
                let InputType::Choice(opts) = &input.ty else {
                    return Err(bool_in_switch(&subject.value));
                };
                let raw = provided(env, &subject.value)?;
                check_choice(&subject.value, opts, raw)?;
                match cases.iter().find(|c| c.value.value == raw) {
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

fn eval(env: &Env, e: &Expr) -> Eval<bool> {
    match e {
        Expr::Var(v) => {
            let input = declared(env, &v.value)?;
            match input.ty {
                InputType::Bool => parse_bool(&v.value, provided(env, &v.value)?),
                InputType::Choice(_) => Err(Stop::Error(format!(
                    "choice input `{0}` needs a comparison, like `{0} == \"...\"`",
                    v.value
                ))),
            }
        }
        Expr::Compare { var, value, equal } => {
            let input = declared(env, &var.value)?;
            let InputType::Choice(opts) = &input.ty else {
                return Err(bool_compared(&var.value));
            };
            let raw = provided(env, &var.value)?;
            check_choice(&var.value, opts, raw)?;
            Ok((raw == value.value) == *equal)
        }
        Expr::Not(inner, _) => Ok(!eval(env, inner)?),
        // `&&` and `||` short-circuit, so the right side may stay unneeded.
        Expr::And(a, b) => Ok(eval(env, a)? && eval(env, b)?),
        Expr::Or(a, b) => Ok(eval(env, a)? || eval(env, b)?),
    }
}

fn declared<'a>(env: &Env<'a>, name: &str) -> Eval<&'a Input> {
    env.f.input(name).ok_or_else(|| Stop::Error(format!("unknown input `{name}`")))
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

fn check_choice(name: &str, opts: &[Spanned<String>], raw: &str) -> Eval<()> {
    if opts.iter().any(|o| o.value == raw) {
        return Ok(());
    }
    let list = opts.iter().map(|o| format!("`{}`", o.value)).collect::<Vec<_>>().join(", ");
    Err(Stop::Error(format!("invalid value `{raw}` for `{name}`; expected one of: {list}")))
}

fn bool_compared(name: &str) -> Stop {
    Stop::Error(format!("bool input `{name}` cannot be compared; use `if {name}` or `if not {name}`"))
}

fn bool_in_switch(name: &str) -> Stop {
    Stop::Error(format!("bool input `{name}` cannot be used in a switch; use `if {name}` or `if not {name}`"))
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

    #[test]
    fn text_return() {
        let f = func("description \"d\"\nreturn \"done\"");
        assert_eq!(run(&f, &inputs(&[])), Ok(Outcome::Text("done".into())));
    }

    #[test]
    fn ref_return() {
        let f = func("description \"d\"\nreturn @ci/fix");
        assert_eq!(run(&f, &inputs(&[])), Ok(Outcome::Ref("ci/fix".into())));
    }

    #[test]
    fn and_does_not_need_right_side_when_left_is_false() {
        let f = func(
            "description \"d\"\ninput a: bool \"a\"\ninput b: bool \"b\"\nif a and b { return \"both\" }\nreturn \"neither\"",
        );
        assert_eq!(run(&f, &inputs(&[("a", "false")])), Ok(Outcome::Text("neither".into())));
        assert_eq!(run(&f, &inputs(&[("a", "true")])), Ok(Outcome::NeedInput("b".into())));
    }

    #[test]
    fn or_short_circuits() {
        let f = func(
            "description \"d\"\ninput a: bool \"a\"\ninput b: bool \"b\"\nif a or b { return \"either\" }\nreturn \"none\"",
        );
        assert_eq!(run(&f, &inputs(&[("a", "true")])), Ok(Outcome::Text("either".into())));
    }

    #[test]
    fn invalid_bool_value_is_error() {
        let f = func("description \"d\"\ninput a: bool \"a\"\nif a { return \"1\" }\nreturn \"2\"");
        let err = run(&f, &inputs(&[("a", "maybe")])).unwrap_err();
        assert!(err.contains("expected true, false, yes or no"), "{err}");
    }

    #[test]
    fn invalid_choice_value_is_error() {
        let f = func(
            "description \"d\"\ninput c: choice(\"x\", \"y\") \"c\"\nif c == \"x\" { return \"1\" }\nreturn \"2\"",
        );
        let err = run(&f, &inputs(&[("c", "purple")])).unwrap_err();
        assert!(err.contains("expected one of: `x`, `y`"), "{err}");
    }

    #[test]
    fn unknown_input_is_error() {
        let f = func("description \"d\"\nreturn \"x\"");
        assert_eq!(run(&f, &inputs(&[("zzz", "1")])), Err("unknown input `zzz`".into()));
    }

    #[test]
    fn switch_falls_through_to_default() {
        let f = func(
            "description \"d\"\ninput c: choice(\"green\", \"red\", \"pending\") \"c\"\n\
             switch c {\n case \"green\" { return \"go\" }\n case \"red\" { return @ci/fix }\n default { return \"wait\" }\n}",
        );
        assert_eq!(run(&f, &inputs(&[("c", "green")])), Ok(Outcome::Text("go".into())));
        assert_eq!(run(&f, &inputs(&[("c", "red")])), Ok(Outcome::Ref("ci/fix".into())));
        assert_eq!(run(&f, &inputs(&[("c", "pending")])), Ok(Outcome::Text("wait".into())));
        assert_eq!(run(&f, &inputs(&[])), Ok(Outcome::NeedInput("c".into())));
    }

    #[test]
    fn not_equal_comparison() {
        let f = func(
            "description \"d\"\ninput c: choice(\"x\", \"y\") \"c\"\nif c != \"x\" { return \"not x\" }\nreturn \"x\"",
        );
        assert_eq!(run(&f, &inputs(&[("c", "y")])), Ok(Outcome::Text("not x".into())));
        assert_eq!(run(&f, &inputs(&[("c", "x")])), Ok(Outcome::Text("x".into())));
    }

    #[test]
    fn yes_no_and_case_insensitive_bools() {
        let f = func("description \"d\"\ninput a: bool \"a\"\nif a { return \"t\" }\nreturn \"f\"");
        assert_eq!(run(&f, &inputs(&[("a", "YES")])), Ok(Outcome::Text("t".into())));
        assert_eq!(run(&f, &inputs(&[("a", "No")])), Ok(Outcome::Text("f".into())));
        assert_eq!(run(&f, &inputs(&[("a", "True")])), Ok(Outcome::Text("t".into())));
    }
}
