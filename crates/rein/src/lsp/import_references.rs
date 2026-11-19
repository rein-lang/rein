//! The sites of named imports: the name in an import line, and each use of the local name in its file. References read
//! them. A function's references include every import of it, and a local name's references stay in its file.

use rein_core::ast::{Function, Import};
use rein_core::diagnostic::Span;
use rein_core::lexer::{lex, Tok};
use rein_core::project::Project;

use super::import_symbols::named_imports;
use super::occurrences::target;

/// Each use of the local name of `import` in the body of its file, in file order. An import line holds no use, and
/// neither does a `name=` argument of a call.
pub fn local_uses(text: &str, f: &Function, import: &Import) -> Vec<Span> {
    let Ok(toks) = lex(text) else { return Vec::new() };
    let mut out = Vec::new();
    for (i, (tok, span)) in toks.iter().enumerate() {
        if !matches!(tok, Tok::Ident(word) if *word == import.local.value) {
            continue;
        }
        let in_import_line = f
            .imports
            .iter()
            .any(|imp| span.start >= imp.span.start && span.end <= imp.span.end);
        let is_arg_name = matches!(toks.get(i + 1), Some((Tok::Assign, _)));
        if !in_import_line && !is_arg_name {
            out.push(*span);
        }
    }
    out
}

/// The sites of the named import with local name `local` in file `file`: its local name in the import line, and each
/// use in the file. Empty when the file has no such import.
pub fn local_sites(project: &Project, file: &str, local: &str) -> Vec<(String, Span)> {
    let Some(sf) = project.get(file) else { return Vec::new() };
    let Some(import) = named_imports(&sf.function).find(|i| i.local.value == local) else {
        return Vec::new();
    };
    let uses = local_uses(&sf.text, &sf.function, import);
    std::iter::once(import.local.span)
        .chain(uses)
        .map(|span| (file.to_string(), span))
        .collect()
}

/// Every use of a named import's local name in a file, with the file's function `f` and text. The parser also
/// reads each use as a reference to the function, so these spans are a subset of its references.
pub fn local_use_spans(f: &Function, text: &str) -> Vec<Span> {
    named_imports(f)
        .flat_map(|import| local_uses(text, f, import))
        .collect()
}

/// The sites of every named import that names function `to`: the name in each importing file's import line. Each use
/// of the import is a reference to `to` too, so `callers` lists the uses.
pub fn import_sites(project: &Project, to: &str) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    for (from, sf) in &project.files {
        for import in named_imports(&sf.function) {
            if target(project, from, import.reference()).as_deref() == Some(to) {
                out.push((from.clone(), import.name.span));
            }
        }
    }
    out
}
