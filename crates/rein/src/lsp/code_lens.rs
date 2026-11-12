//! Code lens above each function: how many references it has. Clicking it lists them.

use lsp_types::{CodeLens, CodeLensParams, Command, Location};
use rein_core::ast::is_struct;
use serde_json::json;

use super::occurrences::{callers, header_span};
use super::structs::type_sites;
use super::text::range;
use super::workspace::{locate, location, State};

/// The client command that shows a list of locations. Its arguments are the file URI, the position, and the
/// locations, as LSP JSON.
const SHOW_REFERENCES: &str = "rein.showReferences";

pub fn code_lens(state: &State, p: &CodeLensParams) -> Option<Vec<CodeLens>> {
    let (project, name) = locate(&state.docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    // A `.struct.rein` file counts the parameter types that name its struct.
    let (at, sites) = match sf.function.exported_struct().filter(|_| is_struct(&name)) {
        Some(s) => (s.name.span, type_sites(&project, &name, &s.name.value)),
        None => (header_span(&project, &name), callers(&project, &name)),
    };
    let at = range(&sf.text, at);
    let locations: Vec<Location> =
        sites.into_iter().filter_map(|(f, span)| location(&state.docs, &project, &f, span)).collect();
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
