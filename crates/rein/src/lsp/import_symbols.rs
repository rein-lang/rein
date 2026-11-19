//! The named imports of a file, and the one at the cursor: its name in an import line, or a use of its local name in
//! the file. Definition, references and rename read it.

use rein_core::ast::{Function, Import};
use rein_core::diagnostic::Span;
use rein_core::lexer::{lex, Tok};

/// The imports of `f` that name a function: `import { name } from "path";` and the default form. A shared struct is
/// not one.
pub fn named_imports(f: &Function) -> impl Iterator<Item = &Import> {
    f.imports.iter().filter(|i| !i.is_struct())
}

/// The named import at `off` in `f`, the span under the cursor, and whether that span is the import's local name.
/// `None` when `off` is not on a named import's name or on a use of its local name.
pub fn named_at<'a>(f: &'a Function, text: &str, off: usize) -> Option<(&'a Import, Span, bool)> {
    // The name in the import line: the declared name, or the local name after `as`.
    for import in named_imports(f) {
        if import.local.span.contains(off) {
            return Some((import, import.local.span, true));
        }
        if import.name.span.contains(off) {
            return Some((import, import.name.span, false));
        }
    }
    // A use of the local name in the body.
    let (tok, span) = lex(text).ok()?.into_iter().find(|(_, s)| s.contains(off))?;
    let Tok::Ident(ident) = tok else { return None };
    named_imports(f).find(|i| i.local.value == ident).map(|i| (i, span, true))
}
