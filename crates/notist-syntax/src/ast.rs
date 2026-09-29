use rowan::{NodeOrToken, TextRange};

use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

pub struct Document(pub(crate) SyntaxNode);
pub struct Heading(pub(crate) SyntaxNode);
pub struct Paragraph(pub(crate) SyntaxNode);
pub struct Inline(pub(crate) SyntaxNode);

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

    pub fn inline(&self) -> Option<Inline> {
        self.0.children().find_map(Inline::cast)
    }

    pub fn content_tokens(&self) -> Vec<SyntaxToken> {
        self.inline().map(|i| i.tokens()).unwrap_or_default()
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

impl Paragraph {
    pub fn inline(&self) -> Option<Inline> {
        self.0.children().find_map(Inline::cast)
    }

    pub fn lines(&self) -> Vec<Line> {
        self.inline().map(|i| i.lines()).unwrap_or_default()
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

pub struct Line {
    pub tokens: Vec<SyntaxToken>,
    pub newline: Option<SyntaxToken>,
}

pub struct Link(pub(crate) SyntaxNode);
pub struct WikiLink(pub(crate) SyntaxNode);

impl Link {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::Link).then_some(Self(node))
    }

    pub fn inline(&self) -> Option<Inline> {
        self.0.children().find_map(Inline::cast)
    }

    pub fn target_tokens(&self) -> Vec<SyntaxToken> {
        let mut inside = false;
        let mut out = Vec::new();
        for token in self.0.children_with_tokens().filter_map(|e| e.into_token()) {
            match token.kind() {
                SyntaxKind::LParen => inside = true,
                SyntaxKind::RParen if inside => break,
                _ if inside => out.push(token),
                _ => {}
            }
        }
        out
    }
}

impl WikiLink {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::WikiLink).then_some(Self(node))
    }

    pub fn inline(&self) -> Option<Inline> {
        self.0.children().find_map(Inline::cast)
    }

    pub fn target_tokens(&self) -> Vec<SyntaxToken> {
        let mut out = Vec::new();
        for token in self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .skip(2)
        {
            if matches!(token.kind(), SyntaxKind::Pipe | SyntaxKind::RBracket) {
                break;
            }
            out.push(token);
        }
        out
    }
}

impl Inline {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::Inline).then_some(Self(node))
    }

    pub fn tokens(&self) -> Vec<SyntaxToken> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect()
    }

    pub fn elements(&self) -> impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>> {
        self.0.children_with_tokens()
    }

    pub fn lines(&self) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut tokens = Vec::new();
        for element in self.0.children_with_tokens() {
            let Some(token) = element.into_token() else {
                continue;
            };
            if token.kind() == SyntaxKind::Newline {
                lines.push(Line {
                    tokens: std::mem::take(&mut tokens),
                    newline: Some(token),
                });
            } else {
                tokens.push(token);
            }
        }
        if !tokens.is_empty() {
            lines.push(Line {
                tokens,
                newline: None,
            });
        }
        lines
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}
