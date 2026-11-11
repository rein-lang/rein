//! Code lens above each function: how many references it has. Clicking it lists them.

use lsp_types::{CodeLens, CodeLensParams, Command, Location};
use serde_json::json;

use super::occurrences::{callers, header_span};
use super::text::range;
use super::workspace::{locate, location, State};

/// The client command that shows a list of locations. Its arguments are the file URI, the position, and the
/// locations, as LSP JSON.
const SHOW_REFERENCES: &str = "rein.showReferences";

pub fn code_lens(state: &State, p: &CodeLensParams) -> Option<Vec<CodeLens>> {
    let (project, name) = locate(&state.docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let at = range(&sf.text, header_span(&project, &name));
    let locations: Vec<Location> =
        callers(&project, &name).into_iter().filter_map(|(f, span)| location(&state.docs, &project, &f, span)).collect();
    let title = match locations.len() {
        1 => "1 reference".to_string(),
        n => format!("{n} references"),
    };
    let command = Command {
        title,
        command: SHOW_REFERENCES.to_string(),
        arguments: Some(vec![json!(p.text_document.uri), json!(at.start), json!(locations)]),
    };
    Some(vec![CodeLens { range: at, command: Some(command), data: None }])
}
