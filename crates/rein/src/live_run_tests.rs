use super::*;
use std::path::PathBuf;

const PICKER: &str = "interface Pick {\n  /** The link of the pull request. */\n  pr: string;\n  /** Which task runs. */\n  task: \"review\" | \"merge\";\n}\n/** Picks a PR. */\nexport default async function main() {\n  /** The pull request of the next notification. */\n  const pick: Pick = await request(`Pick the next pull request notification.`);\n  return `Do ${pick.task} on ${pick.pr}.`;\n}\n";

fn picker() -> Run {
    let project = Project::from_sources(PathBuf::from("/p"), vec![(PathBuf::from("/p/main.rein"), PICKER.to_string())]);
    Run::start(project, "main".into())
}

#[test]
fn a_struct_request_lists_each_field_with_type_options_and_description() {
    let run = picker();
    let lines = prompt_lines(run.last()).unwrap();
    assert_eq!(lines[0], "request: Pick the next pull request notification.");
    assert_eq!(lines[1], "then answer: main.pick: struct - The pull request of the next notification.");
    assert_eq!(lines[2], "  pr: string - The link of the pull request.");
    assert_eq!(lines[3], "  task: \"review\" | \"merge\" - Which task runs.");
    assert_eq!(lines.len(), 4);
}

#[test]
fn a_struct_answer_is_validated_like_a_step_answer() {
    let mut run = picker();
    for bad in ["not json", r#"{"pr":"u/1"}"#, r#"{"pr":"u/1","task":"review","x":"y"}"#, r#"{"pr":"u/1","task":"close"}"#, r#"{"pr":"","task":"merge"}"#] {
        assert!(matches!(run.answer(None, bad.into()), StepResponse::Error { .. }), "{bad}");
    }
    assert!(run.asked().is_some());
    let done = run.answer(None, r#"{"pr":"u/1","task":"merge"}"#.into());
    assert!(matches!(done, StepResponse::Done { .. }));
}

#[test]
fn a_json_answer_line_takes_an_object_value() {
    let (key, value) = parse_answer(r#"{"value":{"pr":"u/1","task":"merge"}}"#, true).unwrap();
    assert_eq!(key, None);
    assert_eq!(value, r#"{"pr":"u/1","task":"merge"}"#);
}
