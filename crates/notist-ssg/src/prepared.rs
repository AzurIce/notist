//! The ownership boundary between evaluated `notc::Block` IR and whole-site
//! link resolution + HTML rendering.

use crate::{
    Error, Page, Route,
    document::{Document, diagnostic},
};
use notc::{AttrValue, Block, BlockKind, CustomNode, CustomValue, Inline, InlineKind, Span};
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
    let sectioned = block.sectionize();
    let route = Route::plan(PathBuf::from("./index.not"), std::path::Path::new("."), false)
        .expect("valid route");
    match PreparedPage::new(route, Arc::from(""), sectioned, "Notist") {
        Ok(prepared) => prepared.render_html(&[], &BTreeMap::new()),
        Err(_) => String::new(),
    }
}

pub(crate) struct PreparedPage {
    pub page: Page,
    pub root: Block,
    pub ids: BTreeSet<String>,
    pub heading_ids: Vec<String>,
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
        Ok(Self {
            page: document.page,
            root: document.root,
            ids: document.ids,
            heading_ids: document.heading_ids,
            references,
            custom_nodes,
        })
    }

    pub fn render_html(
        &self,
        resolved_urls: &[Option<String>],
        web_components: &BTreeMap<(String, String), String>,
    ) -> String {
        let mut html = String::new();
        let mut ref_cursor = 0usize;
        let mut heading_cursor = 0usize;
        render_block(
            &self.root,
            &self.heading_ids,
            resolved_urls,
            web_components,
            &mut heading_cursor,
            &mut ref_cursor,
            &mut html,
        );
        html
    }
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
        BlockKind::Callout { body, .. } | BlockKind::Quote(body) => {
            collect_block_references(body, origin, source_text, out, customs);
        }
        BlockKind::List { items, .. } => {
            for item in items.iter() {
                collect_block_references(item, origin, source_text, out, customs);
            }
        }
        BlockKind::Table { headers, rows } => {
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
        BlockKind::CodeBlock { .. } | BlockKind::Math(_) | BlockKind::Divider => {}
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
        InlineKind::Strong(inner) | InlineKind::Emph(inner) => {
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
        InlineKind::Text(_) | InlineKind::Raw(_) | InlineKind::Math(_) => {}
    }
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
}

fn render_custom_node(
    custom: &CustomNode,
    is_block: bool,
    attrs: &notc::Attrs,
    heading_ids: &[String],
    resolved_urls: &[Option<String>],
    web_components: &BTreeMap<(String, String), String>,
    heading_cursor: &mut usize,
    ref_cursor: &mut usize,
    out: &mut String,
) {
    let key = (custom.module.to_string(), custom.name.to_string());
    let mut scalar_props = serde_json::Map::new();
    for (k, v) in custom.props.iter() {
        match v {
            CustomValue::Unit => {
                scalar_props.insert(k.to_string(), serde_json::Value::Null);
            }
            CustomValue::Bool(b) => {
                scalar_props.insert(k.to_string(), serde_json::Value::Bool(*b));
            }
            CustomValue::Int(n) => {
                scalar_props.insert(k.to_string(), serde_json::Value::Number((*n).into()));
            }
            CustomValue::Float(f) => {
                if let Some(num) = serde_json::Number::from_f64(*f) {
                    scalar_props.insert(k.to_string(), serde_json::Value::Number(num));
                }
            }
            CustomValue::String(s) => {
                scalar_props.insert(k.to_string(), serde_json::Value::String(s.to_string()));
            }
            CustomValue::Inline(_) | CustomValue::Block(_) => {}
        }
    }

    let props_json = serde_json::to_string(&scalar_props).unwrap_or_else(|_| "{}".into());
    if let Some(tag) = web_components.get(&key) {
        let _ = write!(
            out,
            "<{tag} data-props=\"{}\"",
            notist_html::escape_attribute(&props_json)
        );
        write_common_attrs(attrs, false, out);
        out.push('>');
        for (_, v) in custom.props.iter() {
            match v {
                CustomValue::Inline(i) => {
                    render_inline(i, heading_ids, resolved_urls, web_components, heading_cursor, ref_cursor, out);
                }
                CustomValue::Block(b) => {
                    render_block(b, heading_ids, resolved_urls, web_components, heading_cursor, ref_cursor, out);
                }
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
                CustomValue::Inline(i) => {
                    render_inline(i, heading_ids, resolved_urls, web_components, heading_cursor, ref_cursor, out);
                }
                CustomValue::Block(b) => {
                    render_block(b, heading_ids, resolved_urls, web_components, heading_cursor, ref_cursor, out);
                }
                _ => {}
            }
        }
        let _ = write!(out, "</{fallback_tag}>");
    }
}

#[allow(clippy::too_many_arguments)]
fn render_block(
    block: &Block,
    heading_ids: &[String],
    resolved_urls: &[Option<String>],
    web_components: &BTreeMap<(String, String), String>,
    heading_cursor: &mut usize,
    ref_cursor: &mut usize,
    out: &mut String,
) {
    let node = block.node();
    match &node.kind {
        BlockKind::Fragment(children) => {
            let has_wrapper_attr = node.attrs.get("id").is_some() || node.attrs.get("class").is_some();
            if has_wrapper_attr {
                out.push_str("<div");
                write_common_attrs(&node.attrs, false, out);
                out.push('>');
            }
            for child in children.iter() {
                render_block(
                    child,
                    heading_ids,
                    resolved_urls,
                    web_components,
                    heading_cursor,
                    ref_cursor,
                    out,
                );
            }
            if has_wrapper_attr {
                out.push_str("</div>");
            }
        }
        BlockKind::Section { heading, body } => {
            out.push_str("<section");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_block(
                heading,
                heading_ids,
                resolved_urls,
                web_components,
                heading_cursor,
                ref_cursor,
                out,
            );
            render_block(
                body,
                heading_ids,
                resolved_urls,
                web_components,
                heading_cursor,
                ref_cursor,
                out,
            );
            out.push_str("</section>");
        }
        BlockKind::Heading { level, title } => {
            let id = heading_ids
                .get(*heading_cursor)
                .map(String::as_str)
                .unwrap_or("");
            *heading_cursor += 1;
            let _ = write!(out, "<h{level}");
            if !id.is_empty() {
                let _ = write!(out, " id=\"{}\"", notist_html::escape_attribute(id));
            }
            write_common_attrs(&node.attrs, true, out);
            out.push('>');
            render_inline(
                title,
                heading_ids,
                resolved_urls,
                web_components,
                heading_cursor,
                ref_cursor,
                out,
            );
            let _ = write!(out, "</h{level}>");
        }
        BlockKind::Paragraph(inline) => {
            out.push_str("<p");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_inline(
                inline,
                heading_ids,
                resolved_urls,
                web_components,
                heading_cursor,
                ref_cursor,
                out,
            );
            out.push_str("</p>");
        }
        BlockKind::List { ordered, items } => {
            let tag = if *ordered { "ol" } else { "ul" };
            let _ = write!(out, "<{tag}");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            for item in items.iter() {
                out.push_str("<li>");
                if let BlockKind::Paragraph(inline) = &item.node().kind {
                    render_inline(
                        inline,
                        heading_ids,
                        resolved_urls,
                        web_components,
                        heading_cursor,
                        ref_cursor,
                        out,
                    );
                } else {
                    render_block(
                        item,
                        heading_ids,
                        resolved_urls,
                        web_components,
                        heading_cursor,
                        ref_cursor,
                        out,
                    );
                }
                out.push_str("</li>");
            }
            let _ = write!(out, "</{tag}>");
        }
        BlockKind::Quote(body) => {
            out.push_str("<blockquote");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_block(
                body,
                heading_ids,
                resolved_urls,
                web_components,
                heading_cursor,
                ref_cursor,
                out,
            );
            out.push_str("</blockquote>");
        }
        BlockKind::Divider => {
            out.push_str("<hr>");
        }
        BlockKind::Table { headers, rows } => {
            out.push_str("<table");
            write_common_attrs(&node.attrs, false, out);
            out.push_str("><thead><tr>");
            for cell in headers.iter() {
                out.push_str("<th>");
                render_inline(
                    cell,
                    heading_ids,
                    resolved_urls,
                    web_components,
                    heading_cursor,
                    ref_cursor,
                    out,
                );
                out.push_str("</th>");
            }
            out.push_str("</tr></thead><tbody>");
            for row in rows.iter() {
                out.push_str("<tr>");
                for cell in row.iter() {
                    out.push_str("<td>");
                    render_inline(
                        cell,
                        heading_ids,
                        resolved_urls,
                        web_components,
                        heading_cursor,
                        ref_cursor,
                        out,
                    );
                    out.push_str("</td>");
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
            render_custom_node(
                custom,
                true,
                &node.attrs,
                heading_ids,
                resolved_urls,
                web_components,
                heading_cursor,
                ref_cursor,
                out,
            );
        }
        BlockKind::Callout { kind, body } => {
            let escaped_kind = notist_html::escape_attribute(kind);
            let _ = write!(out, "<aside class=\"callout callout-{escaped_kind}\"");
            if let Some(AttrValue::String(id)) = node.attrs.get("id") {
                let _ = write!(out, " id=\"{}\"", notist_html::escape_attribute(id));
            }
            out.push('>');
            render_block(
                body,
                heading_ids,
                resolved_urls,
                web_components,
                heading_cursor,
                ref_cursor,
                out,
            );
            out.push_str("</aside>");
        }
        BlockKind::CodeBlock { lang, text } => {
            let language = if lang.is_empty() {
                None
            } else {
                Some(lang.as_ref())
            };
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

#[allow(clippy::too_many_arguments)]
fn render_inline(
    inline: &Inline,
    heading_ids: &[String],
    resolved_urls: &[Option<String>],
    web_components: &BTreeMap<(String, String), String>,
    heading_cursor: &mut usize,
    ref_cursor: &mut usize,
    out: &mut String,
) {
    let node = inline.node();
    match &node.kind {
        InlineKind::Fragment(children) => {
            let has_wrapper_attr = node.attrs.get("id").is_some() || node.attrs.get("class").is_some();
            if has_wrapper_attr {
                out.push_str("<span");
                write_common_attrs(&node.attrs, false, out);
                out.push('>');
            }
            for child in children.iter() {
                render_inline(
                    child,
                    heading_ids,
                    resolved_urls,
                    web_components,
                    heading_cursor,
                    ref_cursor,
                    out,
                );
            }
            if has_wrapper_attr {
                out.push_str("</span>");
            }
        }
        InlineKind::Text(text) => {
            out.push_str(&notist_html::escape_text(text));
        }
        InlineKind::Strong(inner) => {
            out.push_str("<strong");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_inline(
                inner,
                heading_ids,
                resolved_urls,
                web_components,
                heading_cursor,
                ref_cursor,
                out,
            );
            out.push_str("</strong>");
        }
        InlineKind::Emph(inner) => {
            out.push_str("<em");
            write_common_attrs(&node.attrs, false, out);
            out.push('>');
            render_inline(
                inner,
                heading_ids,
                resolved_urls,
                web_components,
                heading_cursor,
                ref_cursor,
                out,
            );
            out.push_str("</em>");
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
            let idx = *ref_cursor;
            *ref_cursor += 1;
            let url = resolved_urls
                .get(idx)
                .and_then(|u| u.as_deref())
                .unwrap_or(target.as_ref());
            if notist_html::is_safe_url(url) {
                let _ = write!(out, "<a href=\"{}\"", notist_html::escape_attribute(url));
                write_common_attrs(&node.attrs, false, out);
                out.push('>');
                render_inline(
                    label,
                    heading_ids,
                    resolved_urls,
                    web_components,
                    heading_cursor,
                    ref_cursor,
                    out,
                );
                out.push_str("</a>");
            } else {
                render_inline(
                    label,
                    heading_ids,
                    resolved_urls,
                    web_components,
                    heading_cursor,
                    ref_cursor,
                    out,
                );
            }
        }
        InlineKind::Image { target, alt } => {
            let idx = *ref_cursor;
            *ref_cursor += 1;
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
        InlineKind::Custom(custom) => {
            render_custom_node(
                custom,
                false,
                &node.attrs,
                heading_ids,
                resolved_urls,
                web_components,
                heading_cursor,
                ref_cursor,
                out,
            );
        }
    }
}
