//! Hover on a removed `@` reference or default import: the removal and the named import to write. The text is the
//! checker's error, so the hover and the diagnostic always agree.

use rein_core::diagnostic::Span;
use rein_core::project::Project;

/// The markdown note for the removed form at `off` in file `name`, with the span it covers. `None` when `off`
/// is not in a removed form.
pub fn note(project: &Project, name: &str, off: usize) -> Option<(String, Span)> {
    let diag = project
        .check()
        .remove(name)?
        .into_iter()
        .find(|d| d.is_removed() && d.span.contains(off))?;
    Some((format!("**Removed**\n\n{}", diag.message), diag.span))
}
