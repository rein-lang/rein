use super::*;
use crate::project_fixture::{project, TICKET_SCHEMA};

fn answers(pairs: &[(&str, &str)]) -> Answers {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn req(entry: &str, pairs: &[(&str, &str)]) -> StepRequest {
    StepRequest {
        version: PROTOCOL_VERSION,
        entry: entry.into(),
        answers: answers(pairs),
        flow: None,
        inputs: BTreeMap::new(),
        walk: false,
    }
}

fn trace(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// `main` asks for `vip`, then hands off to `escalate` or returns text.
fn vip_project() -> Project {
    project(&[
        ("main.rein", "import { escalate } from \"./escalate\";\n\n/** Entry. */\nexport default function main(\n  /** Is the customer on the VIP plan? */\n  vip: boolean,\n) {\n  if (vip) {\n    return escalate;\n  }\n  return `Answer the ticket.`;\n}\n"),
        ("escalate.rein", "/** Escalate. */\nexport default function escalate() {\n  return `Assign the ticket to tier 2.`;\n}\n"),
    ])
}

/// `main` picks a path. `merge` goes to `merge_pr`, which asks two questions.
fn merge_project() -> Project {
    project(&[
        ("main.rein", "import { mergePr } from \"./merge_pr\";\n\n/** Entry. */\nexport default function main(\n  /** Which path? */\n  path: \"merge\" | \"review\",\n) {\n  switch (path) {\n    case \"merge\":\n      return mergePr;\n    case \"review\":\n      return `Review the code.`;\n  }\n}\n"),
        ("merge_pr.rein", "interface Input {\n  /** Does the PR change only docs? */\n  docs_only: boolean;\n  /** What is the CI status? */\n  ci: \"green\" | \"red\";\n}\n/** Merge a PR. */\nexport default function mergePr({ docs_only, ci }: Input) {\n  if (docs_only) {\n    return `approved`;\n  }\n  switch (ci) {\n    case \"green\":\n      return `merge it`;\n    case \"red\":\n      return `wait for CI`;\n  }\n}\n"),
    ])
}

/// `main` asks `article_found` through `answer/faq`, which asks a request, then escalates.
fn faq_project() -> Project {
    project(&[
        ("main.rein", "import { answerFaq } from \"./answer/faq\";\n\n/** Entry. */\nexport default function main() {\n  return answerFaq;\n}\n"),
        ("answer/faq.rein", "import { escalate } from \"../escalate\";\n\n/** Answer a how-to question. */\nexport default async function answerFaq(\n  /** Did you find an article? */\n  article_found: boolean,\n) {\n  if (!article_found) {\n    return escalate;\n  }\n  /** Did the article solve it? */\n  const solved: boolean = await request(\n    `Send the article and ask if it solved the problem.`,\n  );\n  if (solved) {\n    return `Close the ticket as solved.`;\n  }\n  return escalate;\n}\n"),
        ("escalate.rein", "/** Escalate. */\nexport default function escalate() {\n  return `Assign the ticket to tier 2.`;\n}\n"),
    ])
}

#[test]
fn question_at_main_when_no_answers() {
    let res = step(&vip_project(), &req("main", &[]));
    assert_eq!(
        res,
        StepResponse::Question {
            version: 1,
            key: "main.vip".into(),
            function: "main".into(),
            name: "vip".into(),
            r#type: "boolean".into(),
            options: None,
            description: "Is the customer on the VIP plan?".into(),
            answers: answers(&[]),
            trace: trace(&["main"]),
        }
    );
}

#[test]
fn answers_for_main_auto_follow_into_merge_pr() {
    let res = step(&merge_project(), &req("main", &[("main.path", "merge")]));
    assert_eq!(
        res,
        StepResponse::Question {
            version: 1,
            key: "merge_pr.docs_only".into(),
            function: "merge_pr".into(),
            name: "docs_only".into(),
            r#type: "boolean".into(),
            options: None,
            description: "Does the PR change only docs?".into(),
            answers: answers(&[("main.path", "merge")]),
            trace: trace(&["main", "merge_pr"]),
        }
    );
}

#[test]
fn union_question_lists_its_options() {
    let res = step(
        &merge_project(),
        &req(
            "main",
            &[("main.path", "merge"), ("merge_pr.docs_only", "no")],
        ),
    );
    let StepResponse::Question {
        key,
        r#type,
        options,
        ..
    } = res
    else {
        panic!("{res:?}")
    };
    assert_eq!(key, "merge_pr.ci");
    assert_eq!(r#type, "union");
    assert_eq!(options, Some(vec!["green".to_string(), "red".to_string()]));
}

#[test]
fn full_path_is_done_with_the_trace() {
    let res = step(
        &merge_project(),
        &req(
            "main",
            &[
                ("main.path", "merge"),
                ("merge_pr.docs_only", "no"),
                ("merge_pr.ci", "green"),
            ],
        ),
    );
    assert_eq!(
        res,
        StepResponse::Done {
            version: 1,
            instruction: "merge it".into(),
            answers: answers(&[
                ("main.path", "merge"),
                ("merge_pr.docs_only", "no"),
                ("merge_pr.ci", "green")
            ]),
            trace: trace(&["main", "merge_pr"]),
        }
    );
}

#[test]
fn async_request_stops_then_the_answer_continues() {
    let project = faq_project();
    let res = step(
        &project,
        &req("main", &[("answer/faq.article_found", "yes")]),
    );
    assert_eq!(
        res,
        StepResponse::Request {
            version: 1,
            key: "answer/faq.solved".into(),
            function: "answer/faq".into(),
            instruction: "Send the article and ask if it solved the problem.".into(),
            response: ResponseSpec {
                name: "solved".into(),
                r#type: "boolean".into(),
                options: None,
                description: "Did the article solve it?".into(),
                fields: None,
            },
            answers: answers(&[("answer/faq.article_found", "yes")]),
            trace: trace(&["main", "answer/faq"]),
        }
    );

    let res = step(
        &project,
        &req(
            "main",
            &[
                ("answer/faq.article_found", "yes"),
                ("answer/faq.solved", "yes"),
            ],
        ),
    );
    assert_eq!(
        res,
        StepResponse::Done {
            version: 1,
            instruction: "Close the ticket as solved.".into(),
            answers: answers(&[
                ("answer/faq.article_found", "yes"),
                ("answer/faq.solved", "yes")
            ]),
            trace: trace(&["main", "answer/faq"]),
        }
    );

    let res = step(
        &project,
        &req(
            "main",
            &[
                ("answer/faq.article_found", "yes"),
                ("answer/faq.solved", "no"),
            ],
        ),
    );
    let StepResponse::Done {
        instruction,
        trace: t,
        ..
    } = res
    else {
        panic!("{res:?}")
    };
    assert_eq!(instruction, "Assign the ticket to tier 2.");
    assert_eq!(t, trace(&["main", "answer/faq", "escalate"]));
}

#[test]
fn unknown_key_is_an_error() {
    let res = step(&vip_project(), &req("main", &[("main.nope", "x")]));
    let StepResponse::Error { message, key, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(message, "unknown key `main.nope`");
    assert_eq!(key, Some("main.nope".into()));
}

#[test]
fn key_without_a_function_is_an_error() {
    let res = step(&vip_project(), &req("main", &[("vip", "yes")]));
    let StepResponse::Error { message, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(message, "unknown key `vip`");
}

#[test]
fn invalid_boolean_value_is_an_error() {
    let res = step(&vip_project(), &req("main", &[("main.vip", "maybe")]));
    let StepResponse::Error { message, key, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(
        message,
        "invalid value `maybe` for `vip`; expected true, false, yes or no"
    );
    assert_eq!(key, Some("main.vip".into()));
}

#[test]
fn invalid_union_value_lists_the_accepted_values() {
    let res = step(&merge_project(), &req("main", &[("main.path", "purple")]));
    let StepResponse::Error { message, .. } = res else {
        panic!("{res:?}")
    };
    assert!(
        message.contains("expected one of: `merge`, `review`"),
        "{message}"
    );
}

#[test]
fn string_parameter_cannot_be_answered() {
    let project = project(&[
        ("main.rein", "import { greet } from \"./greet\";\n\n/** Entry. */\nexport default function main() {\n  return greet(who=`world`);\n}\n"),
        ("greet.rein", "/** Greet. */\nexport default function greet(\n  /** Who? */\n  who: string,\n) {\n  return `Hello ${who}`;\n}\n"),
    ]);
    assert_eq!(
        step(&project, &req("main", &[])),
        StepResponse::Done {
            version: 1,
            instruction: "Hello world".into(),
            answers: answers(&[]),
            trace: trace(&["main"])
        }
    );
    let res = step(&project, &req("main", &[("greet.who", "x")]));
    let StepResponse::Error { key, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(key, Some("greet.who".into()));
}

#[test]
fn same_parameter_name_in_two_functions_gets_separate_keys() {
    let project = project(&[
        ("main.rein", "import { mergePr } from \"./merge_pr\";\n\n/** Entry. */\nexport default function main(\n  /** CI? */\n  ci: \"green\" | \"red\",\n) {\n  if (ci === \"green\") {\n    return mergePr;\n  }\n  return `Wait.`;\n}\n"),
        ("merge_pr.rein", "/** Merge. */\nexport default function mergePr(\n  /** CI for the merge? */\n  ci: \"green\" | \"red\",\n) {\n  switch (ci) {\n    case \"green\":\n      return `Merge ${ci}`;\n    case \"red\":\n      return `Stop.`;\n  }\n}\n"),
    ]);
    // The answer to main.ci does not answer merge_pr.ci.
    let res = step(&project, &req("main", &[("main.ci", "green")]));
    let StepResponse::Question { key, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(key, "merge_pr.ci");

    let res = step(
        &project,
        &req("main", &[("main.ci", "green"), ("merge_pr.ci", "green")]),
    );
    let StepResponse::Done { instruction, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(instruction, "Merge green");
}

#[test]
fn reference_cycle_hits_the_hop_limit() {
    let project = project(&[
        (
            "main.rein",
            "import { a } from \"./a\";\n\n/** Entry. */\nexport default function main() {\n  return a;\n}\n",
        ),
        (
            "a.rein",
            "import { b } from \"./b\";\n\n/** A. */\nexport default function a() {\n  return b;\n}\n",
        ),
        (
            "b.rein",
            "import { a } from \"./a\";\n\n/** B. */\nexport default function b() {\n  return a;\n}\n",
        ),
    ]);
    let res = step(&project, &req("main", &[]));
    let StepResponse::Error {
        message, trace: t, ..
    } = res
    else {
        panic!("{res:?}")
    };
    assert_eq!(
        message,
        "process did not finish in 64 steps (reference cycle?)"
    );
    assert_eq!(t.len(), HOP_LIMIT);
}

#[test]
fn template_as_entry_is_an_error() {
    let project = project(&[
        ("main.rein", "/** Entry. */\nexport default function main() {\n  return `x`;\n}\n"),
        ("greet.rein", "/** Greet. */\nexport default function greet(\n  /** Who? */\n  who: string,\n) {\n  return `Hello ${who}`;\n}\n"),
    ]);
    let res = step(&project, &req("greet", &[]));
    let StepResponse::Error { message, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(message, "templates are called from functions, not run");
}

#[test]
fn template_call_adds_no_hop_to_the_trace() {
    let project = project(&[
        ("main.rein", "import { tpl } from \"./tpl\";\n\n/** Entry. */\nexport default function main() {\n  return tpl(a=`Hello`);\n}\n"),
        ("tpl.rein", "/** Template. */\nexport default function tpl(\n  /** Greeting. */\n  a: string,\n) {\n  return `Hi ${a}`;\n}\n"),
    ]);
    let res = step(&project, &req("main", &[]));
    assert_eq!(
        res,
        StepResponse::Done {
            version: 1,
            instruction: "Hi Hello".into(),
            answers: answers(&[]),
            trace: trace(&["main"]),
        }
    );
}

#[test]
fn nested_template_call_adds_no_hop_to_the_trace() {
    let project = project(&[
        ("main.rein", "import { tpl } from \"./tpl\";\n\n/** Entry. */\nexport default function main() {\n  return delegate(model=tpl(a=`x`), task=`task`);\n}\n"),
        ("tpl.rein", "/** Template. */\nexport default function tpl(\n  /** Greeting. */\n  a: string,\n) {\n  return `Hi ${a}`;\n}\n"),
    ]);
    let res = step(&project, &req("main", &[]));
    assert_eq!(
        res,
        StepResponse::Done {
            version: 1,
            instruction: "Delegate this task to a subagent.\nModel: Hi x\nTask: task".into(),
            answers: answers(&[]),
            trace: trace(&["main"]),
        }
    );
}

#[test]
fn project_with_errors_is_refused() {
    let project = project(&[(
        "main.rein",
        "/** Entry. */\nexport default function main() {\n  return @./missing;\n}\n",
    )]);
    let res = step(&project, &req("main", &[]));
    let StepResponse::Error {
        message,
        diagnostics,
        ..
    } = res
    else {
        panic!("{res:?}")
    };
    assert_eq!(
        message,
        "project has 1 error(s); run `rein check` and fix them"
    );
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].file, "main.rein");
    assert!(
        diagnostics[0].message.contains("cannot find function"),
        "{diagnostics:?}"
    );
}

#[test]
fn unknown_entry_is_an_error() {
    let res = step(&vip_project(), &req("nope", &[]));
    let StepResponse::Error { message, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(message, "unknown function `nope`");
}

#[test]
fn request_defaults_to_version_one_and_main() {
    let parsed: StepRequest = serde_json::from_str("{}").unwrap();
    assert_eq!(
        parsed,
        StepRequest {
            version: 1,
            entry: "main".into(),
            answers: answers(&[]),
            flow: None,
            inputs: BTreeMap::new(),
            walk: false,
        }
    );
}

#[test]
fn other_protocol_version_is_refused() {
    let mut r = req("main", &[]);
    r.version = 2;
    let res = step(&vip_project(), &r);
    let StepResponse::Error { message, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(
        message,
        "unsupported protocol version 2; this build speaks 1"
    );
}

#[test]
fn same_request_gives_byte_equal_responses() {
    let project = merge_project();
    let forward = req(
        "main",
        &[("main.path", "merge"), ("merge_pr.docs_only", "no")],
    );
    let mut backward = req("main", &[]);
    backward
        .answers
        .insert("merge_pr.docs_only".into(), "no".into());
    backward.answers.insert("main.path".into(), "merge".into());
    let a = serde_json::to_string(&step(&project, &forward)).unwrap();
    let b = serde_json::to_string(&step(&project, &forward)).unwrap();
    let c = serde_json::to_string(&step(&project, &backward)).unwrap();
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn project_and_protocol_types_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Project>();
    assert_send_sync::<StepRequest>();
    assert_send_sync::<StepResponse>();
}

/// `main` takes a link as an input and hands it to `merge`, which asks the agent for `ci`.
fn pr_project() -> Project {
    project(&[
        ("main.rein", "import { merge } from \"./merge\";\n\ninterface Input {\n  /** The link. */\n  pr: string;\n  /** Draft? */\n  draft: boolean;\n}\n/** Entry. */\nexport default function main({ pr, draft }: Input) {\n  if (draft) {\n    return `Stop ${pr}.`;\n  }\n  return merge(pr=pr);\n}\n"),
        ("merge.rein", "interface Input {\n  /** The link. */\n  pr: string;\n  /** CI? */\n  ci: \"green\" | \"red\";\n}\n/** Merge. */\nexport default function merge({ pr, ci }: Input) {\n  switch (ci) {\n    case \"green\":\n      return `Merge ${pr}.`;\n    case \"red\":\n      return `Wait.`;\n  }\n}\n"),
    ])
}

/// A request with `answers` and `inputs`, both as `(name, value)` pairs.
fn req_with(entry: &str, answers: &[(&str, &str)], inputs: &[(&str, &str)]) -> StepRequest {
    let mut r = req(entry, answers);
    r.inputs = inputs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    r
}

#[test]
fn string_input_is_never_asked() {
    let res = step(&pr_project(), &req_with("main", &[], &[("pr", "L")]));
    let StepResponse::Question { key, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(key, "main.draft");
}

#[test]
fn missing_entry_input_is_an_error() {
    let res = step(&pr_project(), &req("main", &[]));
    let StepResponse::Error { message, key, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(
        message,
        "missing input `pr`; give it with --input pr=<value>"
    );
    assert_eq!(key, None);
}

#[test]
fn unknown_entry_input_is_an_error() {
    let res = step(
        &pr_project(),
        &req_with("main", &[], &[("pr", "L"), ("zz", "x")]),
    );
    let StepResponse::Error { message, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(message, "unknown input `zz`");
}

#[test]
fn a_boolean_parameter_is_not_an_input() {
    let res = step(
        &pr_project(),
        &req_with("main", &[], &[("pr", "L"), ("draft", "no")]),
    );
    let StepResponse::Error { message, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(message, "unknown input `draft`");
}

#[test]
fn hand_off_binds_the_input_for_the_target() {
    let res = step(
        &pr_project(),
        &req_with(
            "main",
            &[("main.draft", "no"), ("merge.ci", "green")],
            &[("pr", "L")],
        ),
    );
    assert_eq!(
        res,
        StepResponse::Done {
            version: 1,
            instruction: "Merge L.".into(),
            answers: answers(&[("main.draft", "no"), ("merge.ci", "green")]),
            trace: trace(&["main", "merge"]),
        }
    );
}

#[test]
fn hand_off_arguments_bind_in_declaration_order() {
    let project = project(&[
        ("main.rein", "import { two } from \"./two\";\n\ninterface Input {\n  /** A */\n  a: string;\n  /** B */\n  b: string;\n}\n/** Entry. */\nexport default function main({ a, b }: Input) {\n  return two(y=a, x=b);\n}\n"),
        ("two.rein", "interface Input {\n  /** X */\n  x: string;\n  /** Y */\n  y: string;\n  /** Keep order? */\n  ok: boolean;\n}\n/** Two. */\nexport default function two({ x, y, ok }: Input) {\n  if (ok) {\n    return `${x} then ${y}`;\n  }\n  return `${y} then ${x}`;\n}\n"),
    ]);
    let inputs = [("a", "A"), ("b", "B")];
    let res = step(&project, &req_with("main", &[("two.ok", "yes")], &inputs));
    let StepResponse::Done { instruction, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(instruction, "B then A");
    let res = step(&project, &req_with("main", &[("two.ok", "no")], &inputs));
    let StepResponse::Done { instruction, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(instruction, "A then B");
}

#[test]
fn string_answer_key_is_still_an_error() {
    let res = step(
        &pr_project(),
        &req_with("main", &[("main.pr", "x")], &[("pr", "L")]),
    );
    let StepResponse::Error { message, key, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!(
        message,
        "`main.pr` is a string parameter; only a template takes one"
    );
    assert_eq!(key, Some("main.pr".into()));
}

#[test]
fn inputs_are_read_from_json_and_left_out_when_empty() {
    let parsed: StepRequest = serde_json::from_str(r#"{"inputs": {"pr": "L"}}"#).unwrap();
    assert_eq!(parsed.inputs.get("pr").map(String::as_str), Some("L"));
    let empty = serde_json::to_string(&req("main", &[])).unwrap();
    assert!(!empty.contains("inputs"), "{empty}");
}

#[test]
fn step_follows_a_hand_off_into_another_program() {
    let p = crate::program_fixture::with_program("return @pr-handling/main(pr=pr);");
    let mut request = req("main", &[]);
    request.inputs.insert("pr".into(), "https://x/pr/1".into());
    let StepResponse::Question { key, function, trace, .. } = step(&p, &request) else { panic!("not a question") };
    assert_eq!(key, "@pr-handling/main.draft");
    assert_eq!(function, "@pr-handling/main");
    assert_eq!(trace, ["main", "@pr-handling/main"]);

    request.answers = answers(&[("@pr-handling/main.draft", "false")]);
    let StepResponse::Question { key, .. } = step(&p, &request) else { panic!("not a question") };
    assert_eq!(key, "@pr-handling/main.task");

    request.answers = answers(&[("@pr-handling/main.draft", "false"), ("@pr-handling/main.task", "merge")]);
    let StepResponse::Done { instruction, .. } = step(&p, &request) else { panic!("not done") };
    assert_eq!(instruction, "Do merge on https://x/pr/1.");
}

const PICKER: &str = "interface Pick {\n  /** The PR link. */\n  pr: string;\n  /** Which task. */\n  task: \"review\" | \"merge\";\n}\n/** Entry. */\nexport default async function main() {\n  /** The pull request of the next notification. */\n  const pick: Pick = await request(`pick a PR`);\n  return @pr-handling/main(pr=pick.pr);\n}\n";

#[test]
fn struct_answer_asks_for_a_json_object_with_the_fields_of_the_struct() {
    let p = crate::program_fixture::with_main(PICKER.to_string());
    let StepResponse::Request { response, key, .. } = step(&p, &req("main", &[])) else { panic!("not a request") };
    assert_eq!((response.r#type.as_str(), key.as_str()), ("struct", "main.pick"));
    let fields = response.fields.unwrap();
    let shown: Vec<(&str, &str)> = fields.iter().map(|f| (f.name.as_str(), f.r#type.as_str())).collect();
    assert_eq!(shown, [("pr", "string"), ("task", "union")]);
    assert_eq!(fields[1].options, Some(vec!["review".to_string(), "merge".to_string()]));
    assert_eq!(fields[0].description, "The PR link.");
}

#[test]
fn struct_answer_field_of_iri_carries_its_json_schema() {
    let p = crate::program_fixture::with_main(LINK_PICKER.to_string());
    let StepResponse::Request { response, .. } = step(&p, &req("main", &[])) else { panic!("not a request") };
    let fields = response.fields.unwrap();
    assert_eq!(fields[0].r#type, "iri");
    assert_eq!(fields[0].schema, Some(serde_json::json!({"type": "string", "format": "iri"})));
}

#[test]
fn struct_answer_plain_string_field_has_no_schema() {
    let p = crate::program_fixture::with_main(PICKER.to_string());
    let StepResponse::Request { response, .. } = step(&p, &req("main", &[])) else { panic!("not a request") };
    let fields = response.fields.unwrap();
    assert_eq!((fields[0].name.as_str(), fields[0].r#type.as_str()), ("pr", "string"));
    assert_eq!(fields[0].schema, None);
}

#[test]
fn plain_string_field_spec_serializes_without_a_schema_key() {
    let field = FieldSpec { name: "pr".into(), r#type: "string".into(), options: None, description: "The PR link.".into(), schema: None };
    let value = serde_json::to_value(&field).unwrap();
    let object = value.as_object().unwrap();
    assert!(!object.contains_key("schema"), "{value}");
    assert_eq!(value["type"], "string");
}

#[test]
fn struct_answer_union_field_has_no_schema() {
    let p = crate::program_fixture::with_main(PICKER.to_string());
    let StepResponse::Request { response, .. } = step(&p, &req("main", &[])) else { panic!("not a request") };
    let fields = response.fields.unwrap();
    assert_eq!((fields[1].name.as_str(), fields[1].r#type.as_str()), ("task", "union"));
    assert_eq!(fields[1].schema, None);
}

#[test]
fn struct_answer_field_of_a_schema_type_carries_the_user_schema() {
    let picker = "import { Ticket } from \"./ticket.schema\";\n\ninterface Pick {\n  /** The ticket. */\n  ticket: Ticket;\n}\n/** Entry. */\nexport default async function main() {\n  /** The ticket to fix. */\n  const pick: Pick = await request(`pick a ticket`);\n  return `Fix ${pick.ticket}.`;\n}\n";
    let p = project(&[("main.rein", picker), ("ticket.schema.json", TICKET_SCHEMA)]);
    let StepResponse::Request { response, .. } = step(&p, &req("main", &[])) else { panic!("not a request") };
    let fields = response.fields.unwrap();
    assert_eq!(fields[0].r#type, "string");
    let schema = fields[0].schema.as_ref().expect("schema of the user type");
    assert_eq!(schema, &serde_json::json!({"title": "Ticket", "type": "string", "pattern": "^[A-Z]+-[0-9]+$", "maxLength": 20, "examples": ["ABC-123"]}));
}

#[test]
fn struct_file_schema_field_carries_the_schema_its_own_file_imports() {
    // The schema import sits in `pick.struct.rein`, not in the file that uses the struct.
    let main = "import { Pick } from \"./pick.struct\";\n\n/** Entry. */\nexport default async function main() {\n  /** The pick. */\n  const pick: Pick = await request(`pick a ticket`);\n  return `Fix ${pick.ticket}.`;\n}\n";
    let shared = "import { Ticket } from \"./ticket.schema\";\n\n/** A picked ticket. */\nexport default interface Pick {\n  /** The ticket. */\n  ticket: Ticket;\n}\n";
    let p = project(&[("main.rein", main), ("pick.struct.rein", shared), ("ticket.schema.json", TICKET_SCHEMA)]);
    let StepResponse::Request { response, .. } = step(&p, &req("main", &[])) else { panic!("not a request: {:?}", step(&p, &req("main", &[]))) };
    let fields = response.fields.unwrap();
    assert_eq!(fields[0].r#type, "string");
    let schema = fields[0].schema.as_ref().expect("schema of the user type from the struct file");
    assert_eq!(schema, &serde_json::json!({"title": "Ticket", "type": "string", "pattern": "^[A-Z]+-[0-9]+$", "maxLength": 20, "examples": ["ABC-123"]}));
}

#[test]
fn struct_answer_is_validated_field_by_field() {
    let p = crate::program_fixture::with_main(PICKER.to_string());
    let cases = [
        ("{\"task\":\"merge\"}", "missing field `pr`"),
        ("{\"pr\":\"u/1\",\"task\":\"merge\",\"x\":\"1\"}", "`x` is not a field of `Pick`; fields are pr, task"),
        ("{\"pr\":\"u/1\",\"task\":\"nope\"}", "invalid value `nope` for `pick.task`"),
        ("{\"pr\":\"\",\"task\":\"merge\"}", "field `pick.pr` is empty"),
        ("{\"pr\":1,\"task\":\"merge\"}", "field `pick.pr` must be a string"),
        ("not json", "the answer `pick` is not valid JSON (expected ident at line 1 column 2): `not json`"),
        ("{\"pr\":\"u/1\",}", "the answer `pick` is not valid JSON (trailing comma at line 1 column 13): `{\"pr\":\"u/1\",}`"),
        ("[1]", "must be a JSON object, like {\"pr\": ..., \"task\": ...}; got `[1]`"),
    ];
    for (raw, part) in cases {
        let StepResponse::Error { message, key, .. } = step(&p, &req("main", &[("main.pick", raw)])) else { panic!("not an error: {raw}") };
        assert!(message.contains(part), "{raw}: {message}");
        assert_eq!(key.as_deref(), Some("main.pick"));
    }
}

#[test]
fn empty_format_field_is_rejected_like_an_empty_string_field() {
    for ty in ["Regex", "JSONPointer"] {
        let source = format!(
            "interface Pick {{\n  /** The value. */\n  value: {ty};\n  /** The name. */\n  name: string;\n}}\n/** Entry. */\nexport default async function main() {{\n  /** The pick. */\n  const pick: Pick = await request(`pick`);\n  return `done`;\n}}\n"
        );
        let p = crate::program_fixture::with_main(source);
        let raw = "{\"value\":\"\",\"name\":\"x\"}";
        let StepResponse::Error { message, .. } = step(&p, &req("main", &[("main.pick", raw)])) else { panic!("not an error: {ty}") };
        assert_eq!(message, "field `pick.value` is empty; give some text", "{ty}");
    }
}

#[test]
fn template_with_an_iri_parameter_behaves_like_a_string_one() {
    let template = |ty: &str| {
        let tpl = format!("/** Template. */\nexport default function tpl(\n  /** Greeting. */\n  a: {ty},\n) {{\n  return `Hi ${{a}}`;\n}}\n");
        project(&[
            ("main.rein", "import { tpl } from \"./tpl\";\n\n/** Entry. */\nexport default function main() {\n  return tpl(a=`https://example.com/a`);\n}\n"),
            ("tpl.rein", tpl.as_str()),
        ])
    };
    let as_string = step(&template("string"), &req("main", &[]));
    let as_iri = step(&template("IRI"), &req("main", &[]));
    assert_eq!(
        as_iri,
        StepResponse::Done {
            version: 1,
            instruction: "Hi https://example.com/a".into(),
            answers: answers(&[]),
            trace: trace(&["main"]),
        }
    );
    assert_eq!(as_iri, as_string);
}

const LINK_PICKER: &str = "interface Pick {\n  /** The PR link. */\n  pr: IRI;\n}\n/** Entry. */\nexport default async function main() {\n  /** The pull request of the next notification. */\n  const pick: Pick = await request(`pick a PR`);\n  return `Open ${pick.pr}.`;\n}\n";

#[test]
fn struct_answer_field_of_iri_must_be_an_absolute_iri() {
    let p = crate::program_fixture::with_main(LINK_PICKER.to_string());
    let cases = [
        ("{\"pr\":\"example.com\"}", "`pick.pr` must be an absolute IRI"),
        ("{\"pr\":1}", "field `pick.pr` must be a string"),
    ];
    for (raw, part) in cases {
        let StepResponse::Error { message, key, .. } = step(&p, &req("main", &[("main.pick", raw)])) else { panic!("not an error: {raw}") };
        assert!(message.contains(part), "{raw}: {message}");
        assert_eq!(key.as_deref(), Some("main.pick"));
    }
    let ok = "{\"pr\":\"https://example.com/pr/1\"}";
    let StepResponse::Done { instruction, .. } = step(&p, &req("main", &[("main.pick", ok)])) else { panic!("not done") };
    assert_eq!(instruction, "Open https://example.com/pr/1.");
}

const IRI_ENTRY: &str = "interface Input {\n  /** The link. */\n  link: IRI;\n}\n/** Entry. */\nexport default function main({ link }: Input) {\n  return delegate(model=`x`, task=`Open ${link}.`);\n}\n";

#[test]
fn iri_entry_input_must_be_an_absolute_iri() {
    let p = project(&[("main.rein", IRI_ENTRY)]);
    let res = step(&p, &req_with("main", &[], &[("link", "https://example.com/a")]));
    let StepResponse::Done { instruction, .. } = res else { panic!("not done: {res:?}") };
    assert!(instruction.ends_with("Task: Open https://example.com/a."), "{instruction}");
    let res = step(&p, &req_with("main", &[], &[("link", "not a link")]));
    let StepResponse::Error { message, .. } = res else { panic!("not an error: {res:?}") };
    assert!(message.contains("`link` must be an absolute IRI"), "{message}");
}

#[test]
fn hand_off_to_an_iri_parameter_checks_the_value() {
    let files = [
        ("main.rein", "import { other } from \"./other\";\n\ninterface Pick {\n  /** The link. */\n  link: string;\n}\n/** Entry. */\nexport default async function main() {\n  /** The link to open. */\n  const pick: Pick = await request(`pick a link`);\n  return other(link=pick.link);\n}\n"),
        ("other.rein", "/** Open a link. */\nexport default function other(\n  /** The link. */ link: IRI,\n) {\n  return `Open ${link}.`;\n}\n"),
    ];
    let p = project(&files);
    let valid = "{\"link\":\"https://example.com/pr/1\"}";
    let StepResponse::Done { instruction, .. } = step(&p, &req("main", &[("main.pick", valid)])) else { panic!("not done") };
    assert_eq!(instruction, "Open https://example.com/pr/1.");
    let invalid = "{\"link\":\"example.com\"}";
    let StepResponse::Error { message, .. } = step(&p, &req("main", &[("main.pick", invalid)])) else { panic!("not an error") };
    assert!(message.contains("must be an absolute IRI"), "{message}");
}

/// `main` has a `string` input `s` and passes it with `call`, its return statement, to a function in `target`.
fn text_to_iri(import: &str, call: &str, target: &str) -> Project {
    let main = format!("{import}\n/** Entry. */\nexport default function main(\n  /** The link. */\n  s: string,\n) {{\n  return {call};\n}}\n");
    project(&[("main.rein", main.as_str()), ("target.rein", target)])
}

/// The walk of `p` from `main` with its unset `string` input `s`: the stand-in value, and the walk flag.
fn walk_of_s(p: &Project) -> StepRequest {
    let value = walk_value(p.get("main").unwrap().function.input("s").unwrap());
    StepRequest { inputs: BTreeMap::from([("s".to_string(), value)]), walk: true, ..req("main", &[]) }
}

/// A hand-off target with an `IRI` parameter. It is not a template, so a walk hands off to it.
const OPEN_IRI: &str = "/** Open a link. */\nexport default function target(\n  /** The link. */ link: IRI,\n) {\n  return delegate(model=`x`, task=`Open ${link}.`);\n}\n";

#[test]
fn walk_of_a_string_handed_to_an_iri_parameter_has_no_error() {
    let p = text_to_iri("", "@./target(link=s)", OPEN_IRI);
    let walked = owners(&p, "main", &answers(&[]));
    assert!(walked.iter().any(|(owner, passed)| owner == "target" && passed.get("link").map(String::as_str) == Some("<s>")), "{walked:?}");
    let res = step(&p, &walk_of_s(&p));
    assert!(matches!(res, StepResponse::Done { .. }), "{res:?}");
}

#[test]
fn walk_of_a_string_in_a_template_literal_argument_has_no_error() {
    let p = text_to_iri("", "@./target(link=`${s}`)", OPEN_IRI);
    let res = step(&p, &walk_of_s(&p));
    assert!(matches!(res, StepResponse::Done { .. }), "{res:?}");
}

#[test]
fn walk_of_a_string_in_a_template_call_has_no_error() {
    let tpl = "/** Template. */\nexport default function tpl(\n  /** Greeting. */\n  a: IRI,\n) {\n  return `Hi ${a}`;\n}\n";
    let p = text_to_iri("import { tpl } from \"./target\";", "tpl(a=s)", tpl);
    let res = step(&p, &walk_of_s(&p));
    assert!(matches!(res, StepResponse::Done { .. }), "{res:?}");
}

#[test]
fn real_run_of_a_string_handed_to_an_iri_parameter_checks_the_value() {
    let p = text_to_iri("", "@./target(link=s)", OPEN_IRI);
    let res = step(&p, &req_with("main", &[], &[("s", "not-a-link")]));
    let StepResponse::Error { message, .. } = res else { panic!("not an error: {res:?}") };
    assert!(message.contains("must be an absolute IRI"), "{message}");
}

#[test]
fn struct_answer_field_reaches_the_hand_off() {
    let p = crate::program_fixture::with_main(PICKER.to_string());
    let pick = "{\"pr\":\"https://x/pr/1\",\"task\":\"merge\"}";
    let StepResponse::Question { key, .. } = step(&p, &req("main", &[("main.pick", pick)])) else { panic!("not a question") };
    assert_eq!(key, "@pr-handling/main.draft");
    let res = step(&p, &req("main", &[("main.pick", pick), ("@pr-handling/main.draft", "false"), ("@pr-handling/main.task", "merge")]));
    let StepResponse::Done { instruction, .. } = res else { panic!("not done: {res:?}") };
    assert_eq!(instruction, "Do merge on https://x/pr/1.");
}

#[test]
fn shared_struct_answer_field_decides_a_branch() {
    let main = "import { Pick } from \"./pick.struct\";\n\n/** Entry. */\nexport default async function main() {\n  /** The pull request of the next notification. */\n  const pick: Pick = await request(`pick a PR`);\n  if (pick.urgent) {\n    return `Hurry ${pick.pr}.`;\n  }\n  return `Handle ${pick.pr}.`;\n}\n";
    let shared = "/** A picked PR. */\nexport default interface Pick {\n  /** The PR link. */\n  pr: string;\n  /** Is it urgent? */\n  urgent: boolean;\n}\n";
    let p = project(&[("main.rein", main), ("pick.struct.rein", shared)]);
    let res = step(&p, &req("main", &[("main.pick", "{\"pr\":\"u/1\",\"urgent\":true}")]));
    let StepResponse::Done { instruction, .. } = res else { panic!("not done: {res:?}") };
    assert_eq!(instruction, "Hurry u/1.");
    let res = step(&p, &req("main", &[("main.pick", "{\"pr\":\"u/1\",\"urgent\":\"yes\"}")]));
    let StepResponse::Error { message, .. } = res else { panic!("not an error") };
    assert!(message.contains("field `pick.urgent` must be true or false"), "{message}");
}

#[test]
fn struct_answer_is_a_json_object_on_the_wire() {
    let wire = r#"{"answers":{"main.pick":{"pr":"u/1","task":"merge"},"main.ci":"green"}}"#;
    let request: StepRequest = serde_json::from_str(wire).unwrap();
    assert_eq!(request.answers["main.pick"], r#"{"pr":"u/1","task":"merge"}"#);
    assert_eq!(request.answers["main.ci"], "green");
    let back = serde_json::to_string(&request).unwrap();
    assert!(back.contains(r#""main.pick":{"pr":"u/1","task":"merge"}"#) && back.contains(r#""main.ci":"green""#), "{back}");
}

#[test]
fn built_in_format_field_serializes_its_schema_in_the_step_json() {
    let p = crate::program_fixture::with_main(LINK_PICKER.to_string());
    let res = step(&p, &req("main", &[]));
    let value = serde_json::to_value(&res).unwrap();
    assert_eq!(value["kind"], "request");
    assert_eq!(value["response"]["fields"][0]["name"], "pr");
    assert_eq!(value["response"]["fields"][0]["type"], "iri");
    assert_eq!(value["response"]["fields"][0]["schema"], serde_json::json!({"type": "string", "format": "iri"}));
}

#[test]
fn double_equals_and_not_equal_branch_like_triple_forms() {
    let make = |op: &str| {
        project(&[("main.rein", &format!("/** Entry. */\nexport default function main(\n  /** CI? */\n  ci: \"green\" | \"red\",\n) {{\n  if (ci {op} \"green\") {{ return `a` }}\n  return `b`;\n}}\n"))])
    };
    let done = |p: &Project, ci: &str| match step(p, &req("main", &[("main.ci", ci)])) {
        StepResponse::Done { instruction, .. } => instruction,
        other => panic!("{other:?}"),
    };
    let eq = make("==");
    assert_eq!(done(&eq, "green"), "a");
    assert_eq!(done(&eq, "red"), "b");
    let ne = make("!=");
    assert_eq!(done(&ne, "green"), "b");
    assert_eq!(done(&ne, "red"), "a");
}

/// `main` awaits `find`. `find` asks `ready`, then ends in text, or hands off to `find/build_list`.
fn await_project() -> Project {
    project(&[
        ("main.rein", "import { find } from \"./find\";\n\ninterface Found {\n  /** The repositories, comma-separated. */\n  repos: string;\n  /** Did the search find none? */\n  none: boolean;\n}\n/** Entry. */\nexport default async function main() {\n  /** The projects. */\n  const found: Found = await find(scope=`all`);\n\n  if (found.none) {\n    return `Nothing to do.`;\n  }\n\n  return `Work on ${found.repos}.`;\n}\n"),
        ("find.rein", "import { buildList } from \"./find/build_list\";\n\ninterface Input {\n  /** Where to search. */\n  scope: string;\n  /** Is the list ready? */\n  ready: boolean;\n}\n/** Find projects. */\nexport default function find({ scope, ready }: Input) {\n  if (!ready) {\n    return buildList;\n  }\n\n  return `List the projects in ${scope}.`;\n}\n"),
        ("find/build_list.rein", "/** Build the list. */\nexport default function buildList() {\n  return `Build the list, then list the projects.`;\n}\n"),
    ])
}

#[test]
fn await_walks_the_callee_then_asks_the_caller_for_its_answer() {
    let p = await_project();
    let r = step(&p, &req("main", &[]));
    let StepResponse::Question { key, trace: t, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "main.found>find.ready");
    assert_eq!(t, trace(&["main", "find"]));

    let r = step(&p, &req("main", &[("main.found>find.ready", "yes")]));
    let StepResponse::Request { key, function, instruction, response, trace: t, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "main.found");
    assert_eq!(function, "main");
    assert_eq!(instruction, "List the projects in all.\n\nThen answer `main.found` with the result.");
    assert_eq!(response.r#type, "struct");
    assert_eq!(t, trace(&["main", "find"]));

    let found = r#"{"repos": "a/b", "none": false}"#;
    let r = step(&p, &req("main", &[("main.found>find.ready", "yes"), ("main.found", found)]));
    let StepResponse::Done { instruction, trace: t, .. } = r else { panic!("{r:?}") };
    assert_eq!(instruction, "Work on a/b.");
    assert_eq!(t, trace(&["main"]));
}

#[test]
fn a_hand_off_inside_the_callee_ends_in_the_caller_request() {
    let p = await_project();
    let r = step(&p, &req("main", &[("main.found>find.ready", "no")]));
    let StepResponse::Request { key, instruction, trace: t, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "main.found");
    assert_eq!(instruction, "Build the list, then list the projects.\n\nThen answer `main.found` with the result.");
    assert_eq!(t, trace(&["main", "find", "find/build_list"]));
}

#[test]
fn a_given_caller_answer_replays_without_the_callee() {
    let p = await_project();
    let r = step(&p, &req("main", &[("main.found", r#"{"repos": "-", "none": true}"#)]));
    let StepResponse::Done { instruction, trace: t, .. } = r else { panic!("{r:?}") };
    assert_eq!(instruction, "Nothing to do.");
    assert_eq!(t, trace(&["main"]));
    let again = step(&p, &req("main", &[("main.found", r#"{"repos": "-", "none": true}"#)]));
    assert_eq!(serde_json::to_string(&again).unwrap(), serde_json::to_string(&step(&p, &req("main", &[("main.found", r#"{"repos": "-", "none": true}"#)]))).unwrap());
}

#[test]
fn an_await_inside_the_callee_asks_the_inner_caller_first() {
    let p = project(&[
        ("main.rein", "import { outer } from \"./outer\";\n\n/** Entry. */\nexport default async function main() {\n  /** Is it done? */\n  const done: boolean = await outer;\n\n  if (done) {\n    return `Close it.`;\n  }\n\n  return `Keep it open.`;\n}\n"),
        ("outer.rein", "import { inner } from \"./outer/inner\";\n\n/** Outer. */\nexport default async function outer() {\n  /** Is it ready? */\n  const ready: boolean = await inner;\n\n  if (ready) {\n    return `Ship it.`;\n  }\n\n  return `Fix it.`;\n}\n"),
        ("outer/inner.rein", "/** Inner. */\nexport default function inner() {\n  return `Check it.`;\n}\n"),
    ]);
    let r = step(&p, &req("main", &[]));
    let StepResponse::Request { key, instruction, trace: t, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "main.done>outer.ready");
    assert_eq!(instruction, "Check it.\n\nThen answer `main.done>outer.ready` with the result.");
    assert_eq!(t, trace(&["main", "outer", "outer/inner"]));

    let r = step(&p, &req("main", &[("main.done>outer.ready", "yes")]));
    let StepResponse::Request { key, instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "main.done");
    assert_eq!(instruction, "Ship it.\n\nThen answer `main.done` with the result.");

    let r = step(&p, &req("main", &[("main.done>outer.ready", "yes"), ("main.done", "no")]));
    let StepResponse::Done { instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!(instruction, "Keep it open.");
}

#[test]
fn await_of_another_program_walks_it_with_its_own_keys() {
    let main = "/** Handler. */\nexport default async function main(\n  /** The PR link. */\n  pr: string,\n) {\n  /** Was the PR merged? */\n  const merged: boolean = await @pr-handling(pr=pr);\n\n  if (merged) {\n    return `Report ${pr} as merged.`;\n  }\n\n  return `Report ${pr} as open.`;\n}\n";
    let p = crate::program_fixture::with_main(main.to_string());
    let pr = [("pr", "u/1")];
    let r = step(&p, &req_with("main", &[], &pr));
    let StepResponse::Question { key, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "main.merged>@pr-handling/main.draft");

    let r = step(&p, &req_with("main", &[("main.merged>@pr-handling/main.draft", "yes")], &pr));
    let StepResponse::Request { key, instruction, trace: t, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "main.merged");
    assert_eq!(instruction, "Skip u/1.\n\nThen answer `main.merged` with the result.");
    assert_eq!(t, trace(&["main", "@pr-handling/main"]));

    let r = step(&p, &req_with("main", &[("main.merged>@pr-handling/main.draft", "yes"), ("main.merged", "no")], &pr));
    let StepResponse::Done { instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!(instruction, "Report u/1 as open.");
}

#[test]
fn each_await_of_the_same_function_gets_its_own_answers() {
    let p = project(&[
        ("main.rein", "import { fn } from \"./fn\";\n\n/** Found. */\ninterface Found {\n  /** Any? */\n  any: boolean;\n}\n\n/** Main. */\nexport default async function main() {\n  /** First? */\n  const a: Found = await fn(subject=`first`);\n\n  /** Second? */\n  const b: Found = await fn(subject=`second`);\n\n  if (a.any) {\n    return `a`;\n  }\n\n  return `b`;\n}\n"),
        ("fn.rein", "/** Fn. */\nexport default async function fn(\n  /** The subject. */\n  subject: string,\n) {\n  /** Inner? */\n  const ok: boolean = await request(`Check ${subject}.`);\n\n  return `Fn ${subject} ends.`;\n}\n"),
    ]);
    let r = step(&p, &req("main", &[]));
    let StepResponse::Request { key, instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!((key.as_str(), instruction.as_str()), ("main.a>fn.ok", "Check first."));

    let given = [("main.a>fn.ok", "true"), ("main.a", r#"{"any": true}"#)];
    let r = step(&p, &req("main", &given));
    let StepResponse::Request { key, instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!((key.as_str(), instruction.as_str()), ("main.b>fn.ok", "Check second."));

    let given = [("main.a>fn.ok", "true"), ("main.a", r#"{"any": true}"#), ("main.b>fn.ok", "false")];
    let r = step(&p, &req("main", &given));
    let StepResponse::Request { key, instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "main.b");
    assert_eq!(instruction, "Fn second ends.\n\nThen answer `main.b` with the result.");
}

#[test]
fn a_recursive_await_asks_each_level_and_ends_when_the_answers_end_it() {
    let p = project(&[(
        "walk.rein",
        "import { walk as walk2 } from \"./walk\";\n\n/** Next item. */\ninterface Next {\n  /** The next item. */\n  item: string;\n}\n\n/** Walk. */\nexport default async function walk(\n  /** The item. */\n  item: string,\n) {\n  /** Is this the last item? */\n  const last: boolean = await request(`Is ${item} the last item?`);\n\n  if (last) {\n    return `Walk ${item} ends.`;\n  }\n\n  /** Next item? */\n  const next: Next = await request(`Pick the item after ${item}.`);\n\n  /** Deeper? */\n  const deeper: boolean = await walk2(item=next.item);\n\n  return `Walk ${item} returned.`;\n}\n",
    )]);
    let top = [("walk.last", "false"), ("walk.next", r#"{"item": "mid"}"#)];
    let r = step(&p, &req_with("walk", &top, &[("item", "top")]));
    let StepResponse::Request { key, instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!((key.as_str(), instruction.as_str()), ("walk.deeper>walk.last", "Is mid the last item?"));

    let mut given = top.to_vec();
    given.extend([("walk.deeper>walk.last", "false"), ("walk.deeper>walk.next", r#"{"item": "end"}"#)]);
    let r = step(&p, &req_with("walk", &given, &[("item", "top")]));
    let StepResponse::Request { key, instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!((key.as_str(), instruction.as_str()), ("walk.deeper>walk.deeper>walk.last", "Is end the last item?"));

    given.push(("walk.deeper>walk.deeper>walk.last", "true"));
    let r = step(&p, &req_with("walk", &given, &[("item", "top")]));
    let StepResponse::Request { key, instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "walk.deeper>walk.deeper");
    assert_eq!(instruction, "Walk end ends.\n\nThen answer `walk.deeper>walk.deeper` with the result.");

    given.push(("walk.deeper>walk.deeper", "true"));
    let r = step(&p, &req_with("walk", &given, &[("item", "top")]));
    let StepResponse::Request { key, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "walk.deeper");

    given.push(("walk.deeper", "true"));
    let r = step(&p, &req_with("walk", &given, &[("item", "top")]));
    let StepResponse::Done { instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!(instruction, "Walk top returned.");
}

#[test]
fn a_key_with_a_call_scope_that_is_not_an_answer_is_unknown() {
    let p = await_project();
    let r = step(&p, &req("main", &[("main.nope>find.ready", "yes")]));
    let StepResponse::Error { key, message, .. } = r else { panic!("{r:?}") };
    assert_eq!(key.as_deref(), Some("main.nope>find.ready"));
    assert!(message.contains("unknown key"), "{message}");
}

#[test]
fn a_prefixed_key_for_a_call_the_walk_never_makes_is_unknown() {
    let p = project(&[
        ("main.rein", "import { fn } from \"./fn\";\n\n/** Main. */\nexport default async function main() {\n  /** First? */\n  const a: boolean = await fn;\n\n  if (a) {\n    return `a`;\n  }\n\n  return `b`;\n}\n"),
        ("fn.rein", "import { leaf } from \"./leaf\";\n\n/** Fn. */\nexport default async function fn() {\n  /** Inner? */\n  const ok: boolean = await leaf;\n\n  return `Fn ends.`;\n}\n"),
        ("leaf.rein", "/** Leaf. */\nexport default function leaf(\n  /** Leaf ok? */\n  ok: boolean,\n) {\n  if (ok) {\n    return `Leaf ok.`;\n  }\n\n  return `Leaf not ok.`;\n}\n"),
    ]);
    for key in ["main.a>leaf.ok", "main.a>fn.ok>fn.ok", "leaf.ok>leaf.ok"] {
        let r = step(&p, &req("main", &[(key, "yes")]));
        let StepResponse::Error { key: k, message, .. } = r else { panic!("{key}: {r:?}") };
        assert_eq!(k.as_deref(), Some(key));
        assert!(message.contains("unknown key"), "{message}");
    }
    let r = step(&p, &req("main", &[("main.a>fn.ok>leaf.ok", "yes")]));
    let StepResponse::Request { key, instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "main.a>fn.ok");
    assert!(instruction.starts_with("Leaf ok."), "{instruction}");
}

#[test]
fn the_first_call_of_a_prefixed_key_must_be_reached_from_the_entry() {
    // The review's p1: `main` awaits `fn` as `a`; `fn` awaits `leaf` as `ok`.
    let leaf = "/** Leaf. */\nexport default function leaf(\n  /** Leaf ok? */\n  ok: boolean,\n) {\n  if (ok) {\n    return `Leaf ok.`;\n  }\n\n  return `Leaf not ok.`;\n}\n";
    let p1 = project(&[
        ("main.rein", "import { fn } from \"./fn\";\n\n/** Main. */\nexport default async function main() {\n  /** First? */\n  const a: boolean = await fn;\n\n  if (a) {\n    return `a`;\n  }\n\n  return `b`;\n}\n"),
        ("fn.rein", "import { leaf } from \"./leaf\";\n\n/** Fn. */\nexport default async function fn() {\n  /** Inner? */\n  const ok: boolean = await leaf;\n\n  return `Fn ends.`;\n}\n"),
        ("leaf.rein", leaf),
    ]);
    let r = step(&p1, &req("main", &[("fn.ok>leaf.ok", "yes")]));
    let StepResponse::Error { key, message, .. } = r else { panic!("{r:?}") };
    assert_eq!(key.as_deref(), Some("fn.ok>leaf.ok"));
    assert!(message.contains("unknown key"), "{message}");

    // The review's p2: `fn` hands off to `other`, which awaits `leaf`. `other` runs only inside the call `main.a`.
    let p2 = project(&[
        ("main.rein", "import { fn } from \"./fn\";\n\n/** Main. */\nexport default async function main() {\n  /** A? */\n  const a: boolean = await fn;\n\n  /** B? */\n  const b: boolean = await fn;\n\n  if (a) {\n    return `a`;\n  }\n\n  return `b`;\n}\n"),
        ("fn.rein", "import { other } from \"./other\";\n\n/** Fn. */\nexport default function fn() {\n  return other;\n}\n"),
        ("other.rein", "import { leaf } from \"./leaf\";\n\n/** Other. */\nexport default async function other() {\n  /** Other ok? */\n  const ok: boolean = await leaf;\n\n  return `Other ends.`;\n}\n"),
        ("leaf.rein", leaf),
    ]);
    for bad in ["fn.ok>leaf.ok", "other.ok>leaf.ok"] {
        let r = step(&p2, &req("main", &[(bad, "yes")]));
        let StepResponse::Error { key, message, .. } = r else { panic!("{bad}: {r:?}") };
        assert_eq!(key.as_deref(), Some(bad));
        assert!(message.contains("unknown key"), "{message}");
    }
    // A hand-off chain inside the callee is a valid call path.
    let r = step(&p2, &req("main", &[("main.a>other.ok>leaf.ok", "yes")]));
    let StepResponse::Request { key, instruction, .. } = r else { panic!("{r:?}") };
    assert_eq!(key, "main.a>other.ok");
    assert!(instruction.starts_with("Leaf ok."), "{instruction}");
}

#[test]
fn self_await_without_a_question_fails_at_the_hop_limit() {
    // Matches the checker warning for `a -> a` with no question before the await.
    let project = project(&[
        ("main.rein", "import { a } from \"./a\";\n\n/** Entry. */\nexport default function main() {\n  return a;\n}\n"),
        ("a.rein", "import { a as a2 } from \"./a\";\n\n/** d */\nexport default async function a() {\n  /** Again? */\n  const again: boolean = await a2;\n  return `done`;\n}\n"),
    ]);
    let StepResponse::Error { message, .. } = step(&project, &req("main", &[])) else {
        panic!("expected the hop limit error")
    };
    assert_eq!(message, "process did not finish in 64 steps (reference cycle?)");
}

#[test]
fn self_await_after_a_question_asks_at_each_level() {
    // The question comes before the await, so the first step asks the agent.
    let project = project(&[
        ("main.rein", "import { a } from \"./a\";\n\n/** Entry. */\nexport default function main() {\n  return a;\n}\n"),
        ("a.rein", "import { a as a2 } from \"./a\";\n\n/** d */\nexport default async function a(\n  /** Keep going? */\n  more: boolean,\n) {\n  if (!more) {\n    return `stopped`;\n  }\n  /** Again? */\n  const again: boolean = await a2;\n  return `done`;\n}\n"),
    ]);
    let StepResponse::Question { key, .. } = step(&project, &req("main", &[])) else {
        panic!("expected a question")
    };
    assert_eq!(key, "a.more");
}

#[test]
fn a_passed_boolean_binds_and_the_callee_does_not_ask() {
    let project = project(&[
        ("main.rein", "import { two } from \"./two\";\n\n/** Entry. */\nexport default function main(\n  /** Keep order? */\n  keep: boolean,\n) {\n  return two(ok=keep);\n}\n"),
        ("two.rein", "/** Two. */\nexport default function two(\n  /** Keep order? */\n  ok: boolean,\n) {\n  if (ok) {\n    return `kept`;\n  }\n  return `swapped`;\n}\n"),
    ]);
    let res = step(&project, &req("main", &[("main.keep", "yes")]));
    let StepResponse::Done { instruction, trace, .. } = res else {
        panic!("{res:?}")
    };
    assert_eq!((instruction.as_str(), trace), ("kept", vec!["main".to_string(), "two".to_string()]));
    let literal = project_with_main_body(&project, "return @./two(ok=false);");
    let res = step(&literal, &req("main", &[]));
    let StepResponse::Done { instruction, .. } = res else { panic!("{res:?}") };
    assert_eq!(instruction, "swapped");
}

/// `project` with `main.rein` replaced by an entry without parameters whose body is `body`.
fn project_with_main_body(project: &Project, body: &str) -> Project {
    let two = project.get("two").unwrap().text.clone();
    let main = format!("/** Entry. */\nexport default function main() {{\n  {body}\n}}\n");
    self::project(&[("main.rein", &main), ("two.rein", &two)])
}

const MERGE_CI: &str = "/** Merge. */\nexport default function merge(\n  /** CI? */\n  ci: \"green\" | \"red\",\n) {\n  switch (ci) {\n    case \"green\":\n      return `Merge it.`;\n    case \"red\":\n      return `Fix CI.`;\n  }\n}\n";

#[test]
fn a_passed_union_binds_through_an_await() {
    let main = "import { merge } from \"./merge\";\n\n/** Entry. */\nexport default async function main() {\n  /** CI? */\n  const ci: \"green\" | \"red\" = await request(`Read CI.`);\n  /** Merged? */\n  const merged: boolean = await merge(ci=ci);\n  if (merged) {\n    return `done`;\n  }\n  return `not merged`;\n}\n";
    let project = project(&[("main.rein", main), ("merge.rein", MERGE_CI)]);
    let res = step(&project, &req("main", &[("main.ci", "red")]));
    let StepResponse::Request { key, instruction, .. } = res else { panic!("{res:?}") };
    assert_eq!(key, "main.merged");
    assert!(instruction.starts_with("Fix CI."), "{instruction}");
}

#[test]
fn struct_fields_pass_to_another_program_through_step() {
    let main = "interface Pick {\n  /** The PR link. */\n  pr: string;\n  /** Which task. */\n  task: \"review\" | \"merge\";\n}\n/** Entry. */\nexport default async function main() {\n  /** The next PR. */\n  const pick: Pick = await request(`pick a PR`);\n  return @pr-handling/main(pr=pick.pr, draft=false, task=pick.task);\n}\n";
    let p = crate::program_fixture::with_main(main.to_string());
    let res = step(&p, &req("main", &[("main.pick", r#"{"pr":"L","task":"merge"}"#)]));
    let StepResponse::Done { instruction, trace, .. } = res else { panic!("{res:?}") };
    assert_eq!(instruction, "Do merge on L.");
    assert_eq!(trace, ["main", "@pr-handling/main"]);
}

/// After the flip of `MISSING_TYPED_ARG`, a hand-off back to `main` must pass `ready`. The passed value wins over
/// the stored answer `main.ready`.
#[test]
fn a_hand_off_back_passes_the_value_and_it_beats_the_stored_answer() {
    let main = "import { fix } from \"./fix\";\n\n/** Entry. */\nexport default function main(\n  /** Ready? */\n  ready: boolean,\n) {\n  if (ready) {\n    return `done`;\n  }\n  return fix;\n}\n";
    let fix = |back: &str| format!("/** Fix. */\nexport default async function fix() {{\n  /** Ready now? */\n  const ready: boolean = await request(`Fix it.`);\n  if (ready) {{\n    return {back};\n  }}\n  return `give up`;\n}}\n");
    let passing = project(&[("main.rein", main), ("fix.rein", &fix("@./main(ready=ready)"))]);
    assert!(passing.check().values().flatten().all(|d| d.severity != crate::diagnostic::Severity::Error && !d.message.contains("missing")));
    let res = step(&passing, &req("main", &[("main.ready", "no"), ("fix.ready", "yes")]));
    let StepResponse::Done { instruction, trace, .. } = res else { panic!("{res:?}") };
    assert_eq!((instruction.as_str(), trace.len()), ("done", 3));

    let bare = project(&[("main.rein", main), ("fix.rein", &fix("@./main"))]);
    let diags: Vec<_> = bare.check().into_values().flatten().filter(|d| d.severity == crate::check::MISSING_TYPED_ARG).collect();
    assert!(diags.iter().any(|d| d.message.contains("`@./main` takes 1 argument, got 0; pass `ready`")), "{diags:?}");
}

/// A hand-off passes a caller union into a wider union of the target. The target switches on it and does not
/// ask for it; a union it was not passed is still a question.
#[test]
fn a_passed_union_is_not_a_question_and_an_unpassed_one_is() {
    let main = "import { go } from \"./go\";\n\n/** Entry. */\nexport default function main(\n  /** Which account? */\n  account: \"work\" | \"home\",\n) {\n  return go(account=account);\n}\n";
    let go = "import { merge } from \"./merge\";\n\ninterface Job {\n  /** Which account? */\n  account: \"work\" | \"home\" | \"shared\";\n  /** CI? */\n  ci: \"green\" | \"red\";\n}\n/** Go. */\nexport default function go({ account, ci }: Job) {\n  switch (account) {\n    case \"work\":\n      return `work`;\n    case \"home\":\n    case \"shared\":\n      return merge(ci=ci);\n  }\n}\n";
    let project = project(&[("main.rein", main), ("go.rein", go), ("merge.rein", MERGE_CI)]);
    let res = step(&project, &req("main", &[("main.account", "work")]));
    let StepResponse::Done { instruction, answers, .. } = res else { panic!("{res:?}") };
    assert_eq!((instruction.as_str(), answers.len()), ("work", 1));
    let res = step(&project, &req("main", &[("main.account", "home")]));
    let StepResponse::Question { key, .. } = res else { panic!("{res:?}") };
    assert_eq!(key, "go.ci");
}

const TICKET_ENTRY: &str = "import { Ticket } from \"./ticket.schema\";\n\ninterface Input {\n  /** The ticket. */\n  ticket: Ticket;\n}\n/** Entry. */\nexport default function main({ ticket }: Input) {\n  return delegate(model=`x`, task=`Fix ${ticket}.`);\n}\n";

#[test]
fn schema_entry_input_must_match_the_schema() {
    let p = project(&[("main.rein", TICKET_ENTRY), ("ticket.schema.json", TICKET_SCHEMA)]);
    let res = step(&p, &req_with("main", &[], &[("ticket", "ABC-123")]));
    let StepResponse::Done { instruction, .. } = res else { panic!("not done: {res:?}") };
    assert!(instruction.ends_with("Task: Fix ABC-123."), "{instruction}");
    let res = step(&p, &req_with("main", &[], &[("ticket", "abc")]));
    let StepResponse::Error { message, .. } = res else { panic!("not an error: {res:?}") };
    assert!(message.contains("does not match `Ticket`"), "{message}");
}

#[test]
fn struct_answer_field_of_a_schema_type_must_match_the_schema() {
    let picker = "import { Ticket } from \"./ticket.schema\";\n\ninterface Pick {\n  /** The ticket. */\n  ticket: Ticket;\n}\n/** Entry. */\nexport default async function main() {\n  /** The ticket to fix. */\n  const pick: Pick = await request(`pick a ticket`);\n  return `Fix ${pick.ticket}.`;\n}\n";
    let p = project(&[("main.rein", picker), ("ticket.schema.json", TICKET_SCHEMA)]);
    let StepResponse::Error { message, key, .. } = step(&p, &req("main", &[("main.pick", "{\"ticket\":\"abc\"}")])) else {
        panic!("not an error")
    };
    assert!(message.contains("does not match `Ticket`"), "{message}");
    assert_eq!(key.as_deref(), Some("main.pick"));
    let StepResponse::Done { instruction, .. } = step(&p, &req("main", &[("main.pick", "{\"ticket\":\"ABC-9\"}")])) else {
        panic!("not done")
    };
    assert_eq!(instruction, "Fix ABC-9.");
}

#[test]
fn schema_type_field_of_a_shared_struct_checks_the_value() {
    let struct_file = "import { Ticket } from \"./ticket.schema\";\n\n/** The input. */\nexport default interface Input {\n  /** The ticket. */\n  ticket: Ticket;\n}\n";
    let main = "import { Input } from \"./input.struct\";\n\n/** Entry. */\nexport default function main({ ticket }: Input) {\n  return delegate(model=`x`, task=`Fix ${ticket}.`);\n}\n";
    let p = project(&[
        ("main.rein", main),
        ("input.struct.rein", struct_file),
        ("ticket.schema.json", TICKET_SCHEMA),
    ]);
    let res = step(&p, &req_with("main", &[], &[("ticket", "ABC-123")]));
    let StepResponse::Done { instruction, .. } = res else { panic!("not done: {res:?}") };
    assert!(instruction.ends_with("Task: Fix ABC-123."), "{instruction}");
    let res = step(&p, &req_with("main", &[], &[("ticket", "abc")]));
    let StepResponse::Error { message, .. } = res else { panic!("not an error: {res:?}") };
    assert!(message.contains("does not match `Ticket`"), "{message}");
}

#[test]
fn hand_off_to_a_schema_parameter_checks_the_value() {
    // `main` passes its string input to `target`, which takes a `Ticket`. The hand-off checks the passed value.
    let main = "import { target } from \"./target\";\n\n/** Entry. */\nexport default function main(\n  /** The ticket. */\n  s: string,\n) {\n  return target(ticket=s);\n}\n";
    let target = "import { Ticket } from \"./ticket.schema\";\n\n/** Fix a ticket. */\nexport default function target(\n  /** The ticket. */\n  ticket: Ticket,\n) {\n  return delegate(model=`x`, task=`Fix ${ticket}.`);\n}\n";
    let p = project(&[
        ("main.rein", main),
        ("target.rein", target),
        ("ticket.schema.json", TICKET_SCHEMA),
    ]);
    let res = step(&p, &req_with("main", &[], &[("s", "ABC-123")]));
    let StepResponse::Done { instruction, .. } = res else { panic!("not done: {res:?}") };
    assert!(instruction.ends_with("Task: Fix ABC-123."), "{instruction}");
    let res = step(&p, &req_with("main", &[], &[("s", "abc")]));
    let StepResponse::Error { message, .. } = res else { panic!("not an error: {res:?}") };
    assert!(message.contains("does not match `Ticket`"), "{message}");
}

const DATE_ANSWER: &str = "interface Date {\n  /** The day. */\n  day: string;\n}\n/** Entry. */\nexport default async function main() {\n  /** The day to plan. */\n  const d: Date = await request(`pick a day`);\n  return `Plan ${d.day}.`;\n}\n";

#[test]
fn local_interface_named_like_a_format_shadows_the_format() {
    let p = crate::program_fixture::with_main(DATE_ANSWER.to_string());
    let StepResponse::Done { instruction, .. } = step(&p, &req("main", &[("main.d", "{\"day\":\"Monday\"}")])) else { panic!("not done") };
    assert_eq!(instruction, "Plan Monday.");
}

#[test]
fn imported_struct_named_like_a_format_is_a_struct() {
    let main = "import { Email } from \"./email.struct\";\n\n/** Entry. */\nexport default async function main() {\n  /** The address to write to. */\n  const e: Email = await request(`pick an address`);\n  return `Write ${e.address}.`;\n}\n";
    let shared = "/** A mail address. */\nexport default interface Email {\n  /** The address. */\n  address: string;\n}\n";
    let p = project(&[("main.rein", main), ("email.struct.rein", shared)]);
    let StepResponse::Done { instruction, .. } = step(&p, &req("main", &[("main.e", "{\"address\":\"a@b.c\"}")])) else { panic!("not done") };
    assert_eq!(instruction, "Write a@b.c.");
}

#[test]
fn format_without_a_local_declaration_still_checks_its_value() {
    let src = "interface Input {\n  /** The day. */\n  day: Date;\n}\n/** Entry. */\nexport default function main({ day }: Input) {\n  return delegate(model=`x`, task=`Plan ${day}.`);\n}\n";
    let p = project(&[("main.rein", src)]);
    let StepResponse::Error { message, .. } = step(&p, &req_with("main", &[], &[("day", "2026-13-45")])) else { panic!("not an error") };
    assert!(message.contains("`day`"), "{message}");
    let StepResponse::Done { instruction, .. } = step(&p, &req_with("main", &[], &[("day", "2026-11-23")])) else { panic!("not done") };
    assert!(instruction.ends_with("Task: Plan 2026-11-23."), "{instruction}");
}
