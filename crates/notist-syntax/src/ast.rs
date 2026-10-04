use rowan::{NodeOrToken, TextRange};

use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

pub struct Document(pub(crate) SyntaxNode);
pub struct Heading(pub(crate) SyntaxNode);
pub struct Table(pub(crate) SyntaxNode);
pub struct TableRow(pub(crate) SyntaxNode);
pub struct TableCell(pub(crate) SyntaxNode);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAlignment {
    None,
    Left,
    Center,
    Right,
}

impl TableAlignment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

impl Table {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::Table).then_some(Self(node))
    }

    pub fn rows(&self) -> impl Iterator<Item = TableRow> + '_ {
        self.0.children().filter_map(TableRow::cast)
    }

    pub fn alignments(&self) -> Vec<TableAlignment> {
        let Some(delimiter) = self
            .0
            .children()
            .find(|n| n.kind() == SyntaxKind::TableDelimiter)
        else {
            return Vec::new();
        };
        delimiter
            .children()
            .filter_map(TableCell::cast)
            .map(|cell| {
                let text = cell.0.text().to_string();
                let text = text.trim();
                match (text.starts_with(':'), text.ends_with(':')) {
                    (true, true) => TableAlignment::Center,
                    (true, false) => TableAlignment::Left,
                    (false, true) => TableAlignment::Right,
                    (false, false) => TableAlignment::None,
                }
            })
            .collect()
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

impl TableRow {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::TableRow).then_some(Self(node))
    }

    pub fn cells(&self) -> impl Iterator<Item = TableCell> + '_ {
        self.0.children().filter_map(TableCell::cast)
    }

    pub fn is_header(&self) -> bool {
        self.0
            .parent()
            .filter(|parent| parent.kind() == SyntaxKind::Table)
            .and_then(|parent| parent.children().next())
            .is_some_and(|first| first == self.0)
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

impl TableCell {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::TableCell).then_some(Self(node))
    }

    pub fn content(&self) -> impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>> + '_ {
        self.0.children_with_tokens()
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

impl Document {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::Document).then_some(Self(node))
    }

    /// The flat element sequence: block nodes, inline content, and trivia.
    pub fn elements(&self) -> impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>> + '_ {
        self.0.children_with_tokens()
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

/// Split an element sequence into lines at newline tokens.
pub fn lines<'a>(
    elements: impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>> + 'a,
) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut tokens = Vec::new();
    for element in elements {
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

pub struct Line {
    pub tokens: Vec<SyntaxToken>,
    pub newline: Option<SyntaxToken>,
}

impl Heading {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::Heading).then_some(Self(node))
    }

    pub fn level(&self) -> usize {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == SyntaxKind::Eq)
            .map_or(0, |t| t.text().len())
    }

    /// The inline content after the marker and its following whitespace.
    pub fn content(&self) -> impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>> + '_ {
        self.0.children_with_tokens().skip(2)
    }

    pub fn content_tokens(&self) -> Vec<SyntaxToken> {
        self.content().filter_map(|e| e.into_token()).collect()
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

pub struct Link(pub(crate) SyntaxNode);
pub struct Embed(pub(crate) SyntaxNode);
pub struct WikiLink(pub(crate) SyntaxNode);
pub struct List(pub(crate) SyntaxNode);
pub struct ListItem(pub(crate) SyntaxNode);
/// The text between the first balanced `()` pair of a node (depth-aware),
/// and its absolute start offset.
fn paren_interior(node: &SyntaxNode) -> (String, u32) {
    let mut depth = 0usize;
    let mut text = String::new();
    let mut base = None;
    for token in node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
    {
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

    /// The payload node when it parsed as a dict literal.
    pub fn payload_dict(&self) -> Option<SyntaxNode> {
        self.0.children().find(|n| n.kind() == SyntaxKind::Dict)
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

    /// The `[` token opening the body: the first bracket outside the parens.
    fn body_open(&self) -> Option<SyntaxToken> {
        let mut paren = 0usize;
        for token in self.0.children_with_tokens().filter_map(|e| e.into_token()) {
            match token.kind() {
                SyntaxKind::LParen => paren += 1,
                SyntaxKind::RParen => paren -= 1,
                SyntaxKind::LBracket if paren == 0 => return Some(token),
                _ => {}
            }
        }
        None
    }

    pub fn has_body(&self) -> bool {
        self.body_open().is_some()
    }

    /// The `[..]` body: elements between the outer brackets (empty when the
    /// call has no body).
    pub fn body(&self) -> Vec<NodeOrToken<SyntaxNode, SyntaxToken>> {
        let Some(open) = self.body_open() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut depth = 0usize;
        for element in self
            .0
            .children_with_tokens()
            .skip_while(|el| *el != NodeOrToken::Token(open.clone()))
            .skip(1)
        {
            match element.kind() {
                SyntaxKind::LBracket => depth += 1,
                SyntaxKind::RBracket => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            out.push(element);
        }
        out
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
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

    /// The complete block content after the marker and its whitespace.
    pub fn content(&self) -> impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>> + '_ {
        let mut state = 0u8; // 0: pre-marker, 1: marker skipped, 2: post-ws skipped
        self.0
            .children_with_tokens()
            .skip_while(move |el| match el.kind() {
                SyntaxKind::Whitespace if state != 2 => {
                    if state == 1 {
                        state = 2;
                    }
                    true
                }
                SyntaxKind::Minus | SyntaxKind::Plus if state == 0 => {
                    state = 1;
                    true
                }
                _ => false,
            })
    }

    pub fn lists(&self) -> impl Iterator<Item = List> {
        self.0.children().filter_map(List::cast)
    }

    /// Absolute indentation of this item's content, including its marker.
    pub fn content_indent(&self) -> usize {
        let marker = self
            .0
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .find(|token| matches!(token.kind(), SyntaxKind::Minus | SyntaxKind::Plus))
            .unwrap();
        let mut prefix = Vec::new();
        let mut previous = marker.prev_token();
        while let Some(token) = previous {
            if let Some(newline) = token.text().rfind(['\n', '\r']) {
                prefix.push(token.text()[newline + 1..].to_string());
                break;
            }
            prefix.push(token.text().to_string());
            previous = token.prev_token();
        }
        prefix.reverse();
        let mut prefix = prefix.concat();
        prefix.push_str(marker.text());
        if let Some(token) = marker.next_token() {
            prefix.push_str(token.text());
        }
        prefix.chars().fold(0, |column, ch| {
            if ch == '\t' {
                (column / 4 + 1) * 4
            } else {
                column + 1
            }
        })
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

impl Link {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::Link).then_some(Self(node))
    }

    /// The link text: elements between the text brackets.
    pub fn content(&self) -> impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>> + '_ {
        self.0
            .children_with_tokens()
            .skip_while(|el| el.kind() != SyntaxKind::LBracket)
            .skip(1)
            .take_while(|el| el.kind() != SyntaxKind::RBracket)
    }

    pub fn target_tokens(&self) -> Vec<SyntaxToken> {
        destination_tokens(&self.0)
    }

    pub fn target(&self) -> String {
        destination(&self.0).0
    }
}

impl Embed {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::Embed).then_some(Self(node))
    }

    pub fn content(&self) -> impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>> + '_ {
        self.0
            .children_with_tokens()
            .skip_while(|el| el.kind() != SyntaxKind::LBracket)
            .skip(1)
            .take_while(|el| el.kind() != SyntaxKind::RBracket)
    }

    /// Resource destination and optional quoted title, with punctuation escapes removed.
    pub fn destination(&self) -> (String, Option<String>) {
        destination(&self.0)
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

fn destination_tokens(node: &SyntaxNode) -> Vec<SyntaxToken> {
    let mut tokens: Vec<_> = node
        .children_with_tokens()
        .skip_while(|el| el.kind() != SyntaxKind::RBracket)
        .skip(1)
        .skip_while(|el| el.kind() != SyntaxKind::LParen)
        .skip(1)
        .filter_map(|el| el.into_token())
        .collect();
    tokens.pop(); // the outer closing parenthesis
    tokens
}

fn destination(node: &SyntaxNode) -> (String, Option<String>) {
    let tokens = destination_tokens(node);
    let text: String = tokens.iter().map(|el| el.to_string()).collect();
    let text = text.trim();
    let mut title_start = None;
    let mut escaped = false;
    let mut previous_space = false;
    for (i, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            previous_space = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            previous_space = false;
            continue;
        }
        if previous_space && matches!(ch, '"' | '\'') {
            title_start = Some((i, ch));
            break;
        }
        previous_space = ch.is_whitespace();
    }
    let (target, title) = match title_start {
        Some((i, quote)) if text.ends_with(quote) && text.len() > i + 1 => {
            (text[..i].trim_end(), Some(&text[i + 1..text.len() - 1]))
        }
        _ => (text, None),
    };
    let target = target
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(target);
    (
        unescape_punctuation(target),
        title.map(unescape_punctuation),
    )
}

fn unescape_punctuation(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek().is_some_and(|next| next.is_ascii_punctuation()) {
            out.push(chars.next().unwrap());
        } else {
            out.push(ch);
        }
    }
    out
}

impl WikiLink {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::WikiLink).then_some(Self(node))
    }

    /// The link text: elements after the pipe (empty when there is no pipe).
    pub fn content(&self) -> impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>> + '_ {
        self.0
            .children_with_tokens()
            .skip_while(|el| el.kind() != SyntaxKind::Pipe)
            .skip(1)
            .take_while(|el| el.kind() != SyntaxKind::RBracket)
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
