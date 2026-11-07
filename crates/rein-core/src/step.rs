//! Stateless stepping of a whole flow, for agents and any other caller.
//!
//! Each call takes the project and every answer so far. It returns the next question, the next
//! request, the final instruction, or an error. Nothing is kept between calls. Stepping is pure:
//! no I/O, no clock, no randomness. Loading the project is I/O and stays outside this module.
//!
//! The types here are the wire protocol, version [`PROTOCOL_VERSION`]. A CLI, a daemon, or an MCP or
//! JSON-RPC server sends a [`StepRequest`] and returns a [`StepResponse`] unchanged. A loaded
//! [`Project`] is `Send + Sync`, so a daemon can load it once and step many requests.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::ast::{Input, InputType};
use crate::diagnostic::{line_col, Severity};
use crate::eval::{self, Outcome};
use crate::project::{Project, ENTRY};

pub const PROTOCOL_VERSION: u32 = 1;

/// Most functions one call may visit. A flow that goes past it is treated as a reference cycle.
const HOP_LIMIT: usize = 64;

/// The answers so far, keyed by `<function path>.<name>`, such as `merge_pr.ci` or `answer/faq.solved`.
pub type Answers = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRequest {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Function path to start from. Default `main`.
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub answers: Answers,
}

fn default_version() -> u32 {
    PROTOCOL_VERSION
}

fn default_entry() -> String {
    ENTRY.to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepResponse {
    /// A parameter is needed. Answer `key`, then step again.
    Question {
        version: u32,
        key: String,
        function: String,
        name: String,
        r#type: String,
        options: Option<Vec<String>>,
        description: String,
        answers: Answers,
        trace: Vec<String>,
    },
    /// The agent must do `instruction`, then answer `response`, then step again.
    Request {
        version: u32,
        key: String,
        function: String,
        instruction: String,
        response: ResponseSpec,
        answers: Answers,
        trace: Vec<String>,
    },
    /// The flow ended. `instruction` is what the agent does next.
    Done {
        version: u32,
        instruction: String,
        answers: Answers,
        trace: Vec<String>,
    },
    Error {
        version: u32,
        message: String,
        /// The answer key the message is about, when there is one.
        key: Option<String>,
        diagnostics: Vec<StepDiagnostic>,
        answers: Answers,
        trace: Vec<String>,
    },
}

/// The answer a request asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseSpec {
    pub name: String,
    pub r#type: String,
    pub options: Option<Vec<String>>,
    pub description: String,
}

/// One error from the checker. `file` is the project path, such as `ci/fix.rein`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepDiagnostic {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// Steps the flow from `req.entry` as far as the answers allow.
pub fn step(project: &Project, req: &StepRequest) -> StepResponse {
    let answers = req.answers.clone();
    if req.version != PROTOCOL_VERSION {
        return invalid_request(format!(
            "unsupported protocol version {}; this build speaks {PROTOCOL_VERSION}",
            req.version
        ));
    }
    let errors = error_diagnostics(project);
    if !errors.is_empty() {
        let message = format!(
            "project has {} error(s); run `rein check` and fix them",
            errors.len()
        );
        return failure(message, None, errors, answers, Vec::new());
    }
    if let Err((key, message)) = check_answers(project, &req.answers) {
        return failure(message, Some(key), Vec::new(), answers, Vec::new());
    }
    let Some(entry) = project.get(&req.entry) else {
        return failure(
            format!("unknown function `{}`", req.entry),
            None,
            Vec::new(),
            answers,
            Vec::new(),
        );
    };
    if entry.function.is_template() {
        let message = "templates are called from functions, not run";
        return failure(message, None, Vec::new(), answers, Vec::new());
    }

    let mut trace = Vec::new();
    let mut current = req.entry.clone();
    for _ in 0..HOP_LIMIT {
        trace.push(current.clone());
        let Some(sf) = project.get(&current) else {
            return failure(
                format!("unknown function `{current}`"),
                None,
                Vec::new(),
                answers,
                trace,
            );
        };
        let inputs = inputs_of(&current, &req.answers);
        let outcome = match eval::run(&sf.function, &inputs, &project.bindings(&current)) {
            Ok(outcome) => outcome,
            Err(message) => return failure(message, None, Vec::new(), answers, trace),
        };
        match outcome {
            // Auto-follow: the agent does not need to see a hop that asks nothing.
            Outcome::Ref(target) => current = target,
            Outcome::Text(instruction) => {
                return StepResponse::Done {
                    version: PROTOCOL_VERSION,
                    instruction,
                    answers,
                    trace,
                };
            }
            Outcome::NeedInput(name) => {
                let Some(input) = sf.function.input(&name) else {
                    return failure(
                        format!("unknown input `{name}`"),
                        None,
                        Vec::new(),
                        answers,
                        trace,
                    );
                };
                let (r#type, options) = type_of(input);
                return StepResponse::Question {
                    version: PROTOCOL_VERSION,
                    key: format!("{current}.{name}"),
                    function: current.clone(),
                    name,
                    r#type,
                    options,
                    description: input.description.value.clone(),
                    answers,
                    trace,
                };
            }
            Outcome::Request {
                instruction,
                response,
            } => {
                let Some(input) = sf.function.response(&response) else {
                    return failure(
                        format!("unknown answer `{response}`"),
                        None,
                        Vec::new(),
                        answers,
                        trace,
                    );
                };
                let (r#type, options) = type_of(input);
                let spec = ResponseSpec {
                    name: response.clone(),
                    r#type,
                    options,
                    description: input.description.value.clone(),
                };
                return StepResponse::Request {
                    version: PROTOCOL_VERSION,
                    key: format!("{current}.{response}"),
                    function: current.clone(),
                    instruction,
                    response: spec,
                    answers,
                    trace,
                };
            }
        }
    }
    let message = format!("flow did not finish in {HOP_LIMIT} steps (reference cycle?)");
    failure(message, None, Vec::new(), answers, trace)
}

fn failure(
    message: impl Into<String>,
    key: Option<String>,
    diagnostics: Vec<StepDiagnostic>,
    answers: Answers,
    trace: Vec<String>,
) -> StepResponse {
    StepResponse::Error {
        version: PROTOCOL_VERSION,
        message: message.into(),
        key,
        diagnostics,
        answers,
        trace,
    }
}

/// The wire type name and the options of an input.
fn type_of(input: &Input) -> (String, Option<Vec<String>>) {
    let options = input
        .ty
        .options()
        .map(|o| o.into_iter().map(str::to_string).collect());
    let name = match input.ty {
        InputType::Bool => "boolean",
        InputType::Choice(_) => "union",
        InputType::Text => "string",
    };
    (name.to_string(), options)
}

/// Errors of every file and of the project, in a fixed order. Warnings are left out.
fn error_diagnostics(project: &Project) -> Vec<StepDiagnostic> {
    let mut out = Vec::new();
    for (name, diags) in project.check() {
        let Some(sf) = project.get(&name) else {
            continue;
        };
        for d in diags.iter().filter(|d| d.severity == Severity::Error) {
            let (line, column) = line_col(&sf.text, d.span.start);
            let file = format!("{name}.rein");
            out.push(StepDiagnostic {
                file,
                line: line + 1,
                column: column + 1,
                message: d.message.clone(),
            });
        }
    }
    for d in project.project_diagnostics() {
        let file = "rein.toml".to_string();
        out.push(StepDiagnostic {
            file,
            line: 1,
            column: 1,
            message: d.message,
        });
    }
    out
}

/// The `(key, message)` of the first answer that names no parameter or answer, or has a bad value.
fn check_answers(project: &Project, answers: &Answers) -> Result<(), (String, String)> {
    for (key, raw) in answers {
        let Some(input) = declared(project, key) else {
            return Err((key.clone(), format!("unknown key `{key}`")));
        };
        if input.ty == InputType::Text {
            return Err((
                key.clone(),
                format!("`{key}` is a string parameter; only a template takes one"),
            ));
        }
        eval::check_value(input, raw).map_err(|message| (key.clone(), message))?;
    }
    Ok(())
}

/// The parameter or `const` answer that `key` names. Names have no dots, so the last dot splits the key.
fn declared<'a>(project: &'a Project, key: &str) -> Option<&'a Input> {
    let (function, name) = key.rsplit_once('.')?;
    let f = &project.get(function)?.function;
    f.input(name).or_else(|| f.response(name))
}

/// The answers that belong to `function`, keyed by the bare name the evaluator uses.
fn inputs_of(function: &str, answers: &Answers) -> HashMap<String, String> {
    answers
        .iter()
        .filter_map(|(key, raw)| {
            let (owner, name) = key.rsplit_once('.')?;
            (owner == function).then(|| (name.to_string(), raw.clone()))
        })
        .collect()
}

/// A response for a request that cannot be stepped, such as bad JSON. It carries no answers.
pub fn invalid_request(message: impl Into<String>) -> StepResponse {
    StepResponse::Error {
        version: PROTOCOL_VERSION,
        message: message.into(),
        key: None,
        diagnostics: Vec::new(),
        answers: Answers::new(),
        trace: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// Writes `files` into a unique temp project, loads it, and removes the directory.
    fn project(files: &[(&str, &str)]) -> Project {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("rein-step-{}-{nanos}-{n}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("rein.toml"), "").unwrap();
        for (rel, text) in files {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let loaded = Project::load(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();
        loaded
    }

    fn answers(pairs: &[(&str, &str)]) -> Answers {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn req(entry: &str, pairs: &[(&str, &str)]) -> StepRequest {
        StepRequest {
            version: PROTOCOL_VERSION,
            entry: entry.into(),
            answers: answers(pairs),
        }
    }

    fn trace(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// `main` asks for `vip`, then hands off to `escalate` or returns text.
    fn vip_project() -> Project {
        project(&[
            ("main.rein", "/** Entry. */\nexport default function main(\n  /** Is the customer on the VIP plan? */\n  vip: boolean,\n) {\n  if (vip) {\n    return @./escalate;\n  }\n  return \"Answer the ticket.\";\n}\n"),
            ("escalate.rein", "/** Escalate. */\nexport default function escalate() {\n  return \"Assign the ticket to tier 2.\";\n}\n"),
        ])
    }

    /// `main` picks a path. `merge` goes to `merge_pr`, which asks two questions.
    fn merge_project() -> Project {
        project(&[
            ("main.rein", "/** Entry. */\nexport default function main(\n  /** Which path? */\n  path: \"merge\" | \"review\",\n) {\n  switch (path) {\n    case \"merge\":\n      return @./merge_pr;\n    case \"review\":\n      return \"Review the code.\";\n  }\n}\n"),
            ("merge_pr.rein", "/** Merge a PR. */\nexport default function mergePr(\n  /** Does the PR change only docs? */\n  docsOnly: boolean,\n  /** What is the CI status? */\n  ci: \"green\" | \"red\",\n) {\n  if (docsOnly) {\n    return \"approved\";\n  }\n  switch (ci) {\n    case \"green\":\n      return \"merge it\";\n    case \"red\":\n      return \"wait for CI\";\n  }\n}\n"),
        ])
    }

    /// `main` asks `articleFound` through `answer/faq`, which asks a request, then escalates.
    fn faq_project() -> Project {
        project(&[
            ("main.rein", "/** Entry. */\nexport default function main() {\n  return @./answer/faq;\n}\n"),
            ("answer/faq.rein", "/** Answer a how-to question. */\nexport default async function answerFaq(\n  /** Did you find an article? */\n  articleFound: boolean,\n) {\n  if (!articleFound) {\n    return @../escalate;\n  }\n  /** Did the article solve it? */\n  const solved: boolean = await request(\"Send the article and ask if it solved the problem.\");\n  if (solved) {\n    return \"Close the ticket as solved.\";\n  }\n  return @../escalate;\n}\n"),
            ("escalate.rein", "/** Escalate. */\nexport default function escalate() {\n  return \"Assign the ticket to tier 2.\";\n}\n"),
        ])
    }

    #[test]
    fn question_at_main_when_no_answers() {
        let res = step(&vip_project(), &req("main", &[]));
        assert_eq!(
            res,
            StepResponse::Question {
                version: 1,
                key: "main.vip".into(),
                function: "main".into(),
                name: "vip".into(),
                r#type: "boolean".into(),
                options: None,
                description: "Is the customer on the VIP plan?".into(),
                answers: answers(&[]),
                trace: trace(&["main"]),
            }
        );
    }

    #[test]
    fn answers_for_main_auto_follow_into_merge_pr() {
        let res = step(&merge_project(), &req("main", &[("main.path", "merge")]));
        assert_eq!(
            res,
            StepResponse::Question {
                version: 1,
                key: "merge_pr.docsOnly".into(),
                function: "merge_pr".into(),
                name: "docsOnly".into(),
                r#type: "boolean".into(),
                options: None,
                description: "Does the PR change only docs?".into(),
                answers: answers(&[("main.path", "merge")]),
                trace: trace(&["main", "merge_pr"]),
            }
        );
    }

    #[test]
    fn union_question_lists_its_options() {
        let res = step(
            &merge_project(),
            &req(
                "main",
                &[("main.path", "merge"), ("merge_pr.docsOnly", "no")],
            ),
        );
        let StepResponse::Question {
            key,
            r#type,
            options,
            ..
        } = res
        else {
            panic!("{res:?}")
        };
        assert_eq!(key, "merge_pr.ci");
        assert_eq!(r#type, "union");
        assert_eq!(options, Some(vec!["green".to_string(), "red".to_string()]));
    }

    #[test]
    fn full_path_is_done_with_the_trace() {
        let res = step(
            &merge_project(),
            &req(
                "main",
                &[
                    ("main.path", "merge"),
                    ("merge_pr.docsOnly", "no"),
                    ("merge_pr.ci", "green"),
                ],
            ),
        );
        assert_eq!(
            res,
            StepResponse::Done {
                version: 1,
                instruction: "merge it".into(),
                answers: answers(&[
                    ("main.path", "merge"),
                    ("merge_pr.docsOnly", "no"),
                    ("merge_pr.ci", "green")
                ]),
                trace: trace(&["main", "merge_pr"]),
            }
        );
    }

    #[test]
    fn async_request_stops_then_the_answer_continues() {
        let project = faq_project();
        let res = step(
            &project,
            &req("main", &[("answer/faq.articleFound", "yes")]),
        );
        assert_eq!(
            res,
            StepResponse::Request {
                version: 1,
                key: "answer/faq.solved".into(),
                function: "answer/faq".into(),
                instruction: "Send the article and ask if it solved the problem.".into(),
                response: ResponseSpec {
                    name: "solved".into(),
                    r#type: "boolean".into(),
                    options: None,
                    description: "Did the article solve it?".into(),
                },
                answers: answers(&[("answer/faq.articleFound", "yes")]),
                trace: trace(&["main", "answer/faq"]),
            }
        );

        let res = step(
            &project,
            &req(
                "main",
                &[
                    ("answer/faq.articleFound", "yes"),
                    ("answer/faq.solved", "yes"),
                ],
            ),
        );
        assert_eq!(
            res,
            StepResponse::Done {
                version: 1,
                instruction: "Close the ticket as solved.".into(),
                answers: answers(&[
                    ("answer/faq.articleFound", "yes"),
                    ("answer/faq.solved", "yes")
                ]),
                trace: trace(&["main", "answer/faq"]),
            }
        );

        let res = step(
            &project,
            &req(
                "main",
                &[
                    ("answer/faq.articleFound", "yes"),
                    ("answer/faq.solved", "no"),
                ],
            ),
        );
        let StepResponse::Done {
            instruction,
            trace: t,
            ..
        } = res
        else {
            panic!("{res:?}")
        };
        assert_eq!(instruction, "Assign the ticket to tier 2.");
        assert_eq!(t, trace(&["main", "answer/faq", "escalate"]));
    }

    #[test]
    fn unknown_key_is_an_error() {
        let res = step(&vip_project(), &req("main", &[("main.nope", "x")]));
        let StepResponse::Error { message, key, .. } = res else {
            panic!("{res:?}")
        };
        assert_eq!(message, "unknown key `main.nope`");
        assert_eq!(key, Some("main.nope".into()));
    }

    #[test]
    fn key_without_a_function_is_an_error() {
        let res = step(&vip_project(), &req("main", &[("vip", "yes")]));
        let StepResponse::Error { message, .. } = res else {
            panic!("{res:?}")
        };
        assert_eq!(message, "unknown key `vip`");
    }

    #[test]
    fn invalid_boolean_value_is_an_error() {
        let res = step(&vip_project(), &req("main", &[("main.vip", "maybe")]));
        let StepResponse::Error { message, key, .. } = res else {
            panic!("{res:?}")
        };
        assert_eq!(
            message,
            "invalid value `maybe` for `vip`; expected true, false, yes or no"
        );
        assert_eq!(key, Some("main.vip".into()));
    }

    #[test]
    fn invalid_union_value_lists_the_accepted_values() {
        let res = step(&merge_project(), &req("main", &[("main.path", "purple")]));
        let StepResponse::Error { message, .. } = res else {
            panic!("{res:?}")
        };
        assert!(
            message.contains("expected one of: `merge`, `review`"),
            "{message}"
        );
    }

    #[test]
    fn string_parameter_cannot_be_answered() {
        let project = project(&[
            ("main.rein", "/** Entry. */\nexport default function main() {\n  return @./greet(\"world\");\n}\n"),
            ("greet.rein", "/** Greet. */\nexport default function greet(\n  /** Who? */\n  who: string,\n) {\n  return `Hello ${who}`;\n}\n"),
        ]);
        assert_eq!(
            step(&project, &req("main", &[])),
            StepResponse::Done {
                version: 1,
                instruction: "Hello world".into(),
                answers: answers(&[]),
                trace: trace(&["main"])
            }
        );
        let res = step(&project, &req("main", &[("greet.who", "x")]));
        let StepResponse::Error { key, .. } = res else {
            panic!("{res:?}")
        };
        assert_eq!(key, Some("greet.who".into()));
    }

    #[test]
    fn same_parameter_name_in_two_functions_gets_separate_keys() {
        let project = project(&[
            ("main.rein", "/** Entry. */\nexport default function main(\n  /** CI? */\n  ci: \"green\" | \"red\",\n) {\n  if (ci === \"green\") {\n    return @./merge_pr;\n  }\n  return \"Wait.\";\n}\n"),
            ("merge_pr.rein", "/** Merge. */\nexport default function mergePr(\n  /** CI for the merge? */\n  ci: \"green\" | \"red\",\n) {\n  switch (ci) {\n    case \"green\":\n      return `Merge ${ci}`;\n    case \"red\":\n      return \"Stop.\";\n  }\n}\n"),
        ]);
        // The answer to main.ci does not answer merge_pr.ci.
        let res = step(&project, &req("main", &[("main.ci", "green")]));
        let StepResponse::Question { key, .. } = res else {
            panic!("{res:?}")
        };
        assert_eq!(key, "merge_pr.ci");

        let res = step(
            &project,
            &req("main", &[("main.ci", "green"), ("merge_pr.ci", "green")]),
        );
        let StepResponse::Done { instruction, .. } = res else {
            panic!("{res:?}")
        };
        assert_eq!(instruction, "Merge green");
    }

    #[test]
    fn reference_cycle_hits_the_hop_limit() {
        let project = project(&[
            (
                "main.rein",
                "/** Entry. */\nexport default function main() {\n  return @./a;\n}\n",
            ),
            (
                "a.rein",
                "/** A. */\nexport default function a() {\n  return @./b;\n}\n",
            ),
            (
                "b.rein",
                "/** B. */\nexport default function b() {\n  return @./a;\n}\n",
            ),
        ]);
        let res = step(&project, &req("main", &[]));
        let StepResponse::Error {
            message, trace: t, ..
        } = res
        else {
            panic!("{res:?}")
        };
        assert_eq!(
            message,
            "flow did not finish in 64 steps (reference cycle?)"
        );
        assert_eq!(t.len(), HOP_LIMIT);
    }

    #[test]
    fn template_as_entry_is_an_error() {
        let project = project(&[
            ("main.rein", "/** Entry. */\nexport default function main() {\n  return \"x\";\n}\n"),
            ("greet.rein", "/** Greet. */\nexport default function greet(\n  /** Who? */\n  who: string,\n) {\n  return `Hello ${who}`;\n}\n"),
        ]);
        let res = step(&project, &req("greet", &[]));
        let StepResponse::Error { message, .. } = res else {
            panic!("{res:?}")
        };
        assert_eq!(message, "templates are called from functions, not run");
    }

    #[test]
    fn project_with_errors_is_refused() {
        let project = project(&[(
            "main.rein",
            "/** Entry. */\nexport default function main() {\n  return @./missing;\n}\n",
        )]);
        let res = step(&project, &req("main", &[]));
        let StepResponse::Error {
            message,
            diagnostics,
            ..
        } = res
        else {
            panic!("{res:?}")
        };
        assert_eq!(
            message,
            "project has 1 error(s); run `rein check` and fix them"
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].file, "main.rein");
        assert!(
            diagnostics[0].message.contains("cannot find function"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn unknown_entry_is_an_error() {
        let res = step(&vip_project(), &req("nope", &[]));
        let StepResponse::Error { message, .. } = res else {
            panic!("{res:?}")
        };
        assert_eq!(message, "unknown function `nope`");
    }

    #[test]
    fn request_defaults_to_version_one_and_main() {
        let parsed: StepRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(
            parsed,
            StepRequest {
                version: 1,
                entry: "main".into(),
                answers: answers(&[])
            }
        );
    }

    #[test]
    fn other_protocol_version_is_refused() {
        let mut r = req("main", &[]);
        r.version = 2;
        let res = step(&vip_project(), &r);
        let StepResponse::Error { message, .. } = res else {
            panic!("{res:?}")
        };
        assert_eq!(
            message,
            "unsupported protocol version 2; this build speaks 1"
        );
    }

    #[test]
    fn same_request_gives_byte_equal_responses() {
        let project = merge_project();
        let forward = req(
            "main",
            &[("main.path", "merge"), ("merge_pr.docsOnly", "no")],
        );
        let mut backward = req("main", &[]);
        backward
            .answers
            .insert("merge_pr.docsOnly".into(), "no".into());
        backward.answers.insert("main.path".into(), "merge".into());
        let a = serde_json::to_string(&step(&project, &forward)).unwrap();
        let b = serde_json::to_string(&step(&project, &forward)).unwrap();
        let c = serde_json::to_string(&step(&project, &backward)).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn project_and_protocol_types_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Project>();
        assert_send_sync::<StepRequest>();
        assert_send_sync::<StepResponse>();
    }
}
