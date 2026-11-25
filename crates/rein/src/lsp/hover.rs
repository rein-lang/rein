//! Hover: the doc of a function, a std item, a struct, or a parameter or field and its type.

use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Range, TextDocumentPositionParams};
use rein_core::ast::{Binding, Function, Input};
use rein_core::project::{Project, SourceFile};

use super::deprecation_hover;
use super::occurrences::{declaration, symbol_at, Symbol};
use super::structs::{struct_decl, struct_markdown};
use super::text::{offset, range, token_at};
use super::workspace::{locate, State};
use crate::cli::description;

pub fn hover(state: &State, p: &TextDocumentPositionParams) -> Option<Hover> {
    let (project, name) = locate(&state.docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let off = offset(&sf.text, p.position);
    let note = deprecation_hover::note(&project, &name, off);
    let target = target_markdown(&project, &name, sf, off);
    match (note, target) {
        (None, target) => target.map(|(value, range)| markdown(value, range)),
        (Some((note, span)), None) => Some(markdown(note, Some(range(&sf.text, span)))),
        (Some((note, _)), Some((value, range))) => Some(markdown(format!("{note}\n\n---\n\n{value}"), range)),
    }
}

/// The markdown of what is under `off`, with its range. `None` when there is nothing to show.
fn target_markdown(project: &Project, name: &str, sf: &SourceFile, off: usize) -> Option<(String, Option<Range>)> {
    let Some((symbol, span)) = symbol_at(project, name, off) else {
        // A file with a syntax error has a partial tree. Fall back to the name under the cursor.
        let token = token_at(&sf.text, off)?;
        let value = match sf.function.input(token).or_else(|| sf.function.response(token)) {
            Some(input) => param_markdown(input),
            None => std_markdown(token)?,
        };
        return Some((value, None));
    };
    let value = match symbol {
        Symbol::Function(target) | Symbol::ImportLocal { target, .. } => {
            let doc = function_markdown(&target, &project.get(&target)?.function);
            match target.strip_prefix('@').and_then(|t| t.split_once('/')) {
                Some((program, _)) => format!("{}\n\n---\n\n{doc}", program_markdown(project, program)?),
                None => doc,
            }
        }
        Symbol::Param { function, name: n } => param_markdown(declaration(&project.get(&function)?.function, &n)?),
        Symbol::Struct { file, name: n } => struct_markdown(struct_decl(project, &file, &n)?),
        Symbol::Field { file, strukt, name: n } => param_markdown(struct_decl(project, &file, &strukt)?.field(&n)?),
        Symbol::Std(n) => std_markdown(&n)?,
    };
    Some((value, Some(range(&sf.text, span))))
}

fn markdown(value: String, range: Option<Range>) -> Hover {
    Hover { contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }), range }
}

/// The program `name` as the project links it: its version, scope and folder.
fn program_markdown(project: &Project, name: &str) -> Option<String> {
    let flow = &project.programs.get(name)?.as_ref().ok()?.flow;
    let version = flow.version.as_ref().map_or_else(String::new, |v| format!(" {v}"));
    Some(format!("program `{name}`{version}, {} scope\n\n`{}`", flow.scope.label(), flow.root.display()))
}

fn std_markdown(n: &str) -> Option<String> {
    match rein_core::stdlib::item(n).ok()? {
        Binding::Request => Some(format!("**request**\n\n{}", rein_core::stdlib::request_doc())),
        Binding::Template { function, .. } => Some(function_markdown(n, function)),
        Binding::Function(_) => None,
    }
}

/// The function's name, its description, then each parameter with its type and question.
pub fn function_markdown(name: &str, f: &Function) -> String {
    let mut out = format!("**{name}**\n\n{}", description(f));
    for input in &f.inputs {
        out.push_str(&format!("\n\n- `{}`: {}", input_sig(input), input.description.value));
    }
    out
}

fn param_markdown(input: &Input) -> String {
    format!("`{}`\n\n{}", input_sig(input), input.description.value)
}

/// `name: boolean` or `name: "a" | "b"`, as TypeScript writes it.
pub fn input_sig(input: &Input) -> String {
    format!("{}: {}", input.name.value, input.ty.ts_type())
}
