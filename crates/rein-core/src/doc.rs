//! The doc block: the `/** */` comment before `export default function`, and the one before each parameter.
//!
//! Each line loses its leading whitespace and one `*`, then one space. The text before the
//! first `@` line is the description. Tags are recorded by name only; the checker decides
//! what each tag means.

use crate::ast::{DocBlock, Spanned};
use crate::diagnostic::Span;

/// One line of the block. `text` is what is left after the decoration. `start` is its offset in the file.
struct Line<'a> {
    text: &'a str,
    start: usize,
}

/// Parses the text between `/**` and `*/`. `base` is the file offset of `inner`'s first byte.
pub fn parse_doc(inner: &str, base: usize) -> DocBlock {
    let mut doc = DocBlock::default();
    let mut desc_lines = Vec::new();
    let mut in_tags = false;
    for line in lines(inner, base) {
        let t = line.text.trim_start();
        let Some(tag) = t.strip_prefix('@') else {
            if !in_tags {
                desc_lines.push(line);
            }
            continue;
        };
        in_tags = true;
        let at = offset(&line, t);
        let name_len = tag.find(char::is_whitespace).unwrap_or(tag.len());
        doc.tags.push(Spanned {
            value: tag[..name_len].to_string(),
            span: Span::new(at, at + 1 + name_len),
        });
    }
    doc.description = description(&desc_lines, base);
    doc
}

/// The description: the lines of each paragraph joined with one space, paragraphs joined with a blank line.
/// A blank line ends a paragraph; a run of blank lines counts as one.
fn description(lines: &[Line], base: usize) -> Spanned<String> {
    let rows: Vec<Option<Spanned<String>>> = lines.iter().map(|l| spanned(l.text, l.start)).collect();
    let mut paragraphs = Vec::new();
    let mut words: Vec<&str> = Vec::new();
    for row in rows.iter() {
        match row {
            Some(s) => words.push(&s.value),
            None if !words.is_empty() => paragraphs.push(std::mem::take(&mut words).join(" ")),
            None => {}
        }
    }
    if !words.is_empty() {
        paragraphs.push(words.join(" "));
    }
    let value = paragraphs.join("\n\n");
    let span = match (rows.iter().flatten().next(), rows.iter().flatten().last()) {
        (Some(first), Some(last)) => first.span.to(last.span),
        _ => Span::new(base, base),
    };
    Spanned { value, span }
}

/// Splits the block into lines and strips the ` * ` decoration from each one.
fn lines(inner: &str, base: usize) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut line_start = 0;
    for raw in inner.split('\n') {
        let indent = raw.len() - raw.trim_start().len();
        let body = &raw[indent..];
        let (text, lead) = match body.strip_prefix('*') {
            Some(rest) => match rest.strip_prefix(' ') {
                Some(r) => (r, indent + 2),
                None => (rest, indent + 1),
            },
            None => (body, indent),
        };
        out.push(Line { text, start: base + line_start + lead });
        line_start += raw.len() + 1;
    }
    out
}

/// Trimmed text with its span. `text` starts at file offset `start`. None when blank.
fn spanned(text: &str, start: usize) -> Option<Spanned<String>> {
    let value = text.trim();
    if value.is_empty() {
        return None;
    }
    let at = start + (text.len() - text.trim_start().len());
    Some(Spanned { value: value.to_string(), span: Span::new(at, at + value.len()) })
}

/// File offset of `suffix`, which must be a suffix of the line's text.
fn offset(line: &Line, suffix: &str) -> usize {
    line.start + (line.text.len() - suffix.len())
}

#[cfg(test)]
#[path = "doc_tests.rs"]
mod tests;
