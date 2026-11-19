//! The LSP form of a removed-form error: the code and a link to the roadmap that names the removal.

use lsp_types::{CodeDescription, Diagnostic, NumberOrString, Url};
use rein_core::diagnostic::Diagnostic as ReinDiagnostic;

/// The docs page that the code of a removed form links to.
pub const DOCS_URL: &str = "https://rein-lang.com/docs/reference/roadmap";

/// Sets the code of `d` on `lsp`. A removed form also gets the docs link.
pub fn mark(lsp: &mut Diagnostic, d: &ReinDiagnostic) {
    let Some(code) = d.code else { return };
    lsp.code = Some(NumberOrString::String(code.to_string()));
    if d.is_removed() {
        lsp.code_description = Url::parse(DOCS_URL).ok().map(|href| CodeDescription { href });
    }
}
