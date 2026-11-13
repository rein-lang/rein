use super::*;
use std::path::Path;

/// A fresh folder for one test. It is not the process environment, so tests can run in parallel.
fn folder(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rein-watch-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, file: &str, lines: &[Value]) {
    let text: Vec<String> = lines.iter().map(|v| v.to_string()).collect();
    fs::write(dir.join(file), text.join("\n") + "\n").unwrap();
}

fn agent_named(list: &Value, id: &str) -> Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("{id}: {list}"))
}

fn question(key: &str, trace: &[&str]) -> Value {
    let (function, name) = key.split_once('.').unwrap();
    json!({"kind": "question", "key": key, "function": function, "name": name, "type": "string", "options": null,
        "description": "", "answers": {}, "trace": trace, "version": 1})
}

fn pid_self() -> u64 {
    u64::from(std::process::id())
}

#[test]
fn mcp_agent_with_a_waiting_run_groups_answers_by_function() {
    let dir = folder("waiting");
    write(
        &dir,
        "mcp-1-1000.jsonl",
        &[
            json!({"ts": 1000, "event": "agent", "transport": "mcp", "pid": pid_self(), "cwd": "/work", "session": null}),
            json!({"ts": 1001, "event": "start", "run": "run-1", "program": null, "entry": "main", "step": question("main.task", &["main"])}),
            json!({"ts": 1002, "event": "answer", "run": "run-1", "key": "main.draft", "value": "no", "step": question("main.task", &["main"])}),
            json!({"ts": 1003, "event": "answer", "run": "run-1", "key": "main.task", "value": "merge", "step": question("merge.ci", &["main", "merge"])}),
            json!({"ts": 1004, "event": "answer", "run": "run-1", "key": "merge.ci", "value": "green", "step": question("merge.conflicts", &["main", "merge"])}),
            json!({"ts": 1005, "event": "answer", "run": "run-1", "key": "merge.ci", "value": "ship", "step": {"kind": "error", "key": "merge.ci", "message": "invalid", "trace": ["main"]}}),
            json!({"ts": 1006, "event": "mystery", "run": "run-1"}),
            // A failed `rein_start` logs a start with no run id. It makes no run.
            json!({"ts": 1007, "event": "start", "run": null, "program": null, "entry": "main", "step": null}),
        ],
    );
    // A bad line and a headerless log are skipped.
    fs::write(
        dir.join("mcp-1-1000.jsonl"),
        fs::read_to_string(dir.join("mcp-1-1000.jsonl")).unwrap() + "{not json\n",
    )
    .unwrap();
    write(
        &dir,
        "mcp-2-1.jsonl",
        &[json!({"ts": 1, "event": "start", "run": "run-1"})],
    );

    let list = agents(&dir, 2000);
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
    let a = agent_named(&list, "mcp-1-1000");
    assert_eq!(
        (a["transport"].clone(), a["alive"].clone(), a["cwd"].clone()),
        (json!("mcp"), json!(true), json!("/work"))
    );
    let run = &a["runs"][0];
    assert_eq!(run["status"], "waiting");
    assert_eq!(run["step"]["key"], "merge.conflicts");
    assert_eq!(run["events"].as_array().unwrap().len(), 5, "{run}");
    assert_eq!(
        run["functions"],
        json!([
            {"name": "main", "visited": true, "answers": {"draft": "no", "task": "merge"}},
            {"name": "merge", "visited": true, "answers": {"ci": "green"}},
        ])
    );
}

#[test]
fn stopped_run_and_a_dead_process_is_not_alive() {
    let dir = folder("stopped");
    let dead_pid = 999_999_999u64;
    write(
        &dir,
        "run-1-1000.jsonl",
        &[
            json!({"ts": 1000, "event": "agent", "transport": "run", "pid": dead_pid, "cwd": "/w", "session": null}),
            json!({"ts": 1001, "event": "start", "run": "run-1", "program": "x", "entry": "main",
            "step": {"kind": "done", "instruction": "Stop.", "trace": ["main"], "version": 1}}),
            json!({"ts": 1002, "event": "stop", "run": "run-1"}),
        ],
    );
    let a = agent_named(&agents(&dir, 2000), "run-1-1000");
    assert_eq!(a["alive"], false);
    assert_eq!(a["runs"][0]["status"], "stopped");
    assert_eq!(a["runs"][0]["program"], "x");
}

#[test]
fn stateless_step_calls_extend_a_run_or_start_a_new_one() {
    let dir = folder("steps");
    let call = |ts: u64, answers: Value, step: Value| {
        json!({"ts": ts, "event": "step", "program": null,
            "request": {"entry": "main", "answers": answers, "inputs": {"pr": "https://x/pr/1"}}, "step": step})
    };
    write(
        &dir,
        "step-s1.jsonl",
        &[
            json!({"ts": 1000, "event": "agent", "transport": "step", "pid": null, "cwd": "/w", "session": "s1"}),
            call(1001, json!({"main.draft": "no"}), question("main.task", &["main"])),
            call(
                1002,
                json!({"main.draft": "no", "main.task": "merge"}),
                question("merge.ci", &["main", "merge"]),
            ),
            call(1003, json!({"main.draft": "yes"}), question("main.task", &["main"])),
        ],
    );
    let a = agent_named(&agents(&dir, 1003 + 60_000), "step-s1");
    let runs = a["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "{a}");
    // Runs are newest first: the call that did not extend is runs[0].
    assert_eq!(runs[1]["events"].as_array().unwrap().len(), 2);
    assert_eq!(
        runs[1]["functions"][0]["answers"],
        json!({"draft": "no", "task": "merge"})
    );
    assert_eq!(runs[1]["inputs"], json!({"pr": "https://x/pr/1"}));
    assert_eq!(runs[1]["step"]["key"], "merge.ci");
    assert_eq!(runs[0]["events"].as_array().unwrap().len(), 1);
    assert_eq!(a["alive"], true);
    assert_eq!(
        agent_named(&agents(&dir, 1003 + 11 * 60_000), "step-s1")["alive"],
        false
    );
}

#[test]
fn a_loaded_program_lists_all_its_functions_and_marks_the_visited_ones() {
    let dir = folder("program");
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/rein/pr-handling/main.rein");
    write(
        &dir,
        "mcp-3-1000.jsonl",
        &[
            json!({"ts": 1000, "event": "agent", "transport": "mcp", "pid": pid_self(), "cwd": "/w", "session": null}),
            json!({"ts": 1001, "event": "start", "run": "run-1", "program": main.to_str().unwrap(), "entry": "main",
            "step": question("merge.ci", &["main", "merge"])}),
        ],
    );
    let a = agent_named(&agents(&dir, 2000), "mcp-3-1000");
    let functions = a["runs"][0]["functions"].as_array().unwrap();
    let find = |name: &str| {
        functions
            .iter()
            .find(|f| f["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("{name}: {a}"))
    };
    assert_eq!(find("main")["visited"], true);
    assert_eq!(find("merge")["visited"], true);
    assert_eq!(find("review")["visited"], false);
}

#[test]
fn host_check_gives_403_and_the_routes_answer() {
    let dir = folder("http");
    let mut out = Vec::new();
    let request = "GET /api/agents HTTP/1.1\r\nHost: evil.example:4000\r\n\r\n";
    http::exchange(request.as_bytes(), &mut out, 4000, |m, p, _| respond(&dir, m, p)).unwrap();
    assert!(String::from_utf8(out).unwrap().starts_with("HTTP/1.1 403 Forbidden"));

    let (status, kind, _) = respond(&dir, "GET", "/api/agents");
    assert_eq!((status, kind), ("200 OK", "application/json"));
    let (status, kind, page) = respond(&dir, "GET", "/");
    assert_eq!((status, kind), ("200 OK", "text/html"));
    assert!(page.contains("rein watch"));
    assert_eq!(respond(&dir, "GET", "/nope").0, "404 Not Found");
}

#[test]
fn a_header_with_no_run_is_an_agent_with_no_runs() {
    let dir = folder("header_only");
    write(
        &dir,
        "mcp-7-1000.jsonl",
        &[json!({"ts": 1000, "event": "agent", "transport": "mcp", "pid": pid_self(), "cwd": "/w", "session": null})],
    );
    let a = agent_named(&agents(&dir, 2000), "mcp-7-1000");
    assert_eq!((a["alive"].clone(), a["runs"].clone()), (json!(true), json!([])));
}

#[test]
fn error_only_run_and_a_failed_start_show_their_message() {
    let dir = folder("errors");
    let message = "missing input `pr`; give it with --input pr=<value>";
    let error = json!({"kind": "error", "version": 1, "message": message, "key": null, "diagnostics": [], "answers": {}, "trace": ["main"]});
    write(
        &dir,
        "step-s2.jsonl",
        &[
            json!({"ts": 1000, "event": "agent", "transport": "step", "pid": null, "cwd": "/w", "session": "s2"}),
            json!({"ts": 1001, "event": "step", "program": null, "request": {"entry": "main", "answers": {}}, "step": error}),
        ],
    );
    write(
        &dir,
        "mcp-5-1000.jsonl",
        &[
            json!({"ts": 1000, "event": "agent", "transport": "mcp", "pid": pid_self(), "cwd": "/w", "session": null}),
            json!({"ts": 1001, "event": "start", "run": null, "program": "x", "entry": "main", "step": {"kind": "error", "version": 1, "message": "no such program", "answers": {}, "trace": []}}),
        ],
    );
    let list = agents(&dir, 2000);
    let step_run = &agent_named(&list, "step-s2")["runs"][0];
    assert_eq!(step_run["status"], "error");
    assert_eq!(step_run["error"], message);
    assert!(step_run["step"].is_null());
    let mcp = agent_named(&list, "mcp-5-1000");
    assert_eq!(mcp["runs"], json!([]));
    assert_eq!(
        mcp["errors"],
        json!([{"ts": 1001, "program": "x", "message": "no such program"}])
    );
}

#[test]
fn runs_and_agents_are_newest_first() {
    let dir = folder("order");
    let call = |ts: u64, answers: Value| json!({"ts": ts, "event": "step", "program": null, "request": {"entry": "main", "answers": answers}, "step": question("main.task", &["main"])});
    write(
        &dir,
        "step-old.jsonl",
        &[
            json!({"ts": 100, "event": "agent", "transport": "step", "pid": null, "cwd": "/a", "session": "old"}),
            call(101, json!({"main.draft": "no"})),
        ],
    );
    write(
        &dir,
        "step-new.jsonl",
        &[
            json!({"ts": 200, "event": "agent", "transport": "step", "pid": null, "cwd": "/b", "session": "new"}),
            call(201, json!({"main.draft": "no"})),
            call(300, json!({"main.draft": "yes"})),
        ],
    );
    let list = agents(&dir, 400);
    assert_eq!(list[0]["id"], "step-new");
    assert_eq!(list[1]["id"], "step-old");
    let runs = list[0]["runs"].as_array().unwrap();
    assert_eq!(
        (runs[0]["last_ts"].clone(), runs[1]["last_ts"].clone()),
        (json!(300), json!(201))
    );
}

#[test]
fn a_folder_program_shows_the_folder_name_from_the_agent_cwd() {
    let dir = folder("folder-program");
    // The program folder has a main.rein, so it is the program the run used.
    let program = dir.join("rein-dependabot-handler");
    fs::create_dir_all(&program).unwrap();
    fs::write(program.join("main.rein"), "export default function main() {\n  return `done`;\n}\n").unwrap();
    write(
        &dir,
        "step-f1.jsonl",
        &[
            json!({"ts": 1000, "event": "agent", "transport": "step", "pid": null,
                "cwd": program.to_str().unwrap(), "session": "f1"}),
            json!({"ts": 1001, "event": "step", "program": ".",
                "request": {"entry": "main", "answers": {}}, "step": question("main.task", &["main"])}),
        ],
    );
    let a = agent_named(&agents(&dir, 1001), "step-f1");
    assert_eq!(a["runs"][0]["program"], "rein-dependabot-handler", "{a}");
}

/// The step-file log of a folder program: one call per `(entry, ts)`, all with the program `.`.
fn folder_program_log(dir: &Path, cwd: &str, calls: &[(&str, u64)]) {
    let mut lines = vec![json!({"ts": 1000, "event": "agent", "transport": "step", "pid": null,
        "cwd": cwd, "session": "g1"})];
    for (entry, ts) in calls {
        lines.push(json!({"ts": ts, "event": "step", "program": ".",
            "request": {"entry": entry, "answers": {}}, "step": question("main.task", &["main"])}));
    }
    write(dir, "step-g1.jsonl", &lines);
}

#[test]
fn the_newest_run_of_a_loadable_program_has_its_walk_as_mermaid() {
    let dir = folder("mermaid");
    let example = format!("{}/../../examples/rein/merge-pr", env!("CARGO_MANIFEST_DIR"));
    // The older run is not the newest, so it gets no graph. The newest one does.
    folder_program_log(&dir, &example, &[("main", 1001), ("review", 1002)]);
    let a = agent_named(&agents(&dir, 1002), "step-g1");
    let runs = a["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "{a}");
    let mermaid = runs[0]["mermaid"].as_str().unwrap_or_default();
    assert!(mermaid.starts_with("flowchart TD"), "{mermaid}");
    assert!(mermaid.lines().any(|l| l.trim_start().starts_with("class ") && l.ends_with(" current")), "{mermaid}");
    assert!(runs[1]["mermaid"].is_null(), "{a}");
    assert_eq!(runs[0]["functions"].as_array().unwrap().len(), 5, "{a}");
}

#[test]
fn a_program_that_does_not_load_has_no_mermaid() {
    let dir = folder("no-mermaid");
    folder_program_log(&dir, "/no/such/folder", &[("main", 1001)]);
    let a = agent_named(&agents(&dir, 1001), "step-g1");
    assert!(a["runs"][0].get("mermaid").is_some_and(Value::is_null), "{a}");
    assert!(a["runs"][0].get("mermaid_full").is_some_and(Value::is_null), "{a}");
}

#[test]
fn a_dotted_program_with_no_main_rein_loads_nothing() {
    let dir = folder("dotted-no-main");
    let empty = dir.join("empty-tree");
    fs::create_dir_all(&empty).unwrap();
    folder_program_log(&dir, empty.to_str().unwrap(), &[("main", 1001)]);
    let a = agent_named(&agents(&dir, 1001), "step-g1");
    // No folder is named, so the program keeps its written name and nothing is loaded.
    assert_eq!(a["runs"][0]["program"], ".", "{a}");
    assert!(a["runs"][0].get("mermaid").is_some_and(Value::is_null), "{a}");
}

#[test]
fn a_non_string_log_field_is_null_and_does_not_break_the_list() {
    let dir = folder("non-string");
    write(
        &dir,
        "step-n1.jsonl",
        &[
            json!({"ts": 1000, "event": "agent", "transport": "step", "pid": "x", "cwd": 5, "session": {"a": 1}}),
            json!({"ts": 1001, "event": "step", "program": 7,
                "request": {"entry": "main", "answers": {}}, "step": question("main.task", &["main"])}),
        ],
    );
    let a = agent_named(&agents(&dir, 1001), "step-n1");
    assert!(a["cwd"].is_null() && a["session"].is_null() && a["pid"].is_null(), "{a}");
    assert!(a["runs"][0]["program"].is_null(), "{a}");
}

#[test]
fn a_program_path_loads_only_a_folder_with_a_main_rein_from_the_agent_cwd() {
    let dir = folder("program-path");
    // `./x.rein` in a folder with no main.rein loads nothing.
    let bare = dir.join("bare");
    fs::create_dir_all(&bare).unwrap();
    fs::write(bare.join("x.rein"), "export default function main() {\n  return `done`;\n}\n").unwrap();
    folder_program_log(&dir, bare.to_str().unwrap(), &[("main", 1001)]);
    let mut lines = vec![json!({"ts": 1000, "event": "agent", "transport": "step", "pid": null,
        "cwd": bare.to_str().unwrap(), "session": "g1"})];
    lines.push(json!({"ts": 1001, "event": "step", "program": "./x.rein",
        "request": {"entry": "main", "answers": {}}, "step": question("main.task", &["main"])}));
    write(&dir, "step-g1.jsonl", &lines);
    let a = agent_named(&agents(&dir, 1001), "step-g1");
    assert_eq!(a["runs"][0]["program"], "./x.rein", "{a}");
    assert!(a["runs"][0].get("mermaid").is_some_and(Value::is_null), "{a}");

    // `./prog` with a main.rein resolves against the agent cwd, not the process cwd, which has no `prog`.
    let project = dir.join("prog");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("main.rein"), "export default function main() {\n  return `done`;\n}\n").unwrap();
    let lines = vec![
        json!({"ts": 1000, "event": "agent", "transport": "step", "pid": null, "cwd": dir.to_str().unwrap(), "session": "g2"}),
        json!({"ts": 1001, "event": "step", "program": "./prog",
            "request": {"entry": "main", "answers": {}}, "step": question("main.task", &["main"])}),
    ];
    write(&dir, "step-g2.jsonl", &lines);
    let a = agent_named(&agents(&dir, 1001), "step-g2");
    assert_eq!(a["runs"][0]["program"], "prog", "{a}");
    assert!(a["runs"][0]["mermaid"].is_string(), "{a}");
}

#[test]
fn an_error_entry_with_a_non_text_message_or_program_is_left_out_or_null() {
    let dir = folder("error-entries");
    write(
        &dir,
        "step-e1.jsonl",
        &[
            json!({"ts": 1000, "event": "agent", "transport": "step", "pid": null, "cwd": "/w", "session": "e1"}),
            json!({"ts": 1001, "event": "start", "run": null, "program": 9,
                "step": {"kind": "error", "message": "missing input"}}),
            json!({"ts": 1002, "event": "start", "run": null, "program": "x",
                "step": {"kind": "error", "message": 42}}),
        ],
    );
    let a = agent_named(&agents(&dir, 1002), "step-e1");
    assert_eq!(a["errors"], json!([{"ts": 1001, "program": null, "message": "missing input"}]), "{a}");
}

#[test]
fn an_empty_trace_draws_only_the_entry_function() {
    let dir = folder("empty-trace");
    let example = format!("{}/../../examples/rein/merge-pr", env!("CARGO_MANIFEST_DIR"));
    let lines = vec![
        json!({"ts": 1000, "event": "agent", "transport": "step", "pid": null, "cwd": example, "session": "t1"}),
        json!({"ts": 1001, "event": "step", "program": ".",
            "request": {"entry": "main", "answers": {}}, "step": question("main.task", &[])}),
    ];
    write(&dir, "step-t1.jsonl", &lines);
    let a = agent_named(&agents(&dir, 1001), "step-t1");
    let path = a["runs"][0]["mermaid"].as_str().unwrap_or_default();
    assert!(path.contains("Which task does the agent do?"), "{path}");
    assert!(!path.contains("Does the PR change only documentation files?"), "{path}");
}

#[test]
fn the_default_graph_names_only_the_functions_on_the_path() {
    let dir = folder("path-graph");
    let example = format!("{}/../../examples/rein/merge-pr", env!("CARGO_MANIFEST_DIR"));
    // The walk stops in `main`, so `merge_pr` is not on the path. Its question is in the whole program only.
    folder_program_log(&dir, &example, &[("main", 1001)]);
    let a = agent_named(&agents(&dir, 1001), "step-g1");
    let path = a["runs"][0]["mermaid"].as_str().unwrap_or_default();
    let full = a["runs"][0]["mermaid_full"].as_str().unwrap_or_default();
    assert!(path.contains("Which task does the agent do?"), "{path}");
    assert!(!path.contains("Does the PR change only documentation files?"), "{path}");
    assert!(full.contains("Does the PR change only documentation files?"), "{full}");
}

#[test]
fn a_program_changed_since_the_run_gives_no_graph() {
    let dir = folder("changed");
    let example = format!("{}/../../examples/rein/merge-pr", env!("CARGO_MANIFEST_DIR"));
    // The trace names a function the program does not have.
    let step = |trace: &[&str]| question("main.task", trace);
    let header = json!({"ts": 1000, "event": "agent", "transport": "step", "pid": null, "cwd": example, "session": "c1"});
    let call = |ts: u64, trace: &[&str]| {
        json!({"ts": ts, "event": "step", "program": ".", "request": {"entry": "main", "answers": {}}, "step": step(trace)})
    };
    write(&dir, "step-c1.jsonl", &[header.clone(), call(1001, &["main", "gone"])]);
    let a = agent_named(&agents(&dir, 1001), "step-c1");
    assert!(a["runs"][0]["mermaid"].is_null() && a["runs"][0]["mermaid_full"].is_null(), "{a}");
    assert_eq!(a["runs"][0]["program_changed"], true, "{a}");
    // The answer keys name a question the program does not ask.
    let stale = json!({"ts": 1002, "event": "step", "program": ".",
        "request": {"entry": "main", "answers": {"main.gone": "x"}}, "step": step(&["main"])});
    write(&dir, "step-c1.jsonl", &[header, stale]);
    let a = agent_named(&agents(&dir, 1002), "step-c1");
    assert!(a["runs"][0]["mermaid"].is_null(), "{a}");
    assert_eq!(a["runs"][0]["program_changed"], true, "{a}");
}
