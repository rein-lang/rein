//! The function that a named import stands for, at the cursor: the name in an import line, or a use of the local name in
//! the file. Go to definition uses it to find the function the import names.

use rein_core::ast::Function;
use rein_core::diagnostic::Span;
use rein_core::lexer::{lex, Tok};
use rein_core::project::Project;

use super::occurrences::target;

/// The project path of the function that the named import at `off` names, and the span under the cursor. `None` when
/// `off` is not on a named import's name or on a use of its local name.
pub fn import_target(project: &Project, from: &str, text: &str, f: &Function, off: usize) -> Option<(String, Span)> {
    let named = f.imports.iter().filter(|i| !i.is_struct());
    // The name in the import line: the declared name, or the local name after `as`.
    for import in named.clone() {
        if import.local.span.contains(off) {
            return Some((target(project, from, import.reference())?, import.local.span));
        }
        if import.name.span.contains(off) {
            return Some((target(project, from, import.reference())?, import.name.span));
        }
    }
    // A use of the local name in the body.
    let (tok, span) = lex(text).ok()?.into_iter().find(|(_, s)| s.contains(off))?;
    let Tok::Ident(ident) = tok else { return None };
    let import = named.into_iter().find(|i| i.local.value == ident)?;
    Some((target(project, from, import.reference())?, span))
}
