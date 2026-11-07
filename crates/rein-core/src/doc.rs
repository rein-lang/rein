//! The doc block: the TSDoc-style `/** */` comment at the top of a function file.
//!
//! Each line loses its leading whitespace and one `*`, then one space. The text before the
//! first `@` line is the description. `@param <name> [-] <text>` gives an input its question,
//! and the text runs on through the lines after it, up to the next `@` line.

use crate::ast::{DocBlock, ParamTag, Spanned};
use crate::diagnostic::Span;

/// One line of the block. `text` is what is left after the decoration. `start` is its offset in the file.
struct Line<'a> {
    text: &'a str,
    start: usize,
}

/// A `@param` tag whose text may still continue on later lines.
struct Pending {
    name: Spanned<String>,
    pieces: Vec<Spanned<String>>,
}

impl Pending {
    fn finish(self) -> ParamTag {
        let value = self.pieces.iter().map(|p| p.value.as_str()).collect::<Vec<_>>().join(" ");
        let span = match (self.pieces.first(), self.pieces.last()) {
            (Some(first), Some(last)) => first.span.to(last.span),
            _ => Span::new(self.name.span.end, self.name.span.end),
        };
        ParamTag { name: self.name, text: Spanned { value, span } }
    }
}

/// Parses the text between `/**` and `*/`. `base` is the file offset of `inner`'s first byte.
pub fn parse_doc(inner: &str, base: usize) -> DocBlock {
    let mut doc = DocBlock::default();
    let mut desc_lines = Vec::new();
    let mut pending: Option<Pending> = None;
    let mut in_tags = false;
    for line in lines(inner, base) {
        let t = line.text.trim_start();
        let Some(tag) = t.strip_prefix('@') else {
            if !in_tags {
                desc_lines.push(line);
            } else if let (Some(p), Some(piece)) = (pending.as_mut(), spanned(line.text, line.start)) {
                p.pieces.push(piece);
            }
            continue;
        };
        in_tags = true;
        if let Some(p) = pending.take() {
            doc.params.push(p.finish());
        }
        let at = offset(&line, t);
        let name_len = tag.find(char::is_whitespace).unwrap_or(tag.len());
        let (tag_name, after_tag) = tag.split_at(name_len);
        if tag_name != "param" {
            doc.unknown_tags.push(Spanned {
                value: tag_name.to_string(),
                span: Span::new(at, at + 1 + name_len),
            });
            continue;
        }
        pending = Some(param_tag(&line, after_tag));
    }
    if let Some(p) = pending {
        doc.params.push(p.finish());
    }
    doc.description = description(&desc_lines, base);
    doc
}

/// Reads the name and the first piece of text after `@param`. `after_tag` is a suffix of the line.
fn param_tag(line: &Line, after_tag: &str) -> Pending {
    let rest = after_tag.trim_start();
    let name_len = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (name, after_name) = rest.split_at(name_len);
    let name_start = offset(line, rest);
    let after_name = after_name.trim_start();
    // The `-` before the text is optional. It counts only as a separate word.
    let text = match after_name.strip_prefix('-') {
        Some(r) if r.is_empty() || r.starts_with(char::is_whitespace) => r.trim_start(),
        _ => after_name,
    };
    Pending {
        name: Spanned {
            value: name.to_string(),
            span: Span::new(name_start, name_start + name_len),
        },
        pieces: spanned(text, offset(line, text)).into_iter().collect(),
    }
}

/// The description: lines joined with `\n`, trimmed, with runs of blank lines cut to one.
fn description(lines: &[Line], base: usize) -> Spanned<String> {
    let rows: Vec<Option<Spanned<String>>> = lines.iter().map(|l| spanned(l.text, l.start)).collect();
    let joined = rows.iter().map(|r| r.as_ref().map_or("", |s| s.value.as_str())).collect::<Vec<_>>().join("\n");
    let mut value = joined.trim().to_string();
    while value.contains("\n\n\n") {
        value = value.replace("\n\n\n", "\n\n");
    }
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
mod tests {
    use super::*;

    fn doc(inner: &str) -> DocBlock {
        parse_doc(inner, 0)
    }

    #[test]
    fn description_keeps_line_breaks_and_collapses_blank_runs() {
        let d = doc("\n * First line.\n * Second line.\n *\n *\n *\n * New paragraph.\n ");
        assert_eq!(d.description.value, "First line.\nSecond line.\n\nNew paragraph.");
        assert!(d.params.is_empty());
    }

    #[test]
    fn single_line_block_is_the_description() {
        assert_eq!(doc(" Page the engineer. ").description.value, "Page the engineer.");
        assert_eq!(doc("").description.value, "");
    }

    #[test]
    fn param_with_and_without_dash() {
        let d = doc("\n * Desc.\n *\n * @param docs_only Does the PR change docs?\n * @param ci - What is CI?\n ");
        assert_eq!(d.params.len(), 2);
        assert_eq!(d.params[0].name.value, "docs_only");
        assert_eq!(d.params[0].text.value, "Does the PR change docs?");
        assert_eq!(d.params[1].name.value, "ci");
        assert_eq!(d.params[1].text.value, "What is CI?");
    }

    #[test]
    fn continuation_lines_join_the_previous_tag() {
        let d = doc("\n * @param ci - What is CI?\n *   Continuation here.\n *\n *   And more.\n * @param other Next.\n ");
        assert_eq!(d.params[0].text.value, "What is CI? Continuation here. And more.");
        assert_eq!(d.params[1].text.value, "Next.");
    }

    #[test]
    fn unknown_tags_stay_out_of_the_description() {
        let d = doc("\n * Desc.\n * @returns Something.\n *   more of returns.\n * @example foo\n ");
        assert_eq!(d.description.value, "Desc.");
        let names: Vec<&str> = d.unknown_tags.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(names, ["returns", "example"]);
    }

    #[test]
    fn param_without_name_or_text_is_kept_empty() {
        let d = doc("\n * @param\n * @param x\n ");
        assert_eq!(d.params[0].name.value, "");
        assert_eq!(d.params[1].name.value, "x");
        assert_eq!(d.params[1].text.value, "");
    }

    #[test]
    fn spans_point_into_the_block() {
        // Block text starts at offset 10. Line 1 is " * @param ci Question?".
        let inner = "\n * @param ci Question?\n ";
        let d = parse_doc(inner, 10);
        let name = &d.params[0].name;
        assert_eq!(&inner[name.span.start - 10..name.span.end - 10], "ci");
        let text = &d.params[0].text;
        assert_eq!(&inner[text.span.start - 10..text.span.end - 10], "Question?");
    }
}
