//! Source spans and the diagnostics that point at them.

use serde::Serialize;

/// Byte range in the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn to(self, other: Span) -> Span {
        Span::new(self.start, other.end)
    }

    pub fn contains(self, offset: usize) -> bool {
        self.start <= offset && offset <= self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// The `code` of a deprecation warning. The language server shows it and links it to the docs.
pub const DEPRECATED_CODE: &str = "deprecated-reference";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub span: Span,
    pub message: String,
    /// A short name for the kind of diagnostic, such as [`DEPRECATED_CODE`]. `None` for most diagnostics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Self { severity: Severity::Error, span, message: message.into(), code: None }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, span, message: message.into(), code: None }
    }

    /// A warning that a form is deprecated. It has the code [`DEPRECATED_CODE`].
    pub fn deprecated(span: Span, message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, span, message: message.into(), code: Some(DEPRECATED_CODE) }
    }

    /// True for a deprecation warning, see [`Diagnostic::deprecated`].
    pub fn is_deprecated(&self) -> bool {
        self.code == Some(DEPRECATED_CODE)
    }
}

/// 0-based line and column (column in chars) for an offset; used by the CLI.
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let before = &src[..offset.min(src.len())];
    let line = before.matches('\n').count();
    let col = before.rsplit('\n').next().unwrap_or("").chars().count();
    (line, col)
}
