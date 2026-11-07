//! Hand-written lexer: source text to tokens with byte spans.

use crate::diagnostic::{Diagnostic, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Str(String),
    /// `@name`, stored without the `@`.
    Ref(String),
    /// `/** ... */`, stored as the raw text between the markers.
    Doc(String),
    Input,
    If,
    Else,
    Switch,
    Case,
    Default,
    Return,
    Bool,
    Choice,
    Not,
    And,
    Or,
    LBrace,
    RBrace,
    LParen,
    RParen,
    Comma,
    Colon,
    EqEq,
    NotEq,
    Eof,
}

pub const KEYWORDS: &[&str] = &[
    "input", "if", "else", "switch", "case", "default", "return", "bool", "choice", "not", "and", "or",
];

impl Tok {
    /// Short text for error messages, such as "`{`" or "name `x`".
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("name `{s}`"),
            Tok::Str(s) => format!("string \"{s}\""),
            Tok::Ref(s) => format!("reference `@{s}`"),
            Tok::Eof => "end of file".to_string(),
            Tok::Doc(_) => "doc comment".to_string(),
            other => format!("`{}`", other.text()),
        }
    }

    fn text(&self) -> &'static str {
        match self {
            Tok::Input => "input",
            Tok::If => "if",
            Tok::Else => "else",
            Tok::Switch => "switch",
            Tok::Case => "case",
            Tok::Default => "default",
            Tok::Return => "return",
            Tok::Bool => "bool",
            Tok::Choice => "choice",
            Tok::Not => "not",
            Tok::And => "and",
            Tok::Or => "or",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::Comma => ",",
            Tok::Colon => ":",
            Tok::EqEq => "==",
            Tok::NotEq => "!=",
            Tok::Ident(_) | Tok::Str(_) | Tok::Ref(_) | Tok::Doc(_) | Tok::Eof => "",
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
            let (tok, end) = if c == '"' {
                lex_string(src, pos)?
            } else if c == '@' {
                lex_ref(src, pos)?
            } else if is_ident_start(c) {
                lex_word(src, pos)?
            } else if src[pos..].starts_with("/**") {
                lex_doc(src, pos)?
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
        "input" => Tok::Input,
        "if" => Tok::If,
        "else" => Tok::Else,
        "switch" => Tok::Switch,
        "case" => Tok::Case,
        "default" => Tok::Default,
        "return" => Tok::Return,
        "bool" => Tok::Bool,
        "choice" => Tok::Choice,
        "not" => Tok::Not,
        "and" => Tok::And,
        "or" => Tok::Or,
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

/// `@seg(/seg)*`. A `/` is part of the name only when a name starts after it.
fn lex_ref(src: &str, pos: usize) -> Result<(Tok, usize), Diagnostic> {
    let mut end = pos + 1;
    let mut name = String::new();
    loop {
        let seg = &src[end..];
        match seg.chars().next() {
            Some(c) if is_ident_start(c) => {}
            _ => {
                return Err(Diagnostic::error(
                    Span::new(pos, end),
                    "expected a function name after `@`",
                ))
            }
        }
        let len = seg.find(|c: char| !is_ident_char(c)).unwrap_or(seg.len());
        name.push_str(&seg[..len]);
        end += len;
        let after = &src[end..];
        if after.starts_with('/') && after[1..].chars().next().is_some_and(is_ident_start) {
            name.push('/');
            end += 1;
        } else {
            return Ok((Tok::Ref(name), end));
        }
    }
}

/// Double-quoted string with `\"`, `\\` and `\n` escapes. Returns the end offset.
fn lex_string(src: &str, pos: usize) -> Result<(Tok, usize), Diagnostic> {
    let mut value = String::new();
    let mut i = pos + 1;
    loop {
        let Some(c) = src[i..].chars().next() else {
            return Err(Diagnostic::error(Span::new(pos, src.len()), "unclosed string; add `\"`"));
        };
        match c {
            '"' => return Ok((Tok::Str(value), i + 1)),
            '\n' => {
                return Err(Diagnostic::error(
                    Span::new(pos, i),
                    "newline in string; write `\\n` instead",
                ))
            }
            '\\' => {
                let ch = match src[i + 1..].chars().next() {
                    Some('"') => '"',
                    Some('\\') => '\\',
                    Some('n') => '\n',
                    _ => {
                        return Err(Diagnostic::error(
                            Span::new(i, i + 1),
                            "unknown escape; use `\\\"`, `\\\\` or `\\n`",
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
        '/' if rest.starts_with("/*") => {
            return Err(Diagnostic::error(
                Span::new(pos, pos + 2),
                "use `//` for comments or `/** */` for the doc block",
            ))
        }
        '=' if rest.starts_with("==") => (Tok::EqEq, 2),
        '!' if rest.starts_with("!=") => (Tok::NotEq, 2),
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
mod tests {
    use super::*;

    #[test]
    fn keywords_lex_as_keywords() {
        for kw in KEYWORDS {
            let toks = lex(kw).unwrap();
            assert!(!matches!(toks[0].0, Tok::Ident(_)), "`{kw}` lexed as a name");
        }
    }

    #[test]
    fn description_is_a_name_now() {
        let toks = lex("description").unwrap();
        assert_eq!(toks[0].0, Tok::Ident("description".into()));
    }

    #[test]
    fn escapes_refs_and_comments() {
        let toks = lex(r#"// note
"a\"b\\c\nü" @ci/fix/x merge_pr== !="#).unwrap();
        let kinds: Vec<Tok> = toks.into_iter().map(|(t, _)| t).collect();
        assert_eq!(
            kinds,
            vec![
                Tok::Str("a\"b\\c\nü".into()),
                Tok::Ref("ci/fix/x".into()),
                Tok::Ident("merge_pr".into()),
                Tok::EqEq,
                Tok::NotEq,
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn doc_block_keeps_raw_inner_text() {
        let toks = lex("/**\n * Hi.\n */ input").unwrap();
        assert_eq!(toks[0].0, Tok::Doc("\n * Hi.\n ".into()));
        assert_eq!(toks[0].1, Span::new(0, 14));
        assert_eq!(toks[1].0, Tok::Input);
    }

    #[test]
    fn empty_doc_block() {
        let toks = lex("/**/").unwrap();
        assert_eq!(toks[0].0, Tok::Doc(String::new()));
        assert_eq!(toks[0].1, Span::new(0, 4));
        let toks = lex("/***/").unwrap();
        assert_eq!(toks[0].0, Tok::Doc(String::new()));
    }

    #[test]
    fn leading_bom_is_whitespace_and_spans_stay_byte_offsets() {
        let toks = lex("\u{feff}/** d */").unwrap();
        assert_eq!(toks[0].0, Tok::Doc(" d ".into()));
        assert_eq!(toks[0].1, Span::new(3, 11));
    }

    #[test]
    fn unclosed_string_is_an_error() {
        let err = lex("\"abc").unwrap_err();
        assert!(err.message.contains("unclosed string"), "{}", err.message);
    }

    #[test]
    fn unterminated_doc_is_an_error() {
        let err = lex("/** abc").unwrap_err();
        assert!(err.message.contains("unterminated doc comment"), "{}", err.message);
    }

    #[test]
    fn plain_block_comment_is_rejected() {
        let err = lex("/* x */").unwrap_err();
        assert_eq!(err.message, "use `//` for comments or `/** */` for the doc block");
        assert_eq!(err.span, Span::new(0, 2));
    }
}
