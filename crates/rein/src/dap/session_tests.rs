use super::*;
use std::path::PathBuf;
use std::sync::OnceLock;

/// The `pr-handling` flow with its named imports, written inline. It asks for `main.draft`, `main.task` and then the
/// `merge` questions. The programs that `main` and `merge` call are stubs, because the tests stop at `main` and `merge`.
const MAIN: &str = r#"import { review } from "./review";
import { address } from "./comments/address";
import { fix } from "./ci/fix";
import { merge } from "./merge";

interface Input {
  /** The link of the pull request to handle. */
  pr: string;
  /** Is the pull request still a draft? */
  draft: boolean;
  /** Which task does the agent do on the pull request? */
  task: "review" | "address comments" | "fix ci" | "merge";
}

/**
 * Decide the next step for a pull request: review it, fix it, or merge it.
 */
export default function main({ pr, draft, task }: Input) {
  if (draft) {
    return `Stop. The pull request ${pr} is a draft. Wait until the author
    marks it ready.`;
  }

  switch (task) {
    case "review":
      return review(pr=pr);
    case "address comments":
      return address(pr=pr);
    case "fix ci":
      return fix(pr=pr);
    case "merge":
      return merge(pr=pr);
  }
}
"#;

const MERGE: &str = r#"import { fix } from "./ci/fix";
import { address } from "./comments/address";
import { review as openReview } from "./review";

interface Input {
  /** The link of the pull request to merge. */
  pr: string;
  /** What is the CI status of the PR? */
  ci: "green" | "red" | "pending";
  /** Does the PR conflict with its base branch? */
  conflicts: boolean;
  /** What is the review state of the PR? */
  review: "approved" | "changes requested" | "no review";
}

/**
 * Decide whether a pull request can merge now.
 */
export default function merge({ pr, ci, conflicts, review }: Input) {
  switch (ci) {
    case "red":
      return fix(pr=pr);
    case "pending":
      return `Wait for CI to finish on ${pr}. Then start this program again.`;
    case "green":
      if (conflicts) {
        return `Rebase ${pr} on its base. Push. Then start this program again.`;
      }

      switch (review) {
        case "approved":
          return `Merge the pull request ${pr}. Then delete its branch.`;
        case "changes requested":
          return address(pr=pr);
        case "no review":
          return openReview(pr=pr);
      }
  }
}
"#;

/// The folder with the inline flow, written once for all tests in this process.
fn fixture() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("rein-dap-flow-{}", std::process::id()));
        let write = |file: &str, text: &str| {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write("main.rein", MAIN);
        write("merge.rein", MERGE);
        write("rein.json", "{\n  \"entry\": \"main.rein\"\n}\n");
        for file in ["review", "comments/address", "ci/fix"] {
            let name = file.rsplit('/').next().unwrap();
            write(
                &format!("{file}.rein"),
                &format!("/** Stub. */\nexport default function {name}(\n  /** The link. */\n  pr: string,\n) {{\n  return `done ${{pr}}`;\n}}\n"),
            );
        }
        root
    })
}

/// The path of `file` in the inline `pr-handling` flow.
fn example(file: &str) -> String {
    fixture().join(file).display().to_string()
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
fn missing_format_input_is_asked_with_its_format_name() {
    let root = std::env::temp_dir().join(format!("rein-dap-iri-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let main = "/** Entry. */\nexport default function main(\n  /** The link. */\n  link: Iri,\n) {\n  return delegate(model=`x`, task=`Open ${link}.`);\n}\n";
    fs::write(root.join("main.rein"), main).unwrap();
    fs::write(root.join("rein.json"), "{\n  \"entry\": \"main.rein\"\n}\n").unwrap();
    let mut session = Session::default();
    session.handle(&request("initialize", json!({})));
    let program = root.join("main.rein").display().to_string();
    let out = session.handle(&request("launch", json!({"program": program})));
    assert!(out[0]["success"].as_bool().unwrap(), "{out:?}");
    let out = session.handle(&request("configurationDone", json!({})));
    let asks = events(&out, "rein/ask");
    assert_eq!(asks[0]["body"]["key"], "link");
    assert_eq!(asks[0]["body"]["type"], "iri");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn invalid_format_answer_is_refused_and_asked_again() {
    let root = std::env::temp_dir().join(format!("rein-dap-iri-retry-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let main = "/** Entry. */\nexport default function main(\n  /** The link. */\n  link: Iri,\n) {\n  return delegate(model=`x`, task=`Open ${link}.`);\n}\n";
    fs::write(root.join("main.rein"), main).unwrap();
    fs::write(root.join("rein.json"), "{\n  \"entry\": \"main.rein\"\n}\n").unwrap();
    let mut session = Session::default();
    session.handle(&request("initialize", json!({})));
    let program = root.join("main.rein").display().to_string();
    let out = session.handle(&request("launch", json!({"program": program})));
    assert!(out[0]["success"].as_bool().unwrap(), "{out:?}");
    let out = session.handle(&request("configurationDone", json!({})));
    assert_eq!(asked(&out), "link");

    let out = answer(&mut session, "link", "example.com");
    assert_eq!(out[0]["success"], false, "{out:?}");
    assert!(out[0]["message"].as_str().unwrap().contains("`link` must match format `iri`"), "{out:?}");
    assert!(events(&out, "terminated").is_empty(), "{out:?}");
    assert!(events(&out, "output").is_empty(), "{out:?}");

    let out = answer(&mut session, "link", "https://example.com/a");
    assert_eq!(out[0]["success"], true, "{out:?}");
    assert_eq!(events(&out, "terminated").len(), 1, "{out:?}");
    fs::remove_dir_all(&root).unwrap();
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
    let line = line_with("main.rein", "merge(pr=pr)");
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
    assert_eq!(frames[1]["line"], line_with("main.rein", "merge(pr=pr)"));
    assert!(frames[0]["source"]["path"]
        .as_str()
        .unwrap()
        .ends_with("/merge.rein"));
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
