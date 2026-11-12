//! Test fixture: a project that hands off to a linked program, built in memory.

use std::path::PathBuf;

use crate::project::{Linked, Project};

const TARGET: &str = "interface Input {\n  /** The PR link. */\n  pr: string;\n  /** Is the PR a draft? */\n  draft: boolean;\n  /** What to do. */\n  task: \"review\" | \"merge\";\n}\n/** Handle a PR. */\nexport default function main({ pr, draft, task }: Input) {\n  if (draft) {\n    return `Skip ${pr}.`;\n  }\n  return `Do ${task} on ${pr}.`;\n}\n";

/// A project whose `main(pr: string)` has `body`, and the linked program `pr-handling`, which takes `pr`,
/// `draft` and `task`.
pub fn with_program(body: &str) -> Project {
    with_main(format!("/** Handler. */\nexport default function main(\n  /** The PR link. */\n  pr: string,\n) {{\n  {body}\n}}\n"))
}

/// Like `with_program`, with `main.rein` written out in full.
pub fn with_main(main: String) -> Project {
    let mut project = Project::from_sources(PathBuf::from("/p"), vec![(PathBuf::from("/p/main.rein"), main)]);
    let target = Project::from_sources(PathBuf::from("/t"), vec![(PathBuf::from("/t/main.rein"), TARGET.to_string())]);
    let linked = Linked { project: Box::new(target), entry: "main".into() };
    project.programs.insert("pr-handling".into(), Ok(linked));
    project
}
