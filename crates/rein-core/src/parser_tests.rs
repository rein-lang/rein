use super::*;

pub(crate) const SAMPLE: &str = r#"
// merge_pr.rein

interface Input {
  /** Does the PR change only documentation files? */
  docs_only: boolean;
  /**
   * What is the CI status of the PR?
   */
  ci: "green" | "red" | "pending";
}

/**
 * Decide whether a pull request can be merged.
 */
export default function mergePr({ docs_only, ci }: Input) {
  if (docs_only) {
    return `approved`;
  }
  switch (ci) {
    case "green":
      return @./review_code;
    case "red":
      return @./ci/fix;
    case "pending":
      return `wait for CI, then run mergePr again`;
  }
}
"#;

fn ok(src: &str) -> Function {
    let (f, diags) = parse(src);
    assert!(diags.is_empty(), "{diags:?}");
    f
}

fn one_error(src: &str) -> String {
    let (_, diags) = parse(src);
    assert_eq!(diags.len(), 1, "{diags:?}");
    diags[0].message.clone()
}

#[test]
fn a_template_as_a_tag_is_an_error_with_the_quoted_form() {
    for (src, want) in [
        ("/** F. */\nexport default function f(/** C? */ c: `green` | \"red\") {\n  return `x`;\n}\n", "a tag uses quotes: write \"green\" instead of `green`"),
        ("/** F. */\nexport default function f(/** C? */ c: \"green\" | \"red\") {\n  if (c === `red`) {\n    return `x`;\n  }\n  return `y`;\n}\n", "a tag uses quotes: write \"red\" instead of `red`"),
        ("/** F. */\nexport default function f(/** C? */ c: \"green\" | \"red\") {\n  switch (c) {\n    case `green`:\n      return `x`;\n    case \"red\":\n      return `y`;\n  }\n}\n", "a tag uses quotes: write \"green\" instead of `green`"),
        ("/** F. */\nexport default function f(/** C? */ c: `a ${b}` | \"red\") {\n  return `x`;\n}\n", "a tag uses quotes: write \"...\" instead of `...`"),
    ] {
        let (_, diags) = parse(src);
        assert_eq!(diags.len(), 1, "{src}: {diags:?}");
        assert_eq!(diags[0].message, want, "{src}");
    }
}

#[test]
fn parses_full_sample() {
    let f = ok(SAMPLE);
    assert_eq!(
        f.description.unwrap().value,
        "Decide whether a pull request can be merged."
    );
    assert_eq!(f.inputs.len(), 2);
    assert_eq!(f.inputs[0].ty, InputType::Bool);
    assert_eq!(
        f.inputs[0].description.value,
        "Does the PR change only documentation files?"
    );
    assert_eq!(
        f.inputs[1].description.value,
        "What is the CI status of the PR?"
    );
    assert_eq!(
        f.inputs[1].ty.options().unwrap(),
        ["green", "red", "pending"]
    );
    assert_eq!(f.body.len(), 2);
    assert!(matches!(&f.body[0], Stmt::If { .. }));
    let Stmt::Switch { cases, .. } = &f.body[1] else {
        panic!("expected switch")
    };
    assert_eq!(cases.len(), 3);
    assert!(
        matches!(&cases[1].body[0], Stmt::Return(r) if matches!(&r.value, ReturnValue::Ref(s) if s.value == "./ci/fix"))
    );
}

#[test]
fn parameter_doc_block_can_be_multi_line() {
    let f = ok("/** d */\nexport default function f(\n  /**\n   * Is it VIP?\n   * Check the CRM.\n   */\n  vip: boolean,\n) { return @./a }");
    assert_eq!(f.inputs[0].description.value, "Is it VIP? Check the CRM.");
}

#[test]
fn parameter_without_doc_has_empty_description() {
    let f = ok("/** d */\nexport default function f(a: boolean) { return \"x\" }");
    assert_eq!(f.inputs[0].description.value, "");
    assert!(f.inputs[0].doc.is_none());
    assert_eq!(f.inputs[0].description.span, f.inputs[0].name.span);
}

#[test]
fn grouped_case_labels_share_one_body() {
    let f = ok(
        "/** d */\nexport default function f(c: \"a\" | \"b\" | \"c\") {\n\
         switch (c) {\n  case \"a\":\n  case \"b\":\n    return \"ab\";\n  case \"c\": { return \"c\"; }\n}\n}",
    );
    let Stmt::Switch { cases, .. } = &f.body[0] else {
        panic!("expected switch")
    };
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].values.len(), 2);
    assert_eq!(cases[0].values[1].value, "b");
    assert_eq!(cases[1].values.len(), 1);
}

#[test]
fn single_quotes_leading_pipe_and_missing_semicolons() {
    let f = ok(
        "/** d */\nexport default function f(\n  /** x */\n  c: | 'x' | 'y',\n  /** y */\n  b: boolean,\n) {\n  if (c === 'x') { return @../a } return 'z' \n}",
    );
    assert_eq!(f.inputs[0].ty.options().unwrap(), ["x", "y"]);
    assert_eq!(f.inputs.len(), 2);
}

#[test]
fn parses_guard_chain_and_logic() {
    let src = "/** d */\nexport default function f(\n  /** a */ a: boolean,\n  /** b */ b: boolean,\n  /** c */ c: \"x\" | \"y\",\n) {\n\
         if (!(a && b) || c === \"x\") { return \"one\" }\n\
         if (a) { return \"two\" }\n\
         return \"three\"\n}";
    let f = ok(src);
    assert_eq!(f.inputs.len(), 3);
    let Stmt::If { cond, .. } = &f.body[0] else {
        panic!("expected if")
    };
    assert!(matches!(cond, Expr::Or(..)));
    assert!(matches!(&f.body[1], Stmt::If { .. }));
}

#[test]
fn else_is_not_in_rein() {
    let msg = one_error("/** d */\nexport default function f(\n  /** a */ a: boolean,\n) {\n  if (a) { return \"1\" }\n  else { return \"2\" }\n}");
    assert_eq!(
        msg,
        "Rein has no `else`: end the `if` block with a return, and handle the other case after it"
    );
}

#[test]
fn return_needs_a_value() {
    let msg = one_error("/** d */\nexport default function f() {\n  return;\n}");
    assert_eq!(
        msg,
        "`return` needs a value: a string, a template, or `@./function`"
    );
}

#[test]
fn return_without_value_is_one_error_with_partial_tree() {
    let (f, diags) = parse("/** x */\nexport default function f() {\n  return\n}");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("`return` needs a value"),
        "{}",
        diags[0].message
    );
    assert!(f.description.is_some());
}

#[test]
fn imports_are_not_in_rein() {
    let msg =
        one_error("import a from \"./a\";\n/** d */\nexport default function f() { return @./a }");
    assert_eq!(
        msg,
        "Rein has no imports; reference functions with `@./path` and use std functions directly"
    );
}

#[test]
fn bare_name_return_is_a_callback() {
    let (f, diags) = parse("/** d */\nexport default function f() { return then; }");
    assert!(diags.is_empty(), "{diags:?}");
    assert!(matches!(&f.body[0], Stmt::Return(r) if matches!(&r.value, ReturnValue::Callback(n) if n.value == "then")));
}

#[test]
fn callback_parameter_and_reference_argument_parse() {
    let src = "/** d */\nexport default function f(\n  /** Next. */\n  then: @,\n) {\n  return @./g(then=@./h);\n}";
    let (f, diags) = parse(src);
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(f.inputs[0].ty, InputType::Callback);
    let Stmt::Return(r) = &f.body[0] else { panic!() };
    let ReturnValue::Call(call) = &r.value else { panic!() };
    assert!(matches!(&call.args[0].value, Value::Ref(p) if p.value == "./h"));
}

#[test]
fn switch_default_is_not_in_rein() {
    let msg = one_error(
        "/** d */\nexport default function f(c: \"x\" | \"y\") {\n switch (c) { case \"x\": return \"a\"; default: return \"b\" }\n}",
    );
    assert_eq!(msg, "Rein has no `default`; add a `case` for every member");
}

#[test]
fn function_doc_must_be_right_before_export() {
    for src in [
        "/** d */\nexport default function f() { return \"x\" }\n/** y */",
        "export default function f(/** a */ a: boolean) { return \"x\" }\n/** y */",
    ] {
        let msg = one_error(src);
        assert!(
            msg.contains("the doc comment must go right before"),
            "{src}: {msg}"
        );
    }
}

#[test]
fn doc_in_a_wrong_place_inside_the_body_is_an_error() {
    let msg =
        one_error("/** d */\nexport default function f() {\n  /** note */\n  return \"x\"\n}");
    assert!(
        msg.contains("the doc comment must go right before"),
        "{msg}"
    );
}

#[test]
fn one_function_per_file() {
    let msg = one_error("/** d */\nexport default function f() { return \"x\" }\nexport default function g() { return \"y\" }");
    assert!(
        msg.contains("a file holds one `export default function`"),
        "{msg}"
    );
}

#[test]
fn loops_and_variables_have_helpful_errors() {
    let head = "/** d */\nexport default function f(a: boolean) {\n";
    assert_eq!(
        one_error(&format!("{head} for (;;) {{ }}\n}}")),
        "Rein has no loops yet"
    );
    assert_eq!(
        one_error(&format!("{head} while (a) {{ }}\n}}")),
        "Rein has no loops yet"
    );
    assert_eq!(
        one_error(&format!("{head} do {{ }}\n}}")),
        "Rein has no loops yet"
    );
    assert!(one_error(&format!("{head} const x = \"a\"\n}}")).starts_with("Rein has no variables"));
    assert_eq!(
        one_error(&format!("{head} let x = \"a\"\n}}")),
        "Rein has no variables"
    );
    assert_eq!(
        one_error(&format!("{head} var x = \"a\"\n}}")),
        "Rein has no variables"
    );
}

#[test]
fn true_and_false_in_conditions_have_a_helpful_error() {
    let head = "/** d */\nexport default function f(\n  /** a */ a: boolean,\n) {\n";
    assert_eq!(
        one_error(&format!("{head} if (true) {{ return \"x\" }}\n}}")),
        "use the boolean parameter directly"
    );
    assert_eq!(
        one_error(&format!("{head} if (a === true) {{ return \"x\" }}\n}}")),
        "use the boolean parameter directly"
    );
}

#[test]
fn old_syntax_is_a_syntax_error() {
    let (_, diags) = parse("input docs_only: bool\nreturn \"x\"");
    assert_eq!(diags.len(), 1, "{diags:?}");
}

#[test]
fn lex_error_gives_one_error_and_no_tree() {
    let (f, diags) = parse("/** abc");
    assert_eq!(diags.len(), 1);
    assert!(f.description.is_none());
    assert!(f.doc.is_none());
}

#[test]
fn template_return_parses() {
    let f = ok(r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  return `Pick ${c}`;
}"#);
    let Stmt::Return(r) = &f.body[0] else {
        panic!("expected return")
    };
    let ReturnValue::Template(lit) = &r.value else {
        panic!("expected template, got {:?}", r.value)
    };
    assert_eq!(lit.pieces.len(), 2);
}

#[test]
fn call_return_parses_with_literal_and_template_args() {
    let f = ok(r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  return @./tpl(a="a", b=`b ${c}`);
}"#);
    let Stmt::Return(r) = &f.body[0] else {
        panic!("expected return")
    };
    let ReturnValue::Call(call) = &r.value else {
        panic!("expected call, got {:?}", r.value)
    };
    assert_eq!(call.callee.key(), "./tpl");
    assert_eq!(call.args.len(), 2);
    assert_eq!(
        call.args[0].name.as_ref().map(|n| n.value.as_str()),
        Some("a")
    );
    assert!(matches!(&call.args[0].value, Value::Str(s) if s.value == "a"));
    assert_eq!(
        call.args[1].name.as_ref().map(|n| n.value.as_str()),
        Some("b")
    );
    assert!(matches!(&call.args[1].value, Value::Template(_)));
}

#[test]
fn positional_argument_parses_with_no_name() {
    let f = ok(r#"/** d */
export default function f() {
  return @./tpl("a");
}"#);
    let Stmt::Return(r) = &f.body[0] else {
        panic!("expected return")
    };
    let ReturnValue::Call(call) = &r.value else {
        panic!("expected call")
    };
    assert!(call.args[0].name.is_none());
    assert!(matches!(&call.args[0].value, Value::Str(s) if s.value == "a"));
}

#[test]
fn call_argument_can_be_a_template_call() {
    let f = ok(r#"/** d */
export default function f() {
  return @./tpl(a=@./inner(delegate(model="m", task="t")), b="b");
}"#);
    let Stmt::Return(r) = &f.body[0] else {
        panic!("expected return")
    };
    let ReturnValue::Call(call) = &r.value else {
        panic!("expected call, got {:?}", r.value)
    };
    assert_eq!(call.args.len(), 2);
    let Value::Call(inner) = &call.args[0].value else {
        panic!("expected nested call, got {:?}", call.args[0])
    };
    assert_eq!(inner.callee.key(), "./inner");
    assert!(matches!(&inner.args[0].value, Value::Call(c) if c.callee.key() == "delegate"));
    assert!(matches!(&call.args[1].value, Value::Str(s) if s.value == "b"));
}

#[test]
fn string_parameter_type_is_text() {
    let f = ok("/** d */\nexport default function f(\n  /** A */ a: string,\n) { return `${a}` }");
    assert_eq!(f.inputs[0].ty, InputType::Text);
}

#[test]
fn request_const_parses_with_its_doc_and_type() {
    let f = ok(r#"/** d */
export default async function f() {
  /** Did it work? */
  const ok: boolean = await request("Do it.");
  if (!ok) { return "n" }
  return "y"
}"#);
    let Stmt::Request {
        response,
        awaited: Awaited::Request { arg, callee },
        ..
    } = &f.body[0]
    else {
        panic!("expected request")
    };
    assert_eq!(response.name.value, "ok");
    assert_eq!(response.ty, InputType::Bool);
    assert_eq!(response.description.value, "Did it work?");
    assert_eq!(callee.value, "request");
    assert!(matches!(arg, Value::Str(s) if s.value == "Do it."));
    assert!(f.is_async);
}

#[test]
fn request_argument_can_be_a_std_template_call() {
    let f = ok(r#"/** d */
export default async function f() {
  /** Did it work? */
  const ok: boolean = await request(delegate(model="m", task="t"));
  return "y"
}"#);
    let Stmt::Request { awaited: Awaited::Request { arg, .. }, .. } = &f.body[0] else {
        panic!("expected request")
    };
    assert!(matches!(arg, Value::Call(c) if c.callee.key() == "delegate" && c.args.len() == 2));
}

#[test]
fn request_answer_needs_a_doc_comment() {
    let msg = one_error("/** d */\nexport default async function f() {\n  const ok: boolean = await request(\"x\");\n  return \"y\"\n}");
    assert!(msg.contains("Rein has no variables"), "{msg}");
}

#[test]
fn only_request_can_be_awaited() {
    let msg = one_error("/** d */\nexport default async function f() {\n  /** Q */\n  const x: boolean = await fetch(\"a\");\n  return \"y\"\n}");
    assert!(msg.contains("only `request` or a reference like `@program` can be awaited"), "{msg}");
}

#[test]
fn call_arguments_can_be_bare_names() {
    let f = ok(r#"/** d */
export default function f(
  /** Link */ pr: string,
) {
  return @./merge(a=pr, b="x", c=`t ${pr}`);
}"#);
    let Stmt::Return(r) = &f.body[0] else {
        panic!("expected return")
    };
    let ReturnValue::Call(call) = &r.value else {
        panic!("expected call")
    };
    assert_eq!(call.callee.key(), "./merge");
    assert!(matches!(&call.args[0].value, Value::Name(n) if n.value == "pr"));
    assert!(matches!(&call.args[1].value, Value::Str(_)));
    assert!(matches!(&call.args[2].value, Value::Template(_)));
}

#[test]
fn bare_name_can_be_a_std_call_argument() {
    let f = ok(r#"/** d */
export default function f(
  /** Link */ pr: string,
) {
  return delegate(model="use opus model", task=pr);
}"#);
    let Stmt::Return(r) = &f.body[0] else {
        panic!("expected return")
    };
    let ReturnValue::Call(call) = &r.value else {
        panic!("expected call")
    };
    assert!(matches!(&call.callee, Callee::Std(s) if s.value == "delegate"));
    assert!(matches!(&call.args[1].value, Value::Name(n) if n.value == "pr"));
}

#[test]
fn bare_name_is_not_a_request_value() {
    let msg = one_error("/** d */\nexport default async function f(\n  /** Link */ pr: string,\n) {\n  /** Ok? */\n  const ok: boolean = await request(pr);\n  return \"y\"\n}");
    assert!(
        msg.contains("expected a string or a template literal"),
        "{msg}"
    );
}

#[test]
fn struct_parameter_binds_the_inline_interface_fields() {
    let src = "interface Input {\n  /** A? */\n  a: boolean;\n  /** B? */\n  b: \"x\" | \"y\";\n}\n/** d */\nexport default function f({ b, a }: Input) {\n  if (a) { return \"1\" }\n  return `${b}`\n}";
    let f = ok(src);
    assert_eq!(f.params.len(), 1);
    let names: Vec<&str> = f.inputs.iter().map(|i| i.name.value.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(f.inputs[0].description.value, "A?");
    assert_eq!(f.inputs[0].ty, InputType::Bool);
    let span = f.inputs[0].name.span;
    assert_eq!(&src[span.start..span.end], "a");
    assert!(span.start > src.find("function").unwrap(), "{span:?}");
    assert_eq!(f.structs.len(), 1);
    assert!(matches!(&f.input_struct, Some(Ok(_))));
}

#[test]
fn a_struct_type_must_be_destructured() {
    assert_eq!(
        one_error("/** d */\nexport default function f(x: Input) { return \"y\" }"),
        "a struct parameter is destructured: write `{ field, ... }: Input`"
    );
    assert_eq!(
        one_error("/** d */\nexport default function f(x: @./ticket) { return \"y\" }"),
        "a struct parameter is destructured: write `{ field, ... }: @./ticket`"
    );
}

#[test]
fn a_struct_file_holds_its_exported_interface() {
    let f = ok("/** A ticket. */\nexport default interface Ticket {\n  /** Title. */\n  title: string\n}\n");
    assert!(f.name.is_none());
    let s = f.exported_struct().unwrap();
    assert_eq!(s.name.value, "Ticket");
    assert_eq!(s.fields.len(), 1);
    assert_eq!(s.fields[0].name.value, "title");
    assert_eq!(s.fields[0].ty, InputType::Text);
}

#[test]
fn a_struct_parameter_has_no_doc_comment() {
    let msg = one_error("/** d */\nexport default function f(/** no */ { a }: Input) { return \"y\" }\ninterface Input {\n  /** A */ a: boolean;\n}");
    assert!(
        msg.contains("a struct parameter has no doc comment"),
        "{msg}"
    );
}

#[test]
fn await_of_a_reference_is_a_call_with_return() {
    let f = ok(r#"/** d */
export default async function f() {
  /** The projects. */
  const p: boolean = await @priority-projects(workplace=`w`);
  /** Done? */
  const d: boolean = await @./other;
  return `y`
}"#);
    let Stmt::Request { response, awaited: Awaited::Call(ReturnValue::Call(call)), .. } = &f.body[0] else {
        panic!("expected a call with return")
    };
    assert_eq!(response.name.value, "p");
    assert_eq!(call.callee.key(), "priority-projects");
    assert_eq!(call.args.len(), 1);
    let Stmt::Request { awaited: Awaited::Call(ReturnValue::Ref(r)), .. } = &f.body[1] else {
        panic!("expected a reference")
    };
    assert_eq!(r.value, "./other");
}

#[test]
fn await_outside_a_const_answer_gives_the_fix() {
    for body in ["  await @./x;\n  return `y`", "  return await @./x;"] {
        let msg = one_error(&format!("/** d */\nexport default async function f() {{\n{body}\n}}"));
        assert!(msg.contains("const name: Type = await @program(...)"), "{body}: {msg}");
    }
}

#[test]
fn a_doc_comment_before_a_bare_await_gives_the_await_hint() {
    let msg = one_error("/** d */\nexport default async function f() {\n  /** Q? */\n  await @./x;\n  return `y`\n}");
    assert!(msg.contains("const name: Type = await @program(...)"), "{msg}");
}
