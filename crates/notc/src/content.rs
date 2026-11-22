//! Immutable single-node content model (`Inline` and `Block`) with structural
//! sharing (`Arc`), canonical flattening of unannotated fragments, attributes,
//! `sectionize`, and immutable tree rewriting.

use crate::ast::Span;
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Arc<str>),
}

/// Immutable ordered key-value attributes attached to a content node.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Attrs(Arc<[(Arc<str>, AttrValue)]>);

impl Attrs {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn entries(&self) -> &[(Arc<str>, AttrValue)] {
        &self.0
    }

    pub fn from_entries(entries: Vec<(Arc<str>, AttrValue)>) -> Self {
        let mut dedup: Vec<(Arc<str>, AttrValue)> = Vec::with_capacity(entries.len());
        for (k, v) in entries {
            if let Some(existing) = dedup.iter_mut().find(|(ek, _)| *ek == k) {
                existing.1 = v;
            } else {
                dedup.push((k, v));
            }
        }
        Self(Arc::from(dedup))
    }

    pub fn get(&self, key: &str) -> Option<&AttrValue> {
        self.0.iter().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v)
    }

    pub fn with(&self, key: Arc<str>, value: AttrValue) -> Self {
        let mut entries: Vec<(Arc<str>, AttrValue)> = self.0.iter().cloned().collect();
        if let Some(existing) = entries.iter_mut().find(|(k, _)| *k == key) {
            existing.1 = value;
        } else {
            entries.push((key, value));
        }
        Self(Arc::from(entries))
    }
}

/// Shared immutable inline content node.
#[derive(Debug, Clone, PartialEq)]
pub struct Inline(Arc<InlineNode>);

#[derive(Debug, Clone)]
pub struct InlineNode {
    pub kind: InlineKind,
    pub attrs: Attrs,
    pub span: Span,
}

impl PartialEq for InlineNode {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.attrs == other.attrs
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InlineKind {
    Fragment(Arc<[Inline]>),
    Text(Arc<str>),
    Strong(Inline),
    Emph(Inline),
    Raw(Arc<str>),
    Link { target: Arc<str>, label: Inline },
}

impl Inline {
    pub fn new(kind: InlineKind, attrs: Attrs, span: Span) -> Self {
        Self(Arc::new(InlineNode { kind, attrs, span }))
    }

    pub fn node(&self) -> &InlineNode {
        &self.0
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn text(text: impl Into<Arc<str>>, span: Span) -> Self {
        Self::new(InlineKind::Text(text.into()), Attrs::empty(), span)
    }

    pub fn empty(span: Span) -> Self {
        Self::new(InlineKind::Fragment(Arc::from([])), Attrs::empty(), span)
    }

    /// Construct a canonical inline fragment: unannotated child fragments
    /// flatten automatically, and an unannotated single-item fragment unwraps.
    pub fn fragment(items: Vec<Inline>, attrs: Attrs, span: Span) -> Self {
        let mut flat = Vec::with_capacity(items.len());
        for item in items {
            if item.0.attrs.is_empty()
                && let InlineKind::Fragment(children) = &item.0.kind
            {
                flat.extend(children.iter().cloned());
                continue;
            }
            flat.push(item);
        }
        if attrs.is_empty() && flat.len() == 1 {
            return flat.pop().expect("len == 1");
        }
        Self::new(InlineKind::Fragment(Arc::from(flat)), attrs, span)
    }

    pub fn concat(left: Inline, right: Inline, span: Span) -> Self {
        Self::fragment(vec![left, right], Attrs::empty(), span)
    }

    pub fn with_attr(&self, key: Arc<str>, value: AttrValue) -> Self {
        Self(Arc::new(InlineNode {
            kind: self.0.kind.clone(),
            attrs: self.0.attrs.with(key, value),
            span: self.0.span,
        }))
    }

    pub fn with_attrs(&self, attrs: Attrs) -> Self {
        if attrs.is_empty() {
            return self.clone();
        }
        let mut merged = self.0.attrs.clone();
        for (k, v) in attrs.entries() {
            merged = merged.with(Arc::clone(k), v.clone());
        }
        Self(Arc::new(InlineNode {
            kind: self.0.kind.clone(),
            attrs: merged,
            span: self.0.span,
        }))
    }

    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        self.push_plain_text(&mut out);
        out
    }

    fn push_plain_text(&self, out: &mut String) {
        match &self.0.kind {
            InlineKind::Fragment(children) => {
                for child in children.iter() {
                    child.push_plain_text(out);
                }
            }
            InlineKind::Text(s) | InlineKind::Raw(s) => out.push_str(s),
            InlineKind::Strong(inner) | InlineKind::Emph(inner) => inner.push_plain_text(out),
            InlineKind::Link { label, .. } => label.push_plain_text(out),
        }
    }

    /// Bottom-up immutable rewrite with structural sharing: unchanged subtrees
    /// retain their original `Arc` pointers.
    pub fn rewrite_bottom_up<E>(
        &self,
        f: &mut impl FnMut(Inline) -> Result<Inline, E>,
    ) -> Result<Inline, E> {
        let rebuilt = match &self.0.kind {
            InlineKind::Fragment(children) => {
                let mut changed = false;
                let mut next = Vec::with_capacity(children.len());
                for child in children.iter() {
                    let rewritten = child.rewrite_bottom_up(f)?;
                    if !rewritten.ptr_eq(child) {
                        changed = true;
                    }
                    next.push(rewritten);
                }
                if changed {
                    Inline::fragment(next, self.0.attrs.clone(), self.0.span)
                } else {
                    self.clone()
                }
            }
            InlineKind::Strong(inner) => {
                let rewritten = inner.rewrite_bottom_up(f)?;
                if rewritten.ptr_eq(inner) {
                    self.clone()
                } else {
                    Inline::new(
                        InlineKind::Strong(rewritten),
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            InlineKind::Emph(inner) => {
                let rewritten = inner.rewrite_bottom_up(f)?;
                if rewritten.ptr_eq(inner) {
                    self.clone()
                } else {
                    Inline::new(
                        InlineKind::Emph(rewritten),
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            InlineKind::Link { target, label } => {
                let rewritten = label.rewrite_bottom_up(f)?;
                if rewritten.ptr_eq(label) {
                    self.clone()
                } else {
                    Inline::new(
                        InlineKind::Link {
                            target: Arc::clone(target),
                            label: rewritten,
                        },
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            InlineKind::Text(_) | InlineKind::Raw(_) => self.clone(),
        };
        f(rebuilt)
    }
}

/// Shared immutable block content node.
#[derive(Debug, Clone, PartialEq)]
pub struct Block(Arc<BlockNode>);

#[derive(Debug, Clone)]
pub struct BlockNode {
    pub kind: BlockKind,
    pub attrs: Attrs,
    pub span: Span,
}

impl PartialEq for BlockNode {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.attrs == other.attrs
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlockKind {
    Fragment(Arc<[Block]>),
    Paragraph(Inline),
    Heading { level: u8, title: Inline },
    Callout { kind: Arc<str>, body: Block },
    CodeBlock { lang: Arc<str>, text: Arc<str> },
    Section { heading: Block, body: Block },
}

impl Block {
    pub fn new(kind: BlockKind, attrs: Attrs, span: Span) -> Self {
        Self(Arc::new(BlockNode { kind, attrs, span }))
    }

    pub fn node(&self) -> &BlockNode {
        &self.0
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn empty(span: Span) -> Self {
        Self::new(BlockKind::Fragment(Arc::from([])), Attrs::empty(), span)
    }

    /// Construct a canonical block fragment: unannotated child fragments
    /// flatten automatically, and an unannotated single-item fragment unwraps.
    pub fn fragment(items: Vec<Block>, attrs: Attrs, span: Span) -> Self {
        let mut flat = Vec::with_capacity(items.len());
        for item in items {
            if item.0.attrs.is_empty()
                && let BlockKind::Fragment(children) = &item.0.kind
            {
                flat.extend(children.iter().cloned());
                continue;
            }
            flat.push(item);
        }
        if attrs.is_empty() && flat.len() == 1 {
            return flat.pop().expect("len == 1");
        }
        Self::new(BlockKind::Fragment(Arc::from(flat)), attrs, span)
    }

    pub fn concat(left: Block, right: Block, span: Span) -> Self {
        Self::fragment(vec![left, right], Attrs::empty(), span)
    }

    pub fn with_attr(&self, key: Arc<str>, value: AttrValue) -> Self {
        Self(Arc::new(BlockNode {
            kind: self.0.kind.clone(),
            attrs: self.0.attrs.with(key, value),
            span: self.0.span,
        }))
    }

    pub fn with_attrs(&self, attrs: Attrs) -> Self {
        if attrs.is_empty() {
            return self.clone();
        }
        let mut merged = self.0.attrs.clone();
        for (k, v) in attrs.entries() {
            merged = merged.with(Arc::clone(k), v.clone());
        }
        Self(Arc::new(BlockNode {
            kind: self.0.kind.clone(),
            attrs: merged,
            span: self.0.span,
        }))
    }

    pub fn plain_text(&self) -> String {
        let mut parts = Vec::new();
        self.collect_plain_text(&mut parts);
        parts.join("\n")
    }

    fn collect_plain_text(&self, out: &mut Vec<String>) {
        match &self.0.kind {
            BlockKind::Fragment(children) => {
                for child in children.iter() {
                    child.collect_plain_text(out);
                }
            }
            BlockKind::Paragraph(inline) | BlockKind::Heading { title: inline, .. } => {
                out.push(inline.plain_text());
            }
            BlockKind::Callout { body, .. } => body.collect_plain_text(out),
            BlockKind::CodeBlock { text, .. } => out.push(text.to_string()),
            BlockKind::Section { heading, body } => {
                heading.collect_plain_text(out);
                body.collect_plain_text(out);
            }
        }
    }

    pub fn headings(&self) -> Vec<(u8, String)> {
        let mut out = Vec::new();
        self.collect_headings(&mut out);
        out
    }

    fn collect_headings(&self, out: &mut Vec<(u8, String)>) {
        match &self.0.kind {
            BlockKind::Fragment(children) => {
                for child in children.iter() {
                    child.collect_headings(out);
                }
            }
            BlockKind::Heading { level, title } => out.push((*level, title.plain_text())),
            BlockKind::Callout { body, .. } => body.collect_headings(out),
            BlockKind::Section { heading, body } => {
                heading.collect_headings(out);
                body.collect_headings(out);
            }
            BlockKind::Paragraph(_) | BlockKind::CodeBlock { .. } => {}
        }
    }

    /// Fold flat headings and subsequent sibling blocks into nested `Section`
    /// nodes by heading level. Pure library operation decoupled from parsing.
    pub fn sectionize(&self) -> Block {
        let items: Vec<Block> = match &self.0.kind {
            BlockKind::Fragment(children) => children.iter().cloned().collect(),
            _ => vec![self.clone()],
        };

        struct OpenSection {
            level: u8,
            heading: Block,
            body: Vec<Block>,
        }

        let mut root: Vec<Block> = Vec::new();
        let mut stack: Vec<OpenSection> = Vec::new();

        let close_top = |stack: &mut Vec<OpenSection>, root: &mut Vec<Block>| {
            let section = stack.pop().expect("non-empty section stack");
            let span = section.heading.0.span;
            let body = Block::fragment(section.body, Attrs::empty(), span);
            let node = Block::new(
                BlockKind::Section {
                    heading: section.heading,
                    body,
                },
                Attrs::empty(),
                span,
            );
            if let Some(parent) = stack.last_mut() {
                parent.body.push(node);
            } else {
                root.push(node);
            }
        };

        for item in items {
            if let BlockKind::Heading { level, .. } = item.0.kind {
                while stack.last().is_some_and(|top| top.level >= level) {
                    close_top(&mut stack, &mut root);
                }
                stack.push(OpenSection {
                    level,
                    heading: item,
                    body: Vec::new(),
                });
            } else if let Some(parent) = stack.last_mut() {
                parent.body.push(item);
            } else {
                root.push(item);
            }
        }
        while !stack.is_empty() {
            close_top(&mut stack, &mut root);
        }

        Block::fragment(root, self.0.attrs.clone(), self.0.span)
    }

    pub fn rewrite_inlines<E>(
        &self,
        f: &mut impl FnMut(Inline) -> Result<Inline, E>,
    ) -> Result<Block, E> {
        Ok(match &self.0.kind {
            BlockKind::Fragment(children) => {
                let mut changed = false;
                let mut next = Vec::with_capacity(children.len());
                for child in children.iter() {
                    let rewritten = child.rewrite_inlines(f)?;
                    if !rewritten.ptr_eq(child) {
                        changed = true;
                    }
                    next.push(rewritten);
                }
                if changed {
                    Block::fragment(next, self.0.attrs.clone(), self.0.span)
                } else {
                    self.clone()
                }
            }
            BlockKind::Paragraph(inline) => {
                let rewritten = inline.rewrite_bottom_up(f)?;
                if rewritten.ptr_eq(inline) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::Paragraph(rewritten),
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            BlockKind::Heading { level, title } => {
                let rewritten = title.rewrite_bottom_up(f)?;
                if rewritten.ptr_eq(title) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::Heading {
                            level: *level,
                            title: rewritten,
                        },
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            BlockKind::Callout { kind, body } => {
                let rewritten = body.rewrite_inlines(f)?;
                if rewritten.ptr_eq(body) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::Callout {
                            kind: Arc::clone(kind),
                            body: rewritten,
                        },
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            BlockKind::Section { heading, body } => {
                let h = heading.rewrite_inlines(f)?;
                let b = body.rewrite_inlines(f)?;
                if h.ptr_eq(heading) && b.ptr_eq(body) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::Section { heading: h, body: b },
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            BlockKind::CodeBlock { .. } => self.clone(),
        })
    }

    pub fn rewrite_blocks<E>(
        &self,
        f: &mut impl FnMut(Block) -> Result<Block, E>,
    ) -> Result<Block, E> {
        let rebuilt = match &self.0.kind {
            BlockKind::Fragment(children) => {
                let mut changed = false;
                let mut next = Vec::with_capacity(children.len());
                for child in children.iter() {
                    let rewritten = child.rewrite_blocks(f)?;
                    if !rewritten.ptr_eq(child) {
                        changed = true;
                    }
                    next.push(rewritten);
                }
                if changed {
                    Block::fragment(next, self.0.attrs.clone(), self.0.span)
                } else {
                    self.clone()
                }
            }
            BlockKind::Callout { kind, body } => {
                let rewritten = body.rewrite_blocks(f)?;
                if rewritten.ptr_eq(body) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::Callout {
                            kind: Arc::clone(kind),
                            body: rewritten,
                        },
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            BlockKind::Section { heading, body } => {
                let h = heading.rewrite_blocks(f)?;
                let b = body.rewrite_blocks(f)?;
                if h.ptr_eq(heading) && b.ptr_eq(body) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::Section { heading: h, body: b },
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            BlockKind::Paragraph(_) | BlockKind::Heading { .. } | BlockKind::CodeBlock { .. } => {
                self.clone()
            }
        };
        f(rebuilt)
    }
}

impl fmt::Display for Inline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.0.attrs.is_empty() {
            write!(f, "@(")?;
            for (i, (k, v)) in self.0.attrs.entries().iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{k}: {v}")?;
            }
            write!(f, ")")?;
        }
        match &self.0.kind {
            InlineKind::Fragment(children) => {
                write!(f, "Fragment[")?;
                for (i, child) in children.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{child}")?;
                }
                write!(f, "]")
            }
            InlineKind::Text(s) => write!(f, "Text({s:?})"),
            InlineKind::Strong(inner) => write!(f, "Strong({inner})"),
            InlineKind::Emph(inner) => write!(f, "Emph({inner})"),
            InlineKind::Raw(s) => write!(f, "Raw({s:?})"),
            InlineKind::Link { target, label } => write!(f, "Link({target:?}, {label})"),
        }
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.0.attrs.is_empty() {
            write!(f, "@(")?;
            for (i, (k, v)) in self.0.attrs.entries().iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{k}: {v}")?;
            }
            write!(f, ")")?;
        }
        match &self.0.kind {
            BlockKind::Fragment(children) => {
                write!(f, "BlockFragment[")?;
                for (i, child) in children.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{child}")?;
                }
                write!(f, "]")
            }
            BlockKind::Paragraph(inline) => write!(f, "Paragraph({inline})"),
            BlockKind::Heading { level, title } => write!(f, "Heading(h{level}, {title})"),
            BlockKind::Callout { kind, body } => write!(f, "Callout({kind:?}, {body})"),
            BlockKind::CodeBlock { lang, text } => write!(f, "CodeBlock({lang:?}, {text:?})"),
            BlockKind::Section { heading, body } => write!(f, "Section({heading}, {body})"),
        }
    }
}

impl fmt::Display for AttrValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int(n) => write!(f, "{n}"),
            Self::Float(v) => write!(f, "{v}"),
            Self::String(s) => write!(f, "{s:?}"),
        }
    }
}
