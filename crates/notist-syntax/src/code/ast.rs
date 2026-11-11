//! Typed syntax views shared by Code modules and content frontends.
use crate::tree::{SyntaxKind, SyntaxNode, SyntaxToken};
use rowan::{NodeOrToken, TextRange};

mod module;
pub use module::{
    ChildrenDecl, DefaultValue, FunctionDecl, ImportDecl, ImportMember, Module, Parameter,
    ParameterList, TypeRef,
};

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
        matches!(
            node.kind(),
            SyntaxKind::Annotation | SyntaxKind::BlockAnnotation
        )
        .then_some(Self(node))
    }

    /// Whether this annotation was parsed at a block boundary.
    pub fn is_block(&self) -> bool {
        self.0.kind() == SyntaxKind::BlockAnnotation
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

/// A content literal's frame. A native dialect parses its body in place as a
/// `Document` child; an external dialect keeps the payload opaque and its tree
/// in `Parse::contents`.
pub struct ContentLiteral(pub(crate) SyntaxNode);

impl ContentLiteral {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::ContentLiteral).then_some(Self(node))
    }

    /// The in-place body, when the literal was parsed by a native dialect.
    pub fn document(&self) -> Option<SyntaxNode> {
        self.0
            .children()
            .find(|node| node.kind() == SyntaxKind::Document)
    }

    /// The opaque body of an external dialect's literal.
    pub fn body(&self) -> Option<SyntaxToken> {
        self.0
            .children_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| token.kind() == SyntaxKind::ContentText)
    }

    pub fn has_closer(&self) -> bool {
        self.0
            .children_with_tokens()
            .any(|element| element.kind() == SyntaxKind::RBracket)
    }

    /// Whether the body is block-level. Whitespace just inside the opening
    /// bracket decides it, so the flavor is a property of the literal itself.
    pub fn flavor(&self) -> crate::content::ContentFlavor {
        // An in-place body keeps its leading token inside the `Document`; an
        // opaque body is a single text token, so its own text decides.
        let padded = match self.document() {
            Some(document) => document.first_child_or_token().is_some_and(|element| {
                matches!(
                    element.kind(),
                    SyntaxKind::Whitespace | SyntaxKind::Newline
                )
            }),
            None => self
                .body()
                .is_some_and(|token| token.text().starts_with([' ', '\t', '\r', '\n'])),
        };
        if padded {
            crate::content::ContentFlavor::Block
        } else {
            crate::content::ContentFlavor::Inline
        }
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }
}

#[derive(Debug, Clone)]
pub struct Path(pub(crate) SyntaxNode);

impl Path {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::Path).then_some(Self(node))
    }

    pub fn segments(&self) -> impl Iterator<Item = SyntaxToken> + '_ {
        self.0
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .filter(|token| token.kind() == SyntaxKind::Ident)
    }

    pub fn text(&self) -> String {
        self.0.text().to_string()
    }

    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

impl CodeCall {
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::CodeCall).then_some(Self(node))
    }

    /// The complete call target, including any module/module segments.
    pub fn name(&self) -> Option<String> {
        self.callee().map(|callee| callee.text().to_string())
    }

    pub fn callee(&self) -> Option<SyntaxNode> {
        self.0
            .children()
            .find(|n| matches!(n.kind(), SyntaxKind::Path | SyntaxKind::MemberExpr))
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
