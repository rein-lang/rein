//! Hover: the doc of a function, a std item, or a parameter and its type.

use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Range, TextDocumentPositionParams};
use rein_core::ast::{Binding, Function, Input};

use super::occurrences::{declaration, symbol_at, Symbol};
use super::text::{offset, range, token_at};
use super::workspace::{locate, State};
use crate::cli::description;

pub fn hover(state: &State, p: &TextDocumentPositionParams) -> Option<Hover> {
    let (project, name) = locate(&state.docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let off = offset(&sf.text, p.position);
    let Some((symbol, span)) = symbol_at(&project, &name, off) else {
        // A file with a syntax error has a partial tree. Fall back to the name under the cursor.
        let token = token_at(&sf.text, off)?;
        let value = match sf.function.input(token).or_else(|| sf.function.response(token)) {
            Some(input) => param_markdown(input),
            None => std_markdown(token)?,
        };
        return Some(markdown(value, None));
    };
    let value = match symbol {
        Symbol::Function(target) => function_markdown(&target, &project.get(&target)?.function),
        Symbol::Param { function, name: n } => param_markdown(declaration(&project.get(&function)?.function, &n)?),
        Symbol::Std(n) => std_markdown(&n)?,
    };
    Some(markdown(value, Some(range(&sf.text, span))))
}

fn markdown(value: String, range: Option<Range>) -> Hover {
    Hover { contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }), range }
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
