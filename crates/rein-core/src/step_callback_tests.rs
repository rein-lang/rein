//! Stepping through callbacks: a function returns the function its caller passed, in one program and across programs.

use super::*;
use std::fs;
use std::path::{Path, PathBuf};

/// `pr` and a callback `then`, as an inline struct.
const INPUT: &str = "interface Input {\n  /** The PR link. */\n  pr: string;\n  /** Where to go when done. */\n  then: @;\n}\n";

/// Passes the callback on to `merge`.
fn handle() -> String {
    format!("{INPUT}/** Handle a PR. */\nexport default function handle({{ pr, then }}: Input) {{\n  return @./merge(pr=pr, then=then);\n}}\n")
}

/// Asks whether the PR merged, then returns the callback.
fn merge() -> String {
    format!(
        "{INPUT}/** Merge a PR. */\nexport default async function merge({{ pr, then }}: Input) {{\n  /** Did it merge? */\n  const merged: boolean = await request(`Merge ${{pr}}.`);\n  if (!merged) {{\n    return `Ask the user.`;\n  }}\n  return then;\n}}\n"
    )
}

const AFTER: &str = "/** After. */\nexport default function after() {\n  return `Pick the next PR.`;\n}\n";

static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Writes each `(file, text)` under a fresh temp folder for `name`, and returns the folder.
fn write(name: &str, files: &[(&str, String)]) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rein-callback-{}-{n}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for (file, text) in files {
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(file), text).unwrap();
    }
    dir
}

fn local() -> Project {
    let main = "/** Entry. */\nexport default function main() {\n  return @./handle(pr=`p`, then=@./after);\n}\n".to_string();
    let dir = write("local", &[("main.rein", main), ("handle.rein", handle()), ("merge.rein", merge()), ("after.rein", AFTER.to_string())]);
    Project::load(&dir).unwrap()
}

fn request_key(res: &StepResponse) -> &str {
    match res {
        StepResponse::Request { key, .. } => key,
        other => panic!("{other:?}"),
    }
}

fn traced(res: &StepResponse) -> Vec<&str> {
    match res {
        StepResponse::Question { trace, .. } | StepResponse::Request { trace, .. } | StepResponse::Done { trace, .. } | StepResponse::Error { trace, .. } => {
            trace.iter().map(String::as_str).collect()
        }
    }
}

fn with(answers: &[(&str, &str)]) -> StepRequest {
    let answers = answers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    StepRequest { version: PROTOCOL_VERSION, entry: "main".into(), answers, flow: None, inputs: BTreeMap::new() }
}

#[test]
fn a_local_callback_is_passed_on_and_followed() {
    let p = local();
    let res = step(&p, &with(&[]));
    assert_eq!(request_key(&res), "merge.merged");
    assert_eq!(traced(&res), ["main", "handle", "merge"]);
    let res = step(&p, &with(&[("merge.merged", "yes")]));
    assert!(matches!(&res, StepResponse::Done { instruction, .. } if instruction == "Pick the next PR."), "{res:?}");
    assert_eq!(traced(&res), ["main", "handle", "merge", "after"]);
}

#[test]
fn an_entry_takes_a_callback_as_an_input() {
    let p = local();
    let mut req = with(&[]);
    req.entry = "handle".into();
    req.inputs = BTreeMap::from([("pr".into(), "p".into()), ("then".into(), "@./after".into())]);
    assert_eq!(request_key(&step(&p, &req)), "merge.merged");
    req.inputs.insert("then".into(), "@./nope".into());
    let res = step(&p, &req);
    assert!(matches!(&res, StepResponse::Error { message, .. } if message == "input `then` is not valid; cannot find function `@./nope`"), "{res:?}");
}

#[test]
fn a_callback_cannot_be_answered() {
    let res = step(&local(), &with(&[("handle.then", "@./after")]));
    assert!(matches!(&res, StepResponse::Error { message, .. } if message.contains("is a callback")), "{res:?}");
}

/// The program `handler` picks a PR and hands it to `pr-handling`, with itself as the callback.
fn programs() -> Project {
    let handler = "interface Pick {\n  /** The PR link. */\n  pr: string;\n}\n/** Pick a PR. */\nexport default async function main() {\n  /** The next PR. */\n  const pick: Pick = await request(`Pick a PR.`);\n  return @pr-handling(pr=pick.pr, then=@handler);\n}\n".to_string();
    let pr_handling = format!("{INPUT}/** Handle a PR. */\nexport default function main({{ pr, then }}: Input) {{\n  return @./merge(pr=pr, then=then);\n}}\n");
    let handler = write("handler", &[("main.rein", handler)]);
    let pr = write("pr-handling", &[("main.rein", pr_handling), ("merge.rein", merge())]);
    let mut project = Project::load(&handler).unwrap();
    let find = move |name: &str| -> Result<(PathBuf, String), String> {
        let dir: &Path = match name {
            "handler" => &handler,
            "pr-handling" => &pr,
            _ => return Err(format!("unknown program `{name}`")),
        };
        Ok((dir.to_path_buf(), "main".into()))
    };
    project.link_programs(&find);
    project
}

#[test]
fn a_callback_goes_back_into_the_calling_program_with_fresh_answers() {
    let p = programs();
    let pick1 = r#"{"pr": "p1"}"#;
    let res = step(&p, &with(&[("main.pick", pick1)]));
    assert_eq!(request_key(&res), "@pr-handling/merge.merged");
    // The callback leaves pr-handling and enters the handler program, which asks for a new pick.
    let res = step(&p, &with(&[("main.pick", pick1), ("@pr-handling/merge.merged", "yes")]));
    assert_eq!(request_key(&res), "@handler/main.pick");
    assert_eq!(traced(&res), ["main", "@pr-handling/main", "@pr-handling/merge", "@handler/main"]);
    // The second pass through pr-handling has its own answers: it asks again, for the new PR.
    let pick2 = r#"{"pr": "p2"}"#;
    let res = step(&p, &with(&[("main.pick", pick1), ("@pr-handling/merge.merged", "yes"), ("@handler/main.pick", pick2)]));
    let StepResponse::Request { key, instruction, .. } = &res else { panic!("{res:?}") };
    assert_eq!(key, "@pr-handling/merge#2.merged");
    assert_eq!(instruction, "Merge p2.");
}
