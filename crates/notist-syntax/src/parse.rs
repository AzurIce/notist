//! Shared lossless parse results and syntax diagnostics.
use crate::{content::ParsedContent, tree::SyntaxNode};
use rowan::{GreenNode, TextRange, TextSize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: TextRange,
    pub message: String,
}

#[derive(Debug)]
pub struct Parse {
    pub(crate) green: GreenNode,
    pub diagnostics: Vec<Diagnostic>,
    /// Frontend-owned content trees, associated by source range.
    pub contents: Vec<ParsedContent>,
}

impl Parse {
    pub fn new(green: GreenNode, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            green,
            diagnostics,
            contents: Vec::new(),
        }
    }

    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// Bytes consumed by a prefix parser; offsets are relative to its input.
    pub fn consumed(&self) -> TextSize {
        self.green.text_len()
    }
}
