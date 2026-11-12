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
        "  if (!ready) {\n    return \"wait\";\n  }\n  return \"go\";",
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
        "  if (c !== \"x\") {\n    return \"a\";\n  }\n  return \"b\";",
        "  /** C? */\n  c: \"x\" | \"y\" | \"z\",\n",
    );
    let g = flow(&p, "main");
    assert_eq!(g.edges[1].label.as_deref(), Some("y / z"));
    assert_eq!(g.edges[2].label.as_deref(), Some("x"));
}

#[test]
fn compound_condition_is_one_condition_node() {
    let p = one(
        "  if (a && (b || !a)) {\n    return \"one\";\n  }\n  return \"two\";",
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
        text.contains("say #quot;hi#quot; #35;1 #96;x#96;"),
        "{text}"
    );
}
