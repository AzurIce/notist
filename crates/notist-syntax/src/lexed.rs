//! Source-backed token storage shared by syntax implementations.
use crate::tree::SyntaxKind;
use rowan::TextSize;

/// Token kinds and source offsets. Text is borrowed from the original source.
pub struct Lexed<'a> {
    src: &'a str,
    kinds: Vec<SyntaxKind>,
    starts: Vec<TextSize>,
}

impl<'a> Lexed<'a> {
    /// Build a token stream with a frontend-provided scanner.
    pub fn scan(src: &'a str, mut token: impl FnMut(usize) -> (SyntaxKind, usize)) -> Self {
        let mut kinds = Vec::new();
        let mut starts = Vec::new();
        let mut offset = 0;
        while offset < src.len() {
            let (kind, len) = token(offset);
            assert!(len > 0 && src.is_char_boundary(offset + len));
            starts.push(TextSize::new(offset as u32));
            kinds.push(kind);
            offset += len;
        }
        starts.push(TextSize::new(src.len() as u32));
        Self { src, kinds, starts }
    }
    pub fn kind(&self, i: usize) -> Option<SyntaxKind> {
        self.kinds.get(i).copied()
    }
    pub fn text(&self, i: usize) -> &'a str {
        &self.src[usize::from(self.starts[i])..usize::from(self.starts[i + 1])]
    }
    pub fn len(&self, i: usize) -> usize {
        self.text(i).len()
    }
    pub fn offset(&self, i: usize) -> TextSize {
        self.starts[i]
    }
}
