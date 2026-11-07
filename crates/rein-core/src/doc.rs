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
        assert!(d.tags.is_empty());
    }

    #[test]
    fn single_line_block_is_the_description() {
        assert_eq!(doc(" Page the engineer. ").description.value, "Page the engineer.");
        assert_eq!(doc("").description.value, "");
    }

    #[test]
    fn multi_line_parameter_doc_is_one_description() {
        let d = doc("\n * Is the customer a VIP?\n * Check the account tier in the CRM.\n ");
        assert_eq!(d.description.value, "Is the customer a VIP?\nCheck the account tier in the CRM.");
    }

    #[test]
    fn tags_are_recorded_by_name_and_stay_out_of_the_description() {
        let d = doc("\n * Desc.\n * @returns Something.\n *   more of returns.\n * @example foo\n ");
        assert_eq!(d.description.value, "Desc.");
        let names: Vec<&str> = d.tags.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(names, ["returns", "example"]);
    }

    #[test]
    fn param_tag_is_recorded_so_the_checker_can_reject_it() {
        let d = doc("\n * Desc.\n * @param ci What is CI?\n ");
        assert_eq!(d.tags[0].value, "param");
        assert_eq!(d.description.value, "Desc.");
    }

    #[test]
    fn tag_span_covers_the_at_sign_and_name() {
        let inner = "\n * @param ci Question?\n ";
        let d = parse_doc(inner, 10);
        let tag = &d.tags[0];
        assert_eq!(&inner[tag.span.start - 10..tag.span.end - 10], "@param");
    }

    #[test]
    fn spans_point_into_the_block() {
        // Block text starts at offset 10. The description line is " * Question?".
        let inner = "\n * Question?\n ";
        let d = parse_doc(inner, 10);
        let text = &d.description.span;
        assert_eq!(&inner[text.start - 10..text.end - 10], "Question?");
    }
}
