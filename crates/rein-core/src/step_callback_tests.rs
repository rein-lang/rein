//! Stepping through callbacks: a function returns the function its caller passed, in one program and across programs.

use super::*;
use std::fs;
use std::path::PathBuf;

use crate::flows::{Flow, Scope};

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
    let find = move |name: &str| -> Result<Flow, String> {
        let root: PathBuf = match name {
            "handler" => handler.clone(),
            "pr-handling" => pr.clone(),
            _ => return Err(format!("unknown program `{name}`")),
        };
        Ok(Flow { name: name.into(), scope: Scope::User, root, description: String::new(), entry: "main".into(), shadowed_by: None, version: None, engines_rein: None })
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

/// A root `main` hands `p` to `pr-handling` with the root `after` as the callback. `pr-handling` has its own `after`.
#[test]
fn a_root_callback_runs_the_root_function_when_linked() {
    let main = "/** Entry. */\nexport default function main() {\n  return @pr-handling(pr=`p`, then=@./after);\n}\n";
    let root_after = "/** Root. */\nexport default function after() {\n  return `Root after.`;\n}\n";
    let root = write("root-after", &[("main.rein", main.to_string()), ("after.rein", root_after.to_string())]);
    let pr = write("pr-after", &[("main.rein", handle()), ("merge.rein", merge()), ("after.rein", AFTER.to_string())]);
    let mut project = Project::load(&root).unwrap();
    let find = move |name: &str| -> Result<Flow, String> {
        if name != "pr-handling" {
            return Err(format!("unknown program `{name}`"));
        }
        Ok(Flow { name: name.into(), scope: Scope::User, root: pr.clone(), description: String::new(), entry: "main".into(), shadowed_by: None, version: None, engines_rein: None })
    };
    project.link_programs(&find);
    assert_eq!(request_key(&step(&project, &with(&[]))), "@pr-handling/merge.merged");
    let res = step(&project, &with(&[("@pr-handling/merge.merged", "yes")]));
    assert!(matches!(&res, StepResponse::Done { instruction, .. } if instruction == "Root after."), "{res:?}");
    assert_eq!(traced(&res), ["main", "@pr-handling/main", "@pr-handling/merge", "after"]);
}

/// `main` hands `then=@./after` to `handle`, which returns it. `after` awaits `ask`, which asks `sure`.
fn awaiting() -> Project {
    let main = "/** Entry. */\nexport default function main() {\n  return @./handle(then=@./after);\n}\n";
    let handler = "interface Input {\n  /** Where to go when done. */\n  then: @;\n}\n/** Handle. */\nexport default function handle({ then }: Input) {\n  return then;\n}\n";
    let after = "/** After. */\nexport default async function after() {\n  /** Is it ok? */\n  const ok: boolean = await @./ask;\n  if (!ok) {\n    return `Stop.`;\n  }\n  return `CONTINUED`;\n}\n";
    let ask = "/** Ask. */\nexport default async function ask() {\n  /** Are you sure? */\n  const sure: boolean = await request(`Ask the user if sure.`);\n  return `Ask done.`;\n}\n";
    let files = [("main.rein", main), ("handle.rein", handler), ("after.rein", after), ("ask.rein", ask)];
    let dir = write("await", &files.map(|(f, t)| (f, t.to_string())));
    Project::load(&dir).unwrap()
}

/// An `await` reached only through a callback takes its answers, and the flow goes on past it.
#[test]
fn an_await_reached_through_a_callback_takes_answers() {
    let p = awaiting();
    assert_eq!(request_key(&step(&p, &with(&[]))), "after.ok>ask.sure");
    assert_eq!(request_key(&step(&p, &with(&[("after.ok>ask.sure", "yes")]))), "after.ok");
    let res = step(&p, &with(&[("after.ok>ask.sure", "yes"), ("after.ok", "yes")]));
    assert!(matches!(&res, StepResponse::Done { instruction, .. } if instruction == "CONTINUED"), "{res:?}");
}

/// The same when the entry gets the callback as an input: `handle` runs with `then=@./after`.
#[test]
fn an_await_reached_through_an_entry_callback_takes_answers() {
    let p = awaiting();
    let mut req = with(&[]);
    req.entry = "handle".into();
    req.inputs = BTreeMap::from([("then".into(), "@./after".into())]);
    assert_eq!(request_key(&step(&p, &req)), "after.ok>ask.sure");
    req.answers.insert("after.ok>ask.sure".into(), "yes".into());
    assert_eq!(request_key(&step(&p, &req)), "after.ok");
}
