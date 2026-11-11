//! Find All References: every place a function is referenced, or a parameter is used or passed by name.

use lsp_types::{Location, ReferenceParams};

use super::occurrences::{callers, declaration, header_span, param_uses, symbol_at, Symbol};
use super::text::offset;
use super::workspace::{locate, location, State};

/// The cursor may be on a reference, a parameter, or a function's name. Anywhere else in a file, it stands for
/// the function of that file.
pub fn references(state: &State, p: &ReferenceParams) -> Option<Vec<Location>> {
    let pos = &p.text_document_position;
    let (project, name) = locate(&state.docs, &pos.text_document.uri)?;
    let sf = project.get(&name)?;
    let symbol = symbol_at(&project, &name, offset(&sf.text, pos.position)).map_or(Symbol::Function(name.clone()), |(s, _)| s);
    let with_declaration = p.context.include_declaration;
    let sites = match &symbol {
        Symbol::Function(target) => {
            let mut sites = callers(&project, target);
            if with_declaration {
                sites.push((target.clone(), header_span(&project, target)));
            }
            sites
        }
        Symbol::Param { function, name: n } => {
            let mut sites = param_uses(&project, function, n);
            if with_declaration {
                let decl = declaration(&project.get(function)?.function, n)?;
                sites.push((function.clone(), decl.name.span));
            }
            sites
        }
        Symbol::Std(_) => return None,
    };
    Some(sites.into_iter().filter_map(|(f, span)| location(&state.docs, &project, &f, span)).collect())
}
