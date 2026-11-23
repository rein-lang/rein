//! `rein migrate imports`: rewrites one file so each `@` reference and each default import becomes a named import.
//!
//! The rewrite works on tokens, so a `@` in a string or a comment stays. It edits by byte span, so every other byte of
//! the file stays. A file that needs no change comes back as it was, so a second run changes nothing.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Import, STRUCT_SUFFIX};
use crate::deprecation::program_or_path;
use crate::diagnostic::{Diagnostic, Span};
use crate::imports;
use crate::lexer::{lex, Tok};
use crate::parser;
use crate::project::Project;
use crate::schema_types::SCHEMA_SUFFIX;
use crate::stdlib;

/// The result of migrating one file.
#[derive(Debug, Clone, PartialEq)]
pub struct Migration {
    /// The file text after the migration. Equal to the input when nothing changes.
    pub text: String,
    /// One warning for each reference or default import that could not migrate. Each keeps its old form.
    pub warnings: Vec<Diagnostic>,
}

/// Migrates the file `from` (its project name, such as `ci/fix`) whose text is `src`, against the project `project`.
///
/// `project` must hold the target files and the linked programs of the file. Its own copy of `from` is not read.
pub fn migrate_text(project: &Project, from: &str, src: &str) -> Migration {
    migrate_selected(project, from, src, None)
}

/// Migrates only the `@` reference or the default import that holds `span`. Every other form keeps its old form.
/// `span` is the span of the warning on that form. The new names and import lines are the ones [`migrate_text`] gives.
pub fn migrate_one(project: &Project, from: &str, src: &str, span: Span) -> Migration {
    migrate_selected(project, from, src, Some(span))
}

fn migrate_selected(project: &Project, from: &str, src: &str, only: Option<Span>) -> Migration {
    let toks = match lex(src) {
        Ok(toks) => toks,
        Err(d) => return unchanged(src, vec![d]),
    };
    let mut pos = 0;
    let mut existing: Vec<(Import, Span)> = Vec::new();
    while toks[pos].0 == Tok::Import {
        let start = toks[pos].1;
        match imports::parse_import(&toks, &mut pos) {
            Ok(import) => existing.push((import, start.to(toks[pos - 1].1))),
            Err(d) => return unchanged(src, vec![d]),
        }
    }
    let body = pos;

    let own = parser::parse(src).0;
    let mut taken: BTreeSet<String> = stdlib::names().into_iter().map(String::from).collect();
    taken.extend(own.name.iter().map(|n| n.value.clone()));
    let own_declared: BTreeSet<String> = imports::own_names(&own).into_iter().map(|(name, _)| name).collect();
    taken.extend(own_declared.iter().cloned());
    taken.extend(existing.iter().map(|(i, _)| i.local.value.clone()));

    let mut warnings = Vec::new();
    // Target key -> the local name this file uses for it. Named imports already in the file come first.
    let mut chosen: BTreeMap<String, String> = BTreeMap::new();
    for (import, _) in existing.iter().filter(|(i, _)| !i.default) {
        if let Ok(target) = resolve(project, from, &import.path.value) {
            chosen.entry(target.key).or_insert_with(|| import.local.value.clone());
        }
    }

    let mut edits: Vec<(Span, String)> = Vec::new();
    // Old local name -> new local name, for the uses to rename.
    let mut renames: Vec<(String, String)> = Vec::new();
    for (import, stmt) in existing.iter().filter(|(i, _)| i.default).filter(|(_, stmt)| selected(only, *stmt)) {
        let target = match resolve(project, from, &import.path.value) {
            Ok(target) => target,
            Err(reason) => {
                let message = format!("`{}` is not migrated: {reason}", import.path.value);
                warnings.push(Diagnostic::warning(import.path.span, message));
                continue;
            }
        };
        let old = import.local.value.clone();
        if old == target.declared && own_declared.contains(&target.declared) {
            // The default name is a parameter, field, const or struct here, and its uses cannot be told apart.
            let message = format!(
                "`{}` is not migrated: `{old}` is also a parameter, field, const or struct of this file",
                import.path.value
            );
            warnings.push(Diagnostic::warning(import.path.span, message));
            continue;
        }
        if let Some(local) = chosen.get(&target.key).cloned() {
            // The file already names this target: the default line goes, and its uses take that name.
            edits.push((*stmt, String::new()));
            if local != old {
                renames.push((old, local));
            }
            continue;
        }
        let local = if old == target.declared || !taken.contains(&target.declared) {
            target.declared.clone()
        } else {
            unique(&local_base(program_or_path(&import.path.value)), &taken)
        };
        taken.insert(local.clone());
        chosen.insert(target.key, local.clone());
        edits.push((*stmt, import_line(&target.declared, &local, program_or_path(&import.path.value))));
        if local != old {
            renames.push((old, local));
        }
    }

    let mut new_lines = Vec::new();
    for (i, (tok, span)) in toks.iter().enumerate().skip(body) {
        let Tok::Ref(path) = tok else { continue };
        if !selected(only, *span) {
            continue;
        }
        let is_struct = i > 0 && toks[i - 1].0 == Tok::Colon;
        let import_path = if is_struct {
            format!("{path}{STRUCT_SUFFIX}")
        } else {
            path.clone()
        };
        let target = match resolve(project, from, &import_path) {
            Ok(target) => target,
            Err(reason) => {
                warnings.push(Diagnostic::warning(
                    *span,
                    format!("`@{path}` is not migrated: {reason}"),
                ));
                continue;
            }
        };
        let local = match chosen.get(&target.key) {
            Some(local) => local.clone(),
            None => {
                // An import of a program names the program, so `pkg/<entry>` writes `pkg`.
                let written = program_or_path(&import_path);
                let local = if taken.contains(&target.declared) {
                    unique(&local_base(written), &taken)
                } else {
                    target.declared.clone()
                };
                taken.insert(local.clone());
                chosen.insert(target.key, local.clone());
                new_lines.push(import_line(&target.declared, &local, written));
                local
            }
        };
        edits.push((*span, local));
    }

    for (tok, span) in toks.iter().skip(body) {
        if let Tok::Ident(name) = tok {
            if let Some((_, new)) = renames.iter().find(|(old, _)| old == name) {
                edits.push((*span, new.clone()));
            }
        }
    }

    if !new_lines.is_empty() {
        match existing.last() {
            Some((_, stmt)) => edits.push((Span::new(stmt.end, stmt.end), format!("\n{}", new_lines.join("\n")))),
            None => {
                let at = toks[body].1.start;
                edits.push((Span::new(at, at), format!("{}\n\n", new_lines.join("\n"))));
            }
        }
    }

    if edits.is_empty() {
        return unchanged(src, warnings);
    }
    // Apply from the end, so each span still points at the right bytes.
    edits.sort_by(|a, b| b.0.start.cmp(&a.0.start).then(b.0.end.cmp(&a.0.end)));
    let mut text = src.to_string();
    for (span, replacement) in edits {
        text.replace_range(span.start..span.end, &replacement);
    }
    Migration { text, warnings }
}

/// True when nothing is selected, or when the selected span lies inside `span`.
fn selected(only: Option<Span>, span: Span) -> bool {
    only.is_none_or(|o| span.start <= o.start && o.end <= span.end)
}

fn unchanged(src: &str, warnings: Vec<Diagnostic>) -> Migration {
    Migration {
        text: src.to_string(),
        warnings,
    }
}

/// What an import path names in the project: the key that identifies it, the name it declares, and the path to write.
struct Target {
    key: String,
    declared: String,
}

/// Resolves an import path, as written in `from`, to the name it declares. A path that starts with `./` or `../` is a
/// file of the project. A `.struct` path is a struct file. A `.schema` path is a schema file. Any other path names an
/// installed program.
fn resolve(project: &Project, from: &str, import_path: &str) -> Result<Target, String> {
    if !(import_path.starts_with("./") || import_path.starts_with("../")) {
        return resolve_program(project, import_path);
    }
    if import_path.ends_with(SCHEMA_SUFFIX) {
        // A schema is not a function, so its key is the schema file. The declared name is the schema title.
        let key = project.schema_key(from, import_path)?;
        let schema = project.schema_for(from, import_path)?;
        return Ok(Target {
            key,
            declared: schema.title.clone(),
        });
    }
    if let Some(base) = import_path.strip_suffix(STRUCT_SUFFIX) {
        let (key, s) = project.struct_for(from, base)?;
        return Ok(Target {
            key,
            declared: s.name.value.clone(),
        });
    }
    let key = project.resolve(from, import_path)?;
    let declared = project
        .get(&key)
        .and_then(|sf| sf.function.name.as_ref())
        .map(|n| n.value.clone())
        .ok_or_else(|| format!("`{import_path}` declares no function"))?;
    Ok(Target { key, declared })
}

/// Resolves `pr-handling` or `pr-handling/<entry>` to the entry function of the linked program.
fn resolve_program(project: &Project, import_path: &str) -> Result<Target, String> {
    let (program, rest) = match import_path.split_once('/') {
        Some((program, rest)) => (program, Some(rest)),
        None => (import_path, None),
    };
    let linked = match project.programs.get(program) {
        Some(Ok(linked)) => linked,
        Some(Err(reason)) => return Err(reason.clone()),
        None => return Err(format!("cannot find program `{program}`")),
    };
    if let Some(rest) = rest.filter(|r| *r != linked.entry) {
        return Err(format!(
            "`@{program}/{rest}` is not the entry of `{program}`; import `{program}` instead"
        ));
    }
    let declared = linked
        .project
        .files
        .get(&linked.entry)
        .and_then(|sf| sf.function.name.as_ref())
        .map(|n| n.value.clone())
        .ok_or_else(|| format!("program `{program}` declares no entry function"))?;
    Ok(Target {
        key: format!("@{program}"),
        declared,
    })
}

/// One import line. The braces hold the declared name; `as` is written only when the local name differs.
fn import_line(declared: &str, local: &str, path: &str) -> String {
    if declared == local {
        format!("import {{ {declared} }} from \"{path}\";")
    } else {
        format!("import {{ {declared} as {local} }} from \"{path}\";")
    }
}

/// The camelCase name of a path: `./ci/fix` gives `ciFix`, `pr-handling` gives `prHandling`, `../message.struct`
/// gives `message`, and `./ticket.schema` gives `ticket`.
fn local_base(path: &str) -> String {
    let base = path
        .strip_suffix(STRUCT_SUFFIX)
        .or_else(|| path.strip_suffix(SCHEMA_SUFFIX))
        .unwrap_or(path);
    let mut out = String::new();
    for segment in base
        .split(['/', '-', '.'])
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
    {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            if out.is_empty() {
                out.extend(first.to_lowercase());
            } else {
                out.extend(first.to_uppercase());
            }
            out.push_str(chars.as_str());
        }
    }
    out
}

/// `base`, or `base` with the first number from 2 that is free.
fn unique(base: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}{n}"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| base.to_string())
}

#[cfg(test)]
#[path = "migrate_imports_tests.rs"]
mod tests;
