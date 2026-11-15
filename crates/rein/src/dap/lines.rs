//! Source lines of the walk's nodes. The step engine reports a node by function and name, not by line, so the
//! line is read from the source here: where a question is first tested, where a request is made, where a return
//! gives its text, and where a function hands off. Each falls back to the function's first line when no
//! statement matches.

use rein_core::ast::{walk_stmts, walk_vars, Awaited, Binding, Expr, Piece, ReturnValue, Stmt};
use rein_core::diagnostic::line_col;
use rein_core::project::{Project, SourceFile};

/// The 1-based line of a byte offset in `sf`.
fn line_of(sf: &SourceFile, offset: usize) -> usize {
    line_col(&sf.text, offset).0 + 1
}

/// The line of the function's header.
fn header(sf: &SourceFile) -> usize {
    line_of(sf, sf.function.span.start)
}

/// The byte offset of the first statement, in source order, that `pick` accepts.
fn first(sf: &SourceFile, pick: impl Fn(&Stmt) -> Option<usize>) -> Option<usize> {
    let mut found = None;
    walk_stmts(&sf.function.body, &mut |s| {
        if found.is_none() {
            found = pick(s);
        }
    });
    found
}

/// The line where parameter `name` is first tested: an `if` on it or a `switch` over it. Else the parameter's line.
pub fn question(sf: &SourceFile, name: &str) -> usize {
    let tested = first(sf, |s| match s {
        Stmt::If { cond, span, .. } if mentions(cond, name) => Some(span.start),
        Stmt::Switch { subject, span, .. } if subject.value == name => Some(span.start),
        _ => None,
    });
    match (tested, sf.function.input(name)) {
        (Some(offset), _) => line_of(sf, offset),
        (None, Some(input)) => line_of(sf, input.name.span.start),
        (None, None) => header(sf),
    }
}

/// The line of the `await request(...)` that asks for answer `name`.
pub fn request(sf: &SourceFile, name: &str) -> usize {
    let asked = first(sf, |s| match s {
        Stmt::Request { response, span, .. } if response.name.value == name => Some(span.start),
        _ => None,
    });
    asked.map_or_else(|| header(sf), |offset| line_of(sf, offset))
}

/// The line of the return that gives `instruction`. A return gives the text when its literal parts all appear in
/// it, so a template with `${...}` parts still matches. When none does, the first return that gives any text
/// counts. A hand-off gives no text.
// ponytail: the walk does not say which return fired. Two returns with the same literal parts resolve to the first.
pub fn done(project: &Project, caller: &str, instruction: &str) -> usize {
    let Some(sf) = project.get(caller) else {
        return 1;
    };
    let bindings = project.bindings(caller);
    let mut matched = None;
    let mut any = None;
    walk_stmts(&sf.function.body, &mut |s| {
        let Stmt::Return(r) = s else { return };
        let offset = r.span.start;
        match &r.value {
            ReturnValue::Text(t) if t.value == instruction => {
                matched.get_or_insert(offset);
            }
            ReturnValue::Template(lit) => {
                any.get_or_insert(offset);
                if lit
                    .pieces
                    .iter()
                    .all(|p| matches!(p, Piece::Name(_)) || literal_in(p, instruction))
                {
                    matched.get_or_insert(offset);
                }
            }
            ReturnValue::Call(call)
                if !matches!(bindings.get(call.callee.key()), Some(Binding::Function(_))) =>
            {
                any.get_or_insert(offset);
            }
            _ => {}
        }
    });
    matched
        .or(any)
        .map_or_else(|| header(sf), |offset| line_of(sf, offset))
}

/// Whether a literal part of a template appears in `text`. Blank parts always do.
fn literal_in(piece: &Piece, text: &str) -> bool {
    match piece {
        Piece::Text(t) => t.trim().is_empty() || text.contains(t.trim()),
        Piece::Name(_) => true,
    }
}

/// The line in `caller` of the return that hands off to `callee`, a project path.
pub fn handoff(project: &Project, caller: &str, callee: &str) -> usize {
    let Some(sf) = project.get(caller) else {
        return 1;
    };
    let bindings = project.bindings(caller);
    let passed = first(sf, |s| {
        let (value, at) = match s {
            Stmt::Return(r) => (&r.value, r.span.start),
            Stmt::Request { awaited: Awaited::Call(value), span, .. } => (value, span.start),
            _ => return None,
        };
        let key = match value {
            ReturnValue::Ref(target) => target.value.as_str(),
            ReturnValue::Call(call) => call.callee.key(),
            _ => return None,
        };
        match bindings.get(key) {
            Some(Binding::Function(path)) if path == callee => Some(at),
            _ => None,
        }
    });
    passed.map_or_else(|| header(sf), |offset| line_of(sf, offset))
}

/// Whether a condition tests the parameter `name`.
fn mentions(cond: &Expr, name: &str) -> bool {
    let mut hit = false;
    walk_vars(cond, &mut |v| hit |= v.value == name);
    hit
}

#[cfg(test)]
#[path = "lines_tests.rs"]
mod tests;
