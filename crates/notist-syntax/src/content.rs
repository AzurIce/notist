//! Syntax-only interface for content dialects embedded in Code.
use std::{any::Any, fmt::Debug};

use rowan::{TextRange, TextSize};

use crate::Diagnostic;

/// A frontend-owned tree. Code retains it separately from its own CST.
pub trait ContentSyntax: Any + Debug {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Debug> ContentSyntax for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentFlavor {
    Inline,
    Block,
}

/// All ranges use byte offsets in the source passed to `parse_content`.
/// `range` includes the opening bracket and the closing bracket when present;
/// `body` excludes them. The frontend owns termination and recovery.
#[derive(Debug)]
pub struct ParsedContent {
    pub range: TextRange,
    pub body: TextRange,
    pub flavor: ContentFlavor,
    pub closed: bool,
    pub syntax: Box<dyn ContentSyntax>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse a bracketed content region beginning at `open`. Implementations
/// receive source coordinates, never a Code token cursor or tree builder.
pub trait ContentParser: Send + Sync {
    fn parse_content(&self, source: &str, open: TextSize) -> ParsedContent;
}
