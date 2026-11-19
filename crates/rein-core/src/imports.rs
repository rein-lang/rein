//! The `import { name } from "path";` form: one import line, the rules a line holds on its own, and the lookups
//! that let a local name stand for the `@` reference it replaces.

use crate::ast::{walk_stmts, Function, Import, Param, Spanned, Stmt, StructRef, STRUCT_SUFFIX};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::Tok;
use crate::stdlib;

const FORM: &str = "write `import { name } from \"path\";`";
const ONE_NAME: &str = "the braces hold one name; a file declares one name, so write one `import` line for each name";

/// Parses one `import` line. `pos` points at the `import` keyword, and moves past the line when the line is valid.
pub fn parse_import(toks: &[(Tok, Span)], pos: &mut usize) -> Result<Import, Diagnostic> {
    let start = toks[*pos].1;
    *pos += 1;
    let (head, head_span) = at(toks, *pos);
    let next = at(toks, *pos + 1).0;
    let (name, local, aliased, default) = match head {
        Tok::Str(_) => {
            return Err(Diagnostic::error(
                head_span,
                format!("a side-effect import is not supported; {FORM}"),
            ));
        }
        Tok::Star => {
            return Err(Diagnostic::error(
                head_span,
                format!("`import *` is not supported; {FORM}"),
            ))
        }
        Tok::Ident(word) if word == "type" && next != Tok::From => {
            return Err(Diagnostic::error(
                head_span,
                format!("`import type` is not supported; {FORM}"),
            ));
        }
        Tok::LBrace => {
            *pos += 1;
            let name = name_token(toks, pos)?;
            let (local, aliased) = if is_word(toks, *pos, "as") {
                *pos += 1;
                (name_token(toks, pos)?, true)
            } else {
                (name.clone(), false)
            };
            if at(toks, *pos).0 == Tok::Comma {
                *pos += 1;
                if at(toks, *pos).0 != Tok::RBrace {
                    return Err(Diagnostic::error(at(toks, *pos).1, ONE_NAME));
                }
            }
            expect(toks, pos, &Tok::RBrace, "`}`")?;
            (name, local, aliased, false)
        }
        Tok::Ident(_) => {
            let local = name_token(toks, pos)?;
            (local.clone(), local, false, true)
        }
        other => {
            return Err(Diagnostic::error(
                head_span,
                format!("expected `{{` or a name after `import`, found {}", other.describe()),
            ));
        }
    };
    expect(toks, pos, &Tok::From, "`from`")?;
    let (path, path_span) = match at(toks, *pos) {
        (Tok::Str(path), span) => {
            *pos += 1;
            (path, span)
        }
        (other, span) => {
            return Err(Diagnostic::error(
                span,
                format!(
                    "expected a path in quotes, such as \"./ci/fix\", found {}",
                    other.describe()
                ),
            ));
        }
    };
    if let Some(message) = path_error(&path) {
        return Err(Diagnostic::error(path_span, message));
    }
    if at(toks, *pos).0 == Tok::Semi {
        *pos += 1;
    }
    Ok(Import {
        name,
        local,
        path: Spanned {
            value: path,
            span: path_span,
        },
        default,
        aliased,
        span: start.to(path_span),
        used: false,
    })
}

/// Adds `import` to `imports`, or errors when its local name is a std name or is already imported.
pub fn add(imports: &mut Vec<Import>, import: Import) -> Result<(), Diagnostic> {
    let std = stdlib::names();
    for part in [&import.name, &import.local] {
        if std.contains(&part.value.as_str()) {
            return Err(Diagnostic::error(
                part.span,
                format!("`{}` is a std name; std names need no import", part.value),
            ));
        }
    }
    if imports.iter().any(|i| i.local.value == import.local.value) {
        let message = format!(
            "`{}` is imported twice; each local name is one import",
            import.local.value
        );
        return Err(Diagnostic::error(import.local.span, message));
    }
    imports.push(import);
    Ok(())
}

/// An error when an import's local name is also a function or struct declared in the same file.
pub fn clash(imports: &[Import], f: &Function) -> Option<Diagnostic> {
    imports
        .iter()
        .find(|i| {
            f.name.as_ref().is_some_and(|n| n.value == i.local.value)
                || f.structs.iter().any(|s| s.name.value == i.local.value)
        })
        .map(|i| {
            Diagnostic::error(
                i.local.span,
                format!("`{}` is already declared in this file", i.local.value),
            )
        })
}

/// The names that the file's own declarations hold, each with its kind for a message: each parameter and each field
/// of a struct parameter, each `const` answer, and each struct. An import's local name must not be one of them.
pub fn own_names(f: &Function) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for param in &f.params {
        match param {
            Param::Scalar(input) => out.push((input.name.value.clone(), "parameter")),
            Param::Struct(s) => out.extend(s.fields.iter().map(|field| (field.value.clone(), "parameter"))),
        }
    }
    walk_stmts(&f.body, &mut |s| {
        if let Stmt::Request { response, .. } = s {
            out.push((response.name.value.clone(), "const"));
        }
    });
    out.extend(f.structs.iter().map(|s| (s.name.value.clone(), "struct")));
    out
}

/// Whether `name` is the local name of an import.
pub fn is_import(imports: &[Import], name: &str) -> bool {
    imports.iter().any(|i| i.local.value == name)
}

/// The reference that the call or return of `name` names: the path of its import, as a `@` reference writes it.
/// Marks the import used. `name` must be an import (see [`is_import`]). A struct import names a type, so it errors.
pub fn reference(imports: &mut [Import], name: &str, span: Span) -> Result<Spanned<String>, Diagnostic> {
    let Some(import) = imports.iter_mut().find(|i| i.local.value == name) else {
        return Ok(Spanned {
            value: name.to_string(),
            span,
        });
    };
    import.used = true;
    if import.is_struct() {
        let message =
            format!("`{name}` is a struct, not a function; use it as a parameter type, such as `({{ a }}: {name})`");
        return Err(Diagnostic::error(span, message));
    }
    Ok(Spanned {
        value: import.reference().to_string(),
        span,
    })
}

/// The struct that the parameter or answer type `name` names: the `.struct.rein` file of its import. Marks the import
/// used. `name` must be an import (see [`is_import`]). A function import names no type, so it errors.
pub fn struct_ref(imports: &mut [Import], name: &str, span: Span) -> Result<StructRef, Diagnostic> {
    let Some(import) = imports.iter_mut().find(|i| i.local.value == name) else {
        return Ok(StructRef::Local(Spanned {
            value: name.to_string(),
            span,
        }));
    };
    import.used = true;
    if !import.is_struct() {
        let message =
            format!("`{name}` is a function, not a struct; a parameter type names a struct, such as `{{ a }}: Name`");
        return Err(Diagnostic::error(span, message));
    }
    Ok(StructRef::Ref(Spanned {
        value: import.reference().to_string(),
        span,
    }))
}

/// The error for a named struct import whose `.struct.rein` file is missing. It names the import form, and the file to add.
pub fn missing_struct(import: &Import) -> String {
    let base = import.reference().rsplit('/').next().unwrap_or_default();
    format!(
        "cannot find struct `{}`; add a `{base}.struct.rein` file that holds `export default interface {}`",
        import.path.value, import.name.value
    )
}

/// Why a path is not valid in an import, or `None` when it is. A relative path starts with `./` or `../`, names
/// folders and a file with letters, digits and `_`, and may end with `.struct`. Any other path names a program.
pub fn path_error(path: &str) -> Option<String> {
    if let Some(stem) = path.strip_suffix(".rein") {
        return Some(format!("an import has no `.rein` extension; write `\"{stem}\"`"));
    }
    if !(path.starts_with("./") || path.starts_with("../")) {
        if path.contains('/') {
            let program = path.split('/').next().unwrap_or(path);
            return Some(format!(
                "`\"{path}\"` names a function inside a program; import the program, such as `\"{program}\"`, \
                 and call its entry: https://rein-lang.com/docs/call-programs-by-entry"
            ));
        }
        let name_ok = path.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if path.is_empty() || path.starts_with('-') || !name_ok {
            return Some(
                "a program is imported by its name, such as `\"pr-handling\"`; a file path starts with `./` or `../`"
                    .into(),
            );
        }
        return None;
    }
    let base = path.strip_suffix(STRUCT_SUFFIX).unwrap_or(path);
    let segments: Vec<&str> = base.split('/').collect();
    let names = &segments[1..];
    let ok = !names.is_empty()
        && names
            .iter()
            .all(|s| *s == ".." || (!s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')))
        && !matches!(names.last(), Some(&"..") | None);
    (!ok).then(|| "a path is `\"./path\"` or `\"../path\"`, with names made of letters, digits and `_`".to_string())
}

fn at(toks: &[(Tok, Span)], pos: usize) -> (Tok, Span) {
    toks[pos.min(toks.len() - 1)].clone()
}

fn is_word(toks: &[(Tok, Span)], pos: usize, word: &str) -> bool {
    matches!(&at(toks, pos).0, Tok::Ident(w) if w == word)
}

fn expect(toks: &[(Tok, Span)], pos: &mut usize, want: &Tok, expected: &str) -> Result<(), Diagnostic> {
    let (tok, span) = at(toks, *pos);
    if tok != *want {
        return Err(Diagnostic::error(
            span,
            format!("expected {expected}, found {}", tok.describe()),
        ));
    }
    *pos += 1;
    Ok(())
}

/// One name in the braces, or the default name. A name is one word, so `a.b` is an error.
fn name_token(toks: &[(Tok, Span)], pos: &mut usize) -> Result<Spanned<String>, Diagnostic> {
    match at(toks, *pos) {
        (Tok::Ident(value), span) if !value.contains('.') => {
            *pos += 1;
            Ok(Spanned { value, span })
        }
        (Tok::Ident(_), span) => Err(Diagnostic::error(
            span,
            "a name is one word; a `.` reads a field of a struct answer",
        )),
        (other, span) => Err(Diagnostic::error(
            span,
            format!("expected a name, found {}", other.describe()),
        )),
    }
}

#[cfg(test)]
#[path = "imports_tests.rs"]
mod tests;
