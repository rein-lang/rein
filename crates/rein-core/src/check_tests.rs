use super::*;
use crate::diagnostic::Severity;
use crate::parser::parse;

const TPL: &str = r#"interface Input {
  /** First. */
  a: string;
  /** Second. */
  b: string;
}
/** Pick. */
export default function tpl({ a, b }: Input) {
  return `${a} ${b}`;
}"#;

fn tpl() -> &'static Function {
    static F: std::sync::OnceLock<Function> = std::sync::OnceLock::new();
    F.get_or_init(|| parse(TPL).0)
}

#[test]
fn a_quoted_string_as_text_is_an_error() {
    // Each source has exactly one quoted string in a text position.
    for src in [
        "/** F. */\nexport default function f() {\n  return \"done\";\n}\n",
        "/** F. */\nexport default function f() {\n  return delegate(model=\"a\", task=`b`);\n}\n",
        "/** F. */\nexport default async function f() {\n  /** Ok? */\n  const ok: boolean = await request(\"do it\");\n  if (ok) {\n    return `yes`;\n  }\n  return `no`;\n}\n",
        "/** F. */\nexport default async function f() {\n  /** Ok? */\n  const ok: boolean = await request(delegate(model=\"a\", task=`b`));\n  if (ok) {\n    return `yes`;\n  }\n  return `no`;\n}\n",
    ] {
        let (f, parse_errors) = crate::parser::parse(src);
        assert!(parse_errors.is_empty(), "{src}: {parse_errors:?}");
        let errors: Vec<_> = quoted_text(&f.body).into_iter().map(|d| (d.message.clone(), src[d.span.start..d.span.end].to_string())).collect();
        assert_eq!(errors.len(), 1, "{src}: {errors:?}");
        assert_eq!(errors[0].0, TEXT_QUOTES);
        assert!(errors[0].1.starts_with('"'), "{src}: {errors:?}");
    }
    let ok = "/** F. */\nexport default function f(/** C? */ c: \"a\" | 'b') {\n  if (c === \"a\") {\n    return `x`;\n  }\n  return delegate(model=`m`, task=`t`);\n}\n";
    let (f, _) = crate::parser::parse(ok);
    assert!(quoted_text(&f.body).is_empty());
}

/// Test resolver: `./tpl` is a template with parameters `a` and `b`, paths with `missing` do not resolve,
/// `./merge` is a function with one `string` parameter `pr`, `./iri` takes `link: Iri`, `./review` takes `pr: string`, `big: boolean` and
/// `ci: "green" | "red"`, `./typed` takes `ok: boolean`, and every other path is a function with none.
fn resolves(path: &str) -> Result<Kind, String> {
    if path == "./tpl" {
        return Ok(Kind::Template {
            params: tpl().inputs.iter().map(|i| i.name.value.clone()).collect(),
        });
    }
    if path.contains("missing") {
        return Err(format!("cannot find function `@{path}`"));
    }
    if path == "./merge" {
        return Ok(Kind::Function { params: vec![("pr".into(), InputType::Text)] });
    }
    if path == "./review" {
        let ci = ["green", "red"].map(|m| Spanned { value: m.to_string(), span: Span::default() });
        let params = vec![("pr".into(), InputType::Text), ("big".into(), InputType::Bool), ("ci".into(), InputType::Choice(ci.to_vec()))];
        return Ok(Kind::Function { params });
    }
    if path == "./typed" {
        return Ok(Kind::Function { params: vec![("ok".into(), InputType::Bool)] });
    }
    if path == "./iri" {
        return Ok(Kind::Function { params: vec![("link".into(), InputType::Format(crate::schema_types::FormatType::Builtin(crate::formats::Format::from_name("Iri").unwrap())))] });
    }
    Ok(Kind::Function { params: Vec::new() })
}

/// A caller of `./review`, which takes `pr`, `big` and `ci`. `args` are the call's arguments.
fn review_caller(args: &str) -> String {
    format!(
        "import {{ review }} from \"./review\";\n\n/** F. */\nexport default async function f(/** PR? */ pr: string) {{\n  /** Big? */\n  const big: boolean = await request(`Count the files.`);\n  /** CI? */\n  const ci: \"green\" | \"red\" = await request(`Read CI.`);\n  return review({args});\n}}\n"
    )
}

#[test]
fn a_call_must_pass_every_parameter_of_the_callee() {
    clean(&review_caller("pr=pr, big=big, ci=ci"));
    let missing = MISSING_TYPED_ARG;
    expect_diag(&review_caller("pr=pr"), missing, "missing argument `big` in call to `@./review`; write `big=value`");
    expect_diag(&review_caller("pr=pr"), missing, "missing argument `ci` in call to `@./review`; write `ci=value`");
    expect_diag(&review_caller("pr=pr, ci=ci"), missing, "missing argument `big` in call to `@./review`");
    expect_diag(&review_caller("big=big, ci=ci"), Severity::Error, "missing argument `pr` in call to `@./review`");
}

#[test]
fn a_bare_reference_must_pass_every_parameter() {
    let src = "import { review } from \"./review\";\n\n/** F. */\nexport default function f() {\n  return review;\n}\n";
    expect_diag(src, Severity::Error, "`@./review` takes 3 arguments, got 0; pass `pr`, `big` and `ci`");
    let typed_only = "import { typed } from \"./typed\";\n\n/** F. */\nexport default function f() {\n  return typed;\n}\n";
    expect_diag(typed_only, MISSING_TYPED_ARG, "`@./typed` takes 1 argument, got 0; pass `ok`");
    let awaited = "import { review } from \"./review\";\n\n/** F. */\nexport default async function f() {\n  /** Ok? */\n  const ok: boolean = await review;\n  if (ok) {\n    return `a`;\n  }\n  return `b`;\n}\n";
    expect_diag(awaited, Severity::Error, "`@./review` takes 3 arguments, got 0");
}

#[test]
fn a_typed_argument_is_a_name_of_the_same_type() {
    let msg = "`big` of `@./review` is `boolean`; pass a parameter or an answer";
    expect_diag(&review_caller("pr=pr, big=`yes`, ci=ci"), Severity::Error, msg);
    let msg = "`ci` is `\"green\" | \"red\"`, but `big` of `@./review` is `boolean`";
    expect_diag(&review_caller("pr=pr, big=ci, ci=ci"), Severity::Error, msg);
    expect_diag(&review_caller("pr=pr, big=big, ci=nope"), Severity::Error, "unknown parameter `nope`");
    let wider = review_caller("pr=pr, big=big, ci=ci").replace("\"red\" =", "\"red\" | \"pending\" =");
    let msg = "`ci` can be `pending`, which `ci` of `@./review` does not have; its members are `green`, `red`";
    expect_diag(&wider, Severity::Error, msg);
}

fn check_src(src: &str) -> Vec<Diagnostic> {
    let (f, parse_diags) = parse(src);
    assert!(parse_diags.is_empty(), "{parse_diags:?}");
    check_function(&f, "test", &resolves)
}

fn expect_diag(src: &str, severity: Severity, part: &str) {
    let diags = check_src(src);
    assert!(
        diags
            .iter()
            .any(|d| d.severity == severity && d.message.contains(part)),
        "wanted {severity:?} containing `{part}`, got {diags:?}"
    );
}

fn clean(src: &str) {
    let diags = check_src(src);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn clean_sample_has_no_diagnostics() {
    let sample = crate::parser::tests::SAMPLE;
    assert!(check_src(sample).is_empty(), "{:?}", check_src(sample));
}

#[test]
fn missing_doc_comment() {
    expect_diag(
        "export default function f() { return `x` }",
        Severity::Error,
        "missing doc comment on `test`; put /** description */ right before `export default function`",
    );
    // A plain line comment is not a doc comment.
    expect_diag(
        "// Says x.\nexport default function f() { return `x` }",
        Severity::Error,
        "missing doc comment on `test`",
    );
}

#[test]
fn empty_description() {
    expect_diag(
        "/**/\nexport default function f() { return `x` }",
        Severity::Error,
        "empty description",
    );
    expect_diag(
        "/**   */\nexport default function f() { return `x` }",
        Severity::Error,
        "empty description on `test`",
    );
    // Tags alone leave the description empty.
    expect_diag(
        "/** @since 1 */\nexport default function f() { return `x` }",
        Severity::Error,
        "empty description on `test`",
    );
}

#[test]
fn documented_function_is_clean() {
    clean("/** Say x. */\nexport default function f() { return `x` }");
}

#[test]
fn param_without_doc_comment() {
    expect_diag(
        "/** d */\nexport default function f(a: boolean) { if (a) { return `1` } return `2`  }",
        Severity::Error,
        "parameter `a` needs a doc comment: /** question */ a: ...",
    );
}

#[test]
fn empty_param_doc_comment_counts_as_missing() {
    expect_diag(
        "/** d */\nexport default function f(\n  /**  */\n  a: boolean,\n) { if (a) { return `1` } return `2`  }",
        Severity::Error,
        "needs a doc comment",
    );
}

#[test]
fn param_tag_anywhere_is_an_error() {
    expect_diag(
        "/** d\n * @param a A?\n */\nexport default function f(\n  /** A? */\n  a: boolean,\n) { if (a) { return `1` } return `2`  }",
        Severity::Error,
        "describe each parameter in its own doc comment, right before it",
    );
    expect_diag(
        "/** d */\nexport default function f(\n  /** a\n   * @param a A\n   */\n  a: boolean,\n) { if (a) { return `1` } return `2`  }",
        Severity::Error,
        "describe each parameter in its own doc comment, right before it",
    );
}

#[test]
fn multi_line_param_doc_is_clean() {
    clean(
        "/** d */\nexport default function f(\n  /**\n   * Is the customer a VIP?\n   * Check the CRM.\n   */\n  vip: boolean,\n) { if (vip) { return `1` } return `2`  }",
    );
}

#[test]
fn unknown_tag_is_a_warning() {
    expect_diag(
        "/** d\n * @returns Something.\n */\nexport default function f() { return `x` }",
        Severity::Warning,
        "unknown tag `@returns`",
    );
}

#[test]
fn duplicate_parameter() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** A */ a: boolean,\n  /** A */ a: boolean,\n) { if (a) { return `1` } return `2`  }",
        Severity::Error,
        "duplicate parameter `a`",
    );
}

#[test]
fn param_without_name_is_a_syntax_error() {
    let (_, diags) = parse("/** d */\nexport default function f(: boolean) { return `x` }");
    assert_eq!(diags.len(), 1, "{diags:?}");
}

#[test]
fn union_with_too_few_members() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"only\",\n) { if (c === \"only\") { return `1` } return `2`  }",
        Severity::Error,
        "needs at least 2 members",
    );
}

#[test]
fn duplicate_member() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"a\" | \"a\",\n) { if (c === \"a\") { return `1` } return `2`  }",
        Severity::Error,
        "duplicate member `a` in union parameter `c`",
    );
}

#[test]
fn empty_member() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"a\" | \"\",\n) { if (c === \"a\") { return `1` } return `2`  }",
        Severity::Error,
        "empty member",
    );
}

#[test]
fn unknown_parameter_in_condition() {
    expect_diag(
        "/** d */\nexport default function f() { if (nope) { return `a` } return `b`  }",
        Severity::Error,
        "unknown parameter `nope`",
    );
}

#[test]
fn unknown_parameter_in_switch_subject() {
    expect_diag(
        "/** d */\nexport default function f() { switch (nope) { case \"a\": return `a` } }",
        Severity::Error,
        "unknown parameter `nope`",
    );
}

#[test]
fn boolean_parameter_compared() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a === \"x\") { return `1` } return `2`  }",
        Severity::Error,
        "boolean parameter `a` cannot be compared; use `if (a)` or `if (!a)`",
    );
}

#[test]
fn double_equals_on_boolean_parameter_is_still_an_error() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a == \"x\") { return \"1\" } return \"2\"  }",
        Severity::Error,
        "boolean parameter `a` cannot be compared; use `if (a)` or `if (!a)`",
    );
}

#[test]
fn double_equals_checks_its_union_member() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c != \"z\") { return \"1\" } return \"2\"  }",
        Severity::Error,
        "`z` is not a member of `c`",
    );
}

#[test]
fn double_not_equal_uses_the_tag_quote_rule() {
    let (_, diags) = parse("/** d */\nexport default function f(\n  /** C */ ci: \"green\" | \"red\",\n) { if (ci != `red`) { return `1` } return `2`  }");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].message, "a tag uses quotes: write \"red\" instead of `red`");
}

#[test]
fn boolean_parameter_as_switch_subject() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { switch (a) { case \"x\": return `1` } }",
        Severity::Error,
        "boolean parameter `a` cannot be used in a switch",
    );
}

#[test]
fn union_parameter_used_bare() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c) { return `1` } return `2`  }",
        Severity::Error,
        "union parameter `c` needs a comparison, like `c === \"...\"`",
    );
}

#[test]
fn compared_value_not_a_member() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c === \"z\") { return `1` } return `2`  }",
        Severity::Error,
        "`z` is not a member of `c`; members are `x`, `y`",
    );
}

#[test]
fn case_value_not_a_member() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"z\": return `1`; case \"x\": return `2` } }",
        Severity::Error,
        "`z` is not a member of `c`",
    );
}

#[test]
fn duplicate_case_value_across_groups() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return `1`; case \"x\": return `2`; case \"y\": return `3` } }",
        Severity::Error,
        "duplicate case `x`",
    );
}

#[test]
fn case_with_statements_must_end_in_return() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": if (c === \"x\") { return `1` } case \"y\": return `2` } }",
        Severity::Error,
        "case falls through; end it with return",
    );
}

#[test]
fn grouped_labels_without_return_fall_through() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\" | \"z\",\n) { switch (c) { case \"x\": return `1`; case \"y\": case \"z\": } }",
        Severity::Error,
        "case falls through; end it with return",
    );
}

#[test]
fn grouped_cases_share_one_return() {
    clean(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\" | \"z\",\n) { switch (c) { case \"x\": case \"y\": return `1`; case \"z\": return `2` } }",
    );
}

#[test]
fn not_all_paths_return() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return `1` } }",
        Severity::Error,
        "every path must return something",
    );
}

#[test]
fn if_with_else_terminates_when_both_branches_return() {
    clean("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return `1` } return `2`  }");
}

#[test]
fn if_block_that_falls_through_is_an_error() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { if (a) { return `2` } } return `1` }",
        Severity::Error,
        "every `if` block must end in a return",
    );
}

#[test]
fn guard_chain_is_clean() {
    clean("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return `1` } return `2` }");
}

#[test]
fn switch_covering_all_members_terminates() {
    clean(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return `1`; case \"y\": return `2` } }",
    );
}

#[test]
fn switch_missing_a_member_is_an_error() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return `1` } }",
        Severity::Error,
        "switch on `c` needs a case for every member; missing: `y`",
    );
}

#[test]
fn switch_missing_several_members_lists_them_all() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\" | \"z\",\n) { switch (c) { case \"x\": return `1` } }",
        Severity::Error,
        "missing: `y`, `z`",
    );
}

#[test]
fn code_after_return_in_a_guard_chain_is_an_error() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return `1` } return `2`  return `3` }",
        Severity::Error,
        "unreachable statement",
    );
}

#[test]
fn code_after_switch_is_an_error() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return `1`; case \"y\": return `2` } return `3` }",
        Severity::Error,
        "unreachable statement",
    );
}

#[test]
fn reference_that_does_not_resolve_is_an_error_on_the_reference() {
    expect_diag(
        "import { missing } from \"./missing\";\n\n/** d */\nexport default function f() { return missing; }",
        Severity::Error,
        "cannot find function `@./missing`",
    );
}

#[test]
fn unused_parameter() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { return `x` }",
        Severity::Warning,
        "unused parameter `a`",
    );
}

#[test]
fn unreachable_statement() {
    expect_diag(
        "/** d */\nexport default function f() { return `a`; return `b` }",
        Severity::Error,
        "unreachable statement",
    );
}

#[test]
fn invalid_file_name() {
    let (f, _) = parse("/** d */\nexport default function f() { return `x` }");
    let diags = check_function(&f, "bad-name", &resolves);
    assert!(
        diags
            .iter()
            .any(|d| d.severity == Severity::Error
                && d.message.contains("invalid function file name")),
        "{diags:?}"
    );
}

#[test]
fn keyword_or_reserved_word_in_file_name() {
    let (f, _) = parse("/** d */\nexport default function f() { return `x` }");
    for name in ["if", "ci/return", "for", "ci/const"] {
        let diags = check_function(&f, name, &resolves);
        assert!(
            diags.iter().any(|d| d.severity == Severity::Error
                && d.message.contains("invalid function file name")),
            "{name}: {diags:?}"
        );
    }
}

const TPL_IMPORT: &str = "";
const STD_IMPORT: &str = "";

fn with(prefix: &str, body: &str) -> String {
    format!("{prefix}{body}")
}

#[test]
fn template_call_with_literal_and_union_args_is_clean() {
    clean(&with(
        TPL_IMPORT,
        r#"import { tpl } from "./tpl";

/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  if (c === "x") {
    return tpl(a=`one`, b=`x ${c}`);
  }
  return tpl(b=`y ${c}`, a=`two`);
}"#,
    ));
}

#[test]
fn template_literal_return_is_clean() {
    clean(
        r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  return `Pick ${c}`;
}"#,
    );
}

#[test]
fn template_call_missing_argument_is_checked() {
    expect_diag(
        &with(
            TPL_IMPORT,
            r#"import { tpl } from "./tpl";

/** d */
export default function f() { return tpl(a=`only one`) }"#,
        ),
        Severity::Error,
        "missing argument `b` in call to `@./tpl`; write `b=value`",
    );
}

#[test]
fn hand_off_cannot_be_a_request_argument() {
    // A function is handed off by `return`. Inside `request(...)`, a call argument cannot be a hand-off.
    expect_diag(
        r#"import { other } from "./other";

/** d */
export default async function f() {
  /** Ok? */
  const ok: boolean = await request(other("a"));
  return `y`
}"#,
        Severity::Error,
        "A hand-off cannot be an argument of a call.",
    );
}

#[test]
fn nested_template_call_arity_is_checked() {
    expect_diag(
        &with_pr(r#"return @./tpl(a=pr, b=@./tpl(a=`only one`));"#),
        Severity::Error,
        "missing argument `b` in call to `@./tpl`; write `b=value`",
    );
    expect_diag(
        &with_pr(r#"return @./tpl(a=delegate(model=`m`), b=pr);"#),
        Severity::Error,
        "missing argument `task` in call to `delegate`; write `task=value`",
    );
}

#[test]
fn hand_off_cannot_be_an_argument() {
    expect_diag(
        &with_pr("return @./tpl(a=pr, b=@./merge(pr=pr));"),
        Severity::Error,
        "A hand-off cannot be an argument of a call.",
    );
    expect_diag(
        &with_pr("return delegate(model=@./merge(pr=pr), task=`task`);"),
        Severity::Error,
        "A hand-off cannot be an argument of a call.",
    );
}

#[test]
fn std_template_call_inside_a_user_template_call_is_clean() {
    clean(&with_pr(
        r#"return @./tpl(a=delegate(model=`use opus model`, task=pr), b=`b`);"#,
    ));
}

#[test]
fn call_of_a_function_without_strings_takes_no_arguments() {
    expect_diag(
        &with(
            "",
            r#"import { other } from "./other";

/** d */
export default function f() { return other("a") }"#,
        ),
        Severity::Error,
        "`@./other` takes no arguments; return it without a call",
    );
}

#[test]
fn calling_an_unimported_name_is_an_error() {
    expect_diag(
        r#"/** d */
export default function f() { return nope("a") }"#,
        Severity::Error,
        "`nope` is not in std",
    );
}

#[test]
fn template_returned_without_a_call_is_an_error() {
    expect_diag(
        &with(
            TPL_IMPORT,
            r#"import { tpl } from "./tpl";

/** d */
export default function f() { return tpl }"#,
        ),
        Severity::Error,
        "`@./tpl` is a template; import it, then call it with arguments, like `tpl(...)`",
    );
}

#[test]
fn boolean_parameter_cannot_be_interpolated() {
    expect_diag(
        r#"/** d */
export default function f(
  /** A */ a: boolean,
) { if (a) { return `x ${a}` } return `y`  }"#,
        Severity::Error,
        "boolean parameter `a` cannot be interpolated",
    );
}

#[test]
fn unknown_name_cannot_be_interpolated() {
    expect_diag(
        r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) { return `x ${zz} ${c}` }"#,
        Severity::Error,
        "unknown parameter `zz`",
    );
}

#[test]
fn boolean_cannot_be_a_call_argument() {
    expect_diag(
        &with(
            TPL_IMPORT,
            r#"import { tpl } from "./tpl";

/** d */
export default function f(
  /** A */ a: boolean,
) { if (a) { return tpl(a=`x`, b=`${a}`) } return `y`  }"#,
        ),
        Severity::Error,
        "boolean parameter `a` cannot be interpolated",
    );
}

#[test]
fn template_file_with_string_params_is_clean() {
    clean(
        r#"/** Say hi. */
export default function hi(
  /** Who? */ who: string,
) {
  return `Hi ${who}`;
}"#,
    );
}

#[test]
fn string_body_that_is_not_one_template_return_is_a_normal_function() {
    // Not a template, so `a` is an input. Here it is never used, which is a warning.
    expect_diag(
        r#"/** d */
export default function t(
  /** A */ a: string,
) { return "x" }"#,
        Severity::Warning,
        "unused parameter `a`",
    );
}

#[test]
fn template_cannot_have_an_if_a_string_input_cannot_be_a_condition() {
    // Not a template, because of the `if`. A `string` input cannot decide a branch.
    expect_diag(
        r#"/** d */
export default function t(
  /** A */ a: string,
) { if (a) { return `${a}` } return `b`  }"#,
        Severity::Error,
        "`a` is a string input; a condition cannot read it",
    );
}

#[test]
fn mixed_parameters_make_a_normal_function_not_a_template() {
    // A union is not a string, so this is not a template. The unused union is a warning, not a template error.
    expect_diag(
        r#"/** d */
export default function t(
  /** A */ a: string,
  /** B */ b: "x" | "y",
) { return `${a}` }"#,
        Severity::Warning,
        "unused parameter `b`",
    );
}

#[test]
fn template_must_use_every_parameter() {
    expect_diag(
        r#"/** d */
export default function t(
  /** A */ a: string,
  /** B */ b: string,
) { return `${a}` }"#,
        Severity::Error,
        "template parameter `b` is not used",
    );
}

#[test]
fn std_template_call_is_clean() {
    clean(&with(
        STD_IMPORT,
        r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  if (c === "x") {
    return delegate(model=`use opus model`, task=`task ${c}`);
  } return delegate(task=`code review`, model=`use haiku model`); 
}"#,
    ));
}

#[test]
fn await_needs_an_async_function() {
    expect_diag(
        r#"/** d */
export default function f() {
  /** Ok? */
  const ok: boolean = await request(`x`);
  if (ok) { return `y` }
  return `n`
}"#,
        Severity::Error,
        "`await` needs `export default async function`",
    );
}

#[test]
fn async_without_await_is_a_warning() {
    expect_diag(
        r#"/** d */
export default async function f() { return `x` }"#,
        Severity::Warning,
        "`async` function has no `await`; drop `async`",
    );
}

#[test]
fn answer_used_as_a_guard_is_clean() {
    clean(
        r#"import { tpl } from "./tpl";

/** d */
export default async function f(
  /** C */ c: "x" | "y",
) {
  /** Ok? */
  const ok: boolean = await request(`pick ${c}`);
  if (!ok) { return tpl(a=`a`, b=`b`) }
  return `y`
}"#,
    );
}

#[test]
fn answer_clashes_with_a_parameter() {
    expect_diag(
        r#"/** d */
export default async function f(
  /** C */ c: "x" | "y",
) {
  /** Ok? */
  const c: boolean = await request(`x`);
  if (c) { return `y` }
  return `n`
}"#,
        Severity::Error,
        "duplicate name `c`",
    );
}

#[test]
fn answer_used_before_it_is_declared() {
    expect_diag(
        r#"/** d */
export default async function f() {
  /** First */
  const first: "x" | "y" = await request(`${second}`);
  /** Second */
  const second: "x" | "y" = await request(`q`);
  return `b`
}"#,
        Severity::Error,
        "`second` is used before it is declared",
    );
}

const PICK: &str = "interface Pick {\n  /** The link of the pull request. */\n  pr: string;\n  /** Which task runs. */\n  task: \"review\" | \"merge\";\n  /** Is it urgent? */\n  urgent: boolean;\n}\n/** d */\nexport default async function f() {\n  /** The pull request of the next notification. */\n  const pick: Pick = await request(`pick a PR`);\n";

#[test]
fn struct_answer_is_used_in_a_hand_off_a_template_a_switch_and_a_condition() {
    clean(&format!("{PICK}  return @./merge(pr=pick.pr);\n}}"));
    clean(&format!("{PICK}  return @./tpl(a=pick.pr, b=`x`);\n}}"));
    clean(&format!("{PICK}  return `Handle ${{pick.pr}}.`;\n}}"));
    clean(&format!("{PICK}  if (pick.urgent) {{\n    return `now`;\n  }}\n  return `later`;\n}}"));
    clean(&format!("{PICK}  switch (pick.task) {{\n    case \"review\":\n      return @./merge(pr=pick.pr);\n    case \"merge\":\n      return `merge it`;\n  }}\n}}"));
}

#[test]
fn bare_string_answer_is_an_error() {
    expect_diag(
        "import { merge } from \"./merge\";\n\n/** d */\nexport default async function f() {\n  /** The link. */\n  const pr: string = await request(`pick a PR`);\n  return merge(pr=pr);\n}",
        Severity::Error,
        "a `string` answer is not allowed; answer a struct with a `string` field",
    );
}

#[test]
fn format_answer_is_an_error_with_its_type() {
    let answer = |ty: &str| {
        format!("/** d */\nexport default async function f() {{\n  /** The link. */\n  const pr: {ty} = await request(`pick a PR`);\n  return `x`;\n}}")
    };
    expect_diag(&answer("Iri"), Severity::Error, "a value of type `Iri` is not allowed as an answer; answer a struct with a field of type `Iri`");
    expect_diag(&answer("Email"), Severity::Error, "a value of type `Email` is not allowed as an answer");
    expect_diag(&answer("Uri"), Severity::Error, "a value of type `Uri` is not allowed as an answer");
}

#[test]
fn unknown_field_of_a_struct_answer_is_an_error() {
    let msg = "`z` is not a field of `Pick`; fields are pr, task, urgent";
    expect_diag(&format!("{PICK}  return @./merge(pr=pick.z);\n}}"), Severity::Error, msg);
    expect_diag(&format!("{PICK}  if (pick.z) {{\n    return `a`;\n  }}\n  return `b`;\n}}"), Severity::Error, msg);
    expect_diag(&format!("{PICK}  return `${{pick.z}}`;\n}}"), Severity::Error, msg);
}

#[test]
fn string_field_cannot_decide_a_branch() {
    expect_diag(
        &format!("{PICK}  if (pick.pr) {{\n    return `a`;\n  }}\n  return `b`;\n}}"),
        Severity::Error,
        "`pick.pr` is a string input; a condition cannot read it",
    );
}

#[test]
fn struct_answer_itself_is_not_a_value() {
    expect_diag(
        &format!("{PICK}  if (pick) {{\n    return `a`;\n  }}\n  return `b`;\n}}"),
        Severity::Error,
        "`pick` is a struct answer; read one of its fields, like `pick.field`",
    );
}

#[test]
fn struct_answer_must_name_a_struct() {
    expect_diag(
        "/** d */\nexport default async function f() {\n  /** The pick. */\n  const pick: Missing = await request(`pick`);\n  return `x`;\n}",
        Severity::Error,
        "cannot find struct `Missing`",
    );
}

#[test]
fn union_answer_needs_every_case() {
    expect_diag(
        r#"/** d */
export default async function f() {
  /** Pick */
  const c: "x" | "y" = await request(`pick`);
  switch (c) {
    case "x":
      return `1`;
  }
}"#,
        Severity::Error,
        "switch on `c` needs a case for every member; missing: `y`",
    );
}

#[test]
fn empty_function_must_return_something() {
    expect_diag(
        "/** d */\nexport default function f() {\n}",
        Severity::Error,
        "function must return something",
    );
}

#[test]
fn body_that_can_end_without_return_says_every_path() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return `1` } }",
        Severity::Error,
        "every path must return something",
    );
}

/// A normal function with one `string` input `pr`, whose body is `ret`.
fn with_pr(ret: &str) -> String {
    format!("/** d */\nexport default function f(\n  /** Link */ pr: string,\n) {{\n  {ret}\n}}")
}

#[test]
fn string_parameter_makes_a_normal_function_when_mixed_with_a_union() {
    // Not all parameters are `string`, so this is not a template. `pr` is an input, and no error is given.
    clean(
        "interface Input {\n  /** Link */\n  pr: string;\n  /** C */\n  c: \"x\" | \"y\";\n}\n/** d */\nexport default function f({ pr, c }: Input) {\n  if (c === \"x\") { return `Pick ${pr}` }\n  return `Other ${pr} ${c}`;\n}",
    );
}

#[test]
fn string_input_is_used_in_a_template_literal() {
    clean(&with_pr("return `Review ${pr}`;"));
}

#[test]
fn string_input_is_passed_to_a_std_template() {
    clean(&with_pr(
        "return delegate(model=`use opus model`, task=pr);",
    ));
}

#[test]
fn string_input_cannot_be_a_condition() {
    for cond in [
        "if (pr) { return `a` }",
        "if (pr === \"x\") { return `a` }",
        "if (!pr) { return `a` }",
    ] {
        let src = format!(
            "/** d */\nexport default function f(\n  /** Link */ pr: string,\n) {{\n  {cond}\n  return `b`\n}}"
        );
        expect_diag(
            &src,
            Severity::Error,
            "`pr` is a string input; a condition cannot read it",
        );
    }
}

#[test]
fn string_input_cannot_be_a_switch_subject() {
    expect_diag(
        &with_pr("switch (pr) { case \"x\": return `a`; case \"y\": return `b` }"),
        Severity::Error,
        "`pr` is a string input; a condition cannot read it",
    );
}

#[test]
fn format_input_cannot_be_a_condition_or_a_switch_subject() {
    let with_link = |ret: &str| {
        format!("/** d */\nexport default function f(\n  /** Link */ link: Iri,\n) {{\n  {ret}\n}}")
    };
    for cond in ["if (link) { return `a` }", "if (link === \"x\") { return `a` }", "if (!link) { return `a` }"] {
        expect_diag(&with_link(&format!("{cond}\n  return `b`")), Severity::Error, "`link` is an input of type `Iri`; a condition cannot read it");
    }
    expect_diag(
        &with_link("switch (link) { case \"x\": return `a`; case \"y\": return `b` }"),
        Severity::Error,
        "`link` is an input of type `Iri`; a condition cannot read it",
    );
}

#[test]
fn union_cannot_be_passed_to_a_format_parameter() {
    let src = "import { iri } from \"./iri\";\n\n/** F. */\nexport default function f(\n  /** Which? */\n  pick: \"a\" | \"b\",\n) {\n  return iri(link=pick);\n}\n";
    expect_diag(src, Severity::Error, "`pick` is not a string input");
}

#[test]
fn hand_off_passes_the_string_inputs_by_name() {
    // `./merge` takes one `string`, `pr`, so one named argument is right.
    clean(&with_pr("return @./merge(pr=pr);"));
}

#[test]
fn hand_off_without_arguments_is_an_error() {
    expect_diag(
        &with_pr("return @./merge;"),
        Severity::Error,
        "`@./merge` takes 1 argument, got 0",
    );
}

#[test]
fn named_arguments_bind_in_any_order() {
    clean(&with("", "import { tpl } from \"./tpl\";\n\ninterface Input {\n  /** A */\n  a: string;\n  /** B */\n  b: string;\n}\n/** d */\nexport default function f({ a, b }: Input) {\n  return tpl(b=a, a=b);\n}"));
    clean(&with("", "import { tpl } from \"./tpl\";\n\ninterface Input {\n  /** A */\n  a: string;\n  /** B */\n  b: string;\n}\n/** d */\nexport default function f({ a, b }: Input) {\n  return tpl(a=a, b=b);\n}"));
}

#[test]
fn positional_argument_is_an_error() {
    expect_diag(
        &with_pr("return @./merge(pr);"),
        Severity::Error,
        "missing argument `pr` in call to `@./merge`; write `pr=value`",
    );
    expect_diag(
        &with_pr("return delegate(\"use opus model\", pr);"),
        Severity::Error,
        "missing argument `model` in call to `delegate`; write `model=value`",
    );
}

#[test]
fn unknown_argument_name_is_an_error() {
    expect_diag(
        &with_pr("return @./merge(pr=pr, link=pr);"),
        Severity::Error,
        "`link` is not a parameter of `@./merge`; parameters are pr",
    );
    expect_diag(
        &with_pr("return delegate(model=`m`, tasks=pr);"),
        Severity::Error,
        "`tasks` is not a parameter of `delegate`; parameters are model, task",
    );
}

#[test]
fn repeated_argument_name_is_an_error() {
    expect_diag(
        &with_pr("return @./merge(pr=pr, pr=pr);"),
        Severity::Error,
        "`pr` is given twice in call to `@./merge`; give each name once",
    );
}

#[test]
fn missing_named_argument_is_an_error() {
    expect_diag(
        &with(
            TPL_IMPORT,
            r#"import { tpl } from "./tpl";

/** d */
export default function f() { return tpl(b=`only two`) }"#,
        ),
        Severity::Error,
        "missing argument `a` in call to `@./tpl`; write `a=value`",
    );
}

#[test]
fn hand_off_arguments_to_a_function_without_strings_are_an_error() {
    expect_diag(
        &with_pr("return @./other(pr);"),
        Severity::Error,
        "`@./other` takes no arguments; return it without a call",
    );
    // A bare reference needs no arguments, so this one is clean.
    clean("import { other } from \"./other\";\n\n/** d */\nexport default function f() {\n  return other;\n}");
}

#[test]
fn argument_that_is_not_a_string_input_is_an_error() {
    expect_diag(
        &with_pr("return @./merge(pr=c);"),
        Severity::Error,
        "`c` is not a string input",
    );
    expect_diag(
        &with_pr("return @./merge(pr=zz);"),
        Severity::Error,
        "`zz` is not a string input",
    );
    expect_diag(
        &with_pr("return delegate(model=`m`, task=c);"),
        Severity::Error,
        "`c` is not a string input",
    );
}

#[test]
fn template_call_accepts_a_string_input_as_an_argument() {
    // `./tpl` takes two `string` parameters.
    clean(&with_pr("return @./tpl(a=pr, b=`b`);"));
}

#[test]
fn string_input_not_used_is_a_warning() {
    expect_diag(
        &with_pr("return \"x\";"),
        Severity::Warning,
        "unused parameter `pr`",
    );
}

/// A function file with one inline struct `Input` (fields `a` and `c`) and the given body.
fn with_input_struct(body: &str) -> String {
    format!(
        "interface Input {{\n  /** A? */ a: boolean;\n  /** C? */ c: \"x\" | \"y\";\n}}\n/** d */\nexport default function f({body}) {{ if (a) {{ return `1` }} if (c === \"x\") {{ return `2` }} return `3` }}"
    )
}

/// Diagnostics for a `.struct.rein` file named `ticket.struct`.
fn check_struct_file(src: &str) -> Vec<Diagnostic> {
    let (f, parse_diags) = parse(src);
    assert!(parse_diags.is_empty(), "{parse_diags:?}");
    check_function(&f, "ticket.struct", &resolves)
}

#[test]
fn a_function_takes_one_parameter() {
    expect_diag(
        "/** d */\nexport default function f(\n  /** A */ a: boolean,\n  /** B */ b: boolean,\n) { if (a) { return `1` } if (b) { return `2` } return `3` }",
        Severity::Error,
        "a function takes one parameter; put `a` and `b` in a struct",
    );
    expect_diag(
        "/** d */\nexport default function f(\n  /** A */ a: boolean,\n  /** B */ b: boolean,\n  /** C */ c: boolean,\n) { if (a) { return `1` } if (b) { return `2` } if (c) { return `3` } return `4` }",
        Severity::Error,
        "a function takes one parameter; put `a`, `b` and `c` in a struct",
    );
}

#[test]
fn inline_struct_parameter_is_clean() {
    clean("interface Input {\n  /** A? */ a: boolean;\n  /** C? */ c: \"x\" | \"y\";\n}\n/** d */\nexport default function f({ a, c }: Input) {\n  if (a) { return `1` }\n  if (c === \"x\") { return `2` }\n  return `3`\n}");
}

#[test]
fn an_inline_struct_cannot_be_exported() {
    let src = "interface Input {\n  /** A? */ a: boolean;\n  /** C? */ c: \"x\" | \"y\";\n}\n/** d */\nexport default function f({ a, c }: Input) {\n  if (a) { return `1` }\n  if (c === \"x\") { return `2` }\n  return `3`\n}";
    let msg = "input struct `Input` cannot be exported; to share it, move it to its own `.struct.rein` file";
    expect_diag(
        &src.replacen("interface Input", "export interface Input", 1),
        Severity::Error,
        msg,
    );
    expect_diag(
        &src.replacen("interface Input", "export default interface Input", 1),
        Severity::Error,
        msg,
    );
}

#[test]
fn destructuring_names_every_field_once() {
    expect_diag(
        &with_input_struct("{ a, c, z }: Input"),
        Severity::Error,
        "`z` is not a field of `Input`; fields are a, c",
    );
    expect_diag(
        "interface Input {\n  /** A? */ a: boolean;\n  /** C? */ c: \"x\" | \"y\";\n}\n/** d */\nexport default function f({ a }: Input) { if (a) { return `1` } return `3` }",
        Severity::Error,
        "destructure every field of `Input`; missing: `c`",
    );
    expect_diag(
        &with_input_struct("{ a, a, c }: Input"),
        Severity::Error,
        "`a` is destructured twice",
    );
}

#[test]
fn an_inline_struct_must_exist() {
    expect_diag(
        "/** d */\nexport default function f({ a }: Nope) { return `x` }",
        Severity::Error,
        "cannot find struct `Nope`; declare `interface Nope { ... }` in this file",
    );
}

#[test]
fn an_unused_inline_struct_is_a_warning() {
    expect_diag(
        "interface Extra {\n  /** A */ a: boolean;\n}\n/** d */\nexport default function f() { return `x` }",
        Severity::Warning,
        "unused struct `Extra`; an inline struct is the input of the function in its file",
    );
}

#[test]
fn a_field_needs_a_doc_comment() {
    expect_diag(
        "interface Input {\n  a: boolean;\n}\n/** d */\nexport default function f({ a }: Input) { if (a) { return `1` } return `2` }",
        Severity::Error,
        "field `a` needs a doc comment: /** question */ a: ...",
    );
}

#[test]
fn struct_file_rules() {
    let ticket = "/** A ticket. */\nexport default interface Ticket {\n  /** Title. */\n  title: string;\n}\n";
    let diags = check_struct_file(ticket);
    assert!(diags.is_empty(), "{diags:?}");

    let with_fn = format!("{ticket}\n/** d */\nexport default function f() {{ return `x` }}");
    let diags = check_struct_file(&with_fn);
    assert!(
        diags.iter().any(|d| d.severity == Severity::Error
            && d.message.contains("a `.struct.rein` file holds one `export default interface` and nothing else; move the function to its own `.rein` file")),
        "{diags:?}"
    );

    let no_doc = "export default interface Ticket {\n  /** Title. */\n  title: string;\n}\n";
    let diags = check_struct_file(no_doc);
    assert!(
        diags.iter().any(|d| d.severity == Severity::Error
            && d.message == "missing doc comment on struct `Ticket`; put /** description */ right before `export default interface`"),
        "{diags:?}"
    );

    let with_other = format!("interface Other {{\n  /** B */ b: boolean;\n}}\n{ticket}");
    let diags = check_struct_file(&with_other);
    assert!(
        diags.iter().any(|d| d.severity == Severity::Error
            && d.message.contains(
                "a `.struct.rein` file holds one `export default interface` and nothing else"
            )),
        "{diags:?}"
    );
}

#[test]
fn all_string_body_that_is_a_hand_off_is_a_normal_function() {
    // All parameters are `string` but the body is not one template literal, so `b` is an input, not a template parameter.
    expect_diag(
        "import { merge } from \"./merge\";\n\n/** d */\nexport default function f(\n  /** A */ a: string,\n  /** B */ b: string,\n) {\n  return merge(pr=a);\n}",
        Severity::Warning,
        "unused parameter `b`",
    );
}

/// The blank-line warnings of one function source: each message, with the text it points at.
fn if_blank_warnings(src: &str) -> Vec<(String, String)> {
    let (f, parse_errors) = crate::parser::parse(src);
    assert!(parse_errors.is_empty(), "{src}: {parse_errors:?}");
    check_if_blank_lines(&f, src)
        .into_iter()
        .inspect(|d| assert_eq!(d.severity, Severity::Warning, "{src}"))
        .map(|d| (d.message.clone(), src[d.span.start..d.span.end].to_string()))
        .collect()
}

#[test]
fn an_if_with_blank_lines_around_it_has_no_warning() {
    let src = "/** F. */\nexport default function f(/** A? */ a: boolean) {\n  if (a) {\n    return `yes`;\n  }\n\n  if (!a) {\n    return `no`;\n  }\n\n  return `maybe`;\n}\n";
    assert!(if_blank_warnings(src).is_empty());
}

#[test]
fn an_if_after_a_statement_needs_an_empty_line_before_it() {
    let src = "/** F. */\nexport default function f(/** A? */ a: boolean, /** B? */ b: boolean) {\n  if (b) {\n    return `no`;\n  }\n\n  // Check the flag.\n  if (a) {\n    return `yes`;\n  }\n\n  return `maybe`;\n}\n";
    assert_eq!(
        if_blank_warnings(src),
        [("add an empty line before `if`".to_string(), "if".to_string())]
    );
}

#[test]
fn the_closing_brace_of_an_if_block_needs_an_empty_line_after_it() {
    let src = "/** F. */\nexport default function f(/** A? */ a: boolean) {\n  if (a) {\n    return `yes`;\n  }\n  // Fallback.\n  return `maybe`;\n}\n";
    assert_eq!(
        if_blank_warnings(src),
        [("add an empty line after the `if` block".to_string(), "}".to_string())]
    );
}

#[test]
fn the_first_statement_of_a_block_needs_no_empty_line_before_it() {
    // The function body, an `if` block and a `case` body each start with an `if`: no warning for it.
    let src = "/** F. */\nexport default function f(/** A? */ a: boolean, /** C? */ c: \"x\" | \"y\") {\n  if (a) {\n    if (a) {\n      return `inner`;\n    }\n\n    return `outer`;\n  }\n\n  switch (c) {\n    case \"x\":\n      if (a) {\n        return `x`;\n      }\n\n      return `x-other`;\n    case \"y\": return `y`;\n  }\n}\n";
    assert!(if_blank_warnings(src).is_empty(), "{:?}", if_blank_warnings(src));
}

#[test]
fn a_closing_brace_followed_by_a_closing_brace_needs_no_empty_line() {
    let src = "/** F. */\nexport default function f(/** A? */ a: boolean) {\n  if (a) {\n    if (a) {\n      return `x`;\n    }\n  }\n\n  return `y`;\n}\n";
    assert!(if_blank_warnings(src).is_empty(), "{:?}", if_blank_warnings(src));
}

#[test]
fn else_is_a_parse_error_so_no_else_if_chain_reaches_the_check() {
    // Rein has no `else`, so an `else if` chain never gets a blank-line warning. It is a syntax error.
    let src = "/** F. */\nexport default function f(/** A? */ a: boolean) {\n  if (a) {\n    return `yes`;\n  } else if (!a) {\n    return `no`;\n  }\n\n  return `maybe`;\n}\n";
    let (_, parse_errors) = crate::parser::parse(src);
    assert!(parse_errors.iter().any(|d| d.message.contains("Rein has no `else`")), "{parse_errors:?}");
}

#[test]
fn an_if_block_that_ends_the_file_needs_no_empty_line_after_it() {
    // The function's `}` shares the last line with the `if` block's `}`, so no line follows the block.
    let src = "/** F. */\nexport default function f(/** A? */ a: boolean) {\n  if (a) {\n    return `yes`;\n  } }";
    let (_, parse_errors) = crate::parser::parse(src);
    assert!(parse_errors.is_empty(), "{parse_errors:?}");
    assert!(if_blank_warnings(src).is_empty(), "{:?}", if_blank_warnings(src));
}

#[test]
fn crlf_line_endings_give_the_same_blank_line_warnings_as_lf() {
    let lf = "/** F. */\nexport default function f(/** A? */ a: boolean) {\n  if (a) {\n    return `yes`;\n  }\n  // Check the flag.\n  if (!a) {\n    return `no`;\n  }\n  return `maybe`;\n}\n";
    let crlf = lf.replace('\n', "\r\n");
    let expected = [
        ("add an empty line after the `if` block".to_string(), "}".to_string()),
        ("add an empty line before `if`".to_string(), "if".to_string()),
        ("add an empty line after the `if` block".to_string(), "}".to_string()),
    ];
    assert_eq!(if_blank_warnings(lf), expected);
    assert_eq!(if_blank_warnings(&crlf), expected);
}

#[test]
fn an_await_request_line_above_an_if_needs_an_empty_line_before_it() {
    let src = "/** F. */\nexport default async function f(/** A? */ a: boolean) {\n  /** Ok? */\n  const ok: boolean = await request(\"do it\");\n  if (ok) {\n    return `yes`;\n  }\n\n  return `no`;\n}\n";
    assert_eq!(
        if_blank_warnings(src),
        [("add an empty line before `if`".to_string(), "if".to_string())]
    );
}

#[test]
fn await_of_a_function_with_its_arguments_is_clean() {
    clean("import { merge } from \"./merge\";\n\n/** F. */\nexport default async function f(\n  /** The PR. */\n  pr: string,\n) {\n  /** Merged? */\n  const ok: boolean = await merge(pr=pr);\n\n  if (ok) {\n    return `yes`;\n  }\n\n  return `no`;\n}\n");
    clean("import { other } from \"./other\";\n\n/** F. */\nexport default async function f() {\n  /** Done? */\n  const ok: boolean = await other;\n\n  if (ok) {\n    return `yes`;\n  }\n\n  return `no`;\n}\n");
}

#[test]
fn await_reports_each_error_of_a_call_with_return() {
    let src = |line: &str| format!("/** F. */\nexport default async function f() {{\n  /** Done? */\n  {line}\n\n  if (ok) {{\n    return `yes`;\n  }}\n\n  return `no`;\n}}\n");
    let cases = [
        ("const ok: boolean = await @./tpl(a=`x`, b=`y`);", "`@./tpl` is a template; only a function can be awaited"),
        ("const ok: boolean = await @./tpl;", "`@./tpl` is a template; only a function can be awaited"),
        ("const ok: boolean = await @./missing;", "cannot find function `@./missing`"),
        ("const ok: boolean = await @./merge;", "`@./merge` takes 1 argument, got 0"),
        ("const ok: boolean = await @./merge(url=`u`);", "`url` is not a parameter of `@./merge`; parameters are pr"),
        ("const ok: boolean = await @./other(a=`x`);", "`@./other` takes no arguments"),
        ("const ok: string = await @./other;", "a `string` answer is not allowed"),
    ];
    for (line, part) in cases {
        expect_diag(&src(line), Severity::Error, part);
    }
    let sync = "import { other } from \"./other\";\n\n/** F. */\nexport default function f() {\n  /** Done? */\n  const ok: boolean = await other;\n\n  if (ok) {\n    return `yes`;\n  }\n\n  return `no`;\n}\n";
    expect_diag(sync, Severity::Error, "`await` needs `export default async function`");
}

#[test]
fn a_literal_of_the_right_type_is_a_typed_argument() {
    clean(&review_caller("pr=pr, big=true, ci=\"green\"").replace("  return", "  if (big || ci === \"red\") {\n    return `x`;\n  }\n\n  return"));
    let blue = check_src(&review_caller("pr=pr, big=big, ci=\"blue\""));
    let msg = "`blue` is not a member of `ci` of `@./review`; members are `green`, `red`";
    assert!(blue.iter().any(|d| d.severity == Severity::Error && d.message == msg), "{blue:?}");
    assert!(!blue.iter().any(|d| d.message.contains("backticks")), "{blue:?}");
    expect_diag(&review_caller("pr=pr, big=big, ci=true"), Severity::Error, "unknown parameter `true`");
    expect_diag(&review_caller("pr=pr, big=\"green\", ci=ci"), Severity::Error, "`big` of `@./review` is `boolean`");
}

#[test]
fn a_bare_hand_off_back_to_a_function_with_a_boolean_is_a_missing_typed_argument() {
    let src = "import { typed } from \"./typed\";\n\n/** F. */\nexport default function f() {\n  return typed;\n}\n";
    let diags = check_src(src);
    assert!(
        diags.iter().any(|d| d.severity == MISSING_TYPED_ARG && d.message.contains("`@./typed` takes 1 argument, got 0; pass `ok`")),
        "{diags:?}"
    );
}

#[test]
fn true_and_false_are_not_names() {
    let param = "/** F. */\nexport default function f(/** Ok? */ true: boolean) {\n  return `b`;\n}\n";
    expect_diag(param, Severity::Error, "`true` is a boolean literal; pick another name");
    let answer = "/** F. */\nexport default async function f() {\n  /** Ok? */\n  const false: boolean = await request(`Ask.`);\n  return `b`;\n}\n";
    expect_diag(answer, Severity::Error, "`false` is a boolean literal; pick another name");
}
