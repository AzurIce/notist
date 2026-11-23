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

/// Renderer-agnostic value stored in an external/custom content node property.
#[derive(Debug, Clone, PartialEq)]
pub enum CustomValue {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Arc<str>),
    Inline(Inline),
    Block(Block),
}

/// Pure data payload produced by bodyless external content declarations
/// (`fn name(params...) -> Inline | Block;`). Completely decoupled from any
/// output renderer (HTML, Web Components, PDF, etc.).
#[derive(Debug, Clone, PartialEq)]
pub struct CustomNode {
    pub module: Arc<str>,
    pub name: Arc<str>,
    pub props: Arc<[(Arc<str>, CustomValue)]>,
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
    Math(Arc<str>),
    Link { target: Arc<str>, label: Inline },
    Image { target: Arc<str>, alt: Arc<str> },
    Custom(CustomNode),
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
            InlineKind::Text(s) | InlineKind::Raw(s) | InlineKind::Math(s) => out.push_str(s),
            InlineKind::Strong(inner) | InlineKind::Emph(inner) => inner.push_plain_text(out),
            InlineKind::Link { label, .. } => label.push_plain_text(out),
            InlineKind::Image { alt, .. } => out.push_str(alt),
            InlineKind::Custom(custom) => {
                for (_, val) in custom.props.iter() {
                    match val {
                        CustomValue::Inline(i) => i.push_plain_text(out),
                        CustomValue::Block(b) => out.push_str(&b.plain_text()),
                        _ => {}
                    }
                }
            }
        }
    }

    pub(crate) fn collect_links(&self, out: &mut Vec<(Arc<str>, Span)>) {
        match &self.0.kind {
            InlineKind::Fragment(children) => {
                for child in children.iter() {
                    child.collect_links(out);
                }
            }
            InlineKind::Strong(inner) | InlineKind::Emph(inner) => inner.collect_links(out),
            InlineKind::Link { target, label } => {
                out.push((Arc::clone(target), self.0.span));
                label.collect_links(out);
            }
            InlineKind::Custom(custom) => {
                for (_, val) in custom.props.iter() {
                    match val {
                        CustomValue::Inline(i) => i.collect_links(out),
                        CustomValue::Block(b) => b.collect_links(out),
                        _ => {}
                    }
                }
            }
            InlineKind::Text(_)
            | InlineKind::Raw(_)
            | InlineKind::Math(_)
            | InlineKind::Image { .. } => {}
        }
    }

    pub(crate) fn collect_images(&self, out: &mut Vec<(Arc<str>, Arc<str>, Span)>) {
        match &self.0.kind {
            InlineKind::Fragment(children) => {
                for child in children.iter() {
                    child.collect_images(out);
                }
            }
            InlineKind::Strong(inner) | InlineKind::Emph(inner) => inner.collect_images(out),
            InlineKind::Link { label, .. } => label.collect_images(out),
            InlineKind::Image { target, alt } => {
                out.push((Arc::clone(target), Arc::clone(alt), self.0.span));
            }
            InlineKind::Custom(custom) => {
                for (_, val) in custom.props.iter() {
                    match val {
                        CustomValue::Inline(i) => i.collect_images(out),
                        CustomValue::Block(b) => b.collect_images(out),
                        _ => {}
                    }
                }
            }
            InlineKind::Text(_) | InlineKind::Raw(_) | InlineKind::Math(_) => {}
        }
    }

    pub(crate) fn collect_ids(&self, out: &mut Vec<(Arc<str>, Span)>) {
        if let Some(AttrValue::String(id)) = self.0.attrs.get("id") {
            out.push((Arc::clone(id), self.0.span));
        }
        match &self.0.kind {
            InlineKind::Fragment(children) => {
                for child in children.iter() {
                    child.collect_ids(out);
                }
            }
            InlineKind::Strong(inner) | InlineKind::Emph(inner) => inner.collect_ids(out),
            InlineKind::Link { label, .. } => label.collect_ids(out),
            InlineKind::Custom(custom) => {
                for (_, val) in custom.props.iter() {
                    match val {
                        CustomValue::Inline(i) => i.collect_ids(out),
                        CustomValue::Block(b) => b.collect_ids(out),
                        _ => {}
                    }
                }
            }
            InlineKind::Text(_)
            | InlineKind::Raw(_)
            | InlineKind::Math(_)
            | InlineKind::Image { .. } => {}
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
            InlineKind::Custom(custom) => {
                let mut changed = false;
                let mut next_props = Vec::with_capacity(custom.props.len());
                for (k, v) in custom.props.iter() {
                    let nv = match v {
                        CustomValue::Inline(i) => {
                            let r = i.rewrite_bottom_up(f)?;
                            if !r.ptr_eq(i) {
                                changed = true;
                            }
                            CustomValue::Inline(r)
                        }
                        CustomValue::Block(b) => {
                            let r = b.rewrite_inlines(f)?;
                            if !r.ptr_eq(b) {
                                changed = true;
                            }
                            CustomValue::Block(r)
                        }
                        other => other.clone(),
                    };
                    next_props.push((Arc::clone(k), nv));
                }
                if changed {
                    Inline::new(
                        InlineKind::Custom(CustomNode {
                            module: Arc::clone(&custom.module),
                            name: Arc::clone(&custom.name),
                            props: Arc::from(next_props),
                        }),
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                } else {
                    self.clone()
                }
            }
            InlineKind::Text(_)
            | InlineKind::Raw(_)
            | InlineKind::Math(_)
            | InlineKind::Image { .. } => self.clone(),
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
    List { ordered: bool, items: Arc<[Block]> },
    Quote(Block),
    Divider,
    Table {
        headers: Arc<[Inline]>,
        rows: Arc<[Arc<[Inline]>]>,
    },
    Math(Arc<str>),
    Custom(CustomNode),
    Div(Block),
    SectionBox(Block),
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
            BlockKind::Callout { body, .. }
            | BlockKind::Quote(body)
            | BlockKind::Div(body)
            | BlockKind::SectionBox(body) => {
                body.collect_plain_text(out);
            }
            BlockKind::List { items, .. } => {
                for item in items.iter() {
                    item.collect_plain_text(out);
                }
            }
            BlockKind::CodeBlock { text, .. } | BlockKind::Math(text) => {
                out.push(text.to_string())
            }
            BlockKind::Table { headers, rows } => {
                for cell in headers.iter() {
                    out.push(cell.plain_text());
                }
                for row in rows.iter() {
                    for cell in row.iter() {
                        out.push(cell.plain_text());
                    }
                }
            }
            BlockKind::Custom(custom) => {
                for (_, val) in custom.props.iter() {
                    match val {
                        CustomValue::Inline(i) => out.push(i.plain_text()),
                        CustomValue::Block(b) => b.collect_plain_text(out),
                        _ => {}
                    }
                }
            }
            BlockKind::Divider => {}
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
            BlockKind::Callout { body, .. }
            | BlockKind::Quote(body)
            | BlockKind::Div(body)
            | BlockKind::SectionBox(body) => body.collect_headings(out),
            BlockKind::List { items, .. } => {
                for item in items.iter() {
                    item.collect_headings(out);
                }
            }
            BlockKind::Custom(custom) => {
                for (_, val) in custom.props.iter() {
                    if let CustomValue::Block(b) = val {
                        b.collect_headings(out);
                    }
                }
            }
            BlockKind::Section { heading, body } => {
                heading.collect_headings(out);
                body.collect_headings(out);
            }
            BlockKind::Paragraph(_)
            | BlockKind::CodeBlock { .. }
            | BlockKind::Table { .. }
            | BlockKind::Math(_)
            | BlockKind::Divider => {}
        }
    }

    pub fn links(&self) -> Vec<(Arc<str>, Span)> {
        let mut out = Vec::new();
        self.collect_links(&mut out);
        out
    }

    fn collect_links(&self, out: &mut Vec<(Arc<str>, Span)>) {
        match &self.0.kind {
            BlockKind::Fragment(children) => {
                for child in children.iter() {
                    child.collect_links(out);
                }
            }
            BlockKind::Paragraph(inline) | BlockKind::Heading { title: inline, .. } => {
                inline.collect_links(out);
            }
            BlockKind::Callout { body, .. }
            | BlockKind::Quote(body)
            | BlockKind::Div(body)
            | BlockKind::SectionBox(body) => body.collect_links(out),
            BlockKind::List { items, .. } => {
                for item in items.iter() {
                    item.collect_links(out);
                }
            }
            BlockKind::Table { headers, rows } => {
                for cell in headers.iter() {
                    cell.collect_links(out);
                }
                for row in rows.iter() {
                    for cell in row.iter() {
                        cell.collect_links(out);
                    }
                }
            }
            BlockKind::Custom(custom) => {
                for (_, val) in custom.props.iter() {
                    match val {
                        CustomValue::Inline(i) => i.collect_links(out),
                        CustomValue::Block(b) => b.collect_links(out),
                        _ => {}
                    }
                }
            }
            BlockKind::Section { heading, body } => {
                heading.collect_links(out);
                body.collect_links(out);
            }
            BlockKind::CodeBlock { .. } | BlockKind::Math(_) | BlockKind::Divider => {}
        }
    }

    pub fn images(&self) -> Vec<(Arc<str>, Arc<str>, Span)> {
        let mut out = Vec::new();
        self.collect_images(&mut out);
        out
    }

    fn collect_images(&self, out: &mut Vec<(Arc<str>, Arc<str>, Span)>) {
        match &self.0.kind {
            BlockKind::Fragment(children) => {
                for child in children.iter() {
                    child.collect_images(out);
                }
            }
            BlockKind::Paragraph(inline) | BlockKind::Heading { title: inline, .. } => {
                inline.collect_images(out);
            }
            BlockKind::Callout { body, .. }
            | BlockKind::Quote(body)
            | BlockKind::Div(body)
            | BlockKind::SectionBox(body) => body.collect_images(out),
            BlockKind::List { items, .. } => {
                for item in items.iter() {
                    item.collect_images(out);
                }
            }
            BlockKind::Table { headers, rows } => {
                for cell in headers.iter() {
                    cell.collect_images(out);
                }
                for row in rows.iter() {
                    for cell in row.iter() {
                        cell.collect_images(out);
                    }
                }
            }
            BlockKind::Custom(custom) => {
                for (_, val) in custom.props.iter() {
                    match val {
                        CustomValue::Inline(i) => i.collect_images(out),
                        CustomValue::Block(b) => b.collect_images(out),
                        _ => {}
                    }
                }
            }
            BlockKind::Section { heading, body } => {
                heading.collect_images(out);
                body.collect_images(out);
            }
            BlockKind::CodeBlock { .. } | BlockKind::Math(_) | BlockKind::Divider => {}
        }
    }

    pub fn ids(&self) -> Vec<(Arc<str>, Span)> {
        let mut out = Vec::new();
        self.collect_ids(&mut out);
        out
    }

    fn collect_ids(&self, out: &mut Vec<(Arc<str>, Span)>) {
        if let Some(AttrValue::String(id)) = self.0.attrs.get("id") {
            out.push((Arc::clone(id), self.0.span));
        }
        match &self.0.kind {
            BlockKind::Fragment(children) => {
                for child in children.iter() {
                    child.collect_ids(out);
                }
            }
            BlockKind::Paragraph(inline) | BlockKind::Heading { title: inline, .. } => {
                inline.collect_ids(out);
            }
            BlockKind::Callout { body, .. }
            | BlockKind::Quote(body)
            | BlockKind::Div(body)
            | BlockKind::SectionBox(body) => body.collect_ids(out),
            BlockKind::List { items, .. } => {
                for item in items.iter() {
                    item.collect_ids(out);
                }
            }
            BlockKind::Table { headers, rows } => {
                for cell in headers.iter() {
                    cell.collect_ids(out);
                }
                for row in rows.iter() {
                    for cell in row.iter() {
                        cell.collect_ids(out);
                    }
                }
            }
            BlockKind::Custom(custom) => {
                for (_, val) in custom.props.iter() {
                    match val {
                        CustomValue::Inline(i) => i.collect_ids(out),
                        CustomValue::Block(b) => b.collect_ids(out),
                        _ => {}
                    }
                }
            }
            BlockKind::Section { heading, body } => {
                heading.collect_ids(out);
                body.collect_ids(out);
            }
            BlockKind::CodeBlock { .. } | BlockKind::Math(_) | BlockKind::Divider => {}
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
            let item = match &item.0.kind {
                BlockKind::Div(body) => {
                    let s = body.sectionize();
                    if s.ptr_eq(body) {
                        item
                    } else {
                        Block::new(BlockKind::Div(s), item.0.attrs.clone(), item.0.span)
                    }
                }
                BlockKind::SectionBox(body) => {
                    let s = body.sectionize();
                    if s.ptr_eq(body) {
                        item
                    } else {
                        Block::new(BlockKind::SectionBox(s), item.0.attrs.clone(), item.0.span)
                    }
                }
                _ => item,
            };
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
            BlockKind::List { ordered, items } => {
                let mut changed = false;
                let mut next = Vec::with_capacity(items.len());
                for item in items.iter() {
                    let rewritten = item.rewrite_inlines(f)?;
                    if !rewritten.ptr_eq(item) {
                        changed = true;
                    }
                    next.push(rewritten);
                }
                if changed {
                    Block::new(
                        BlockKind::List {
                            ordered: *ordered,
                            items: Arc::from(next),
                        },
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                } else {
                    self.clone()
                }
            }
            BlockKind::Quote(body) => {
                let rewritten = body.rewrite_inlines(f)?;
                if rewritten.ptr_eq(body) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::Quote(rewritten),
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            BlockKind::Div(body) => {
                let rewritten = body.rewrite_inlines(f)?;
                if rewritten.ptr_eq(body) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::Div(rewritten),
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            BlockKind::SectionBox(body) => {
                let rewritten = body.rewrite_inlines(f)?;
                if rewritten.ptr_eq(body) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::SectionBox(rewritten),
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
            BlockKind::Table { headers, rows } => {
                let mut changed = false;
                let mut next_headers = Vec::with_capacity(headers.len());
                for cell in headers.iter() {
                    let r = cell.rewrite_bottom_up(f)?;
                    if !r.ptr_eq(cell) {
                        changed = true;
                    }
                    next_headers.push(r);
                }
                let mut next_rows = Vec::with_capacity(rows.len());
                for row in rows.iter() {
                    let mut next_row = Vec::with_capacity(row.len());
                    for cell in row.iter() {
                        let r = cell.rewrite_bottom_up(f)?;
                        if !r.ptr_eq(cell) {
                            changed = true;
                        }
                        next_row.push(r);
                    }
                    next_rows.push(Arc::from(next_row));
                }
                if changed {
                    Block::new(
                        BlockKind::Table {
                            headers: Arc::from(next_headers),
                            rows: Arc::from(next_rows),
                        },
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                } else {
                    self.clone()
                }
            }
            BlockKind::Custom(custom) => {
                let mut changed = false;
                let mut next_props = Vec::with_capacity(custom.props.len());
                for (k, v) in custom.props.iter() {
                    let nv = match v {
                        CustomValue::Inline(i) => {
                            let r = i.rewrite_bottom_up(f)?;
                            if !r.ptr_eq(i) {
                                changed = true;
                            }
                            CustomValue::Inline(r)
                        }
                        CustomValue::Block(b) => {
                            let r = b.rewrite_inlines(f)?;
                            if !r.ptr_eq(b) {
                                changed = true;
                            }
                            CustomValue::Block(r)
                        }
                        other => other.clone(),
                    };
                    next_props.push((Arc::clone(k), nv));
                }
                if changed {
                    Block::new(
                        BlockKind::Custom(CustomNode {
                            module: Arc::clone(&custom.module),
                            name: Arc::clone(&custom.name),
                            props: Arc::from(next_props),
                        }),
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                } else {
                    self.clone()
                }
            }
            BlockKind::CodeBlock { .. } | BlockKind::Math(_) | BlockKind::Divider => self.clone(),
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
            BlockKind::List { ordered, items } => {
                let mut changed = false;
                let mut next = Vec::with_capacity(items.len());
                for item in items.iter() {
                    let rewritten = item.rewrite_blocks(f)?;
                    if !rewritten.ptr_eq(item) {
                        changed = true;
                    }
                    next.push(rewritten);
                }
                if changed {
                    Block::new(
                        BlockKind::List {
                            ordered: *ordered,
                            items: Arc::from(next),
                        },
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                } else {
                    self.clone()
                }
            }
            BlockKind::Quote(body) => {
                let rewritten = body.rewrite_blocks(f)?;
                if rewritten.ptr_eq(body) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::Quote(rewritten),
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            BlockKind::Div(body) => {
                let rewritten = body.rewrite_blocks(f)?;
                if rewritten.ptr_eq(body) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::Div(rewritten),
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            BlockKind::SectionBox(body) => {
                let rewritten = body.rewrite_blocks(f)?;
                if rewritten.ptr_eq(body) {
                    self.clone()
                } else {
                    Block::new(
                        BlockKind::SectionBox(rewritten),
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                }
            }
            BlockKind::Custom(custom) => {
                let mut changed = false;
                let mut next_props = Vec::with_capacity(custom.props.len());
                for (k, v) in custom.props.iter() {
                    let nv = match v {
                        CustomValue::Block(b) => {
                            let r = b.rewrite_blocks(f)?;
                            if !r.ptr_eq(b) {
                                changed = true;
                            }
                            CustomValue::Block(r)
                        }
                        other => other.clone(),
                    };
                    next_props.push((Arc::clone(k), nv));
                }
                if changed {
                    Block::new(
                        BlockKind::Custom(CustomNode {
                            module: Arc::clone(&custom.module),
                            name: Arc::clone(&custom.name),
                            props: Arc::from(next_props),
                        }),
                        self.0.attrs.clone(),
                        self.0.span,
                    )
                } else {
                    self.clone()
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
            BlockKind::Paragraph(_)
            | BlockKind::Heading { .. }
            | BlockKind::CodeBlock { .. }
            | BlockKind::Table { .. }
            | BlockKind::Math(_)
            | BlockKind::Divider => self.clone(),
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
            InlineKind::Math(s) => write!(f, "Math({s:?})"),
            InlineKind::Link { target, label } => write!(f, "Link({target:?}, {label})"),
            InlineKind::Image { target, alt } => write!(f, "Image({target:?}, {alt:?})"),
            InlineKind::Custom(c) => write!(f, "Custom({}::{})", c.module, c.name),
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
            BlockKind::List { ordered, items } => {
                write!(f, "{}[", if *ordered { "OrderedList" } else { "List" })?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            BlockKind::Quote(body) => write!(f, "Quote({body})"),
            BlockKind::Divider => write!(f, "Divider"),
            BlockKind::Table { headers, rows } => {
                write!(f, "Table({} cols, {} rows)", headers.len(), rows.len())
            }
            BlockKind::Math(s) => write!(f, "BlockMath({s:?})"),
            BlockKind::Custom(c) => write!(f, "CustomBlock({}::{})", c.module, c.name),
            BlockKind::Div(body) => write!(f, "Div({body})"),
            BlockKind::SectionBox(body) => write!(f, "SectionBox({body})"),
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
