//! The ownership boundary between evaluated `notc::Block` IR and whole-site
//! link resolution + HTML rendering.

use crate::{
    Error, Page, Route,
    document::{Document, diagnostic},
};
use notc::{
    AttrValue, Block, BlockKind, CustomNode, CustomValue, Inline, InlineKind, Span, TableAlign,
};
use notist::{Diagnostic, Phase, SourceDiagnostic, TextRange};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    path::PathBuf,
    sync::Arc,
};

pub(crate) struct Reference {
    pub target: String,
    pub is_image: bool,
    pub origin: PathBuf,
    span: Span,
    source_text: Arc<str>,
}

impl Reference {
    pub fn issue(&self, message: String) -> SourceDiagnostic {
        let range = TextRange::new(self.span.start.into(), self.span.end.into());
        let d = Diagnostic::new(Phase::Semantic, range, message);
        diagnostic(&self.origin, &self.source_text, d)
    }
}

/// Render an evaluated `notc::Block` directly to standalone HTML without site routing.
pub fn render_standalone_html(block: &Block) -> String {
    render_standalone_html_with_components(block, &BTreeMap::new())
}

/// Render an evaluated `notc::Block` directly to standalone HTML with an optional
/// `(module_virtual_path, name) -> custom_element_tag` map for Web Components.
pub fn render_standalone_html_with_components(
    block: &Block,
    web_components: &BTreeMap<(String, String), String>,
) -> String {
    let sectioned = block.sectionize();
    let route = Route::plan(PathBuf::from("./index.md"), std::path::Path::new("."), false)
        .expect("valid route");
    match PreparedPage::new(route, Arc::from(""), sectioned, "Notist") {
        Ok(prepared) => prepared.render_html(&[], web_components),
        Err(_) => String::new(),
    }
}

/// Collect unique `(module_virtual_path, name)` custom component nodes used in `block`.
pub fn collect_block_custom_nodes(block: &Block) -> BTreeSet<(String, String)> {
    let mut refs = Vec::new();
    let mut customs = BTreeSet::new();
    let origin = PathBuf::new();
    let empty_src: Arc<str> = Arc::from("");
    collect_block_references(block, &origin, &empty_src, &mut refs, &mut customs);
    customs
}

enum HtmlPlan {
    PreRendered {
        buffer: Box<str>,
        slots: Box<[HtmlSlot]>,
    },
    Deferred(Box<Block>),
}

struct HtmlSlot {
    offset: u32,
    ref_idx: u32,
    kind: HtmlSlotKind,
}

enum HtmlSlotKind {
    LinkOpen {
        fallback: Box<str>,
        attrs: Box<str>,
    },
    LinkClose {
        fallback_safe: bool,
    },
    Image {
        fallback: Box<str>,
        alt: Box<str>,
        attrs: Box<str>,
    },
}

pub(crate) struct PreparedPage {
    pub page: Page,
    plan: HtmlPlan,
    pub ids: BTreeSet<String>,
    heading_ids: Vec<String>,
    pub references: Vec<Reference>,
    /// Unique `(module_virtual_path, name)` custom content declarations used on this page.
    /// Used strictly by SSG to resolve optional web-component JS modules by convention.
    pub custom_nodes: BTreeSet<(String, String)>,
}

impl PreparedPage {
    pub fn new(
        route: Route,
        source_text: Arc<str>,
        block: Block,
        site_title: &str,
    ) -> Result<Self, Error> {
        let document = Document::from_block(route, block, site_title)?;
        let mut references = Vec::new();
        let mut custom_nodes = BTreeSet::new();
        collect_block_references(
            &document.root,
            &document.page.route.source,
            &source_text,
            &mut references,
            &mut custom_nodes,
        );
        let (plan, heading_ids) = if custom_nodes.is_empty() {
            let mut buffer = String::new();
            let mut slots = Vec::with_capacity(references.len() * 2);
            let mut ctx = RenderCtx {
                heading_ids: &document.heading_ids,
                heading_cursor: 0,
                ref_cursor: 0,
                target: RenderTarget::Plan { slots: &mut slots },
            };
            render_block(&document.root, &mut ctx, &mut buffer);
            // Drop `document.root` and `document.heading_ids` immediately so peak RSS stays bounded.
            (
                HtmlPlan::PreRendered {
                    buffer: buffer.into_boxed_str(),
                    slots: slots.into_boxed_slice(),
                },
                Vec::new(),
            )
        } else {
            (HtmlPlan::Deferred(Box::new(document.root)), document.heading_ids)
        };
        Ok(Self {
            page: document.page,
            plan,
            ids: document.ids,
            heading_ids,
            references,
            custom_nodes,
        })
    }

    pub fn render_html(
        &self,
        resolved_urls: &[Option<String>],
        web_components: &BTreeMap<(String, String), String>,
    ) -> String {
        match &self.plan {
            HtmlPlan::PreRendered { buffer, slots } => {
                if slots.is_empty() {
                    return buffer.to_string();
                }
                let mut out = String::with_capacity(buffer.len() + slots.len() * 32);
                let mut cursor = 0usize;
                for slot in slots.iter() {
                    let pos = slot.offset as usize;
                    out.push_str(&buffer[cursor..pos]);
                    cursor = pos;
                    let idx = slot.ref_idx as usize;
                    match &slot.kind {
                        HtmlSlotKind::LinkOpen { fallback, attrs } => {
                            let url = resolved_urls
                                .get(idx)
                                .and_then(|u| u.as_deref())
                                .unwrap_or(fallback.as_ref());
                            if notist_html::is_safe_url(url) {
                                let _ = write!(
                                    out,
                                    "<a href=\"{}\"{}>",
                                    notist_html::escape_attribute(url),
                                    attrs
                                );
                            }
                        }
                        HtmlSlotKind::LinkClose { fallback_safe } => {
                            let is_safe = resolved_urls
                                .get(idx)
                                .and_then(|u| u.as_deref())
                                .map(notist_html::is_safe_url)
                                .unwrap_or(*fallback_safe);
                            if is_safe {
                                out.push_str("</a>");
                            }
                        }
                        HtmlSlotKind::Image {
                            fallback,
                            alt,
                            attrs,
                        } => {
                            let url = resolved_urls
                                .get(idx)
                                .and_then(|u| u.as_deref())
                                .unwrap_or(fallback.as_ref());
                            if notist_html::is_safe_url(url) {
                                let _ = write!(
                                    out,
                                    "<img src=\"{}\" alt=\"{}\"{}>",
                                    notist_html::escape_attribute(url),
                                    alt,
                                    attrs
                                );
                            }
                        }
                    }
                }
                out.push_str(&buffer[cursor..]);
                out
            }
            HtmlPlan::Deferred(root) => {
                let mut html = String::new();
                let mut ctx = RenderCtx {
                    heading_ids: &self.heading_ids,
                    heading_cursor: 0,
                    ref_cursor: 0,
                    target: RenderTarget::Direct {
                        resolved_urls,
                        web_components,
                    },
                };
                render_block(root, &mut ctx, &mut html);
                html
            }
        }
    }
}

enum RenderTarget<'a> {
    Direct {
        resolved_urls: &'a [Option<String>],
        web_components: &'a BTreeMap<(String, String), String>,
    },
    Plan {
        slots: &'a mut Vec<HtmlSlot>,
    },
}

struct RenderCtx<'a> {
    heading_ids: &'a [String],
    heading_cursor: usize,
    ref_cursor: usize,
    target: RenderTarget<'a>,
}

fn collect_block_references(
    block: &Block,
    origin: &PathBuf,
    source_text: &Arc<str>,
    out: &mut Vec<Reference>,
    customs: &mut BTreeSet<(String, String)>,
) {
    match &block.node().kind {
        BlockKind::Fragment(children) => {
            for child in children.iter() {
                collect_block_references(child, origin, source_text, out, customs);
            }
        }
        BlockKind::Paragraph(inline) | BlockKind::Heading { title: inline, .. } => {
            collect_inline_references(inline, origin, source_text, out, customs);
        }
        BlockKind::Section { heading, body } => {
            collect_block_references(heading, origin, source_text, out, customs);
            collect_block_references(body, origin, source_text, out, customs);
        }
        BlockKind::Callout { body, .. }
        | BlockKind::Quote(body)
        | BlockKind::Div(body)
        | BlockKind::SectionBox(body) => {
            collect_block_references(body, origin, source_text, out, customs);
        }
        BlockKind::List { items, .. } => {
            for item in items.iter() {
                collect_block_references(item, origin, source_text, out, customs);
            }
        }
        BlockKind::Table { headers, rows, .. } => {
            for cell in headers.iter() {
                collect_inline_references(cell, origin, source_text, out, customs);
            }
            for row in rows.iter() {
                for cell in row.iter() {
                    collect_inline_references(cell, origin, source_text, out, customs);
                }
            }
        }
        BlockKind::Custom(custom) => {
            customs.insert((custom.module.to_string(), custom.name.to_string()));
            for (_, val) in custom.props.iter() {
                match val {
                    CustomValue::Inline(i) => {
                        collect_inline_references(i, origin, source_text, out, customs);
                    }
                    CustomValue::Block(b) => {
                        collect_block_references(b, origin, source_text, out, customs);
                    }
                    _ => {}
                }
            }
        }
        BlockKind::Math(_) | BlockKind::CodeBlock { .. } | BlockKind::Divider => {}
    }
}

fn collect_inline_references(
    inline: &Inline,
    origin: &PathBuf,
    source_text: &Arc<str>,
    out: &mut Vec<Reference>,
    customs: &mut BTreeSet<(String, String)>,
) {
    match &inline.node().kind {
        InlineKind::Fragment(children) => {
            for child in children.iter() {
                collect_inline_references(child, origin, source_text, out, customs);
            }
        }
        InlineKind::Strong(inner)
        | InlineKind::Emph(inner)
        | InlineKind::Strike(inner)
        | InlineKind::Underline(inner) => {
            collect_inline_references(inner, origin, source_text, out, customs);
        }
        InlineKind::Link { target, label } => {
            out.push(Reference {
                target: target.to_string(),
                is_image: false,
                origin: origin.clone(),
                span: inline.node().span,
                source_text: Arc::clone(source_text),
            });
            collect_inline_references(label, origin, source_text, out, customs);
        }
        InlineKind::Image { target, .. } => {
            out.push(Reference {
                target: target.to_string(),
                is_image: true,
                origin: origin.clone(),
                span: inline.node().span,
                source_text: Arc::clone(source_text),
            });
        }
        InlineKind::Custom(custom) => {
            customs.insert((custom.module.to_string(), custom.name.to_string()));
            for (_, val) in custom.props.iter() {
                match val {
                    CustomValue::Inline(i) => {
                        collect_inline_references(i, origin, source_text, out, customs);
                    }
                    CustomValue::Block(b) => {
                        collect_block_references(b, origin, source_text, out, customs);
                    }
                    _ => {}
                }
            }
        }
        InlineKind::Math(_) | InlineKind::Text(_) | InlineKind::Raw(_) => {}
    }
}

fn has_html_wrapper_attrs(attrs: &notc::Attrs) -> bool {
    attrs.entries().iter().any(|(k, v)| {
        matches!(v, AttrValue::String(_))
            && (k.as_ref() == "id"
                || k.as_ref() == "class"
                || k.starts_with("aria-")
                || k.starts_with("data-"))
    })
}

fn write_common_attrs(attrs: &notc::Attrs, skip_id: bool, out: &mut String) {
    if !skip_id
        && let Some(AttrValue::String(id)) = attrs.get("id")
    {
        let _ = write!(out, " id=\"{}\"", notist_html::escape_attribute(id));
    }
    if let Some(AttrValue::String(class)) = attrs.get("class") {
        let _ = write!(out, " class=\"{}\"", notist_html::escape_attribute(class));
    }
    for (k, v) in attrs.entries() {
        if (k.starts_with("aria-") || k.starts_with("data-"))
            && let AttrValue::String(s) = v
        {
            let _ = write!(out, " {k}=\"{}\"", notist_html::escape_attribute(s));
        }
    }
}

fn render_custom_node(
    custom: &CustomNode,
    is_block: bool,
    attrs: &notc::Attrs,
    ctx: &mut RenderCtx<'_>,
    out: &mut String,
) {
    let key = (custom.module.to_string(), custom.name.to_string());
    let mut scalar_props = serde_json::Map::new();
    let mut html_prop_attrs: Vec<(String, String)> = Vec::new();
    for (k, v) in custom.props.iter() {
        match v {
            CustomValue::Unit => {
                scalar_props.insert(k.to_string(), serde_json::Value::Null);
            }
            CustomValue::Bool(b) => {
                scalar_props.insert(k.to_string(), serde_json::Value::Bool(*b));
                html_prop_attrs.push((format!("notist-{k}"), b.to_string()));
            }
            CustomValue::Int(n) => {
                scalar_props.insert(k.to_string(), serde_json::Value::Number((*n).into()));
                html_prop_attrs.push((format!("notist-{k}"), n.to_string()));
            }
            CustomValue::Float(f) => {
                if let Some(num) = serde_json::Number::from_f64(*f) {
                    scalar_props.insert(k.to_string(), serde_json::Value::Number(num));
                }
                html_prop_attrs.push((format!("notist-{k}"), f.to_string()));
            }
            CustomValue::String(s) => {
                scalar_props.insert(k.to_string(), serde_json::Value::String(s.to_string()));
                html_prop_attrs.push((format!("notist-{k}"), s.to_string()));
            }
            CustomValue::Inline(_) | CustomValue::Block(_) => {}
        }
    }

    let props_json = serde_json::to_string(&scalar_props).unwrap_or_else(|_| "{}".into());
    let content_mode = if is_block { "block" } else { "inline" };
    let tag_opt = match &ctx.target {
        RenderTarget::Direct { web_components, .. } => web_components.get(&key).cloned(),
        RenderTarget::Plan { .. } => None,
    };
    if let Some(tag) = tag_opt {
        let _ = write!(
            out,
            "<{tag} notist-content=\"{content_mode}\" data-props=\"{}\"",
            notist_html::escape_attribute(&props_json)
        );
        for (attr_name, attr_val) in &html_prop_attrs {
            let _ = write!(
                out,
                " {attr_name}=\"{}\"",
                notist_html::escape_attribute(attr_val)
            );
        }
        write_common_attrs(attrs, false, out);
        out.push('>');
        for (_, v) in custom.props.iter() {
            match v {
                CustomValue::Inline(i) => render_inline(i, ctx, out),
                CustomValue::Block(b) => render_block(b, ctx, out),
                _ => {}
            }
        }
        let _ = write!(out, "</{tag}>");
    } else {
        let fallback_tag = if is_block { "div" } else { "span" };
        let _ = write!(
            out,
            "<{fallback_tag} class=\"notist-component notist-{}\" data-component=\"{}::{}\" data-props=\"{}\"",
            notist_html::escape_attribute(&custom.name),
            notist_html::escape_attribute(&custom.module),
            notist_html::escape_attribute(&custom.name),
            notist_html::escape_attribute(&props_json)
        );
        write_common_attrs(attrs, false, out);
        out.push('>');
        for (_, v) in custom.props.iter() {
            match v {
                CustomValue::Inline(i) => render_inline(i, ctx, out),
                CustomValue::Block(b) => render_block(b, ctx, out),
                _ => {}
            }
        }
        let _ = write!(out, "</{fallback_tag}>");
    }
}

fn render_block(block: &Block, ctx: &mut RenderCtx<'_>, out: &mut String) {
    let node = block.node();
    match &node.kind {
        BlockKind::Fragment(children) => {
            let has_wrapper_attr = has_html_wrapper_attrs(&node.attrs);
            if has_wrapper_attr {
                out.push_str("<div");
                write_common_attrs(&node.attrs, false, out);
                out.push('>');
            }
            for child in children.iter() {
                render_block(child, ctx, out);
            }
            if has_wrapper_attr {
                out.push_str("</div>");
            }
        }
        BlockKind::Section { heading, body } => {
            let id = ctx
                .heading_ids
                .get(ctx.heading_cursor)
                .map(String::as_str)
                .unwrap_or("");
            ctx.heading_cursor += 1;
            out.push_str("<section");
            if !id.is_empty() {
                let _ = write!(out, " id=\"{}\"", notist_html::escape_attribute(id));
            }
            write_common_attrs(&node.attrs, true, out);
            out.push('>');
            if let BlockKind::Heading { level, title } = &heading.node().kind {
                let _ = write!(out, "<h{level}");
                write_common_attrs(&heading.node().attrs, true, out);
                out.push('>');
                render_inline(title, ctx, out);
                let _ = write!(out, "</h{level}>");
            } else {
                render_block(heading, ctx, out);
            }
            render_block(body, ctx, out);
            out.push_str("</section>");
        }
        BlockKind::Heading { level, title } => {
            let id = ctx
                .heading_ids
                .get(ctx.heading_cursor)
                .map(String::as_str)
                .unwrap_or("");
            ctx.heading_cursor += 1;
            let _ = write!(out, "<h{level}");
            if !id.is_empty() {
                let _ = write!(out, " id=\"{}\"", notist_html::escape_attribute(id));
            }
            write_common_attrs(&node.attrs, true, out);
            out.push('>');
            render_inline(title, ctx, out);
            let _ = write!(out, "</h{level}>");
        }
        BlockKind::Paragraph(inline) => {
            out.push_str("<p");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_inline(inline, ctx, out);
            out.push_str("</p>");
        }
        BlockKind::List {
            ordered,
            start,
            items,
        } => {
            let tag = if *ordered { "ol" } else { "ul" };
            let _ = write!(out, "<{tag}");
            if *ordered && *start != 1 {
                let _ = write!(out, " start=\"{start}\"");
            }
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            for item in items.iter() {
                out.push_str("<li>");
                if let BlockKind::Paragraph(inline) = &item.node().kind {
                    render_inline(inline, ctx, out);
                } else {
                    render_block(item, ctx, out);
                }
                out.push_str("</li>");
            }
            let _ = write!(out, "</{tag}>");
        }
        BlockKind::Quote(body) => {
            out.push_str("<blockquote");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_block(body, ctx, out);
            out.push_str("</blockquote>");
        }
        BlockKind::Div(body) => {
            out.push_str("<div");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_block(body, ctx, out);
            out.push_str("</div>");
        }
        BlockKind::SectionBox(body) => {
            out.push_str("<section");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_block(body, ctx, out);
            out.push_str("</section>");
        }
        BlockKind::Divider => {
            out.push_str("<hr>");
        }
        BlockKind::Table {
            headers,
            alignments,
            rows,
        } => {
            out.push_str("<table");
            write_common_attrs(&node.attrs, false, out);
            out.push_str("><thead><tr>");
            for (idx, cell) in headers.iter().enumerate() {
                let align = alignments.get(idx).copied().unwrap_or(TableAlign::None);
                render_table_cell("th", cell, align, ctx, out);
            }
            out.push_str("</tr></thead><tbody>");
            for row in rows.iter() {
                out.push_str("<tr>");
                for (idx, cell) in row.iter().enumerate() {
                    let align = alignments.get(idx).copied().unwrap_or(TableAlign::None);
                    render_table_cell("td", cell, align, ctx, out);
                }
                out.push_str("</tr>");
            }
            out.push_str("</tbody></table>");
        }
        BlockKind::Math(expr) => {
            out.push_str("<div class=\"notist-math\"");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            out.push_str(&notist_html::escape_text(expr));
            out.push_str("</div>");
        }
        BlockKind::Custom(custom) => {
            render_custom_node(custom, true, &node.attrs, ctx, out);
        }
        BlockKind::Callout { kind, body } => {
            let escaped_kind = notist_html::escape_attribute(kind);
            let _ = write!(out, "<aside class=\"callout callout-{escaped_kind}\"");
            if let Some(AttrValue::String(id)) = node.attrs.get("id") {
                let _ = write!(out, " id=\"{}\"", notist_html::escape_attribute(id));
            }
            out.push('>');
            render_block(body, ctx, out);
            out.push_str("</aside>");
        }
        BlockKind::CodeBlock { lang, text } => {
            let language = notist_html::code_language(lang);
            out.push_str("<pre><code");
            if let Some(l) = language {
                let _ = write!(
                    out,
                    " class=\"language-{}\"",
                    notist_html::escape_attribute(l)
                );
            }
            out.push('>');
            if let Some(highlighted) =
                language.and_then(|l| notist_html::highlight::highlight_str(l, text))
            {
                out.push_str(&highlighted);
            } else {
                out.push_str(&notist_html::escape_text(text));
            }
            out.push_str("</code></pre>");
        }
    }
}

fn render_table_cell(
    tag: &str,
    cell: &Inline,
    align: TableAlign,
    ctx: &mut RenderCtx<'_>,
    out: &mut String,
) {
    let node = cell.node();
    let _ = write!(out, "<{tag}");
    match align {
        TableAlign::None => {}
        TableAlign::Left => out.push_str(" style=\"text-align:left\""),
        TableAlign::Center => out.push_str(" style=\"text-align:center\""),
        TableAlign::Right => out.push_str(" style=\"text-align:right\""),
    }
    let lift_attrs = matches!(node.kind, InlineKind::Text(_) | InlineKind::Fragment(_))
        && has_html_wrapper_attrs(&node.attrs);
    if lift_attrs {
        write_common_attrs(&node.attrs, false, out);
    }
    out.push('>');
    if lift_attrs {
        match &node.kind {
            InlineKind::Text(text) => out.push_str(&notist_html::escape_text(text)),
            InlineKind::Fragment(children) => {
                for child in children.iter() {
                    render_inline(child, ctx, out);
                }
            }
            _ => unreachable!(),
        }
    } else {
        render_inline(cell, ctx, out);
    }
    let _ = write!(out, "</{tag}>");
}

fn render_inline(inline: &Inline, ctx: &mut RenderCtx<'_>, out: &mut String) {
    let node = inline.node();
    match &node.kind {
        InlineKind::Fragment(children) => {
            let has_wrapper_attr = has_html_wrapper_attrs(&node.attrs);
            if has_wrapper_attr {
                out.push_str("<span");
                write_common_attrs(&node.attrs, false, out);
                out.push('>');
            }
            for child in children.iter() {
                render_inline(child, ctx, out);
            }
            if has_wrapper_attr {
                out.push_str("</span>");
            }
        }
        InlineKind::Text(text) => {
            let has_wrapper_attr = has_html_wrapper_attrs(&node.attrs);
            if has_wrapper_attr {
                out.push_str("<span");
                write_common_attrs(&node.attrs, false, out);
                out.push('>');
            }
            out.push_str(&notist_html::escape_text(text));
            if has_wrapper_attr {
                out.push_str("</span>");
            }
        }
        InlineKind::Strong(inner) => {
            out.push_str("<strong");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_inline(inner, ctx, out);
            out.push_str("</strong>");
        }
        InlineKind::Emph(inner) => {
            out.push_str("<em");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_inline(inner, ctx, out);
            out.push_str("</em>");
        }
        InlineKind::Strike(inner) => {
            out.push_str("<del");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_inline(inner, ctx, out);
            out.push_str("</del>");
        }
        InlineKind::Underline(inner) => {
            out.push_str("<u");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_inline(inner, ctx, out);
            out.push_str("</u>");
        }
        InlineKind::Raw(code) => {
            out.push_str("<code");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            out.push_str(&notist_html::escape_text(code));
            out.push_str("</code>");
        }
        InlineKind::Math(expr) => {
            out.push_str("<span class=\"notist-math\"");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            out.push_str(&notist_html::escape_text(expr));
            out.push_str("</span>");
        }
        InlineKind::Link { target, label } => {
            let idx = ctx.ref_cursor;
            ctx.ref_cursor += 1;
            match &mut ctx.target {
                RenderTarget::Direct { resolved_urls, .. } => {
                    let url = resolved_urls
                        .get(idx)
                        .and_then(|u| u.as_deref())
                        .unwrap_or(target.as_ref());
                    if notist_html::is_safe_url(url) {
                        let _ = write!(out, "<a href=\"{}\"", notist_html::escape_attribute(url));
                        write_common_attrs(&node.attrs, false, out);
                        out.push('>');
                        render_inline(label, ctx, out);
                        out.push_str("</a>");
                    } else {
                        render_inline(label, ctx, out);
                    }
                }
                RenderTarget::Plan { slots } => {
                    let mut attrs_buf = String::new();
                    write_common_attrs(&node.attrs, false, &mut attrs_buf);
                    slots.push(HtmlSlot {
                        offset: out.len() as u32,
                        ref_idx: idx as u32,
                        kind: HtmlSlotKind::LinkOpen {
                            fallback: target.as_ref().into(),
                            attrs: attrs_buf.into_boxed_str(),
                        },
                    });
                    render_inline(label, ctx, out);
                    if let RenderTarget::Plan { slots } = &mut ctx.target {
                        slots.push(HtmlSlot {
                            offset: out.len() as u32,
                            ref_idx: idx as u32,
                            kind: HtmlSlotKind::LinkClose {
                                fallback_safe: notist_html::is_safe_url(target.as_ref()),
                            },
                        });
                    }
                }
            }
        }
        InlineKind::Image { target, alt } => {
            let idx = ctx.ref_cursor;
            ctx.ref_cursor += 1;
            match &mut ctx.target {
                RenderTarget::Direct { resolved_urls, .. } => {
                    let url = resolved_urls
                        .get(idx)
                        .and_then(|u| u.as_deref())
                        .unwrap_or(target.as_ref());
                    if notist_html::is_safe_url(url) {
                        let _ = write!(
                            out,
                            "<img src=\"{}\" alt=\"{}\"",
                            notist_html::escape_attribute(url),
                            notist_html::escape_attribute(alt)
                        );
                        write_common_attrs(&node.attrs, false, out);
                        out.push('>');
                    }
                }
                RenderTarget::Plan { slots } => {
                    let mut attrs_buf = String::new();
                    write_common_attrs(&node.attrs, false, &mut attrs_buf);
                    slots.push(HtmlSlot {
                        offset: out.len() as u32,
                        ref_idx: idx as u32,
                        kind: HtmlSlotKind::Image {
                            fallback: target.as_ref().into(),
                            alt: notist_html::escape_attribute(alt).into_boxed_str(),
                            attrs: attrs_buf.into_boxed_str(),
                        },
                    });
                }
            }
        }
        InlineKind::Custom(custom) => {
            render_custom_node(custom, false, &node.attrs, ctx, out);
        }
    }
}
