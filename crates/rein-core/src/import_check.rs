//! The import checks that need the project: each import names what its target declares, and a warning for each
//! import that is not used or that `as` renames to the same name.

use crate::ast::{Binding, Function, Import};
use crate::diagnostic::{Diagnostic, Span};
use crate::imports::{missing_struct, own_names};
use crate::project::Project;

/// The errors and warnings of the imports of function `from`.
pub fn check(project: &Project, from: &str, f: &Function) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let own = own_names(f);
    for import in &f.imports {
        if let Some(d) = wrong_name(project, from, import) {
            out.push(d);
        }
        if let Some(d) = shadowed(import, &own) {
            out.push(d);
        }
        if !import.used {
            let message = format!("`{}` is imported but not used", import.local.value);
            out.push(Diagnostic::warning(import.span, message));
        }
        if import.aliased && import.name.value == import.local.value {
            let message = format!(
                "`{0} as {0}` keeps the name; write `import {{ {0} }} from \"{1}\"`",
                import.name.value, import.path.value
            );
            out.push(Diagnostic::warning(import.span, message));
        }
    }
    out
}

/// A warning when the local name of `import` is a parameter, a field, a `const` or a struct of the same file. Both
/// names then read the same, so the message gives an alias. It is a warning in rein 0.1.x, so installed programs
/// keep working in a patch release.
fn shadowed(import: &Import, own: &[(String, &str)]) -> Option<Diagnostic> {
    let local = &import.local.value;
    let (_, kind) = own.iter().find(|(name, _)| name == local)?;
    let declared = &import.name.value;
    let alias = open_name(local);
    let path = &import.path.value;
    let message = format!(
        "import `{local}` clashes with the {kind} `{local}`; give the import another name, like `import {{ {declared} as {alias} }} from \"{path}\"`; this becomes an error in rein 0.2.0"
    );
    Some(Diagnostic::warning(import.local.span, message))
}

/// `open` followed by `name` with its first letter capitalized, such as `openReview` for `review`.
fn open_name(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => format!("open{}{}", first.to_uppercase(), chars.as_str()),
        None => "open".to_string(),
    }
}

/// An error when the target of `import` does not resolve, or declares another name than the import gives.
fn wrong_name(project: &Project, from: &str, import: &Import) -> Option<Diagnostic> {
    let (span, message) = name_problem(project, from, import)?;
    Some(Diagnostic::error(span, message))
}

/// Why `import` of function `from` is wrong: where the error points and what it says. `None` when the import names
/// what its target declares. A program that a hand-off reaches checks its imports with this too, so the error reads
/// the same wherever it is reported.
pub fn name_problem(project: &Project, from: &str, import: &Import) -> Option<(Span, String)> {
    let declared = if import.is_struct() {
        match project.struct_for(from, import.reference()) {
            Ok((_, s)) => s.name.value.clone(),
            Err(_) => return Some((import.path.span, missing_struct(import))),
        }
    } else {
        match project.binding_for(from, import.reference()) {
            Ok(Binding::Function(target)) => declared_function(project, &target)?,
            Ok(Binding::Template { path: Some(target), .. }) => declared_function(project, &target)?,
            Ok(_) => return None,
            Err(reason) => return Some((import.path.span, reason)),
        }
    };
    (declared != import.name.value).then(|| {
        let path = &import.path.value;
        let message = format!(
            "`{path}` declares `{declared}`, not `{}`; write `import {{ {declared} }} from \"{path}\"`",
            import.name.value
        );
        (import.name.span, message)
    })
}

/// The name the function at project path `target` declares. `None` when that file has no function, such as a file
/// with a syntax error, which reports its own error.
fn declared_function(project: &Project, target: &str) -> Option<String> {
    project.get(target)?.function.name.as_ref().map(|n| n.value.clone())
}

#[cfg(test)]
#[path = "import_check_tests.rs"]
mod tests;
