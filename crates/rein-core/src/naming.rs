//! The snake_case rule for the names a function declares: its parameter, the fields of its structs, and its
//! answers. Paths are snake_case, so names are too, and an answer key such as `ci/fix.next_step` has one style.

use crate::ast::{walk_stmts, Function, Param, Stmt};
use crate::diagnostic::Diagnostic;

/// A warning for each declared name that holds an uppercase letter, with the snake_case name to use.
pub fn camel_case_names(f: &Function) -> Vec<Diagnostic> {
    let mut names = Vec::new();
    for p in &f.params {
        if let Param::Scalar(input) = p {
            names.push(&input.name);
        }
    }
    for s in &f.structs {
        names.extend(s.fields.iter().map(|field| &field.name));
    }
    walk_stmts(&f.body, &mut |s| {
        if let Stmt::Request { response, .. } = s {
            names.push(&response.name);
        }
    });
    names
        .into_iter()
        .filter(|n| n.value.chars().any(|c| c.is_ascii_uppercase()))
        .map(|n| {
            let msg = format!("`{}` is camelCase; names are snake_case: write `{}`", n.value, snake_case(&n.value));
            Diagnostic::warning(n.span, msg)
        })
        .collect()
}

/// `isWorkInProgress` becomes `is_work_in_progress`; a run of capitals stays one word: `prURL` becomes `pr_url`.
pub fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let prev_lower = i > 0 && (chars[i - 1].is_ascii_lowercase() || chars[i - 1].is_ascii_digit());
            let next_lower = chars.get(i + 1).is_some_and(char::is_ascii_lowercase);
            let prev_upper = i > 0 && chars[i - 1].is_ascii_uppercase();
            if i > 0 && !out.ends_with('_') && (prev_lower || (prev_upper && next_lower)) {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
#[path = "naming_tests.rs"]
mod tests;
