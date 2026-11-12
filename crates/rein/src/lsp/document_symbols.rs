//! Document symbols, for the outline: the file's function, with its parameters and answers inside, then each
//! struct with its fields.

use lsp_types::{DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, Range, SymbolKind};

use super::occurrences::{declarations, header_span};
use super::text::{position, range};
use super::workspace::{locate, State};
use crate::cli::description;

pub fn document_symbols(state: &State, p: &DocumentSymbolParams) -> Option<DocumentSymbolResponse> {
    let (project, name) = locate(&state.docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let f = &sf.function;
    let mut out = Vec::new();
    if f.name.is_some() {
        let children = declarations(f)
            .into_iter()
            .map(|i| {
                let at = range(&sf.text, i.name.span);
                symbol(i.name.value.clone(), Some(i.ty.ts_type()), SymbolKind::VARIABLE, range(&sf.text, i.span), at, None)
            })
            .collect();
        let whole = Range::new(position(&sf.text, 0), position(&sf.text, sf.text.len()));
        let detail = Some(description(f)).filter(|d| !d.is_empty());
        let at = range(&sf.text, header_span(&project, &name));
        out.push(symbol(name, detail, SymbolKind::FUNCTION, whole, at, Some(children)));
    }
    for s in &f.structs {
        let fields = s
            .fields
            .iter()
            .map(|i| {
                let at = range(&sf.text, i.name.span);
                symbol(i.name.value.clone(), Some(i.ty.ts_type()), SymbolKind::FIELD, range(&sf.text, i.span), at, None)
            })
            .collect();
        let detail = s.doc.as_ref().map(|d| d.description.value.clone()).filter(|d| !d.is_empty());
        let at = range(&sf.text, s.name.span);
        out.push(symbol(s.name.value.clone(), detail, SymbolKind::STRUCT, range(&sf.text, s.span), at, Some(fields)));
    }
    Some(DocumentSymbolResponse::Nested(out))
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
