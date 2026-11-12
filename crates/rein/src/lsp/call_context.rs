//! The call around the cursor, read from the text, so it works while the call is still being typed.

use rein_core::ast::{Binding, Input};
use rein_core::project::Project;

use super::text::{is_identifier, is_path_char, is_token_char};

/// A call whose `(` is still open at the cursor.
#[derive(Debug, PartialEq)]
pub struct CallContext {
    /// The callee as written: a reference path such as `./x`, or a std name.
    pub callee: String,
    /// The argument the cursor is in, from 0.
    pub index: usize,
    /// The names of the arguments before the current one.
    pub given: Vec<String>,
    /// The name of the current argument, once it has `name=`.
    pub current: Option<String>,
    /// The cursor is where an argument name goes: at the start of an argument, or in its name.
    pub at_name: bool,
}

/// The innermost call open at byte `off`. Strings and comments are skipped. Each delimiter is ASCII, so the
/// scan reads bytes.
pub fn call_at(text: &str, off: usize) -> Option<CallContext> {
    let b = &text.as_bytes()[..off];
    // Each open `(`, with the offsets of the commas at its depth.
    let mut open: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut in_string = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            q @ (b'"' | b'\'' | b'`') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                in_string = i >= b.len();
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            b'(' => open.push((i, Vec::new())),
            b')' => {
                open.pop();
            }
            b',' => {
                if let Some((_, commas)) = open.last_mut() {
                    commas.push(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    let (paren, commas) = open.pop()?;
    let callee_start = text[..paren].rfind(|c: char| !(is_path_char(c) || c == '@')).map_or(0, |j| j + 1);
    let callee = text[callee_start..paren].trim_start_matches('@');
    if callee.is_empty() {
        return None;
    }
    let mut starts = vec![paren + 1];
    starts.extend(commas.iter().map(|c| c + 1));
    let name_of = |arg: &str| arg.split_once('=').map(|(n, _)| n.trim()).filter(|n| is_identifier(n)).map(str::to_string);
    let given = starts.windows(2).filter_map(|w| name_of(&text[w[0]..w[1] - 1])).collect();
    let current = &text[*starts.last()?..off];
    Some(CallContext {
        callee: callee.to_string(),
        index: commas.len(),
        given,
        current: name_of(current),
        at_name: !in_string && current.trim_start().chars().all(is_token_char),
    })
}

/// The parameters a call to `callee`, written in function `from`, binds by name: all of a template's, or a
/// function's `string` parameters.
pub fn callee_params<'a>(project: &'a Project, from: &str, callee: &str) -> Option<Vec<&'a Input>> {
    match project.binding_for(from, callee).ok()? {
        Binding::Template { function, .. } => Some(function.inputs.iter().collect()),
        Binding::Function(path) => Some(project.get(&path)?.function.text_inputs().collect()),
        Binding::Request => None,
    }
}

#[cfg(test)]
#[path = "call_context_tests.rs"]
mod tests;
