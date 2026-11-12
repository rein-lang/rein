use super::*;

/// The `pr-handling` example, which asks for `main.draft`, `main.task` and then the `merge` questions.
fn example(file: &str) -> String {
    format!(
        "{}/../../examples/rein/pr-handling/{file}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The 1-based line of the first line of `file` that contains `needle`.
fn line_with(file: &str, needle: &str) -> usize {
    let text = fs::read_to_string(example(file)).unwrap();
    text.lines()
        .position(|l| l.contains(needle))
        .map(|i| i + 1)
        .unwrap()
}

fn request(command: &str, args: Value) -> Value {
    json!({"seq": 1, "type": "request", "command": command, "arguments": args})
}

fn launch(session: &mut Session, args: Value) {
    let mut args = args;
    args["program"] = json!(example("main.rein"));
    let out = session.handle(&request("launch", args));
    assert!(out[0]["success"].as_bool().unwrap(), "{out:?}");
}

fn events<'a>(out: &'a [Value], name: &str) -> Vec<&'a Value> {
    out.iter()
        .filter(|m| m["type"] == "event" && m["event"] == name)
        .collect()
}

/// The `key` of the one `rein/ask` in `out`, or a panic when there is none.
fn asked(out: &[Value]) -> String {
    let asks = events(out, "rein/ask");
    assert_eq!(asks.len(), 1, "{out:?}");
    asks[0]["body"]["key"].as_str().unwrap().to_string()
}

fn answer(session: &mut Session, key: &str, value: &str) -> Vec<Value> {
    session.handle(&request("rein/answer", json!({"key": key, "value": value})))
}

/// A session with the `pr` input given, started and stopped at its first question.
fn started(extra: Value) -> Session {
    let mut session = Session::default();
    session.handle(&request("initialize", json!({})));
    let mut args = json!({"inputs": {"pr": "https://example.test/pull/1"}});
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        args[k] = v;
    }
    launch(&mut session, args);
    session
}

#[test]
fn walk_asks_each_question_and_ends_with_the_instruction() {
    let mut session = started(json!({}));
    let out = session.handle(&request("configurationDone", json!({})));
    assert_eq!(asked(&out), "main.draft");
    assert_eq!(events(&out, "stopped")[0]["body"]["reason"], "pause");

    let out = answer(&mut session, "main.draft", "no");
    assert_eq!(asked(&out), "main.task");

    let out = answer(&mut session, "main.task", "merge");
    assert_eq!(asked(&out), "merge.ci");
    assert_eq!(
        events(&out, "rein/ask")[0]["body"]["options"],
        json!(["green", "red", "pending"])
    );

    let out = answer(&mut session, "merge.ci", "green");
    assert_eq!(asked(&out), "merge.conflicts");
    let out = answer(&mut session, "merge.conflicts", "no");
    assert_eq!(asked(&out), "merge.review");

    let out = answer(&mut session, "merge.review", "approved");
    let output = events(&out, "output");
    assert_eq!(output[0]["body"]["category"], "console");
    assert!(output[0]["body"]["output"]
        .as_str()
        .unwrap()
        .starts_with("Merge the pull request"));
    assert_eq!(events(&out, "terminated").len(), 1);
}

#[test]
fn refused_answer_is_not_stored() {
    let mut session = started(json!({}));
    session.handle(&request("configurationDone", json!({})));
    let out = answer(&mut session, "main.draft", "maybe");
    assert_eq!(out[0]["success"], false, "{out:?}");
    let out = answer(&mut session, "main.draft", "no");
    assert_eq!(asked(&out), "main.task");
}

#[test]
fn answer_to_a_question_not_waiting_is_refused() {
    let mut session = started(json!({}));
    session.handle(&request("configurationDone", json!({})));
    let out = answer(&mut session, "main.task", "merge");
    assert_eq!(out[0]["success"], false);
}

#[test]
fn missing_string_input_is_asked_first() {
    let mut session = Session::default();
    session.handle(&request("initialize", json!({})));
    launch(&mut session, json!({}));
    let out = session.handle(&request("configurationDone", json!({})));
    let asks = events(&out, "rein/ask");
    assert_eq!(asks[0]["body"]["kind"], "input");
    assert_eq!(asks[0]["body"]["key"], "pr");

    let out = answer(&mut session, "pr", "https://example.test/pull/2");
    assert_eq!(asked(&out), "main.draft");
}

#[test]
fn stop_on_entry_stops_before_the_first_question() {
    let mut session = started(json!({"stopOnEntry": true}));
    let out = session.handle(&request("configurationDone", json!({})));
    assert_eq!(events(&out, "stopped")[0]["body"]["reason"], "entry");
    assert!(events(&out, "rein/ask").is_empty());

    let out = session.handle(&request("continue", json!({"threadId": 1})));
    assert_eq!(asked(&out), "main.draft");
}

#[test]
fn run_without_debugging_asks_but_never_stops() {
    let mut session = started(json!({"noDebug": true}));
    let line = line_with("main.rein", "switch (task)");
    let source = json!({"path": example("main.rein")});
    session.handle(&request(
        "setBreakpoints",
        json!({"source": source, "breakpoints": [{"line": line}]}),
    ));
    let out = session.handle(&request("configurationDone", json!({})));
    assert_eq!(asked(&out), "main.draft");
    assert!(events(&out, "stopped").is_empty(), "{out:?}");

    let out = answer(&mut session, "main.draft", "no");
    assert_eq!(asked(&out), "main.task");
    assert!(events(&out, "stopped").is_empty(), "{out:?}");
}

#[test]
fn breakpoint_on_a_question_stops_before_it_is_asked() {
    let mut session = started(json!({}));
    let line = line_with("main.rein", "switch (task)");
    let source = json!({"path": example("main.rein")});
    session.handle(&request(
        "setBreakpoints",
        json!({"source": source, "breakpoints": [{"line": line}]}),
    ));
    session.handle(&request("configurationDone", json!({})));
    let out = answer(&mut session, "main.draft", "no");
    assert!(events(&out, "rein/ask").is_empty(), "{out:?}");
    assert_eq!(events(&out, "stopped")[0]["body"]["reason"], "breakpoint");

    let out = session.handle(&request("continue", json!({"threadId": 1})));
    assert_eq!(asked(&out), "main.task");
}

#[test]
fn breakpoint_on_a_hand_off_stops_at_the_next_question() {
    let mut session = started(json!({}));
    let line = line_with("main.rein", "@./merge(pr=pr)");
    let source = json!({"path": example("main.rein")});
    session.handle(&request(
        "setBreakpoints",
        json!({"source": source, "breakpoints": [{"line": line}]}),
    ));
    session.handle(&request("configurationDone", json!({})));
    answer(&mut session, "main.draft", "no");
    let out = answer(&mut session, "main.task", "merge");
    assert!(events(&out, "rein/ask").is_empty(), "{out:?}");
    assert_eq!(events(&out, "stopped")[0]["body"]["reason"], "breakpoint");
}

#[test]
fn stack_trace_has_a_frame_per_function_with_its_lines() {
    let mut session = started(json!({}));
    session.handle(&request("configurationDone", json!({})));
    answer(&mut session, "main.draft", "no");
    answer(&mut session, "main.task", "merge");

    let out = session.handle(&request("stackTrace", json!({"threadId": 1})));
    let frames = out[0]["body"]["stackFrames"].as_array().unwrap();
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert_eq!(frames[0]["name"], "merge");
    assert_eq!(frames[0]["line"], line_with("merge.rein", "switch (ci)"));
    assert_eq!(frames[1]["name"], "main");
    assert_eq!(frames[1]["line"], line_with("main.rein", "@./merge(pr=pr)"));
    assert!(frames[0]["source"]["path"]
        .as_str()
        .unwrap()
        .ends_with("pr-handling/merge.rein"));
}

#[test]
fn scopes_show_inputs_answers_and_the_current_step() {
    let mut session = started(json!({}));
    session.handle(&request("configurationDone", json!({})));
    answer(&mut session, "main.draft", "no");

    let out = session.handle(&request("scopes", json!({"frameId": 1})));
    let scopes = out[0]["body"]["scopes"].as_array().unwrap();
    let names: Vec<&str> = scopes.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Inputs", "Answers", "Current step"]);

    let mut vars = |reference: i64| {
        let out = session.handle(&request(
            "variables",
            json!({"variablesReference": reference}),
        ));
        out[0]["body"]["variables"].as_array().unwrap().clone()
    };
    assert_eq!(vars(INPUTS)[0]["name"], "pr");
    assert_eq!(vars(ANSWERS)[0]["name"], "main.draft");
    let current = vars(CURRENT);
    assert!(
        current
            .iter()
            .any(|v| v["name"] == "key" && v["value"] == "main.task"),
        "{current:?}"
    );
}
