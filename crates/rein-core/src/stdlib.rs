//! The standard library. Import it with `import { name } from "std";`.

use std::sync::OnceLock;

use crate::ast::{Binding, Function};
use crate::parser::parse;

/// The module name in `import { name } from "std"`.
pub const MODULE: &str = "std";

/// `request` is built in. It is not a template, so the checker and evaluator handle it.
const REQUEST: &str = "request";

/// The std templates. Each name is the file's function name, and the file is part of the binary.
const TEMPLATES: &[(&str, &str)] = &[
    ("askUser", include_str!("../std/askUser.rein")),
    ("delegate", include_str!("../std/delegate.rein")),
];

const REQUEST_DOC: &str = "Ask the agent to do a step, then wait for its answer. Use it in an async function, as `/** question */ const ok: boolean = await request(\"...\");`. The answer is a parameter for the rest of its block.";

fn templates() -> &'static [(&'static str, Function)] {
    static PARSED: OnceLock<Vec<(&'static str, Function)>> = OnceLock::new();
    PARSED.get_or_init(|| {
        TEMPLATES
            .iter()
            .map(|(name, src)| {
                let (f, diags) = parse(src);
                assert!(diags.is_empty(), "std `{name}` does not parse: {diags:?}");
                (*name, f)
            })
            .collect()
    })
}

/// Every std name, sorted.
pub fn names() -> Vec<&'static str> {
    let mut names: Vec<&str> = templates().iter().map(|(n, _)| *n).chain(std::iter::once(REQUEST)).collect();
    names.sort_unstable();
    names
}

/// What `import { name } from "std"` binds. Unknown names are an error that lists the std names.
pub fn item(name: &str) -> Result<Binding<'static>, String> {
    if name == REQUEST {
        return Ok(Binding::Request);
    }
    match templates().iter().find(|(n, _)| *n == name) {
        Some((_, function)) => Ok(Binding::Template { path: None, function }),
        None => Err(format!("`{name}` is not in std; std has: {}", names().join(", "))),
    }
}

/// The source text of a std template. None for `request`, which is built in, and for unknown names.
pub fn source(name: &str) -> Option<&'static str> {
    TEMPLATES.iter().find(|(n, _)| *n == name).map(|(_, src)| *src)
}

/// The text of `request`, for hover.
pub fn request_doc() -> &'static str {
    REQUEST_DOC
}

/// The description of a std name, from its doc comment. Empty when it has none.
pub fn doc(name: &str) -> String {
    if name == REQUEST {
        return REQUEST_DOC.to_string();
    }
    templates()
        .iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, f)| f.description.as_ref())
        .map_or_else(String::new, |d| d.value.clone())
}

#[cfg(test)]
#[path = "stdlib_tests.rs"]
mod tests;
