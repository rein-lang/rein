use super::*;
use std::path::PathBuf;

fn project(files: &[(&str, &str)]) -> Project {
    let root = PathBuf::from("/p");
    Project::from_sources(
        root.clone(),
        files
            .iter()
            .map(|(n, t)| (root.join(n), t.to_string()))
            .collect(),
    )
}

fn one(body: &str, params: &str) -> Project {
    project(&[(
        "main.rein",
        &format!("/** M. */\nexport default function main(\n{params}) {{\n{body}\n}}\n"),
    )])
}

#[test]
fn negation_swaps_yes_and_no() {
    let p = one(
        "  if (!ready) {\n    return `wait`;\n  }\n  return `go`;",
        "  /** Ready? */\n  ready: boolean,\n",
    );
    let g = flow(&p, "main");
    let edges: Vec<_> = g
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str(), e.label.as_deref()))
        .collect();
    assert_eq!(
        edges,
        vec![
            ("f0", "f0_0", None),
            ("f0_0", "f0_1", Some("no")),
            ("f0_0", "f0_2", Some("yes"))
        ]
    );
    assert_eq!(g.nodes[1].label, "Ready?\nready: boolean");
    assert_eq!(g.nodes[2].label, "wait");
}

#[test]
fn compare_labels_the_other_members() {
    let p = one(
        "  if (c !== \"x\") {\n    return `a`;\n  }\n  return `b`;",
        "  /** C? */\n  c: \"x\" | \"y\" | \"z\",\n",
    );
    let g = flow(&p, "main");
    assert_eq!(g.edges[1].label.as_deref(), Some("y / z"));
    assert_eq!(g.edges[2].label.as_deref(), Some("x"));
}

#[test]
fn compound_condition_is_one_condition_node() {
    let p = one(
        "  if (a && (b || !a)) {\n    return `one`;\n  }\n  return `two`;",
        "  /** A? */\n  a: boolean,\n  /** B? */\n  b: boolean,\n",
    );
    let g = flow(&p, "main");
    assert_eq!(g.nodes[1].kind, NodeKind::Condition);
    assert_eq!(g.nodes[1].label, "a && (b || !a)");
}

#[test]
fn unresolved_reference_is_its_own_node() {
    let p = one("  return @./gone;", "");
    let g = flow(&p, "main");
    assert_eq!(g.nodes[1].kind, NodeKind::Unresolved);
    assert!(mermaid(&g).contains("classDef unresolved"));
}

#[test]
fn mermaid_escapes_quotes_hashes_and_backticks() {
    let p = one("  return \"say \\\"hi\\\" #1 `x`\";", "");
    let text = mermaid(&flow(&p, "main"));
    assert!(
        text.contains("say #quot;hi#quot; #35;1 \u{2cb}x\u{2cb}"),
        "{text}"
    );
}

#[test]
fn a_backtick_in_a_question_with_a_line_break_has_no_markdown_code_span() {
    // Mermaid turns a backtick pair into a code span, which fails in a label with <br/>. The label must not hold one.
    let p = one(
        "  if (account == \"work\") {\n    return `a`;\n  }\n  return `b`;",
        "  /** Run `gh api user --jq .login` first? */\n  account: \"private\" | \"work\",\n",
    );
    let text = mermaid(&flow(&p, "main"));
    let label = text.lines().find(|l| l.contains("<br/>")).unwrap_or_default();
    assert!(label.contains("gh api user"), "{text}");
    assert!(!label.contains('`') && !label.contains("#96;"), "{text}");
}

#[test]
fn program_hand_off_is_an_edge_to_a_program_node() {
    let p = crate::program_fixture::with_program("return @pr-handling/main(pr=pr);");
    let g = flow(&p, "main");
    let node = g.nodes.iter().find(|n| n.kind == NodeKind::Program).expect("program node");
    assert_eq!(node.label, "@pr-handling/main");
    let edge = g.edges.iter().find(|e| e.to == node.id).expect("edge");
    assert_eq!(edge.kind, EdgeKind::Handoff);
    assert_eq!((edge.args[0].name.as_str(), edge.args[0].value.as_str()), ("pr", "pr"));
    let r = refs(&p, "main");
    assert!(r.nodes.iter().any(|n| n.kind == NodeKind::Program && n.label == "@pr-handling/main"));
    assert!(r.edges.iter().any(|e| e.kind == EdgeKind::Handoff && e.from == "f0"));
}

#[test]
fn double_equals_is_labelled_like_triple() {
    let params = "  /** C? */\n  c: \"x\" | \"y\" | \"z\",\n";
    let triple = one("  if (c === \"x\") {\n    return \"a\";\n  }\n  return \"b\";", params);
    let double = one("  if (c == \"x\") {\n    return \"a\";\n  }\n  return \"b\";", params);
    let labels = |p: &Project| flow(p, "main").nodes.iter().map(|n| n.label.clone()).collect::<Vec<_>>();
    assert_eq!(labels(&double), labels(&triple));
}

#[test]
fn walk_follows_a_compound_condition_and_stops_at_a_missing_answer() {
    let p = project(&[(
        "main.rein",
        "interface Input {\n  /** A? */\n  a: boolean;\n  /** B? */\n  b: boolean;\n}\n/** M. */\nexport default function main({ a, b }: Input) {\n  if (a && b) {\n    return `one`;\n  }\n  return `two`;\n}\n",
    )]);
    let mut req = crate::step::StepRequest {
        version: crate::step::PROTOCOL_VERSION,
        entry: "main".into(),
        answers: Default::default(),
        flow: None,
        inputs: Default::default(),
    };
    req.answers.insert("main.a".into(), "yes".into());
    req.answers.insert("main.b".into(), "no".into());
    let g = walk(&p, "main", &crate::step::step(&p, &req));
    assert_eq!(g.path, ["f0", "f0_0", "f0_2"]);
    let current = g.current.expect("current");
    assert_eq!((current.node.as_str(), current.kind), ("f0_2", "done"));
    assert_eq!(g.edges.iter().filter(|e| e.taken).count(), 2);

    req.answers.remove("main.b");
    let g = walk(&p, "main", &crate::step::step(&p, &req));
    assert_eq!(g.path, ["f0", "f0_0"]);
    assert_eq!(g.current.expect("current").key.as_deref(), Some("main.b"));
}

#[test]
fn await_is_a_request_node_with_a_call_edge_and_the_flow_goes_on() {
    let p = project(&[
        ("main.rein", "/** M. */\nexport default async function main() {\n  /** Done? */\n  const done: boolean = await @./work(task=`t`);\n\n  if (done) {\n    return `close`;\n  }\n\n  return `open`;\n}\n"),
        ("work.rein", "interface Input {\n  /** The task. */\n  task: string;\n  /** Ready? */\n  ready: boolean;\n}\n/** W. */\nexport default function work({ task, ready }: Input) {\n  if (ready) {\n    return `do ${task}`;\n  }\n\n  return `wait`;\n}\n"),
    ]);
    let g = flow(&p, "main");
    let node = g.nodes.iter().find(|n| n.kind == NodeKind::Request).expect("await node");
    assert_eq!(node.label, "await @./work\ndone: boolean");
    let call = g.edges.iter().find(|e| e.kind == EdgeKind::Call).expect("call edge");
    assert_eq!((call.from.as_str(), call.to.as_str()), (node.id.as_str(), "f1"));
    assert_eq!((call.args[0].name.as_str(), call.args[0].value.as_str()), ("task", "`t`"));
    assert!(g.edges.iter().any(|e| e.from == node.id && e.kind == EdgeKind::Flow), "the flow goes on from the answer");
    assert!(mermaid(&g).contains(&format!("{} ==>", node.id)), "{}", mermaid(&g));
    assert!(dot(&g).contains("style=bold"), "{}", dot(&g));
}

#[test]
fn walk_marks_the_call_that_step_walks_when_one_function_is_called_twice() {
    let p = project(&[
        ("main.rein", "/** Found. */\ninterface Found {\n  /** Any? */\n  any: boolean;\n}\n\n/** Main. */\nexport default async function main() {\n  /** First? */\n  const a: Found = await @./fn(subject=`first`);\n\n  /** Second? */\n  const b: Found = await @./fn(subject=`second`);\n\n  if (a.any) {\n    return `a`;\n  }\n\n  return `b`;\n}\n"),
        ("fn.rein", "/** Fn. */\nexport default async function fn(\n  /** The subject. */\n  subject: string,\n) {\n  /** Inner? */\n  const ok: boolean = await @./leaf;\n\n  return `Fn ${subject} ends.`;\n}\n"),
        ("leaf.rein", "/** Leaf. */\nexport default function leaf(\n  /** Leaf ok? */\n  ok: boolean,\n) {\n  if (ok) {\n    return `Leaf ok.`;\n  }\n\n  return `Leaf not ok.`;\n}\n"),
    ]);
    // The text `rein step` gives, and the label of the node the graph marks current.
    let walk_of = |pairs: &[(&str, &str)]| {
        let mut req = crate::step::StepRequest {
            version: crate::step::PROTOCOL_VERSION,
            entry: "main".into(),
            answers: Default::default(),
            flow: None,
            inputs: Default::default(),
        };
        for (k, v) in pairs {
            req.answers.insert(k.to_string(), v.to_string());
        }
        let response = crate::step::step(&p, &req);
        let StepResponse::Request { instruction, .. } = &response else { panic!("{response:?}") };
        let text = instruction.lines().next().unwrap_or_default().to_string();
        let g = walk(&p, "main", &response);
        let label = |id: &str| g.nodes.iter().find(|n| n.id == id).unwrap().label.clone();
        let current = label(&g.current.as_ref().expect("current").node);
        // The last outcome on the path is the text the callee ended with.
        let ended = g.path.iter().map(|id| label(id)).rfind(|l| l.starts_with("Leaf")).unwrap_or_default();
        assert_eq!(current, "await @./leaf\nok: boolean", "the request is asked at the caller's await");
        (text, ended)
    };
    // The walk is the call `main.a`. The answer of `main.b` sorts last, but the walk does not reach it.
    let (text, label) = walk_of(&[("main.a>fn.ok>leaf.ok", "yes"), ("main.b>fn.ok>leaf.ok", "no")]);
    assert_eq!((text.as_str(), label.as_str()), ("Leaf ok.", "Leaf ok."));
    // The review's answers: `main.a` is answered, so the walk is the call `main.b`.
    let review = [
        ("main.a>fn.ok>leaf.ok", "yes"),
        ("main.b>fn.ok>leaf.ok", "no"),
        ("main.a>fn.ok", "true"),
        ("main.a", r#"{"any": true}"#),
    ];
    let (text, label) = walk_of(&review);
    assert_eq!((text.as_str(), label.as_str()), ("Leaf not ok.", "Leaf not ok."));
}
