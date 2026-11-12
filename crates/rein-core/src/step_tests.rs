use super::*;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Writes `files` into a unique temp project, loads it, and removes the directory.
fn project(files: &[(&str, &str)]) -> Project {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("rein-step-{}-{nanos}-{n}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for (rel, text) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let loaded = Project::load(&root).unwrap();
    fs::remove_dir_all(&root).unwrap();
    loaded
}

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
    }
}

fn trace(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// `main` asks for `vip`, then hands off to `escalate` or returns text.
fn vip_project() -> Project {
    project(&[
        ("main.rein", "/** Entry. */\nexport default function main(\n  /** Is the customer on the VIP plan? */\n  vip: boolean,\n) {\n  if (vip) {\n    return @./escalate;\n  }\n  return \"Answer the ticket.\";\n}\n"),
        ("escalate.rein", "/** Escalate. */\nexport default function escalate() {\n  return \"Assign the ticket to tier 2.\";\n}\n"),
    ])
}

/// `main` picks a path. `merge` goes to `merge_pr`, which asks two questions.
fn merge_project() -> Project {
    project(&[
        ("main.rein", "/** Entry. */\nexport default function main(\n  /** Which path? */\n  path: \"merge\" | \"review\",\n) {\n  switch (path) {\n    case \"merge\":\n      return @./merge_pr;\n    case \"review\":\n      return \"Review the code.\";\n  }\n}\n"),
        ("merge_pr.rein", "interface Input {\n  /** Does the PR change only docs? */\n  docsOnly: boolean;\n  /** What is the CI status? */\n  ci: \"green\" | \"red\";\n}\n/** Merge a PR. */\nexport default function mergePr({ docsOnly, ci }: Input) {\n  if (docsOnly) {\n    return \"approved\";\n  }\n  switch (ci) {\n    case \"green\":\n      return \"merge it\";\n    case \"red\":\n      return \"wait for CI\";\n  }\n}\n"),
    ])
}

/// `main` asks `articleFound` through `answer/faq`, which asks a request, then escalates.
fn faq_project() -> Project {
    project(&[
        ("main.rein", "/** Entry. */\nexport default function main() {\n  return @./answer/faq;\n}\n"),
        ("answer/faq.rein", "/** Answer a how-to question. */\nexport default async function answerFaq(\n  /** Did you find an article? */\n  articleFound: boolean,\n) {\n  if (!articleFound) {\n    return @../escalate;\n  }\n  /** Did the article solve it? */\n  const solved: boolean = await request(\n    \"Send the article and ask if it solved the problem.\",\n  );\n  if (solved) {\n    return \"Close the ticket as solved.\";\n  }\n  return @../escalate;\n}\n"),
        ("escalate.rein", "/** Escalate. */\nexport default function escalate() {\n  return \"Assign the ticket to tier 2.\";\n}\n"),
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
            key: "merge_pr.docsOnly".into(),
            function: "merge_pr".into(),
            name: "docsOnly".into(),
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
            &[("main.path", "merge"), ("merge_pr.docsOnly", "no")],
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
                ("merge_pr.docsOnly", "no"),
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
                ("merge_pr.docsOnly", "no"),
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
        &req("main", &[("answer/faq.articleFound", "yes")]),
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
            answers: answers(&[("answer/faq.articleFound", "yes")]),
            trace: trace(&["main", "answer/faq"]),
        }
    );

    let res = step(
        &project,
        &req(
            "main",
            &[
                ("answer/faq.articleFound", "yes"),
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
                ("answer/faq.articleFound", "yes"),
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
                ("answer/faq.articleFound", "yes"),
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
        ("main.rein", "/** Entry. */\nexport default function main() {\n  return @./greet(who=\"world\");\n}\n"),
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
        ("main.rein", "/** Entry. */\nexport default function main(\n  /** CI? */\n  ci: \"green\" | \"red\",\n) {\n  if (ci === \"green\") {\n    return @./merge_pr;\n  }\n  return \"Wait.\";\n}\n"),
        ("merge_pr.rein", "/** Merge. */\nexport default function mergePr(\n  /** CI for the merge? */\n  ci: \"green\" | \"red\",\n) {\n  switch (ci) {\n    case \"green\":\n      return `Merge ${ci}`;\n    case \"red\":\n      return \"Stop.\";\n  }\n}\n"),
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
            "/** Entry. */\nexport default function main() {\n  return @./a;\n}\n",
        ),
        (
            "a.rein",
            "/** A. */\nexport default function a() {\n  return @./b;\n}\n",
        ),
        (
            "b.rein",
            "/** B. */\nexport default function b() {\n  return @./a;\n}\n",
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
        "flow did not finish in 64 steps (reference cycle?)"
    );
    assert_eq!(t.len(), HOP_LIMIT);
}

#[test]
fn template_as_entry_is_an_error() {
    let project = project(&[
        ("main.rein", "/** Entry. */\nexport default function main() {\n  return \"x\";\n}\n"),
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
        ("main.rein", "/** Entry. */\nexport default function main() {\n  return @./tpl(a=\"Hello\");\n}\n"),
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
        ("main.rein", "/** Entry. */\nexport default function main() {\n  return delegate(model=@./tpl(a=\"x\"), task=\"task\");\n}\n"),
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
        &[("main.path", "merge"), ("merge_pr.docsOnly", "no")],
    );
    let mut backward = req("main", &[]);
    backward
        .answers
        .insert("merge_pr.docsOnly".into(), "no".into());
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
        ("main.rein", "interface Input {\n  /** The link. */\n  pr: string;\n  /** Draft? */\n  draft: boolean;\n}\n/** Entry. */\nexport default function main({ pr, draft }: Input) {\n  if (draft) {\n    return `Stop ${pr}.`;\n  }\n  return @./merge(pr=pr);\n}\n"),
        ("merge.rein", "interface Input {\n  /** The link. */\n  pr: string;\n  /** CI? */\n  ci: \"green\" | \"red\";\n}\n/** Merge. */\nexport default function merge({ pr, ci }: Input) {\n  switch (ci) {\n    case \"green\":\n      return `Merge ${pr}.`;\n    case \"red\":\n      return \"Wait.\";\n  }\n}\n"),
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
        ("main.rein", "interface Input {\n  /** A */\n  a: string;\n  /** B */\n  b: string;\n}\n/** Entry. */\nexport default function main({ a, b }: Input) {\n  return @./two(y=a, x=b);\n}\n"),
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

const PICKER: &str = "interface Pick {\n  /** The PR link. */\n  pr: string;\n  /** Which task. */\n  task: \"review\" | \"merge\";\n}\n/** Entry. */\nexport default async function main() {\n  /** The pull request of the next notification. */\n  const pick: Pick = await request(\"pick a PR\");\n  return @pr-handling/main(pr=pick.pr);\n}\n";

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
fn struct_answer_is_validated_field_by_field() {
    let p = crate::program_fixture::with_main(PICKER.to_string());
    let cases = [
        ("{\"task\":\"merge\"}", "missing field `pr`"),
        ("{\"pr\":\"u/1\",\"task\":\"merge\",\"x\":\"1\"}", "`x` is not a field of `Pick`; fields are pr, task"),
        ("{\"pr\":\"u/1\",\"task\":\"nope\"}", "invalid value `nope` for `pick.task`"),
        ("{\"pr\":\"\",\"task\":\"merge\"}", "field `pick.pr` is empty"),
        ("{\"pr\":1,\"task\":\"merge\"}", "field `pick.pr` must be a string"),
        ("not json", "must be a JSON object"),
    ];
    for (raw, part) in cases {
        let StepResponse::Error { message, key, .. } = step(&p, &req("main", &[("main.pick", raw)])) else { panic!("not an error: {raw}") };
        assert!(message.contains(part), "{raw}: {message}");
        assert_eq!(key.as_deref(), Some("main.pick"));
    }
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
    let main = "/** Entry. */\nexport default async function main() {\n  /** The pull request of the next notification. */\n  const pick: @./pick = await request(\"pick a PR\");\n  if (pick.urgent) {\n    return `Hurry ${pick.pr}.`;\n  }\n  return `Handle ${pick.pr}.`;\n}\n";
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
