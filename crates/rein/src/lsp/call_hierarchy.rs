//! Call hierarchy: the functions that reference a function, and the functions it references.

use std::collections::BTreeMap;

use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem, CallHierarchyOutgoingCall,
    CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams, Range, SymbolKind,
};
use rein_core::diagnostic::Span;
use rein_core::project::Project;

use super::occurrences::{callers, header_span, refs, symbol_at, target, Symbol};
use super::text::{offset, position, range};
use super::workspace::{locate, uri_for, Docs, State};
use crate::cli::description;

/// The function referenced at the cursor. Anywhere else, the function of the file.
pub fn prepare(state: &State, p: &CallHierarchyPrepareParams) -> Option<Vec<CallHierarchyItem>> {
    let pos = &p.text_document_position_params;
    let (project, name) = locate(&state.docs, &pos.text_document.uri)?;
    let sf = project.get(&name)?;
    let target = match symbol_at(&project, &name, offset(&sf.text, pos.position)) {
        Some((Symbol::Function(target), _)) => target,
        _ => name,
    };
    Some(vec![item(&state.docs, &project, &target)?])
}

/// Each function that references the item, with the spans of its references.
pub fn incoming(state: &State, p: &CallHierarchyIncomingCallsParams) -> Option<Vec<CallHierarchyIncomingCall>> {
    let (project, name) = locate(&state.docs, &p.item.uri)?;
    let mut by_caller: BTreeMap<String, Vec<Span>> = BTreeMap::new();
    for (caller, span) in callers(&project, &name) {
        by_caller.entry(caller).or_default().push(span);
    }
    by_caller
        .into_iter()
        .map(|(caller, spans)| {
            let text = &project.get(&caller)?.text;
            Some(CallHierarchyIncomingCall {
                from: item(&state.docs, &project, &caller)?,
                from_ranges: spans.iter().map(|s| range(text, *s)).collect(),
            })
        })
        .collect()
}

/// Each function the item references, with the spans of those references in the item's file.
pub fn outgoing(state: &State, p: &CallHierarchyOutgoingCallsParams) -> Option<Vec<CallHierarchyOutgoingCall>> {
    let (project, name) = locate(&state.docs, &p.item.uri)?;
    let sf = project.get(&name)?;
    let mut by_target: BTreeMap<String, Vec<Span>> = BTreeMap::new();
    for (r, _) in refs(&sf.function) {
        if let Some(to) = target(&project, &name, &r.value) {
            by_target.entry(to).or_default().push(r.span);
        }
    }
    by_target
        .into_iter()
        .map(|(target, spans)| {
            Some(CallHierarchyOutgoingCall {
                to: item(&state.docs, &project, &target)?,
                from_ranges: spans.iter().map(|s| range(&sf.text, *s)).collect(),
            })
        })
        .collect()
}

/// The item of function `name`: its whole file, with its name selected.
fn item(docs: &Docs, project: &Project, name: &str) -> Option<CallHierarchyItem> {
    let sf = project.get(name)?;
    Some(CallHierarchyItem {
        name: name.to_string(),
        kind: SymbolKind::FUNCTION,
        tags: None,
        detail: Some(description(&sf.function)).filter(|d| !d.is_empty()),
        uri: uri_for(docs, &sf.path)?,
        range: Range::new(position(&sf.text, 0), position(&sf.text, sf.text.len())),
        selection_range: range(&sf.text, header_span(project, name)),
        data: None,
    })
}
