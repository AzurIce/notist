//! Source-positioned metadata carried alongside IR forests until shaping ends.

use rowan::TextRange;

use crate::item::Dict;

/// Where an annotation sits relative to the syntax around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationKind {
    /// A standalone line; it targets the first block after its position.
    Block,
    /// Written inside an inline sequence; it targets the immediately
    /// following inline element.
    Inline,
}

/// A node annotation waiting to be attached after shaping.
///
/// Annotations are not content nodes: their source span and dict travel
/// beside the forest, so shaping never has to preserve or split them.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub dict: Dict,
    pub span: TextRange,
    pub kind: AnnotationKind,
}

impl Annotation {
    pub fn map_spans(&mut self, map: &mut impl FnMut(TextRange) -> TextRange) {
        self.span = map(self.span);
    }
}
