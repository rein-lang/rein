//! Signature help inside call parentheses: the callee's parameters, with the current one marked.

use lsp_types::{
    Documentation, ParameterInformation, ParameterLabel, SignatureHelp, SignatureHelpParams, SignatureInformation,
};

use super::call_context::{call_at, callee_params};
use super::hover::input_sig;
use super::text::offset;
use super::workspace::{locate, State};

pub fn signature_help(state: &State, p: &SignatureHelpParams) -> Option<SignatureHelp> {
    let pos = &p.text_document_position_params;
    let (project, name) = locate(&state.docs, &pos.text_document.uri)?;
    let sf = project.get(&name)?;
    let call = call_at(&sf.text, offset(&sf.text, pos.position))?;
    let params = callee_params(&project, &name, &call.callee)?;
    let shown = if call.callee.starts_with('.') { format!("@{}", call.callee) } else { call.callee.clone() };
    let mut label = format!("{shown}(");
    let mut parameters = Vec::new();
    for (i, input) in params.iter().enumerate() {
        if i > 0 {
            label.push_str(", ");
        }
        // Offsets count UTF-16 units, as LSP positions do.
        let start = label.encode_utf16().count() as u32;
        label.push_str(&input_sig(input));
        parameters.push(ParameterInformation {
            label: ParameterLabel::LabelOffsets([start, label.encode_utf16().count() as u32]),
            documentation: Some(Documentation::String(input.description.value.clone())),
        });
    }
    label.push(')');
    let named = call.current.as_ref().and_then(|n| params.iter().position(|i| &i.name.value == n));
    let active = named.unwrap_or(call.index) as u32;
    let signature =
        SignatureInformation { label, documentation: None, parameters: Some(parameters), active_parameter: Some(active) };
    Some(SignatureHelp { signatures: vec![signature], active_signature: Some(0), active_parameter: Some(active) })
}
