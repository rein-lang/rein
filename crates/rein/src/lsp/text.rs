//! Text positions: byte offsets to LSP positions and back, and the name under the cursor.

use lsp_types::{Position, Range};
use rein_core::diagnostic::Span;

pub fn range(text: &str, span: Span) -> Range {
    Range::new(position(text, span.start), position(text, span.end))
}

/// LSP position for a byte offset. The column counts UTF-16 units.
pub fn position(text: &str, offset: usize) -> Position {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line = text[..offset].matches('\n').count();
    let character = text[line_start..offset].encode_utf16().count();
    Position::new(line as u32, character as u32)
}

/// Byte offset for an LSP position. A column past the end of its line clamps to the line end.
pub fn offset(text: &str, pos: Position) -> usize {
    let mut line_start = 0;
    for _ in 0..pos.line {
        match text[line_start..].find('\n') {
            Some(i) => line_start += i + 1,
            None => return text.len(),
        }
    }
    let mut units = 0u32;
    for (i, c) in text[line_start..].char_indices() {
        if c == '\n' || units >= pos.character {
            return line_start + i;
        }
        units += c.len_utf16() as u32;
    }
    text.len()
}

pub fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Start of the name that ends at or covers `offset`.
fn word_start(text: &str, offset: usize) -> usize {
    text[..offset]
        .char_indices()
        .rev()
        .find(|&(_, c)| !is_token_char(c))
        .map_or(0, |(i, c)| i + c.len_utf8())
}

/// End of the name that starts at or covers `offset`.
fn word_end(text: &str, offset: usize) -> usize {
    text[offset..]
        .char_indices()
        .find(|&(_, c)| !is_token_char(c))
        .map_or(text.len(), |(i, _)| offset + i)
}

/// The name at `offset`. Returns None inside a string literal.
pub fn token_at(text: &str, offset: usize) -> Option<&str> {
    let start = word_start(text, offset);
    let end = word_end(text, offset);
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let quotes = text[line_start..start].chars().filter(|&c| c == '"' || c == '\'').count();
    (start < end && quotes % 2 == 0).then(|| &text[start..end])
}

/// The text of the current line before `offset`.
pub fn line_before(text: &str, offset: usize) -> &str {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    &text[line_start..offset]
}

/// Characters a reference path can hold after `@`.
pub fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '/' || c == '.'
}

/// The import path from function `from` to function `to`, without `.rein`. Such as `./x` or `../x`.
pub fn relative_import(from: &str, to: &str) -> String {
    let from_dir: Vec<&str> = from.split('/').collect();
    let from_dir = &from_dir[..from_dir.len() - 1];
    let to_parts: Vec<&str> = to.split('/').collect();
    let common = from_dir.iter().zip(&to_parts[..to_parts.len() - 1]).take_while(|(a, b)| a == b).count();
    let ups = from_dir.len() - common;
    let prefix = if ups == 0 { "./".to_string() } else { "../".repeat(ups) };
    format!("{prefix}{}", to_parts[common..].join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_columns_count_surrogate_pairs() {
        // "😀" is 4 bytes but 2 UTF-16 units.
        let text = "a😀 @x\n";
        let at = text.find('@').unwrap();
        assert_eq!(position(text, at), Position::new(0, 4));
        assert_eq!(offset(text, Position::new(0, 4)), at);
    }

    #[test]
    fn utf16_column_resets_per_line() {
        let text = "é\nxé @y";
        let at = text.find('@').unwrap();
        assert_eq!(position(text, at), Position::new(1, 3));
        assert_eq!(offset(text, Position::new(1, 3)), at);
    }

    #[test]
    fn token_at_finds_import_and_parameter_names() {
        let text = "return mergePr;\n  if (docsOnly) {\n";
        let at_import = text.find("Pr").unwrap() + 1;
        assert_eq!(token_at(text, at_import).unwrap(), "mergePr");
        let at_param = text.find("docsOnly").unwrap() + 2;
        assert_eq!(token_at(text, at_param).unwrap(), "docsOnly");
    }

    #[test]
    fn token_inside_string_is_none() {
        let text = "import a from \"./ci/fix\";\n";
        let at = text.find("ci").unwrap() + 1;
        assert!(token_at(text, at).is_none());
        let text = "import a from './ci/fix';\n";
        let at = text.find("ci").unwrap() + 1;
        assert!(token_at(text, at).is_none());
    }

    #[test]
    fn relative_import_paths() {
        assert_eq!(relative_import("ci/fix", "merge_pr"), "../merge_pr");
        assert_eq!(relative_import("ci/fix", "ci/deeper"), "./deeper");
        assert_eq!(relative_import("ci/sub/leaf", "top"), "../../top");
        assert_eq!(relative_import("main", "ci/fix"), "./ci/fix");
    }
}
