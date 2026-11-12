//! Structs in the language server: which struct a function's parameter names, which functions take a
//! struct, and where its name and fields are written.

use rein_core::ast::{Struct, StructRef, STRUCT_SUFFIX};
use rein_core::diagnostic::Span;
use rein_core::project::Project;

use super::hover::input_sig;
use super::occurrences::{declaration, param_uses};

/// The struct that the parameter of `function` names: the file that declares it, and its name.
pub fn struct_of(project: &Project, function: &str) -> Option<(String, String)> {
    let p = project.get(function)?.function.struct_param()?;
    match &p.ty {
        StructRef::Local(n) => Some((function.to_string(), n.value.clone())),
        StructRef::Ref(r) => {
            let (file, s) = project.struct_for(function, &r.value).ok()?;
            Some((file, s.name.value.clone()))
        }
    }
}

/// The `interface` named `name` in the file `file`.
pub fn struct_decl<'a>(project: &'a Project, file: &str, name: &str) -> Option<&'a Struct> {
    project.get(file)?.function.structs.iter().find(|s| s.name.value == name)
}

/// The functions whose parameter is the struct `name` of `file`.
pub fn users(project: &Project, file: &str, name: &str) -> Vec<String> {
    project
        .functions()
        .map(|(n, _)| n.clone())
        .filter(|n| struct_of(project, n).is_some_and(|(f, s)| f == file && s == name))
        .collect()
}

/// Each parameter type that names the struct: the function, and the span of the type.
pub fn type_sites(project: &Project, file: &str, name: &str) -> Vec<(String, Span)> {
    users(project, file, name)
        .into_iter()
        .filter_map(|u| {
            let span = project.get(&u)?.function.struct_param()?.ty.name().span;
            Some((u, span))
        })
        .collect()
}

/// Every place field `field` of the struct is written, without its declaration: in `{ }` of each function
/// that takes the struct, its uses in that function, and `field=` in each call to that function.
pub fn field_sites(project: &Project, file: &str, name: &str, field: &str) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    for u in users(project, file, name) {
        if let Some(input) = project.get(&u).and_then(|sf| declaration(&sf.function, field)) {
            out.push((u.clone(), input.name.span));
        }
        out.extend(param_uses(project, &u, field));
    }
    out
}

/// The span of field `field` in its `interface`.
pub fn field_span(project: &Project, file: &str, name: &str, field: &str) -> Option<Span> {
    Some(struct_decl(project, file, name)?.field(field)?.name.span)
}

/// The struct's name, its doc, then each field with its type and question.
pub fn struct_markdown(s: &Struct) -> String {
    let mut out = format!("**interface {}**", s.name.value);
    if let Some(doc) = s.doc.as_ref().map(|d| d.description.value.as_str()).filter(|d| !d.is_empty()) {
        out.push_str(&format!("\n\n{doc}"));
    }
    for f in &s.fields {
        out.push_str(&format!("\n\n- `{}`: {}", input_sig(f), f.description.value));
    }
    out
}

/// A shared struct is named by its file: `ticket` for `billing/ticket.struct`.
pub fn base(file: &str) -> &str {
    let stem = file.strip_suffix(STRUCT_SUFFIX).unwrap_or(file);
    stem.rsplit('/').next().unwrap_or(stem)
}
