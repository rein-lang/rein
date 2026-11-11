//! Document symbols, for the outline: the file's function, with its parameters and answers inside.

use lsp_types::{DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, Range, SymbolKind};

use super::occurrences::{declarations, header_span};
use super::text::{position, range};
use super::workspace::{locate, State};
use crate::cli::description;

pub fn document_symbols(state: &State, p: &DocumentSymbolParams) -> Option<DocumentSymbolResponse> {
    let (project, name) = locate(&state.docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let children = declarations(&sf.function)
        .into_iter()
        .map(|i| {
            let at = range(&sf.text, i.name.span);
            symbol(i.name.value.clone(), Some(i.ty.ts_type()), SymbolKind::VARIABLE, range(&sf.text, i.span), at, None)
        })
        .collect();
    let whole = Range::new(position(&sf.text, 0), position(&sf.text, sf.text.len()));
    let detail = Some(description(&sf.function)).filter(|d| !d.is_empty());
    let at = range(&sf.text, header_span(&project, &name));
    Some(DocumentSymbolResponse::Nested(vec![symbol(name, detail, SymbolKind::FUNCTION, whole, at, Some(children))]))
}

#[allow(deprecated)] // `deprecated` is a field of the type, so it must be set.
fn symbol(
    name: String,
    detail: Option<String>,
    kind: SymbolKind,
    range: Range,
    selection_range: Range,
    children: Option<Vec<DocumentSymbol>>,
) -> DocumentSymbol {
    DocumentSymbol { name, detail, kind, tags: None, deprecated: None, range, selection_range, children }
}
