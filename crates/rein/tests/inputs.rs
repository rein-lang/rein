//! Program inputs through the built CLI: `rein step --input`, `rein run --set` for a `string` parameter,
//! the checker's errors and warnings for `string` inputs, and a full walk of the pr-handling example.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const LINK: &str = "https://example.com/pr/1";

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/.rein/pr-handling")
        .canonicalize()
        .unwrap()
}

/// A temporary flow root, removed when the value is dropped.
struct Flow {
    root: PathBuf,
}

impl Flow {
    fn new(files: &[(&str, &str)]) -> Flow {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rein-inputs-{}-{nanos}-{n}", std::process::id()));
        for (rel, text) in files {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        Flow { root }
    }

    fn path(&self) -> &str {
        self.root.to_str().unwrap()
    }
}

impl Drop for Flow {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Runs `rein` with `args`, and `stdin` as its standard input when given.
fn rein(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rein"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut pipe = child.stdin.take().unwrap();
        if let Some(text) = stdin {
            pipe.write_all(text.as_bytes()).unwrap();
        }
    }
    child.wait_with_output().unwrap()
}

/// `rein step --root <root> <extra>`, with no stdin.
fn step_in(root: &str, extra: &[&str]) -> Output {
    let mut args = vec!["step", "--root", root];
    args.extend_from_slice(extra);
    rein(&args, None)
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap()
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!("stdout is not JSON ({e}): {}", String::from_utf8_lossy(&out.stdout))
    })
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The `--answer` flags for a walk, as `key=value` strings.
fn answer_flags(answers: &[(String, String)]) -> Vec<String> {
    answers.iter().flat_map(|(k, v)| ["--answer".to_string(), format!("{k}={v}")]).collect()
}

/// Walks the pr-handling example, answering each key with `answer_for`, and returns the last response.
/// Each call gives `--input pr=<LINK>` and every answer so far. Returns the response and the exit codes.
fn walk_pr_handling(answer_for: impl Fn(&str) -> &'static str) -> (Value, Vec<i32>) {
    let root = example();
    let mut answers: Vec<(String, String)> = Vec::new();
    let mut codes = Vec::new();
    loop {
        let flags = answer_flags(&answers);
        let mut args: Vec<&str> = vec!["--input", "pr=https://example.com/pr/1", "--json"];
        args.extend(flags.iter().map(String::as_str));
        let out = step_in(root.to_str().unwrap(), &args);
        codes.push(code(&out));
        let v = json(&out);
        match v["kind"].as_str().unwrap() {
            "question" | "request" => {
                let key = v["key"].as_str().unwrap().to_string();
                answers.push((key.clone(), answer_for(&key).to_string()));
            }
            _ => return (v, codes),
        }
    }
}

#[test]
fn pr_handling_merge_walk_ends_in_done_with_the_link() {
    let (last, codes) = walk_pr_handling(|key| match key {
        "main.draft" => "no",
        "main.task" => "merge",
        "merge.ci" => "green",
        "merge.conflicts" => "no",
        "merge.review" => "approved",
        other => panic!("the walk asked an unexpected key `{other}`"),
    });
    assert_eq!(codes, [2, 2, 2, 2, 2, 0]);
    assert_eq!(last["kind"], "done");
    assert_eq!(last["instruction"], format!("Merge the pull request {LINK}. Then delete its branch."));
    assert_eq!(last["trace"], json!(["main", "merge"]));
}

#[test]
fn pr_handling_review_walk_passes_the_link_to_the_review_template() {
    let (last, codes) = walk_pr_handling(|key| match key {
        "main.draft" => "no",
        "main.task" => "review",
        "review.ownPr" => "no",
        "review.securitySensitive" => "no",
        "review.verdict" => "clean",
        other => panic!("the walk asked an unexpected key `{other}`"),
    });
    assert_eq!(codes, [2, 2, 2, 2, 3, 0]);
    let text = last["instruction"].as_str().unwrap();
    assert!(text.contains(LINK), "{text}");
    assert!(text.contains("Type: approve"), "{text}");
    assert_eq!(last["trace"], json!(["main", "review"]));
}

#[test]
fn pr_handling_draft_stops_and_names_the_link() {
    let (last, _) = walk_pr_handling(|key| match key {
        "main.draft" => "yes",
        other => panic!("the walk asked an unexpected key `{other}`"),
    });
    assert_eq!(last["kind"], "done");
    let text = last["instruction"].as_str().unwrap();
    assert!(text.contains(LINK) && text.contains("is a draft"), "{text}");
}

#[test]
fn pr_handling_checks_with_no_errors_or_warnings() {
    let out = rein(&["check", example().to_str().unwrap()], None);
    assert_eq!(code(&out), 0, "{}{}", stdout(&out), stderr(&out));
    assert!(stderr(&out).contains("0 errors, 0 warnings"), "{}", stderr(&out));
}

#[test]
fn string_input_is_never_asked() {
    let root = example();
    let out = step_in(root.to_str().unwrap(), &["--input", &format!("pr={LINK}"), "--json"]);
    assert_eq!(code(&out), 2);
    assert_eq!(json(&out)["key"], "main.draft");
}

#[test]
fn missing_input_is_an_error_response() {
    let root = example();
    let out = step_in(root.to_str().unwrap(), &["--json"]);
    assert_eq!(code(&out), 1);
    let v = json(&out);
    assert_eq!(v["kind"], "error");
    assert_eq!(v["message"], "missing input `pr`; give it with --input pr=<value>");
    assert_eq!(v["key"], Value::Null);
}

#[test]
fn missing_input_is_printed_in_text_mode() {
    let root = example();
    let out = step_in(root.to_str().unwrap(), &[]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("error: missing input `pr`; give it with --input pr=<value>"), "{}", stderr(&out));
}

#[test]
fn unknown_input_is_an_error() {
    let root = example();
    let out = step_in(root.to_str().unwrap(), &["--input", &format!("pr={LINK}"), "--input", "zz=1", "--json"]);
    assert_eq!(code(&out), 1);
    assert_eq!(json(&out)["message"], "unknown input `zz`");
}

#[test]
fn a_boolean_parameter_cannot_be_given_as_an_input() {
    let root = example();
    let out = step_in(root.to_str().unwrap(), &["--input", &format!("pr={LINK}"), "--input", "draft=no", "--json"]);
    assert_eq!(code(&out), 1);
    assert_eq!(json(&out)["message"], "unknown input `draft`");
}

#[test]
fn input_given_twice_is_an_error() {
    let root = example();
    let out = step_in(root.to_str().unwrap(), &["--input", "pr=a", "--input", "pr=b"]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("error: input `pr` set twice"), "{}", stderr(&out));
}

#[test]
fn input_without_equals_is_an_error() {
    let root = example();
    let out = step_in(root.to_str().unwrap(), &["--input", "pr"]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("error: invalid --input `pr`; expected name=value"), "{}", stderr(&out));
}

#[test]
fn answer_naming_a_string_input_is_still_an_error() {
    let root = example();
    let out = step_in(
        root.to_str().unwrap(),
        &["--input", &format!("pr={LINK}"), "--answer", "main.pr=x", "--json"],
    );
    assert_eq!(code(&out), 1);
    let v = json(&out);
    assert_eq!(v["key"], "main.pr");
    assert_eq!(v["message"], "`main.pr` is a string parameter; only a template takes one");
}

#[test]
fn stdin_request_carries_the_inputs() {
    let root = example();
    let request = json!({
        "inputs": { "pr": LINK },
        "answers": { "main.draft": "no", "main.task": "merge", "merge.ci": "green", "merge.conflicts": "no", "merge.review": "approved" },
    });
    let out = rein(
        &["step", "--root", root.to_str().unwrap(), "--stdin"],
        Some(&request.to_string()),
    );
    assert_eq!(code(&out), 0, "{}", stdout(&out));
    assert_eq!(json(&out)["kind"], "done");
}

#[test]
fn stdin_cannot_be_combined_with_input_flags() {
    let root = example();
    let out = rein(
        &["step", "--root", root.to_str().unwrap(), "--stdin", "--input", &format!("pr={LINK}")],
        Some("{}"),
    );
    assert_eq!(code(&out), 1);
    assert_eq!(json(&out)["kind"], "error");
}

#[test]
fn rein_run_set_gives_a_string_parameter_its_value() {
    let flow = Flow::new(&[(
        "main.rein",
        "/** Entry. */\nexport default function main(\n  /** The link. */\n  pr: string,\n  /** Ok? */\n  ok: boolean,\n) {\n  if (ok) {\n    return `Link ${pr}`;\n  }\n  return \"none\";\n}\n",
    )]);
    let out = rein(&["run", "main", "--root", flow.path(), "--set", &format!("pr={LINK}"), "--set", "ok=yes"], None);
    assert_eq!(code(&out), 0, "{}{}", stdout(&out), stderr(&out));
    assert_eq!(stdout(&out).trim(), format!("Link {LINK}"));
}

#[test]
fn rein_run_asks_for_a_missing_string_value() {
    let flow = Flow::new(&[(
        "main.rein",
        "/** Entry. */\nexport default function main(\n  /** The link. */\n  pr: string,\n  /** Ok? */\n  ok: boolean,\n) {\n  if (ok) {\n    return `Link ${pr}`;\n  }\n  return \"none\";\n}\n",
    )]);
    let out = rein(&["run", "main", "--root", flow.path(), "--set", "ok=yes"], None);
    assert_eq!(code(&out), 2);
    let text = stdout(&out);
    assert!(text.contains("need input pr: string - The link."), "{text}");
    assert!(text.contains("hint: rerun with --set pr=<value>"), "{text}");
}

#[test]
fn all_string_body_that_is_not_one_template_literal_runs_as_a_function() {
    // Every parameter is `string`, but the body is not one template literal, so this is not a template.
    let flow = Flow::new(&[(
        "main.rein",
        "/** Entry. */\nexport default function main(\n  /** The link. */\n  pr: string,\n) {\n  return \"done\";\n}\n",
    )]);
    let out = rein(&["run", "main", "--root", flow.path()], None);
    assert_eq!(code(&out), 0, "{}{}", stdout(&out), stderr(&out));
    assert_eq!(stdout(&out).trim(), "done");
}

#[test]
fn all_string_template_run_by_itself_is_still_an_error() {
    let flow = Flow::new(&[(
        "main.rein",
        "/** Entry. */\nexport default function main() {\n  return \"x\";\n}\n",
    ), (
        "tpl.rein",
        "/** Template. */\nexport default function tpl(\n  /** A */\n  a: string,\n) {\n  return `Hi ${a}`;\n}\n",
    )]);
    let out = rein(&["run", "tpl", "--root", flow.path()], None);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("templates are called from functions, not run"), "{}", stderr(&out));
}

#[test]
fn unused_string_input_is_a_warning() {
    let flow = Flow::new(&[(
        "main.rein",
        "/** Entry. */\nexport default function main(\n  /** The link. */\n  pr: string,\n) {\n  return \"done\";\n}\n",
    )]);
    let out = rein(&["check", flow.path()], None);
    assert_eq!(code(&out), 0, "{}{}", stdout(&out), stderr(&out));
    assert!(stdout(&out).contains("warning: unused parameter `pr`"), "{}", stdout(&out));
    assert!(stderr(&out).contains("0 errors, 1 warnings"), "{}", stderr(&out));
}

#[test]
fn string_input_in_a_condition_is_a_check_error() {
    let flow = Flow::new(&[(
        "main.rein",
        "/** Entry. */\nexport default function main(\n  /** The link. */\n  pr: string,\n) {\n  if (pr) {\n    return \"a\";\n  }\n  return \"b\";\n}\n",
    )]);
    let out = rein(&["check", flow.path()], None);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("error: `pr` is a string input; a condition cannot read it"), "{}", stdout(&out));
}

#[test]
fn hand_off_with_too_few_arguments_is_a_check_error() {
    let flow = Flow::new(&[
        ("main.rein", "/** Entry. */\nexport default function main(\n  /** The link. */\n  pr: string,\n) {\n  return @./merge;\n}\n"),
        ("merge.rein", "/** Merge. */\nexport default function merge(\n  /** The link. */\n  pr: string,\n  /** CI? */\n  ci: \"green\" | \"red\",\n) {\n  switch (ci) {\n    case \"green\":\n      return `Merge ${pr}`;\n    case \"red\":\n      return \"Wait.\";\n  }\n}\n"),
    ]);
    let out = rein(&["check", flow.path()], None);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("`@./merge` takes 1 argument, got 0"), "{}", stdout(&out));
}

#[test]
fn hand_off_with_too_many_arguments_is_a_check_error() {
    let flow = Flow::new(&[
        ("main.rein", "/** Entry. */\nexport default function main(\n  /** The link. */\n  pr: string,\n) {\n  return @./merge(pr, pr);\n}\n"),
        ("merge.rein", "/** Merge. */\nexport default function merge(\n  /** The link. */\n  pr: string,\n  /** CI? */\n  ci: \"green\" | \"red\",\n) {\n  switch (ci) {\n    case \"green\":\n      return `Merge ${pr}`;\n    case \"red\":\n      return \"Wait.\";\n  }\n}\n"),
    ]);
    let out = rein(&["check", flow.path()], None);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("`@./merge` takes 1 argument, got 2"), "{}", stdout(&out));
}

#[test]
fn hand_off_arguments_to_a_function_without_strings_is_a_check_error() {
    let flow = Flow::new(&[
        ("main.rein", "/** Entry. */\nexport default function main(\n  /** The link. */\n  pr: string,\n) {\n  return @./plain(pr);\n}\n"),
        ("plain.rein", "/** Plain. */\nexport default function plain(\n  /** Ok? */\n  ok: boolean,\n) {\n  if (ok) {\n    return \"yes\";\n  }\n  return \"no\";\n}\n"),
    ]);
    let out = rein(&["check", flow.path()], None);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("`@./plain` takes no arguments; return it without a call"), "{}", stdout(&out));
}

#[test]
fn argument_that_is_not_a_string_input_is_a_check_error() {
    let flow = Flow::new(&[
        ("main.rein", "/** Entry. */\nexport default function main(\n  /** C */\n  ci: \"green\" | \"red\",\n) {\n  return @./merge(ci);\n}\n"),
        ("merge.rein", "/** Merge. */\nexport default function merge(\n  /** The link. */\n  pr: string,\n) {\n  return `Merge ${pr}`;\n}\n"),
    ]);
    let out = rein(&["check", flow.path()], None);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("`ci` is not a string input"), "{}", stdout(&out));
}

#[test]
fn string_input_is_a_template_argument_and_the_check_is_clean() {
    let flow = Flow::new(&[
        ("main.rein", "/** Entry. */\nexport default function main(\n  /** The link. */\n  pr: string,\n  /** Ok? */\n  ok: boolean,\n) {\n  if (ok) {\n    return @./tpl(pr, \"b\");\n  }\n  return \"n\";\n}\n"),
        ("tpl.rein", "/** Template. */\nexport default function tpl(\n  /** A */\n  a: string,\n  /** B */\n  b: string,\n) {\n  return `${a} then ${b}`;\n}\n"),
    ]);
    let out = rein(&["check", flow.path()], None);
    assert_eq!(code(&out), 0, "{}{}", stdout(&out), stderr(&out));
    assert!(stderr(&out).contains("0 errors, 0 warnings"), "{}", stderr(&out));
    let run = rein(&["run", "main", "--root", flow.path(), "--set", &format!("pr={LINK}"), "--set", "ok=yes"], None);
    assert_eq!(code(&run), 0, "{}{}", stdout(&run), stderr(&run));
    assert_eq!(stdout(&run).trim(), format!("{LINK} then b"));
}
