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
    walk(&pr_handling(), &serde_json::to_string(&first(answers)).unwrap())
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
    let out = walk(&pr_handling(), "{");
    assert_eq!(out["step"]["kind"], "error");
    assert!(out["step"]["message"].as_str().unwrap().starts_with("invalid request JSON"));
}

#[test]
fn schema_lists_inputs_and_keys_with_types() {
    let s = schema(&pr_handling(), "main");
    assert_eq!(s["inputs"], json!([{"name": "pr", "description": "The link of the pull request to handle."}]));
    let list = s["answers"].as_array().unwrap();
    let find = |key: &str| list.iter().find(|a| a["key"] == key).cloned().unwrap_or_else(|| panic!("{key}: {s}"));
    assert_eq!(find("main.draft")["type"], "boolean");
    assert_eq!(find("main.task")["options"], json!(["review", "address comments", "fix ci", "merge"]));
    assert_eq!(find("merge.ci")["options"], json!(["green", "red", "pending"]));
    assert_eq!(find("review.verdict")["type"], "union");
    assert_eq!(find("review.securityProblem")["type"], "boolean");
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
fn routes_and_the_host_check() {
    let server = Server { project: pr_handling(), first: first(&[]), port: 4000 };
    assert_eq!(server.respond("GET", "/nope", "").0, "404 Not Found");
    assert_eq!(server.respond("GET", "/schema", "").1, "application/json");
    assert_eq!(server.respond("GET", "/", "").1, "text/html");
    assert!(server.local_host("127.0.0.1:4000") && server.local_host("localhost:4000"));
    assert!(!server.local_host("evil.example:4000"));
}
