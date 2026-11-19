//! The import checks that need the project: each import names what its target declares, and a warning for each
//! import that is not used or that `as` renames to the same name.

use crate::ast::{Binding, Function, Import};
use crate::diagnostic::Diagnostic;
use crate::project::Project;

/// The errors and warnings of the imports of function `from`.
pub fn check(project: &Project, from: &str, f: &Function) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for import in &f.imports {
        if let Some(d) = wrong_name(project, from, import) {
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

/// An error when the target of `import` does not resolve, or declares another name than the import gives.
fn wrong_name(project: &Project, from: &str, import: &Import) -> Option<Diagnostic> {
    let declared = if import.is_struct() {
        match project.struct_for(from, import.reference()) {
            Ok((_, s)) => s.name.value.clone(),
            Err(reason) => return Some(Diagnostic::error(import.path.span, reason)),
        }
    } else {
        match project.binding_for(from, import.reference()) {
            Ok(Binding::Function(target)) => declared_function(project, &target)?,
            Ok(Binding::Template { path: Some(target), .. }) => declared_function(project, &target)?,
            Ok(_) => return None,
            Err(reason) => return Some(Diagnostic::error(import.path.span, reason)),
        }
    };
    (declared != import.name.value).then(|| {
        let path = &import.path.value;
        let message = format!(
            "`{path}` declares `{declared}`, not `{}`; write `import {{ {declared} }} from \"{path}\"`",
            import.name.value
        );
        Diagnostic::error(import.name.span, message)
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
