//! Where each name of a project is written: references to functions, and uses of parameters and answers.
//! References, rename, call hierarchy and code lens all read these sites. They come from the parsed tree.

use rein_core::ast::{walk_stmts, walk_vars, Awaited, Binding, Call, Callee, Function, Input, Piece, ReturnValue, Spanned, Stmt, TemplateLit, Value};
use rein_core::diagnostic::Span;
use rein_core::lexer::{lex, Tok};
use rein_core::project::Project;

use super::structs::struct_of;

/// What a name under the cursor stands for.
#[derive(Debug, Clone, PartialEq)]
pub enum Symbol {
    /// A function of the project, by project path.
    Function(String),
    /// The parameter or `const` answer `name` of function `function`.
    Param { function: String, name: String },
    /// An `interface`: the file that declares it, and its name. A `.struct.rein` file declares a shared one.
    Struct { file: String, name: String },
    /// A field of a struct. Each function that takes the struct reads the field by its name.
    Field { file: String, strukt: String, name: String },
    /// A std item, such as `delegate`.
    Std(String),
}

/// Every call in `f`, nested ones included.
pub fn calls(f: &Function) -> Vec<&Call> {
    let mut out = Vec::new();
    walk_stmts(&f.body, &mut |s| match s {
        Stmt::Return(Spanned { value: ReturnValue::Call(call), .. }) => out.extend(call.calls()),
        Stmt::Request { awaited: Awaited::Request { arg: Value::Call(call), .. } | Awaited::Call(ReturnValue::Call(call)), .. } => {
            out.extend(call.calls())
        }
        _ => {}
    });
    out
}

/// Every reference token in `f` (`@./path` or `@program/path`), in file order, with the call it starts when it is a call.
pub fn refs(f: &Function) -> Vec<(&Spanned<String>, Option<&Call>)> {
    let mut out = Vec::new();
    walk_stmts(&f.body, &mut |s| {
        if let Stmt::Return(Spanned { value: ReturnValue::Ref(r), .. })
        | Stmt::Request { awaited: Awaited::Call(ReturnValue::Ref(r)), .. } = s
        {
            out.push((r, None));
        }
    });
    for call in calls(f) {
        if let Callee::Ref(r) = &call.callee {
            out.push((r, Some(call)));
        }
    }
    out.sort_by_key(|(r, _)| r.span.start);
    out
}

/// The function a reference written in `from` names: a project path for `@./x`, or `@program/path` for a function
/// of another installed program (`@program` alone names its entry). The checker resolves it the same way.
pub fn target(project: &Project, from: &str, key: &str) -> Option<String> {
    match project.binding_for(from, key).ok()? {
        Binding::Function(path) | Binding::Template { path: Some(path), .. } => Some(path),
        Binding::Template { path: None, .. } | Binding::Request => None,
    }
}

/// The parameters of `f`, then its `const` answers, in file order.
pub fn declarations(f: &Function) -> Vec<&Input> {
    let mut out: Vec<&Input> = f.inputs.iter().collect();
    walk_stmts(&f.body, &mut |s| {
        if let Stmt::Request { response, .. } = s {
            out.push(response);
        }
    });
    out
}

/// The parameter or answer `n` of `f`.
pub fn declaration<'a>(f: &'a Function, n: &str) -> Option<&'a Input> {
    declarations(f).into_iter().find(|i| i.name.value == n)
}

/// Every use of a parameter or answer name in the body of `f`: conditions, switch subjects, `${name}`, and
/// bare names passed as arguments. In file order.
pub fn name_uses(f: &Function) -> Vec<&Spanned<String>> {
    let mut out = Vec::new();
    walk_stmts(&f.body, &mut |s| match s {
        Stmt::If { cond, .. } => walk_vars(cond, &mut |v| out.push(v)),
        Stmt::Switch { subject, .. } => out.push(subject),
        Stmt::Request { awaited: Awaited::Request { arg, .. }, .. } => value_names(arg, &mut out),
        Stmt::Request { awaited: Awaited::Call(ReturnValue::Call(call)), .. } => {
            call.args.iter().for_each(|a| value_names(&a.value, &mut out))
        }
        Stmt::Request { .. } => {}
        Stmt::Return(r) => match &r.value {
            ReturnValue::Template(lit) => template_names(lit, &mut out),
            ReturnValue::Call(call) => call.args.iter().for_each(|a| value_names(&a.value, &mut out)),
            ReturnValue::Text(_) | ReturnValue::Ref(_) => {}
        },
    });
    out.sort_by_key(|s| s.span.start);
    out
}

fn value_names<'a>(v: &'a Value, out: &mut Vec<&'a Spanned<String>>) {
    match v {
        // `true` and `false` are boolean literals, not names.
        Value::Name(n) if matches!(n.value.as_str(), "true" | "false") => {}
        Value::Name(n) => out.push(n),
        Value::Template(lit) => template_names(lit, out),
        Value::Call(call) => call.args.iter().for_each(|a| value_names(&a.value, out)),
        Value::Str(_) => {}
    }
}

fn template_names<'a>(lit: &'a TemplateLit, out: &mut Vec<&'a Spanned<String>>) {
    out.extend(lit.pieces.iter().filter_map(|p| match p {
        Piece::Name(n) => Some(n),
        Piece::Text(_) => None,
    }));
}

/// The span of the name after `export default function`. None when the file does not lex.
pub fn name_span(text: &str) -> Option<Span> {
    let toks = lex(text).ok()?;
    let at = toks.iter().position(|(t, _)| *t == Tok::Function)?;
    match toks.get(at + 1)? {
        (Tok::Ident(_), span) => Some(*span),
        _ => None,
    }
}

/// The span of function `name`'s own name. The file start when it has none.
pub fn header_span(project: &Project, name: &str) -> Span {
    project.get(name).and_then(|sf| name_span(&sf.text)).unwrap_or_default()
}

/// The symbol at byte `off` in function `name`, and the span of the name it is written as.
pub fn symbol_at(project: &Project, name: &str, off: usize) -> Option<(Symbol, Span)> {
    let sf = project.get(name)?;
    let f = &sf.function;
    let local = |n: &Spanned<String>| (param_symbol(project, name, &n.value), n.span);
    if let Some((r, _)) = refs(f).into_iter().find(|(r, _)| r.span.contains(off)) {
        return Some((Symbol::Function(target(project, name, &r.value)?), r.span));
    }
    for call in calls(f) {
        if let Callee::Std(s) = &call.callee {
            if s.span.contains(off) {
                return Some((Symbol::Std(s.value.clone()), s.span));
            }
        }
        if let Some(arg) = call.args.iter().filter_map(|a| a.name.as_ref()).find(|a| a.span.contains(off)) {
            // A std template's parameters have no file to point to.
            let Callee::Ref(r) = &call.callee else { return None };
            let callee = target(project, name, &r.value)?;
            return Some((param_symbol(project, &callee, &arg.value), arg.span));
        }
    }
    let mut awaited = None;
    walk_stmts(&f.body, &mut |s| {
        if let Stmt::Request { awaited: Awaited::Request { callee, .. }, .. } = s {
            if callee.span.contains(off) {
                awaited = Some(callee.clone());
            }
        }
    });
    if let Some(c) = awaited {
        return Some((Symbol::Std(c.value), c.span));
    }
    if let Some(p) = f.struct_param().filter(|p| p.ty.name().span.contains(off)) {
        let (file, s) = struct_of(project, name)?;
        return Some((Symbol::Struct { file, name: s }, p.ty.name().span));
    }
    for s in &f.structs {
        if s.name.span.contains(off) {
            return Some((Symbol::Struct { file: name.to_string(), name: s.name.value.clone() }, s.name.span));
        }
        if let Some(field) = s.fields.iter().find(|i| i.name.span.contains(off)) {
            let symbol = Symbol::Field { file: name.to_string(), strukt: s.name.value.clone(), name: field.name.value.clone() };
            return Some((symbol, field.name.span));
        }
    }
    if let Some(input) = declarations(f).into_iter().find(|i| i.name.span.contains(off)) {
        return Some(local(&input.name));
    }
    if let Some(n) = name_uses(f).into_iter().find(|n| n.span.contains(off)) {
        return Some(local(n));
    }
    let header = name_span(&sf.text)?;
    header.contains(off).then(|| (Symbol::Function(name.to_string()), header))
}

/// The field `n` of the struct that `function` takes, when `n` is in its `{ }`. Else the parameter or answer
/// `n` of `function`.
fn param_symbol(project: &Project, function: &str, n: &str) -> Symbol {
    let destructured = project
        .get(function)
        .and_then(|sf| sf.function.struct_param())
        .is_some_and(|p| p.fields.iter().any(|f| f.value == n));
    match struct_of(project, function).filter(|_| destructured) {
        Some((file, strukt)) => Symbol::Field { file, strukt, name: n.to_string() },
        None => Symbol::Param { function: function.to_string(), name: n.to_string() },
    }
}

/// Every reference to function `target`, `@./` or `@program/`, in this project: the function it is written in, and the span of the token.
/// References from other programs are not searched.
pub fn callers(project: &Project, to: &str) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    for (name, sf) in &project.files {
        for (r, _) in refs(&sf.function) {
            if target(project, name, &r.value).as_deref() == Some(to) {
                out.push((name.clone(), r.span));
            }
        }
    }
    out
}

/// Every use of name `n` of function `function`, without its declaration: its uses in the body, and `n=` in
/// each call to the function.
pub fn param_uses(project: &Project, function: &str, n: &str) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    if let Some(sf) = project.get(function) {
        out.extend(name_uses(&sf.function).into_iter().filter(|u| u.value == n).map(|u| (function.to_string(), u.span)));
    }
    for (caller, sf) in &project.files {
        for (r, call) in refs(&sf.function) {
            let Some(call) = call else { continue };
            if target(project, caller, &r.value).as_deref() != Some(function) {
                continue;
            }
            let named = call.args.iter().filter_map(|a| a.name.as_ref()).filter(|a| a.value == n);
            out.extend(named.map(|a| (caller.clone(), a.span)));
        }
    }
    out
}
