//! The formats that a `string` input must have, because the input reaches a format parameter.
//!
//! A `string` input that is passed to an `IRI` (or other format) parameter, by a hand-off or a template call, must
//! hold a value of that format. A format input has its own format. The walk and the answer check both use this.

use std::collections::BTreeSet;

use crate::ast::{Awaited, Binding, Call, Function, Input, InputType, ReturnValue, Stmt, Value, walk_stmts};
use crate::formats::Format;
use crate::project::Project;

/// The formats that the value of `input`, an input of `function`, must have. A format input has its own format. A
/// `string` input has the format of each parameter it reaches, directly or through the parameters of the functions
/// it is passed to. Any other input has none.
pub fn formats_of(project: &Project, function: &str, input: &Input) -> Vec<Format> {
    match input.ty {
        InputType::Format(format) => vec![format],
        InputType::Text => {
            let mut out = Vec::new();
            reached(project, function, &input.name.value, &mut BTreeSet::new(), &mut out);
            out
        }
        InputType::Bool | InputType::Choice(_) | InputType::Struct(_) => Vec::new(),
    }
}

/// Adds to `out` the format of each parameter that the `string` parameter `name` of `function` is passed to. `seen`
/// holds the parameters already walked, so a recursive hand-off ends.
fn reached(project: &Project, function: &str, name: &str, seen: &mut BTreeSet<(String, String)>, out: &mut Vec<Format>) {
    if !seen.insert((function.to_string(), name.to_string())) {
        return;
    }
    let Some(sf) = project.get(function) else { return };
    let bindings = project.bindings(function);
    for call in passed_calls(&sf.function) {
        let Some(binding) = bindings.get(call.callee.key()) else { continue };
        for arg in &call.args {
            let (Some(param), Value::Name(passed)) = (&arg.name, &arg.value) else { continue };
            if passed.value != name {
                continue;
            }
            match binding {
                Binding::Template { function: template, .. } => {
                    if let Some(InputType::Format(format)) = template.input(&param.value).map(|i| &i.ty) {
                        out.push(*format);
                    }
                }
                Binding::Function(path) => {
                    let target = project.qualify(function, path);
                    let Some(input) = project.get(&target).and_then(|t| t.function.input(&param.value)) else { continue };
                    match input.ty {
                        InputType::Format(format) => out.push(format),
                        InputType::Text => reached(project, &target, &param.value, seen, out),
                        InputType::Bool | InputType::Choice(_) | InputType::Struct(_) => {}
                    }
                }
                Binding::Request => {}
            }
        }
    }
}

/// Every call a function makes with arguments: a hand-off or template call that is returned, awaited, or the
/// argument of a request, and each call nested in those arguments.
fn passed_calls(function: &Function) -> Vec<&Call> {
    let mut roots: Vec<&Call> = Vec::new();
    walk_stmts(&function.body, &mut |stmt| match stmt {
        Stmt::Return(ret) => {
            if let ReturnValue::Call(call) = &ret.value {
                roots.push(call);
            }
        }
        Stmt::Request { awaited: Awaited::Call(ReturnValue::Call(call)), .. } => roots.push(call),
        Stmt::Request { awaited: Awaited::Request { arg: Value::Call(call), .. }, .. } => roots.push(call),
        Stmt::Request { .. } | Stmt::If { .. } | Stmt::Switch { .. } => {}
    });
    roots.into_iter().flat_map(Call::calls).collect()
}
