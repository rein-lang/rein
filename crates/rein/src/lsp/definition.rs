//! Go to definition: a reference goes to its function, a parameter name to its declaration, a struct type or a
//! field name to its `interface`, and a std name to its source.

use std::env;
use std::fs;
use std::path::PathBuf;

use lsp_types::{Location, TextDocumentPositionParams, Url};

use super::occurrences::{declaration, header_span, name_span, symbol_at, Symbol};
use super::structs::{field_span, struct_decl};
use super::text::{offset, range};
use super::workspace::{canonical, locate, location, State};

pub fn definition(state: &State, p: &TextDocumentPositionParams) -> Option<Location> {
    let (project, name) = locate(&state.docs, &p.text_document.uri)?;
    let sf = project.get(&name)?;
    let (symbol, _) = symbol_at(&project, &name, offset(&sf.text, p.position))?;
    match symbol {
        Symbol::Function(target) => location(&state.docs, &project, &target, header_span(&project, &target)),
        Symbol::Param { function, name: n } => {
            let input = declaration(&project.get(&function)?.function, &n)?;
            location(&state.docs, &project, &function, input.name.span)
        }
        Symbol::Struct { file, name: n } => {
            location(&state.docs, &project, &file, struct_decl(&project, &file, &n)?.name.span)
        }
        Symbol::Field { file, strukt, name: n } => {
            location(&state.docs, &project, &file, field_span(&project, &file, &strukt, &n)?)
        }
        Symbol::Std(n) => std_location(&n),
    }
}

/// Where std sources are written for the editor to open. The server publishes no diagnostics for them.
pub fn std_dir() -> PathBuf {
    canonical(env::temp_dir().join("rein-std"))
}

/// A std template's source, written to a temp file so the editor can open it. `request` has no source.
fn std_location(n: &str) -> Option<Location> {
    let src = rein_core::stdlib::source(n)?;
    let dir = std_dir();
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{n}.rein"));
    fs::write(&path, src).ok()?;
    Some(Location::new(Url::from_file_path(&path).ok()?, range(src, name_span(src).unwrap_or_default())))
}
