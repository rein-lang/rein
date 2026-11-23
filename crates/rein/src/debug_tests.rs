use super::*;
use std::path::Path;

fn pr_handling() -> Project {
    Project::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/rein/pr-handling")).unwrap()
}

fn first(answers: &[(&str, &str)]) -> StepRequest {
    StepRequest {
        version: rein_core::step::PROTOCOL_VERSION,
        entry: "main".into(),
        answers: answers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        flow: None,
        inputs: [("pr".to_string(), "https://x/pr/1".to_string())].into(),
    }
}

fn walk_with(answers: &[(&str, &str)]) -> Value {
    walk(&pr_handling(), "main", &serde_json::to_string(&first(answers)).unwrap())
}

#[test]
fn partial_walk_stops_at_the_next_question() {
    let out = walk_with(&[("main.draft", "no"), ("main.task", "merge")]);
    assert_eq!(out["step"]["kind"], "question");
    assert_eq!(out["step"]["key"], "merge.ci");
    assert_eq!(out["step"]["trace"], json!(["main", "merge"]));
    assert_eq!(out["current"]["key"], "merge.ci");
    assert_eq!(out["path"], json!(["f2", "f2_0", "f2_2", "f3", "f3_0"]));
    assert!(out["mermaid"].as_str().unwrap().contains("class f3_0 current"));
    assert_eq!(out["keys"]["f3_0"], "merge.ci");
}

#[test]
fn full_walk_ends_with_the_instruction() {
    let answers = [
        ("main.draft", "no"),
        ("main.task", "merge"),
        ("merge.ci", "green"),
        ("merge.conflicts", "no"),
        ("merge.review", "approved"),
    ];
    let out = walk_with(&answers);
    assert_eq!(out["step"]["kind"], "done");
    assert_eq!(out["step"]["instruction"], "Merge the pull request https://x/pr/1. Then delete its branch.");
    assert_eq!(out["current"]["kind"], "done");
}

#[test]
fn invalid_answer_is_the_step_error() {
    let out = walk_with(&[("main.task", "ship")]);
    assert_eq!(out["step"]["kind"], "error");
    let message = "invalid value `ship` for `task`; expected one of: `review`, `address comments`, `fix ci`, `merge`";
    assert_eq!(out["step"]["message"], message);
    assert!(out["current"].is_null());
    assert!(!out["mermaid"].as_str().unwrap().contains("classDef taken"));
}

#[test]
fn bad_json_is_an_error_step() {
    let out = walk(&pr_handling(), "main", "{");
    assert_eq!(out["step"]["kind"], "error");
    assert!(out["step"]["message"].as_str().unwrap().starts_with("invalid request JSON"));
}

#[test]
fn schema_lists_inputs_and_keys_with_types() {
    let s = schema(&pr_handling(), "main");
    assert_eq!(s["inputs"]["main"], json!([{"name": "pr", "description": "The link of the pull request to handle."}]));
    let list = s["answers"].as_array().unwrap();
    let find = |key: &str| list.iter().find(|a| a["key"] == key).cloned().unwrap_or_else(|| panic!("{key}: {s}"));
    assert_eq!(find("main.draft")["type"], "boolean");
    assert_eq!(find("main.task")["options"], json!(["review", "address comments", "fix ci", "merge"]));
    assert_eq!(find("merge.ci")["options"], json!(["green", "red", "pending"]));
    assert_eq!(find("review.verdict")["type"], "union");
    assert_eq!(find("review/security_review.security_problem")["type"], "boolean");
    assert!(list.iter().all(|a| a["key"] != "main.pr"), "a string parameter is an input, not an answer");
}

#[test]
fn page_has_the_controls_and_the_pinned_mermaid() {
    let html = page(&first(&[("main.task", "merge")]));
    for part in ["id=\"controls\"", "id=\"step\"", "id=\"diagram\"", graph_cmd::MERMAID, "\"main.task\":\"merge\""] {
        assert!(html.contains(part), "{part}");
    }
}

#[test]
fn page_keeps_the_state_in_the_url_and_has_a_copy_button() {
    let html = page(&first(&[]));
    let parts = [
        "history.replaceState",
        "URLSearchParams",
        "location.search",
        "id=\"copy\"",
        "Copy link",
        "navigator.clipboard",
    ];
    for part in parts {
        assert!(html.contains(part), "{part}");
    }
}

#[test]
fn page_has_the_dark_mode_and_the_theme_switch() {
    let html = page(&first(&[]));
    for part in [
        "@media (prefers-color-scheme: dark)",
        ":root[data-theme=\"dark\"]",
        "id=\"theme\"",
        "localStorage.getItem(\"rein-theme\")",
        "theme: dark ? \"dark\" : \"default\"",
        "applyTheme(theme.value)",
    ] {
        assert!(html.contains(part), "{part}");
    }
}

#[test]
fn start_url_carries_the_starting_state_as_its_query() {
    let url = start_url(&first(&[("main.draft", "no")]), 4000);
    assert_eq!(url, "http://127.0.0.1:4000/?entry=main&in.pr=https%3A%2F%2Fx%2Fpr%2F1&a.main.draft=no");
    let bare = start_url(&first(&[("main.draft", "")]), 4000);
    assert_eq!(bare, "http://127.0.0.1:4000/?entry=main&in.pr=https%3A%2F%2Fx%2Fpr%2F1");
}

#[test]
fn page_has_the_zoom_and_the_pan() {
    let html = page(&first(&[]));
    for part in ["id=\"zoom-in\"", "id=\"zoom-fit\"", "id=\"zoom-100\"", "function centerCurrent()", "addEventListener(\"wheel\""] {
        assert!(html.contains(part), "{part}");
    }
}

#[test]
fn routes_and_the_host_check() {
    let server = Server { project: pr_handling(), first: first(&[]), port: 4000 };
    assert_eq!(server.respond("GET", "/nope", "").0, "404 Not Found");
    assert_eq!(server.respond("GET", "/schema", "").1, "application/json");
    assert_eq!(server.respond("GET", "/", "").1, "text/html");
    assert_eq!(server.respond("GET", "/?entry=main&a.main.draft=no", "").1, "text/html");
    assert!(http::local_host("127.0.0.1:4000", 4000) && http::local_host("localhost:4000", 4000));
    assert!(!http::local_host("evil.example:4000", 4000));
}

#[test]
fn zero_args_walk_shows_the_first_question_with_the_input_unset() {
    let out = walk(&pr_handling(), "main", "{}");
    assert_eq!(out["step"]["kind"], "question", "{out}");
    assert_eq!(out["step"]["key"], "main.draft");
    assert_eq!(out["placeholders"], json!(["pr"]));
    assert_eq!(out["current"]["node"], "f2_0", "{out}");
    let s = schema(&pr_handling(), "main");
    assert_eq!(s["entry"], "main");
    assert!(s["functions"].as_array().unwrap().contains(&json!("merge")), "{s}");
    let other = walk(&pr_handling(), "main", r#"{"entry": "merge"}"#);
    assert_eq!(other["step"]["key"], "merge.ci", "{other}");
    assert_eq!(other["placeholders"], json!(["pr"]));
}

#[test]
fn unset_input_shows_as_a_placeholder_and_a_set_one_fills_the_text() {
    let body = json!({"answers": {"main.draft": "yes"}}).to_string();
    let out = walk(&pr_handling(), "main", &body);
    assert_eq!(out["step"]["kind"], "done", "{out}");
    assert!(out["step"]["instruction"].as_str().unwrap().contains("The pull request <pr> is a draft"), "{out}");
    let out = walk_with(&[("main.draft", "yes")]);
    assert!(out["step"]["instruction"].as_str().unwrap().contains("The pull request https://x/pr/1 is a draft"), "{out}");
    assert_eq!(out["placeholders"], json!([]));
}

/// A project whose entry takes one `IRI` parameter and asks nothing.
fn iri_entry_project() -> Project {
    let root = std::env::temp_dir().join(format!("rein-debug-iri-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = "/** Entry. */\nexport default function main(\n  /** The link. */\n  link: IRI,\n) {\n  return delegate(model=`x`, task=`Open ${link}.`);\n}\n";
    std::fs::write(root.join("main.rein"), source).unwrap();
    let project = Project::load(&root).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    project
}

#[test]
fn walk_with_an_unset_iri_input_uses_a_valid_example() {
    let out = walk(&iri_entry_project(), "main", "{}");
    assert_eq!(out["step"]["kind"], "done", "{out}");
    assert!(out["step"]["instruction"].as_str().unwrap().contains("Task: Open urn:rein:link."), "{out}");
    assert_eq!(out["placeholders"], json!(["link"]));
}

#[test]
fn walk_with_an_unset_string_input_passed_to_an_iri_uses_a_valid_example() {
    let root = std::env::temp_dir().join(format!("rein-debug-text-iri-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let main = "import { other } from \"./other\";\n\n/** Entry. */\nexport default function main(\n  /** The link. */\n  link: string,\n) {\n  return other(link=link);\n}\n";
    let other = "/** Open a link. */\nexport default function other(\n  /** The link. */ link: IRI,\n) {\n  return `Open ${link}.`;\n}\n";
    std::fs::write(root.join("main.rein"), main).unwrap();
    std::fs::write(root.join("other.rein"), other).unwrap();
    let project = Project::load(&root).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let out = walk(&project, "main", "{}");
    assert_eq!(out["step"]["kind"], "done", "{out}");
    assert!(out["step"]["instruction"].as_str().unwrap().contains("Open urn:rein:link."), "{out}");
    assert_eq!(out["placeholders"], json!(["link"]));
}

#[test]
fn full_walk_with_the_input_unset_ends_with_the_placeholder() {
    let answers = json!({
        "main.draft": "no",
        "main.task": "merge",
        "merge.ci": "green",
        "merge.conflicts": "no",
        "merge.review": "approved"
    });
    let out = walk(&pr_handling(), "main", &json!({"answers": answers}).to_string());
    assert_eq!(out["step"]["kind"], "done", "{out}");
    assert_eq!(out["step"]["instruction"], "Merge the pull request <pr>. Then delete its branch.");
}
