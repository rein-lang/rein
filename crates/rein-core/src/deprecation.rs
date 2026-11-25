//! The deprecation warnings of the `@` references and the default imports. Both work until
//! [`IMPORT_REMOVAL_DATE`], then they are errors. A reference that is an import use is not a `@` reference, so it
//! gets no warning here: the lexer finds only the `@` tokens.

use crate::ast::{Binding, Import};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::{self, Tok};
use crate::project::{Project, SourceFile};
use crate::schema_types::SCHEMA_SUFFIX;

/// The date when `@` references and default imports become errors. The one place that holds the date.
pub const IMPORT_REMOVAL_DATE: &str = "2026-12-19";

/// The deprecation warnings of file `from`: one for each `@` reference and one for each default import.
pub fn check(project: &Project, from: &str, sf: &SourceFile) -> Vec<Diagnostic> {
    let Ok(tokens) = lexer::lex(&sf.text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, (tok, span)) in tokens.iter().enumerate() {
        let Tok::Ref(path) = tok else {
            continue;
        };
        // A struct type follows `:`, such as `({ a }: @./message)`. Any other reference is a function.
        let struct_type = i > 0 && tokens[i - 1].0 == Tok::Colon;
        out.push(reference_warning(project, from, path, *span, struct_type));
    }
    for import in sf.function.imports.iter().filter(|i| i.default) {
        out.push(default_warning(project, from, import));
    }
    out
}

/// The warning for one `@` reference. It suggests the named import that replaces it.
fn reference_warning(project: &Project, from: &str, path: &str, span: Span, struct_type: bool) -> Diagnostic {
    let is_struct = struct_type && !is_program(path);
    let import_path = if is_struct {
        format!("{path}.struct")
    } else {
        program_or_path(path).to_string()
    };
    let name = declared_name(project, from, path, is_struct).unwrap_or_else(|| fallback_name(path, is_struct));
    let message = format!(
        "`@{path}` is deprecated and becomes an error on {IMPORT_REMOVAL_DATE}; write `import {{ {name} }} from \"{import_path}\";` and use `{name}`. Run `rein migrate imports` to change every file"
    );
    Diagnostic::deprecated(span, message)
}

/// The warning for one default import. It suggests the named import with the name the target declares.
fn default_warning(project: &Project, from: &str, import: &Import) -> Diagnostic {
    let reference = import.reference();
    let name = declared_name(project, from, reference, import.is_struct()).unwrap_or_else(|| import.name.value.clone());
    let path = &import.path.value;
    let message = format!(
        "default imports are deprecated and become an error on {IMPORT_REMOVAL_DATE}; write `import {{ {name} }} from \"{path}\"`"
    );
    Diagnostic::deprecated(import.span, message)
}

/// The name that the target of `reference` declares, or `None` when the target does not resolve.
fn declared_name(project: &Project, from: &str, reference: &str, is_struct: bool) -> Option<String> {
    if reference.ends_with(SCHEMA_SUFFIX) {
        return project.schema_for(from, reference).ok().map(|s| s.title.clone());
    }
    if is_program(reference) {
        let Some(Ok(linked)) = project.programs.get(program_or_path(reference)) else {
            return None;
        };
        return linked
            .project
            .get(&linked.entry)?
            .function
            .name
            .as_ref()
            .map(|n| n.value.clone());
    }
    if is_struct {
        return project
            .struct_for(from, reference)
            .ok()
            .map(|(_, s)| s.name.value.clone());
    }
    match project.binding_for(from, reference).ok()? {
        Binding::Function(target) | Binding::Template { path: Some(target), .. } => {
            project.get(&target)?.function.name.as_ref().map(|n| n.value.clone())
        }
        _ => None,
    }
}

/// The name to suggest when the target does not resolve: the last path segment. A struct is capitalized, and a
/// program is camel-cased from its name, so `pr-handling` gives `prHandling`.
fn fallback_name(path: &str, is_struct: bool) -> String {
    if is_program(path) {
        return camel_case(program_or_path(path));
    }
    let last = path
        .split('/')
        .rev()
        .find(|s| !s.is_empty() && *s != "." && *s != "..")
        .unwrap_or(path);
    if is_struct {
        capitalize(last)
    } else {
        last.to_string()
    }
}

/// A bare path, such as `pr-handling`, names an installed program. A `./` or `../` path names a file.
pub(crate) fn is_program(path: &str) -> bool {
    !(path.starts_with("./") || path.starts_with("../"))
}

/// The import path of a program reference: the program name, without the function after `/`.
pub(crate) fn program_or_path(path: &str) -> &str {
    if is_program(path) {
        path.split('/').next().unwrap_or(path)
    } else {
        path
    }
}

/// `pr-handling` becomes `prHandling`.
fn camel_case(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in name.chars() {
        if c == '-' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
#[path = "deprecation_tests.rs"]
mod tests;
