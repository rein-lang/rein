//! The LSP form of a deprecation warning: a tag that strikes the text through, the code, and a link to the docs.

use lsp_types::{CodeDescription, Diagnostic, DiagnosticTag, NumberOrString, Url};
use rein_core::diagnostic::Diagnostic as ReinDiagnostic;

/// The docs page that the code of a deprecation links to.
pub const DOCS_URL: &str = "https://rein-lang.com/docs/reference/roadmap";

/// Sets the code of `d` on `lsp`. A deprecation also gets the deprecated tag and the docs link.
pub fn mark(lsp: &mut Diagnostic, d: &ReinDiagnostic) {
    let Some(code) = d.code else { return };
    lsp.code = Some(NumberOrString::String(code.to_string()));
    if d.is_deprecated() {
        lsp.tags = Some(vec![DiagnosticTag::DEPRECATED]);
        lsp.code_description = Url::parse(DOCS_URL).ok().map(|href| CodeDescription { href });
    }
}
