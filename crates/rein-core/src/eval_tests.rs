use super::*;
use crate::ast::Binding;
use crate::parser::parse;

fn func(src: &str) -> Function {
    let (f, d) = parse(src);
    assert!(d.is_empty(), "{d:?}");
    f
}

fn inputs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
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

const TWO_BOOLS: &str =
    "/** d */\nexport default function f(\n  /** A */ a: boolean,\n  /** B */ b: boolean,\n) {\n";

#[test]
fn text_return() {
    let f = func("/** d */\nexport default function f() { return `done` }");
    assert_eq!(run_plain(&f, &[]), Ok(Outcome::Text("done".into())));
}

#[test]
fn ref_return_resolves_the_import_to_its_project_path() {
    let f = func("/** d */\nexport default function f() { return @./ci/fix }");
    let bindings = BTreeMap::from([("./ci/fix".to_string(), Binding::Function("ci/fix".into()))]);
    assert_eq!(
        run(&f, &inputs(&[]), &bindings),
        Ok(Outcome::Ref {
            target: "ci/fix".into(),
            args: vec![]
        })
    );
}

#[test]
fn and_does_not_need_right_side_when_left_is_false() {
    let f = func(&format!(
        "{TWO_BOOLS}  if (a && b) {{ return `both` }}\n  return `neither`\n}}"
    ));
    assert_eq!(
        run_plain(&f, &[("a", "false")]),
        Ok(Outcome::Text("neither".into()))
    );
    assert_eq!(
        run_plain(&f, &[("a", "true")]),
        Ok(Outcome::NeedInput("b".into()))
    );
}

#[test]
fn or_short_circuits() {
    let f = func(&format!(
        "{TWO_BOOLS}  if (a || b) {{ return `either` }}\n  return `none`\n}}"
    ));
    assert_eq!(
        run_plain(&f, &[("a", "true")]),
        Ok(Outcome::Text("either".into()))
    );
}

#[test]
fn invalid_bool_value_is_error() {
    let f = func("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return `1` } return `2`  }");
    let err = run_plain(&f, &[("a", "maybe")]).unwrap_err();
    assert!(err.contains("expected true, false, yes or no"), "{err}");
}

#[test]
fn invalid_union_value_is_error() {
    let f = func(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c === \"x\") { return `1` } return `2`  }",
    );
    let err = run_plain(&f, &[("c", "purple")]).unwrap_err();
    assert!(err.contains("expected one of: `x`, `y`"), "{err}");
}

#[test]
fn unknown_input_is_error() {
    let f = func("/** d */\nexport default function f() { return `x` }");
    assert_eq!(
        run_plain(&f, &[("zzz", "1")]),
        Err("unknown input `zzz`".into())
    );
}

#[test]
fn switch_runs_the_matching_case() {
    let f = func(
        "/** d */\nexport default function f(\n  /** C */ c: \"green\" | \"red\" | \"pending\",\n) {\n\
         switch (c) {\n  case \"green\":\n    return `go`;\n  case \"red\":\n    return `fix`;\n  case \"pending\":\n    return `wait`;\n}\n}",
    );
    assert_eq!(
        run_plain(&f, &[("c", "green")]),
        Ok(Outcome::Text("go".into()))
    );
    assert_eq!(
        run_plain(&f, &[("c", "red")]),
        Ok(Outcome::Text("fix".into()))
    );
    assert_eq!(
        run_plain(&f, &[("c", "pending")]),
        Ok(Outcome::Text("wait".into()))
    );
    assert_eq!(run_plain(&f, &[]), Ok(Outcome::NeedInput("c".into())));
}

#[test]
fn grouped_labels_run_one_body() {
    let f = func(
        "/** d */\nexport default function f(\n  /** C */ c: \"a\" | \"b\" | \"c\",\n) {\n switch (c) {\n  case \"a\":\n  case \"b\":\n    return `ab`;\n  case \"c\":\n    return `other`;\n}\n}",
    );
    assert_eq!(run_plain(&f, &[("c", "a")]), Ok(Outcome::Text("ab".into())));
    assert_eq!(run_plain(&f, &[("c", "b")]), Ok(Outcome::Text("ab".into())));
    assert_eq!(
        run_plain(&f, &[("c", "c")]),
        Ok(Outcome::Text("other".into()))
    );
}

#[test]
fn not_equal_comparison() {
    let f = func(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c !== \"x\") { return `not x` } return `x`  }",
    );
    assert_eq!(
        run_plain(&f, &[("c", "y")]),
        Ok(Outcome::Text("not x".into()))
    );
    assert_eq!(run_plain(&f, &[("c", "x")]), Ok(Outcome::Text("x".into())));
}

#[test]
fn yes_no_and_case_insensitive_bools() {
    let f = func("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return `t` } return `f`  }");
    assert_eq!(
        run_plain(&f, &[("a", "YES")]),
        Ok(Outcome::Text("t".into()))
    );
    assert_eq!(run_plain(&f, &[("a", "No")]), Ok(Outcome::Text("f".into())));
    assert_eq!(
        run_plain(&f, &[("a", "True")]),
        Ok(Outcome::Text("t".into()))
    );
}

#[test]
fn std_template_call_expands_to_text() {
    let f = func(
        r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  if (c === "x") {
    return delegate(model=`use opus model`, task=`code review ${c}`);
  } return delegate(model=`use haiku model`, task=`code review the pr`); 
}"#,
    );
    let bindings = BTreeMap::from([std_binding("delegate")]);
    assert_eq!(
        run(&f, &inputs(&[("c", "x")]), &bindings),
        Ok(Outcome::Text(
            "Delegate this task to a subagent.\nModel: use opus model\nTask: code review x".into()
        ))
    );
    assert_eq!(
        run(&f, &inputs(&[("c", "y")]), &bindings),
        Ok(Outcome::Text(
            "Delegate this task to a subagent.\nModel: use haiku model\nTask: code review the pr"
                .into()
        ))
    );
}

#[test]
fn user_template_call_expands_its_own_params() {
    let tpl = func(
        r#"/** Pick. */
export default function tpl(
  /** First. */ a: string,
  /** Second. */ b: string,
) {
  return `${a} then ${b}`;
}"#,
    );
    let f = func(
        r#"/** d */
export default function f() {
  return @./tpl(a=`one`, b=`two`);
}"#,
    );
    let bindings = BTreeMap::from([(
        "./tpl".to_string(),
        Binding::Template {
            path: Some("tpl".into()),
            function: Box::leak(Box::new(tpl)),
        },
    )]);
    assert_eq!(
        run(&f, &inputs(&[]), &bindings),
        Ok(Outcome::Text("one then two".into()))
    );
}

#[test]
fn template_literal_return_expands_the_union_value() {
    let f = func("/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) {\n  return `Pick ${c}`;\n}");
    assert_eq!(
        run_plain(&f, &[("c", "y")]),
        Ok(Outcome::Text("Pick y".into()))
    );
    assert_eq!(run_plain(&f, &[]), Ok(Outcome::NeedInput("c".into())));
}

#[test]
fn running_a_template_is_an_error() {
    let f =
        func("/** d */\nexport default function t(\n  /** A */ a: string,\n) { return `${a}` }");
    assert_eq!(
        run_plain(&f, &[]),
        Err("templates are called from functions, not run".into())
    );
}

#[test]
fn request_without_an_answer_stops_for_it() {
    let f = func(
        r#"/** d */
export default async function f() {
  /** Did it work? */
  const ok: boolean = await request(`Do the thing.`);
  if (ok) { return `y` }
  return `n`
}"#,
    );
    assert_eq!(
        run_plain(&f, &[]),
        Ok(Outcome::Request {
            instruction: "Do the thing.".into(),
            response: "ok".into()
        })
    );
    assert_eq!(
        run_plain(&f, &[("ok", "yes")]),
        Ok(Outcome::Text("y".into()))
    );
    assert_eq!(
        run_plain(&f, &[("ok", "no")]),
        Ok(Outcome::Text("n".into()))
    );
}

#[test]
fn request_instruction_can_be_a_std_template_call() {
    let f = func(
        r#"/** d */
export default async function f() {
  /** Did it work? */
  const ok: boolean = await request(delegate(model=`use opus model`, task=`code review the pr`));
  return `y`
}"#,
    );
    let bindings = BTreeMap::from([std_binding("delegate")]);
    assert_eq!(
        run(&f, &inputs(&[]), &bindings),
        Ok(Outcome::Request {
            instruction:
                "Delegate this task to a subagent.\nModel: use opus model\nTask: code review the pr"
                    .into(),
            response: "ok".into(),
        })
    );
}

#[test]
fn request_instruction_that_interpolates_a_union_needs_it_first() {
    let f = func(
        r#"/** d */
export default async function f(
  /** C */ c: "x" | "y",
) {
  /** Did it work? */
  const ok: boolean = await request(`fix ${c}`);
  return `y`
}"#,
    );
    assert_eq!(run_plain(&f, &[]), Ok(Outcome::NeedInput("c".into())));
    assert_eq!(
        run_plain(&f, &[("c", "x")]),
        Ok(Outcome::Request {
            instruction: "fix x".into(),
            response: "ok".into()
        })
    );
}

#[test]
fn request_answer_is_validated_when_used() {
    let f = func(
        r#"/** d */
export default async function f() {
  /** Did it work? */
  const ok: boolean = await request(`Do it.`);
  return `y`
}"#,
    );
    let err = run_plain(&f, &[("ok", "maybe")]).unwrap_err();
    assert!(err.contains("invalid value `maybe` for `ok`"), "{err}");
}

#[test]
fn answer_is_a_parameter_for_the_rest_of_its_block() {
    let f = func(
        r#"/** d */
export default async function f(
  /** C */ c: "x" | "y",
) {
  if (c === "x") {
    /** Did it work? */
    const ok: boolean = await request(`fix it`);
    if (ok) { return `fixed` }
    return `not fixed`
  }
  return `other`
}"#,
    );
    assert_eq!(
        run_plain(&f, &[("c", "y")]),
        Ok(Outcome::Text("other".into()))
    );
    assert_eq!(
        run_plain(&f, &[("c", "x")]),
        Ok(Outcome::Request {
            instruction: "fix it".into(),
            response: "ok".into()
        })
    );
    assert_eq!(
        run_plain(&f, &[("c", "x"), ("ok", "no")]),
        Ok(Outcome::Text("not fixed".into()))
    );
}

#[test]
fn hand_off_returns_the_string_arguments() {
    let f = func(
        r#"/** d */
export default function f(
  /** Link */ pr: string,
) {
  return @./merge(pr=pr);
}"#,
    );
    let bindings = BTreeMap::from([("./merge".to_string(), Binding::Function("merge".into()))]);
    assert_eq!(
        run(
            &f,
            &inputs(&[("pr", "https://example.com/pr/1")]),
            &bindings
        ),
        Ok(Outcome::Ref {
            target: "merge".into(),
            args: vec![("pr".into(), "https://example.com/pr/1".into())]
        })
    );
}

#[test]
fn string_input_is_interpolated_in_a_template_literal() {
    // A union makes this a normal function, so `pr` is an input and not a template parameter.
    let f = func("/** d */\nexport default function f(\n  /** Link */ pr: string,\n  /** C */ c: \"x\" | \"y\",\n) {\n  if (c === \"x\") { return `Review ${pr}` }\n  return `Other ${pr}`;\n}");
    assert_eq!(
        run_plain(&f, &[("c", "x"), ("pr", "L")]),
        Ok(Outcome::Text("Review L".into()))
    );
    assert_eq!(
        run_plain(&f, &[("c", "y"), ("pr", "L")]),
        Ok(Outcome::Text("Other L".into()))
    );
    assert_eq!(
        run_plain(&f, &[("c", "x")]),
        Ok(Outcome::NeedInput("pr".into()))
    );
}

#[test]
fn string_input_is_a_bare_argument_of_a_std_template() {
    let f = func("/** d */\nexport default function f(\n  /** Link */ pr: string,\n) {\n  return delegate(model=`use opus model`, task=pr);\n}");
    let bindings = BTreeMap::from([std_binding("delegate")]);
    assert_eq!(
        run(&f, &inputs(&[("pr", "L")]), &bindings),
        Ok(Outcome::Text(
            "Delegate this task to a subagent.\nModel: use opus model\nTask: L".into()
        ))
    );
}

#[test]
fn string_input_in_a_condition_is_an_error() {
    let f = func("/** d */\nexport default function f(\n  /** Link */ pr: string,\n) {\n  if (pr) { return `a` }\n  return `b`\n}");
    assert_eq!(
        run_plain(&f, &[("pr", "x")]),
        Err("`pr` is a string input; a condition cannot read it".into())
    );
}
