//! Host diagnostic primitives shared across `notist-ssg` and `notist-cli`.
use std::{
    fmt,
    path::{Path, PathBuf},
};

/// UTF-8 byte offset within a source file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextSize(u32);

impl TextSize {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }
}

impl From<u32> for TextSize {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

impl From<TextSize> for u32 {
    fn from(size: TextSize) -> Self {
        size.0
    }
}

impl From<TextSize> for usize {
    fn from(size: TextSize) -> Self {
        size.0 as usize
    }
}

/// Half-open UTF-8 byte range `[start, end)` within a source file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TextRange {
    start: TextSize,
    end: TextSize,
}

impl TextRange {
    pub const fn new(start: TextSize, end: TextSize) -> Self {
        Self { start, end }
    }

    pub const fn empty(at: TextSize) -> Self {
        Self { start: at, end: at }
    }

    pub const fn start(self) -> TextSize {
        self.start
    }

    pub const fn end(self) -> TextSize {
        self.end
    }
}

/// Which compilation phase produced a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Phase {
    Syntax,
    Semantic,
    Type,
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Phase::Syntax => "syntax",
            Phase::Semantic => "semantic",
            Phase::Type => "type",
        })
    }
}

/// Stable classification for host policy, independent of diagnostic wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DiagnosticCode {
    OutsideVault,
    UnpublishedResource,
}

/// Source-scoped diagnostic with phase, byte span, message, and optional code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: TextRange,
    pub message: String,
    pub phase: Phase,
    pub code: Option<DiagnosticCode>,
}

impl Diagnostic {
    pub fn new(phase: Phase, span: TextRange, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            phase,
            code: None,
        }
    }
}

/// Diagnostic paired with the path and source text it points into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDiagnostic {
    pub path: PathBuf,
    pub source: String,
    pub diagnostic: Diagnostic,
}

impl SourceDiagnostic {
    pub fn locate(path: &Path, source: &str, diagnostic: Diagnostic) -> Self {
        Self {
            path: path.to_path_buf(),
            source: source.to_owned(),
            diagnostic,
        }
    }
}
