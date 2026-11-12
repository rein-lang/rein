//! Hand-written lexer: source text to tokens with byte spans.

use crate::ast::{Piece, Spanned};
use crate::diagnostic::{Diagnostic, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    /// A string in single or double quotes, with escapes applied.
    Str(String),
    /// `/** ... */`, stored as the raw text between the markers.
    Doc(String),
    /// A template literal in backticks.
    Template(Vec<Piece>),
    /// A reference such as `@./ci/fix`, with the path as written.
    Ref(String),
    /// A word Rein reserves so it can give a helpful error, such as `for` or `const`.
    Reserved(String),
    Import,
    From,
    Export,
    Default,
    Function,
    If,
    Else,
    Switch,
    Case,
    Return,
    Boolean,
    LBrace,
    RBrace,
    LParen,
    RParen,
    Comma,
    Colon,
    Semi,
    Pipe,
    Bang,
    /// A single `=`. Rein has no assignment, but the lexer reads it so `const x = 1` gets its own message.
    Assign,
    AndAnd,
    OrOr,
    EqEqEq,
    NotEqEq,
    Eof,
}

/// Words with a grammar role. A function or parameter cannot use them as a name.
pub const KEYWORDS: &[&str] = &[
    "import", "from", "export", "default", "function", "if", "else", "switch", "case", "return", "boolean",
];

/// Words from other languages. Rein rejects them with a specific message.
pub const RESERVED: &[&str] = &["for", "while", "do", "const", "let", "var"];

impl Tok {
    /// Short text for error messages, such as "`{`" or "name `x`".
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("name `{s}`"),
            Tok::Str(s) => format!("string \"{s}\""),
            Tok::Reserved(s) => format!("reserved word `{s}`"),
            Tok::Eof => "end of file".to_string(),
            Tok::Doc(_) => "doc comment".to_string(),
            Tok::Template(_) => "template literal".to_string(),
            Tok::Ref(p) => format!("reference `@{p}`"),
            other => format!("`{}`", other.text()),
        }
    }

    fn text(&self) -> &'static str {
        match self {
            Tok::Import => "import",
            Tok::From => "from",
            Tok::Export => "export",
            Tok::Default => "default",
            Tok::Function => "function",
            Tok::If => "if",
            Tok::Else => "else",
            Tok::Switch => "switch",
            Tok::Case => "case",
            Tok::Return => "return",
            Tok::Boolean => "boolean",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::Comma => ",",
            Tok::Colon => ":",
            Tok::Semi => ";",
            Tok::Pipe => "|",
            Tok::Bang => "!",
            Tok::Assign => "=",
            Tok::AndAnd => "&&",
            Tok::OrOr => "||",
            Tok::EqEqEq => "===",
            Tok::NotEqEq => "!==",
            Tok::Ident(_) | Tok::Str(_) | Tok::Reserved(_) | Tok::Doc(_) | Tok::Template(_) | Tok::Ref(_) | Tok::Eof => "",
        }
    }
}

pub fn lex(src: &str) -> Result<Vec<(Tok, Span)>, Diagnostic> {
    let mut out = Vec::new();
    // A UTF-8 byte order mark at offset 0 counts as whitespace.
    let mut pos = if src.starts_with('\u{feff}') { '\u{feff}'.len_utf8() } else { 0 };
    while let Some(c) = src[pos..].chars().next() {
        if c.is_whitespace() {
            pos += c.len_utf8();
        } else if src[pos..].starts_with("//") {
            pos = src[pos..].find('\n').map_or(src.len(), |i| pos + i);
        } else {
            let (tok, end) = if c == '"' || c == '\'' {
                lex_string(src, pos)?
            } else if is_ident_start(c) {
                lex_word(src, pos)?
            } else if src[pos..].starts_with("/**") {
                lex_doc(src, pos)?
            } else if c == '`' {
                lex_template(src, pos)?
            } else if c == '@' {
                lex_ref(src, pos)?
            } else {
                lex_punct(src, pos)?
            };
            out.push((tok, Span::new(pos, end)));
            pos = end;
        }
    }
    out.push((Tok::Eof, Span::new(src.len(), src.len())));
    Ok(out)
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn keyword(word: &str) -> Option<Tok> {
    Some(match word {
        "import" => Tok::Import,
        "from" => Tok::From,
        "export" => Tok::Export,
        "default" => Tok::Default,
        "function" => Tok::Function,
        "if" => Tok::If,
        "else" => Tok::Else,
        "switch" => Tok::Switch,
        "case" => Tok::Case,
        "return" => Tok::Return,
        "boolean" => Tok::Boolean,
        w if RESERVED.contains(&w) => Tok::Reserved(w.to_string()),
        _ => return None,
    })
}

/// Returns the token and its end offset. `pos` points at an identifier start.
fn lex_word(src: &str, pos: usize) -> Result<(Tok, usize), Diagnostic> {
    let rest = &src[pos..];
    let len = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
    let word = &rest[..len];
    let tok = keyword(word).unwrap_or_else(|| Tok::Ident(word.to_string()));
    Ok((tok, pos + len))
}

/// `/** ... */`. Returns the raw text between the markers. `/**/` is an empty doc block.
fn lex_doc(src: &str, pos: usize) -> Result<(Tok, usize), Diagnostic> {
    let body = pos + 3;
    if src[pos + 2..].starts_with("*/") {
        return Ok((Tok::Doc(String::new()), pos + 4));
    }
    match src[body..].find("*/") {
        Some(i) => Ok((Tok::Doc(src[body..body + i].to_string()), body + i + 2)),
        None => Err(Diagnostic::error(Span::new(pos, src.len()), "unterminated doc comment; add `*/`")),
    }
}

/// Single- or double-quoted string with `\"`, `\'`, `\\` and `\n` escapes. Returns the end offset.
fn lex_string(src: &str, pos: usize) -> Result<(Tok, usize), Diagnostic> {
    let quote = src[pos..].chars().next().unwrap_or('"');
    let mut value = String::new();
    let mut i = pos + 1;
    loop {
        let Some(c) = src[i..].chars().next() else {
            return Err(Diagnostic::error(Span::new(pos, src.len()), format!("unclosed string; add `{quote}`")));
        };
        match c {
            c if c == quote => return Ok((Tok::Str(value), i + 1)),
            '\n' => {
                return Err(Diagnostic::error(
                    Span::new(pos, i),
                    "newline in string; write `\\n` instead",
                ))
            }
            '\\' => {
                let ch = match src[i + 1..].chars().next() {
                    Some(q @ ('"' | '\'' | '\\')) => q,
                    Some('n') => '\n',
                    _ => {
                        return Err(Diagnostic::error(
                            Span::new(i, i + 1),
                            "unknown escape; use `\\\"`, `\\'`, `\\\\` or `\\n`",
                        ))
                    }
                };
                value.push(ch);
                i += 2;
            }
            _ => {
                value.push(c);
                i += c.len_utf8();
            }
        }
    }
}

/// `@./path` or `@../path`: a reference to another function of the project, relative to this file.
/// Any other `@` starts a reference to another program (`lex_program`).
/// The path has no extension. Returns the path as written.
fn lex_ref(src: &str, pos: usize) -> Result<(Tok, usize), Diagnostic> {
    let rest = &src[pos + 1..];
    if !(rest.starts_with("./") || rest.starts_with("../")) {
        return lex_program(src, pos);
    }
    let len = rest.find(|c: char| !(is_ident_char(c) || c == '/' || c == '.')).unwrap_or(rest.len());
    let path = &rest[..len];
    let span = Span::new(pos, pos + 1 + len);
    let segments: Vec<&str> = path.split('/').collect();
    let dotted = segments.iter().any(|s| s.contains('.') && *s != "." && *s != "..");
    if dotted {
        return Err(Diagnostic::error(span, "references have no extension; write `@./name`"));
    }
    let names = &segments[1..];
    let ok = !names.is_empty()
        && names.iter().all(|s| *s == ".." || (!s.is_empty() && s.chars().all(is_ident_char)))
        && !matches!(names.last(), Some(&"..") | None);
    if !ok {
        return Err(Diagnostic::error(span, "a reference is `@./path` or `@../path`, with path names made of letters, digits and `_`"));
    }
    Ok((Tok::Ref(path.to_string()), pos + 1 + len))
}

/// `@program` or `@program/function`: a reference to another installed program, by the name `rein list` shows.
/// The program name may hold `-`. A function path holds names of letters, digits and `_`. Returns the text as written.
fn lex_program(src: &str, pos: usize) -> Result<(Tok, usize), Diagnostic> {
    let rest = &src[pos + 1..];
    let len = rest.find(|c: char| !(is_ident_char(c) || c == '-' || c == '/')).unwrap_or(rest.len());
    let path = &rest[..len];
    let span = Span::new(pos, pos + 1 + len);
    let mut segments = path.split('/');
    let program = segments.next().unwrap_or_default();
    let ok = !program.is_empty()
        && !program.starts_with('-')
        && segments.all(|s| !s.is_empty() && s.chars().all(is_ident_char));
    if !ok {
        let message = "a reference is `@./path`, `@../path`, or `@program/function` for another program";
        return Err(Diagnostic::error(span, message));
    }
    Ok((Tok::Ref(path.to_string()), pos + 1 + len))
}

/// A template literal in backticks. Parts are text and `${name}`. Returns the end offset.
fn lex_template(src: &str, pos: usize) -> Result<(Tok, usize), Diagnostic> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut i = pos + 1;
    loop {
        let Some(c) = src[i..].chars().next() else {
            return Err(Diagnostic::error(Span::new(pos, src.len()), "unclosed template literal; add `` ` ``"));
        };
        match c {
            '`' => {
                if !text.is_empty() {
                    pieces.push(Piece::Text(text));
                }
                return Ok((Tok::Template(pieces), i + 1));
            }
            '$' if src[i + 1..].starts_with('{') => {
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                let name_start = i + 2;
                let rest = &src[name_start..];
                let len = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
                let name_end = name_start + len;
                let valid = rest[..len].chars().next().is_some_and(is_ident_start) && src[name_end..].starts_with('}');
                if !valid {
                    let end = src[name_end..].find('}').map_or(name_end, |k| name_end + k + 1);
                    return Err(Diagnostic::error(
                        Span::new(i, end),
                        "only a parameter name can go in `${}`; write `${name}`",
                    ));
                }
                let value = rest[..len].to_string();
                pieces.push(Piece::Name(Spanned { value, span: Span::new(name_start, name_end) }));
                i = name_end + 1;
            }
            '\\' => {
                let escaped = match src[i + 1..].chars().next() {
                    Some(q @ ('`' | '\\' | '$')) => q,
                    Some('n') => '\n',
                    _ => {
                        return Err(Diagnostic::error(
                            Span::new(i, i + 2),
                            "unknown escape; use `\\``, `\\\\`, `\\$` or `\\n`",
                        ))
                    }
                };
                text.push(escaped);
                i += 2;
            }
            _ => {
                text.push(c);
                i += c.len_utf8();
            }
        }
    }
}

fn lex_punct(src: &str, pos: usize) -> Result<(Tok, usize), Diagnostic> {
    let rest = &src[pos..];
    let c = rest.chars().next().unwrap_or('\0');
    let (tok, len) = match c {
        '{' => (Tok::LBrace, 1),
        '}' => (Tok::RBrace, 1),
        '(' => (Tok::LParen, 1),
        ')' => (Tok::RParen, 1),
        ',' => (Tok::Comma, 1),
        ':' => (Tok::Colon, 1),
        ';' => (Tok::Semi, 1),
        '|' if rest.starts_with("||") => (Tok::OrOr, 2),
        '|' => (Tok::Pipe, 1),
        '&' if rest.starts_with("&&") => (Tok::AndAnd, 2),
        '=' if rest.starts_with("===") => (Tok::EqEqEq, 3),
        '=' if rest.starts_with("==") => {
            return Err(Diagnostic::error(Span::new(pos, pos + 2), "use `===` instead of `==`"))
        }
        '=' => (Tok::Assign, 1),
        '!' if rest.starts_with("!==") => (Tok::NotEqEq, 3),
        '!' if rest.starts_with("!=") => {
            return Err(Diagnostic::error(Span::new(pos, pos + 2), "use `!==` instead of `!=`"))
        }
        '!' => (Tok::Bang, 1),
        '/' if rest.starts_with("/*") => {
            return Err(Diagnostic::error(
                Span::new(pos, pos + 2),
                "use `//` for comments or `/** */` for the doc block",
            ))
        }
        _ => {
            return Err(Diagnostic::error(
                Span::new(pos, pos + c.len_utf8()),
                format!("unexpected character `{c}`"),
            ))
        }
    };
    Ok((tok, pos + len))
}

#[cfg(test)]
#[path = "lexer_tests.rs"]
mod tests;
