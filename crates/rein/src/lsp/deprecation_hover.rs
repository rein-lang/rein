//! Hover on a deprecated `@` reference or default import: the deprecation, the date it becomes an error, and the
//! named import to write. The text is the checker's warning, so the hover and the diagnostic always agree.

use rein_core::diagnostic::Span;
use rein_core::project::Project;

/// The markdown note for the deprecated form at `off` in file `name`, with the span it covers. `None` when `off`
/// is not in a deprecated form.
pub fn note(project: &Project, name: &str, off: usize) -> Option<(String, Span)> {
    let diag = project
        .check()
        .remove(name)?
        .into_iter()
        .find(|d| d.is_deprecated() && d.span.contains(off))?;
    Some((format!("**Deprecated**\n\n{}", diag.message), diag.span))
}
