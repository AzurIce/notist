use rowan::{NodeOrToken, TextRange};

use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

pub struct Document(pub(crate) SyntaxNode);
pub struct Heading(pub(crate) SyntaxNode);
pub struct Paragraph(pub(crate) SyntaxNode);
pub struct Inline(pub(crate) SyntaxNode);

pub enum Block {
    Heading(Heading),
    Paragraph(Paragraph),
    List(List),
    Annotation(Annotation),
}

impl Block {
    fn cast(node: SyntaxNode) -> Option<Self> {
        match node.kind() {
            SyntaxKind::Heading => Some(Block::Heading(Heading(node))),
            SyntaxKind::Paragraph => Some(Block::Paragraph(Paragraph(node))),
            SyntaxKind::List => Some(Block::List(List(node))),
            SyntaxKind::Annotation => Some(Block::Annotation(Annotation(node))),
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
pub struct List(pub(crate) SyntaxNode);
pub struct ListItem(pub(crate) SyntaxNode);
/// The text between the first balanced `()` pair of a node (depth-aware),
/// and its absolute start offset.
fn paren_interior(node: &SyntaxNode) -> (String, u32) {
    let mut depth = 0usize;
    let mut text = String::new();
    let mut base = None;
    for token in node.children_with_tokens().filter_map(|e| e.into_token()) {
        match token.kind() {
            SyntaxKind::LParen => {
                depth += 1;
                if depth > 1 {
                    text.push_str(token.text());
                }
            }
            SyntaxKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                text.push_str(token.text());
            }
            _ if depth >= 1 => {
                if base.is_none() {
                    base = Some(u32::from(token.text_range().start()));
                }
                text.push_str(token.text());
            }
            _ => {}
        }
    }
    (text, base.unwrap_or(0))
}

pub struct Annotation(pub(crate) SyntaxNode);

impl Annotation {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::Annotation).then_some(Self(node))
    }

    pub fn is_module(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == SyntaxKind::Bang)
    }

    /// The text between the parens, and its absolute start offset.
    pub fn payload(&self) -> (String, u32) {
        paren_interior(&self.0)
    }

    pub fn entries(&self) -> impl Iterator<Item = Entry> + '_ {
        self.0.children().filter_map(Entry::cast)
    }

    /// Payload elements that are not entries (stray values; a payload must
    /// be all `key: value`).
    pub fn stray_elements(&self) -> Vec<NodeOrToken<SyntaxNode, SyntaxToken>> {
        self.0
            .children_with_tokens()
            .filter(|el| {
                !matches!(
                    el.kind(),
                    SyntaxKind::At
                        | SyntaxKind::Bang
                        | SyntaxKind::LParen
                        | SyntaxKind::RParen
                        | SyntaxKind::Comma
                        | SyntaxKind::Whitespace
                        | SyntaxKind::Newline
                        | SyntaxKind::LineComment
                        | SyntaxKind::BlockComment
                        | SyntaxKind::Entry
                )
            })
            .collect()
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

pub struct CodeCall(pub(crate) SyntaxNode);

impl CodeCall {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::CodeCall).then_some(Self(node))
    }

    /// The constructor name: the `Ident` token after `#`.
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == SyntaxKind::Ident)
            .map(|t| t.text().to_string())
    }

    /// The argument members: `Entry` nodes and bare literal elements between
    /// the parens (trivia and commas excluded).
    pub fn args(&self) -> Vec<NodeOrToken<SyntaxNode, SyntaxToken>> {
        let mut depth = 0usize;
        let mut out = Vec::new();
        for element in self.0.children_with_tokens() {
            match element.kind() {
                SyntaxKind::LParen => depth += 1,
                SyntaxKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ if depth == 1 => {
                    let is_trivia = matches!(
                        element.kind(),
                        SyntaxKind::Whitespace
                            | SyntaxKind::Newline
                            | SyntaxKind::LineComment
                            | SyntaxKind::BlockComment
                            | SyntaxKind::Comma
                    );
                    if !is_trivia {
                        out.push(element);
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// The `[..]` body, parsed as inline markup.
    pub fn body(&self) -> Option<Inline> {
        self.0.children().find_map(Inline::cast)
    }
}

/// A `key: value` entry in a dict literal, annotation payload, or call
/// argument list.
pub struct Entry(pub(crate) SyntaxNode);

impl Entry {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::Entry).then_some(Self(node))
    }

    pub fn key_token(&self) -> Option<SyntaxToken> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| matches!(t.kind(), SyntaxKind::Ident | SyntaxKind::Str))
    }

    pub fn value(&self) -> Option<NodeOrToken<SyntaxNode, SyntaxToken>> {
        let mut seen_colon = false;
        for element in self.0.children_with_tokens() {
            match &element {
                NodeOrToken::Token(t) if t.kind() == SyntaxKind::Colon => {
                    seen_colon = true;
                    continue;
                }
                NodeOrToken::Token(t)
                    if matches!(
                        t.kind(),
                        SyntaxKind::Whitespace
                            | SyntaxKind::Newline
                            | SyntaxKind::LineComment
                            | SyntaxKind::BlockComment
                    ) =>
                {
                    continue;
                }
                _ if seen_colon => return Some(element),
                _ => {}
            }
        }
        None
    }
}

impl List {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::List).then_some(Self(node))
    }

    pub fn items(&self) -> impl Iterator<Item = ListItem> {
        self.0.children().filter_map(ListItem::cast)
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

impl ListItem {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::ListItem).then_some(Self(node))
    }

    pub fn marker(&self) -> Option<SyntaxKind> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .map(|t| t.kind())
            .find(|k| matches!(k, SyntaxKind::Minus | SyntaxKind::Plus))
    }

    pub fn inline(&self) -> Option<Inline> {
        self.0.children().find_map(Inline::cast)
    }

    pub fn lists(&self) -> impl Iterator<Item = List> {
        self.0.children().filter_map(List::cast)
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

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
