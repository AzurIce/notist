use rowan::TextRange;

use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

pub struct Document(pub(crate) SyntaxNode);
pub struct Heading(pub(crate) SyntaxNode);
pub struct Paragraph(pub(crate) SyntaxNode);

pub enum Block {
    Heading(Heading),
    Paragraph(Paragraph),
}

impl Block {
    fn cast(node: SyntaxNode) -> Option<Self> {
        match node.kind() {
            SyntaxKind::Heading => Some(Block::Heading(Heading(node))),
            SyntaxKind::Paragraph => Some(Block::Paragraph(Paragraph(node))),
            _ => None,
        }
    }
}

impl Document {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::Document).then_some(Self(node))
    }

    pub fn blocks(&self) -> impl Iterator<Item = Block> {
        self.0.children().filter_map(Block::cast)
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

impl Heading {
    pub fn level(&self) -> usize {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == SyntaxKind::Eq)
            .map_or(0, |t| t.text().len())
    }

    pub fn content_tokens(&self) -> Vec<SyntaxToken> {
        let mut tokens = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token());
        tokens.next();
        tokens.next();
        tokens.collect()
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

impl Paragraph {
    pub fn lines(&self) -> Vec<Vec<SyntaxToken>> {
        let mut lines = vec![Vec::new()];
        for element in self.0.children_with_tokens() {
            let Some(token) = element.into_token() else {
                continue;
            };
            if token.kind() == SyntaxKind::Newline {
                lines.push(Vec::new());
            } else {
                lines.last_mut().unwrap().push(token);
            }
        }
        lines
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}
